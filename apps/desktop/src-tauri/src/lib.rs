mod snapshot;
use snapshot::actions::{Action, ActionState};

#[tauri::command]
async fn load_ghost_snapshot() -> Result<snapshot::GhostSnapshot, &'static str> {
    tauri::async_runtime::spawn_blocking(snapshot::load_from_environment)
        .await
        .map_err(|_| "Local snapshot could not be loaded.")
}

#[tauri::command(rename_all = "snake_case")]
async fn search_ghost_memory(
    window: tauri::WebviewWindow,
    query: String,
    project_alias: Option<String>,
) -> Result<snapshot::search::SearchResponse, &'static str> {
    if window.label() != "main" {
        return Err("Local search is unavailable here.");
    }
    tauri::async_runtime::spawn_blocking(move || {
        snapshot::search::from_environment(&query, project_alias.as_deref())
    })
    .await
    .map_err(|_| "Local search could not be completed.")
}

#[tauri::command(rename_all = "snake_case")]
fn open_ghost_artifact(
    window: tauri::WebviewWindow,
    project_alias: String,
    relative_path: String,
) -> ActionState {
    if window.label() != "main" {
        return ActionState::Rejected;
    }
    snapshot::actions::from_environment(&project_alias, &relative_path, Action::Open)
}

#[tauri::command(rename_all = "snake_case")]
fn reveal_ghost_artifact(
    window: tauri::WebviewWindow,
    project_alias: String,
    relative_path: String,
) -> ActionState {
    if window.label() != "main" {
        return ActionState::Rejected;
    }
    snapshot::actions::from_environment(&project_alias, &relative_path, Action::Reveal)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_ghost_snapshot,
            open_ghost_artifact,
            reveal_ghost_artifact,
            search_ghost_memory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
