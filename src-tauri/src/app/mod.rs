//! Обвязка Tauri: состояние приложения, окно, трей, логи.

mod commands;
mod tray;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};

use crate::config::AppConfig;
use crate::events::{Event, EventSink};
use crate::manager::ServerManager;
use crate::net::Http;
use crate::paths::Paths;

/// Имя события, в которое ядро шлёт всё: строки консоли, статусы, прогресс установки.
pub const EVENT_NAME: &str = "hearth://event";

/// Сколько ждём сохранения миров при выходе, прежде чем завершить серверы принудительно.
const QUIT_STOP_TIMEOUT: Duration = Duration::from_secs(60);

pub struct AppState {
    pub manager: Arc<ServerManager>,
    pub config: Mutex<AppConfig>,
    pub paths: Paths,
    quitting: AtomicBool,
}

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: Event) {
        if let Err(e) = self.0.emit(EVENT_NAME, event) {
            tracing::debug!("Не удалось отправить событие в окно: {e}");
        }
    }
}

pub fn run() {
    let paths = Paths::from_system().expect("не удалось определить папки приложения");
    let _log_guard = init_logging(&paths.logs_dir());
    tracing::info!("Hearth {} запускается", env!("CARGO_PKG_VERSION"));

    let app = tauri::Builder::default()
        // Второй экземпляр не запускаем: два Hearth начали бы делить одни серверы.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let config = AppConfig::load(&paths.config_file());
            let http = Http::new(paths.cache_dir())?;
            let sink = Arc::new(TauriSink(app.handle().clone()));
            let manager = ServerManager::new(&paths, http, config.servers_dir(&paths), sink);
            app.manage(AppState {
                manager,
                config: Mutex::new(config),
                paths: paths.clone(),
                quitting: AtomicBool::new(false),
            });
            tray::create(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let to_tray = state
                    .config
                    .lock()
                    .map(|c| c.close_to_tray)
                    .unwrap_or(true);
                if to_tray && !state.quitting.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("не удалось запустить Hearth");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = &event {
            let state = app.state::<AppState>();
            if !state.quitting.load(Ordering::SeqCst) && state.manager.has_running() {
                // Выход с работающими серверами: сначала корректно их останавливаем.
                api.prevent_exit();
                request_quit(app);
            }
        }
    });
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Выход из приложения: `stop` всем серверам, ожидание сохранения, затем выход.
pub(crate) fn request_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let manager = state.manager.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if manager.has_running() {
            tracing::info!("Выход: останавливаю серверы");
            manager.stop_all(QUIT_STOP_TIMEOUT).await;
        }
        app.exit(0);
    });
}

fn init_logging(dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    use tracing_subscriber::EnvFilter;

    std::fs::create_dir_all(dir).ok()?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("hearth")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer.and(std::io::stderr))
        .init();
    Some(guard)
}
