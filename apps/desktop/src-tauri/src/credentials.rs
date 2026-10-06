//! Native-only credentials. No credential type is an IPC DTO or a plaintext storage format.
// M41 will wire these internal APIs; M40 deliberately exposes no connect/mutation command.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;
use zeroize::Zeroizing;

pub(crate) const SERVICE: &str = "com.kavisara.ghost.credentials";
pub(crate) const MAX_SECRET_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialError {
    InvalidIdentifier,
    InvalidSecret,
    Missing,
    Conflict,
    Unavailable,
    Store,
}
impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentifier => "invalid_credential_identifier",
            Self::InvalidSecret => "invalid_secret",
            Self::Missing => "credential_missing",
            Self::Conflict => "credential_exists",
            Self::Unavailable => "credential_store_unavailable",
            Self::Store => "credential_store_failed",
        })
    }
}

pub(crate) struct Secret {
    value: Zeroizing<String>,
}
impl Secret {
    pub(crate) fn new(value: String) -> Result<Self, CredentialError> {
        let value = Zeroizing::new(value);
        if value.is_empty()
            || value.len() > MAX_SECRET_BYTES
            || !value.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(CredentialError::InvalidSecret);
        }
        Ok(Self { value })
    }
    // Only trusted native code can borrow the value; no public field, Clone or Serialize.
    pub(crate) fn expose(&self) -> &str {
        &self.value
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}
impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provider {
    Google,
}
impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::Google => "google",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AccountId(Uuid);
impl AccountId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub(crate) fn parse(value: &str) -> Result<Self, CredentialError> {
        let id = Uuid::parse_str(value).map_err(|_| CredentialError::InvalidIdentifier)?;
        if id.get_version_num() != 4
            || id.get_variant() != uuid::Variant::RFC4122
            || id.to_string() != value
        {
            return Err(CredentialError::InvalidIdentifier);
        }
        Ok(Self(id))
    }
}
impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Serialize for AccountId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for AccountId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("invalid_account_id"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CredentialKind {
    OAuthRefreshToken,
    OAuthClientSecret,
}
impl CredentialKind {
    fn name(self) -> &'static str {
        match self {
            Self::OAuthRefreshToken => "oauth_refresh_token",
            Self::OAuthClientSecret => "oauth_client_secret",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CredentialId {
    provider: Provider,
    account_id: AccountId,
    kind: CredentialKind,
}
impl CredentialId {
    pub(crate) fn new(provider: Provider, account_id: AccountId, kind: CredentialKind) -> Self {
        Self {
            provider,
            account_id,
            kind,
        }
    }
    fn canonical(&self) -> String {
        format!(
            "v1/{}/{}/{}",
            self.provider.name(),
            self.account_id,
            self.kind.name()
        )
    }
    pub(crate) fn is_client_secret(&self, provider: Provider) -> bool {
        self.provider == provider && self.kind == CredentialKind::OAuthClientSecret
    }
}

pub(crate) trait CredentialStore {
    /// Insert only: implementations MUST reject an existing record, never upsert.
    fn put(&mut self, id: &CredentialId, value: &Secret) -> Result<(), CredentialError>;
    fn get(&self, id: &CredentialId) -> Result<Secret, CredentialError>;
    fn delete(&mut self, id: &CredentialId) -> Result<(), CredentialError>;
    fn contains(&self, id: &CredentialId) -> Result<bool, CredentialError> {
        match self.get(id) {
            Ok(_) => Ok(true),
            Err(CredentialError::Missing) => Ok(false),
            Err(error) => Err(error),
        }
    }
}
pub(crate) struct CredentialBroker<S: CredentialStore> {
    store: S,
}
impl<S: CredentialStore> CredentialBroker<S> {
    pub(crate) fn new(store: S) -> Self {
        Self { store }
    }
    pub(crate) fn put(
        &mut self,
        id: &CredentialId,
        secret: &Secret,
    ) -> Result<(), CredentialError> {
        self.store.put(id, secret)
    }
    pub(crate) fn get(&self, id: &CredentialId) -> Result<Secret, CredentialError> {
        self.store.get(id)
    }
    pub(crate) fn delete(&mut self, id: &CredentialId) -> Result<(), CredentialError> {
        self.store.delete(id)
    }
    pub(crate) fn discard_created(
        &mut self,
        id: &CredentialId,
        inserted: &Secret,
    ) -> Result<(), CredentialError> {
        use subtle::ConstantTimeEq;
        let current = match self.get(id) {
            Ok(value) => value,
            Err(CredentialError::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        // Compensate only our insert, never knowingly delete a changed credential.
        // Keychain and metadata cannot form one atomic transaction; callers reconcile ambiguity.
        if !bool::from(
            current
                .expose()
                .as_bytes()
                .ct_eq(inserted.expose().as_bytes()),
        ) {
            return Err(CredentialError::Conflict);
        }
        self.delete(id)
    }
    pub(crate) fn contains(&self, id: &CredentialId) -> Result<bool, CredentialError> {
        self.store.contains(id)
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::KeychainStore as PlatformStore;
impl CredentialBroker<PlatformStore> {
    pub(crate) fn platform() -> Self {
        Self::new(PlatformStore::default())
    }
}

// Compile and exercise this backend on macOS tests too, without touching Keychain.
#[cfg(any(not(target_os = "macos"), test))]
#[derive(Default)]
pub(crate) struct UnavailableStore;
#[cfg(any(not(target_os = "macos"), test))]
impl CredentialStore for UnavailableStore {
    fn put(&mut self, _: &CredentialId, _: &Secret) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn get(&self, _: &CredentialId) -> Result<Secret, CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&mut self, _: &CredentialId) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) use UnavailableStore as PlatformStore;

#[cfg(test)]
pub(crate) mod tests;
