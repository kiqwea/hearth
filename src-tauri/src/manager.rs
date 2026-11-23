//! Управление серверами: установка, запуск, остановка, консоль.
//!
//! Правила, которые здесь соблюдаются:
//! - сервер останавливается только командой `stop` с ожиданием сохранения мира;
//!   принудительное завершение — отдельная операция `kill`;
//! - процесс привязан к Job Object, чтобы не пережить падение Hearth;
//! - перед запуском проверяется, свободен ли порт.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{oneshot, watch};
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::events::{
    BackupsEvent, ConsoleEvent, Event, EventSink, InstallEvent, InstallProgress, InstallStage, StatusEvent,
};
use crate::java::{self, JavaManager};
use crate::net::Http;
use crate::paths::Paths;
use crate::process::{self, job::Job, ConsoleBuffer, ConsoleLine, ConsoleStream, ServerStatus};
use crate::providers;
use crate::servers::{
    self, CreateServerRequest, PlayitTunnel, PropertyEntry, ServerMeta, ServerPatch, ServerStore,
};
use uuid::Uuid;

use crate::backups::{self, BackupInfo, BackupKind, SavedWorld, WorldSource};
use crate::direct::{DirectInfo, DirectManager, DirectState};
use crate::friendpack::{self, PackFormat, PackOptions, PackSummary};
use crate::mods::{self, ModList, ModSearch};
use crate::pirate;
use crate::presets::{self, PregenStatus, PresetResult};
use crate::upgrade::{ModCompat, ModCompatStatus, UpdateInfo, UpdatePlan, UpdateResult};
use crate::players::{self, PlayerAction, PlayerInfo, ServerStats, StatsEvent};
use crate::rcon::{self, Rcon};
use crate::tunnel::api::PlayitApi;
use crate::tunnel::secret::SecretStore;
use crate::tunnel::PlayitManager;

/// Сколько ждём сохранения мира после `stop`, прежде чем предложить принудительное завершение.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerSummary {
    pub meta: ServerMeta,
    pub status: ServerStatus,
    /// Прямое подключение (если включено и Hearth уже что-то о нём знает).
    pub direct: Option<DirectInfo>,
    /// Память, процессор, TPS и игроки онлайн (пока сервер работает).
    pub stats: Option<ServerStats>,
}

/// Итог переключения пиратского режима.
#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PirateResult {
    pub meta: ServerMeta,
    /// Путь к бэкапу мира, сделанному перед переключением.
    pub backup: Option<String>,
    /// Скольким игрокам перенесли вещи и права на новые UUID.
    pub migrated: u32,
    /// Какой плагин авторизации поставлен.
    pub auth: Option<String>,
}

/// Итог попытки включить прямое подключение.
#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirectAttempt {
    pub meta: ServerMeta,
    /// Адрес для друзей, если напрямую получилось.
    pub address: Option<String>,
    /// Почему не получилось (тогда интерфейс предлагает playit).
    pub reason: Option<String>,
}

struct RunningServer {
    stdin: tokio::sync::Mutex<ChildStdin>,
    kill_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// Пользователь (или Hearth) попросил остановку: выход процесса не считается падением.
    stop_requested: AtomicBool,
    exited: watch::Receiver<bool>,
    port: u16,
    pid: Option<u32>,
    /// Служебный канал (RCON): порт и пароль этого запуска, соединение создаётся по требованию.
    rcon_port: u16,
    rcon_password: String,
    rcon: tokio::sync::Mutex<Option<Rcon>>,
    /// Разбудить сбор статистики раньше срока (игрок зашёл или вышел, выполнена команда).
    stats_wake: tokio::sync::Notify,
    is_paper: bool,
    /// Выключить, если никого нет столько минут (0 — не выключать). Меняется из настроек на ходу.
    auto_stop_minutes: AtomicU32,
}

#[derive(Default)]
struct Inner {
    running: HashMap<String, Arc<RunningServer>>,
    /// Серверы, которые сейчас устанавливаются, запускаются или удаляются.
    busy: HashSet<String>,
    /// Серверы, мир которых сейчас копируется.
    backing_up: HashSet<String>,
    statuses: HashMap<String, ServerStatus>,
    consoles: HashMap<String, ConsoleBuffer>,
    /// Последняя статистика запущенных серверов.
    stats: HashMap<String, ServerStats>,
}

/// Как часто обновлять статистику работающего сервера.
const STATS_EVERY: Duration = Duration::from_secs(5);

pub struct ServerManager {
    http: Http,
    java: JavaManager,
    store: RwLock<ServerStore>,
    sink: Arc<dyn EventSink>,
    inner: Mutex<Inner>,
    job: Option<Arc<Job>>,
    playit: Arc<PlayitManager>,
    direct: Arc<DirectManager>,
    /// Включение и отключение доступа для друзей идут по очереди, чтобы не создать два туннеля.
    access_ops: tokio::sync::Mutex<()>,
    /// Длина «минуты» автовыключения в миллисекундах. В живых тестах её сокращают.
    minute_ms: AtomicU64,
    /// Установка и удаление плагинов идут по очереди.
    mod_ops: tokio::sync::Mutex<()>,
}

/// Снимает отметку «занят», когда операция закончилась (в том числе с ошибкой).
struct BusyGuard<'a> {
    manager: &'a ServerManager,
    id: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.manager.lock().busy.remove(&self.id);
    }
}

/// Снимает отметку «копируется» и сообщает интерфейсу, что список копий мог измениться.
struct BackupGuard<'a> {
    manager: &'a ServerManager,
    id: String,
}

impl Drop for BackupGuard<'_> {
    fn drop(&mut self) {
        self.manager.lock().backing_up.remove(&self.id);
        self.manager.emit_backups(&self.id, false);
    }
}

/// Сколько ждать `save-all flush`: большой мир сохраняется не мгновенно.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(120);
/// Первый автобэкап — не раньше, чем через столько после запуска.
const FIRST_AUTO_BACKUP: Duration = Duration::from_secs(10 * 60);

/// «15 мин», «2 ч».
fn minutes_text(m: u32) -> String {
    if m >= 60 && m.is_multiple_of(60) { format!("{} ч", m / 60) } else { format!("{m} мин") }
}

/// Имя папки мира из server.properties.
fn level_name(store: &ServerStore, id: &str) -> String {
    store
        .read_properties(id)
        .ok()
        .and_then(|p| p.get("level-name").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "world".into())
}

impl ServerManager {
    pub fn new(paths: &Paths, http: Http, servers_dir: PathBuf, sink: Arc<dyn EventSink>) -> Arc<Self> {
        #[cfg(windows)]
        let secrets: Box<dyn SecretStore> = Box::new(crate::tunnel::secret::CredentialStore::new("Hearth/playit.gg"));
        #[cfg(not(windows))]
        let secrets: Box<dyn SecretStore> = Box::new(crate::tunnel::secret::MemoryStore::default());
        Self::with_secrets(paths, http, servers_dir, sink, secrets)
    }

    /// Как `new`, но со своим хранилищем ключа playit (для тестов).
    pub fn with_secrets(
        paths: &Paths,
        http: Http,
        servers_dir: PathBuf,
        sink: Arc<dyn EventSink>,
        secrets: Box<dyn SecretStore>,
    ) -> Arc<Self> {
        let job = match Job::new() {
            Ok(job) => Some(Arc::new(job)),
            Err(e) => {
                tracing::warn!("Не удалось создать Job Object: {e}");
                None
            }
        };
        let playit = PlayitManager::new(
            http.clone(),
            PlayitApi::new(http.client().clone()),
            paths.playit_dir(),
            secrets,
            sink.clone(),
            job.clone(),
        );
        let direct = DirectManager::new(sink.clone());
        Arc::new(Self {
            direct,
            java: JavaManager::new(paths.java_dir(), &paths.cache_dir(), http.clone()),
            http,
            store: RwLock::new(ServerStore::new(servers_dir)),
            sink,
            inner: Mutex::new(Inner::default()),
            job,
            playit,
            access_ops: tokio::sync::Mutex::new(()),
            minute_ms: AtomicU64::new(60_000),
            mod_ops: tokio::sync::Mutex::new(()),
        })
    }

