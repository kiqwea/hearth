//! Единый тип ошибок приложения.
//!
//! Сообщения пишутся для пользователя (по-русски), во фронтенд ошибка уходит
//! как `{ kind, message }`: `kind` нужен интерфейсу, чтобы показать нужную
//! подсказку или кнопку, `message` показывается как есть.

use serde::ser::{Serialize, SerializeStruct, Serializer};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Нет связи с интернетом или сервис недоступен: {0}")]
    Network(String),

    #[error("{service} ответил ошибкой ({status}): {message}")]
    Api {
        service: String,
        status: u16,
        message: String,
    },

    #[error("Файл {file} скачался повреждённым (не совпала контрольная сумма). Попробуйте ещё раз.")]
    Checksum { file: String },

    #[error("Ошибка работы с файлами: {0}")]
    Io(#[from] std::io::Error),

    #[error("Не удалось разобрать ответ {service}: {message}")]
    Parse { service: String, message: String },

    #[error("Сервер «{0}» не найден")]
    ServerNotFound(String),

    #[error("Версия {version} для {loader} не найдена")]
    VersionNotFound { loader: String, version: String },

    #[error("Порт {0} уже занят. Закройте программу, которая его использует, или выберите другой порт в настройках сервера.")]
    PortBusy(u16),

    #[error("Сервер уже запущен")]
    AlreadyRunning,

    #[error("Сервер сейчас занят: дождитесь окончания установки или запуска")]
    Busy,

    #[error("Сервер не запущен")]
    NotRunning,

    #[error("Чтобы запустить сервер, нужно принять EULA Mojang")]
    EulaNotAccepted,

    #[error("Не удалось найти Java {0} для Windows у Eclipse Adoptium")]
    JavaUnavailable(u32),

    #[error("Сервер не остановился вовремя. Можно подождать ещё или завершить его принудительно (несохранённые изменения мира могут пропасть).")]
    StopTimeout,

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0}")]
    Other(String),
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
            "сервер не ответил вовремя".to_string()
        } else if e.is_connect() {
            "не удалось подключиться".to_string()
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
        Error::Other(format!("Не удалось распаковать архив: {e}"))
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Error::Other(format!("Внутренняя ошибка фоновой задачи: {e}"))
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
