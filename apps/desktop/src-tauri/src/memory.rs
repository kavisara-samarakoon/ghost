//! Explicit local memory writes and ephemeral, data-only context. No model execution.
mod context;
pub(crate) mod ipc;
pub(crate) mod model;
pub(crate) mod mutations;
mod search;
#[cfg(unix)]
mod storage;
#[cfg(not(unix))]
mod storage {
    use super::{
        model::{Document, Record},
        mutations::AuditEvent,
        Result,
    };
    pub struct MemoryStore;
    impl MemoryStore {
        pub fn open(_: &std::path::Path) -> Result<Self> {
            Err("unavailable")
        }
        pub fn list(&self) -> Result<Vec<Record>> {
            Err("unavailable")
        }
        pub fn mutate(
            &self,
            _: &AuditEvent,
            _: impl FnOnce(&mut Document) -> Result<()>,
            _: impl FnMut(&str) -> Result<()>,
        ) -> Result<bool> {
            Err("unavailable")
        }
    }
}
#[cfg(all(test, unix))]
mod tests;
pub(crate) type Result<T> = std::result::Result<T, &'static str>;
pub(crate) fn now() -> Result<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|_| "unavailable")
}
fn hash(value: &impl serde::Serialize) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&canonical(
                serde_json::to_value(value).map_err(|_| "invalid_input")?
            ))
            .map_err(|_| "invalid_input")?
        )
    ))
}

fn canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical).collect())
        }
        value => value,
    }
}