    /// Для живых тестов: сколько длится «минута» автовыключения.
    #[doc(hidden)]
    pub fn set_minute_for_tests(&self, minute: Duration) {
        self.minute_ms.store(minute.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn playit(&self) -> &Arc<PlayitManager> {
        &self.playit
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    pub fn java(&self) -> &JavaManager {
        &self.java
    }

    pub fn store(&self) -> ServerStore {
        self.store.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Смена корневой папки серверов. Серверы не переносятся: интерфейс должен
    /// предупредить, что старые останутся в прежней папке.
    pub fn set_servers_dir(&self, dir: PathBuf) -> Result<()> {
        {
            let inner = self.lock();
            if !inner.running.is_empty() || !inner.busy.is_empty() {
                return Err(Error::invalid(
                    "Остановите все серверы, прежде чем менять папку серверов",
                ));
            }
        }
        *self.store.write().unwrap_or_else(|p| p.into_inner()) = ServerStore::new(dir);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn begin_busy(&self, id: &str) -> Result<BusyGuard<'_>> {
        let mut inner = self.lock();
        if inner.running.contains_key(id) {
            return Err(Error::AlreadyRunning);
        }
        if inner.backing_up.contains(id) {
            return Err(Error::invalid("Сейчас делается копия мира, подождите немного"));
        }
        if !inner.busy.insert(id.to_string()) {
            return Err(Error::Busy);
        }
        Ok(BusyGuard {
            manager: self,
            id: id.to_string(),
        })
    }

    fn begin_backup(&self, id: &str) -> Result<BackupGuard<'_>> {
        if !self.lock().backing_up.insert(id.to_string()) {
            return Err(Error::invalid("Копия мира уже делается, подождите немного"));
        }
        self.emit_backups(id, true);
        Ok(BackupGuard {
            manager: self,
            id: id.to_string(),
        })
    }

    fn emit_backups(&self, id: &str, in_progress: bool) {
        self.sink.emit(Event::Backups(BackupsEvent {
            server_id: id.to_string(),
            in_progress,
        }));
    }

    pub fn backup_in_progress(&self, id: &str) -> bool {
        self.lock().backing_up.contains(id)
    }

    fn running(&self, id: &str) -> Option<Arc<RunningServer>> {
        self.lock().running.get(id).cloned()
    }

    pub fn has_running(&self) -> bool {
        !self.lock().running.is_empty()
    }

    pub fn status(&self, id: &str) -> ServerStatus {
        self.lock()
            .statuses
            .get(id)
            .cloned()
            .unwrap_or(ServerStatus::Stopped)
    }

    fn set_status(&self, id: &str, status: ServerStatus) {
        self.lock().statuses.insert(id.to_string(), status.clone());
        self.sink.emit(Event::Status(StatusEvent {
            server_id: id.to_string(),
            status,
        }));
    }

    fn push_line(&self, id: &str, stream: ConsoleStream, text: String) {
        let line = self
            .lock()
            .consoles
            .entry(id.to_string())
            .or_default()
            .push(stream, text);
        self.sink.emit(Event::Console(ConsoleEvent {
            server_id: id.to_string(),
            line,
        }));
    }

    fn emit_install(&self, id: &str, progress: InstallProgress) {
        emit_install(&self.sink, id, progress);
    }

    pub fn console(&self, id: &str) -> Vec<ConsoleLine> {
        self.lock()
            .consoles
            .get(id)
            .map(ConsoleBuffer::snapshot)
            .unwrap_or_default()
    }

    // ---- серверы на диске ----

    pub fn list(&self) -> Vec<ServerSummary> {
        self.store()
            .list()
            .into_iter()
            .map(|meta| self.summary(meta))
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<ServerSummary> {
        Ok(self.summary(self.store().load(id)?))
    }

    fn summary(&self, meta: ServerMeta) -> ServerSummary {
        ServerSummary {
            status: self.status(&meta.id),
            direct: self.direct.info(&meta.id),
            stats: self.lock().stats.get(&meta.id).cloned(),
            meta,
        }
    }

    pub async fn create(&self, req: CreateServerRequest) -> Result<ServerMeta> {
        self.store().create(req).await
    }

    pub async fn update(&self, id: &str, patch: ServerPatch) -> Result<ServerMeta> {
        let store = self.store();
        let mut meta = store.load(id)?;
        if let Some(name) = patch.name {
            meta.name = servers::validate_name(&name)?;
        }
        if let Some(mb) = patch.memory_mb {
            meta.memory_mb = servers::validate_memory(mb)?;
        }
        if let Some(auto) = patch.auto_backup {
            meta.auto_backup = auto.validate()?;
        }
        if let Some(start) = patch.start_with_app {
            meta.start_with_app = start;
        }
        if let Some(minutes) = patch.auto_stop_minutes {
            meta.auto_stop_minutes = servers::validate_auto_stop(minutes)?;
        }
        store.save(&meta).await?;
        // Автовыключение работает сразу, без перезапуска.
        if let Some(running) = self.running(id) {
            running.auto_stop_minutes.store(meta.auto_stop_minutes, Ordering::SeqCst);
        }
        Ok(meta)
    }

    // ---- миры и резервные копии ----

    /// Копии мира сервера, новые первыми.
    pub fn backups(&self, id: &str) -> Result<Vec<BackupInfo>> {
        servers::validate_id(id)?;
        Ok(backups::list(&self.store().backups_dir(id)))
    }

    /// Пока сервер работает, он постоянно дописывает мир. Чтобы копия была
    /// целой: `save-off` → `save-all flush` → работа с файлами → `save-on`
    /// (даже если работа не удалась).
    async fn with_saving_paused<T>(
        running: &RunningServer,
        work: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let prefix = players::command_prefix(running.is_paper);
        Self::rcon_command(running, &format!("{prefix}save-off")).await?;
        let result = async {
            Self::rcon_long(running, &format!("{prefix}save-all flush"), FLUSH_TIMEOUT).await?;
            work.await
        }
        .await;
        if let Err(e) = Self::rcon_command(running, &format!("{prefix}save-on")).await {
            tracing::warn!("Не удалось вернуть автосохранение мира (save-on): {e}");
        }
        result
    }

    /// Делает копию мира сейчас. Работает и на запущенном сервере.
    /// `None`, если мира ещё нет (сервер ни разу не запускался).
    pub async fn backup(&self, id: &str, kind: BackupKind) -> Result<Option<BackupInfo>> {
        let store = self.store();
        let meta = store.load(id)?;
        let game_dir = store.game_dir(id);
        let level = level_name(&store, id);
        let dir = store.backups_dir(id);
        let path = match self.running(id) {
            Some(running) => {
                if self.status(id) != ServerStatus::Running {
                    return Err(Error::invalid("Подождите, пока сервер запустится"));
                }
                let _backup = self.begin_backup(id)?;
                Self::with_saving_paused(&running, backups::backup_world(&game_dir, &level, &dir, kind)).await?
            }
            None => {
                let _busy = self.begin_busy(id)?;
                let _backup = self.begin_backup(id)?;
                backups::backup_world(&game_dir, &level, &dir, kind).await?
            }
        };
        let Some(path) = path else { return Ok(None) };
        if kind == BackupKind::Auto {
            let keep = meta.auto_backup.keep as usize;
            let dir = dir.clone();
            let removed = tokio::task::spawn_blocking(move || backups::prune_auto(&dir, keep)).await?;
            if removed > 0 {
                tracing::info!("Сервер {id}: удалено старых автобэкапов: {removed}");
            }
        }
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Ok(backups::list(&dir).into_iter().find(|b| b.file == file))
    }

    pub async fn delete_backup(&self, id: &str, file: &str) -> Result<()> {
        servers::validate_id(id)?;
        let path = backups::backup_path(&self.store().backups_dir(id), file)?;
        tokio::fs::remove_file(path).await?;
        self.emit_backups(id, false);
        Ok(())
    }

    /// Копирует файл копии туда, куда выбрал пользователь.
    pub async fn export_backup(&self, id: &str, file: &str, dest: &str) -> Result<()> {
        servers::validate_id(id)?;
        let path = backups::backup_path(&self.store().backups_dir(id), file)?;
        tokio::fs::copy(path, dest).await?;
        Ok(())
    }

    /// Мир одним zip для одиночной игры. Работает и на запущенном сервере.
    pub async fn export_world(&self, id: &str, dest: &str) -> Result<()> {
        let store = self.store();
        let meta = store.load(id)?;
        let game_dir = store.game_dir(id);
        let level = level_name(&store, id);
        let dest = std::path::Path::new(dest);
        let work = backups::export_world(&game_dir, &level, &meta.name, dest);
        match self.running(id) {
            Some(running) => {
                if self.status(id) != ServerStatus::Running {
                    return Err(Error::invalid("Подождите, пока сервер запустится"));
                }
                let _backup = self.begin_backup(id)?;
                Self::with_saving_paused(&running, work).await
            }
            None => {
                let _busy = self.begin_busy(id)?;
                work.await
            }
        }
    }

    fn ensure_stopped(&self, id: &str, what: &str) -> Result<()> {
        if self.running(id).is_some() {
            return Err(Error::invalid(format!("Остановите сервер, чтобы {what}")));
        }
        Ok(())
    }

    /// Возвращает мир из копии. Текущий мир перед этим тоже сохраняется в копию.
    /// Возвращает эту страховочную копию.
    pub async fn restore_backup(&self, id: &str, file: &str) -> Result<Option<BackupInfo>> {
        self.ensure_stopped(id, "восстановить копию мира")?;
        let _busy = self.begin_busy(id)?;
        let store = self.store();
        store.load(id)?;
        let dir = store.backups_dir(id);
        let source = backups::backup_path(&dir, file)?;
        self.replace_world(&store, id, WorldSource::Zip(&source), BackupKind::BeforeRestore).await
    }

    /// Ставит на сервер мир из одиночной игры (папка с level.dat) или из zip.
    /// Текущий мир перед этим сохраняется в копию.
    pub async fn import_world(&self, id: &str, source: &str) -> Result<Option<BackupInfo>> {
        self.ensure_stopped(id, "заменить мир")?;
        let _busy = self.begin_busy(id)?;
        let store = self.store();
        store.load(id)?;
        let path = std::path::Path::new(source);
        let source = if path.is_dir() {
            backups::check_world_folder(path)?;
            WorldSource::Folder(path)
        } else if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            WorldSource::Zip(path)
        } else {
            return Err(Error::invalid("Выберите папку мира или zip-архив с миром"));
        };
        self.replace_world(&store, id, source, BackupKind::BeforeImport).await
    }

    async fn replace_world(
        &self,
        store: &ServerStore,
        id: &str,
        source: WorldSource<'_>,
        kind: BackupKind,
    ) -> Result<Option<BackupInfo>> {
        let game_dir = store.game_dir(id);
        let level = level_name(store, id);
        let dir = store.backups_dir(id);
        let safety = {
            let _backup = self.begin_backup(id)?;
            backups::backup_world(&game_dir, &level, &dir, kind).await?
        };
        backups::replace_world(source, &game_dir, &level, &store.staging_dir(id)).await?;
        tracing::info!("Сервер {id}: мир заменён ({kind:?})");
        let safety_file = safety.map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned());
        Ok(safety_file.and_then(|f| backups::list(&dir).into_iter().find(|b| b.file == f)))
    }

    /// Миры одиночной игры на этом компьютере.
    pub async fn saved_worlds(&self) -> Result<Vec<SavedWorld>> {
        Ok(tokio::task::spawn_blocking(backups::saved_worlds).await?)
    }

    // ---- плагины и моды ----

    /// Плагины или моды сервера с названиями и доступными обновлениями.
    pub async fn mods(&self, id: &str) -> Result<ModList> {
        let store = self.store();
        let meta = store.load(id)?;
        let mut list = mods::list(&self.http, meta.loader, &meta.game_version, &store.game_dir(id)).await?;
        for item in &mut list.items {
            item.nick_protection = meta.auth_plugin.as_deref() == Some(item.file.as_str());
        }
        Ok(list)
    }

    pub async fn search_mods(&self, id: &str, query: &str, offset: u32) -> Result<ModSearch> {
        let meta = self.store().load(id)?;
        mods::search(&self.http, meta.loader, &meta.game_version, query, offset).await
    }

    /// Ставит плагин или мод с зависимостями. На работающем сервере можно
    /// поставить новый (заработает после перезапуска), но не обновить старый:
    /// его файл занят.
    pub async fn install_mod(&self, id: &str, project: &str) -> Result<Vec<String>> {
        let _ops = self.mod_ops.lock().await;
        let store = self.store();
        let meta = store.load(id)?;
        let running = self.running(id).is_some();
        let _busy = if running { None } else { Some(self.begin_busy(id)?) };
        let files = mods::install(&self.http, meta.loader, &meta.game_version, &store.game_dir(id), project, !running).await?;
        tracing::info!("Сервер {id}: поставлено {files:?}");
        self.follow_auth_plugin(&store, id, &files).await?;
        Ok(files.into_iter().map(|f| f.file).collect())
    }

    // ---- сборка модов для друзей ----

    /// Адрес для друзей, если Hearth его знает: постоянный playit или прямой.
    fn friend_address(&self, meta: &ServerMeta) -> Option<String> {
        if let Some(address) = meta.playit.as_ref().and_then(|t| self.playit.address(&t.tunnel_id)) {
            return Some(address);
        }
        let host = self.direct.info(&meta.id).and_then(|d| d.address)?;
        Some(if meta.port == servers::DEFAULT_PORT || host.contains(':') { host } else { format!("{host}:{}", meta.port) })
    }

    /// Что попадёт в сборку (без скачивания).
    pub async fn pack_preview(&self, id: &str, opts: PackOptions) -> Result<PackSummary> {
        let store = self.store();
        let meta = store.load(id)?;
        friendpack::preview(&self.http, &meta, &store.game_dir(id), opts, self.friend_address(&meta)).await
    }

    /// Собирает файл сборки для друзей в `dest`.
    pub async fn export_pack(&self, id: &str, opts: PackOptions, format: PackFormat, dest: &str) -> Result<PackSummary> {
        let store = self.store();
        let meta = store.load(id)?;
        let summary = friendpack::build(
            &self.http,
            &meta,
            &store.game_dir(id),
            opts,
            format,
            std::path::Path::new(dest),
            self.friend_address(&meta),
        )
        .await?;
        tracing::info!("Сервер {id}: сборка для друзей ({format:?}): {:?}", summary.mods);
        Ok(summary)
    }

    // ---- обновление сервера ----

    /// Есть ли новая сборка ядра этой версии игры и новые версии Minecraft.
    pub async fn check_update(&self, id: &str) -> Result<UpdateInfo> {
        let meta = self.store().load(id)?;
        let versions = providers::list_versions(&self.http, meta.loader, false).await?;
        let new_build = if meta.loader == providers::Loader::Vanilla {
            None
        } else {
            match providers::resolve(&self.http, meta.loader, &meta.game_version).await {
                Ok(artifact) => artifact.loader_version.filter(|b| meta.loader_version.as_ref() != Some(b)),
                Err(e) => {
                    tracing::warn!("Не удалось проверить сборку {} {}: {e}", meta.loader.display_name(), meta.game_version);
                    None
                }
            }
        };
        Ok(UpdateInfo {
            newer_versions: providers::newer_stable(&versions, &meta.game_version),
            current_version: meta.game_version,
            current_build: meta.loader_version,
            new_build,
        })
    }

    /// Какие моды и плагины есть под версию `target`.
    pub async fn plan_update(&self, id: &str, target: &str) -> Result<UpdatePlan> {
        let store = self.store();
        let meta = store.load(id)?;
        let mut compat = Vec::new();
        if target != meta.game_version && mods::folder_name(meta.loader).is_some() {
            let list = mods::list(&self.http, meta.loader, &meta.game_version, &store.game_dir(id)).await?;
            if !list.online && !list.items.is_empty() {
                return Err(Error::Network("не удалось проверить моды на Modrinth".into()));
            }
            for item in list.items {
                let (status, new_version) = match &item.project_id {
                    Some(project) => match mods::find_file(&self.http, meta.loader, target, project).await? {
                        Some(file) => (ModCompatStatus::Ready, Some(file.version)),
                        None => (ModCompatStatus::Missing, None),
                    },
                    None => (ModCompatStatus::Unknown, None),
                };
                compat.push(ModCompat { title: item.title, file: item.file, status, new_version, client_side: item.client_side });
            }
        }
        Ok(UpdatePlan {
            target: target.to_string(),
            mods: compat,
            disable_missing: meta.loader.is_modded(),
        })
    }

    /// Обновляет сервер. `target = None` — свежая сборка той же версии игры.
    /// Порядок: копия мира → новое ядро (и Java) → моды и плагины под новую
    /// версию. Если ядро не скачалось, сервер остаётся на прежней версии.
    pub async fn update_server(&self, id: &str, target: Option<&str>) -> Result<UpdateResult> {
        self.ensure_stopped(id, "обновить его")?;
        let _busy = self.begin_busy(id)?;
        let store = self.store();
        let old = store.load(id)?;
        let target = target.unwrap_or(&old.game_version).to_string();
        let changes_version = target != old.game_version;
        if changes_version {
            let versions = providers::list_versions(&self.http, old.loader, false).await?;
            if !providers::newer_stable(&versions, &old.game_version).contains(&target) {
                return Err(Error::invalid(format!("Обновиться можно только на более новую версию, а не на {target}")));
            }
        }
        let plan = if changes_version { Some(self.plan_update(id, &target).await?) } else { None };

        let game_dir = store.game_dir(id);
        let dir = store.backups_dir(id);
        let backup = {
            let _backup = self.begin_backup(id)?;
            backups::backup_world(&game_dir, &level_name(&store, id), &dir, BackupKind::BeforeUpdate).await?
        };
        let backup = backup
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .and_then(|f| backups::list(&dir).into_iter().find(|b| b.file == f));

        // Новое ядро. При ошибке возвращаем прежние метаданные: старый jar на месте.
        let mut next = old.clone();
        next.game_version = target.clone();
        store.save(&next).await?;
        let old_jar = old.jar.clone();
        let mut meta = match self.install_locked(id).await {
            Ok(meta) => meta,
            Err(e) => {
                store.save(&old).await?;
                return Err(e);
            }
        };
        if let Some(old_jar) = old_jar.filter(|j| Some(j) != meta.jar.as_ref()) {
            let _ = tokio::fs::remove_file(game_dir.join(old_jar)).await;
        }

        let mut updated = Vec::new();
        let mut disabled = Vec::new();
        let mut notes = Vec::new();
        if let Some(plan) = plan {
            let folder = game_dir.join(mods::folder_name(meta.loader).unwrap_or("mods"));
            for item in plan.mods {
                match item.status {
                    ModCompatStatus::Ready => {
                        // Ищем проект по файлу заново: после обновления зависимостей имя могло смениться.
                        let list = mods::list(&self.http, meta.loader, &old.game_version, &game_dir).await?;
                        let Some(project) = list.items.iter().find(|m| m.file == item.file).and_then(|m| m.project_id.clone()) else {
                            continue; // уже обновлён как зависимость
                        };
                        match mods::install(&self.http, meta.loader, &target, &game_dir, &project, true).await {
                            Ok(files) => {
                                self.follow_auth_plugin(&store, id, &files).await?;
                                updated.extend(files.into_iter().map(|f| f.file));
                            }
                            Err(e) => notes.push(format!("{}: не обновился ({e})", item.title)),
                        }
                    }
                    ModCompatStatus::Missing if plan.disable_missing => {
                        let off = game_dir.join("mods-disabled");
                        tokio::fs::create_dir_all(&off).await?;
                        crate::fsx::rename_with_retry(&folder.join(&item.file), &off.join(&item.file)).await?;
                        disabled.push(item.file.clone());
                        notes.push(format!("{}: ещё не вышел под {target}, убран в папку mods-disabled", item.title));
                    }
                    ModCompatStatus::Missing => {
                        notes.push(format!("{}: нет версии под {target}; оставлен, возможно, заработает и так", item.title))
                    }
                    ModCompatStatus::Unknown => {
                        notes.push(format!("{}: файл не с Modrinth, проверьте его совместимость сами", item.title))
                    }
                }
            }
            // Плагин защиты ников убран: защита больше не работает.
            meta = store.load(id)?;
            if meta.auth_plugin.as_ref().is_some_and(|f| disabled.contains(f)) {
                meta.auth_plugin = None;
                meta.nick_protection = false;
                store.save(&meta).await?;
                notes.push("Защита ников выключена: её мод ещё не вышел под новую версию".into());
            }
        }
        tracing::info!(
            "Сервер {id}: обновлён {} {} → {} (сборка {:?}), моды: {updated:?}, убраны: {disabled:?}",
            meta.loader.display_name(),
            old.game_version,
            meta.game_version,
            meta.loader_version
        );
        Ok(UpdateResult { meta, backup, updated, disabled, notes })
    }

    /// Hearth помнит плагин защиты ников по имени файла: если его обновили,
    /// запоминаем новое имя.
    async fn follow_auth_plugin(&self, store: &ServerStore, id: &str, files: &[mods::InstalledFile]) -> Result<()> {
        let mut meta = store.load(id)?;
        let Some(current) = meta.auth_plugin.clone() else { return Ok(()) };
        if let Some(new) = files.iter().find(|f| f.replaced.as_deref() == Some(current.as_str())) {
            meta.auth_plugin = Some(new.file.clone());
            store.save(&meta).await?;
        }
        Ok(())
    }

    /// Обновляет все плагины или моды, для которых вышли новые версии.
    pub async fn update_mods(&self, id: &str) -> Result<Vec<String>> {
        self.ensure_stopped(id, "обновить плагины и моды")?;
        let list = self.mods(id).await?;
        let mut updated = Vec::new();
        for item in list.items.iter().filter(|m| m.update.is_some()) {
            if let Some(project) = &item.project_id {
                updated.extend(self.install_mod(id, project).await?);
            }
        }
        Ok(updated)
    }

    pub async fn remove_mod(&self, id: &str, file: &str) -> Result<()> {
        self.ensure_stopped(id, "удалить плагин или мод")?;
        let _ops = self.mod_ops.lock().await;
        let _busy = self.begin_busy(id)?;
        let store = self.store();
        let meta = store.load(id)?;
        if meta.auth_plugin.as_deref() == Some(file) {
            return Err(Error::invalid("Это защита ников: выключить её можно в настройках сервера, раздел «Доступ»"));
        }
        let path = mods::mod_path(&store.game_dir(id), meta.loader, file)?;
        tokio::fs::remove_file(path).await?;
        tracing::info!("Сервер {id}: удалён {file}");
        Ok(())
    }

    // ---- пресет «Производительность» и предгенерация ----

    /// Моды-оптимизации (Fabric), server.properties и настройки Paper.
    /// Всё заработает после перезапуска. Моды без стабильной версии под эту
    /// версию игры пропускаются с объяснением.
    pub async fn apply_preset(&self, id: &str) -> Result<PresetResult> {
        let store = self.store();
        let meta = store.load(id)?;
        let mut result = PresetResult::default();

        if meta.loader.is_modded() {
            for (project, title) in presets::FABRIC_MODS {
                match mods::find_file(&self.http, meta.loader, &meta.game_version, project).await {
                    Ok(Some(file)) if file.stable => match self.install_mod(id, project).await {
                        Ok(files) if files.is_empty() => result.already.push(title.to_string()),
                        Ok(files) => result.installed.extend(files),
                        Err(e) => result.skipped.push(format!("{title}: {e}")),
                    },
                    Ok(Some(_)) => result.skipped.push(format!("{title}: под {} только тестовая версия", meta.game_version)),
                    Ok(None) => result.skipped.push(format!("{title}: нет версии под {}", meta.game_version)),
                    Err(e) => result.skipped.push(format!("{title}: {e}")),
                }
            }
        }

        let mut props = store.read_properties(id)?;
        let changes = presets::tune_properties(meta.loader, &mut props);
        if !changes.is_empty() {
            store.write_properties(id, &props).await?;
            result.settings.extend(changes);
        }

        if meta.loader.is_paper_like() {
            let path = store.game_dir(id).join("config").join("paper-world-defaults.yml");
            match tokio::fs::read_to_string(&path).await {
                Ok(mut text) => {
                    let mut changed = false;
                    for (key, value, what) in presets::PAPER_WORLD {
                        if presets::set_yaml_scalar(&mut text, key, value) == Some(true) {
                            result.settings.push(what.to_string());
                            changed = true;
                        }
                    }
                    if changed {
                        crate::fsx::write_atomic(&path, text.as_bytes()).await?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => result.paper_config_missing = true,
                Err(e) => return Err(e.into()),
            }
        }
        tracing::info!("Сервер {id}: пресет «Производительность»: {result:?}");
        Ok(result)
    }

    /// Запущенный и полностью загрузившийся сервер, на котором работает Chunky.
    async fn chunky_server(&self, id: &str) -> Result<Arc<RunningServer>> {
        let running = self
            .running(id)
            .ok_or_else(|| Error::invalid("Запустите сервер, чтобы предгенерировать мир"))?;
        if self.status(id) != ServerStatus::Running {
            return Err(Error::invalid("Подождите, пока сервер запустится"));
        }
        let reply = Self::rcon_command(&running, "chunky").await?;
        if is_unknown_command(&reply) {
            let meta = self.store().load(id)?;
            let installed = mods::list(&self.http, meta.loader, &meta.game_version, &self.store().game_dir(id))
                .await
                .is_ok_and(|l| l.items.iter().any(|m| m.project_id.as_deref() == Some(presets::CHUNKY)));
            return Err(Error::invalid(if installed {
                "Chunky поставлен, но ещё не загружен: перезапустите сервер"
            } else {
                "Для предгенерации нужен Chunky: поставьте его кнопкой выше"
            }));
        }
        Ok(running)
    }

    /// Предгенерация мира вокруг точки появления радиусом `radius` блоков.
    pub async fn pregen_start(&self, id: &str, radius: u32) -> Result<PregenStatus> {
        if !(100..=50_000).contains(&radius) {
            return Err(Error::invalid("Радиус: от 100 до 50 000 блоков"));
        }
        let running = self.chunky_server(id).await?;
        for command in ["chunky spawn".to_string(), format!("chunky radius {radius}")] {
            let reply = Self::rcon_command(&running, &command).await?;
            tracing::info!("Chunky {command}: {reply}");
        }
        let reply = Self::rcon_command(&running, "chunky start").await?;
        tracing::info!("Chunky start: {reply}");
        self.push_line(id, ConsoleStream::System, format!("Предгенерация мира: радиус {radius} блоков"));
        if reply.to_lowercase().contains("already") {
            return Err(Error::invalid("Предгенерация уже идёт"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.pregen_status(id).await
    }

    pub async fn pregen_status(&self, id: &str) -> Result<PregenStatus> {
        let running = self.chunky_server(id).await?;
        let reply = Self::rcon_command(&running, "chunky progress").await?;
        Ok(presets::parse_progress(&reply))
    }

    pub async fn pregen_cancel(&self, id: &str) -> Result<()> {
        let running = self.chunky_server(id).await?;
        let reply = Self::rcon_command(&running, "chunky cancel").await?;
        tracing::info!("Chunky cancel: {reply}");
        // Chunky просит подтвердить отмену.
        if reply.to_lowercase().contains("confirm") {
            let reply = Self::rcon_command(&running, "chunky confirm").await?;
            tracing::info!("Chunky confirm: {reply}");
        }
        Ok(())
    }

    /// Удаляет сервер с диска. `delete_tunnel`: удалить и его адрес в аккаунте
    /// playit (если не получится, например без интернета, сервер всё равно удаляется).
    pub async fn delete(&self, id: &str, delete_tunnel: bool) -> Result<()> {
        let _guard = self.begin_busy(id)?;
        let store = self.store();
        let tunnel = store.load(id).ok().and_then(|m| m.playit);
        store.delete(id).await?;
        {
            let mut inner = self.lock();
            inner.statuses.remove(id);
            inner.consoles.remove(id);
        }
        self.playit.release(id).await;
        self.direct.forget(id);
        if let (true, Some(tunnel)) = (delete_tunnel, tunnel) {
            if let Err(e) = self.playit.delete_tunnel(&tunnel.tunnel_id).await {
                tracing::warn!("Не удалось удалить туннель playit {} сервера {id}: {e}", tunnel.tunnel_id);
            }
        }
        Ok(())
    }

    // ---- доступ для друзей (playit.gg) ----

    /// Порт сервера. Источник правды — server.properties: его могли поправить вручную.
    fn current_port(&self, store: &ServerStore, id: &str, meta: &ServerMeta) -> u16 {
        store
            .read_properties(id)
            .ok()
            .and_then(|p| p.get("server-port").and_then(|p| p.trim().parse::<u16>().ok()))
            .unwrap_or(meta.port)
    }

    /// Создаёт серверу постоянный адрес для друзей. Если сервер запущен,
    /// адрес заработает сразу.
    pub async fn enable_friend_access(self: &Arc<Self>, id: &str) -> Result<ServerMeta> {
        let _ops = self.access_ops.lock().await;
        let store = self.store();
        let mut meta = store.load(id)?;
        if let Some(tunnel) = &meta.playit {
            // Туннель есть. Создаём новый, только если его удалили в панели playit.
            if !self.playit.tunnel_missing(&tunnel.tunnel_id).await {
                return Ok(meta);
            }
        }
        let port = self.current_port(&store, id, &meta);
        let tunnel_id = self.playit.create_tunnel(id, port).await?;
        meta.playit = Some(PlayitTunnel {
            tunnel_id,
            local_port: port,
            created_at: chrono::Utc::now(),
        });
        store.save(&meta).await?;
        if self.running(id).is_some() {
            self.start_friend_access(id, &meta, port);
        }
        Ok(meta)
    }

    // ---- пиратский режим ----

    /// Включает или выключает вход с пиратских клиентов. Сервер должен быть
    /// остановлен. Порядок: бэкап мира → плагин авторизации → перенос вещей
    /// игроков на новые UUID → server.properties.
    ///
    /// `protect`: поставить защиту ников паролем (AuthMe/EasyAuth). Для
    /// маленькой компании друзей её можно не ставить. Если плагин не вышел
    /// под эту версию игры, вернётся `AuthUnavailable`, и интерфейс предложит
    /// включить без защиты.
    pub async fn set_pirate_mode(&self, id: &str, enable: bool, protect: bool) -> Result<PirateResult> {
        let _guard = self.begin_busy(id)?;
        let store = self.store();
        let mut meta = store.load(id)?;
        let mut props = store.read_properties(id)?;
        let game_dir = store.game_dir(id);
        let level = props.get("level-name").unwrap_or("world").to_string();
        if meta.pirate_mode == enable {
            return Ok(PirateResult { meta, backup: None, migrated: 0, auth: None });
        }

        let plugin = pirate::auth_plugin_for(meta.loader).filter(|_| protect);

        let backup = backups::backup_world(
            &game_dir,
            &level,
            &store.backups_dir(id),
            BackupKind::Pirate,
        )
        .await?;

        let mut auth = None;
        if enable {
            if let Some(plugin) = &plugin {
                let file = pirate::install_auth(&self.http, plugin, &game_dir, &meta.game_version).await?;
                meta.auth_plugin = Some(file);
                auth = Some(plugin.title.to_string());
            }
            meta.nick_protection = meta.auth_plugin.is_some();
        } else {
            remove_auth_plugin(&mut meta, &game_dir).await;
        }

        // Вещи, достижения и права игроков: на новые UUID.
        let players = pirate::known_players(&game_dir);
        let map: HashMap<Uuid, Uuid> = if enable {
            players.iter().map(|(name, uuid)| (*uuid, pirate::offline_uuid(name))).collect()
        } else {
            let names: Vec<String> = players.iter().map(|(n, _)| n.clone()).collect();
            match pirate::online_uuids(&self.http, &names).await {
                // В пиратском режиме файлы игрока лежат под UUID, посчитанным из ника.
                Ok(online) => players
                    .iter()
                    .filter_map(|(name, _)| Some((pirate::offline_uuid(name), *online.get(&name.to_lowercase())?)))
                    .collect(),
                Err(e) => {
                    tracing::warn!("Не удалось узнать лицензионные UUID, вещи игроков не перенесены: {e}");
                    HashMap::new()
                }
            }
        };
        let map: HashMap<Uuid, Uuid> = map.into_iter().filter(|(a, b)| a != b).collect();
        let migrated = {
            let world = game_dir.join(&level);
            let gd = game_dir.clone();
            let map = map.clone();
            tokio::task::spawn_blocking(move || {
                let n = pirate::migrate_player_files(&world, &map);
                pirate::migrate_lists(&gd, &map);
                n
            })
            .await?
        };

        props.set("online-mode", if enable { "false" } else { "true" });
        // Пиратские клиенты не подписывают чат: без этого им нельзя писать сообщения.
        props.set("enforce-secure-profile", if enable { "false" } else { "true" });
        store.write_properties(id, &props).await?;

        meta.pirate_mode = enable;
        store.save(&meta).await?;
        tracing::info!(
            "Пиратский режим сервера {id}: {} (авторизация: {auth:?}, перенесено игроков: {migrated})",
            if enable { "включён" } else { "выключен" }
        );
        Ok(PirateResult {
            meta,
            backup: backup.map(|p| p.to_string_lossy().into_owned()),
            migrated: migrated as u32,
            auth,
        })
    }

    /// Включает или убирает защиту ников (пароль при входе) на сервере в
    /// пиратском режиме. Сервер должен быть остановлен. Пароли игроков при
    /// выключении сохраняются в данных плагина.
    pub async fn set_nick_protection(&self, id: &str, on: bool) -> Result<ServerMeta> {
        let _guard = self.begin_busy(id)?;
        let store = self.store();
        let mut meta = store.load(id)?;
        if !meta.pirate_mode {
            return Err(Error::invalid("Защита ников нужна только при входе с пиратских клиентов"));
        }
        let game_dir = store.game_dir(id);
        if on {
            let plugin = pirate::auth_plugin_for(meta.loader)
                .ok_or_else(|| Error::AuthUnavailable(format!("Для {} Hearth пока не умеет ставить защиту ников", meta.loader.display_name())))?;
            if meta.auth_plugin.is_none() {
                meta.auth_plugin = Some(pirate::install_auth(&self.http, &plugin, &game_dir, &meta.game_version).await?);
            }
        } else {
            remove_auth_plugin(&mut meta, &game_dir).await;
        }
        meta.nick_protection = on;
        store.save(&meta).await?;
        Ok(meta)
    }

    /// Пробует прямое подключение (проброс порта на роутере). Если получилось,
    /// сервер дальше работает напрямую, без playit. Если нет, в ответе причина,
    /// и интерфейс предлагает playit.
    pub async fn try_direct_access(self: &Arc<Self>, id: &str) -> Result<DirectAttempt> {
        let _ops = self.access_ops.lock().await;
        let store = self.store();
        let mut meta = store.load(id)?;
        let port = self.current_port(&store, id, &meta);
        let running = self.running(id).is_some();
        match self.direct.probe(id, port, running).await {
            Ok(address) => {
                tracing::info!("Прямое подключение для сервера {id}: {address}");
                meta.direct_access = true;
                store.save(&meta).await?;
                // Интерфейс перечитает сервер и увидит, что доступ включён.
                self.direct.notify(id);
                Ok(DirectAttempt {
                    meta,
                    address: Some(address),
                    reason: None,
                })
            }
            Err(e) => {
                tracing::info!("Прямое подключение для сервера {id} недоступно: {e}");
                Ok(DirectAttempt {
                    meta,
                    address: None,
                    reason: Some(e.to_string()),
                })
            }
        }
    }

    /// Убирает адрес для друзей: проброс порта снимается, туннель удаляется из аккаунта playit.
    pub async fn disable_friend_access(&self, id: &str) -> Result<ServerMeta> {
        let _ops = self.access_ops.lock().await;
        let store = self.store();
        let mut meta = store.load(id)?;
        if meta.direct_access {
            meta.direct_access = false;
            self.direct.forget(id);
            store.save(&meta).await?;
            self.sink.emit(Event::Direct(crate::direct::DirectEvent {
                server_id: id.to_string(),
                info: DirectInfo::default(),
            }));
            return Ok(meta);
        }
        if let Some(tunnel) = &meta.playit {
            match self.playit.delete_tunnel(&tunnel.tunnel_id).await {
                Ok(()) => {}
                // Без интернета пусть попробует позже: иначе туннель останется в аккаунте.
                Err(e @ Error::Network(_)) => return Err(e),
                // Остальное (playit не подключён, отказ API) не должно мешать отключить доступ:
                // агент для этого сервера больше не запускается, а туннель можно удалить в панели playit.
                Err(e) => tracing::warn!("Туннель playit {} не удалён: {e}", tunnel.tunnel_id),
            }
        }
        meta.playit = None;
        store.save(&meta).await?;
        self.playit.release(id).await;
        Ok(meta)
    }

    /// Доступ для друзей ещё не настроен: Hearth делает это сам при запуске.
    /// Сначала напрямую (порт на роутере), иначе playit, если он уже подключён.
    /// Если playit не подключён, интерфейс предложит войти в него: без
    /// пользователя этого не сделать.
    fn auto_friend_access(self: &Arc<Self>, id: &str) {
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            this.direct.set_state(&id, DirectState::Opening, None);
            let reason = match this.try_direct_access(&id).await {
                Ok(attempt) if attempt.address.is_some() => {
                    if let Some(address) = attempt.address {
                        this.push_line(&id, ConsoleStream::System, format!("Адрес для друзей: {address}"));
                    }
                    return;
                }
                Ok(attempt) => attempt.reason.unwrap_or_default(),
                Err(e) => e.to_string(),
            };
            if this.running(&id).is_none() {
                this.direct.set_state(&id, DirectState::Off, None);
                return;
            }
            this.direct.set_state(&id, DirectState::Failed, Some(reason.clone()));
            if this.playit.linked() {
                // Адрес появится через playit; enable сам поднимет агент для запущенного сервера.
                if let Err(e) = this.enable_friend_access(&id).await {
                    this.push_line(&id, ConsoleStream::System, format!("Доступ для друзей не включился: {e}"));
                }
            } else {
                this.push_line(
                    &id,
                    ConsoleStream::System,
                    format!("Напрямую друзья подключиться не смогут ({reason}). Войдите в playit.gg на вкладке «Обзор»."),
                );
            }
        });
    }

    /// Сервер с доступом для друзей запустился: поправить порт туннеля, если
    /// он поменялся, и поднять агент playit. Ошибки пишутся в консоль сервера.
    fn start_friend_access(self: &Arc<Self>, id: &str, meta: &ServerMeta, port: u16) {
        if meta.direct_access {
            self.direct.start(id, port);
            return;
        }
        let Some(tunnel) = meta.playit.clone() else {
            self.auto_friend_access(id);
            return;
        };
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            if tunnel.local_port != port {
                match this.playit.configure_port(&tunnel.tunnel_id, port).await {
                    Ok(()) => {
                        let _ops = this.access_ops.lock().await;
                        let store = this.store();
                        if let Ok(mut meta) = store.load(&id) {
                            if let Some(t) = meta.playit.as_mut().filter(|t| t.tunnel_id == tunnel.tunnel_id) {
                                t.local_port = port;
                                let _ = store.save(&meta).await;
                            }
                        }
                    }
                    Err(e) => this.push_line(&id, ConsoleStream::System, format!("Доступ для друзей: {e}")),
                }
            }
            if this.running(&id).is_none() {
                return;
            }
            let result = this.playit.acquire(&id).await;
            if this.running(&id).is_none() {
                // Сервер успел остановиться, пока запускался агент.
                this.playit.release(&id).await;
                return;
            }
            match result {
                Ok(()) => {
                    if let Some(address) = this.playit.address(&tunnel.tunnel_id) {
                        this.push_line(&id, ConsoleStream::System, format!("Адрес для друзей: {address}"));
                    }
                }
                Err(e) => this.push_line(&id, ConsoleStream::System, format!("Доступ для друзей не включился: {e}")),
            }
        });
    }

    pub fn properties(&self, id: &str) -> Result<Vec<PropertyEntry>> {
        Ok(self
            .store()
            .read_properties(id)?
            .entries()
            .into_iter()
            .map(|(key, value)| PropertyEntry { key, value })
            .collect())
    }

    /// Меняет server.properties. На работающем сервере изменения вступят в силу
    /// после перезапуска.
    pub async fn set_properties(&self, id: &str, entries: Vec<PropertyEntry>) -> Result<()> {
        let store = self.store();
        let mut meta = store.load(id)?;
        let mut props = store.read_properties(id)?;
        let mut port_changed = false;
        for entry in entries {
            let key = entry.key.trim();
            if key.is_empty() || key.contains(['\n', '\r']) || entry.value.contains(['\n', '\r']) {
                return Err(Error::invalid(format!("Некорректная настройка «{}»", entry.key)));
            }
            if key == "server-port" {
                let port: u16 = entry
                    .value
                    .trim()
                    .parse()
                    .ok()
                    .filter(|p| *p >= 1024)
                    .ok_or_else(|| Error::invalid("Порт должен быть числом от 1024 до 65535"))?;
                if port != meta.port {
                    meta.port = port;
                    port_changed = true;
                }
            }
            props.set(key, &entry.value);
        }
        store.write_properties(id, &props).await?;
        if port_changed {
            store.save(&meta).await?;
        }
        Ok(())
    }

    // ---- установка ----

    pub async fn install(&self, id: &str) -> Result<ServerMeta> {
        let _guard = self.begin_busy(id)?;
        self.install_locked(id).await
    }

    /// Установка, когда отметка «занят» уже взята вызывающим.
    async fn install_locked(&self, id: &str) -> Result<ServerMeta> {
        self.set_status(id, ServerStatus::Installing);
        let result = self.do_install(id).await;
        match &result {
            Ok(_) => self.emit_install(id, InstallProgress::step(InstallStage::Done, "Сервер установлен")),
            Err(e) => {
                self.emit_install(id, InstallProgress::step(InstallStage::Failed, e.to_string()));
                self.push_line(id, ConsoleStream::System, format!("Установка не удалась: {e}"));
            }
        }
        self.set_status(id, ServerStatus::Stopped);
        result
    }

    async fn do_install(&self, id: &str) -> Result<ServerMeta> {
        let store = self.store();
        let mut meta = store.load(id)?;
        let title = format!("{} {}", meta.loader.display_name(), meta.game_version);

        self.emit_install(
            id,
            InstallProgress::step(InstallStage::Resolving, format!("Получаю данные о {title}")),
        );
        let artifact = providers::resolve(&self.http, meta.loader, &meta.game_version).await?;

        let java_msg = format!("Подготавливаю Java {}", artifact.java_major);
        self.emit_install(id, InstallProgress::step(InstallStage::Java, java_msg.clone()));
        let (java_major, java_exe) = self
            .java
            .ensure(artifact.java_major, progress_reporter(&self.sink, id, InstallStage::Java, java_msg))
            .await?;

        let game_dir = store.game_dir(id);
        for spec in artifact.extra.iter().chain(std::iter::once(&artifact.jar)) {
            let msg = format!("Скачиваю {}", spec.file_name);
            self.emit_install(id, InstallProgress::step(InstallStage::Server, msg.clone()));
            self.http
                .download(
                    &spec.url,
                    &game_dir.join(&spec.file_name),
                    spec.checksum.as_ref(),
                    progress_reporter(&self.sink, id, InstallStage::Server, msg),
                )
                .await?;
        }

        // Пиратский режим выбран в мастере: ставим защиту ников вместе с сервером.
        if meta.pirate_mode && meta.nick_protection && meta.auth_plugin.is_none() {
            if let Some(plugin) = pirate::auth_plugin_for(meta.loader) {
                self.emit_install(
                    id,
                    InstallProgress::step(InstallStage::Server, format!("Ставлю защиту ников ({})", plugin.title)),
                );
                match pirate::install_auth(&self.http, &plugin, &game_dir, &meta.game_version).await {
                    Ok(file) => meta.auth_plugin = Some(file),
                    Err(Error::AuthUnavailable(reason)) => {
                        // Сервер всё равно запустится: без защиты, но с понятным объяснением.
                        meta.nick_protection = false;
                        self.push_line(
                            id,
                            ConsoleStream::System,
                            format!("Защита ников не поставлена: {reason}. Включить её можно позже в настройках сервера."),
                        );
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        // NeoForge и Forge: скачан установщик, запускаем его. Он сам качает
        // Minecraft и библиотеки и создаёт файл аргументов запуска.
        let launch = match &artifact.installer_args {
            Some(args_file) => {
                let msg = format!("Устанавливаю {title}: скачиваются библиотеки, это пара минут");
                self.emit_install(id, InstallProgress::step(InstallStage::Server, msg));
                self.run_installer(id, &java_exe, &game_dir, &artifact.jar.file_name).await?;
                if !game_dir.join(args_file).is_file() {
                    return Err(Error::other(format!(
                        "Установщик {} завершился, но не создал файл запуска. Подробности в консоли.",
                        meta.loader.display_name()
                    )));
                }
                args_file.clone()
            }
            None => artifact.jar.file_name.clone(),
        };

        meta.jar = Some(launch);
        meta.java_major = Some(java_major);
        meta.jvm_flags = artifact.jvm_flags;
        meta.loader_version = artifact.loader_version;
        meta.installed = true;
        store.save(&meta).await?;
        Ok(meta)
    }

    /// Запускает установщик NeoForge/Forge (`--install-server`) в папке сервера.
    /// Его вывод идёт в консоль сервера; после установки сам установщик и его
    /// лог удаляются.
    async fn run_installer(&self, id: &str, java: &std::path::Path, game_dir: &std::path::Path, installer: &str) -> Result<()> {
        let mut cmd = tokio::process::Command::new(java);
        // `--installServer` понимают и Forge, и NeoForge (у NeoForge есть ещё `--install-server`).
        cmd.args(["-jar", installer, "--installServer", "."])
            .current_dir(game_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| Error::other(format!("Не удалось запустить установщик: {e}")))?;
        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, child.raw_handle()) {
            let _ = job.assign(handle);
        }
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let pump = |reader: Box<dyn AsyncRead + Unpin + Send>| async move {
            let mut lines = BufReader::new(reader).lines();
            let mut out = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push(line);
            }
            out
        };
        let (status, out, err) = tokio::join!(child.wait(), pump(Box::new(stdout)), pump(Box::new(stderr)));
        let status = status?;
        // В консоль — только итог и ошибки: полный лог установщика очень длинный.
        // Только итог и настоящие ошибки: в именах библиотек и классов бывают
        // «errorprone» и «…FailedProblem», а успешных шагов сотни.
        for line in out.iter().chain(err.iter()).filter(|l| {
            l.to_lowercase().contains("installed successfully") || l.contains("ERROR") || l.contains("Exception:")
        }) {
            self.push_line(id, ConsoleStream::System, format!("Установщик: {line}"));
        }
        let _ = tokio::fs::remove_file(game_dir.join(installer)).await;
        let _ = tokio::fs::remove_file(game_dir.join(format!("{installer}.log"))).await;
        if !status.success() {
            for line in err.iter().rev().take(5).rev() {
                self.push_line(id, ConsoleStream::Stderr, line.clone());
            }
            return Err(Error::other(format!("Установщик завершился с ошибкой (код {})", status.code().unwrap_or(-1))));
        }
        Ok(())
    }

    // ---- запуск и остановка ----

    pub async fn start(self: &Arc<Self>, id: &str) -> Result<()> {
        let _guard = self.begin_busy(id)?;
        let store = self.store();
        let mut meta = store.load(id)?;
        let accepted_at = meta.eula_accepted_at.ok_or(Error::EulaNotAccepted)?;
        let game_dir = store.game_dir(id);

        let jar_missing = meta
            .jar
            .as_ref()
            .is_none_or(|jar| !game_dir.join(jar).is_file());
        if !meta.installed || jar_missing {
            meta = self.install_locked(id).await?;
        }
        let jar = meta
            .jar
            .clone()
            .ok_or_else(|| Error::other("Не удалось определить файл сервера"))?;

        // Источник правды для порта — server.properties: его могли поправить вручную.
        let mut props = store.read_properties(id)?;
        let port = props
            .get("server-port")
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(meta.port);
        if self.lock().running.values().any(|r| r.port == port) || !port_is_free(port) {
            return Err(Error::PortBusy(port));
        }

        // Служебный канал для списка игроков, TPS и команд из вкладки «Игроки»:
        // новый случайный порт и пароль при каждом запуске.
        let rcon_port = rcon::free_port().filter(|p| *p != port).unwrap_or(port.wrapping_add(10));
        let rcon_password = rcon::random_password();
        props.set("enable-rcon", "true");
        props.set("rcon.port", &rcon_port.to_string());
        props.set("rcon.password", &rcon_password);
        props.set("broadcast-rcon-to-ops", "false");
        store.write_properties(id, &props).await?;

        servers::write_eula(&game_dir, accepted_at).await?;
        // AuthMe создаёт config.yml при первом запуске: со второго запуска он говорит по-русски.
        if meta.pirate_mode && meta.loader.is_paper_like() && !meta.auth_lang_set && pirate::authme_russian(&game_dir) {
            meta.auth_lang_set = true;
            store.save(&meta).await?;
        }
        let required = meta
            .java_major
            .unwrap_or_else(|| java::fallback_major(&meta.game_version));
        let (java_major, java_exe) = self.java.ensure(required, |_, _| {}).await?;
        let args = process::build_args(&meta, &jar);

        if let Some(console) = self.lock().consoles.get_mut(id) {
            console.clear();
        }
        self.push_line(
            id,
            ConsoleStream::System,
            format!(
                "Запуск {} {} (Java {java_major}, память {} МБ, порт {port})",
                meta.loader.display_name(),
                meta.game_version,
                meta.memory_mb
            ),
        );
        self.set_status(id, ServerStatus::Starting);

        let mut cmd = tokio::process::Command::new(&java_exe);
        cmd.args(&args)
            .current_dir(&game_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.push_line(id, ConsoleStream::System, format!("Не удалось запустить Java: {e}"));
                self.set_status(id, ServerStatus::Stopped);
                return Err(Error::other(format!("Не удалось запустить Java: {e}")));
            }
        };

        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, child.raw_handle()) {
            if let Err(e) = job.assign(handle) {
                tracing::warn!("Не удалось привязать сервер {id} к Job Object: {e}");
            }
        }

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = watch::channel(false);
        let running = Arc::new(RunningServer {
            stdin: tokio::sync::Mutex::new(stdin),
            kill_tx: Mutex::new(Some(kill_tx)),
            stop_requested: AtomicBool::new(false),
            exited: exit_rx,
            port,
            pid: child.id(),
            rcon_port,
            rcon_password,
            rcon: tokio::sync::Mutex::new(None),
            stats_wake: tokio::sync::Notify::new(),
            is_paper: meta.loader.is_paper_like(),
            auto_stop_minutes: AtomicU32::new(meta.auto_stop_minutes),
        });
        self.lock().running.insert(id.to_string(), running.clone());
        self.spawn_stats(id, running.clone());

        let out_task = self.spawn_reader(id, stdout, ConsoleStream::Stdout);
        let err_task = self.spawn_reader(id, stderr, ConsoleStream::Stderr);

        let this = self.clone();
        let id_owned = id.to_string();
        let id = id.to_string();
        tokio::spawn(async move {
            let kill = async move {
                // Ошибка значит, что отправитель уничтожен без команды: ждём обычного выхода.
                if kill_rx.await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let exit = tokio::select! {
                status = child.wait() => status,
                _ = kill => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            // Даём дочитать последние строки вывода.
            let _ = tokio::time::timeout(Duration::from_secs(3), async {
                let _ = out_task.await;
                let _ = err_task.await;
            })
            .await;

            let code = exit.ok().and_then(|s| s.code());
            let requested = running.stop_requested.load(Ordering::SeqCst);
            {
                let mut inner = this.lock();
                inner.running.remove(&id);
                inner.stats.remove(&id);
            }
            let (status, message) = if requested || code == Some(0) {
                (ServerStatus::Stopped, "Сервер остановлен".to_string())
            } else {
                let code_text = code.map_or("неизвестен".to_string(), |c| c.to_string());
                (
                    ServerStatus::Crashed { code },
                    format!("Сервер неожиданно завершился (код {code_text}). Подробности в строках выше."),
                )
            };
            this.push_line(&id, ConsoleStream::System, message);
            this.set_status(&id, status);
            let _ = exit_tx.send(true);
            this.direct.stop(&id);
            this.playit.release(&id).await;
        });
        self.start_friend_access(&id_owned, &meta, port);
        Ok(())
    }

    fn spawn_reader<R>(self: &Arc<Self>, id: &str, reader: R, stream: ConsoleStream) -> tokio::task::JoinHandle<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut buf = Vec::with_capacity(256);
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) => break,
                    Ok(_) => this.on_output(&id, stream, process::decode_line(&buf)),
                    Err(e) => {
                        tracing::warn!("Чтение вывода сервера {id} прервано: {e}");
                        break;
                    }
                }
            }
        })
    }

    fn on_output(&self, id: &str, stream: ConsoleStream, text: String) {
        let ready = process::is_ready_line(&text);
        let hint = process::hint_for_line(&text);
        // Игрок зашёл или вышел: обновить список онлайн сразу, а не через 5 секунд.
        if text.contains(" joined the game") || text.contains(" left the game") {
            if let Some(running) = self.running(id) {
                running.stats_wake.notify_one();
            }
        }
        self.push_line(id, stream, text);
        if let Some(hint) = hint {
            self.push_line(id, ConsoleStream::System, hint.to_string());
        }
        if ready && self.status(id) == ServerStatus::Starting {
            self.set_status(id, ServerStatus::Running);
        }
    }

    // ---- игроки и статистика ----

    /// Команда через служебный канал (RCON). Соединение открывается при первой
    /// команде и переоткрывается, если оборвалось.
    async fn rcon_command(running: &RunningServer, command: &str) -> Result<String> {
        let mut guard = running.rcon.lock().await;
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(Rcon::connect(running.rcon_port, &running.rcon_password).await?);
            }
            match guard.as_mut().expect("connected").command(command).await {
                Ok(reply) => return Ok(reply),
                Err(e) if attempt == 0 => {
                    tracing::debug!("RCON оборвался ({e}), переподключаюсь");
                    *guard = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("цикл возвращает результат на второй попытке")
    }

    /// Долгая команда (`save-all flush`): без повтора, со своим временем ожидания.
    async fn rcon_long(running: &RunningServer, command: &str, timeout: Duration) -> Result<String> {
        let mut guard = running.rcon.lock().await;
        if guard.is_none() {
            *guard = Some(Rcon::connect(running.rcon_port, &running.rcon_password).await?);
        }
        let result = guard.as_mut().expect("connected").command_timeout(command, timeout).await;
        if result.is_err() {
            *guard = None;
        }
        result
    }

    /// Когда сделать первый автобэкап этого запуска: по расписанию от прошлой
    /// автоматической копии, но не раньше чем через 10 минут после запуска.
    fn first_auto_backup(&self, id: &str) -> Instant {
        let store = self.store();
        let every = store
            .load(id)
            .map(|m| Duration::from_secs(u64::from(m.auto_backup.every_minutes) * 60))
            .unwrap_or(FIRST_AUTO_BACKUP);
        let wait = match backups::last_auto(&store.backups_dir(id)) {
            Some(at) => every.saturating_sub((chrono::Utc::now() - at).to_std().unwrap_or_default()),
            None => Duration::ZERO,
        };
        Instant::now() + wait.max(FIRST_AUTO_BACKUP)
    }

    /// Автобэкап в фоне: ход и итог видны в консоли сервера.
    fn spawn_auto_backup(self: &Arc<Self>, id: &str) {
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            match this.backup(&id, BackupKind::Auto).await {
                Ok(Some(info)) => this.push_line(
                    &id,
                    ConsoleStream::System,
                    format!("Автобэкап мира готов ({} МБ)", info.size_bytes.div_ceil(1024 * 1024)),
                ),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Автобэкап сервера {id} не удался: {e}");
                    this.push_line(&id, ConsoleStream::System, format!("Автобэкап не удался: {e}"));
                }
            }
        });
    }

    /// Раз в несколько секунд: память и процессор процесса, игроки онлайн, TPS.
    fn spawn_stats(self: &Arc<Self>, id: &str, running: Arc<RunningServer>) {
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let mut exited = running.exited.clone();
            let mut sys = sysinfo::System::new().ok();
            let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
            let mut tps_supported = true;
            // Автовыключение: с какого момента на сервере никого нет.
            let mut empty_since: Option<Instant> = None;
            // Автобэкап: когда пора и играл ли кто-нибудь с прошлой копии.
            let mut next_backup = this.first_auto_backup(&id);
            let mut played = false;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(STATS_EVERY) => {}
                    _ = running.stats_wake.notified() => {
                        // Дать серверу дописать списки (например, ops.json) перед опросом.
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    _ = exited.changed() => break,
                }
                if *exited.borrow() {
                    break;
                }
                let mut stats = this.lock().stats.get(&id).cloned().unwrap_or_default();
                if let (Some(sys), Some(pid)) = (sys.as_mut(), running.pid) {
                    let pid = sysinfo::Pid::from_u32(pid);
                    sys.refresh_processes_specifics(
                        sysinfo::ProcessesToUpdate::Some(&[pid]),
                        true,
                        sysinfo::ProcessRefreshKind::nothing().with_memory().with_cpu(),
                    );
                    if let Some(p) = sys.process(pid) {
                        stats.memory_mb = Some((p.memory() / 1024 / 1024) as u32);
                        stats.cpu_percent = Some((p.cpu_usage() / cpus).clamp(0.0, 100.0));
                    }
                }
                // Игроки и TPS доступны только когда сервер уже полностью запустился.
                let mut online_known = false;
                if this.status(&id) == ServerStatus::Running {
                    let prefix = players::command_prefix(running.is_paper);
                    if let Ok(reply) = Self::rcon_command(&running, &format!("{prefix}list")).await {
                        if let Some((online, max)) = players::parse_list(&reply) {
                            stats.online = online;
                            stats.max_players = Some(max);
                            online_known = true;
                        }
                    }
                    if tps_supported {
                        let (cmd, parse): (&str, fn(&str) -> Option<f32>) = if running.is_paper {
                            ("tps", players::parse_paper_tps)
                        } else {
                            ("tick query", players::parse_tick_query)
                        };
                        match Self::rcon_command(&running, cmd).await.map(|r| parse(&r)) {
                            Ok(Some(tps)) => stats.tps = Some((tps * 10.0).round() / 10.0),
                            // Старая версия игры без такой команды: больше не спрашиваем.
                            Ok(None) => tps_supported = false,
                            Err(_) => {}
                        }
                    }
                }
                if this.running(&id).is_none() {
                    break;
                }
                if online_known && !running.stop_requested.load(Ordering::SeqCst) {
                    if stats.online.is_empty() {
                        let since = *empty_since.get_or_insert_with(Instant::now);
                        let minutes = running.auto_stop_minutes.load(Ordering::SeqCst);
                        let minute = Duration::from_millis(this.minute_ms.load(Ordering::Relaxed));
                        if minutes > 0 && since.elapsed() >= minute * minutes {
                            empty_since = None;
                            this.push_line(
                                &id,
                                ConsoleStream::System,
                                format!(
                                    "На сервере никого нет {}: выключаю, чтобы не тратить ресурсы компьютера. Это можно поменять в настройках сервера.",
                                    minutes_text(minutes)
                                ),
                            );
                            let this = this.clone();
                            let id = id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = this.stop(&id, STOP_TIMEOUT).await {
                                    tracing::warn!("Автовыключение сервера {id}: {e}");
                                }
                            });
                        }
                    } else {
                        empty_since = None;
                        played = true;
                    }
                    if played && Instant::now() >= next_backup && !this.backup_in_progress(&id) {
                        let auto = this.store().load(&id).map(|m| m.auto_backup).unwrap_or_default();
                        if auto.enabled {
                            played = false;
                            next_backup = Instant::now() + Duration::from_secs(u64::from(auto.every_minutes) * 60);
                            this.spawn_auto_backup(&id);
                        } else {
                            // Автобэкап выключен: проверим настройку ещё раз через минуту.
                            next_backup = Instant::now() + Duration::from_secs(60);
                        }
                    }
                }
                this.lock().stats.insert(id.clone(), stats.clone());
                this.sink.emit(Event::Stats(StatsEvent {
                    server_id: id.clone(),
                    stats,
                }));
            }
        });
    }

    /// Служебная команда запущенному серверу; ответ не попадает в консоль.
    pub async fn rcon(&self, id: &str, command: &str) -> Result<String> {
        let running = self.running(id).ok_or(Error::NotRunning)?;
        Self::rcon_command(&running, command).await
    }

    /// Игроки сервера: кто онлайн, операторы, белый список, баны.
    pub fn players(&self, id: &str) -> Result<Vec<PlayerInfo>> {
        servers::validate_id(id)?;
        let online = self.lock().stats.get(id).map(|s| s.online.clone()).unwrap_or_default();
        Ok(players::read_players(&self.store().game_dir(id), &online))
    }

    /// Кик, бан, оператор, белый список. Нужен запущенный сервер: изменения
    /// делает сам Minecraft, он же сохраняет списки в файлы.
    pub async fn player_action(&self, id: &str, action: PlayerAction, name: &str, reason: Option<&str>) -> Result<String> {
        let running = self.running(id).ok_or_else(|| Error::invalid("Запустите сервер, чтобы управлять игроками"))?;
        let command = players::action_command(action, name, reason, players::command_prefix(running.is_paper))?;
        if self.status(id) != ServerStatus::Running {
            return Err(Error::invalid("Подождите, пока сервер запустится"));
        }
        let reply = Self::rcon_command(&running, &command).await?;
        tracing::info!("Игроки сервера {id}: {command} → {reply}");
        running.stats_wake.notify_one();
        Ok(reply)
    }

    /// Отправляет `stop` и ждёт, пока сервер сохранит мир и завершится.
    pub async fn stop(&self, id: &str, timeout: Duration) -> Result<()> {
        let running = self.running(id).ok_or(Error::NotRunning)?;
        if !running.stop_requested.swap(true, Ordering::SeqCst) {
            self.set_status(id, ServerStatus::Stopping);
            self.push_line(id, ConsoleStream::Input, "stop".into());
            if let Err(e) = write_line(&running, "stop").await {
                tracing::warn!("Не удалось отправить stop серверу {id}: {e}");
            }
        }
        wait_exit(&running, timeout).await
    }

    /// Принудительное завершение. Интерфейс вызывает его только после
    /// предупреждения о том, что несохранённый прогресс может пропасть.
    pub fn kill(&self, id: &str) -> Result<()> {
        let running = self.running(id).ok_or(Error::NotRunning)?;
        running.stop_requested.store(true, Ordering::SeqCst);
        self.push_line(id, ConsoleStream::System, "Принудительное завершение сервера".into());
        if let Some(tx) = running.kill_tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    pub async fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        if self.running(id).is_some() {
            self.stop(id, STOP_TIMEOUT).await?;
        }
        self.start(id).await
    }

    pub async fn send_command(&self, id: &str, command: &str) -> Result<()> {
        let command = command.lines().next().unwrap_or("").trim();
        // В консоли сервера слэш не нужен, а старые версии на него ругаются.
        let command = command.strip_prefix('/').unwrap_or(command).trim();
        if command.is_empty() {
            return Err(Error::invalid("Введите команду"));
        }
        let running = self.running(id).ok_or(Error::NotRunning)?;
        if command.eq_ignore_ascii_case("stop") && !running.stop_requested.swap(true, Ordering::SeqCst) {
            self.set_status(id, ServerStatus::Stopping);
        }
        self.push_line(id, ConsoleStream::Input, command.to_string());
        write_line(&running, command)
            .await
            .map_err(|e| Error::other(format!("Не удалось отправить команду: {e}")))
    }

    /// Останавливает все серверы при выходе из приложения. Сервер, который не
    /// успел сохраниться за `timeout`, завершается принудительно.
    pub async fn stop_all(&self, timeout: Duration) {
        let ids: Vec<String> = self.lock().running.keys().cloned().collect();
        let tasks = ids.iter().map(|id| async move {
            if let Err(e) = self.stop(id, timeout).await {
                tracing::warn!("Сервер {id} не остановился ({e}), завершаю принудительно");
                if let Some(running) = self.running(id) {
                    let _ = self.kill(id);
                    let _ = wait_exit(&running, Duration::from_secs(10)).await;
                }
            }
        });
        futures_util::future::join_all(tasks).await;
    }
}

