pub(crate) mod api;
pub(crate) mod connection;
pub(crate) mod data;
pub(crate) mod ipc;
pub(crate) mod model;
pub(crate) mod mutations;
pub(crate) mod runtime;

use crate::connectors::{accounts::AccountRepository, oauth::OAuthClientConfig, ConnectorError};
use model::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleClientConfig {
    pub version: u8,
    pub client_id: String,
}
impl GoogleClientConfig {
    pub fn new(client_id: String) -> Result<Self> {
        let config = Self {
            version: 1,
            client_id,
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !self.client_id.ends_with(".apps.googleusercontent.com")
            || self.client_id.len() <= 25
            || self.client_id.len() > 256
            || !self
                .client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err("invalid_client");
        }
        OAuthClientConfig::new(self.client_id.clone(), None).map_err(map_error)?;
        Ok(())
    }
    pub fn oauth(&self) -> Result<OAuthClientConfig> {
        self.validate()?;
        OAuthClientConfig::new(self.client_id.clone(), None).map_err(map_error)
    }
}
pub trait DiskStore: AccountRepository {
    fn client_config(&self) -> Result<Option<GoogleClientConfig>>;
    fn save_client_config(&mut self, config: &GoogleClientConfig) -> Result<()>;
    fn audit(&self, event: &mutations::AuditEvent) -> Result<()>;
}
pub fn map_error(error: ConnectorError) -> &'static str {
    match error {
        ConnectorError::Credential(crate::credentials::CredentialError::Missing) => {
            "reconnect_required"
        }
        ConnectorError::Credential(crate::credentials::CredentialError::Unavailable)
        | ConnectorError::Unavailable => "unavailable",
        ConnectorError::InvalidPermissions => "permission_missing",
        ConnectorError::Provider | ConnectorError::RefreshTokenRequired => "reconnect_required",
        ConnectorError::Transport => "transport_failed",
        ConnectorError::ReconciliationRequired => "reconciliation_required",
        ConnectorError::InvalidClient => "invalid_client",
        ConnectorError::ExpiredFlow => "timeout",
        ConnectorError::InvalidCallback => "invalid_callback",
        ConnectorError::ProviderDenied => "consent_denied",
        ConnectorError::InvalidTokenResponse => "invalid_response",
        ConnectorError::DuplicateAccount => "account_exists",
        ConnectorError::InvalidAccount => "invalid_input",
        _ => "storage_failed",
    }
}
pub fn now() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|n| n.as_secs())
        .map_err(|_| "unavailable")
}
#[cfg(test)]
mod tests;
