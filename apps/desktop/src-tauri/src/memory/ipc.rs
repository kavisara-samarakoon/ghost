use super::{context, model::*, mutations::*, search, storage::MemoryStore, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub fn main_window(label: &str) -> Result<()> {
    if label == "main" {
        Ok(())
    } else {
        Err("unavailable")
    }
}
fn home() -> Result<PathBuf> {
    crate::snapshot::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    )
    .map_err(|_| "storage_failed")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetInput {
    pub memory_id: String,
}
#[derive(Serialize)]
pub struct MemoryStatus {
    pub record_count: usize,
    pub active: usize,
    pub archived: usize,
    pub expired: usize,
    pub plaintext: bool,
    pub provider_transmission: bool,
}
macro_rules! command {
    ($name:ident,$input:ty,$output:ty,$body:expr) => {
        #[tauri::command(rename_all = "snake_case")]
        pub async fn $name(
            window: tauri::WebviewWindow,
            app: tauri::AppHandle,
            input: $input,
        ) -> Result<$output> {
            main_window(window.label())?;
            tauri::async_runtime::spawn_blocking(move || {
                use tauri::Manager;
                let state = app.state::<MemoryState>();
                let mut runtime = state.0.try_lock().map_err(|_| "busy")?;
                let home = home()?;
                runtime.bind(&home);
                let store = MemoryStore::open(&home)?;
                let now = super::now()?;
                ($body)(&store, &mut runtime, &home, input, now, &app)
            })
            .await
            .map_err(|_| "unavailable")?
        }
    };
}
command!(
    get_personal_memory_status,
    EmptyInput,
    MemoryStatus,
    |store: &MemoryStore,
     _: &mut Runtime,
     _: &std::path::Path,
     _: EmptyInput,
     now: i64,
     _: &tauri::AppHandle| {
        let records = store.list()?;
        Ok(MemoryStatus {
            record_count: records.len(),
            active: records
                .iter()
                .filter(|r| r.status == Status::Active && !r.expired(now))
                .count(),
            archived: records
                .iter()
                .filter(|r| r.status == Status::Archived)
                .count(),
            expired: records.iter().filter(|r| r.expired(now)).count(),
            plaintext: true,
            provider_transmission: false,
        })
    }
);
command!(
    list_personal_memories,
    search::ListInput,
    Vec<search::Hit>,
    |store: &MemoryStore,
     _: &mut Runtime,
     _: &std::path::Path,
     input: search::ListInput,
     now: i64,
     _: &tauri::AppHandle| search::list(&store.list()?, &input, now)
);
command!(
    search_personal_memory,
    search::SearchInput,
    Vec<search::Hit>,
    |store: &MemoryStore,
     _: &mut Runtime,
     _: &std::path::Path,
     input: search::SearchInput,
     now: i64,
     _: &tauri::AppHandle| search::search(&store.list()?, &input, now)
);
command!(
    get_personal_memory,
    GetInput,
    Record,
    |store: &MemoryStore,
     _: &mut Runtime,
     _: &std::path::Path,
     input: GetInput,
     _: i64,
     _: &tauri::AppHandle| {
        id(&input.memory_id)?;
        store
            .list()?
            .into_iter()
            .find(|r| r.memory_id == input.memory_id)
            .ok_or("memory_missing")
    }
);
command!(
    prepare_personal_memory_mutation,
    PrepareInput,
    Preview,
    |store: &MemoryStore,
     runtime: &mut Runtime,
     home: &std::path::Path,
     input: PrepareInput,
     now: i64,
     _: &tauri::AppHandle| runtime.prepare(store, home, input, now)
);
command!(
    execute_personal_memory_mutation,
    ExecuteInput,
    Outcome,
    |store: &MemoryStore,
     runtime: &mut Runtime,
     home: &std::path::Path,
     input: ExecuteInput,
     now: i64,
     _: &tauri::AppHandle| runtime.execute(store, home, input, now)
);
command!(
    build_unified_context,
    context::Input,
    context::Pack,
    |store: &MemoryStore,
     _: &mut Runtime,
     home: &std::path::Path,
     input: context::Input,
     now: i64,
     app: &tauri::AppHandle| {
        use tauri::Manager;
        let records = if input.sources.personal_memory {
            store.list()?
        } else {
            Vec::new()
        };
        context::build(
            home,
            &records,
            input,
            now,
            &app.state::<crate::connectors::assistant::runtime::AssistantState>(),
        )
    }
);
