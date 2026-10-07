use super::{api::*, model::*, mutations::*, runtime::*, *};
use crate::connectors::{
    accounts::{Account, AccountRepository},
    google,
    oauth::{LoopbackRedirect, OAuthPendingFlow},
    ConnectorError, Permission,
};
use crate::credentials::{
    tests::FakeStore, AccountId, CredentialBroker, CredentialId, CredentialKind, Provider, Secret,
};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

#[derive(Default)]
struct Disk {
    accounts: Vec<Account>,
    config: Option<GoogleClientConfig>,
    events: RefCell<Vec<Value>>,
    fail_add: bool,
    fail_remove: bool,
    fail_audit: bool,
    fail_completion_audit: bool,
}
impl AccountRepository for Disk {
    fn list(&self) -> std::result::Result<Vec<Account>, ConnectorError> {
        Ok(self.accounts.clone())
    }
    fn get(&self, id: AccountId) -> std::result::Result<Option<Account>, ConnectorError> {
        Ok(self.accounts.iter().find(|a| a.id() == id).cloned())
    }
    fn add(&mut self, a: Account) -> std::result::Result<(), ConnectorError> {
        if self.fail_add {
            return Err(ConnectorError::Storage);
        }
        if self.accounts.iter().any(|v| v.id() == a.id()) {
            return Err(ConnectorError::DuplicateAccount);
        }
        self.accounts.push(a);
        Ok(())
    }
    fn update(&mut self, a: Account) -> std::result::Result<(), ConnectorError> {
        let index = self
            .accounts
            .iter()
            .position(|v| v.id() == a.id())
            .ok_or(ConnectorError::AccountMissing)?;
        self.accounts[index] = a;
        Ok(())
    }
    fn remove(&mut self, id: AccountId) -> std::result::Result<(), ConnectorError> {
        if self.fail_remove {
            return Err(ConnectorError::Storage);
        }
        self.accounts.retain(|a| a.id() != id);
        Ok(())
    }
}
impl DiskStore for Disk {
    fn client_config(&self) -> Result<Option<GoogleClientConfig>> {
        Ok(self.config.clone())
    }
    fn save_client_config(&mut self, c: &GoogleClientConfig) -> Result<()> {
        self.config = Some(c.clone());
        Ok(())
    }
    fn audit(&self, e: &AuditEvent) -> Result<()> {
        if self.fail_completion_audit && matches!(e.result, AuditResult::Completed) {
            return Err("audit_failed");
        }
        if self.fail_audit {
            return Err("audit_failed");
        }
        self.events
            .borrow_mut()
            .push(serde_json::to_value(e).unwrap());
        Ok(())
    }
}

