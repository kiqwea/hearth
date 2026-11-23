//! Единый тип ошибок приложения.
//!
//! Сообщения пишутся для пользователя (на языке интерфейса), во фронтенд ошибка уходит
//! как `{ kind, message }`: `kind` нужен интерфейсу, чтобы показать нужную
//! подсказку или кнопку, `message` показывается как есть.

use serde::ser::{Serialize, SerializeStruct, Serializer};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Network(String),
    Api {
        service: String,
        status: u16,
        message: String,
    },
    Checksum { file: String },
    Io(std::io::Error),
    Parse { service: String, message: String },
    ServerNotFound(String),
    VersionNotFound { loader: String, version: String },
    PortBusy(u16),
    AlreadyRunning,
    Busy,
    NotRunning,
    EulaNotAccepted,
    JavaUnavailable(u32),
    StopTimeout,
    PlayitNotLinked,
    PlayitKeyInvalid,
    /// Плагин авторизации для пиратского режима недоступен под эту версию игры.
    AuthUnavailable(String),
    InvalidInput(String),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::Network(e) => tr!(
                format!("Нет связи с интернетом или сервис недоступен: {e}"),
                format!("No internet connection or the service is unavailable: {e}")
            ),
            Error::Api { service, status, message } => tr!(
                format!("{service} ответил ошибкой ({status}): {message}"),
                format!("{service} returned an error ({status}): {message}")
            ),
            Error::Checksum { file } => tr!(
                format!("Файл {file} скачался повреждённым (не совпала контрольная сумма). Попробуйте ещё раз."),
                format!("{file} was corrupted while downloading (checksum mismatch). Please try again.")
            ),
            Error::Io(e) => tr!(format!("Ошибка работы с файлами: {e}"), format!("File error: {e}")),
            Error::Parse { service, message } => tr!(
                format!("Не удалось разобрать ответ {service}: {message}"),
                format!("Could not read the response from {service}: {message}")
            ),
            Error::ServerNotFound(id) => tr!(format!("Сервер «{id}» не найден"), format!("Server \"{id}\" was not found")),
            Error::VersionNotFound { loader, version } => tr!(
                format!("Версия {version} для {loader} не найдена"),
                format!("{loader} {version} was not found")
            ),
            Error::PortBusy(port) => tr!(
                format!("Порт {port} уже занят. Закройте программу, которая его использует, или выберите другой порт в настройках сервера."),
                format!("Port {port} is already in use. Close the program using it or pick another port in the server settings.")
            ),
            Error::AlreadyRunning => tr!("Сервер уже запущен", "The server is already running").into(),
            Error::Busy => tr!(
                "Сервер сейчас занят: дождитесь окончания установки или запуска",
                "The server is busy: wait for the installation or start to finish"
            )
            .into(),
            Error::NotRunning => tr!("Сервер не запущен", "The server is not running").into(),
            Error::EulaNotAccepted => tr!(
                "Чтобы запустить сервер, нужно принять EULA Mojang",
                "You need to accept the Mojang EULA to start the server"
            )
            .into(),
            Error::JavaUnavailable(major) => tr!(
                format!("Не удалось найти Java {major} для Windows у Eclipse Adoptium"),
                format!("Could not find Java {major} for Windows at Eclipse Adoptium")
            ),
            Error::StopTimeout => tr!(
                "Сервер не остановился вовремя. Можно подождать ещё или завершить его принудительно (несохранённые изменения мира могут пропасть).",
                "The server didn't stop in time. You can wait longer or force it to stop (unsaved world changes may be lost)."
            )
            .into(),
            Error::PlayitNotLinked => tr!(
                "Сначала подключите playit.gg: без этого Hearth не может выдать адрес для друзей",
                "Connect playit.gg first: without it Hearth can't give your friends an address"
            )
            .into(),
            Error::PlayitKeyInvalid => tr!(
                "Подключение к playit.gg больше не действует (например, агента удалили в аккаунте playit). Подключите playit заново.",
                "The playit.gg connection is no longer valid (for example, the agent was deleted in your playit account). Connect playit again."
            )
            .into(),
            Error::AuthUnavailable(s) | Error::InvalidInput(s) | Error::Other(s) => s.clone(),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl Error {
    pub fn other(message: impl Into<String>) -> Self {
        Error::Other(message.into())
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Error::InvalidInput(message.into())
    }

    /// Короткий машинный код для фронтенда.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Network(_) => "network",
            Error::Api { .. } => "api",
            Error::Checksum { .. } => "checksum",
            Error::Io(_) => "io",
            Error::Parse { .. } => "parse",
            Error::ServerNotFound(_) => "serverNotFound",
            Error::VersionNotFound { .. } => "versionNotFound",
            Error::PortBusy(_) => "portBusy",
            Error::AlreadyRunning => "alreadyRunning",
            Error::Busy => "busy",
            Error::NotRunning => "notRunning",
            Error::EulaNotAccepted => "eulaNotAccepted",
            Error::JavaUnavailable(_) => "javaUnavailable",
            Error::StopTimeout => "stopTimeout",
            Error::PlayitNotLinked => "playitNotLinked",
            Error::PlayitKeyInvalid => "playitKeyInvalid",
            Error::AuthUnavailable(_) => "authUnavailable",
            Error::InvalidInput(_) => "invalidInput",
            Error::Other(_) => "other",
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Api { status: 404, .. })
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        let reason = if e.is_timeout() {
            tr!("сервер не ответил вовремя", "the server did not respond in time").to_string()
        } else if e.is_connect() {
            tr!("не удалось подключиться", "could not connect").to_string()
        } else {
            // Цепочка причин даёт больше, чем верхнее сообщение reqwest.
            let mut text = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                text.push_str(": ");
                text.push_str(&s.to_string());
                source = s.source();
            }
            text
        };
        let host = e
            .url()
            .and_then(|u| u.host_str())
            .map(|h| format!(" ({h})"))
            .unwrap_or_default();
        Error::Network(format!("{reason}{host}"))
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Other(tr!(format!("Не удалось распаковать архив: {e}"), format!("Could not unpack the archive: {e}")))
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Error::Other(tr!(format!("Внутренняя ошибка фоновой задачи: {e}"), format!("Internal background task error: {e}")))
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("Error", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}
