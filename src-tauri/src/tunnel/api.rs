//! Клиент API playit.gg (`https://api.playit.gg`).
//!
//! Форматы запросов и ответов взяты из официального агента
//! (github.com/playit-cloud/playit-agent, `packages/api_client/src/api.rs`).
//! Все вызовы: `POST <путь>` с JSON, авторизация агента заголовком
//! `Authorization: Agent-Key <секрет>`. Ответ всегда одного вида:
//! `{"status":"success"|"fail"|"error","data":...}`.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};

pub const API_BASE: &str = "https://api.playit.gg";
const SERVICE: &str = "playit.gg";

/// Адрес страницы, где пользователь подтверждает подключение Hearth к своему аккаунту.
pub fn claim_url(code: &str) -> String {
    format!("https://playit.gg/claim/{code}")
}

/// Ответ API: успех или «штатный отказ» (`fail`) с кодом вроде `CodeNotFound`.
pub type Outcome<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub struct PlayitApi {
    client: reqwest::Client,
    base: String,
}

#[derive(Deserialize)]
#[serde(tag = "status", content = "data")]
enum Envelope {
    #[serde(rename = "success")]
    Success(Value),
    #[serde(rename = "fail")]
    Fail(Value),
    #[serde(rename = "error")]
    Error(ErrorBody),
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: Value,
}

/// Состояние подтверждения в браузере (`/claim/setup`).
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimSetup {
    WaitingForUserVisit,
    WaitingForUser,
    UserAccepted,
    UserRejected,
}