#[test]
fn completed_mutation_is_preserved_when_completion_audit_fails() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    a.disk.fail_completion_audit = true;
    a.transport.replies.push_back(reply(json!({"id":"abc123"})));
    let result = a
        .execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared: prepared.clone(),
        })
        .unwrap();
    assert!(!result.audit_recorded);
    assert_eq!(a.transport.calls.len(), 1);
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("already_used")
    );
}
#[test]
fn expired_or_changed_native_context_cannot_execute() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    let saved = a.runtime.pending.get_mut(&prepared.request_id).unwrap();
    saved.expires_at = now().unwrap() - 1;
    saved.request_sha256 = saved.digest().unwrap();
    let saved = saved.clone();
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: saved.confirmation(),
            prepared: saved
        })
        .err(),
        Some("review_expired")
    );
    let prepared = prepare_send(&mut a);
    a.disk.accounts[0] = a.disk.accounts[0]
        .updated(
            "Updated account".into(),
            None,
            vec![Permission::MailSend],
            crate::connectors::accounts::AccountStatus::Connected,
            101,
        )
        .unwrap();
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("changed_review")
    );
    assert!(a.transport.calls.is_empty());
}
#[test]
fn one_native_operation_guard_and_new_runtime_reject_replay() {
    let state = AssistantState::default();
    let guard = state.inner.try_lock().unwrap();
    assert!(state.inner.try_lock().is_err());
    drop(guard);
    assert!(state.inner.try_lock().is_ok());
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    let mut restarted = Runtime::default();
    let mut b = assistant(&mut restarted, vec![Permission::MailSend]);
    assert_eq!(
        b.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("already_used")
    );
    assert!(b.transport.calls.is_empty());
}
#[test]
fn malformed_freebusy_errors_and_missing_snippet_are_handled_safely() {
    let window = Window {
        start: "2026-10-10T09:00:00Z".into(),
        end: "2026-10-10T10:00:00Z".into(),
    };
    assert_eq!(
        data::free_time(
            &json!({"calendars":{"primary":{"busy":[],"errors":{}}}}),
            &window,
            30
        )
        .err(),
        Some("invalid_response")
    );
    let message = json!({"id":"abc123","threadId":"def456","internalDate":"1700000000000","labelIds":[],"payload":{"headers":[]}});
    assert_eq!(data::mail_metadata(&message, "abc123").unwrap().snippet, "");
}
#[test]
fn mime_sender_date_and_folding_are_native_bound_values() {
    let message = mail();
    let raw = message
        .raw_at(Some("sender@example.invalid"), 1700000000)
        .unwrap();
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, raw).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("From: sender@example.invalid\r\n"));
    assert!(text.contains("Date: Tue, 14 Nov 2023"));
    assert!(message
        .raw_at(
            Some("sender@example.invalid\r\nBcc: other@example.invalid"),
            1700000000
        )
        .is_err());
    for line in text.split("\r\n") {
        assert!(line.len() < 998);
    }
}
#[test]
fn unavailable_keychain_keeps_account_visible_for_recovery() {
    struct Unavailable;
    impl crate::credentials::CredentialStore for Unavailable {
        fn get(
            &self,
            _: &CredentialId,
        ) -> std::result::Result<Secret, crate::credentials::CredentialError> {
            Err(crate::credentials::CredentialError::Unavailable)
        }
        fn put(
            &mut self,
            _: &CredentialId,
            _: &Secret,
        ) -> std::result::Result<(), crate::credentials::CredentialError> {
            Err(crate::credentials::CredentialError::Unavailable)
        }
        fn delete(
            &mut self,
            _: &CredentialId,
        ) -> std::result::Result<(), crate::credentials::CredentialError> {
            Err(crate::credentials::CredentialError::Unavailable)
        }
    }
    let mut runtime = Runtime::default();
    let account = account(vec![Permission::MailRead]);
    let mut a = Assistant {
        runtime: &mut runtime,
        broker: CredentialBroker::new(Unavailable),
        disk: Disk {
            accounts: vec![account],
            config: None,
            ..Default::default()
        },
        transport: Transport::default(),
        budget: 0,
    };
    let status = a.status().unwrap();
    assert!(!status.credential_checks_available);
    assert_eq!(status.accounts.len(), 1);
    assert_eq!(
        status.accounts[0].status,
        crate::connectors::accounts::AccountStatus::Disconnected
    );
}
#[derive(Default)]
struct Transport {
    replies: VecDeque<HttpReply>,
    calls: Vec<(String, String, Option<Value>, Option<String>)>,
    refreshes: usize,
    refresh_error: bool,
    refresh_scope: String,
}
fn reply(value: Value) -> HttpReply {
    HttpReply {
        status: 200,
        body: Zeroizing::new(serde_json::to_vec(&value).unwrap()),
    }
}
fn token_reply(scope: &str) -> google::TokenHttpResponse {
    google::TokenHttpResponse::fixture(200,serde_json::to_vec(&json!({"access_token":"synthetic-access-value","refresh_token":"synthetic-refresh-value","expires_in":3600,"token_type":"Bearer","scope":scope})).unwrap())
}
impl ApiTransport for Transport {
    fn call(&mut self, operation: &ApiOperation, token: &Secret) -> Result<HttpReply> {
        let request = operation.request(&client()?, token)?;
        assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
        let body = request
            .body()
            .and_then(|b| b.as_bytes())
            .map(|b| serde_json::from_slice(b).unwrap());
        self.calls.push((
            request.method().to_string(),
            request.url().to_string(),
            body,
            request
                .headers()
                .get(reqwest::header::IF_MATCH)
                .map(|h| h.to_str().unwrap().into()),
        ));
        self.replies.pop_front().ok_or("transport_failed")
    }
    fn refresh(
        &mut self,
        _: google::TokenRequest,
    ) -> std::result::Result<google::TokenHttpResponse, ConnectorError> {
        self.refreshes += 1;
        if self.refresh_error {
            Err(ConnectorError::Provider)
        } else {
            Ok(token_reply(&self.refresh_scope))
        }
    }
}
fn account(perms: Vec<Permission>) -> Account {
    Account::new(
        AccountId::new(),
        Provider::Google,
        "Synthetic account".into(),
        None,
        perms,
        100,
    )
    .unwrap()
}
fn assistant<'a>(
    runtime: &'a mut Runtime,
    perms: Vec<Permission>,
) -> Assistant<'a, FakeStore, Disk, Transport> {
    let account = account(perms.clone());
    let config = GoogleClientConfig::new("synthetic.apps.googleusercontent.com".into()).unwrap();
    runtime.cache.insert(
        account.id(),
        CachedAccess {
            token: Secret::new("synthetic-access-value".into()).unwrap(),
            expires: Instant::now() + Duration::from_secs(3600),
            granted: perms.clone(),
            context: context(&account, &config.client_id).unwrap(),
        },
    );
    let mut broker = CredentialBroker::new(FakeStore::default());
    broker
        .put(
            &CredentialId::new(
                Provider::Google,
                account.id(),
                CredentialKind::OAuthRefreshToken,
            ),
            &Secret::new("synthetic-refresh-value".into()).unwrap(),
        )
        .unwrap();
    Assistant {
        runtime,
        broker,
        disk: Disk {
            accounts: vec![account],
            config: Some(config),
            ..Default::default()
        },
        transport: Transport {
            refresh_scope: google::scopes(&perms).join(" "),
            ..Default::default()
        },
        budget: 0,
    }
}
fn mail() -> MailInput {
    MailInput {
        to: vec!["recipient@example.invalid".into()],
        cc: vec![],
        subject: "Synthetic subject".into(),
        body: "Synthetic plain text\nSecond line".into(),
    }
}
fn event_input() -> EventInput {
    let start = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        + chrono::Duration::days(1);
    EventInput {
        summary: "Synthetic event".into(),
        description: Some("Synthetic description".into()),
        location: Some("Synthetic place".into()),
        start: EventTime {
            date_time: Some(iso(start)),
            date: None,
        },
        end: EventTime {
            date_time: Some(iso(start + chrono::Duration::hours(1))),
            date: None,
        },
    }
}
fn provider_event() -> Value {
    let event = event_input();
    json!({"id":"synthetic_event","etag":"\"synthetic-etag\"","summary":event.summary,"description":event.description,"location":event.location,"start":event.start.provider(),"end":event.end.provider(),"status":"confirmed","colorId":"7"})
}
fn changes() -> EventChanges {
    EventChanges {
        summary: Some("Updated synthetic".into()),
        description: None,
        location: None,
        start: None,
        end: None,
    }
}
fn prepare_send(a: &mut Assistant<'_, FakeStore, Disk, Transport>) -> PreparedMutation {
    a.prepare(PrepareInput {
        account_id: a.disk.accounts[0].id(),
        payload: MutationAction::SendMail { mail: mail() },
    })
    .unwrap()
}

