use super::{google, permissions, ConnectorError, Permission};
use crate::credentials::{CredentialId, Provider, Secret};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use url::Url;
use zeroize::Zeroizing;

const CALLBACK_PATH: &str = "/oauth/callback";
const FLOW_LIFETIME: Duration = Duration::from_secs(300);
const MAX_CALLBACK_BYTES: usize = 16 * 1024;

pub(crate) struct OAuthClientConfig {
    pub(super) client_id: String,
    pub(super) client_secret: Option<CredentialId>,
}
impl OAuthClientConfig {
    pub(crate) fn new(
        client_id: String,
        client_secret: Option<CredentialId>,
    ) -> Result<Self, ConnectorError> {
        if client_id.is_empty()
            || client_id.len() > 256
            || !client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            || client_secret.is_some_and(|id| !id.is_client_secret(Provider::Google))
        {
            return Err(ConnectorError::InvalidClient);
        }
        Ok(Self {
            client_id,
            client_secret,
        })
    }
}

pub(crate) struct LoopbackRedirect {
    uri: String,
    port: u16,
}
impl LoopbackRedirect {
    // M41 must keep a listener bound to this OS-assigned port before opening the browser.
    pub(crate) fn from_bound_listener(
        listener: &std::net::TcpListener,
    ) -> Result<Self, ConnectorError> {
        let address = listener
            .local_addr()
            .map_err(|_| ConnectorError::InvalidClient)?;
        if address.ip() != std::net::Ipv4Addr::LOCALHOST {
            return Err(ConnectorError::InvalidClient);
        }
        Self::new(address.port())
    }
    fn new(port: u16) -> Result<Self, ConnectorError> {
        if port == 0 {
            return Err(ConnectorError::InvalidClient);
        }
        Ok(Self {
            uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
            port,
        })
    }
    pub(super) fn uri(&self) -> &str {
        &self.uri
    }
}
pub(crate) struct OAuthAuthorizationRequest {
    url: Zeroizing<String>,
}
impl OAuthAuthorizationRequest {
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}
pub(crate) struct OAuthPendingFlow {
    config: OAuthClientConfig,
    state: Secret,
    verifier: Secret,
    redirect: LoopbackRedirect,
    requested: Vec<Permission>,
    created_at: Instant,
    expires_at: Instant,
}
pub(crate) struct OAuthExchange {
    pub(super) config: OAuthClientConfig,
    pub(super) code: Secret,
    pub(super) verifier: Secret,
    pub(super) redirect: LoopbackRedirect,
    pub(super) requested: Vec<Permission>,
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
impl OAuthPendingFlow {
    pub(crate) fn new(
        config: OAuthClientConfig,
        redirect: LoopbackRedirect,
        requested: Vec<Permission>,
    ) -> Result<(OAuthAuthorizationRequest, Self), ConnectorError> {
        Self::generate(config, redirect, requested, Instant::now(), |bytes| {
            getrandom::fill(bytes).map_err(|_| ConnectorError::Randomness)
        })
    }
    fn generate(
        config: OAuthClientConfig,
        redirect: LoopbackRedirect,
        requested: Vec<Permission>,
        now: Instant,
        mut random: impl FnMut(&mut [u8]) -> Result<(), ConnectorError>,
    ) -> Result<(OAuthAuthorizationRequest, Self), ConnectorError> {
        let requested = permissions(requested)?;
        let mut state = Zeroizing::new([0u8; 32]);
        let mut verifier = Zeroizing::new([0u8; 64]);
        random(&mut state[..])?;
        random(&mut verifier[..])?;
        let state = Secret::new(URL_SAFE_NO_PAD.encode(&state[..]))?;
        let verifier = Secret::new(URL_SAFE_NO_PAD.encode(&verifier[..]))?;
        let request = OAuthAuthorizationRequest {
            url: google::authorization_url(
                &config,
                redirect.uri(),
                &requested,
                &state,
                &challenge(verifier.expose()),
            )?,
        };
        let pending = Self {
            config,
            state,
            verifier,
            redirect,
            requested,
            created_at: now,
            expires_at: now
                .checked_add(FLOW_LIFETIME)
                .ok_or(ConnectorError::ExpiredFlow)?,
        };
        Ok((request, pending))
    }
    pub(crate) fn accept_callback(
        self,
        value: &str,
        now: Instant,
    ) -> Result<OAuthExchange, ConnectorError> {
        if now < self.created_at || now >= self.expires_at {
            return Err(ConnectorError::ExpiredFlow);
        }
        let code = self.parse_callback(value)?;
        // Moving the pending flow consumes state and makes code/verifier exchange single-use.
        Ok(OAuthExchange {
            config: self.config,
            code,
            verifier: self.verifier,
            redirect: self.redirect,
            requested: self.requested,
        })
    }
    pub(super) fn validate_callback(
        &self,
        value: &str,
        now: Instant,
    ) -> Result<(), ConnectorError> {
        if now < self.created_at || now >= self.expires_at {
            return Err(ConnectorError::ExpiredFlow);
        }
        self.parse_callback(value).map(|_| ())
    }
    fn parse_callback(&self, value: &str) -> Result<Secret, ConnectorError> {
        let invalid = ConnectorError::InvalidCallback;
        if value.len() > MAX_CALLBACK_BYTES || value.chars().any(char::is_control) {
            return Err(invalid);
        }
        // Parse only the non-sensitive base with Url; do not make an unzeroized copy of the code/query.
        let (base, raw) = value.split_once('?').ok_or(invalid)?;
        let url = Url::parse(base).map_err(|_| invalid)?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port() != Some(self.redirect.port)
            || url.path() != CALLBACK_PATH
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || base != self.redirect.uri()
            || value.contains('#')
        {
            return Err(invalid);
        }
        if raw.len() > 8192 {
            return Err(invalid);
        }
        let bytes = raw.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'%'
                && (index + 2 >= bytes.len()
                    || !bytes[index + 1].is_ascii_hexdigit()
                    || !bytes[index + 2].is_ascii_hexdigit())
            {
                return Err(invalid);
            }
        }
        let mut fields = std::collections::BTreeMap::new();
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            if !matches!(
                key.as_ref(),
                "state"
                    | "code"
                    | "error"
                    | "error_description"
                    | "error_uri"
                    | "scope"
                    | "authuser"
                    | "prompt"
            ) || fields.len() >= 8
                || fields
                    .insert(key.into_owned(), Zeroizing::new(value.into_owned()))
                    .is_some()
            {
                return Err(invalid);
            }
        }
        let state = fields.get("state").ok_or(invalid)?;
        if !bool::from(state.as_bytes().ct_eq(self.state.expose().as_bytes())) {
            return Err(invalid);
        }
        if fields.contains_key("error") {
            if fields.contains_key("code") {
                return Err(invalid);
            }
            return Err(ConnectorError::ProviderDenied);
        }
        if fields.contains_key("error_description") || fields.contains_key("error_uri") {
            return Err(invalid);
        }
        if let Some(scope) = fields.get("scope") {
            google::granted_permissions(scope, &self.requested)?;
        }
        let code = fields.remove("code").ok_or(invalid)?;
        if code.len() > 4096 {
            return Err(invalid);
        }
        Secret::new(code.to_string()).map_err(|_| invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture(
        permissions: Vec<Permission>,
    ) -> (OAuthAuthorizationRequest, OAuthPendingFlow) {
        OAuthPendingFlow::generate(
            OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), None).unwrap(),
            LoopbackRedirect::new(49152).unwrap(),
            permissions,
            Instant::now(),
            |bytes| {
                bytes.fill(42);
                Ok(())
            },
        )
        .unwrap()
    }
    #[test]
    fn rfc7636_s256_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn production_cs_random_state_and_verifier() {
        let make = || {
            OAuthPendingFlow::generate(
                OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), None)
                    .unwrap(),
                LoopbackRedirect::new(49152).unwrap(),
                vec![Permission::MailRead],
                Instant::now(),
                |bytes| getrandom::fill(bytes).map_err(|_| ConnectorError::Randomness),
            )
            .unwrap()
        };
        let (_, a) = make();
        let (_, b) = make();
        assert!(a.state.expose() != b.state.expose());
        assert!(a.verifier.expose() != b.verifier.expose());
        assert_eq!(a.state.expose().len(), 43);
        assert!((43..=128).contains(&a.verifier.expose().len()));
        assert!(a
            .verifier
            .expose()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }
    #[test]
    fn random_failure_closed() {
        let result = OAuthPendingFlow::generate(
            OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), None).unwrap(),
            LoopbackRedirect::new(49152).unwrap(),
            vec![Permission::MailRead],
            Instant::now(),
            |_| Err(ConnectorError::Randomness),
        );
        assert!(matches!(result, Err(ConnectorError::Randomness)));
    }
    #[test]
    fn callback_success_and_provider_denial() {
        let (_, pending) = fixture(vec![Permission::MailRead]);
        let callback = format!(
            "{}?state={}&code=synthetic-code",
            pending.redirect.uri(),
            pending.state.expose()
        );
        let now = Instant::now();
        let exchange = pending.accept_callback(&callback, now).unwrap();
        assert_eq!(exchange.code.expose(), "synthetic-code");
        let (_, pending) = fixture(vec![Permission::MailRead]);
        let callback = format!(
            "{}?state={}&error=access_denied&error_description=private",
            pending.redirect.uri(),
            pending.state.expose()
        );
        assert!(matches!(
            pending.accept_callback(&callback, Instant::now()),
            Err(ConnectorError::ProviderDenied)
        ));
    }
    #[test]
    fn callback_rejects_adversarial_urls_without_echoing_codes() {
        for value in [
            "http://127.0.0.1:49152/oauth/callback?state=wrong&code=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?code=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?state={state}&state={state}&code=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?state={state}",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=a&code=b",
            "http://example.invalid:49152/oauth/callback?state={state}&code=synthetic-code",
            "http://127.0.0.1:49152/wrong?state={state}&code=synthetic-code",
            "https://127.0.0.1:49152/oauth/callback?state={state}&code=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=synthetic-code#credential",
            "http://user@127.0.0.1:49152/oauth/callback?state={state}&code=synthetic-code",
            "http://localhost:49152/oauth/callback?state={state}&code=synthetic-code",
            "http://127.0.0.1:49153/oauth/callback?state={state}&code=synthetic-code",
            "http://127.1:49152/oauth/callback?state={state}&code=synthetic-code",
            "http://127.0.0.1:49152/a/../oauth/callback?state={state}&code=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=%GG",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=%FF",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=&access_token=synthetic-code",
            "http://127.0.0.1:49152/oauth/callback?state={state}&code=synthetic-code&error=denied",
        ] {
            let (_, pending) = fixture(vec![Permission::MailRead]);
            let value = value.replace("{state}", pending.state.expose());
            let error = pending
                .accept_callback(&value, Instant::now())
                .err()
                .unwrap();
            assert!(!error.to_string().contains("synthetic-code"));
        }
    }
    #[test]
    fn callback_expiry_and_size_bounds() {
        let (_, pending) = fixture(vec![Permission::MailRead]);
        let expired = pending.expires_at;
        assert!(matches!(
            pending.accept_callback("ignored", expired),
            Err(ConnectorError::ExpiredFlow)
        ));
        let (_, pending) = fixture(vec![Permission::MailRead]);
        assert!(matches!(
            pending.accept_callback(&"x".repeat(MAX_CALLBACK_BYTES + 1), Instant::now()),
            Err(ConnectorError::InvalidCallback)
        ));
    }
    #[test]
    fn authorization_url_contains_exact_requested_scopes_and_no_credentials() {
        let (request, pending) = fixture(vec![Permission::MailRead, Permission::ContactsRead]);
        let url = Url::parse(request.url()).unwrap();
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://accounts.google.com"
        );
        assert_eq!(url.path(), "/o/oauth2/v2/auth");
        let fields: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(fields["response_type"], "code");
        assert_eq!(fields["state"], pending.state.expose());
        assert_eq!(fields["code_challenge_method"], "S256");
        assert_eq!(
            fields["code_challenge"],
            challenge(pending.verifier.expose())
        );
        assert_eq!(fields["redirect_uri"], pending.redirect.uri());
        assert_eq!(
            fields["scope"],
            google::scopes(&pending.requested).join(" ")
        );
        assert!(!fields.contains_key("client_secret"));
        assert!(!fields.contains_key("code_verifier"));
        assert!(!fields.contains_key("access_token"));
    }
    #[test]
    fn client_config_rejects_invalid_identifiers_and_refresh_token_reference() {
        for id in [
            "",
            "https://client.invalid",
            "x.apps.googleusercontent.com?secret",
            "a\n.apps.googleusercontent.com",
        ] {
            assert!(OAuthClientConfig::new(id.into(), None).is_err());
        }
        let reference = CredentialId::new(
            Provider::Google,
            crate::credentials::AccountId::new(),
            crate::credentials::CredentialKind::OAuthRefreshToken,
        );
        assert!(OAuthClientConfig::new(
            "synthetic.apps.googleusercontent.com".into(),
            Some(reference)
        )
        .is_err());
    }
}
