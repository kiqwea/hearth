//! Прямое подключение без посредников: Hearth просит роутер (UPnP) открыть
//! порт сервера и показывает внешний IPv4-адрес роутера.
//!
//! Работает, только если у роутера свой «белый» IPv4 и включён UPnP. Если
//! провайдер даёт общий адрес (CGNAT, DS-Lite), роутер сообщит внутренний
//! адрес вроде 100.64.x.x или не сообщит никакого, и Hearth предложит playit.
//!
//! Проброс открывается при запуске сервера, продлевается, пока сервер
//! работает, и снимается при остановке. Внешний IP может меняться между
//! запусками: Hearth каждый раз показывает актуальный.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use igd_next::aio::tokio::search_gateway;
use igd_next::{AddPortError, GatewayIpVersion, PortMappingProtocol, SearchOptions};
use serde::Serialize;
use tokio::sync::oneshot;
use ts_rs::TS;

use crate::events::{Event, EventSink};

/// Срок проброса. Пока сервер работает, Hearth продлевает его раньше срока;
/// если Hearth упадёт, роутер сам закроет порт.
const LEASE_SECS: u32 = 3600;
const RENEW_EVERY: Duration = Duration::from_secs(20 * 60);
const DEFAULT_MC_PORT: u16 = 25565;

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DirectState {
    /// Сервер не запущен: порт закрыт.
    #[default]
    Off,
    Opening,
    Open,
    Failed,
}

/// Состояние прямого подключения сервера для интерфейса.
#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirectInfo {
    pub state: DirectState,
    /// Адрес для друзей (последний известный, если сервер сейчас выключен).
    pub address: Option<String>,
    pub error: Option<String>,
}

/// Почему не получилось напрямую.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectError {
    /// Роутер не ответил по UPnP: он выключен в настройках или не поддерживается.
    NoRouter,
    /// У роутера нет своего белого IPv4 (CGNAT, DS-Lite).
    NoPublicIp(Option<IpAddr>),
    /// Роутер отказал в пробросе.
    Refused(String),
}

impl std::fmt::Display for DirectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectError::NoRouter => write!(
                f,
                "роутер не разрешает автоматически открыть порт (UPnP выключен или не поддерживается)"
            ),
            DirectError::NoPublicIp(_) => write!(
                f,
                "провайдер не выдаёт роутеру собственный IPv4-адрес (общий адрес, CGNAT или DS-Lite)"
            ),
            DirectError::Refused(reason) => write!(f, "роутер не открыл порт: {reason}"),
        }
    }
}

/// Адрес, доступный из интернета: не локальный, не общий адрес провайдера.
pub fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || (a == 100 && (64..128).contains(&b)) // CGNAT 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // DS-Lite и служебные 192.0.0.0/24
        || (a == 198 && (b == 18 || b == 19)) // тестовые сети
        || a >= 240)
}

/// Адрес для Minecraft: порт можно не писать, если он стандартный.
pub fn format_address(ip: Ipv4Addr, port: u16) -> String {
    if port == DEFAULT_MC_PORT { ip.to_string() } else { format!("{ip}:{port}") }
}

fn search_options() -> SearchOptions {
    let mut options = SearchOptions::default();
    options.timeout = Some(Duration::from_secs(4));
    options.single_search_timeout = Some(Duration::from_secs(3));
    options.gateway_ip_version = GatewayIpVersion::V4;
    options
}

/// Локальный адрес этого ПК в сторону роутера (UDP «connect» ничего не отправляет).
fn local_ip_towards(gateway: SocketAddr) -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(gateway).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Открывает (или продлевает) проброс порта. Возвращает внешний IPv4.
pub async fn open_port(port: u16, description: &str) -> Result<Ipv4Addr, DirectError> {
    let gateway = search_gateway(search_options()).await.map_err(|e| {
        tracing::info!("UPnP-роутер не найден: {e}");
        DirectError::NoRouter
    })?;
    let external = match gateway.get_external_ip().await {
        Ok(IpAddr::V4(ip)) if is_public_v4(ip) => ip,
        Ok(other) => return Err(DirectError::NoPublicIp(Some(other))),
        Err(e) => {
            tracing::info!("Роутер не сообщил внешний IP: {e}");
            return Err(DirectError::NoPublicIp(None));
        }
    };
    let local = local_ip_towards(gateway.addr)
        .ok_or_else(|| DirectError::Refused("не удалось определить адрес этого компьютера в сети".into()))?;
    let target = SocketAddr::new(IpAddr::V4(local), port);
    let mut result = gateway
        .add_port(PortMappingProtocol::TCP, port, target, LEASE_SECS, description)
        .await;
    if matches!(result, Err(AddPortError::OnlyPermanentLeasesSupported)) {
        result = gateway.add_port(PortMappingProtocol::TCP, port, target, 0, description).await;
    }
    result.map_err(|e| match e {
        AddPortError::PortInUse => DirectError::Refused(format!(
            "порт {port} на роутере уже отдан другому устройству"
        )),
        AddPortError::ActionNotAuthorized => DirectError::Refused("роутер запретил открывать порты".into()),
        other => DirectError::Refused(other.to_string()),
    })?;
    Ok(external)
}

/// Закрывает проброс. Ошибки не важны: срок проброса всё равно истечёт.
pub async fn close_port(port: u16) {
    if let Ok(gateway) = search_gateway(search_options()).await {
        if let Err(e) = gateway.remove_port(PortMappingProtocol::TCP, port).await {
            tracing::debug!("Не удалось снять проброс порта {port}: {e}");
        }
    }
}

struct Entry {
    info: DirectInfo,
    stop: Option<oneshot::Sender<()>>,
}

