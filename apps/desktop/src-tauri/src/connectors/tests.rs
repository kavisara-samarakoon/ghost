use super::*;
use crate::credentials::{tests::FakeStore, Secret};
use accounts::{AccountFile, MAX_ACCOUNTS, MAX_ACCOUNT_BYTES};

fn account() -> Account {
    Account::new(
        AccountId::new(),
        Provider::Google,
        "Synthetic account".into(),
        Some("synthetic@example.invalid".into()),
        vec![Permission::MailRead],
        100,
    )
    .unwrap()
}
fn tokens() -> google::OAuthTokenResponse {
    google::parse_token_response(google::TokenHttpResponse::fixture(200,
        br#"{"access_token":"synthetic-access-value","refresh_token":"synthetic-refresh-value","expires_in":3600,"token_type":"Bearer","scope":"https://www.googleapis.com/auth/gmail.readonly"}"#.to_vec()),
        &[Permission::MailRead],true).unwrap()
}
#[test]
fn account_schema_rejects_secret_and_unknown_fields() {
    for name in [
        "access_token",
        "refresh_token",
        "client_secret",
        "code",
        "code_verifier",
        "state",
        "cookies",
        "raw_response",
        "unexpected",
    ] {
        let mut value = serde_json::to_value(account()).unwrap();
        value[name] = serde_json::json!("synthetic-only-value");
        assert!(serde_json::from_value::<Account>(value).is_err());
    }
    let mut value = serde_json::to_value(account()).unwrap();
    value["provider"] = serde_json::json!("arbitrary");
    assert!(serde_json::from_value::<Account>(value).is_err());
    let mut value = serde_json::to_value(account()).unwrap();
    value["granted_permissions"] = serde_json::json!(["https://mail.google.com/"]);
    assert!(serde_json::from_value::<Account>(value).is_err());
}
#[test]
fn account_field_and_timestamp_bounds() {
    for label in [
        "".into(),
        "x".repeat(129),
        "leading ".into(),
        "control\nvalue".into(),
        "password=synthetic-value".into(),
    ] {
        assert!(Account::new(
            AccountId::new(),
            Provider::Google,
            label,
            None,
            vec![Permission::MailRead],
            100
        )
        .is_err());
    }
    for email in [
        "missing-at",
        "@domain.invalid",
        "synthetic@",
        "a@b@c.invalid",
        "a\n@b.invalid",
    ] {
        assert!(Account::new(
            AccountId::new(),
            Provider::Google,
            "Synthetic".into(),
            Some(email.into()),
            vec![Permission::MailRead],
            100
        )
        .is_err());
    }
    assert!(Account::new(
        AccountId::new(),
        Provider::Google,
        "Synthetic".into(),
        None,
        vec![Permission::MailRead],
        0
    )
    .is_err());
    assert!(Account::new(
        AccountId::new(),
        Provider::Google,
        "Synthetic".into(),
        None,
        vec![Permission::MailRead],
        253402300800
    )
    .is_err());
    let current = account();
    assert!(current
        .updated(
            "Synthetic".into(),
            None,
            vec![Permission::MailRead],
            AccountStatus::Connected,
            99
        )
        .is_err());
}
#[test]
fn account_document_count_duplicates_and_deterministic_order() {
    let first = account();
    let mut duplicate = AccountFile {
        version: 1,
        accounts: vec![first.clone(), first],
    };
    assert!(duplicate.bytes().is_err());
    let mut excessive = AccountFile {
        version: 1,
        accounts: (0..=MAX_ACCOUNTS).map(|_| account()).collect(),
    };
    assert!(excessive.bytes().is_err());
    let mut file = AccountFile {
        version: 1,
        accounts: vec![account(), account(), account()],
    };
    let bytes = file.bytes().unwrap();
    file.accounts.reverse();
    assert_eq!(bytes, file.bytes().unwrap());
}

#[derive(Default)]
struct FakeAccounts {
    values: Vec<Account>,
    error: Option<ConnectorError>,
    adds: usize,
}
impl AccountRepository for FakeAccounts {
    fn list(&self) -> Result<Vec<Account>, ConnectorError> {
        Ok(self.values.clone())
    }
    fn get(&self, id: AccountId) -> Result<Option<Account>, ConnectorError> {
        Ok(self.values.iter().find(|a| a.id() == id).cloned())
    }
    fn add(&mut self, account: Account) -> Result<(), ConnectorError> {
        self.adds += 1;
        if self.error == Some(ConnectorError::ReconciliationRequired) {
            self.values.push(account);
            return Err(ConnectorError::ReconciliationRequired);
        }
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.values.iter().any(|a| a.id() == account.id()) {
            return Err(ConnectorError::DuplicateAccount);
        }
        self.values.push(account);
        Ok(())
    }
    fn update(&mut self, _: Account) -> Result<(), ConnectorError> {
        Err(ConnectorError::Unavailable)
    }
    fn remove(&mut self, id: AccountId) -> Result<(), ConnectorError> {
        self.values.retain(|a| a.id() != id);
        Ok(())
    }
}
#[test]
fn finalize_keychain_and_metadata_success() {
    let mut broker = CredentialBroker::new(FakeStore::default());
    let mut accounts = FakeAccounts::default();
    let id = AccountId::new();
    let view = finalize_connection(
        &mut broker,
        &mut accounts,
        id,
        "Synthetic".into(),
        None,
        tokens(),
        100,
    )
    .unwrap();
    assert_eq!(view.status, AccountStatus::Connected);
    let key = CredentialId::new(Provider::Google, id, CredentialKind::OAuthRefreshToken);
    assert!(broker.contains(&key).unwrap());
    assert_eq!(accounts.values.len(), 1);
    assert_eq!(accounts.adds, 1);
}
#[test]
fn keychain_failure_prevents_connected_metadata() {
    let mut store = FakeStore::default();
    store.fail_put = true;
    let mut broker = CredentialBroker::new(store);
    let mut accounts = FakeAccounts::default();
    assert!(finalize_connection(
        &mut broker,
        &mut accounts,
        AccountId::new(),
        "Synthetic".into(),
        None,
        tokens(),
        100
    )
    .is_err());
    assert_eq!(accounts.adds, 0);
    assert!(accounts.values.is_empty());
}
#[test]
fn metadata_failure_cleans_new_keychain_secret() {
    let mut broker = CredentialBroker::new(FakeStore::default());
    let mut accounts = FakeAccounts {
        error: Some(ConnectorError::Storage),
        ..Default::default()
    };
    let id = AccountId::new();
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            id,
            "Synthetic".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::Storage)
    ));
    assert!(!broker
        .contains(&CredentialId::new(
            Provider::Google,
            id,
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
    assert!(accounts.values.is_empty());
}
#[test]
fn cleanup_failure_returns_reconciliation_required() {
    let mut store = FakeStore::default();
    store.fail_delete = true;
    let mut broker = CredentialBroker::new(store);
    let mut accounts = FakeAccounts {
        error: Some(ConnectorError::Storage),
        ..Default::default()
    };
    let id = AccountId::new();
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            id,
            "Synthetic".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::ReconciliationRequired)
    ));
    assert!(broker
        .contains(&CredentialId::new(
            Provider::Google,
            id,
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
    assert!(accounts.values.is_empty());
}
#[test]
fn ambiguous_metadata_commit_preserves_secret_for_reconciliation() {
    let mut broker = CredentialBroker::new(FakeStore::default());
    let mut accounts = FakeAccounts {
        error: Some(ConnectorError::ReconciliationRequired),
        ..Default::default()
    };
    let id = AccountId::new();
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            id,
            "Synthetic".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::ReconciliationRequired)
    ));
    assert!(broker
        .contains(&CredentialId::new(
            Provider::Google,
            id,
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
    assert_eq!(accounts.values.len(), 1);
}
#[test]
fn duplicate_metadata_and_existing_credential_cannot_be_overwritten() {
    let original = account();
    let id = original.id();
    let mut accounts = FakeAccounts {
        values: vec![original],
        ..Default::default()
    };
    let mut broker = CredentialBroker::new(FakeStore::default());
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            id,
            "Synthetic".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::DuplicateAccount)
    ));
    assert_eq!(accounts.adds, 0);
    let mut accounts = FakeAccounts::default();
    let key = CredentialId::new(Provider::Google, id, CredentialKind::OAuthRefreshToken);
    broker
        .put(
            &key,
            &Secret::new("existing-synthetic-value".into()).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            id,
            "Synthetic".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::Credential(CredentialError::Conflict))
    ));
    assert_eq!(
        broker.get(&key).unwrap().expose(),
        "existing-synthetic-value"
    );
    assert_eq!(accounts.adds, 0);
}
#[test]
fn finalization_rejects_token_echo_in_metadata_before_any_write() {
    let mut broker = CredentialBroker::new(FakeStore::default());
    let mut accounts = FakeAccounts::default();
    assert!(matches!(
        finalize_connection(
            &mut broker,
            &mut accounts,
            AccountId::new(),
            "synthetic-access-value".into(),
            None,
            tokens(),
            100
        ),
        Err(ConnectorError::InvalidAccount)
    ));
    assert_eq!(accounts.adds, 0);
}
#[test]
fn registry_account_status_and_audit_serialize_metadata_only() {
    let account = account();
    let broker = CredentialBroker::new(FakeStore::default());
    let view = account_status(&account, &broker).unwrap();
    assert_eq!(view.status, AccountStatus::Disconnected);
    let event = ConnectorAuditEvent {
        timestamp: 100,
        event: ConnectorAuditKind::ConnectionFinalized,
        provider: Provider::Google,
        account_id: account.id(),
        permissions: vec![Permission::MailRead],
        result: ConnectorAuditResult::Completed,
    };
    for json in [
        serde_json::to_string(&registry()).unwrap(),
        serde_json::to_string(&view).unwrap(),
        serde_json::to_string(&event).unwrap(),
    ] {
        for forbidden in [
            "synthetic-access-value",
            "synthetic-refresh-value",
            "client_secret",
            "code_verifier",
            "raw_response",
            "https://www.googleapis.com",
            "synthetic@example.invalid",
        ] {
            assert!(!json.contains(forbidden));
        }
    }
    let fields = serde_json::to_value(view).unwrap();
    assert_eq!(fields.as_object().unwrap().len(), 5);
    assert!(account.allows(Permission::MailRead));
    assert!(!account.allows(Permission::MailSend));
}

