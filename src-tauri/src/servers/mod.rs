//! Серверы на диске: `servers/<id>/server.json` (метаданные) и `servers/<id>/server/`
//! (файлы самого Minecraft).

pub mod properties;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::fsx;
use crate::providers::Loader;
pub use properties::Properties;

const META_FILE: &str = "server.json";
const GAME_DIR: &str = "server";
const SCHEMA: u32 = 1;
pub const DEFAULT_PORT: u16 = 25565;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Edition {
    #[default]
    Java,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    pub fn as_property(self) -> &'static str {
        match self {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    pub fn as_property(self) -> &'static str {
        match self {
            Difficulty::Peaceful => "peaceful",
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hard => "hard",
        }
    }
}

fn schema_v1() -> u32 {
    SCHEMA
}

/// Содержимое `server.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerMeta {
    #[serde(default = "schema_v1")]
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub edition: Edition,
    pub loader: Loader,
    pub game_version: String,
    /// Сборка Paper или версия загрузчика Fabric. Заполняется при установке.
    #[serde(default)]
    pub loader_version: Option<String>,
    /// Имя jar в папке сервера. Заполняется при установке.
    #[serde(default)]
    pub jar: Option<String>,
    #[serde(default)]
    pub java_major: Option<u32>,
    pub memory_mb: u32,
    pub port: u16,
    #[serde(default)]
    pub jvm_flags: Vec<String>,
    #[serde(default)]
    pub installed: bool,
    pub eula_accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Постоянный адрес для друзей через playit.gg. `null`: доступ не включён.
    #[serde(default)]
    pub playit: Option<PlayitTunnel>,
    /// Доступ для друзей напрямую: Hearth открывает порт на роутере (UPnP).
    #[serde(default)]
    pub direct_access: bool,
}

/// Туннель playit этого сервера. Создаётся один раз и не меняется между запусками.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayitTunnel {
    pub tunnel_id: String,
    /// Порт, на который туннель ведёт игроков. Если порт сервера поменяли,
    /// Hearth перенастроит туннель при следующем запуске.
    pub local_port: u16,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateServerRequest {
    pub name: String,
    pub loader: Loader,
    pub game_version: String,
    pub memory_mb: u32,
    pub port: Option<u16>,
    pub game_mode: GameMode,
    pub difficulty: Difficulty,
    pub max_players: u32,
    pub seed: Option<String>,
    /// Пользователь прочитал и принял EULA Mojang в мастере.
    pub eula_accepted: bool,
}

/// Изменение метаданных из настроек сервера. `None` = не менять.
#[derive(Deserialize, Clone, Debug, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerPatch {
    pub name: Option<String>,
    pub memory_mb: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub struct PropertyEntry {
    pub key: String,
    pub value: String,
}

pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid("Введите название сервера"));
    }
    if name.chars().count() > 64 {
        return Err(Error::invalid("Название слишком длинное (до 64 символов)"));
    }
    Ok(name.to_string())
}

pub fn validate_memory(mb: u32) -> Result<u32> {
    if !(512..=65536).contains(&mb) {
        return Err(Error::invalid("Память для сервера должна быть от 512 МБ до 64 ГБ"));
    }
    Ok(mb)
}

/// id сервера идёт в путь к папке, поэтому разрешаем только безопасные символы.
pub fn validate_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(Error::ServerNotFound(id.to_string()))
    }
}

/// Читаемое имя папки: латиница из названия плюс случайный хвост.
pub fn make_id(name: &str) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 24 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "server" } else { slug };
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..6];
    format!("{slug}-{suffix}")
}

#[derive(Clone, Debug)]
pub struct ServerStore {
    root: PathBuf,
}

