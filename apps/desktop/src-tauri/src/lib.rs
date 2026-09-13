mod snapshot;
use snapshot::actions::{Action, ActionState};

#[tauri::command(rename_all = "snake_case")]
fn prepare_ghost_action_request(
    window: tauri::WebviewWindow,
    project_alias: String,
    action: snapshot::requests::Action,
) -> Result<snapshot::requests::ActionRequest, &'static str> {
    if window.label() != "main" {
        return Err("Action requests are unavailable here.");
    }
    snapshot::requests::prepare(project_alias, action)
}

#[tauri::command(rename_all = "snake_case")]
async fn save_ghost_action_request(
    window: tauri::WebviewWindow,
    request: snapshot::requests::ActionRequest,
    confirmed: bool,
) -> Result<snapshot::requests::SavedRequest, &'static str> {
    if window.label() != "main" {
        return Err("Action requests are unavailable here.");
    }
    tauri::async_runtime::spawn_blocking(move || {
        snapshot::requests::save_from_environment(request, confirmed)
    })
    .await
    .map_err(|_| "Request storage could not be checked. Review recent requests before retrying.")?
}

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
            search_ghost_memory,
            prepare_ghost_action_request,
            save_ghost_action_request
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