#[cfg(unix)]
mod private_storage {
    use super::*;
    use crate::connectors::storage::AccountStore;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    fn home() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap().join("ghost-home");
        (temp, home)
    }
    #[test]
    fn private_account_roundtrip_update_remove_and_modes() {
        let (_temp, home) = home();
        let mut store = AccountStore::open(&home).unwrap();
        let original = account();
        let id = original.id();
        store.add(original.clone()).unwrap();
        assert!(store.get(id).unwrap().unwrap() == original);
        assert_eq!(
            store.add(original.clone()),
            Err(ConnectorError::DuplicateAccount)
        );
        let changed = original
            .updated(
                "Updated synthetic".into(),
                None,
                vec![Permission::ContactsRead],
                AccountStatus::Disconnected,
                101,
            )
            .unwrap();
        store.update(changed.clone()).unwrap();
        assert!(store.get(id).unwrap().unwrap() == changed);
        for (path, mode) in [
            (home.clone(), 0o700),
            (home.join("connectors"), 0o700),
            (home.join("connectors/accounts.json"), 0o600),
            (home.join("connectors/.accounts-lock"), 0o600),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o7777,
                mode
            );
        }
        let bytes = fs::read(home.join("connectors/accounts.json")).unwrap();
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("synthetic-refresh-value"));
        store.remove(id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.remove(id), Err(ConnectorError::AccountMissing));
    }
    #[test]
    fn private_storage_rejects_symlink_hardlink_and_unsafe_modes() {
        for variant in ["symlink", "hardlink", "mode", "directory", "special-mode"] {
            let (temp, home) = home();
            let store = AccountStore::open(&home).unwrap();
            let path = home.join("connectors/accounts.json");
            let outside = temp.path().join("outside");
            fs::write(&outside, b"private synthetic").unwrap();
            match variant {
                "symlink" => symlink(&outside, &path).unwrap(),
                "hardlink" => fs::hard_link(&outside, &path).unwrap(),
                "mode" | "special-mode" => {
                    fs::write(&path, b"{}").unwrap();
                    fs::set_permissions(
                        &path,
                        fs::Permissions::from_mode(if variant == "mode" { 0o644 } else { 0o4600 }),
                    )
                    .unwrap();
                }
                _ => fs::create_dir(&path).unwrap(),
            }
            assert!(matches!(store.list(), Err(ConnectorError::Storage)));
            assert_eq!(fs::read(&outside).unwrap(), b"private synthetic");
        }
    }
    #[test]
    fn unsafe_directory_paths_and_missing_parent_fail_closed() {
        let (temp, home) = home();
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(AccountStore::open(&home).is_err());
        assert!(AccountStore::open(
            &temp
                .path()
                .canonicalize()
                .unwrap()
                .join("absent/ghost-home")
        )
        .is_err());
        assert!(AccountStore::open(std::path::Path::new("relative")).is_err());
        assert!(AccountStore::open(std::path::Path::new("/")).is_err());
        let alias = temp.path().canonicalize().unwrap().join("link");
        symlink(&home, &alias).unwrap();
        assert!(AccountStore::open(&alias).is_err());
        assert!(
            AccountStore::open(&temp.path().canonicalize().unwrap().join(".env/accounts")).is_err()
        );
    }
    #[test]
    fn corrupt_oversized_unknown_and_secret_fields_are_generic_errors() {
        for data in [
            b"not JSON synthetic-only-value".to_vec(),
            vec![b'x'; MAX_ACCOUNT_BYTES + 1],
            br#"{"version":1,"accounts":[],"refresh_token":"synthetic-only-value"}"#.to_vec(),
            br#"{"version":9,"accounts":[]}"#.to_vec(),
            vec![255],
        ] {
            let (_temp, home) = home();
            let store = AccountStore::open(&home).unwrap();
            let path = home.join("connectors/accounts.json");
            fs::write(&path, data).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let error = store.list().err().unwrap();
            assert_eq!(error, ConnectorError::CorruptAccounts);
            assert!(!error.to_string().contains("synthetic-only-value"));
        }
    }
    #[test]
    fn bounded_account_count_preserves_existing_records() {
        let (_temp, home) = home();
        let mut store = AccountStore::open(&home).unwrap();
        for _ in 0..MAX_ACCOUNTS {
            store.add(account()).unwrap();
        }
        assert_eq!(store.add(account()), Err(ConnectorError::AccountLimit));
        assert_eq!(store.list().unwrap().len(), MAX_ACCOUNTS);
    }
    #[test]
    fn staging_failure_and_ambiguous_commit_are_distinguished() {
        for stage in ["opened", "file-synced", "installed", "parent-synced"] {
            let (_temp, home) = home();
            let store = AccountStore::open(&home).unwrap();
            let mut file = AccountFile {
                version: 1,
                accounts: vec![account()],
            };
            let error = store
                .test_write(&mut file, |current| {
                    if current == stage {
                        Err(ConnectorError::Storage)
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            if stage == "opened" || stage == "file-synced" {
                assert_eq!(error, ConnectorError::Storage);
                assert!(store.list().unwrap().is_empty());
            } else {
                assert_eq!(error, ConnectorError::ReconciliationRequired);
                assert_eq!(store.list().unwrap().len(), 1);
            }
        }
    }
    #[test]
    fn directory_replacement_detected_before_publication() {
        let (_temp, home) = home();
        let store = AccountStore::open(&home).unwrap();
        let mut file = AccountFile {
            version: 1,
            accounts: vec![account()],
        };
        let moved = home.with_file_name("moved-home");
        assert_eq!(
            store.test_write(&mut file, |stage| {
                if stage == "opened" {
                    fs::rename(&home, &moved).unwrap();
                    fs::create_dir(&home).unwrap();
                    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
                }
                Ok(())
            }),
            Err(ConnectorError::Storage)
        );
        assert!(!home.join("connectors/accounts.json").exists());
        assert!(!moved.join("connectors/accounts.json").exists());
    }
    #[test]
    fn parallel_writers_do_not_lose_accounts() {
        let (_temp, home) = home();
        AccountStore::open(&home).unwrap();
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let home = home.clone();
                std::thread::spawn(move || {
                    AccountStore::open(&home).unwrap().add(account()).unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(AccountStore::open(&home).unwrap().list().unwrap().len(), 8);
    }
}
