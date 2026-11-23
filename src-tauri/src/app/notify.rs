//! Уведомления Windows о том, что происходит с серверами, пока Hearth свёрнут
//! или в трее: сервер запустился или упал, игрок зашёл, автобэкап, автовыключение.
//! Если окно Hearth открыто и в фокусе, хватает подсказок внутри приложения.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use super::AppState;
use crate::events::Event;
use crate::process::{ConsoleStream, ServerStatus};

/// Что сообщить: заголовок и текст.
pub fn message(event: &Event, server_name: &str) -> Option<(String, String)> {
    match event {
        Event::Status(s) => match s.status {
            ServerStatus::Running => Some((format!("Сервер «{server_name}» запущен"), "Друзья могут заходить".into())),
            ServerStatus::Crashed { .. } => Some((
                format!("Сервер «{server_name}» упал"),
                "Откройте Hearth: в консоли видно, что случилось".into(),
            )),
            _ => None,
        },
        Event::Console(c) => {
            let text = c.line.text.as_str();
            match c.line.stream {
                ConsoleStream::Stdout => joined_player(text)
                    .map(|name| (format!("{name} зашёл на сервер"), format!("Сервер «{server_name}»"))),
                ConsoleStream::System if text.starts_with("Автобэкап мира готов") => {
                    Some((format!("Бэкап «{server_name}» готов"), text.to_string()))
                }
                ConsoleStream::System if text.starts_with("Автобэкап не удался") => {
                    Some((format!("Бэкап «{server_name}» не удался"), text.to_string()))
                }
                ConsoleStream::System if text.starts_with("На сервере никого нет") => Some((
                    format!("Сервер «{server_name}» выключается"),
                    "На нём давно никого нет: Hearth сохраняет мир и выключает его".into(),
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `[12:00:00 INFO]: Steve joined the game` → `Steve`.
pub fn joined_player(line: &str) -> Option<&str> {
    let before = line.strip_suffix(" joined the game")?;
    // После префикса лога (`...]: `) должен стоять ровно один ник: в чате было бы `<Bob> ...`.
    let name = before.rsplit_once("]: ").map_or(before, |(_, rest)| rest).trim();
    let valid = !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some(name)
}

/// Показывает уведомление, если включены уведомления и окно не в фокусе.
pub fn on_event(app: &AppHandle, event: &Event) {
    let server_id = match event {
        Event::Status(s) => &s.server_id,
        Event::Console(c) => &c.server_id,
        _ => return,
    };
    // Сначала дешёвая проверка: строк консоли много, уведомление нужно редко.
    if message(event, "").is_none() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else { return };
    if !state.config.lock().map(|c| c.notifications).unwrap_or(true) {
        return;
    }
    let focused = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if focused {
        return;
    }
    let Ok(meta) = state.manager.store().load(server_id) else { return };
    let Some((title, body)) = message(event, &meta.name) else { return };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!("Уведомление не показано: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ConsoleEvent, StatusEvent};
    use crate::process::ConsoleLine;

    fn console(stream: ConsoleStream, text: &str) -> Event {
        Event::Console(ConsoleEvent { server_id: "s".into(), line: ConsoleLine { seq: 1, stream, text: text.into() } })
    }

    #[test]
    fn player_names() {
        assert_eq!(joined_player("[12:00:00 INFO]: Steve joined the game"), Some("Steve"));
        assert_eq!(joined_player("[12:00:00] [Server thread/INFO]: Alex_2 joined the game"), Some("Alex_2"));
        assert_eq!(joined_player("<Bob> who joined the game"), None, "чат, а не вход");
        assert_eq!(joined_player("Steve left the game"), None);
    }

    #[test]
    fn messages() {
        let running = Event::Status(StatusEvent { server_id: "s".into(), status: ServerStatus::Running });
        assert_eq!(message(&running, "Мир").unwrap().0, "Сервер «Мир» запущен");
        let starting = Event::Status(StatusEvent { server_id: "s".into(), status: ServerStatus::Starting });
        assert!(message(&starting, "Мир").is_none());
        let join = console(ConsoleStream::Stdout, "[12:00:00 INFO]: Steve joined the game");
        assert_eq!(message(&join, "Мир").unwrap().0, "Steve зашёл на сервер");
        let backup = console(ConsoleStream::System, "Автобэкап мира готов (12 МБ)");
        assert!(message(&backup, "Мир").unwrap().0.contains("Бэкап"));
        // Игрок в чате не может притвориться системным сообщением Hearth.
        assert!(message(&console(ConsoleStream::Stdout, "Автобэкап мира готов"), "Мир").is_none());
    }
}
