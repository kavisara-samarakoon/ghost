use super::{interpret::*, Result};
use tauri::Manager;
fn home() -> Result<std::path::PathBuf> {
    crate::snapshot::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    )
    .map_err(|_| "unavailable")
}
#[tauri::command]
pub fn get_jarvis_capabilities(window: tauri::WebviewWindow) -> Result<super::model::Registry> {
    super::main_window(window.label())?;
    Ok(super::model::registry())
}
#[tauri::command(rename_all = "snake_case")]
pub async fn prepare_jarvis_request(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: PrepareInput,
) -> Result<Review> {
    super::main_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<JarvisState>();
        let mut runtime = state.0.try_lock().map_err(|_| "busy")?;
        runtime.pending = None;
        let home = home()?;
        let review = prepare(&home, input, crate::memory::now()?)?;
        Ok(runtime.remember(&home, review))
    })
    .await
    .map_err(|_| "unavailable")?
}
#[tauri::command(rename_all = "snake_case")]
pub async fn interpret_jarvis_request(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: SendInput,
) -> Result<Proposal> {
    super::main_window(window.label())?;
    let lease = crate::intent::IntentLease::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let state = app.state::<JarvisState>();
        let mut runtime = state.0.try_lock().map_err(|_| "busy")?;
        let home = home()?;
        let now = crate::memory::now()?;
        let review = runtime.take(&home, input, now, |fields| {
            if let Some(alias) = &fields.project_alias {
                crate::snapshot::validate_project_binding(&home, alias)
                    .map_err(|_| "invalid_context")?;
            }
            super::context::load(
                &home,
                &fields.sharing,
                fields.context_query.as_deref(),
                fields.project_alias.as_deref(),
                now,
            )
        })?;
        #[cfg(unix)]
        {
            let store = crate::intent::storage::IntentStore::open(&home)?;
            interpret_with(
                review,
                now,
                |event| store.append_jarvis(event),
                || std::env::var("OPENAI_API_KEY").ok(),
                crate::intent::send_structured,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = review;
            Err("unavailable")
        }
    })
    .await
    .map_err(|_| "transport")?
}