impl ServerStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn server_root(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Папка с файлами Minecraft (рабочая директория процесса).
    pub fn game_dir(&self, id: &str) -> PathBuf {
        self.server_root(id).join(GAME_DIR)
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.server_root(id).join(META_FILE)
    }

    fn properties_path(&self, id: &str) -> PathBuf {
        self.game_dir(id).join("server.properties")
    }

    pub fn list(&self) -> Vec<ServerMeta> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut list: Vec<ServerMeta> = entries
            .flatten()
            .filter(|e| e.path().join(META_FILE).is_file())
            .filter_map(|e| {
                let id = e.file_name().to_string_lossy().into_owned();
                match self.load(&id) {
                    Ok(meta) => Some(meta),
                    Err(err) => {
                        tracing::warn!("Пропускаю сервер {id}: {err}");
                        None
                    }
                }
            })
            .collect();
        list.sort_by_key(|m| m.created_at);
        list
    }

    pub fn load(&self, id: &str) -> Result<ServerMeta> {
        validate_id(id)?;
        let text = match std::fs::read_to_string(self.meta_path(id)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::ServerNotFound(id.to_string()))
            }
            Err(e) => return Err(e.into()),
        };
        let mut meta: ServerMeta = serde_json::from_str(&text).map_err(|e| Error::Parse {
            service: "server.json".into(),
            message: e.to_string(),
        })?;
        // Папка главнее записанного id: её могли переименовать вручную.
        meta.id = id.to_string();
        Ok(meta)
    }

    pub async fn save(&self, meta: &ServerMeta) -> Result<()> {
        validate_id(&meta.id)?;
        let data = serde_json::to_vec_pretty(meta).expect("meta serializes");
        fsx::write_atomic(&self.meta_path(&meta.id), &data).await?;
        Ok(())
    }

    pub async fn create(&self, req: CreateServerRequest) -> Result<ServerMeta> {
        if !req.eula_accepted {
            return Err(Error::EulaNotAccepted);
        }
        let name = validate_name(&req.name)?;
        let memory_mb = validate_memory(req.memory_mb)?;
        let game_version = req.game_version.trim().to_string();
        if game_version.is_empty() {
            return Err(Error::invalid("Выберите версию игры"));
        }
        if !(1..=1000).contains(&req.max_players) {
            return Err(Error::invalid("Количество слотов должно быть от 1 до 1000"));
        }
        let port = req.port.unwrap_or(DEFAULT_PORT);
        if port < 1024 {
            return Err(Error::invalid("Порт должен быть от 1024 до 65535"));
        }

        let id = make_id(&name);
        let now = Utc::now();
        let meta = ServerMeta {
            schema: SCHEMA,
            id: id.clone(),
            name: name.clone(),
            edition: Edition::Java,
            loader: req.loader,
            game_version,
            loader_version: None,
            jar: None,
            java_major: None,
            memory_mb,
            port,
            jvm_flags: Vec::new(),
            installed: false,
            eula_accepted_at: Some(now),
            created_at: now,
            playit: None,
            direct_access: false,
        };

        let game_dir = self.game_dir(&id);
        tokio::fs::create_dir_all(&game_dir).await?;

        let mut props = Properties::default();
        props.set("motd", &name);
        props.set("server-port", &port.to_string());
        props.set("gamemode", req.game_mode.as_property());
        props.set("difficulty", req.difficulty.as_property());
        props.set("max-players", &req.max_players.to_string());
        props.set("level-seed", req.seed.as_deref().unwrap_or("").trim());
        props.set("online-mode", "true");
        // Новые версии Minecraft (26.x) по умолчанию включают белый список,
        // и друзья не могут зайти. Hearth рассчитан на игру с друзьями,
        // поэтому выключаем его явно; включить можно в настройках сервера.
        props.set("white-list", "false");
        props.set("enable-rcon", "false");
        fsx::write_atomic(&self.properties_path(&id), props.to_text().as_bytes()).await?;
        write_eula(&game_dir, now).await?;

        self.save(&meta).await?;
        Ok(meta)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let dir = self.server_root(id);
        if !dir.join(META_FILE).is_file() {
            return Err(Error::ServerNotFound(id.to_string()));
        }
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(())
    }

    pub fn read_properties(&self, id: &str) -> Result<Properties> {
        validate_id(id)?;
        match std::fs::read(self.properties_path(id)) {
            Ok(bytes) => Ok(Properties::parse_bytes(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Properties::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn write_properties(&self, id: &str, props: &Properties) -> Result<()> {
        validate_id(id)?;
        fsx::write_atomic(&self.properties_path(id), props.to_text().as_bytes()).await?;
        Ok(())
    }
}

/// eula.txt пишем только после того, как пользователь принял EULA в мастере.
pub async fn write_eula(game_dir: &Path, accepted_at: DateTime<Utc>) -> Result<()> {
    let text = format!(
        "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\n#Accepted by the user in Hearth at {}\neula=true\n",
        accepted_at.to_rfc3339()
    );
    fsx::write_atomic(&game_dir.join("eula.txt"), text.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str) -> CreateServerRequest {
        CreateServerRequest {
            name: name.into(),
            loader: Loader::Paper,
            game_version: "26.3".into(),
            memory_mb: 4096,
            port: None,
            game_mode: GameMode::Survival,
            difficulty: Difficulty::Normal,
            max_players: 10,
            seed: Some("12345".into()),
            eula_accepted: true,
        }
    }

    #[test]
    fn id_is_safe_for_paths() {
        let id = make_id("Сервер с друзьями!");
        assert!(id.starts_with("server-"));
        validate_id(&id).unwrap();
        assert!(make_id("My Cool SMP").starts_with("my-cool-smp-"));
        assert!(validate_id("..\\evil").is_err());
        assert!(validate_id("a/b").is_err());
    }

    #[tokio::test]
    async fn create_writes_meta_properties_and_eula() {
        let dir = tempfile::tempdir().unwrap();
        let store = ServerStore::new(dir.path().to_path_buf());
        let meta = store.create(request("Тест")).await.unwrap();

        assert_eq!(store.list(), vec![meta.clone()]);
        let props = store.read_properties(&meta.id).unwrap();
        assert_eq!(props.get("motd"), Some("Тест"));
        assert_eq!(props.get("server-port"), Some("25565"));
        assert_eq!(props.get("level-seed"), Some("12345"));
        assert_eq!(props.get("white-list"), Some("false"));
        let eula = std::fs::read_to_string(store.game_dir(&meta.id).join("eula.txt")).unwrap();
        assert!(eula.contains("eula=true"));

        store.delete(&meta.id).await.unwrap();
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn create_requires_eula() {
        let dir = tempfile::tempdir().unwrap();
        let store = ServerStore::new(dir.path().to_path_buf());
        let mut req = request("x");
        req.eula_accepted = false;
        assert!(matches!(store.create(req).await, Err(Error::EulaNotAccepted)));
        assert!(store.list().is_empty());
    }
}