/// Что знает о себе агент (`/v1/agents/rundata`): его туннели, туннели в
/// процессе создания и уведомления от playit.
#[derive(Deserialize, Clone, Debug)]
pub struct RunData {
    pub agent_id: String,
    #[serde(default)]
    pub tunnels: Vec<RunTunnel>,
    #[serde(default)]
    pub pending: Vec<PendingTunnel>,
    #[serde(default)]
    pub notices: Vec<Notice>,
    pub permissions: Permissions,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RunTunnel {
    pub id: String,
    pub display_address: String,
    #[serde(default)]
    pub disabled_reason: Option<String>,
    #[serde(default)]
    pub agent_config: Option<AgentConfig>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AgentConfig {
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ConfigField {
    pub name: String,
    pub value: String,
}

impl RunTunnel {
    /// Локальный порт, на который playit ведёт игроков.
    pub fn local_port(&self) -> Option<u16> {
        self.agent_config
            .as_ref()?
            .fields
            .iter()
            .find(|f| f.name == "local_port")?
            .value
            .parse()
            .ok()
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct PendingTunnel {
    pub id: String,
    #[serde(default)]
    pub status_msg: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Notice {
    pub priority: String,
    pub message: String,
    #[serde(default)]
    pub resolve_link: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Permissions {
    pub account_status: String,
}

impl RunData {
    pub fn is_guest(&self) -> bool {
        self.permissions.account_status == "guest"
    }
}


/// Имя туннеля в аккаунте playit: только ASCII (иначе `TunnelNameIsNotAscii`) и коротко.
pub fn tunnel_name(server_id: &str) -> String {
    let mut name = format!("Hearth {server_id}");
    name.retain(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');
    name.truncate(40);
    name
}

impl PlayitApi {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_base(client, API_BASE)
    }

    pub fn with_base(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    async fn call<T: DeserializeOwned>(&self, path: &str, secret: Option<&str>, body: Value) -> Result<Outcome<T>> {
        let mut req = self
            .client
            .post(format!("{}{path}", self.base))
            .timeout(Duration::from_secs(30))
            .json(&body);
        if let Some(secret) = secret {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Agent-Key {}", secret.trim()));
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::Api {
                service: SERVICE.into(),
                status: status.as_u16(),
                message: "слишком много запросов, попробуйте через минуту".into(),
            });
        }
        let text = resp.text().await?;
        // Тело ответа без заголовков: ключ агента сюда не попадает.
        tracing::debug!("playit {path} → {status}: {}", text.chars().take(500).collect::<String>());
        let envelope: Envelope = serde_json::from_str(&text).map_err(|e| {
            if status.is_success() {
                Error::Parse {
                    service: SERVICE.into(),
                    message: e.to_string(),
                }
            } else {
                Error::Api {
                    service: SERVICE.into(),
                    status: status.as_u16(),
                    message: text.chars().take(200).collect(),
                }
            }
        })?;
        match envelope {
            Envelope::Success(data) => serde_json::from_value(data)
                .map(Ok)
                .map_err(|e| Error::Parse {
                    service: SERVICE.into(),
                    message: format!("{path}: {e}"),
                }),
            Envelope::Fail(data) => Ok(Err(fail_code(&data))),
            Envelope::Error(err) => {
                let e = api_error(status.as_u16(), err);
                tracing::warn!("playit {path}: {e}");
                Err(e)
            }
        }
    }

    /// Регистрирует код подтверждения и сообщает, на каком шаге пользователь.
    /// Агент типа `self-managed`: он сам создаёт свои туннели через API.
    pub async fn claim_setup(&self, code: &str, version: &str) -> Result<Outcome<ClaimSetup>> {
        self.call(
            "/claim/setup",
            None,
            json!({ "code": code, "agent_type": "self-managed", "version": version }),
        )
        .await
    }

    /// После подтверждения в браузере меняет код на секретный ключ агента.
    pub async fn claim_exchange(&self, code: &str) -> Result<Outcome<String>> {
        #[derive(Deserialize)]
        struct Key {
            secret_key: String,
        }
        Ok(self
            .call::<Key>("/claim/exchange", None, json!({ "code": code }))
            .await?
            .map(|k| k.secret_key))
    }

    pub async fn rundata(&self, secret: &str) -> Result<RunData> {
        self.call("/v1/agents/rundata", Some(secret), json!({}))
            .await?
            .map_err(|code| Error::other(format!("playit.gg не отдал данные агента: {code}")))
    }

    /// Создаёт туннель Minecraft Java, который ведёт на `127.0.0.1:<local_port>`.
    /// Возвращает id туннеля или код отказа (см. [`create_error_text`]).
    pub async fn create_tunnel(&self, secret: &str, agent_id: &str, name: &str, local_port: u16) -> Result<Outcome<String>> {
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        // Формат `/tunnels/create`. Новый `/v1/tunnels/create` с ключом агента
        // отвечает «failed to parse body» на любой вариант тела (проверено 2026-11-22).
        let body = json!({
            "name": name,
            "tunnel_type": "minecraft-java",
            "port_type": "tcp",
            "port_count": 1,
            "origin": { "type": "agent", "data": {
                "agent_id": agent_id,
                "local_ip": "127.0.0.1",
                "local_port": local_port,
            }},
            "enabled": true,
            "alloc": null,
            "firewall_id": null,
            "proxy_protocol": null,
        });
        Ok(self
            .call::<Id>("/tunnels/create", Some(secret), body)
            .await?
            .map(|id| id.id))
    }

    /// Меняет локальный порт туннеля (если порт сервера поменяли в настройках).
    pub async fn configure_tunnel(&self, secret: &str, tunnel_id: &str, local_port: u16) -> Result<()> {
        let body = json!({
            "tunnel_id": tunnel_id,
            "local_ip": "127.0.0.1",
            "local_port": local_port,
            "agent_id": null,
            "enabled": true,
        });
        match self.call::<Value>("/tunnels/update", Some(secret), body).await? {
            Ok(_) => Ok(()),
            Err(code) => Err(Error::other(format!("Не удалось поменять порт туннеля playit: {code}"))),
        }
    }

    /// Удаляет туннель. Уже удалённый туннель ошибкой не считается.
    pub async fn delete_tunnel(&self, secret: &str, tunnel_id: &str) -> Result<()> {
        match self
            .call::<Value>("/tunnels/delete", Some(secret), json!({ "tunnel_id": tunnel_id }))
            .await?
        {
            Ok(_) => Ok(()),
            Err(code) if code == "TunnelNotFound" => Ok(()),
            Err(code) => Err(Error::other(format!("Не удалось удалить туннель playit: {code}"))),
        }
    }

    /// Ссылка входа в панель playit для гостевого аккаунта (без пароля).
    pub async fn guest_login_url(&self, secret: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Session {
            session_key: String,
        }
        Ok(self
            .call::<Session>("/login/guest", Some(secret), json!({}))
            .await?
            .ok()
            .map(|s| format!("https://playit.gg/login/guest-account/{}", s.session_key)))
    }
}

/// Код отказа: строка (`"CodeNotFound"`) или объект (`{"error":"TunnelNotFound"}`).
fn fail_code(data: &Value) -> String {
    match data {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| data.to_string()),
        other => other.to_string(),
    }
}

fn api_error(status: u16, err: ErrorBody) -> Error {
    let message = match &err.message {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if err.kind == "auth" && matches!(message.as_str(), "InvalidAgentKey" | "NoLongerValid") {
        return Error::PlayitKeyInvalid;
    }
    Error::Api {
        service: SERVICE.into(),
        status,
        message: if message.is_empty() { err.kind } else { format!("{}: {message}", err.kind) },
    }
}

/// Отказы, которые бывают, пока агент ни разу не подключался к playit
/// (сервер ещё не знает его версию и схему настроек).
pub fn needs_agent_online(code: &str) -> bool {
    matches!(
        code,
        "AgentVersionTooOld" | "AgentVersionUnknown" | "InvalidTunnelConfig" | "ConfigNotCompatibleWithAgent"
    )
}

pub fn create_error_text(code: &str) -> String {
    let reason = match code {
        "RequiresVerifiedAccount" => {
            "playit.gg просит подтвердить почту аккаунта. Откройте панель playit, подтвердите почту и попробуйте снова."
        }
        "RequiresPlayitPremium" | "RegionRequiresPlayitPremium" | "PublicPortRequiresPlayitPremium" => {
            "на бесплатном тарифе playit больше туннелей не создать. Удалите лишние туннели в панели playit."
        }
        "AgentVersionTooOld" => "агент playit устарел, перезапустите Hearth, чтобы он обновился.",
        "AgentNotFound" | "InvalidAgentId" => "агент Hearth не найден в аккаунте playit. Подключите playit заново.",
        _ => "",
    };
    if reason.is_empty() {
        format!("Не удалось создать адрес в playit.gg: {code}")
    } else {
        format!("Не удалось создать адрес в playit.gg: {reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_variants_parse() {
        let ok: Envelope = serde_json::from_str(r#"{"status":"success","data":"WaitingForUserVisit"}"#).unwrap();
        assert!(matches!(ok, Envelope::Success(Value::String(_))));
        let fail: Envelope = serde_json::from_str(r#"{"status":"fail","data":"CodeNotFound"}"#).unwrap();
        let Envelope::Fail(data) = fail else { panic!() };
        assert_eq!(fail_code(&data), "CodeNotFound");
        let err: Envelope =
            serde_json::from_str(r#"{"status":"error","data":{"type":"auth","message":"InvalidAgentKey"}}"#).unwrap();
        let Envelope::Error(body) = err else { panic!() };
        assert!(matches!(api_error(200, body), Error::PlayitKeyInvalid));
    }

    #[test]
    fn object_fail_codes() {
        assert_eq!(fail_code(&json!({"error": "TunnelNotFound"})), "TunnelNotFound");
    }

    #[test]
    fn rundata_parses_and_reads_port() {
        let data: RunData = serde_json::from_value(json!({
            "agent_id": "8a6b3c1e-0000-4000-8000-000000000001",
            "tunnels": [{
                "id": "t1", "internal_id": 5, "name": "Hearth x", "display_address": "abc.gl.joinmc.link",
                "port_type": "tcp", "port_count": 1, "tunnel_type": "minecraft-java",
                "tunnel_type_display": "Minecraft Java",
                "agent_config": { "fields": [
                    { "name": "local_ip", "value": "127.0.0.1" },
                    { "name": "local_port", "value": "25566" }
                ]},
                "disabled_reason": null
            }],
            "pending": [{ "id": "t2", "name": "n", "tunnel_type": null, "tunnel_type_display": "",
                          "port_type": "tcp", "port_count": 1, "status_msg": "allocating" }],
            "notices": [],
            "permissions": { "is_self_managed": true, "has_premium": false, "account_status": "guest" }
        }))
        .unwrap();
        assert!(data.is_guest());
        assert_eq!(data.tunnels[0].local_port(), Some(25566));
        assert_eq!(data.pending[0].status_msg, "allocating");
    }

    #[test]
    fn tunnel_names_are_ascii_and_short() {
        assert_eq!(tunnel_name("survival-a1b2c3"), "Hearth survival-a1b2c3");
        assert!(tunnel_name(&"x".repeat(64)).len() <= 40);
    }

    /// Живая проверка формата ответов настоящего API (без аккаунта).
    #[tokio::test]
    #[ignore]
    async fn live_claim_setup_responds() {
        let api = PlayitApi::new(reqwest::Client::new());
        let state = api.claim_setup(&super::super::new_claim_code(), "Hearth test").await.unwrap();
        assert_eq!(state, Ok(ClaimSetup::WaitingForUserVisit));
        assert!(matches!(api.rundata("deadbeef").await, Err(Error::PlayitKeyInvalid)));
    }
}
