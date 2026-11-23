//! Типы и помощники для процесса сервера: статус, буфер консоли, аргументы запуска.

pub mod job;

use std::collections::VecDeque;

use serde::Serialize;
use ts_rs::TS;

use crate::servers::ServerMeta;

/// Сколько строк консоли держим в памяти на сервер.
const CONSOLE_CAPACITY: usize = 5000;

#[derive(Serialize, Clone, Debug, PartialEq, Eq, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum ServerStatus {
    Stopped,
    Installing,
    Starting,
    Running,
    Stopping,
    /// Процесс завершился сам, без команды `stop`.
    Crashed { code: Option<i32> },
}

impl ServerStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ServerStatus::Starting | ServerStatus::Running | ServerStatus::Stopping
        )
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConsoleStream {
    Stdout,
    Stderr,
    /// Команда, которую ввёл пользователь.
    Input,
    /// Сообщение от самого Hearth.
    System,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConsoleLine {
    /// Сквозной номер строки: фронтенд по нему склеивает историю и живые события.
    #[ts(type = "number")]
    pub seq: u64,
    pub stream: ConsoleStream,
    pub text: String,
}

#[derive(Default, Debug)]
pub struct ConsoleBuffer {
    lines: VecDeque<ConsoleLine>,
    next_seq: u64,
}

impl ConsoleBuffer {
    pub fn push(&mut self, stream: ConsoleStream, text: String) -> ConsoleLine {
        let line = ConsoleLine {
            seq: self.next_seq,
            stream,
            text,
        };
        self.next_seq += 1;
        if self.lines.len() == CONSOLE_CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line.clone());
        line
    }

    pub fn snapshot(&self) -> Vec<ConsoleLine> {
        self.lines.iter().cloned().collect()
    }

    /// Очищает строки, но не сбрасывает нумерацию.
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// Строка из stdout/stderr без перевода строки. Кодировку задаём флагами
/// `-Dstdout.encoding=UTF-8`, но на всякий случай декодируем с заменой.
pub fn decode_line(bytes: &[u8]) -> String {
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Сервер дописал загрузку мира. Формат одинаковый у Vanilla, Paper и Fabric:
/// `Done (3.512s)! For help, type "help"`.
pub fn is_ready_line(line: &str) -> bool {
    line.contains("Done (") && line.contains("For help")
}

/// Подсказка по известным ошибкам в выводе сервера.
pub fn hint_for_line(line: &str) -> Option<&'static str> {
    if line.contains("FAILED TO BIND TO PORT") || line.contains("Failed to bind to port") {
        Some("Порт сервера занят другой программой или другим сервером. Измените порт в настройках.")
    } else if line.contains("Unrecognized VM option") || line.contains("Could not create the Java Virtual Machine") {
        Some("Java не приняла параметры запуска. Проверьте флаги JVM в настройках сервера.")
    } else if line.contains("UnsupportedClassVersionError") {
        Some("Ядру нужна более новая версия Java, чем установлена для этого сервера.")
    } else if line.contains("Could not reserve enough space") || line.contains("Cannot allocate memory") {
        Some("Не хватает оперативной памяти для выбранного объёма. Уменьшите память сервера.")
    } else {
        None
    }
}

/// Аргументы `java` для запуска сервера.
pub fn build_args(meta: &ServerMeta, jar: &str) -> Vec<String> {
    let mut args = vec![
        format!("-Xms{}M", meta.memory_mb),
        format!("-Xmx{}M", meta.memory_mb),
    ];
    args.extend(meta.jvm_flags.iter().cloned());
    // Вывод в UTF-8, иначе кириллица в консоли превратится в кракозябры
    // (на русской Windows Java по умолчанию пишет в CP1251).
    args.extend(
        [
            "-Dfile.encoding=UTF-8",
            "-Dstdout.encoding=UTF-8",
            "-Dstderr.encoding=UTF-8",
            "-Dsun.stdout.encoding=UTF-8",
            "-Dsun.stderr.encoding=UTF-8",
            // Лог сборок мусора: по нему Hearth видит, сколько памяти миру
            // реально нужно (график на вкладке «Мониторинг»).
            "-Xlog:gc:file=hearth-gc.log:uptime",
        ]
        .map(String::from),
    );
    if meta.loader.is_paper_like() {
        // Цвета в консоли без настоящего терминала.
        args.push("-Dterminal.jline=false".into());
        args.push("-Dterminal.ansi=true".into());
    }
    // NeoForge и Forge запускаются через файл аргументов, который создал их установщик.
    if jar.ends_with(".txt") {
        args.push(format!("@{jar}"));
    } else {
        args.push("-jar".into());
        args.push(jar.into());
    }
    args.push("nogui".into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_drops_oldest_but_keeps_numbering() {
        let mut b = ConsoleBuffer::default();
        for i in 0..(CONSOLE_CAPACITY + 10) {
            b.push(ConsoleStream::Stdout, i.to_string());
        }
        let snap = b.snapshot();
        assert_eq!(snap.len(), CONSOLE_CAPACITY);
        assert_eq!(snap[0].seq, 10);
        b.clear();
        assert_eq!(b.push(ConsoleStream::System, "x".into()).seq, (CONSOLE_CAPACITY + 10) as u64);
    }

    #[test]
    fn detects_ready_line() {
        assert!(is_ready_line(
            "[12:00:01] [Server thread/INFO]: Done (3.512s)! For help, type \"help\""
        ));
        assert!(!is_ready_line("[12:00:01] [Server thread/INFO]: Preparing spawn area: 50%"));
    }

    #[test]
    fn decodes_crlf_lines() {
        assert_eq!(decode_line("Привет\r\n".as_bytes()), "Привет");
    }
}
