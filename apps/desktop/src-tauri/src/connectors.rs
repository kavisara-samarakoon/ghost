//! Internal connector foundation. No live connection, network action or new IPC command.
#![allow(dead_code)]

pub(crate) mod accounts;
pub(crate) mod assistant;
pub(crate) mod google;
pub(crate) mod oauth;
#[cfg(unix)]
pub(crate) mod storage;

use crate::credentials::{
    AccountId, CredentialBroker, CredentialError, CredentialId, CredentialKind, CredentialStore,
    Provider,
};
use accounts::{Account, AccountRepository, AccountStatus, AccountView};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectorError {
    InvalidAccount,
    DuplicateAccount,
    AccountMissing,
    AccountLimit,
    CorruptAccounts,
    Storage,
    ReconciliationRequired,
    Unavailable,
    Credential(CredentialError),
    InvalidPermissions,
    InvalidClient,
    Randomness,
    InvalidCallback,
    ExpiredFlow,
    ProviderDenied,
    Transport,
    Provider,
    InvalidTokenResponse,
    RefreshTokenRequired,
}
impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidAccount => "invalid_account",
            Self::DuplicateAccount => "account_exists",
            Self::AccountMissing => "account_missing",
            Self::AccountLimit => "account_limit",
            Self::CorruptAccounts => "account_storage_corrupt",
            Self::Storage => "account_storage_failed",
            Self::ReconciliationRequired => "connector_reconciliation_required",
            Self::Unavailable => "connector_unavailable",
            Self::Credential(_) => "connector_credential_failed",
            Self::InvalidPermissions => "invalid_permissions",
            Self::InvalidClient => "invalid_oauth_client",
            Self::Randomness => "oauth_randomness_unavailable",
            Self::InvalidCallback => "invalid_oauth_callback",
            Self::ExpiredFlow => "oauth_flow_expired",
            Self::ProviderDenied => "oauth_provider_denied",
            Self::Transport => "oauth_transport_failed",
            Self::Provider => "oauth_provider_failed",
            Self::InvalidTokenResponse => "invalid_oauth_response",
            Self::RefreshTokenRequired => "oauth_refresh_token_required",
        })
    }
}
impl From<CredentialError> for ConnectorError {
    fn from(error: CredentialError) -> Self {
        Self::Credential(error)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Permission {
    MailRead,
    MailDraft,
    MailSend,
    CalendarRead,
    CalendarEventCreate,
    CalendarEventUpdate,
    ContactsRead,
}
impl Permission {
    pub(crate) const ALL: [Self; 7] = [
        Self::MailRead,
        Self::MailDraft,
        Self::MailSend,
        Self::CalendarRead,
        Self::CalendarEventCreate,
        Self::CalendarEventUpdate,
        Self::ContactsRead,
    ];
}

pub(super) fn permissions(mut values: Vec<Permission>) -> Result<Vec<Permission>, ConnectorError> {
    if values.is_empty() || values.len() > Permission::ALL.len() {
        return Err(ConnectorError::InvalidPermissions);
    }
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ConnectorError::InvalidPermissions);
    }
    Ok(values)
}

#[derive(Serialize)]
pub(crate) struct ConnectorDescriptor {
    provider: Provider,
    display_name: &'static str,
    supported_permissions: [Permission; 7],
    connection_capability: &'static str,
    credential_backend_supported: bool,
}
pub(crate) fn registry() -> [ConnectorDescriptor; 1] {
    [ConnectorDescriptor {
        provider: Provider::Google,
        display_name: "Google",
        supported_permissions: Permission::ALL,
        connection_capability: "installed_desktop_oauth",
        credential_backend_supported: cfg!(target_os = "macos"),
    }]
}
pub(crate) fn account_status<S: CredentialStore>(
    account: &Account,
    broker: &CredentialBroker<S>,
) -> Result<AccountView, ConnectorError> {
    let mut view = account.view();
    if view.status == AccountStatus::Connected
        && !broker.contains(&CredentialId::new(
            account.provider(),
            account.id(),
            CredentialKind::OAuthRefreshToken,
        ))?
    {
        view.status = AccountStatus::Disconnected;
    }
    Ok(view)
}

pub(crate) fn finalize_connection<S: CredentialStore, A: AccountRepository>(
    broker: &mut CredentialBroker<S>,
    accounts: &mut A,
    id: AccountId,
    label: String,
    email: Option<String>,
    tokens: google::OAuthTokenResponse,
    now: u64,
) -> Result<AccountView, ConnectorError> {
    finalize_connection_ref(broker, accounts, id, label, email, &tokens, now)
}
pub(super) fn finalize_connection_ref<S: CredentialStore, A: AccountRepository>(
    broker: &mut CredentialBroker<S>,
    accounts: &mut A,
    id: AccountId,
    label: String,
    email: Option<String>,
    tokens: &google::OAuthTokenResponse,
    now: u64,
) -> Result<AccountView, ConnectorError> {
    let account = Account::new(
        id,
        Provider::Google,
        label,
        email,
        tokens.permissions.clone(),
        now,
    )?;
    // Refuse provider echoes in human metadata too, even when a token has no recognizable prefix.
    if account.contains_secret(tokens.access_token.expose())
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|value| account.contains_secret(value.expose()))
    {
        return Err(ConnectorError::InvalidAccount);
    }
    if accounts.get(id)?.is_some() {
        return Err(ConnectorError::DuplicateAccount);
    }
    let refresh = tokens
        .refresh_token
        .as_ref()
        .ok_or(ConnectorError::RefreshTokenRequired)?;
    let credential = CredentialId::new(Provider::Google, id, CredentialKind::OAuthRefreshToken);
    broker.put(&credential, refresh)?;
    match accounts.add(account.clone()) {
        Ok(()) => Ok(account.view()),
        // A commit/fsync failure may have installed metadata. Preserve the credential for reconciliation.
        Err(ConnectorError::ReconciliationRequired) => Err(ConnectorError::ReconciliationRequired),
        Err(error) => match broker.discard_created(&credential, refresh) {
            Ok(()) | Err(CredentialError::Missing) => Err(error),
            Err(_) => Err(ConnectorError::ReconciliationRequired),
        },
    }
}

// Audit writes are deferred until M41 has a consented connection lifecycle. This DTO is the
// allowlist: it cannot carry OAuth material, account email or provider/error payloads.
#[derive(Serialize)]
pub(crate) struct ConnectorAuditEvent {
    pub(super) timestamp: u64,
    pub(super) event: ConnectorAuditKind,
    pub(super) provider: Provider,
    pub(super) account_id: AccountId,
    pub(super) permissions: Vec<Permission>,
    pub(super) result: ConnectorAuditResult,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectorAuditKind {
    ConnectionFinalized,
    ConnectionFailed,
    CredentialRemoved,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectorAuditResult {
    Completed,
    Failed,
    ReconciliationRequired,
}

#[cfg(test)]
mod tests;
