use super::{permissions, ConnectorError, Permission};
use crate::credentials::{AccountId, Provider};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_ACCOUNTS: usize = 32;
pub(crate) const MAX_ACCOUNT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountStatus {
    Connected,
    Disconnected,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "AccountFields")]
pub(crate) struct Account {
    version: u8,
    account_id: AccountId,
    provider: Provider,
    display_label: String,
    email: Option<String>,
    granted_permissions: Vec<Permission>,
    created_at: u64,
    updated_at: u64,
    status: AccountStatus,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountFields {
    version: u8,
    account_id: AccountId,
    provider: Provider,
    display_label: String,
    email: Option<String>,
    granted_permissions: Vec<Permission>,
    created_at: u64,
    updated_at: u64,
    status: AccountStatus,
}
fn text(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value.trim() == value
        && !value.chars().any(char::is_control)
        && crate::snapshot::text::redact(value) == value
}
impl TryFrom<AccountFields> for Account {
    type Error = &'static str;
    fn try_from(fields: AccountFields) -> Result<Self, Self::Error> {
        let grants = permissions(fields.granted_permissions).map_err(|_| "invalid_account")?;
        if fields.version != 1
            || !text(&fields.display_label, 128)
            || fields.created_at == 0
            || fields.updated_at < fields.created_at
            || fields.updated_at > 253402300799
            || fields.email.as_ref().is_some_and(|value| {
                !text(value, 254)
                    || !value.is_ascii()
                    || value.bytes().any(|b| b.is_ascii_whitespace())
                    || value.matches('@').count() != 1
                    || value.starts_with('@')
                    || value.ends_with('@')
            })
        {
            return Err("invalid_account");
        }
        Ok(Self {
            version: 1,
            account_id: fields.account_id,
            provider: fields.provider,
            display_label: fields.display_label,
            email: fields.email,
            granted_permissions: grants,
            created_at: fields.created_at,
            updated_at: fields.updated_at,
            status: fields.status,
        })
    }
}
impl Account {
    pub(crate) fn new(
        id: AccountId,
        provider: Provider,
        label: String,
        email: Option<String>,
        grants: Vec<Permission>,
        now: u64,
    ) -> Result<Self, ConnectorError> {
        AccountFields {
            version: 1,
            account_id: id,
            provider,
            display_label: label,
            email,
            granted_permissions: grants,
            created_at: now,
            updated_at: now,
            status: AccountStatus::Connected,
        }
        .try_into()
        .map_err(|_| ConnectorError::InvalidAccount)
    }
    pub(crate) fn id(&self) -> AccountId {
        self.account_id
    }
    pub(crate) fn provider(&self) -> Provider {
        self.provider
    }
    pub(crate) fn permissions(&self) -> &[Permission] {
        &self.granted_permissions
    }
    pub(crate) fn updated(
        &self,
        label: String,
        email: Option<String>,
        grants: Vec<Permission>,
        status: AccountStatus,
        now: u64,
    ) -> Result<Self, ConnectorError> {
        if now < self.updated_at {
            return Err(ConnectorError::InvalidAccount);
        }
        AccountFields {
            version: 1,
            account_id: self.account_id,
            provider: self.provider,
            display_label: label,
            email,
            granted_permissions: grants,
            created_at: self.created_at,
            updated_at: now,
            status,
        }
        .try_into()
        .map_err(|_| ConnectorError::InvalidAccount)
    }
    pub(crate) fn allows(&self, permission: Permission) -> bool {
        self.status == AccountStatus::Connected && self.granted_permissions.contains(&permission)
    }
    pub(super) fn contains_secret(&self, value: &str) -> bool {
        self.display_label.contains(value)
            || self
                .email
                .as_ref()
                .is_some_and(|email| email.contains(value))
    }
    pub(crate) fn view(&self) -> AccountView {
        AccountView {
            account_id: self.account_id,
            provider: self.provider,
            display_label: self.display_label.clone(),
            granted_permissions: self.granted_permissions.clone(),
            status: self.status,
        }
    }
    pub(super) fn same_identity(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.provider == other.provider
            && self.created_at == other.created_at
            && self.updated_at <= other.updated_at
    }
}
#[derive(Serialize)]
pub(crate) struct AccountView {
    pub(super) account_id: AccountId,
    pub(super) provider: Provider,
    pub(super) display_label: String,
    pub(super) granted_permissions: Vec<Permission>,
    pub(super) status: AccountStatus,
}

pub(crate) trait AccountRepository {
    fn list(&self) -> Result<Vec<Account>, ConnectorError>;
    fn get(&self, id: AccountId) -> Result<Option<Account>, ConnectorError>;
    /// Storage means no commit; ReconciliationRequired means commit outcome may be ambiguous.
    fn add(&mut self, account: Account) -> Result<(), ConnectorError>;
    fn update(&mut self, account: Account) -> Result<(), ConnectorError>;
    /// Metadata only. No UI calls this; future removal must also handle broker credentials.
    fn remove(&mut self, id: AccountId) -> Result<(), ConnectorError>;
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountFile {
    pub(super) version: u8,
    pub(super) accounts: Vec<Account>,
}
impl AccountFile {
    pub(super) fn validate(&self) -> Result<(), ConnectorError> {
        if self.version != 1 || self.accounts.len() > MAX_ACCOUNTS {
            return Err(ConnectorError::CorruptAccounts);
        }
        let mut ids: Vec<_> = self.accounts.iter().map(Account::id).collect();
        ids.sort();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ConnectorError::CorruptAccounts);
        }
        Ok(())
    }
    pub(super) fn bytes(&mut self) -> Result<Vec<u8>, ConnectorError> {
        self.validate()?;
        self.accounts.sort_by_key(Account::id);
        let bytes = serde_json::to_vec(self).map_err(|_| ConnectorError::Storage)?;
        if bytes.len() > MAX_ACCOUNT_BYTES {
            return Err(ConnectorError::Storage);
        }
        Ok(bytes)
    }
}
