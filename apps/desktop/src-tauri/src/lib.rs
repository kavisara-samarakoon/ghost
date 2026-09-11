mod snapshot;

#[tauri::command]
async fn load_ghost_snapshot() -> Result<snapshot::GhostSnapshot, &'static str> {
    tauri::async_runtime::spawn_blocking(snapshot::load_from_environment)
        .await
        .map_err(|_| "Local snapshot could not be loaded.")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![load_ghost_snapshot])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