#[test]
fn calendar_read_requires_both_exact_scopes() {
    assert_eq!(
        google::permission_scopes(Permission::CalendarRead),
        &[
            "https://www.googleapis.com/auth/calendar.events.readonly",
            "https://www.googleapis.com/auth/calendar.events.freebusy"
        ]
    );
    assert!(google::granted_permissions(
        "https://www.googleapis.com/auth/calendar.events.readonly",
        &[Permission::CalendarRead]
    )
    .is_err());
}
#[test]
fn client_config_is_bounded_versioned_and_rejects_secret_endpoint_fields() {
    for value in [
        "",
        "x.apps.googleusercontent.com\n",
        "https://client.invalid",
        "x",
    ] {
        assert!(GoogleClientConfig::new(value.into()).is_err());
    }
    assert!(
        GoogleClientConfig::new(format!("{}.apps.googleusercontent.com", "x".repeat(300))).is_err()
    );
    for key in ["client_secret", "endpoint", "refresh_token"] {
        let value = json!({"version":1,"client_id":"synthetic.apps.googleusercontent.com",key:"synthetic-only-value"});
        assert!(serde_json::from_value::<GoogleClientConfig>(value).is_err());
    }
}
#[test]
fn callback_http_limits_methods_host_and_query_validation() {
    let listener = connection::listener().unwrap();
    let address = listener.local_addr().unwrap();
    assert_eq!(address.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
    let (url, pending) = OAuthPendingFlow::new(
        GoogleClientConfig::new("synthetic.apps.googleusercontent.com".into())
            .unwrap()
            .oauth()
            .unwrap(),
        LoopbackRedirect::from_bound_listener(&listener).unwrap(),
        vec![Permission::MailRead],
    )
    .unwrap();
    let parsed = url::Url::parse(url.url()).unwrap();
    let fields: std::collections::BTreeMap<_, _> = parsed.query_pairs().collect();
    let state = fields["state"].as_ref();
    let host = format!("127.0.0.1:{}", address.port());
    let valid = format!(
        "GET /oauth/callback?state={state}&code=synthetic-code HTTP/1.1\r\nHost: {host}\r\n\r\n"
    );
    let callback = connection::parse_http(valid.as_bytes(), address.port()).unwrap();
    assert!(pending.validate_callback(&callback, Instant::now()).is_ok());
    for bad in [
        valid.replace("GET ", "POST "),
        valid.replace(&host, "example.invalid"),
        valid.replace("Host:", &format!("Host: {host}\r\nHost:")),
        valid.replace("/oauth/callback?", "/wrong?"),
        valid.replace("HTTP/1.1", "HTTP/2"),
        valid.replace("\r\n\r\n", "\r\nContent-Length: 3\r\n\r\n"),
        "x".repeat(16385),
    ] {
        assert!(connection::parse_http(bad.as_bytes(), address.port()).is_err());
    }
    for query in [
        "state=wrong&code=synthetic-code".into(),
        format!("state={state}&state={state}&code=synthetic-code"),
        format!("state={state}&code=a&code=b"),
        format!("state={state}&code=%GG"),
        format!("state={state}&code=synthetic-code#secret"),
    ] {
        let input = format!("http://{host}/oauth/callback?{query}");
        assert!(pending.validate_callback(&input, Instant::now()).is_err());
    }
    assert_eq!(
        connection::receive(&listener, &pending, Instant::now() - Duration::from_secs(1)).err(),
        Some("timeout")
    );
}
#[test]
fn real_loopback_listener_returns_static_page_without_code() {
    let listener = connection::listener().unwrap();
    let port = listener.local_addr().unwrap().port();
    let (url, pending) = OAuthPendingFlow::new(
        GoogleClientConfig::new("synthetic.apps.googleusercontent.com".into())
            .unwrap()
            .oauth()
            .unwrap(),
        LoopbackRedirect::from_bound_listener(&listener).unwrap(),
        vec![Permission::ContactsRead],
    )
    .unwrap();
    let state = url::Url::parse(url.url())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let writer = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut stream =
            std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(format!("GET /oauth/callback?state={state}&code=synthetic-code HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        assert!(!text.contains("synthetic-code"));
        assert!(!text.contains(&state));
        assert!(text.contains("Authorization received"));
    });
    let callback =
        connection::receive(&listener, &pending, Instant::now() + Duration::from_secs(3)).unwrap();
    pending.accept_callback(&callback, Instant::now()).unwrap();
    writer.join().unwrap();
}
fn connect_input(perms: Vec<Permission>) -> ConnectInput {
    ConnectInput {
        display_label: "Synthetic Google".into(),
        requested_permissions: perms,
        confirmed: true,
    }
}
fn fake_connection(
    a: &mut Assistant<'_, FakeStore, Disk, Transport>,
    perms: Vec<Permission>,
) -> Result<ConnectResult> {
    let callback = RefCell::new(String::new());
    let count = Cell::new(0);
    let scope = google::scopes(&perms).join(" ");
    let result = a.connect(
        connect_input(perms),
        |url| {
            let url = url::Url::parse(url.url()).unwrap();
            let fields: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
            *callback.borrow_mut() = format!(
                "{}?state={}&code=synthetic-code",
                fields["redirect_uri"], fields["state"]
            );
            Ok(())
        },
        |_, _, _| Ok(Zeroizing::new(callback.borrow().clone())),
        |_, _| {
            count.set(count.get() + 1);
            Ok(token_reply(&scope))
        },
    );
    assert_eq!(count.get(), 1);
    result
}
#[test]
fn connection_exchanges_once_and_profile_is_metadata_only() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    a.disk.accounts.clear();
    a.transport
        .replies
        .push_back(reply(json!({"emailAddress":"synthetic@example.invalid"})));
    let result = fake_connection(&mut a, vec![Permission::MailRead]).unwrap();
    assert_eq!(a.transport.calls.len(), 1);
    assert!(a.transport.calls[0].1.contains("/users/me/profile"));
    let metadata = serde_json::to_value(&a.disk.accounts[0]).unwrap();
    assert_eq!(metadata["email"], "synthetic@example.invalid");
    assert!(a.runtime.cache.contains_key(&result.account.account_id));
    let audit = serde_json::to_string(&a.disk.events).unwrap();
    for value in [
        "synthetic@example.invalid",
        "synthetic-code",
        "synthetic-refresh-value",
        "code_verifier",
        "state",
    ] {
        assert!(!audit.contains(value));
    }
}
#[test]
fn contacts_only_connection_does_not_fetch_mail_profile() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::ContactsRead]);
    a.disk.accounts.clear();
    fake_connection(&mut a, vec![Permission::ContactsRead]).unwrap();
    assert!(a.transport.calls.is_empty());
    assert!(serde_json::to_value(&a.disk.accounts[0]).unwrap()["email"].is_null());
}
#[test]
fn browser_failure_closes_listener_without_exchange() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::ContactsRead]);
    a.disk.accounts.clear();
    let port = Cell::new(0);
    let result = a.connect(
        connect_input(vec![Permission::ContactsRead]),
        |url| {
            let parsed = url::Url::parse(url.url()).unwrap();
            let redirect = parsed
                .query_pairs()
                .find(|(k, _)| k == "redirect_uri")
                .unwrap()
                .1
                .into_owned();
            port.set(url::Url::parse(&redirect).unwrap().port().unwrap());
            Err("browser_failed")
        },
        |_, _, _| panic!("no callback after browser failure"),
        |_, _| panic!("no exchange after browser failure"),
    );
    assert_eq!(result.err(), Some("browser_failed"));
    assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port.get())).is_ok());
    assert!(a.disk.accounts.is_empty());
}
#[test]
fn connect_finalization_failure_is_compensated() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::ContactsRead]);
    a.disk.accounts.clear();
    a.disk.fail_add = true;
    assert_eq!(
        fake_connection(&mut a, vec![Permission::ContactsRead]).err(),
        Some("storage_failed")
    );
    assert!(a.disk.accounts.is_empty());
    let ids: Vec<_> = a
        .disk
        .events
        .borrow()
        .iter()
        .filter_map(|v| v["account_id"].as_str().map(str::to_owned))
        .collect();
    let id = AccountId::parse(&ids[0]).unwrap();
    assert!(!a
        .broker
        .contains(&CredentialId::new(
            Provider::Google,
            id,
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
}
#[test]
fn connect_confirmation_and_scope_selection_fail_before_browser() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::ContactsRead]);
    let mut input = connect_input(vec![]);
    input.confirmed = false;
    assert_eq!(
        a.connect(input, |_| panic!(), |_, _, _| panic!(), |_, _| panic!())
            .err(),
        Some("confirmation_required")
    );
    assert_eq!(
        a.connect(
            connect_input(vec![]),
            |_| panic!(),
            |_, _, _| panic!(),
            |_, _| panic!()
        )
        .err(),
        Some("permission_missing")
    );
}
#[test]
fn token_cache_hit_margin_missing_refresh_and_failure() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let account = a.disk.accounts[0].clone();
    a.ensure_token(&account, Permission::MailRead, Instant::now())
        .unwrap();
    assert_eq!(a.transport.refreshes, 0);
    a.runtime.cache.get_mut(&account.id()).unwrap().expires =
        Instant::now() + Duration::from_secs(59);
    a.ensure_token(&account, Permission::MailRead, Instant::now())
        .unwrap();
    assert_eq!(a.transport.refreshes, 1);
    a.runtime.cache.clear();
    a.ensure_token(&account, Permission::MailRead, Instant::now())
        .unwrap();
    assert_eq!(a.transport.refreshes, 2);
    a.runtime.cache.clear();
    a.transport.refresh_error = true;
    assert_eq!(
        a.ensure_token(&account, Permission::MailRead, Instant::now()),
        Err("reconnect_required")
    );
    assert!(a.runtime.cache.is_empty());
    assert!(a
        .broker
        .contains(&CredentialId::new(
            Provider::Google,
            account.id(),
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
}
#[test]
fn unauthorized_api_invalidates_cache_without_retry() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let account = a.disk.accounts[0].clone();
    a.transport.replies.push_back(HttpReply {
        status: 401,
        body: Zeroizing::new(b"synthetic-private-error".to_vec()),
    });
    assert_eq!(
        a.call(&account, Permission::MailRead, &ApiOperation::Profile)
            .err(),
        Some("auth_required")
    );
    assert!(a.runtime.cache.is_empty());
    assert_eq!(a.transport.calls.len(), 1);
    assert_eq!(a.transport.refreshes, 0);
}
#[test]
fn scope_loss_during_refresh_cannot_grant_calendar_read() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::CalendarRead]);
    let account = a.disk.accounts[0].clone();
    a.runtime.cache.clear();
    a.transport.refresh_scope = "https://www.googleapis.com/auth/calendar.events.readonly".into();
    assert!(a
        .ensure_token(&account, Permission::CalendarRead, Instant::now())
        .is_err());
    assert!(a.transport.calls.is_empty());
}
#[test]
fn cache_scope_change_and_expiry_clear_native_memory() {
    let mut runtime = Runtime::default();
    let a = assistant(&mut runtime, vec![Permission::MailRead]);
    let id = a.disk.accounts[0].id();
    a.runtime.cache.get_mut(&id).unwrap().expires = Instant::now() - Duration::from_secs(1);
    a.runtime.prune(Instant::now(), now().unwrap());
    assert!(a.runtime.cache.is_empty());
    a.runtime.bind(std::path::Path::new("/synthetic/one"));
    a.runtime.bind(std::path::Path::new("/synthetic/two"));
    assert!(a.runtime.pending.is_empty());
}
#[test]
fn gmail_query_cap_projection_sanitization_digest_and_no_body() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let id = a.disk.accounts[0].id();
    a.transport.replies.push_back(reply(
        json!({"messages":[{"id":"abc123","threadId":"def456"}],"nextPageToken":"synthetic-page"}),
    ));
    a.transport.replies.push_back(reply(json!({"id":"abc123","threadId":"def456","internalDate":"1700000000000","snippet":"synthetic-access-value <script>sample</script>","labelIds":["UNREAD","IMPORTANT"],"payload":{"headers":[{"name":"From","value":"Synthetic\nSender"},{"name":"Subject","value":"Synthetic subject"}],"body":{"data":"must not be returned"}}})));
    let result = a
        .search(
            MailSearch {
                account_id: id,
                query: String::new(),
                limit: 20,
            },
            false,
        )
        .unwrap();
    assert_eq!(result.messages.len(), 1);
    assert_eq!(result.unread_in_results, 1);
    assert!(result.truncated);
    assert!(result.digest.contains("Synthetic subject"));
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("synthetic-access-value"));
    assert!(!json.contains("must not be returned"));
    assert!(!result.messages[0].from.contains('\n'));
    assert!(a.transport.calls[0].1.contains("includeSpamTrash=false"));
    assert!(a.transport.calls[1].1.contains("format=metadata"));
    let audits = serde_json::to_string(&a.disk.events).unwrap();
    assert!(!audits.contains("Synthetic subject"));
    assert!(!audits.contains("SyntheticSender"));
}
#[test]
fn invalid_search_limits_fail_before_network() {
    for (q, limit) in [
        ("x".repeat(513), 20),
        ("control\nquery".into(), 20),
        ("safe".into(), 0),
        ("safe".into(), 21),
    ] {
        let mut runtime = Runtime::default();
        let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
        let id = a.disk.accounts[0].id();
        assert_eq!(
            a.search(
                MailSearch {
                    account_id: id,
                    query: q,
                    limit
                },
                false
            )
            .err(),
            Some("invalid_input")
        );
        assert!(a.transport.calls.is_empty());
    }
}
#[test]
fn strict_provider_json_utf8_size_duplicates_errors_and_budgets() {
    let key = Secret::new("synthetic-access-value".into()).unwrap();
    for bytes in [
        vec![255],
        b"not JSON".to_vec(),
        b"{\"id\":\"a\",\"id\":\"b\"}".to_vec(),
        vec![b'x'; MAX_RESPONSE_BYTES + 1],
        b"[]".to_vec(),
    ] {
        assert_eq!(
            api::parse(
                HttpReply {
                    status: 200,
                    body: Zeroizing::new(bytes)
                },
                &key,
                &mut 0
            )
            .err(),
            Some("invalid_response")
        );
    }
    for (code, result) in [
        (401, "auth_required"),
        (403, "permission_missing"),
        (412, "conflict"),
        (500, "provider_failed"),
    ] {
        assert_eq!(
            api::parse(
                HttpReply {
                    status: code,
                    body: Zeroizing::new(b"synthetic-access-value".to_vec())
                },
                &key,
                &mut 0
            )
            .err(),
            Some(result)
        );
    }
    let mut budget = MAX_TOTAL_BYTES;
    assert_eq!(
        api::parse(reply(json!({"id":"safe"})), &key, &mut budget).err(),
        Some("invalid_response")
    );
}
#[test]
fn malformed_mail_metadata_and_caps_are_rejected() {
    let v = json!({"id":"abc123","threadId":"def456","internalDate":"bad","snippet":"safe","labelIds":[],"payload":{"headers":[]}});
    assert!(data::mail_metadata(&v, "abc123").is_err());
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let id = a.disk.accounts[0].id();
    a.transport
        .replies
        .push_back(reply(json!({"messages":[{"id":"abc123"},{"id":"def456"}]})));
    assert_eq!(
        a.search(
            MailSearch {
                account_id: id,
                query: String::new(),
                limit: 1
            },
            false
        )
        .err(),
        Some("invalid_response")
    );
}
#[test]
fn mime_header_injection_recipient_subject_body_bounds_and_unicode() {
    let mut value = mail();
    value.to[0] = "a@example.invalid\r\nBcc: attacker@example.invalid".into();
    assert!(value.normalized().is_err());
    let mut value = mail();
    value.to = vec!["a@example.invalid".into(); 6];
    assert!(value.normalized().is_err());
    for subject in ["bad\r\nHeader".into(), "x".repeat(257)] {
        let mut value = mail();
        value.subject = subject;
        assert!(value.normalized().is_err());
    }
    for body in ["x".repeat(32769), "bad\0body".into()] {
        let mut value = mail();
        value.body = body;
        assert!(value.normalized().is_err());
    }
    let mut value = mail();
    value.subject = "Unicode λ title".into();
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        value.raw().unwrap(),
    )
    .unwrap();
    let mime = String::from_utf8(bytes).unwrap();
    assert!(mime.contains("Content-Type: text/plain"));
    assert!(mime.contains("=?UTF-8?B?"));
    assert!(!mime.contains("Bcc:"));
    assert!(!mime.contains("Content-Type: text/html"));
}
#[test]
fn mail_draft_and_send_permissions_do_not_substitute() {
    for permission in [Permission::MailRead, Permission::MailDraft] {
        let mut runtime = Runtime::default();
        let mut a = assistant(&mut runtime, vec![permission]);
        let id = a.disk.accounts[0].id();
        assert_eq!(
            a.prepare(PrepareInput {
                account_id: id,
                payload: MutationAction::SendMail { mail: mail() }
            })
            .err(),
            Some("permission_missing")
        );
        assert!(a.transport.calls.is_empty());
    }
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let id = a.disk.accounts[0].id();
    assert!(a
        .prepare(PrepareInput {
            account_id: id,
            payload: MutationAction::CreateMailDraft { mail: mail() }
        })
        .is_err());
}
#[test]
fn mail_send_preview_digest_confirmation_once_no_replay_and_audit() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    assert_eq!(prepared.preview.mail.as_ref().unwrap().body, mail().body);
    assert_eq!(prepared.request_sha256, prepared.digest().unwrap());
    assert_eq!(
        a.execute(ExecuteInput {
            prepared: prepared.clone(),
            confirmation: "SEND".into()
        })
        .err(),
        Some("changed_review")
    );
    a.transport.replies.push_back(reply(json!({"id":"abc123"})));
    assert!(a
        .execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared: prepared.clone()
        })
        .is_ok());
    assert_eq!(a.transport.calls.len(), 1);
    assert!(a.transport.calls[0].1.contains("/messages/send"));
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("already_used")
    );
    let audit = serde_json::to_string(&a.disk.events).unwrap();
    for secret in [
        "Synthetic subject",
        "recipient@example.invalid",
        "Synthetic plain text",
        "synthetic-access-value",
    ] {
        assert!(!audit.contains(secret));
    }
    assert_eq!(a.disk.events.borrow()[0]["result"], "confirmed");
    assert_eq!(a.disk.events.borrow()[1]["result"], "completed");
}
#[test]
fn changed_payload_preview_hash_account_or_expiry_is_rejected() {
    for change in 0..5 {
        let mut runtime = Runtime::default();
        let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
        let mut prepared = prepare_send(&mut a);
        match change {
            0 => {
                if let MutationAction::SendMail { mail } = &mut prepared.payload {
                    mail.body.push('x');
                }
            }
            1 => prepared.preview.account_label = "changed".into(),
            2 => prepared.request_sha256 = "0".repeat(64),
            3 => prepared.account_id = AccountId::new(),
            _ => prepared.expires_at = 0,
        }
        assert_eq!(
            a.execute(ExecuteInput {
                confirmation: prepared.confirmation(),
                prepared
            })
            .err(),
            Some("changed_review")
        );
        assert!(a.transport.calls.is_empty());
    }
}
#[test]
fn failed_mutation_and_failed_completion_audit_never_replay() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared: prepared.clone()
        })
        .err(),
        Some("transport_failed")
    );
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("already_used")
    );
    assert_eq!(a.transport.calls.len(), 1);
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let prepared = prepare_send(&mut a);
    a.disk.fail_audit = true;
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("audit_failed")
    );
    assert!(a.transport.calls.is_empty());
}
#[test]
fn draft_creates_one_remote_draft_and_never_sends() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailDraft]);
    let id = a.disk.accounts[0].id();
    let prepared = a
        .prepare(PrepareInput {
            account_id: id,
            payload: MutationAction::CreateMailDraft { mail: mail() },
        })
        .unwrap();
    a.transport
        .replies
        .push_back(reply(json!({"id":"synthetic_draft"})));
    a.execute(ExecuteInput {
        confirmation: prepared.confirmation(),
        prepared,
    })
    .unwrap();
    assert_eq!(a.transport.calls.len(), 1);
    assert!(a.transport.calls[0].1.contains("/drafts"));
    assert!(a.transport.calls[0]
        .2
        .as_ref()
        .unwrap()
        .pointer("/message/raw")
        .is_some());
}
#[test]
fn calendar_agenda_dates_caps_and_safe_projection() {
    let event = provider_event();
    let result = data::agenda(&json!({"items":[event.clone()]}), 1).unwrap();
    assert_eq!(result.events.len(), 1);
    assert!(data::agenda(&json!({"items":[event.clone(),event.clone()]}), 1).is_err());
    let mut bad = event;
    bad["start"] = json!({"dateTime":"bad date"});
    assert!(data::event(&bad, false).is_err());
}
#[test]
fn freebusy_merge_clipping_free_slots_invalid_windows() {
    let window = Window {
        start: "2026-10-10T09:00:00Z".into(),
        end: "2026-10-10T12:00:00Z".into(),
    };
    let value = json!({"calendars":{"primary":{"busy":[{"start":"2026-10-10T10:00:00Z","end":"2026-10-10T11:00:00Z"},{"start":"2026-10-10T10:30:00Z","end":"2026-10-10T11:30:00Z"}]}}});
    let free = data::free_time(&value, &window, 30).unwrap();
    assert_eq!(free.busy.len(), 1);
    assert_eq!(free.free.len(), 2);
    assert_eq!(free.candidates.len(), 2);
    assert_eq!(free.candidates[0].end, "2026-10-10T09:30:00Z");
    assert!(data::free_time(&value, &window, 0).is_err());
    assert!(Window {
        start: window.end.clone(),
        end: window.start.clone()
    }
    .normalized()
    .is_err());
    assert!(Window {
        start: "2026-10-01T00:00:00Z".into(),
        end: "2027-10-01T00:00:00Z".into()
    }
    .normalized()
    .is_err());
}
#[test]
fn freebusy_requires_calendar_read_before_network() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::CalendarEventCreate]);
    let id = a.disk.accounts[0].id();
    assert_eq!(
        a.free_time(FreeTimeInput {
            account_id: id,
            window: Window {
                start: "2026-10-10T09:00:00Z".into(),
                end: "2026-10-10T10:00:00Z".into()
            },
            duration_minutes: 30
        })
        .err(),
        Some("permission_missing")
    );
    assert!(a.transport.calls.is_empty());
}
#[test]
fn event_create_exact_preview_allowlist_confirmation_once() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::CalendarEventCreate]);
    let id = a.disk.accounts[0].id();
    let event = event_input();
    let prepared = a
        .prepare(PrepareInput {
            account_id: id,
            payload: MutationAction::CreateCalendarEvent { event },
        })
        .unwrap();
    assert!(prepared.preview.new_event.is_some());
    a.transport
        .replies
        .push_back(reply(json!({"id":"synthetic_event"})));
    a.execute(ExecuteInput {
        confirmation: prepared.confirmation(),
        prepared,
    })
    .unwrap();
    assert_eq!(a.transport.calls.len(), 1);
    let body = a.transport.calls[0].2.as_ref().unwrap();
    for field in [
        "attendees",
        "recurrence",
        "conferenceData",
        "attachments",
        "reminders",
    ] {
        assert!(body.get(field).is_none());
    }
}
#[test]
fn event_dates_all_day_and_unknown_fields() {
    let mut value = event_input();
    value.end = value.start.clone();
    assert!(value
        .normalized(chrono::DateTime::from(std::time::SystemTime::now()))
        .is_err());
    let mut raw = serde_json::to_value(event_input()).unwrap();
    raw["attendees"] = json!([]);
    assert!(serde_json::from_value::<EventInput>(raw).is_err());
    let time = EventTime {
        date_time: Some("2026-10-10T09:00:00Z".into()),
        date: Some("2026-10-10".into()),
    };
    assert!(time.normalized().is_err());
    assert!(EventTime {
        date_time: None,
        date: Some("2026-02-30".into())
    }
    .normalized()
    .is_err());
}
#[test]
fn event_update_etag_old_new_patch_and_remote_conflict() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::CalendarEventUpdate]);
    let id = a.disk.accounts[0].id();
    a.transport.replies.push_back(reply(provider_event()));
    let prepared = a
        .prepare(PrepareInput {
            account_id: id,
            payload: MutationAction::UpdateCalendarEvent {
                event_id: "synthetic_event".into(),
                etag: "\"synthetic-etag\"".into(),
                changes: changes(),
            },
        })
        .unwrap();
    assert!(prepared.preview.old_event.is_some());
    assert_eq!(
        prepared.preview.new_event.as_ref().unwrap().summary,
        "Updated synthetic"
    );
    a.transport.replies.push_back(HttpReply {
        status: 412,
        body: Zeroizing::new(b"private provider body".to_vec()),
    });
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared: prepared.clone()
        })
        .err(),
        Some("conflict")
    );
    assert_eq!(a.transport.calls.len(), 2);
    assert_eq!(a.transport.calls[1].0, "PATCH");
    assert_eq!(
        a.transport.calls[1].3.as_deref(),
        Some("\"synthetic-etag\"")
    );
    assert_eq!(
        a.transport.calls[1].2.as_ref().unwrap(),
        &json!({"summary":"Updated synthetic"})
    );
    assert_eq!(
        a.execute(ExecuteInput {
            confirmation: prepared.confirmation(),
            prepared
        })
        .err(),
        Some("already_used")
    );
}
#[test]
fn event_update_changed_etag_or_guest_recurrence_rejected_before_patch() {
    for kind in ["etag", "attendees", "recurringEventId"] {
        let mut runtime = Runtime::default();
        let mut a = assistant(&mut runtime, vec![Permission::CalendarEventUpdate]);
        let id = a.disk.accounts[0].id();
        let mut event = provider_event();
        event[kind] = match kind {
            "etag" => json!("\"different\""),
            "attendees" => json!([{"email":"synthetic@example.invalid"}]),
            _ => json!("synthetic_series"),
        };
        a.transport.replies.push_back(reply(event));
        assert!(a
            .prepare(PrepareInput {
                account_id: id,
                payload: MutationAction::UpdateCalendarEvent {
                    event_id: "synthetic_event".into(),
                    etag: "\"synthetic-etag\"".into(),
                    changes: changes()
                }
            })
            .is_err());
        assert_eq!(a.transport.calls.len(), 1);
        assert!(a.runtime.pending.is_empty());
    }
}
#[test]
fn contacts_mask_primary_resource_search_and_bounds() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::ContactsRead]);
    let id = a.disk.accounts[0].id();
    a.transport.replies.push_back(reply(json!({"connections":[{"resourceName":"people/synthetic123","names":[{"displayName":"Synthetic contact"}],"emailAddresses":[{"value":"synthetic@example.invalid"}],"phoneNumbers":[{"value":"+000 synthetic"}]}],"nextPageToken":"synthetic-page"})));
    let result = a
        .contacts(ContactsInput {
            account_id: id,
            query: "contact".into(),
        })
        .unwrap();
    assert_eq!(result.contacts.len(), 1);
    assert!(result.truncated);
    assert!(a.transport.calls[0]
        .1
        .starts_with("https://people.googleapis.com/v1/people/me/connections?"));
    assert!(a.transport.calls[0].1.contains("personFields="));
    assert_eq!(a.transport.calls[0].0, "GET");
    assert!(!serde_json::to_string(&a.disk.events)
        .unwrap()
        .contains("Synthetic contact"));
    assert!(data::contacts(&json!({"connections":vec![json!({});101]}), "").is_err());
}
#[test]
fn api_urls_methods_headers_have_no_generic_destination() {
    let key = Secret::new("synthetic-access-value".into()).unwrap();
    let request = ApiOperation::Contacts
        .request(&client().unwrap(), &key)
        .unwrap();
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    for id in ["../other", "https://example.invalid", "a?query=bad", "a/b"] {
        assert!(ApiOperation::EventGet { id: id.into() }
            .request(&client().unwrap(), &key)
            .is_err());
    }
    assert!(serde_json::from_value::<MutationAction>(
        json!({"action":"delete_calendar_event","event_id":"synthetic"})
    )
    .is_err());
}
#[test]
fn disconnect_clears_cache_metadata_credential_and_has_explicit_notice() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let id = a.disk.accounts[0].id();
    assert_eq!(
        a.disconnect(id, "DISCONNECT".into()).err(),
        Some("confirmation_required")
    );
    let result = a.disconnect(id, format!("DISCONNECT {id}")).unwrap();
    assert!(result.local_credentials_removed);
    assert!(result.notice.contains("Google grant may remain"));
    assert!(a.runtime.cache.is_empty());
    assert!(a.disk.accounts.is_empty());
    assert!(!a
        .broker
        .contains(&CredentialId::new(
            Provider::Google,
            id,
            CredentialKind::OAuthRefreshToken
        ))
        .unwrap());
}
#[test]
fn disconnect_failure_keeps_visible_disconnected_metadata() {
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailRead]);
    let id = a.disk.accounts[0].id();
    a.disk.fail_remove = true;
    assert_eq!(
        a.disconnect(id, format!("DISCONNECT {id}")).err(),
        Some("reconciliation_required")
    );
    assert_eq!(a.disk.accounts.len(), 1);
    assert!(!a.disk.accounts[0].allows(Permission::MailRead));
    assert!(a.runtime.cache.is_empty());
}
#[test]
fn main_window_and_metadata_ipc_do_not_expose_secrets() {
    assert_eq!(ipc::main_window("other"), Err("unavailable"));
    let mut runtime = Runtime::default();
    let mut a = assistant(&mut runtime, vec![Permission::MailSend]);
    let status = serde_json::to_string(&a.status().unwrap()).unwrap();
    let prepared = serde_json::to_string(&prepare_send(&mut a)).unwrap();
    for output in [status, prepared] {
        for forbidden in [
            "synthetic-access-value",
            "synthetic-refresh-value",
            "code_verifier",
            "Authorization",
        ] {
            assert!(!output.contains(forbidden));
        }
    }
}

