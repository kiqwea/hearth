//! Команды, которые вызывает фронтенд через `invoke`.
//!
//! Аргументы приходят из JS в camelCase (`includeUnstable`), Tauri сам
//! переводит их в snake_case. Ошибки уходят как `{ kind, message }`.

use tauri::State;

use super::AppState;
use crate::backups::{BackupInfo, BackupKind, SavedWorld};
use crate::mods::{self, ModList, ModSearch};
use crate::presets::{PregenStatus, PresetResult};
use crate::config::AppConfig;
use crate::error::{Error, Result};
use crate::java::InstalledJava;
use crate::manager::{DirectAttempt, PirateResult, ServerSummary, STOP_TIMEOUT};
use crate::process::ConsoleLine;
use crate::providers::{self, GameVersion, Loader};
use crate::servers::{self, CreateServerRequest, PropertyEntry, ServerMeta, ServerPatch};
use crate::system::{self, NetworkInfo, SystemInfo};
use crate::players::{PlayerAction, PlayerInfo};
use crate::tunnel::PlayitStatus;

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
        get_playit_status,
        playit_start_claim,
        playit_cancel_claim,
        playit_unlink,
        playit_retry,
        playit_dashboard_url,
        enable_friend_access,
        disable_friend_access,
        try_direct_access,
        set_pirate_mode,
        set_nick_protection,
        get_players,
        player_action,
        list_backups,
        backup_in_progress,
        create_backup,
        delete_backup,
        restore_backup,
        export_backup,
        export_world,
        list_saved_worlds,
        import_world,
        open_backups_folder,
        list_mods,
        search_mods,
        install_mod,
        update_mods,
        remove_mod,
        open_mods_folder,
        apply_preset,
        pregen_start,
        pregen_status,
        pregen_cancel,
    ]
}

#[tauri::command]
fn list_backups(state: State<'_, AppState>, id: String) -> Result<Vec<BackupInfo>> {
    state.manager.backups(&id)
}

#[tauri::command]
fn backup_in_progress(state: State<'_, AppState>, id: String) -> bool {
    state.manager.backup_in_progress(&id)
}

/// Копия мира сейчас (и на работающем сервере). `null`, если мира ещё нет.
#[tauri::command]
async fn create_backup(state: State<'_, AppState>, id: String) -> Result<Option<BackupInfo>> {
    state
        .manager
        .backup(&id, BackupKind::Manual)
        .await
        .inspect_err(|e| tracing::warn!("Бэкап сервера {id}: {e}"))
}

#[tauri::command]
async fn delete_backup(state: State<'_, AppState>, id: String, file: String) -> Result<()> {
    state.manager.delete_backup(&id, &file).await
}

/// Возвращает мир из копии. Ответ: страховочная копия мира, который был до этого.
#[tauri::command]
async fn restore_backup(state: State<'_, AppState>, id: String, file: String) -> Result<Option<BackupInfo>> {
    state
        .manager
        .restore_backup(&id, &file)
        .await
        .inspect_err(|e| tracing::warn!("Восстановление сервера {id} из {file}: {e}"))
}

#[tauri::command]
async fn export_backup(state: State<'_, AppState>, id: String, file: String, dest: String) -> Result<()> {
    state.manager.export_backup(&id, &file, &dest).await
}

#[tauri::command]
async fn export_world(state: State<'_, AppState>, id: String, dest: String) -> Result<()> {
    state
        .manager
        .export_world(&id, &dest)
        .await
        .inspect_err(|e| tracing::warn!("Экспорт мира сервера {id}: {e}"))
}

#[tauri::command]
async fn list_saved_worlds(state: State<'_, AppState>) -> Result<Vec<SavedWorld>> {
    state.manager.saved_worlds().await
}

/// Мир из одиночной игры (папка) или из zip. Ответ: страховочная копия прежнего мира.
#[tauri::command]
async fn import_world(state: State<'_, AppState>, id: String, source: String) -> Result<Option<BackupInfo>> {
    state
        .manager
        .import_world(&id, &source)
        .await
        .inspect_err(|e| tracing::warn!("Импорт мира в сервер {id}: {e}"))
}

#[tauri::command]
fn open_backups_folder(state: State<'_, AppState>, id: String) -> Result<()> {
    servers::validate_id(&id)?;
    let dir = state.manager.store().backups_dir(&id);
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| Error::other(format!("Не удалось открыть папку: {e}")))
}

#[tauri::command]
fn get_players(state: State<'_, AppState>, id: String) -> Result<Vec<PlayerInfo>> {
    state.manager.players(&id)
}

/// Кик, бан, оператор, белый список. Возвращает ответ сервера.
#[tauri::command]
async fn player_action(
    state: State<'_, AppState>,
    id: String,
    action: PlayerAction,
    name: String,
    reason: Option<String>,
) -> Result<String> {
    state.manager.player_action(&id, action, &name, reason.as_deref()).await
}

/// Вход с пиратских клиентов: бэкап, плагин авторизации, перенос вещей игроков.
#[tauri::command]
async fn set_pirate_mode(
    state: State<'_, AppState>,
    id: String,
    enable: bool,
    protect: bool,
) -> Result<PirateResult> {
    state
        .manager
        .set_pirate_mode(&id, enable, protect)
        .await
        .inspect_err(|e| tracing::warn!("Пиратский режим сервера {id}: {e}"))
}

