//! Доступ для друзей через playit.gg.
//!
//! Как это устроено:
//! - один раз пользователь подтверждает в браузере подключение Hearth к своему
//!   аккаунту playit (claim-ссылка). Hearth получает секретный ключ агента и
//!   хранит его в Диспетчере учётных данных Windows;
//! - у каждого сервера свой постоянный туннель Minecraft Java, его id лежит в
//!   `server.json`. Туннель ведёт на `127.0.0.1:<порт сервера>`;
//! - агент playit (`playitd.exe`) один на все туннели. Hearth запускает его,
//!   когда запущен хотя бы один сервер с доступом для друзей, и выключает,
//!   когда таких серверов не осталось;
//! - адреса туннелей Hearth узнаёт у API playit и запоминает в
//!   `playit/tunnels.json`, чтобы показывать их и без интернета.

pub mod agent;
pub mod api;
pub mod secret;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::oneshot;
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::events::{Event, EventSink};
use crate::fsx;
use crate::net::Http;
use crate::process::job::Job;
use api::{ClaimSetup, PlayitApi, RunData};
use secret::SecretStore;

/// Сколько ждём, пока пользователь подтвердит подключение в браузере.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Если агент проработал столько и не упал, считаем, что он на связи.
const AGENT_SETTLE: Duration = Duration::from_secs(3);
/// Данные от playit старше этого обновляем при запросе статуса.
const STALE_AFTER: Duration = Duration::from_secs(60);
const CACHE_FILE: &str = "tunnels.json";
pub const DASHBOARD_URL: &str = "https://playit.gg/account/tunnels";

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlayitAgentState {
    #[default]
    Off,
    Downloading,
    Starting,
    Online,
    Error,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayitTunnelInfo {
    pub id: String,
    /// Адрес для друзей, например `name.gl.joinmc.link`. `null`, пока playit его не выдал.
    pub address: Option<String>,
    /// Туннель ещё создаётся: сообщение playit о том, что происходит.
    pub pending: Option<String>,
    /// Почему playit отключил туннель.
    pub disabled: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayitNotice {
    pub message: String,
    pub link: Option<String>,
    pub important: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayitStatus {
    /// Hearth подключён к аккаунту playit (ключ агента сохранён).
    pub linked: bool,
    /// Идёт подключение: ссылка, которую нужно открыть в браузере.
    pub claim_url: Option<String>,
    /// Подключение не удалось или было отменено с ошибкой.
    pub claim_error: Option<String>,
    pub agent: PlayitAgentState,
    pub agent_error: Option<String>,
    pub tunnels: Vec<PlayitTunnelInfo>,
    /// Список туннелей получен от playit в этом запуске Hearth. Если `false`,
    /// адреса взяты из кэша и судить, удалён ли туннель, нельзя.
    pub synced: bool,
    /// Гостевой аккаунт playit: адреса пропадут, если не привязать почту.
    pub guest: bool,
    pub notices: Vec<PlayitNotice>,
}

struct Claim {
    seq: u64,
    url: String,
}

struct AgentProcess {
    seq: u64,
    kill_tx: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct State {
    /// `None`: ещё не читали из хранилища.
    secret: Option<Option<String>>,
    claim: Option<Claim>,
    claim_seq: u64,
    claim_error: Option<String>,
    agent: PlayitAgentState,
    agent_error: Option<String>,
    agent_seq: u64,
    process: Option<AgentProcess>,
    /// Серверы с доступом для друзей, которые сейчас запущены.
    users: HashSet<String>,
    run: Option<RunData>,
    run_at: Option<Instant>,
    /// Кэш адресов: id туннеля → адрес.
    addresses: HashMap<String, String>,
}

pub struct PlayitManager {
    http: Http,
    api: PlayitApi,
    dir: PathBuf,
    secrets: Box<dyn SecretStore>,
    sink: Arc<dyn EventSink>,
    job: Option<Arc<Job>>,
    state: Mutex<State>,
    /// Запуск и остановка агента идут строго по очереди.
    agent_ops: tokio::sync::Mutex<()>,
    poller: AtomicBool,
}

/// Случайный код подтверждения: 10 шестнадцатеричных символов, как у официального агента.
pub(crate) fn new_claim_code() -> String {
    hex::encode(&uuid::Uuid::new_v4().as_bytes()[..5])
}

impl PlayitManager {
    pub fn new(
        http: Http,
        api: PlayitApi,
        dir: PathBuf,
        secrets: Box<dyn SecretStore>,
        sink: Arc<dyn EventSink>,
        job: Option<Arc<Job>>,
    ) -> Arc<Self> {
        let addresses = std::fs::read(dir.join(CACHE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Arc::new(Self {
            http,
            api,
            dir,
            secrets,
            sink,
            job,
            state: Mutex::new(State {
                addresses,
                ..State::default()
            }),
            agent_ops: tokio::sync::Mutex::new(()),
            poller: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn secret(&self) -> Option<String> {
        let mut st = self.lock();
        if st.secret.is_none() {
            st.secret = Some(self.secrets.load().unwrap_or_else(|e| {
                tracing::warn!("{e}");
                None
            }));
        }
        st.secret.clone().flatten()
    }

    fn require_secret(&self) -> Result<String> {
        self.secret().ok_or(Error::PlayitNotLinked)
    }

    pub fn linked(&self) -> bool {
        self.secret().is_some()
    }

    pub fn status(&self) -> PlayitStatus {
        let linked = self.linked();
        let st = self.lock();
        let (tunnels, synced) = match &st.run {
            Some(run) => {
                let mut list: Vec<PlayitTunnelInfo> = run
                    .tunnels
                    .iter()
                    .map(|t| PlayitTunnelInfo {
                        id: t.id.clone(),
                        address: Some(t.display_address.clone()).filter(|a| !a.is_empty()),
                        pending: None,
                        disabled: t.disabled_reason.clone(),
                    })
                    .collect();
                list.extend(run.pending.iter().map(|p| PlayitTunnelInfo {
                    id: p.id.clone(),
                    address: st.addresses.get(&p.id).cloned(),
                    pending: Some(p.status_msg.clone()),
                    disabled: None,
                }));
                (list, true)
            }
            None => (
                st.addresses
                    .iter()
                    .map(|(id, address)| PlayitTunnelInfo {
                        id: id.clone(),
                        address: Some(address.clone()),
                        pending: None,
                        disabled: None,
                    })
                    .collect(),
                false,
            ),
        };
        PlayitStatus {
            linked,
            claim_url: st.claim.as_ref().map(|c| c.url.clone()),
            claim_error: st.claim_error.clone(),
            agent: st.agent,
            agent_error: st.agent_error.clone(),
            tunnels,
            synced: synced && linked,
            guest: st.run.as_ref().is_some_and(RunData::is_guest),
            notices: st
                .run
                .iter()
                .flat_map(|r| r.notices.iter())
                .map(|n| PlayitNotice {
                    message: n.message.clone(),
                    link: n.resolve_link.clone(),
                    important: n.priority != "Low",
                })
                .collect(),
        }
    }

    fn emit(&self) {
        self.sink.emit(Event::Playit(self.status()));
    }

    /// Статус для интерфейса. Устаревшие данные о туннелях сначала обновляет
    /// (без интернета просто отдаёт то, что есть).
    pub async fn status_fresh(self: &Arc<Self>) -> PlayitStatus {
        let stale = self.lock().run_at.is_none_or(|t| t.elapsed() > STALE_AFTER);
        if stale && self.linked() {
            if let Err(e) = self.refresh().await {
                tracing::debug!("Не удалось обновить данные playit: {e}");
            }
        }
        self.status()
    }

    // ---- подключение аккаунта ----

    /// Начинает подключение к аккаунту playit. Возвращает ссылку, которую нужно
    /// открыть в браузере; дальше Hearth сам ждёт подтверждения.
    pub fn start_claim(self: &Arc<Self>) -> Result<String> {
        if self.linked() {
            return Err(Error::invalid("Hearth уже подключён к playit.gg"));
        }
        // Без среды tokio фоновая задача не запустится (а tokio::spawn упал бы).
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::other("Внутренняя ошибка: подключение playit запущено вне фоновой среды"))?;
        let (seq, code, url) = {
            let mut st = self.lock();
            if let Some(claim) = &st.claim {
                return Ok(claim.url.clone());
            }
            st.claim_seq += 1;
            let code = new_claim_code();
            let url = api::claim_url(&code);
            st.claim = Some(Claim {
                seq: st.claim_seq,
                url: url.clone(),
            });
            st.claim_error = None;
            (st.claim_seq, code, url)
        };
        self.emit();
        let this = self.clone();
        runtime.spawn(async move { this.run_claim(seq, code).await });
        Ok(url)
    }

    pub fn cancel_claim(&self) {
        {
            let mut st = self.lock();
            st.claim = None;
            st.claim_error = None;
        }
        self.emit();
    }

    fn claim_current(&self, seq: u64) -> bool {
        self.lock().claim.as_ref().is_some_and(|c| c.seq == seq)
    }

    fn finish_claim(&self, seq: u64, error: Option<String>) {
        {
            let mut st = self.lock();
            if !st.claim.as_ref().is_some_and(|c| c.seq == seq) {
                return;
            }
            st.claim = None;
            st.claim_error = error;
        }
        self.emit();
    }

    async fn run_claim(self: Arc<Self>, seq: u64, code: String) {
        match self.claim_secret(seq, &code).await {
            Ok(Some(secret)) => {
                if let Err(e) = self.secrets.save(&secret) {
                    self.finish_claim(seq, Some(e.to_string()));
                    return;
                }
                self.lock().secret = Some(Some(secret));
                tracing::info!("Hearth подключён к playit.gg");
                self.finish_claim(seq, None);
                if let Err(e) = self.refresh().await {
                    tracing::warn!("Не удалось получить данные агента playit: {e}");
                }
                // Если серверы с доступом для друзей уже запущены, поднимаем агент.
                if !self.lock().users.is_empty() {
                    let _ = self.ensure_agent().await;
                }
            }
            Ok(None) => {}
            Err(e) => self.finish_claim(seq, Some(e.to_string())),
        }
    }

    /// Ждёт подтверждения и меняет код на ключ. `Ok(None)`: подключение отменили.
    async fn claim_secret(&self, seq: u64, code: &str) -> Result<Option<String>> {
        let version = format!("Hearth {}", env!("CARGO_PKG_VERSION"));
        let deadline = Instant::now() + CLAIM_TIMEOUT;
        loop {
            if !self.claim_current(seq) {
                return Ok(None);
            }
            if Instant::now() > deadline {
                return Err(Error::other("Подключение к playit не подтвердили вовремя. Попробуйте ещё раз."));
            }
            match self.api.claim_setup(code, &version).await {
                Ok(Ok(ClaimSetup::UserAccepted)) => break,
                Ok(Ok(ClaimSetup::UserRejected)) => {
                    return Err(Error::other("Подключение отклонили на сайте playit.gg"));
                }
                Ok(Ok(_)) => {}
                Ok(Err(code)) => {
                    return Err(Error::other(format!(
                        "Ссылка подключения playit больше не действует ({code}). Попробуйте ещё раз."
                    )));
                }
                // Нет сети: просто ждём, пользователь может ещё входить в аккаунт.
                Err(e) => tracing::debug!("playit claim/setup: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if !self.claim_current(seq) {
                return Ok(None);
            }
            match self.api.claim_exchange(code).await {
                Ok(Ok(secret)) => return Ok(Some(secret)),
                Ok(Err(code)) if code == "UserRejected" => {
                    return Err(Error::other("Подключение отклонили на сайте playit.gg"));
                }
                Ok(Err(code)) if Instant::now() > deadline => {
                    return Err(Error::other(format!("playit.gg не выдал ключ агента ({code}). Попробуйте ещё раз.")));
                }
                Err(e) if Instant::now() > deadline => return Err(e),
                _ => {}
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Отключает Hearth от аккаунта playit. Туннели в аккаунте остаются.
    pub async fn unlink(&self) -> Result<()> {
        self.stop_agent().await;
        self.secrets.delete()?;
        {
            let mut st = self.lock();
            st.secret = Some(None);
            st.run = None;
            st.run_at = None;
            st.claim = None;
            st.agent_error = None;
        }
        self.emit();
        Ok(())
    }

    // ---- данные от playit ----

    /// Свежие данные агента: туннели, адреса, уведомления.
    pub async fn refresh(&self) -> Result<RunData> {
        let secret = self.require_secret()?;
        match self.api.rundata(&secret).await {
            Ok(run) => {
                let changed_cache = {
                    let mut st = self.lock();
                    let mut changed = false;
                    for t in &run.tunnels {
                        if !t.display_address.is_empty() && st.addresses.get(&t.id) != Some(&t.display_address) {
                            st.addresses.insert(t.id.clone(), t.display_address.clone());
                            changed = true;
                        }
                    }
                    st.run = Some(run.clone());
                    st.run_at = Some(Instant::now());
                    changed.then(|| st.addresses.clone())
                };
                if let Some(addresses) = changed_cache {
                    let data = serde_json::to_vec_pretty(&addresses).expect("map serializes");
                    if let Err(e) = fsx::write_atomic(&self.dir.join(CACHE_FILE), &data).await {
                        tracing::warn!("Не удалось сохранить адреса playit: {e}");
                    }
                }
                self.emit();
                Ok(run)
            }
            Err(Error::PlayitKeyInvalid) => {
                tracing::warn!("Ключ агента playit больше не действует, отключаю Hearth от playit");
                let _ = self.unlink().await;
                self.lock().claim_error = Some(Error::PlayitKeyInvalid.to_string());
                self.emit();
                Err(Error::PlayitKeyInvalid)
            }
            Err(e) => Err(e),
        }
    }

    /// `true`, если playit точно не знает о таком туннеле (его удалили в панели playit).
    pub async fn tunnel_missing(&self, tunnel_id: &str) -> bool {
        match self.refresh().await {
            Ok(run) => !run.tunnels.iter().any(|t| t.id == tunnel_id) && !run.pending.iter().any(|p| p.id == tunnel_id),
            Err(_) => false,
        }
    }

    pub fn address(&self, tunnel_id: &str) -> Option<String> {
        self.lock().addresses.get(tunnel_id).cloned()
    }

    /// Создаёт туннель для сервера и возвращает его id.
    pub async fn create_tunnel(self: &Arc<Self>, server_id: &str, local_port: u16) -> Result<String> {
        let secret = self.require_secret()?;
        let run = self.refresh().await?;
        let name = api::tunnel_name(server_id);
        let mut outcome = self.api.create_tunnel(&secret, &run.agent_id, &name, local_port).await?;
        if let Err(code) = &outcome {
            if api::needs_agent_online(code) {
                // Новый агент ещё ни разу не выходил на связь: запускаем его
                // ненадолго, чтобы playit узнал его версию, и пробуем снова.
                tracing::info!("playit отказал в создании туннеля ({code}), запускаю агент и повторяю");
                const SETUP_USER: &str = "\u{0}setup";
                self.lock().users.insert(SETUP_USER.to_string());
                let started = self.ensure_agent().await;
                if started.is_ok() {
                    for _ in 0..10 {
                        tokio::time::sleep(Duration::from_secs(4)).await;
                        match self.api.create_tunnel(&secret, &run.agent_id, &name, local_port).await {
                            Ok(Err(code)) if api::needs_agent_online(&code) => {
                                tracing::info!("playit всё ещё отвечает {code}, жду агент");
                                outcome = Err(code);
                            }
                            Ok(result) => {
                                outcome = result;
                                break;
                            }
                            Err(e) => tracing::debug!("playit create: {e}"),
                        }
                    }
                }
                self.release(SETUP_USER).await;
                started?;
            }
        }
        let id = outcome.map_err(|code| {
            tracing::warn!("playit отказал в создании туннеля: {code}");
            Error::other(api::create_error_text(&code))
        })?;
        tracing::info!("Создан туннель playit {id} для сервера {server_id}");
        let _ = self.refresh().await;
        self.ensure_poller();
        Ok(id)
    }

    pub async fn configure_port(&self, tunnel_id: &str, local_port: u16) -> Result<()> {
        let secret = self.require_secret()?;
        self.api.configure_tunnel(&secret, tunnel_id, local_port).await
    }

    pub async fn delete_tunnel(&self, tunnel_id: &str) -> Result<()> {
        let secret = self.require_secret()?;
        self.api.delete_tunnel(&secret, tunnel_id).await?;
        let addresses = {
            let mut st = self.lock();
            st.addresses.remove(tunnel_id);
            st.addresses.clone()
        };
        let data = serde_json::to_vec_pretty(&addresses).expect("map serializes");
        let _ = fsx::write_atomic(&self.dir.join(CACHE_FILE), &data).await;
        let _ = self.refresh().await;
        Ok(())
    }

    /// Ссылка на панель playit. Гостевому аккаунту нужна ссылка со входом.
    pub async fn dashboard_url(&self) -> String {
        if let Some(secret) = self.secret() {
            let guest = self.lock().run.as_ref().is_some_and(RunData::is_guest);
            if guest {
                if let Ok(Some(url)) = self.api.guest_login_url(&secret).await {
                    return url;
                }
            }
        }
        DASHBOARD_URL.to_string()
    }

    // ---- агент ----

    /// Сервер с доступом для друзей запущен: агент должен работать.
    pub async fn acquire(self: &Arc<Self>, server_id: &str) -> Result<()> {
        self.lock().users.insert(server_id.to_string());
        self.ensure_agent().await
    }

    /// Сервер остановлен: если других серверов с доступом нет, агент выключается.
    pub async fn release(&self, server_id: &str) {
        let empty = {
            let mut st = self.lock();
            st.users.remove(server_id);
            st.users.is_empty()
        };
        if empty {
            self.stop_agent().await;
        }
    }

    /// Повторная попытка после ошибки агента.
    pub async fn retry(self: &Arc<Self>) -> Result<()> {
        if self.lock().users.is_empty() {
            {
                self.lock().agent_error = None;
            }
            self.emit();
            return Ok(());
        }
        self.ensure_agent().await
    }

    fn set_agent(&self, state: PlayitAgentState, error: Option<String>) {
        {
            let mut st = self.lock();
            st.agent = state;
            st.agent_error = error;
        }
        self.emit();
    }

    pub async fn ensure_agent(self: &Arc<Self>) -> Result<()> {
        let _ops = self.agent_ops.lock().await;
        if self.lock().process.is_some() {
            return Ok(());
        }
        let secret = self.require_secret()?;
        self.set_agent(PlayitAgentState::Downloading, None);
        let binary = match agent::ensure_binary(&self.http, &self.dir).await {
            Ok(path) => path,
            Err(e) => {
                let message = format!("Не удалось скачать агент playit: {e}");
                self.set_agent(PlayitAgentState::Error, Some(message.clone()));
                return Err(Error::other(message));
            }
        };
        if self.lock().users.is_empty() {
            // Пока скачивали, серверы уже остановили.
            self.set_agent(PlayitAgentState::Off, None);
            return Ok(());
        }
        self.set_agent(PlayitAgentState::Starting, None);
        let mut child = match agent::spawn(&binary, &secret, &self.dir) {
            Ok(child) => child,
            Err(e) => {
                self.set_agent(PlayitAgentState::Error, Some(e.to_string()));
                return Err(e);
            }
        };
        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, child.raw_handle()) {
            if let Err(e) = job.assign(handle) {
                tracing::warn!("Не удалось привязать агент playit к Job Object: {e}");
            }
        }
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let seq = {
            let mut st = self.lock();
            st.agent_seq += 1;
            st.process = Some(AgentProcess {
                seq: st.agent_seq,
                kill_tx: Some(kill_tx),
            });
            st.agent_seq
        };
        tracing::info!("Агент playit запущен");

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::info!("playitd: {line}");
                        *tail.lock().unwrap_or_else(|p| p.into_inner()) = line;
                    }
                }
            });
        }

        let this = self.clone();
        tokio::spawn(async move {
            let killed = tokio::select! {
                _ = child.wait() => false,
                res = kill_rx => res.is_ok(),
            };
            if killed {
                let _ = child.kill().await;
            }
            let code = child.wait().await.ok().and_then(|s| s.code());
            let unexpected = {
                let mut st = this.lock();
                if st.process.as_ref().is_some_and(|p| p.seq == seq) {
                    st.process = None;
                }
                !killed && st.agent_seq == seq
            };
            if unexpected {
                let tail = stderr_tail.lock().unwrap_or_else(|p| p.into_inner()).clone();
                let code = code.map_or("неизвестен".to_string(), |c| c.to_string());
                let detail = if tail.is_empty() { String::new() } else { format!(": {tail}") };
                tracing::warn!("Агент playit завершился (код {code}){detail}");
                this.set_agent(
                    PlayitAgentState::Error,
                    Some(format!("Агент playit неожиданно завершился (код {code}){detail}")),
                );
            }
        });

        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(AGENT_SETTLE).await;
            let alive = this.lock().process.as_ref().is_some_and(|p| p.seq == seq);
            if alive {
                this.set_agent(PlayitAgentState::Online, None);
                let _ = this.refresh().await;
            }
        });
        self.ensure_poller();
        Ok(())
    }

    async fn stop_agent(&self) {
        let _ops = self.agent_ops.lock().await;
        let process = {
            let mut st = self.lock();
            st.agent_seq += 1;
            st.process.take()
        };
        if let Some(mut p) = process {
            if let Some(tx) = p.kill_tx.take() {
                let _ = tx.send(());
            }
            tracing::info!("Агент playit остановлен");
        }
        self.set_agent(PlayitAgentState::Off, None);
    }

    /// При выходе из Hearth.
    pub async fn shutdown(&self) {
        self.lock().users.clear();
        self.stop_agent().await;
    }

    /// Пока работает агент или создаётся туннель, раз в несколько секунд
    /// спрашиваем playit о состоянии туннелей.
    fn ensure_poller(self: &Arc<Self>) {
        if self.poller.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let (running, pending) = {
                    let st = this.lock();
                    (
                        st.process.is_some(),
                        st.run.as_ref().is_some_and(|r| !r.pending.is_empty() || r.tunnels.iter().any(|t| t.display_address.is_empty())),
                    )
                };
                if !(running || pending) || !this.linked() {
                    break;
                }
                tokio::time::sleep(if pending { Duration::from_secs(5) } else { Duration::from_secs(30) }).await;
                if let Err(e) = this.refresh().await {
                    tracing::debug!("Не удалось обновить данные playit: {e}");
                }
            }
            this.poller.store(false, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullSink;
    use secret::MemoryStore;

    fn manager(dir: &std::path::Path) -> Arc<PlayitManager> {
        let http = Http::new(dir.join("cache")).unwrap();
        // Порт 9 на localhost закрыт: любой запрос к «API» сразу падает.
        let api = PlayitApi::with_base(http.client().clone(), "http://127.0.0.1:9");
        PlayitManager::new(http, api, dir.join("playit"), Box::new(MemoryStore::default()), Arc::new(NullSink), None)
    }

    #[tokio::test]
    async fn not_linked_until_secret_saved() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        assert!(!m.linked());
        assert!(matches!(m.create_tunnel("s", 25565).await, Err(Error::PlayitNotLinked)));
        assert!(matches!(m.acquire("s").await, Err(Error::PlayitNotLinked)));
        m.secrets.save("key").unwrap();
        m.lock().secret = None;
        assert!(m.linked());
    }

    #[tokio::test]
    async fn claim_can_be_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        let url = m.start_claim().unwrap();
        assert!(url.starts_with("https://playit.gg/claim/"));
        assert_eq!(m.start_claim().unwrap(), url, "повторный вызов отдаёт ту же ссылку");
        assert_eq!(m.status().claim_url.as_deref(), Some(url.as_str()));
        m.cancel_claim();
        assert_eq!(m.status().claim_url, None);
        assert_eq!(m.status().claim_error, None);
    }

    #[tokio::test]
    async fn cached_addresses_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("playit")).unwrap();
        std::fs::write(dir.path().join("playit").join(CACHE_FILE), r#"{"t1":"abc.gl.joinmc.link"}"#).unwrap();
        let m = manager(dir.path());
        assert_eq!(m.address("t1").as_deref(), Some("abc.gl.joinmc.link"));
        let status = m.status();
        assert!(!status.synced);
        assert_eq!(status.tunnels[0].address.as_deref(), Some("abc.gl.joinmc.link"));
    }

    /// Синхронные команды Tauri выполняются вне tokio: вместо падения должна быть ошибка.
    #[test]
    fn claim_outside_runtime_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        assert!(m.start_claim().is_err());
        assert_eq!(m.status().claim_url, None);
    }

    #[test]
    fn claim_codes_are_hex() {
        let code = new_claim_code();
        assert_eq!(code.len(), 10);
        assert!(code.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
