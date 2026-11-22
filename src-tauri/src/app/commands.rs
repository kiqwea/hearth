//! Команды, которые вызывает фронтенд через `invoke`.
//!
//! Аргументы приходят из JS в camelCase (`includeUnstable`), Tauri сам
//! переводит их в snake_case. Ошибки уходят как `{ kind, message }`.

use tauri::State;

use super::AppState;
use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::java::InstalledJava;
use crate::manager::{ServerSummary, STOP_TIMEOUT};
use crate::process::ConsoleLine;
use crate::providers::{self, GameVersion, Loader};
use crate::servers::{self, CreateServerRequest, PropertyEntry, ServerMeta, ServerPatch};
use crate::system::{self, NetworkInfo, SystemInfo};

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        get_system_info,
        get_network_info,
        get_config,
        set_config,
        list_versions,
        list_java,
        list_servers,
        get_server,
        create_server,
        update_server,
        delete_server,
        install_server,
        start_server,
        stop_server,
        kill_server,
        stop_all_servers,
        restart_server,
        send_command,
        get_console,
        get_properties,
        set_properties,
        open_server_folder,
    ]
}

#[tauri::command]
fn get_system_info() -> SystemInfo {
    system::system_info()
}

#[tauri::command]
fn get_network_info() -> NetworkInfo {
    system::network_info()
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.config.lock().map(|c| c.clone()).unwrap_or_default()
}

#[tauri::command]
async fn set_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig> {
    let old = state.config.lock().map(|c| c.clone()).unwrap_or_default();
    if old.servers_dir != config.servers_dir {
        state
            .manager
            .set_servers_dir(config.servers_dir(&state.paths))?;
    }
    config.save(&state.paths.config_file()).await?;
    if let Ok(mut current) = state.config.lock() {
        *current = config.clone();
    }
    Ok(config)
}

#[tauri::command]
async fn list_versions(
    state: State<'_, AppState>,
    loader: Loader,
    include_unstable: bool,
) -> Result<Vec<GameVersion>> {
    providers::list_versions(state.manager.http(), loader, include_unstable).await
}

#[tauri::command]
fn list_java(state: State<'_, AppState>) -> Vec<InstalledJava> {
    state.manager.java().installed()
}

#[tauri::command]
fn list_servers(state: State<'_, AppState>) -> Vec<ServerSummary> {
    state.manager.list()
}

#[tauri::command]
fn get_server(state: State<'_, AppState>, id: String) -> Result<ServerSummary> {
    state.manager.get(&id)
}

#[tauri::command]
async fn create_server(state: State<'_, AppState>, request: CreateServerRequest) -> Result<ServerMeta> {
    state.manager.create(request).await
}

#[tauri::command]
async fn update_server(state: State<'_, AppState>, id: String, patch: ServerPatch) -> Result<ServerMeta> {
    state.manager.update(&id, patch).await
}

#[tauri::command]
async fn delete_server(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.delete(&id).await
}

#[tauri::command]
async fn install_server(state: State<'_, AppState>, id: String) -> Result<ServerMeta> {
    state.manager.install(&id).await
}

#[tauri::command]
async fn start_server(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.start(&id).await
}

#[tauri::command]
async fn stop_server(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.stop(&id, STOP_TIMEOUT).await
}

/// Перед установкой обновления: корректно останавливает все серверы.
#[tauri::command]
async fn stop_all_servers(state: State<'_, AppState>) -> Result<()> {
    state.manager.stop_all(STOP_TIMEOUT).await;
    Ok(())
}

#[tauri::command]
fn kill_server(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.kill(&id)
}

#[tauri::command]
async fn restart_server(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.restart(&id).await
}

#[tauri::command]
async fn send_command(state: State<'_, AppState>, id: String, command: String) -> Result<()> {
    state.manager.send_command(&id, &command).await
}

#[tauri::command]
fn get_console(state: State<'_, AppState>, id: String) -> Vec<ConsoleLine> {
    state.manager.console(&id)
}

#[tauri::command]
fn get_properties(state: State<'_, AppState>, id: String) -> Result<Vec<PropertyEntry>> {
    state.manager.properties(&id)
}

#[tauri::command]
async fn set_properties(
    state: State<'_, AppState>,
    id: String,
    entries: Vec<PropertyEntry>,
) -> Result<()> {
    state.manager.set_properties(&id, entries).await
}

#[tauri::command]
fn open_server_folder(state: State<'_, AppState>, id: String) -> Result<()> {
    servers::validate_id(&id)?;
    let dir = state.manager.store().game_dir(&id);
    if !dir.is_dir() {
        return Err(Error::ServerNotFound(id));
    }
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| Error::other(format!("Не удалось открыть папку: {e}")))
}
