//! Local attention scheduling only. No task execution authority.
pub(crate) mod evaluate;
pub(crate) mod ipc;
pub(crate) mod model;
pub(crate) mod mutations;
#[cfg(unix)]
pub(crate) mod storage;
#[cfg(not(unix))]
pub(crate) mod storage {
    use super::{model::Document, mutations::AuditEvent, Result};
    pub struct AutomationStore;
    impl AutomationStore {
        pub fn open(_: &std::path::Path) -> Result<Self> {
            Err("unavailable")
        }
        pub fn read_document(&self) -> Result<Document> {
            Err("unavailable")
        }
        pub fn transaction<T>(
            &self,
            _: impl FnOnce(&mut Document) -> Result<(T, Vec<AuditEvent>)>,
        ) -> Result<(T, bool)> {
            Err("unavailable")
        }
    }
}
#[cfg(all(test, unix))]
mod tests;
pub(crate) type Result<T> = std::result::Result<T, &'static str>;
