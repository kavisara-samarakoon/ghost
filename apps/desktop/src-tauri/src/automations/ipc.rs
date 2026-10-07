use super::{evaluate, model::*, mutations::*, storage::AutomationStore, Result};
use serde::{Deserialize, Serialize};
use tauri::Manager;
pub fn main_window(label: &str) -> Result<()> {
    if label == "main" {
        Ok(())
    } else {
        Err("unavailable")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemInput {
    pub item_id: String,
}
#[derive(Serialize)]
pub struct Status {
    pub definitions: usize,
    pub pending_count: usize,
    pub plaintext: bool,
    pub local_only: bool,
}
#[derive(Serialize)]
pub struct View {
    pub definition: Definition,
    pub next_due: Option<i64>,
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
                let home = crate::snapshot::resolve_home(
                    std::env::var_os("GHOST_HOME"),
                    std::env::home_dir(),
                    std::env::current_dir().ok(),
                )
                .map_err(|_| "storage")?;
                let state = app.state::<AutomationState>();
                let mut runtime = state.0.try_lock().map_err(|_| "busy")?;
                let store = AutomationStore::open(&home)?;
                ($body)(&store, &mut runtime, &home, input, crate::memory::now()?)
            })
            .await
            .map_err(|_| "unavailable")?
        }
    };
}
command!(
    get_automation_status,
    EmptyInput,
    Status,
    |store: &AutomationStore, _: &mut Runtime, _: &std::path::Path, _: EmptyInput, _: i64| {
        let d = store.read_document()?;
        Ok(Status {
            definitions: d.definitions.len(),
            pending_count: d
                .inbox
                .iter()
                .filter(|i| i.status == ItemStatus::Pending)
                .count(),
            plaintext: true,
            local_only: true,
        })
    }
);
command!(
    list_automations,
    EmptyInput,
    Vec<View>,
    |store: &AutomationStore, _: &mut Runtime, _: &std::path::Path, _: EmptyInput, now: i64| {
        store
            .read_document()?
            .definitions
            .into_iter()
            .map(|definition| {
                Ok(View {
                    next_due: evaluate::next_due(&definition, now)?,
                    definition,
                })
            })
            .collect()
    }
);
command!(
    list_automation_inbox,
    EmptyInput,
    Vec<Item>,
    |store: &AutomationStore, _: &mut Runtime, _: &std::path::Path, _: EmptyInput, _: i64| {
        Ok(store.read_document()?.inbox)
    }
);
command!(
    prepare_automation_mutation,
    PrepareInput,
    Preview,
    |store: &AutomationStore,
     runtime: &mut Runtime,
     home: &std::path::Path,
     input: PrepareInput,
     now: i64| runtime.prepare(store, home, input, now)
);
command!(
    execute_automation_mutation,
    ExecuteInput,
    Outcome,
    |store: &AutomationStore,
     runtime: &mut Runtime,
     home: &std::path::Path,
     input: ExecuteInput,
     now: i64| runtime.execute(store, home, input, now)
);
command!(
    evaluate_automations,
    EmptyInput,
    evaluate::Evaluation,
    |store: &AutomationStore, _: &mut Runtime, home: &std::path::Path, _: EmptyInput, now: i64| {
        evaluate::evaluate(store, home, now)
    }
);
command!(
    acknowledge_automation_item,
    ItemInput,
    bool,
    |store: &AutomationStore, _: &mut Runtime, _: &std::path::Path, input: ItemInput, now: i64| {
        handle_item(store, &input.item_id, ItemStatus::Acknowledged, now)
    }
);
command!(
    dismiss_automation_item,
    ItemInput,
    bool,
    |store: &AutomationStore, _: &mut Runtime, _: &std::path::Path, input: ItemInput, now: i64| {
        handle_item(store, &input.item_id, ItemStatus::Dismissed, now)
    }
);
