use super::{
    api::GoogleTransport, connection, model::*, mutations::*, runtime::*, DiskStore,
    GoogleClientConfig,
};
use crate::connectors::storage::AccountStore;
use crate::credentials::{AccountId, CredentialBroker, PlatformStore};
use serde::Deserialize;

pub fn main_window(window: &str) -> Result<()> {
    if window == "main" {
        Ok(())
    } else {
        Err("unavailable")
    }
}
#[cfg(target_os = "macos")]
fn run<R>(
    window: &str,
    state: &AssistantState,
    operation: impl FnOnce(
        &mut Assistant<'_, PlatformStore, AccountStore, GoogleTransport>,
    ) -> Result<R>,
) -> Result<R> {
    main_window(window)?;
    let mut runtime = state.inner.try_lock().map_err(|_| "busy")?;
    let home = crate::snapshot::resolve_home(
        std::env::var_os("GHOST_HOME"),
        std::env::home_dir(),
        std::env::current_dir().ok(),
    )
    .map_err(|_| "storage_failed")?;
    runtime.bind(&home);
    runtime.prune(std::time::Instant::now(), super::now()?);
    let disk = AccountStore::open(&home).map_err(super::map_error)?;
    let mut assistant = Assistant {
        runtime: &mut runtime,
        broker: CredentialBroker::platform(),
        disk,
        transport: GoogleTransport::default(),
        budget: 0,
    };
    operation(&mut assistant)
}
#[cfg(not(target_os = "macos"))]
fn run<R>(
    _: &str,
    _: &AssistantState,
    _: impl FnOnce(&mut Assistant<'_, PlatformStore, AccountStore, GoogleTransport>) -> Result<R>,
) -> Result<R> {
    Err("unavailable")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveClientInput {
    pub client_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectInput {
    pub account_id: AccountId,
    pub confirmation: String,
}

#[tauri::command]
pub async fn get_google_assistant_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Status> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AssistantState>();
        run(&label, &state, |assistant| assistant.status())
    })
    .await
    .map_err(|_| "unavailable")?
}
#[tauri::command(rename_all = "snake_case")]
pub async fn save_google_client_id(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: SaveClientInput,
) -> Result<ConfigStatus> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AssistantState>();
        run(&label, &state, |assistant| {
            let config = GoogleClientConfig::new(input.client_id)?;
            assistant.disk.save_client_config(&config)?;
            assistant.runtime.cache.clear();
            assistant.runtime.pending.clear();
            Ok(ConfigStatus {
                configured: true,
                client_id: Some(config.client_id),
            })
        })
    })
    .await
    .map_err(|_| "unavailable")?
}
#[tauri::command(rename_all = "snake_case")]
pub async fn connect_google(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: ConnectInput,
) -> Result<ConnectResult> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = handle.state::<AssistantState>();
        run(&label, &state, |assistant| {
            assistant.connect(
                input,
                |request| connection::open_browser(&handle, request),
                connection::receive,
                |request, timeout| {
                    crate::connectors::google::send_token_request_with_timeout(request, timeout)
                },
            )
        })
    })
    .await
    .map_err(|_| "unavailable")?
}
macro_rules! blocking_command {
    ($name:ident,$input:ty,$output:ty,$method:ident) => {
        #[tauri::command(rename_all = "snake_case")]
        pub async fn $name(
            window: tauri::WebviewWindow,
            app: tauri::AppHandle,
            input: $input,
        ) -> Result<$output> {
            main_window(window.label())?;
            let label = window.label().to_owned();
            tauri::async_runtime::spawn_blocking(move || {
                use tauri::Manager;
                let state = app.state::<AssistantState>();
                run(&label, &state, |assistant| assistant.$method(input))
            })
            .await
            .map_err(|_| "unavailable")?
        }
    };
}
blocking_command!(list_google_agenda, AgendaInput, AgendaResult, agenda);
blocking_command!(
    find_google_free_time,
    FreeTimeInput,
    FreeTimeResult,
    free_time
);
blocking_command!(
    lookup_google_contacts,
    ContactsInput,
    ContactResult,
    contacts
);
blocking_command!(
    prepare_google_mutation,
    PrepareInput,
    PreparedMutation,
    prepare
);
blocking_command!(
    execute_google_mutation,
    ExecuteInput,
    MutationResult,
    execute
);
#[tauri::command(rename_all = "snake_case")]
pub async fn search_google_mail(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: MailSearch,
) -> Result<MailResult> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AssistantState>();
        run(&label, &state, |assistant| assistant.search(input, false))
    })
    .await
    .map_err(|_| "unavailable")?
}
#[tauri::command(rename_all = "snake_case")]
pub async fn get_google_mail_digest(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: MailSearch,
) -> Result<MailResult> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AssistantState>();
        run(&label, &state, |assistant| assistant.search(input, true))
    })
    .await
    .map_err(|_| "unavailable")?
}
#[tauri::command(rename_all = "snake_case")]
pub async fn disconnect_google_account(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    input: DisconnectInput,
) -> Result<DisconnectResult> {
    main_window(window.label())?;
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AssistantState>();
        run(&label, &state, |assistant| {
            assistant.disconnect(input.account_id, input.confirmation)
        })
    })
    .await
    .map_err(|_| "unavailable")?
}
