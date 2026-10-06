use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct FakeStore {
    values: HashMap<CredentialId, Secret>,
    pub(crate) fail_put: bool,
    pub(crate) fail_delete: bool,
}
impl CredentialStore for FakeStore {
    fn put(&mut self, id: &CredentialId, secret: &Secret) -> Result<(), CredentialError> {
        if self.fail_put {
            return Err(CredentialError::Store);
        }
        if self.values.contains_key(id) {
            return Err(CredentialError::Conflict);
        }
        self.values
            .insert(*id, Secret::new(secret.expose().to_owned())?);
        Ok(())
    }
    fn get(&self, id: &CredentialId) -> Result<Secret, CredentialError> {
        let value = self.values.get(id).ok_or(CredentialError::Missing)?;
        Secret::new(value.expose().to_owned())
    }
    fn delete(&mut self, id: &CredentialId) -> Result<(), CredentialError> {
        if self.fail_delete {
            return Err(CredentialError::Store);
        }
        self.values.remove(id).ok_or(CredentialError::Missing)?;
        Ok(())
    }
}

// Inference is ambiguous (and compilation fails) if this type gains Serialize.
macro_rules! assert_not_serialize {
    ($type:ty) => {
        const _: fn() = || {
            trait Ambiguous<A> {
                fn check() {}
            }
            impl<T: ?Sized> Ambiguous<()> for T {}
            struct IfSerializable;
            impl<T: ?Sized + serde::Serialize> Ambiguous<IfSerializable> for T {}
            let _ = <$type as Ambiguous<_>>::check;
        };
    };
}
assert_not_serialize!(Secret);
assert_not_serialize!(crate::connectors::oauth::OAuthClientConfig);
assert_not_serialize!(crate::connectors::oauth::OAuthAuthorizationRequest);
assert_not_serialize!(crate::connectors::oauth::OAuthPendingFlow);
assert_not_serialize!(crate::connectors::oauth::OAuthExchange);
assert_not_serialize!(crate::connectors::google::OAuthTokenResponse);
assert_not_serialize!(crate::connectors::google::TokenRequest);
assert_not_serialize!(crate::connectors::google::TokenHttpResponse);

const _: fn() = || {
    trait Ambiguous<A> {
        fn check() {}
    }
    impl<T: ?Sized> Ambiguous<()> for T {}
    struct IfClone;
    impl<T: ?Sized + Clone> Ambiguous<IfClone> for T {}
    let _ = <Secret as Ambiguous<_>>::check;
    let _ = <crate::connectors::oauth::OAuthPendingFlow as Ambiguous<_>>::check;
    let _ = <crate::connectors::oauth::OAuthExchange as Ambiguous<_>>::check;
    let _ = <crate::connectors::google::TokenRequest as Ambiguous<_>>::check;
};

#[test]
fn secret_formatting_and_validation_never_expose_values() {
    let value = Secret::new("synthetic-only-value".into()).unwrap();
    assert_eq!(format!("{value}"), "[REDACTED]");
    assert_eq!(format!("{value:?}"), "Secret([REDACTED])");
    for value in [
        "".into(),
        "contains space".into(),
        "control\nvalue".into(),
        "\0".into(),
        "x".repeat(MAX_SECRET_BYTES + 1),
    ] {
        let error = Secret::new(value).err().unwrap();
        assert_eq!(error.to_string(), "invalid_secret");
    }
}
#[test]
fn typed_ids_are_canonical_and_allowlisted() {
    for value in [
        "",
        "../owner",
        "synthetic@example.invalid",
        "00000000-0000-0000-0000-000000000000",
        "00000000000040008000000000000001",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    ] {
        assert!(AccountId::parse(value).is_err());
    }
    let account = AccountId::parse("00000000-0000-4000-8000-000000000001").unwrap();
    let id = CredentialId::new(Provider::Google, account, CredentialKind::OAuthRefreshToken);
    assert_eq!(
        id.canonical(),
        "v1/google/00000000-0000-4000-8000-000000000001/oauth_refresh_token"
    );
    assert!(!id.is_client_secret(Provider::Google));
    assert!(serde_json::from_str::<Provider>("\"arbitrary\"").is_err());
}
#[test]
fn fake_broker_put_get_delete_missing_and_insert_only() {
    let id = CredentialId::new(
        Provider::Google,
        AccountId::new(),
        CredentialKind::OAuthRefreshToken,
    );
    let mut broker = CredentialBroker::new(FakeStore::default());
    assert!(!broker.contains(&id).unwrap());
    assert!(matches!(broker.get(&id), Err(CredentialError::Missing)));
    broker
        .put(&id, &Secret::new("synthetic-only-value".into()).unwrap())
        .unwrap();
    assert!(broker.contains(&id).unwrap());
    assert_eq!(broker.get(&id).unwrap().expose(), "synthetic-only-value");
    assert_eq!(
        broker.put(
            &id,
            &Secret::new("different-synthetic-value".into()).unwrap()
        ),
        Err(CredentialError::Conflict)
    );
    broker.delete(&id).unwrap();
    assert!(!broker.contains(&id).unwrap());
}
#[test]
fn broker_failures_are_generic() {
    let id = CredentialId::new(
        Provider::Google,
        AccountId::new(),
        CredentialKind::OAuthClientSecret,
    );
    let mut broker = CredentialBroker::new(FakeStore {
        fail_put: true,
        fail_delete: true,
        ..Default::default()
    });
    let error = broker
        .put(&id, &Secret::new("synthetic-only-value".into()).unwrap())
        .unwrap_err();
    assert!(!format!("{error} {error:?}").contains("synthetic-only-value"));
    assert_eq!(broker.delete(&id), Err(CredentialError::Store));
}
#[test]
fn unsupported_backend_never_falls_back_to_files() {
    let mut store = UnavailableStore;
    let id = CredentialId::new(
        Provider::Google,
        AccountId::new(),
        CredentialKind::OAuthRefreshToken,
    );
    assert_eq!(
        store.put(&id, &Secret::new("synthetic-only-value".into()).unwrap()),
        Err(CredentialError::Unavailable)
    );
    assert!(matches!(store.get(&id), Err(CredentialError::Unavailable)));
    assert_eq!(store.delete(&id), Err(CredentialError::Unavailable));
    assert_eq!(store.contains(&id), Err(CredentialError::Unavailable));
}

#[test]
fn cleanup_refuses_a_changed_credential_and_accepts_an_already_missing_one() {
    let id = CredentialId::new(
        Provider::Google,
        AccountId::new(),
        CredentialKind::OAuthRefreshToken,
    );
    let mut broker = CredentialBroker::new(FakeStore::default());
    let inserted = Secret::new("synthetic-inserted-value".into()).unwrap();
    assert_eq!(broker.discard_created(&id, &inserted), Ok(()));
    broker
        .put(
            &id,
            &Secret::new("different-synthetic-value".into()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        broker.discard_created(&id, &inserted),
        Err(CredentialError::Conflict)
    );
    assert_eq!(
        broker.get(&id).unwrap().expose(),
        "different-synthetic-value"
    );
}