/// Защита ников паролем (AuthMe/EasyAuth) отдельно от пиратского режима.
#[tauri::command]
async fn set_nick_protection(state: State<'_, AppState>, id: String, on: bool) -> Result<ServerMeta> {
    state
        .manager
        .set_nick_protection(&id, on)
        .await
        .inspect_err(|e| tracing::warn!("Защита ников сервера {id}: {e}"))
}

/// Сначала пробуем без посредников: проброс порта на роутере.
#[tauri::command]
async fn try_direct_access(state: State<'_, AppState>, id: String) -> Result<DirectAttempt> {
    state.manager.try_direct_access(&id).await
}

#[tauri::command]
async fn get_playit_status(state: State<'_, AppState>) -> Result<PlayitStatus> {
    Ok(state.manager.playit().status_fresh().await)
}

/// Начинает подключение к playit.gg. Возвращает ссылку для браузера.
/// `async`: команда запускает фоновую задачу, а синхронные команды Tauri
/// выполняются вне среды tokio.
#[tauri::command]
async fn playit_start_claim(state: State<'_, AppState>) -> Result<String> {
    state.manager.playit().start_claim()
}

#[tauri::command]
fn playit_cancel_claim(state: State<'_, AppState>) {
    state.manager.playit().cancel_claim();
}

#[tauri::command]
async fn playit_unlink(state: State<'_, AppState>) -> Result<()> {
    state.manager.playit().unlink().await
}

#[tauri::command]
async fn playit_retry(state: State<'_, AppState>) -> Result<()> {
    state.manager.playit().retry().await
}

#[tauri::command]
async fn playit_dashboard_url(state: State<'_, AppState>) -> Result<String> {
    Ok(state.manager.playit().dashboard_url().await)
}

#[tauri::command]
async fn enable_friend_access(state: State<'_, AppState>, id: String) -> Result<ServerMeta> {
    state
        .manager
        .enable_friend_access(&id)
        .await
        .inspect_err(|e| tracing::warn!("Доступ для друзей сервера {id} не включился: {e:?}"))
}

#[tauri::command]
async fn disable_friend_access(state: State<'_, AppState>, id: String) -> Result<ServerMeta> {
    state.manager.disable_friend_access(&id).await
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
async fn delete_server(state: State<'_, AppState>, id: String, delete_tunnel: Option<bool>) -> Result<()> {
    state.manager.delete(&id, delete_tunnel.unwrap_or(true)).await
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

#[tauri::command]
async fn list_mods(state: State<'_, AppState>, id: String) -> Result<ModList> {
    state.manager.mods(&id).await
}

#[tauri::command]
async fn search_mods(state: State<'_, AppState>, id: String, query: String, offset: u32) -> Result<ModSearch> {
    state.manager.search_mods(&id, &query, offset).await
}

/// Ставит плагин или мод с Modrinth вместе с зависимостями. Ответ: поставленные файлы.
#[tauri::command]
async fn install_mod(state: State<'_, AppState>, id: String, project: String) -> Result<Vec<String>> {
    state
        .manager
        .install_mod(&id, &project)
        .await
        .inspect_err(|e| tracing::warn!("Установка {project} на сервер {id}: {e}"))
}

#[tauri::command]
async fn update_mods(state: State<'_, AppState>, id: String) -> Result<Vec<String>> {
    state
        .manager
        .update_mods(&id)
        .await
        .inspect_err(|e| tracing::warn!("Обновление модов сервера {id}: {e}"))
}

#[tauri::command]
async fn remove_mod(state: State<'_, AppState>, id: String, file: String) -> Result<()> {
    state.manager.remove_mod(&id, &file).await
}

#[tauri::command]
fn open_mods_folder(state: State<'_, AppState>, id: String) -> Result<()> {
    let store = state.manager.store();
    let meta = store.load(&id)?;
    let folder = mods::folder_name(meta.loader).ok_or_else(|| Error::invalid("У Vanilla нет папки модов"))?;
    let dir = store.game_dir(&id).join(folder);
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| Error::other(format!("Не удалось открыть папку: {e}")))
}

/// Пресет «Производительность»: моды-оптимизации и настройки. Работает после перезапуска.
#[tauri::command]
async fn apply_preset(state: State<'_, AppState>, id: String) -> Result<PresetResult> {
    state
        .manager
        .apply_preset(&id)
        .await
        .inspect_err(|e| tracing::warn!("Пресет сервера {id}: {e}"))
}

#[tauri::command]
async fn pregen_start(state: State<'_, AppState>, id: String, radius: u32) -> Result<PregenStatus> {
    state.manager.pregen_start(&id, radius).await
}

#[tauri::command]
async fn pregen_status(state: State<'_, AppState>, id: String) -> Result<PregenStatus> {
    state.manager.pregen_status(&id).await
}

#[tauri::command]
async fn pregen_cancel(state: State<'_, AppState>, id: String) -> Result<()> {
    state.manager.pregen_cancel(&id).await
}
