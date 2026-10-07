mod connectors;
mod credentials;
mod intent;
mod snapshot;
mod voice;
use snapshot::actions::{Action, ActionState};

#[tauri::command(rename_all = "snake_case")]
fn prepare_ghost_intent(
    window: tauri::WebviewWindow,
    project_alias: String,
    intent: String,
) -> Result<intent::PreparedIntent, &'static str> {
    intent::prepare(window.label(), project_alias, intent)
}
#[tauri::command(rename_all = "snake_case")]
async fn interpret_ghost_intent(
    window: tauri::WebviewWindow,
    prepared: intent::PreparedIntent,
    confirmed: Option<bool>,
) -> Result<intent::IntentResult, &'static str> {
    intent::validate_review(window.label(), &prepared, confirmed)?;
    let lease = intent::IntentLease::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        intent::interpret_from_environment(prepared, confirmed)
    })
    .await
    .map_err(|_| "transport")?
}
#[tauri::command(rename_all = "snake_case")]
async fn save_ghost_intent_plan(
    window: tauri::WebviewWindow,
    proposal: intent::IntentResult,
    confirmed: Option<bool>,
) -> Result<intent::SavedPlan, &'static str> {
    intent::validate_save(window.label(), &proposal, confirmed)?;
    tauri::async_runtime::spawn_blocking(move || intent::save_from_environment(proposal, confirmed))
        .await
        .map_err(|_| "save")?
}

#[tauri::command]
async fn transcribe_ghost_voice(
    window: tauri::WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<voice::VoiceResult, &'static str> {
    let header = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let audio = voice::validate(
        window.label(),
        request.body(),
        header("x-ghost-voice-mime"),
        header("x-ghost-voice-duration-ms"),
        header("x-ghost-voice-confirmed"),
    )?;
    let lease = voice::VoiceLease::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        voice::from_environment(audio)
    })
    .await
    .map_err(|_| "transport")?
}

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
        .manage(connectors::assistant::runtime::AssistantState::default())
        .invoke_handler(tauri::generate_handler![
            load_ghost_snapshot,
            open_ghost_artifact,
            reveal_ghost_artifact,
            search_ghost_memory,
            prepare_ghost_action_request,
            save_ghost_action_request,
            transcribe_ghost_voice,
            prepare_ghost_intent,
            interpret_ghost_intent,
            save_ghost_intent_plan,
            connectors::assistant::ipc::get_google_assistant_status,
            connectors::assistant::ipc::save_google_client_id,
            connectors::assistant::ipc::connect_google,
            connectors::assistant::ipc::search_google_mail,
            connectors::assistant::ipc::get_google_mail_digest,
            connectors::assistant::ipc::list_google_agenda,
            connectors::assistant::ipc::find_google_free_time,
            connectors::assistant::ipc::lookup_google_contacts,
            connectors::assistant::ipc::prepare_google_mutation,
            connectors::assistant::ipc::execute_google_mutation,
            connectors::assistant::ipc::disconnect_google_account
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