#[cfg(unix)]
mod private_disk {
    use super::*;
    use crate::connectors::storage::AccountStore;
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[test]
    fn configuration_roundtrip_modes_and_no_secret_fields() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap().join("ghost-home");
        let mut store = AccountStore::open(&home).unwrap();
        let config =
            GoogleClientConfig::new("synthetic.apps.googleusercontent.com".into()).unwrap();
        store.save_client_config(&config).unwrap();
        assert_eq!(
            store.client_config().unwrap().unwrap().client_id,
            config.client_id
        );
        assert_eq!(
            std::fs::metadata(home.join("connectors/google-client.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
        let raw = std::fs::read_to_string(home.join("connectors/google-client.json")).unwrap();
        assert!(!raw.contains("client_secret"));
    }
    #[test]
    fn config_rejects_links_corruption_and_change_with_existing_accounts() {
        for variant in ["symlink", "mode", "secret"] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().canonicalize().unwrap().join("ghost-home");
            let store = AccountStore::open(&home).unwrap();
            let path = home.join("connectors/google-client.json");
            match variant {
                "symlink" => {
                    let outside = temp.path().join("outside");
                    std::fs::write(&outside, b"synthetic").unwrap();
                    symlink(&outside, &path).unwrap();
                }
                "mode" => {
                    std::fs::write(&path, b"{}").unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
                _ => {
                    std::fs::write(&path,br#"{"version":1,"client_id":"synthetic.apps.googleusercontent.com","client_secret":"synthetic"}"#).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
            }
            assert!(store.client_config().is_err());
        }
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            AccountStore::open(&temp.path().canonicalize().unwrap().join("ghost-home")).unwrap();
        store
            .save_client_config(
                &GoogleClientConfig::new("synthetic.apps.googleusercontent.com".into()).unwrap(),
            )
            .unwrap();
        store.add(account(vec![Permission::MailRead])).unwrap();
        assert_eq!(
            store.save_client_config(
                &GoogleClientConfig::new("different.apps.googleusercontent.com".into()).unwrap()
            ),
            Err("accounts_exist")
        );
    }
    #[test]
    fn audit_file_is_private_metadata_only_and_refuses_hardlinks() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap().join("ghost-home");
        let store = AccountStore::open(&home).unwrap();
        let event = AuditEvent {
            timestamp: 100,
            provider: Provider::Google,
            account_id: AccountId::new(),
            operation: AuditOperation::SendMail,
            permission: Some(Permission::MailSend),
            result: AuditResult::Confirmed,
            request_sha256: Some("a".repeat(64)),
            recipient_count: Some(1),
        };
        store.audit(&event).unwrap();
        let path = home.join("connectors/google-audit.jsonl");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "subject",
            "body",
            "recipient@example.invalid",
            "refresh_token",
            "access_token",
        ] {
            assert!(!raw.contains(forbidden));
        }
        std::fs::hard_link(&path, temp.path().join("link")).unwrap();
        assert!(store.audit(&event).is_err());
    }
}
