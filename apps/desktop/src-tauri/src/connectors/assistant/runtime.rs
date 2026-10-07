use super::{
    api::{self, ApiOperation, ApiTransport},
    connection, data, map_error,
    model::*,
    mutations::*,
    now, DiskStore, GoogleClientConfig,
};
use crate::connectors::{
    self,
    accounts::{Account, AccountView},
    google,
    oauth::{LoopbackRedirect, OAuthAuthorizationRequest, OAuthPendingFlow},
    Permission,
};
use crate::credentials::{
    AccountId, CredentialBroker, CredentialError, CredentialId, CredentialKind, CredentialStore,
    Provider, Secret,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub struct CachedAccess {
    pub token: Secret,
    pub expires: Instant,
    pub granted: Vec<Permission>,
    pub context: String,
}
#[derive(Default)]
pub struct Runtime {
    pub cache: HashMap<AccountId, CachedAccess>,
    pub pending: HashMap<String, PreparedMutation>,
    pub disconnected: HashSet<AccountId>,
    pub home: Option<std::path::PathBuf>,
}
#[derive(Default)]
pub struct AssistantState {
    pub inner: std::sync::Mutex<Runtime>,
}
impl Runtime {
    pub fn bind(&mut self, home: &std::path::Path) {
        if self.home.as_deref() != Some(home) {
            self.cache.clear();
            self.pending.clear();
            self.disconnected.clear();
            self.home = Some(home.into());
        }
    }
    pub fn prune(&mut self, time: Instant, seconds: u64) {
        self.cache.retain(|_, c| c.expires > time);
        self.pending.retain(|_, p| p.expires_at > seconds);
    }
}
pub struct Assistant<'a, S: CredentialStore, D: DiskStore, T: ApiTransport> {
    pub runtime: &'a mut Runtime,
    pub broker: CredentialBroker<S>,
    pub disk: D,
    pub transport: T,
    pub budget: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectInput {
    pub display_label: String,
    pub requested_permissions: Vec<Permission>,
    pub confirmed: bool,
}
#[derive(Serialize)]
pub struct ConfigStatus {
    pub configured: bool,
    pub client_id: Option<String>,
}
#[derive(Serialize)]
pub struct Status {
    pub credential_checks_available: bool,
    pub config: ConfigStatus,
    pub accounts: Vec<AccountView>,
    pub connectors: [connectors::ConnectorDescriptor; 1],
}
#[derive(Serialize)]
pub struct ConnectResult {
    pub account: AccountView,
    pub audit_recorded: bool,
}
#[derive(Serialize)]
pub struct DisconnectResult {
    pub local_credentials_removed: bool,
    pub notice: &'static str,
    pub audit_recorded: bool,
}
impl<'a, S: CredentialStore, D: DiskStore, T: ApiTransport> Assistant<'a, S, D, T> {
    pub fn status(&mut self) -> Result<Status> {
        let config = self.disk.client_config()?;
        let mut accounts = Vec::new();
        let mut credential_checks_available = true;
        for account in self.disk.list().map_err(map_error)? {
            let mut view = match connectors::account_status(&account, &self.broker) {
                Ok(view) => view,
                Err(_) => {
                    credential_checks_available = false;
                    let mut view = account.view();
                    view.status = connectors::accounts::AccountStatus::Disconnected;
                    view
                }
            };
            if view.status == connectors::accounts::AccountStatus::Disconnected {
                self.runtime.cache.remove(&account.id());
            }
            if self.runtime.disconnected.contains(&account.id()) {
                view.status = connectors::accounts::AccountStatus::Disconnected;
            }
            accounts.push(view);
        }
        Ok(Status {
            credential_checks_available,
            config: ConfigStatus {
                configured: config.is_some(),
                client_id: config.map(|c| c.client_id),
            },
            accounts,
            connectors: connectors::registry(),
        })
    }
    pub fn config(&self) -> Result<GoogleClientConfig> {
        self.disk.client_config()?.ok_or("client_required")
    }
    pub fn account(&self, id: AccountId, permission: Permission) -> Result<Account> {
        let account = self
            .disk
            .get(id)
            .map_err(map_error)?
            .ok_or("auth_required")?;
        if self.runtime.disconnected.contains(&id) {
            return Err("auth_required");
        }
        if !account.allows(permission) {
            return Err("permission_missing");
        }
        Ok(account)
    }
    fn audit(
        &self,
        id: AccountId,
        operation: AuditOperation,
        permission: Option<Permission>,
        result: AuditResult,
        hash: Option<String>,
        recipients: Option<usize>,
    ) -> Result<()> {
        self.disk.audit(&AuditEvent {
            timestamp: now()?,
            provider: Provider::Google,
            account_id: id,
            operation,
            permission,
            result,
            request_sha256: hash,
            recipient_count: recipients,
        })
    }
    pub fn ensure_token(
        &mut self,
        account: &Account,
        required: Permission,
        time: Instant,
    ) -> Result<()> {
        let config = self.config()?;
        let binding = context(account, &config.client_id)?;
        if self.runtime.cache.get(&account.id()).is_some_and(|c| {
            c.expires > time + Duration::from_secs(60)
                && c.context == binding
                && c.granted.contains(&required)
        }) {
            return Ok(());
        }
        self.runtime.cache.remove(&account.id());
        let response = google::refresh_request(&config.oauth()?, account, &self.broker)
            .map_err(map_error)?
            .execute(|request| self.transport.refresh(request))
            .map_err(map_error)?;
        if !response.permissions.contains(&required) {
            return Err("permission_missing");
        }
        if let Some(rotated) = &response.refresh_token {
            let current = self
                .broker
                .get(&CredentialId::new(
                    Provider::Google,
                    account.id(),
                    CredentialKind::OAuthRefreshToken,
                ))
                .map_err(|_| "reconnect_required")?;
            if current.expose() != rotated.expose() {
                return Err("reconnect_required");
            }
        }
        let expires = time
            .checked_add(response.expires_in)
            .ok_or("invalid_response")?;
        self.runtime.cache.insert(
            account.id(),
            CachedAccess {
                token: response.access_token,
                expires,
                granted: response.permissions,
                context: binding,
            },
        );
        Ok(())
    }
    pub fn call(
        &mut self,
        account: &Account,
        permission: Permission,
        operation: &ApiOperation,
    ) -> Result<serde_json::Value> {
        self.ensure_token(account, permission, Instant::now())?;
        let cached = self
            .runtime
            .cache
            .get(&account.id())
            .ok_or("auth_required")?;
        let result = self
            .transport
            .call(operation, &cached.token)
            .and_then(|reply| api::parse(reply, &cached.token, &mut self.budget));
        if matches!(result, Err("auth_required")) {
            self.runtime.cache.remove(&account.id());
        }
        result
    }
    pub fn search(&mut self, input: MailSearch, digest: bool) -> Result<MailResult> {
        input_text(&input.query, 512, false)?;
        if !(1..=MAX_MESSAGES).contains(&input.limit) {
            return Err("invalid_input");
        }
        let account = self.account(input.account_id, Permission::MailRead)?;
        let query = if input.query.trim().is_empty() {
            "in:inbox newer_than:14d".into()
        } else {
            input.query.trim().into()
        };
        let operation = if digest {
            AuditOperation::MailDigest
        } else {
            AuditOperation::MailSearch
        };
        let result = (|| {
            let response = self.call(
                &account,
                Permission::MailRead,
                &ApiOperation::MailList {
                    query,
                    limit: input.limit,
                },
            )?;
            let empty = Vec::new();
            let messages = match response.get("messages") {
                None => &empty,
                Some(v) => v.as_array().ok_or("invalid_response")?,
            };
            if messages.len() > input.limit {
                return Err("invalid_response");
            }
            let mut output = Vec::new();
            let mut seen = HashSet::new();
            for message in messages {
                let id = mail_id(data::string(message, "id")?)?;
                if !seen.insert(id.clone()) {
                    return Err("invalid_response");
                }
                let value = self.call(
                    &account,
                    Permission::MailRead,
                    &ApiOperation::MailMetadata { id: id.clone() },
                )?;
                output.push(data::mail_metadata(&value, &id)?);
            }
            Ok(data::mail_result(
                output,
                response
                    .get("nextPageToken")
                    .is_some_and(serde_json::Value::is_string),
            ))
        })();
        self.audit(
            account.id(),
            operation,
            Some(Permission::MailRead),
            if result.is_ok() {
                AuditResult::Completed
            } else {
                AuditResult::Failed
            },
            None,
            None,
        )?;
        result
    }
    pub fn agenda(&mut self, input: AgendaInput) -> Result<AgendaResult> {
        let window = input.window.normalized()?;
        if !(1..=MAX_EVENTS).contains(&input.limit) {
            return Err("invalid_input");
        }
        let account = self.account(input.account_id, Permission::CalendarRead)?;
        let result = self
            .call(
                &account,
                Permission::CalendarRead,
                &ApiOperation::Agenda {
                    window,
                    limit: input.limit,
                },
            )
            .and_then(|v| data::agenda(&v, input.limit));
        self.audit(
            account.id(),
            AuditOperation::Agenda,
            Some(Permission::CalendarRead),
            if result.is_ok() {
                AuditResult::Completed
            } else {
                AuditResult::Failed
            },
            None,
            None,
        )?;
        result
    }
    pub fn free_time(&mut self, input: FreeTimeInput) -> Result<FreeTimeResult> {
        let window = input.window.normalized()?;
        if !(5..=480).contains(&input.duration_minutes) {
            return Err("invalid_input");
        }
        let account = self.account(input.account_id, Permission::CalendarRead)?;
        let result = self
            .call(
                &account,
                Permission::CalendarRead,
                &ApiOperation::FreeBusy {
                    window: window.clone(),
                },
            )
            .and_then(|v| data::free_time(&v, &window, input.duration_minutes));
        self.audit(
            account.id(),
            AuditOperation::FreeTime,
            Some(Permission::CalendarRead),
            if result.is_ok() {
                AuditResult::Completed
            } else {
                AuditResult::Failed
            },
            None,
            None,
        )?;
        result
    }
    pub fn contacts(&mut self, input: ContactsInput) -> Result<ContactResult> {
        input_text(&input.query, 128, false)?;
        let account = self.account(input.account_id, Permission::ContactsRead)?;
        let result = self
            .call(&account, Permission::ContactsRead, &ApiOperation::Contacts)
            .and_then(|v| data::contacts(&v, &input.query));
        self.audit(
            account.id(),
            AuditOperation::Contacts,
            Some(Permission::ContactsRead),
            if result.is_ok() {
                AuditResult::Completed
            } else {
                AuditResult::Failed
            },
            None,
            None,
        )?;
        result
    }
    pub fn prepare(&mut self, input: PrepareInput) -> Result<PreparedMutation> {
        let seconds = now()?;
        self.runtime.prune(Instant::now(), seconds);
        if self.runtime.pending.len() >= 16 {
            return Err("busy");
        }
        let permission = input.payload.permission();
        let account = self.account(input.account_id, permission)?;
        let mut preview = MutationPreview {
            sender: account.email().map(str::to_owned),
            account_label: account.view().display_label,
            mail: None,
            old_event: None,
            new_event: None,
            body_bytes: None,
        };
        let payload = match input.payload {
            MutationAction::CreateMailDraft { mail } => {
                let mail = mail.normalized()?;
                preview.body_bytes = Some(mail.body.len());
                preview.mail = Some(mail.clone());
                MutationAction::CreateMailDraft { mail }
            }
            MutationAction::SendMail { mail } => {
                let mail = mail.normalized()?;
                preview.body_bytes = Some(mail.body.len());
                preview.mail = Some(mail.clone());
                MutationAction::SendMail { mail }
            }
            MutationAction::CreateCalendarEvent { event } => {
                let event = event.normalized(chrono::DateTime::<chrono::Utc>::from(
                    std::time::SystemTime::now(),
                ))?;
                preview.new_event = Some(event.clone());
                MutationAction::CreateCalendarEvent { event }
            }
            MutationAction::UpdateCalendarEvent {
                event_id,
                etag: expected,
                changes,
            } => {
                identifier(&event_id).map_err(|_| "invalid_input")?;
                etag(&expected).map_err(|_| "invalid_input")?;
                let value = self.call(
                    &account,
                    Permission::CalendarEventUpdate,
                    &ApiOperation::EventGet {
                        id: event_id.clone(),
                    },
                )?;
                let current = data::event(&value, true)?;
                for field in ["attendees", "recurrence"] {
                    match value.get(field) {
                        None | Some(serde_json::Value::Null) => (),
                        Some(serde_json::Value::Array(v)) if v.is_empty() => (),
                        Some(serde_json::Value::Array(_)) => return Err("unsupported_event"),
                        _ => return Err("invalid_response"),
                    }
                }
                match value.get("recurringEventId") {
                    None | Some(serde_json::Value::Null) => (),
                    Some(serde_json::Value::String(_)) => return Err("unsupported_event"),
                    _ => return Err("invalid_response"),
                }
                match value.get("eventType") {
                    None | Some(serde_json::Value::Null) => (),
                    Some(serde_json::Value::String(s)) if s == "default" => (),
                    Some(serde_json::Value::String(_)) => return Err("unsupported_event"),
                    _ => return Err("invalid_response"),
                }
                if !matches!(current.status.as_str(), "confirmed" | "tentative") {
                    return Err("unsupported_event");
                }
                if current.event_id != event_id || current.etag != expected {
                    return Err("conflict");
                }
                let next = changes.apply(
                    &current.fields,
                    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()),
                )?;
                let changes = changes.normalized(&next);
                preview.old_event = Some(current.fields);
                preview.new_event = Some(next);
                MutationAction::UpdateCalendarEvent {
                    event_id,
                    etag: expected,
                    changes,
                }
            }
        };
        let mut prepared = PreparedMutation {
            version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            account_id: account.id(),
            required_permission: permission,
            payload,
            preview,
            created_at: seconds,
            expires_at: seconds + 300,
            context_sha256: context(&account, &self.config()?.client_id)?,
            request_sha256: String::new(),
        };
        prepared.request_sha256 = prepared.digest()?;
        self.runtime
            .pending
            .insert(prepared.request_id.clone(), prepared.clone());
        Ok(prepared)
    }
    pub fn execute(&mut self, input: ExecuteInput) -> Result<MutationResult> {
        let supplied = input.prepared;
        if serde_json::to_vec(&supplied)
            .map_err(|_| "changed_review")?
            .len()
            > 128 * 1024
        {
            return Err("changed_review");
        }
        let saved = self
            .runtime
            .pending
            .get(&supplied.request_id)
            .ok_or("already_used")?;
        if saved != &supplied
            || saved.digest()? != supplied.request_sha256
            || input.confirmation != saved.confirmation()
        {
            return Err("changed_review");
        }
        // Claims are consumed before any external attempt. Failure never makes the same request reusable.
        let saved = self
            .runtime
            .pending
            .remove(&supplied.request_id)
            .ok_or("already_used")?;
        if saved.expires_at <= now()? {
            return Err("review_expired");
        }
        let account = self.account(saved.account_id, saved.required_permission)?;
        if context(&account, &self.config()?.client_id)? != saved.context_sha256 {
            return Err("changed_review");
        }
        let operation = AuditOperation::mutation(&saved.payload);
        let recipients = saved.preview.mail.as_ref().map(|m| m.to.len() + m.cc.len());
        self.audit(
            account.id(),
            operation,
            Some(saved.required_permission),
            AuditResult::Confirmed,
            Some(saved.request_sha256.clone()),
            recipients,
        )?;
        let result = self
            .call(
                &account,
                saved.required_permission,
                &saved.payload.request(account.email(), saved.created_at)?,
            )
            .and_then(|v| {
                let id = match &saved.payload {
                    MutationAction::CreateMailDraft { .. } => identifier(data::string(&v, "id")?),
                    MutationAction::SendMail { .. } => mail_id(data::string(&v, "id")?),
                    _ => identifier(data::string(&v, "id")?),
                }?;
                Ok(id)
            });
        let recorded = self
            .audit(
                account.id(),
                operation,
                Some(saved.required_permission),
                if result.is_ok() {
                    AuditResult::Completed
                } else {
                    AuditResult::Failed
                },
                Some(saved.request_sha256),
                recipients,
            )
            .is_ok();
        result.map(|provider_id| MutationResult {
            operation: saved.payload.operation().into(),
            provider_id,
            audit_recorded: recorded,
        })
    }
    pub fn connect(
        &mut self,
        input: ConnectInput,
        open: impl FnOnce(&OAuthAuthorizationRequest) -> Result<()>,
        receive: impl FnOnce(
            &std::net::TcpListener,
            &OAuthPendingFlow,
            Instant,
        ) -> Result<Zeroizing<String>>,
        exchange: impl FnOnce(
            google::TokenRequest,
            Duration,
        ) -> std::result::Result<
            google::TokenHttpResponse,
            connectors::ConnectorError,
        >,
    ) -> Result<ConnectResult> {
        if !input.confirmed {
            return Err("confirmation_required");
        }
        input_text(&input.display_label, 128, false)?;
        if self.disk.list().map_err(map_error)?.len() >= crate::connectors::accounts::MAX_ACCOUNTS {
            return Err("account_limit");
        }
        let config = self.config()?;
        let permissions =
            connectors::permissions(input.requested_permissions).map_err(map_error)?;
        let account_id = AccountId::new();
        Account::new(
            account_id,
            Provider::Google,
            input.display_label.clone(),
            None,
            permissions.clone(),
            now()?,
        )
        .map_err(map_error)?;
        self.audit(
            account_id,
            AuditOperation::Connect,
            None,
            AuditResult::Confirmed,
            None,
            None,
        )?;
        let result = (|| {
            let listener = connection::listener()?;
            let redirect = LoopbackRedirect::from_bound_listener(&listener).map_err(map_error)?;
            let deadline = Instant::now() + Duration::from_secs(300);
            let (url, pending) =
                OAuthPendingFlow::new(config.oauth()?, redirect, permissions).map_err(map_error)?;
            open(&url)?;
            let callback = receive(&listener, &pending, deadline)?;
            if Instant::now() >= deadline {
                return Err("timeout");
            }
            let flow = pending
                .accept_callback(&callback, Instant::now())
                .map_err(map_error)?;
            let token_time = Instant::now();
            let tokens = flow
                .token_request(&self.broker)
                .map_err(map_error)?
                .execute(|request| {
                    exchange(request, deadline.saturating_duration_since(Instant::now()))
                })
                .map_err(map_error)?;
            if Instant::now() >= deadline {
                return Err("timeout");
            }
            self.transport.set_deadline(Some(deadline));
            let email = if tokens.permissions.contains(&Permission::MailRead) {
                let response = self
                    .transport
                    .call(&ApiOperation::Profile, &tokens.access_token)?;
                let value = api::parse(response, &tokens.access_token, &mut self.budget)?;
                Some(
                    address(data::string(&value, "emailAddress")?)
                        .map_err(|_| "invalid_response")?,
                )
            } else {
                None
            };
            if Instant::now() >= deadline {
                return Err("timeout");
            }
            let view = connectors::finalize_connection_ref(
                &mut self.broker,
                &mut self.disk,
                account_id,
                input.display_label,
                email,
                &tokens,
                now()?,
            )
            .map_err(map_error)?;
            let account = self
                .disk
                .get(account_id)
                .map_err(map_error)?
                .ok_or("reconciliation_required")?;
            self.runtime.cache.insert(
                account_id,
                CachedAccess {
                    token: tokens.access_token,
                    expires: token_time + tokens.expires_in,
                    granted: tokens.permissions,
                    context: context(&account, &config.client_id)?,
                },
            );
            Ok(view)
        })();
        let recorded = self
            .audit(
                account_id,
                AuditOperation::Connect,
                None,
                match &result {
                    Ok(_) => AuditResult::Completed,
                    Err("reconciliation_required") => AuditResult::ReconciliationRequired,
                    _ => AuditResult::Failed,
                },
                None,
                None,
            )
            .is_ok();
        result.map(|account| ConnectResult {
            account,
            audit_recorded: recorded,
        })
    }
    pub fn disconnect(&mut self, id: AccountId, confirmation: String) -> Result<DisconnectResult> {
        if confirmation != format!("DISCONNECT {id}") {
            return Err("confirmation_required");
        }
        let account = self
            .disk
            .get(id)
            .map_err(map_error)?
            .ok_or("auth_required")?;
        self.audit(
            id,
            AuditOperation::Disconnect,
            None,
            AuditResult::Confirmed,
            None,
            None,
        )?;
        self.runtime.cache.remove(&id);
        self.runtime.pending.retain(|_, p| p.account_id != id);
        self.runtime.disconnected.insert(id);
        let result = (|| {
            self.disk
                .update(account.disconnected(now()?).map_err(map_error)?)
                .map_err(|_| "reconciliation_required")?;
            match self.broker.delete(&CredentialId::new(
                Provider::Google,
                id,
                CredentialKind::OAuthRefreshToken,
            )) {
                Ok(()) | Err(CredentialError::Missing) => (),
                Err(_) => return Err("reconciliation_required"),
            }
            self.disk
                .remove(id)
                .map_err(|_| "reconciliation_required")?;
            Ok(())
        })();
        let recorded = self
            .audit(
                id,
                AuditOperation::Disconnect,
                None,
                if result.is_ok() {
                    AuditResult::Completed
                } else {
                    AuditResult::ReconciliationRequired
                },
                None,
                None,
            )
            .is_ok();
        result?;
        self.runtime.disconnected.remove(&id);
        Ok(DisconnectResult{local_credentials_removed:true,notice:"Local credentials removed. The Google grant may remain active until revoked in Google Account settings.",audit_recorded:recorded})
    }
}