/// Проброс портов для запущенных серверов с прямым подключением.
pub struct DirectManager {
    sink: Arc<dyn EventSink>,
    entries: Mutex<HashMap<String, Entry>>,
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirectEvent {
    pub server_id: String,
    pub info: DirectInfo,
}

impl DirectManager {
    pub fn new(sink: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self {
            sink,
            entries: Mutex::new(HashMap::new()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn info(&self, id: &str) -> Option<DirectInfo> {
        self.lock().get(id).map(|e| e.info.clone())
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut DirectInfo)) {
        let info = {
            let mut entries = self.lock();
            let entry = entries.entry(id.to_string()).or_insert_with(|| Entry {
                info: DirectInfo::default(),
                stop: None,
            });
            f(&mut entry.info);
            entry.info.clone()
        };
        self.sink.emit(Event::Direct(DirectEvent {
            server_id: id.to_string(),
            info,
        }));
    }

    /// Повторно сообщает интерфейсу текущее состояние (например, после сохранения server.json).
    pub fn notify(&self, id: &str) {
        self.update(id, |_| {});
    }

    /// Состояние для интерфейса без проброса: «проверяю» или «не получилось, причина».
    pub fn set_state(&self, id: &str, state: DirectState, error: Option<String>) {
        self.update(id, |info| {
            info.state = state;
            info.error = error;
        });
    }

    /// Проверка при включении доступа: можно ли напрямую. Порт сразу закрывается,
    /// если сервер не запущен (`keep = false`).
    pub async fn probe(self: &Arc<Self>, id: &str, port: u16, keep: bool) -> Result<String, DirectError> {
        let ip = open_port(port, &description(id)).await?;
        let address = format_address(ip, port);
        if keep {
            self.start_with(id, port, Some(address.clone()));
        } else {
            close_port(port).await;
            self.update(id, |info| {
                *info = DirectInfo {
                    state: DirectState::Off,
                    address: Some(address.clone()),
                    error: None,
                };
            });
        }
        Ok(address)
    }

    /// Сервер запустился: открыть порт и продлевать, пока не остановится.
    pub fn start(self: &Arc<Self>, id: &str, port: u16) {
        self.start_with(id, port, None);
    }

    fn start_with(self: &Arc<Self>, id: &str, port: u16, opened: Option<String>) {
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        if let Some(old) = self.lock().get_mut(id).and_then(|e| e.stop.take()) {
            let _ = old.send(());
        }
        self.update(id, |info| {
            info.state = if opened.is_some() { DirectState::Open } else { DirectState::Opening };
            info.error = None;
            if opened.is_some() {
                info.address = opened.clone();
            }
        });
        if let Some(entry) = self.lock().get_mut(id) {
            entry.stop = Some(stop_tx);
        }
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let desc = description(&id);
            let mut first = opened.is_none();
            loop {
                if first {
                    first = false;
                } else {
                    tokio::select! {
                        _ = tokio::time::sleep(RENEW_EVERY) => {}
                        _ = &mut stop_rx => break,
                    }
                }
                let result = tokio::select! {
                    r = open_port(port, &desc) => r,
                    _ = &mut stop_rx => break,
                };
                match result {
                    Ok(ip) => this.update(&id, |info| {
                        info.state = DirectState::Open;
                        info.address = Some(format_address(ip, port));
                        info.error = None;
                    }),
                    Err(e) => {
                        tracing::warn!("Прямое подключение сервера {id}: {e}");
                        this.update(&id, |info| {
                            info.state = DirectState::Failed;
                            info.error = Some(e.to_string());
                        });
                    }
                }
            }
            close_port(port).await;
        });
    }

    /// Сервер остановлен: снять проброс. Последний адрес остаётся для показа.
    pub fn stop(&self, id: &str) {
        let had = {
            let mut entries = self.lock();
            match entries.get_mut(id) {
                Some(entry) => {
                    if let Some(tx) = entry.stop.take() {
                        let _ = tx.send(());
                    }
                    true
                }
                None => false,
            }
        };
        if had {
            self.update(id, |info| {
                info.state = DirectState::Off;
                info.error = None;
            });
        }
    }

    /// Прямое подключение для сервера выключили или сервер удалили.
    pub fn forget(&self, id: &str) {
        if let Some(mut entry) = self.lock().remove(id) {
            if let Some(tx) = entry.stop.take() {
                let _ = tx.send(());
            }
        }
    }
}

fn description(id: &str) -> String {
    let mut d = format!("Hearth {id}");
    d.truncate(40);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses() {
        assert!(is_public_v4("92.208.220.160".parse().unwrap()));
        assert!(is_public_v4("8.8.8.8".parse().unwrap()));
        for ip in ["192.168.0.1", "10.1.2.3", "172.16.5.4", "100.64.0.1", "100.127.255.255", "192.0.0.2", "127.0.0.1", "0.0.0.0", "169.254.1.1"] {
            assert!(!is_public_v4(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public_v4("100.128.0.1".parse().unwrap()));
    }

    /// Живая проверка на настоящей сети: сколько длится попытка и чем кончается.
    /// Если проброс откроется, он сразу снимается.
    #[tokio::test]
    #[ignore]
    async fn live_probe() {
        let started = std::time::Instant::now();
        let result = open_port(25599, "Hearth test").await;
        println!("Результат за {:?}: {result:?}", started.elapsed());
        if result.is_ok() {
            close_port(25599).await;
        }
        assert!(started.elapsed() < Duration::from_secs(15));
    }

    #[test]
    fn address_omits_default_port() {
        let ip: Ipv4Addr = "92.208.220.160".parse().unwrap();
        assert_eq!(format_address(ip, 25565), "92.208.220.160");
        assert_eq!(format_address(ip, 25566), "92.208.220.160:25566");
    }
}
