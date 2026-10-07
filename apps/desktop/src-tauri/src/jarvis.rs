//! Planner only. This module has no action execution or provider-data ingestion bridge.
mod context;
pub(crate) mod interpret;
pub(crate) mod ipc;
pub(crate) mod model;
#[cfg(test)]
mod tests;
pub type Result<T> = std::result::Result<T, &'static str>;
pub const VERSION: u8 = 1;
pub fn main_window(window: &str) -> Result<()> {
    if window == "main" {
        Ok(())
    } else {
        Err("unavailable")
    }
}
pub fn hash(value: &impl serde::Serialize) -> Result<String> {
    crate::memory::hash(value)
}
