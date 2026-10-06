use super::accounts::Account;
use super::oauth::{OAuthClientConfig, OAuthExchange};
use super::{permissions, ConnectorError, Permission};
use crate::credentials::{CredentialBroker, CredentialId, CredentialKind, CredentialStore, Secret};
use reqwest::blocking::{Body, Client};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

pub(super) const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub(super) const MAX_RESPONSE_BYTES: usize = 64 * 1024;

fn validate_client(config: &OAuthClientConfig) -> Result<(), ConnectorError> {
    if !config.client_id.ends_with(".apps.googleusercontent.com") {
        return Err(ConnectorError::InvalidClient);
    }
    Ok(())
}
pub(super) fn authorization_url(
    config: &OAuthClientConfig,
    redirect: &str,
    requested: &[Permission],
    state: &Secret,
    challenge: &str,
) -> Result<Zeroizing<String>, ConnectorError> {
    validate_client(config)?;
    let mut url =
        url::Url::parse(AUTHORIZATION_ENDPOINT).map_err(|_| ConnectorError::InvalidClient)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", redirect),
        ("scope", scopes(requested).join(" ").as_str()),
        ("state", state.expose()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ]);
    Ok(Zeroizing::new(url.into()))
}

pub(super) fn permission_scopes(permission: Permission) -> &'static [&'static str] {
    match permission {
        Permission::MailRead => &["https://www.googleapis.com/auth/gmail.readonly"],
        Permission::MailDraft => &["https://www.googleapis.com/auth/gmail.compose"],
        Permission::MailSend => &["https://www.googleapis.com/auth/gmail.send"],
        Permission::CalendarRead => &["https://www.googleapis.com/auth/calendar.events.readonly"],
        Permission::CalendarEventCreate | Permission::CalendarEventUpdate => {
            &["https://www.googleapis.com/auth/calendar.events.owned"]
        }
        Permission::ContactsRead => &["https://www.googleapis.com/auth/contacts.readonly"],
    }
}
pub(super) fn scopes(requested: &[Permission]) -> Vec<&'static str> {
    requested
        .iter()
        .flat_map(|permission| permission_scopes(*permission).iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(super) fn granted_permissions(
    raw: &str,
    requested: &[Permission],
) -> Result<Vec<Permission>, ConnectorError> {
    let requested = permissions(requested.to_vec())?;
    if raw.is_empty() || raw.len() > 2048 || !raw.is_ascii() || raw.chars().any(char::is_control) {
        return Err(ConnectorError::InvalidPermissions);
    }
    let allowed: BTreeSet<_> = scopes(&requested).into_iter().collect();
    let mut granted = BTreeSet::new();
    for value in raw.split(' ') {
        if !allowed.contains(value) || !granted.insert(value) {
            return Err(ConnectorError::InvalidPermissions);
        }
    }
    // Provider scopes can authorize MORE than a GHOST action. Never infer additional GHOST
    // permissions: gmail.compose also sends; calendar events scopes also allow deletion.
    permissions(
        requested
            .into_iter()
            .filter(|permission| {
                permission_scopes(*permission)
                    .iter()
                    .all(|scope| granted.contains(scope))
            })
            .collect(),
    )
}

pub(crate) struct OAuthTokenResponse {
    pub(super) access_token: Secret,
    pub(super) refresh_token: Option<Secret>,
    pub(super) permissions: Vec<Permission>,
    expires_in: Duration,
    refresh_expires_in: Option<Duration>,
}
impl OAuthTokenResponse {
    pub(crate) fn authorization_header(
        &self,
    ) -> Result<reqwest::header::HeaderValue, ConnectorError> {
        let raw = Zeroizing::new(format!("Bearer {}", self.access_token.expose()));
        let mut header = reqwest::header::HeaderValue::from_str(&raw)
            .map_err(|_| ConnectorError::InvalidTokenResponse)?;
        header.set_sensitive(true);
        Ok(header)
    }
}
fn deserialize_secret<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Secret, D::Error> {
    Secret::new(String::deserialize(deserializer)?)
        .map_err(|_| serde::de::Error::custom("invalid_oauth_secret"))
}
fn deserialize_optional_secret<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Secret>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(Secret::new)
        .transpose()
        .map_err(|_| serde::de::Error::custom("invalid_oauth_secret"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenFields {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: Secret,
    #[serde(default, deserialize_with = "deserialize_optional_secret")]
    refresh_token: Option<Secret>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
    refresh_token_expires_in: Option<u64>,
}
pub(crate) struct TokenHttpResponse {
    status: u16,
    body: Zeroizing<Vec<u8>>,
}
impl TokenHttpResponse {
    fn read(status: u16, length: Option<u64>, body: impl Read) -> Result<Self, ConnectorError> {
        if status != 200 {
            return Err(ConnectorError::Provider);
        }
        if length.is_some_and(|size| size > MAX_RESPONSE_BYTES as u64) {
            return Err(ConnectorError::InvalidTokenResponse);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        body.take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ConnectorError::Transport)?;
        if bytes.len() > MAX_RESPONSE_BYTES || length.is_some_and(|size| size != bytes.len() as u64)
        {
            return Err(ConnectorError::InvalidTokenResponse);
        }
        Ok(Self {
            status,
            body: bytes,
        })
    }
    #[cfg(test)]
    pub(super) fn fixture(status: u16, bytes: Vec<u8>) -> Self {
        Self {
            status,
            body: Zeroizing::new(bytes),
        }
    }
}
pub(super) fn parse_token_response(
    response: TokenHttpResponse,
    requested: &[Permission],
    require_refresh: bool,
) -> Result<OAuthTokenResponse, ConnectorError> {
    if response.status != 200 {
        return Err(ConnectorError::Provider);
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(ConnectorError::InvalidTokenResponse);
    }
    let text =
        std::str::from_utf8(&response.body).map_err(|_| ConnectorError::InvalidTokenResponse)?;
    let fields: TokenFields =
        serde_json::from_str(text).map_err(|_| ConnectorError::InvalidTokenResponse)?;
    if !fields.token_type.eq_ignore_ascii_case("Bearer")
        || fields.expires_in == 0
        || fields.expires_in > 86400
        || fields
            .refresh_token_expires_in
            .is_some_and(|value| value == 0 || value > 315360000)
    {
        return Err(ConnectorError::InvalidTokenResponse);
    }
    let requested = permissions(requested.to_vec())?;
    // RFC 6749: omitted scope means identical to the requested scope, never an expanded grant.
    let granted = match fields.scope {
        Some(scope) => granted_permissions(&scope, &requested)?,
        None => requested,
    };
    if require_refresh && fields.refresh_token.is_none() {
        return Err(ConnectorError::RefreshTokenRequired);
    }
    Ok(OAuthTokenResponse {
        access_token: fields.access_token,
        refresh_token: fields.refresh_token,
        permissions: granted,
        expires_in: Duration::from_secs(fields.expires_in),
        refresh_expires_in: fields.refresh_token_expires_in.map(Duration::from_secs),
    })
}

pub(crate) struct TokenRequest {
    form: Zeroizing<String>,
    requested: Vec<Permission>,
    require_refresh: bool,
}
impl TokenRequest {
    fn form(
        config: &OAuthClientConfig,
        secret: Option<&Secret>,
        values: &[(&str, &str)],
    ) -> Zeroizing<String> {
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        encoded.append_pair("client_id", &config.client_id);
        if let Some(secret) = secret {
            encoded.append_pair("client_secret", secret.expose());
        }
        encoded.extend_pairs(values.iter().copied());
        Zeroizing::new(encoded.finish())
    }
    pub(crate) fn execute(
        self,
        transport: impl FnOnce(Self) -> Result<TokenHttpResponse, ConnectorError>,
    ) -> Result<OAuthTokenResponse, ConnectorError> {
        let requested = self.requested.clone();
        let require_refresh = self.require_refresh;
        let response = transport(self)?;
        parse_token_response(response, &requested, require_refresh)
    }
}
impl OAuthExchange {
    pub(crate) fn token_request<S: CredentialStore>(
        self,
        broker: &CredentialBroker<S>,
    ) -> Result<TokenRequest, ConnectorError> {
        validate_client(&self.config)?;
        let secret = self
            .config
            .client_secret
            .map(|id| broker.get(&id))
            .transpose()?;
        let form = TokenRequest::form(
            &self.config,
            secret.as_ref(),
            &[
                ("grant_type", "authorization_code"),
                ("code", self.code.expose()),
                ("code_verifier", self.verifier.expose()),
                ("redirect_uri", self.redirect.uri()),
            ],
        );
        Ok(TokenRequest {
            form,
            requested: self.requested,
            require_refresh: true,
        })
    }
}
pub(crate) fn refresh_request<S: CredentialStore>(
    config: &OAuthClientConfig,
    account: &Account,
    broker: &CredentialBroker<S>,
) -> Result<TokenRequest, ConnectorError> {
    validate_client(config)?;
    if !account
        .permissions()
        .iter()
        .all(|permission| account.allows(*permission))
    {
        return Err(ConnectorError::InvalidAccount);
    }
    let refresh = broker.get(&CredentialId::new(
        account.provider(),
        account.id(),
        CredentialKind::OAuthRefreshToken,
    ))?;
    let client_secret = config.client_secret.map(|id| broker.get(&id)).transpose()?;
    let form = TokenRequest::form(
        config,
        client_secret.as_ref(),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
        ],
    );
    Ok(TokenRequest {
        form,
        requested: account.permissions().to_vec(),
        require_refresh: false,
    })
}

// reqwest owns this reader, so its original request payload clears on drop as well.
// TLS/HTTP/framework internal copies cannot be guaranteed zeroized by this application.
struct SecretBody {
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
}
impl Read for SecretBody {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        let count = target.len().min(self.bytes.len() - self.offset);
        target[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
fn client() -> Result<Client, ConnectorError> {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| ConnectorError::Transport)
}
pub(crate) fn send_token_request(
    request: TokenRequest,
) -> Result<TokenHttpResponse, ConnectorError> {
    let length = request.form.len();
    let body = SecretBody {
        bytes: Zeroizing::new(request.form.as_bytes().to_vec()),
        offset: 0,
    };
    let response = client()?
        .post(TOKEN_ENDPOINT)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(Body::sized(body, length as u64))
        .send()
        .map_err(|_| ConnectorError::Transport)?;
    TokenHttpResponse::read(
        response.status().as_u16(),
        response.content_length(),
        response,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::oauth::{LoopbackRedirect, OAuthPendingFlow};
    use crate::credentials::tests::FakeStore;
    use crate::credentials::{AccountId, CredentialError, Provider};

    fn response() -> Vec<u8> {
        br#"{"access_token":"synthetic-access-value","refresh_token":"synthetic-refresh-value","expires_in":3600,"token_type":"Bearer","scope":"https://www.googleapis.com/auth/gmail.readonly"}"#.to_vec()
    }
    #[test]
    fn every_permission_maps_to_exact_minimum_scope() {
        let expected = [
            "gmail.readonly",
            "gmail.compose",
            "gmail.send",
            "calendar.events.readonly",
            "calendar.events.owned",
            "calendar.events.owned",
            "contacts.readonly",
        ];
        for (permission, suffix) in Permission::ALL.into_iter().zip(expected) {
            assert_eq!(
                permission_scopes(permission),
                &[format!("https://www.googleapis.com/auth/{suffix}")]
            );
        }
        let mut reversed = Permission::ALL.to_vec();
        reversed.reverse();
        assert_eq!(scopes(&Permission::ALL), scopes(&reversed));
        assert_eq!(scopes(&Permission::ALL).len(), 6);
        assert!(!scopes(&Permission::ALL)
            .iter()
            .any(|scope| scope.contains("mail.google.com")
                || *scope == "https://www.googleapis.com/auth/calendar"));
    }
    #[test]
    fn grants_never_infer_actions_from_provider_scope_overlap() {
        let raw = permission_scopes(Permission::CalendarEventCreate)[0];
        assert_eq!(
            granted_permissions(raw, &[Permission::CalendarEventCreate]).unwrap(),
            vec![Permission::CalendarEventCreate]
        );
        assert_eq!(
            granted_permissions(
                permission_scopes(Permission::MailDraft)[0],
                &[Permission::MailDraft]
            )
            .unwrap(),
            vec![Permission::MailDraft]
        );
        for raw in ["https://mail.google.com/", "https://www.googleapis.com/auth/calendar", "unknown",
            "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send",
            "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.readonly"] {
            assert!(granted_permissions(raw, &[Permission::MailRead]).is_err());
        }
        assert!(permissions(vec![]).is_err());
        assert!(permissions(vec![Permission::MailRead, Permission::MailRead]).is_err());
        assert_eq!(
            granted_permissions(
                permission_scopes(Permission::MailRead)[0],
                &[Permission::MailRead, Permission::ContactsRead]
            )
            .unwrap(),
            vec![Permission::MailRead]
        );
    }
    #[test]
    fn token_success_and_refresh_requirement() {
        let tokens = parse_token_response(
            TokenHttpResponse::fixture(200, response()),
            &[Permission::MailRead],
            true,
        )
        .unwrap();
        assert_eq!(tokens.access_token.expose(), "synthetic-access-value");
        assert_eq!(tokens.expires_in, Duration::from_secs(3600));
        let header = tokens.authorization_header().unwrap();
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains("synthetic-access-value"));
        let value =
            br#"{"access_token":"synthetic-access-value","expires_in":3600,"token_type":"Bearer"}"#
                .to_vec();
        assert!(matches!(
            parse_token_response(
                TokenHttpResponse::fixture(200, value.clone()),
                &[Permission::MailRead],
                true
            ),
            Err(ConnectorError::RefreshTokenRequired)
        ));
        assert!(parse_token_response(
            TokenHttpResponse::fixture(200, value),
            &[Permission::MailRead],
            false
        )
        .is_ok());
    }
    #[test]
    fn malformed_token_responses_and_provider_errors_are_redacted() {
        let mut cases = vec![
            vec![255],
            b"not JSON synthetic-access-value".to_vec(),
            vec![b'x'; MAX_RESPONSE_BYTES + 1],
            br#"{"error":"synthetic-access-value","error_description":"private"}"#.to_vec(),
        ];
        let original: serde_json::Value = serde_json::from_slice(&response()).unwrap();
        for field in ["access_token", "expires_in", "token_type"] {
            let mut value = original.clone();
            value.as_object_mut().unwrap().remove(field);
            cases.push(serde_json::to_vec(&value).unwrap());
        }
        for (field, value) in [
            ("access_token", serde_json::json!("")),
            ("refresh_token", serde_json::json!("bad\nvalue")),
            ("expires_in", serde_json::json!(0)),
            ("expires_in", serde_json::json!(86401)),
            ("token_type", serde_json::json!("unknown")),
            ("id_token", serde_json::json!("synthetic-access-value")),
            ("scope", serde_json::json!("https://mail.google.com/")),
        ] {
            let mut changed = original.clone();
            changed[field] = value;
            cases.push(serde_json::to_vec(&changed).unwrap());
        }
        cases.push(br#"{"access_token":"synthetic-access-value","access_token":"other","expires_in":3600,"token_type":"Bearer"}"#.to_vec());
        for body in cases {
            let error = parse_token_response(
                TokenHttpResponse::fixture(200, body),
                &[Permission::MailRead],
                true,
            )
            .err()
            .unwrap();
            assert!(!error.to_string().contains("synthetic-access-value"));
            assert!(!format!("{error:?}").contains("synthetic-access-value"));
        }
        assert!(matches!(
            parse_token_response(
                TokenHttpResponse::fixture(400, response()),
                &[Permission::MailRead],
                true
            ),
            Err(ConnectorError::Provider)
        ));
    }
    #[test]
    fn transport_reads_are_bounded_and_framing_is_checked() {
        assert!(
            TokenHttpResponse::read(200, Some(MAX_RESPONSE_BYTES as u64 + 1), &b""[..]).is_err()
        );
        assert!(
            TokenHttpResponse::read(200, None, vec![0; MAX_RESPONSE_BYTES + 1].as_slice()).is_err()
        );
        assert!(TokenHttpResponse::read(200, Some(20), &b"short"[..]).is_err());
        struct NeverRead;
        impl Read for NeverRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("provider error body must not be read")
            }
        }
        assert!(matches!(
            TokenHttpResponse::read(400, None, NeverRead),
            Err(ConnectorError::Provider)
        ));
    }
    #[test]
    fn exchange_and_refresh_use_broker_and_single_injected_transport() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let config =
            OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), None).unwrap();
        let (url, pending) = OAuthPendingFlow::new(
            config,
            LoopbackRedirect::from_bound_listener(&listener).unwrap(),
            vec![Permission::MailRead],
        )
        .unwrap();
        let url = url::Url::parse(url.url()).unwrap();
        let fields: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        let callback = format!(
            "{}?state={}&code=synthetic-code",
            fields["redirect_uri"], fields["state"]
        );
        let exchange = pending
            .accept_callback(&callback, std::time::Instant::now())
            .unwrap();
        let mut broker = CredentialBroker::new(FakeStore::default());
        let result = exchange
            .token_request(&broker)
            .unwrap()
            .execute(|request| {
                assert!(request.form.contains("code=synthetic-code"));
                assert!(request.form.contains("code_verifier="));
                assert!(request.form.contains("redirect_uri="));
                assert!(!request.form.contains("refresh_token="));
                Ok(TokenHttpResponse::fixture(200, response()))
            })
            .unwrap();
        let id = AccountId::new();
        broker
            .put(
                &CredentialId::new(Provider::Google, id, CredentialKind::OAuthRefreshToken),
                result.refresh_token.as_ref().unwrap(),
            )
            .unwrap();
        let account = Account::new(
            id,
            Provider::Google,
            "Synthetic".into(),
            None,
            result.permissions,
            100,
        )
        .unwrap();
        let config =
            OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), None).unwrap();
        let request = refresh_request(&config, &account, &broker).unwrap();
        assert!(request.form.contains("grant_type=refresh_token"));
        assert!(request
            .form
            .contains("refresh_token=synthetic-refresh-value"));
        assert!(!request.form.contains("code_verifier"));
        assert!(!request.form.contains("code="));
        broker
            .delete(&CredentialId::new(
                Provider::Google,
                id,
                CredentialKind::OAuthRefreshToken,
            ))
            .unwrap();
        assert!(matches!(
            refresh_request(&config, &account, &broker),
            Err(ConnectorError::Credential(CredentialError::Missing))
        ));
    }
    #[test]
    fn optional_client_secret_must_be_loaded_from_broker() {
        let id = CredentialId::new(
            Provider::Google,
            AccountId::new(),
            CredentialKind::OAuthClientSecret,
        );
        let config =
            OAuthClientConfig::new("synthetic.apps.googleusercontent.com".into(), Some(id))
                .unwrap();
        let account = Account::new(
            AccountId::new(),
            Provider::Google,
            "Synthetic".into(),
            None,
            vec![Permission::MailRead],
            100,
        )
        .unwrap();
        let mut broker = CredentialBroker::new(FakeStore::default());
        let refresh = CredentialId::new(
            Provider::Google,
            account.id(),
            CredentialKind::OAuthRefreshToken,
        );
        broker
            .put(
                &refresh,
                &Secret::new("synthetic-refresh-value".into()).unwrap(),
            )
            .unwrap();
        assert!(refresh_request(&config, &account, &broker).is_err());
        broker
            .put(&id, &Secret::new("synthetic-client-value".into()).unwrap())
            .unwrap();
        let request = refresh_request(&config, &account, &broker).unwrap();
        assert!(request
            .form
            .contains("client_secret=synthetic-client-value"));
    }

    #[test]
    fn failing_exchange_invokes_transport_once_without_retry() {
        let request = TokenRequest {
            form: Zeroizing::new("synthetic-payload".into()),
            requested: vec![Permission::MailRead],
            require_refresh: true,
        };
        let calls = std::cell::Cell::new(0);
        assert!(matches!(
            request.execute(|_| {
                calls.set(calls.get() + 1);
                Err(ConnectorError::Transport)
            }),
            Err(ConnectorError::Transport)
        ));
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn provider_specific_config_rejected_before_credential_lookup() {
        let config = OAuthClientConfig::new("provider-neutral-client".into(), None).unwrap();
        let account = Account::new(
            AccountId::new(),
            Provider::Google,
            "Synthetic".into(),
            None,
            vec![Permission::MailRead],
            100,
        )
        .unwrap();
        assert!(matches!(
            refresh_request(
                &config,
                &account,
                &CredentialBroker::new(FakeStore::default())
            ),
            Err(ConnectorError::InvalidClient)
        ));
    }
}