/// Убирает jar плагина авторизации. Его данные (пароли игроков) остаются:
/// при повторном включении защиты они пригодятся.
async fn remove_auth_plugin(meta: &mut ServerMeta, game_dir: &std::path::Path) {
    if let (Some(file), Some(folder)) = (meta.auth_plugin.take(), pirate::auth_folder(game_dir, meta.loader)) {
        let _ = tokio::fs::remove_file(folder.join(file)).await;
    }
    meta.nick_protection = false;
}

async fn write_line(running: &RunningServer, line: &str) -> std::io::Result<()> {
    let mut stdin = running.stdin.lock().await;
    stdin.write_all(format!("{line}\n").as_bytes()).await?;
    stdin.flush().await
}

async fn wait_exit(running: &RunningServer, timeout: Duration) -> Result<()> {
    let mut rx = running.exited.clone();
    let waited = tokio::time::timeout(timeout, async move {
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    waited.map_err(|_| Error::StopTimeout)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

fn emit_install(sink: &Arc<dyn EventSink>, id: &str, progress: InstallProgress) {
    sink.emit(Event::Install(InstallEvent {
        server_id: id.to_string(),
        progress,
    }));
}

/// Колбэк прогресса скачивания, который превращает байты в события установки.
fn progress_reporter(
    sink: &Arc<dyn EventSink>,
    id: &str,
    stage: InstallStage,
    message: String,
) -> impl FnMut(u64, Option<u64>) + Send + use<> {
    let sink = sink.clone();
    let id = id.to_string();
    move |downloaded, total| {
        emit_install(
            &sink,
            &id,
            InstallProgress {
                stage,
                message: message.clone(),
                downloaded: Some(downloaded),
                total,
            },
        );
    }
}

/// Ответ сервера на неизвестную команду (Vanilla/Fabric и Paper пишут по-разному).
fn is_unknown_command(reply: &str) -> bool {
    let lower = reply.to_lowercase();
    lower.contains("unknown or incomplete command") || lower.contains("unknown command")
}
