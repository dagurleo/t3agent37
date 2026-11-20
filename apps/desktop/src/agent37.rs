//! Client for the Agent37 Cloud API (https://www.agent37.com/docs). Holds the workspace's
//! `sk_live_` key, which only this computer ever sees: it is never copied to a machine.
//!
//! Two hosts take the key: `api.agent37.com` as `Authorization: Bearer`, and everything under
//! `agent37.app` (the machine's own URLs and the exec plane) as `X-Agent37-Key`, which leaves
//! `Authorization` free for T3's own bearer tokens. [`authorize`] adds it to a request only for
//! an `agent37.app` host, so the key cannot follow a request anywhere else.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

const API: &str = "https://api.agent37.com/v1";
const SANDBOX: &str = "https://sandbox.agent37.app/v1";
const APP_DOMAIN: &str = "agent37.app";
/// The image every machine runs: Ubuntu with Docker, as a full VM.
pub const TEMPLATE: &str = "agent37-ubuntu";
pub const KEYS_PAGE: &str = "https://www.agent37.com/dashboard/cloud/api-keys";

static KEY: RwLock<Option<String>> = RwLock::new(None);

pub fn key() -> Option<String> {
    KEY.read().unwrap().clone()
}

fn set_global_key(key: Option<String>) {
    *KEY.write().unwrap() = key;
}

/// Whether `url` is one of the workspace's own `*.agent37.app` addresses.
pub fn is_app_url(url: &str) -> bool {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).is_some_and(|h| {
        h.strip_suffix(APP_DOMAIN).is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
    })
}

/// Adds the workspace key to a request for an `agent37.app` address; leaves any other alone.
pub fn authorize(rb: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
    match key() {
        Some(k) if is_app_url(url) => rb.header("X-Agent37-Key", k),
        _ => rb,
    }
}

/// The same for a WebSocket handshake.
pub fn authorize_ws(req: &mut tokio_tungstenite::tungstenite::handshake::client::Request) {
    if let Some(k) = key()
        && is_app_url(&req.uri().to_string())
        && let Ok(v) = k.parse()
    {
        req.headers_mut().insert("X-Agent37-Key", v);
    }
}

/// HTTP to the machines (their T3 servers and watchdogs) through Agent37's edge: every request
/// carries the workspace key, and only to an `agent37.app` host. No redirects are followed.
#[derive(Clone, Debug)]
pub struct EdgeClient(reqwest::Client);

impl EdgeClient {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        ))
    }

    pub fn request(&self, method: reqwest::Method, url: impl AsRef<str>) -> reqwest::RequestBuilder {
        let url = url.as_ref();
        authorize(self.0.request(method, url), url)
    }

    pub fn get(&self, url: impl AsRef<str>) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::GET, url)
    }

    pub fn post(&self, url: impl AsRef<str>) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::POST, url)
    }
}

/// The address of `port` on the machine, reached through Agent37's edge with the key.
pub fn port_url(instance: &str, port: u16) -> String {
    format!("https://{instance}-{port}.{APP_DOMAIN}")
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Resources {
    pub cpu: u32,
    pub memory: u32,
    pub disk: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PublicPort {
    pub port: u16,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Instance {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub slept_at: Option<i64>,
    pub resources: Resources,
    #[serde(default)]
    pub public_ports: Vec<PublicPort>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub auto_sleep: bool,
    #[serde(default)]
    pub past_due: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

impl Instance {
    /// When it went to sleep, while it sleeps. Stopped instances carry none.
    pub fn slept_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        matches!(self.status.as_str(), "sleeping" | "waking")
            .then_some(self.slept_at?)
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExecResult {
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Billing {
    pub balance_usd: f64,
    #[serde(default)]
    pub memory_limit_gb: u32,
    #[serde(default)]
    pub past_due: bool,
}

/// One instance's spend in a usage window.
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub dollars: f64,
    pub awake_minutes: u64,
}

/// A refused call, with Agent37's stable error code when it gave one.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "Agent37: {} (HTTP {})", self.code, self.status)
        } else {
            write!(f, "Agent37: {} ({})", self.message, self.code)
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Safe to retry as is: the instance is between states, or the edge is.
    fn transient(&self) -> bool {
        matches!(self.code.as_str(), "try_again" | "wake_timeout" | "wake_failed" | "no_capacity")
            || self.status == 502 && self.code == "provisioning_failed"
    }
}

/// Error bodies come in two shapes: `{"error": {"code", "message"}}` on the API, and a flat
/// `{"error": "<code>", "message"?}` from the edge.
fn api_error(status: u16, body: &str) -> ApiError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let (code, message) = match &v["error"] {
        Value::String(code) => (code.clone(), v["message"].as_str().unwrap_or_default().to_owned()),
        Value::Object(e) => (
            e.get("code").and_then(Value::as_str).unwrap_or_default().to_owned(),
            e.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
        ),
        _ => (String::new(), body.chars().take(300).collect()),
    };
    ApiError { status, code: if code.is_empty() { format!("http_{status}") } else { code }, message }
}

#[derive(Clone)]
pub struct Agent37 {
    http: reqwest::Client,
    key: String,
}

impl std::fmt::Debug for Agent37 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Agent37 { .. }")
    }
}

/// Where the key is kept on this computer: the app's data dir, readable by this user only.
pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent37-key")
}

impl Agent37 {
    pub fn new(key: &str) -> Result<Self> {
        let key = key.trim();
        if !key.starts_with("sk_live_") || key.len() < 16 || key.chars().any(char::is_whitespace) {
            bail!("that isn't an Agent37 API key: they start with sk_live_");
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("t3agent37/", env!("CARGO_PKG_VERSION")))
            .build()?;
        set_global_key(Some(key.to_owned()));
        Ok(Self { http, key: key.to_owned() })
    }

    /// The key saved on this computer, if any.
    pub fn load(data_dir: &Path) -> Option<Self> {
        let key = std::fs::read_to_string(key_path(data_dir)).ok()?;
        Self::new(&key).ok()
    }

    /// Checks the key against the workspace, then keeps it.
    pub async fn save(data_dir: &Path, key: &str) -> Result<Self> {
        let a37 = Self::new(key)?;
        a37.billing().await.context("checking the key with Agent37")?;
        crate::store::write_private(&key_path(data_dir), a37.key.as_bytes())?;
        Ok(a37)
    }

    pub fn forget(data_dir: &Path) {
        let _ = std::fs::remove_file(key_path(data_dir));
        set_global_key(None);
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
        let mut rb = self.http.request(method, format!("{API}{path}")).bearer_auth(&self.key).timeout(timeout);
        if let Some(body) = body {
            rb = rb.json(body);
        }
        let res = rb.send().await.with_context(|| format!("calling Agent37 {path}"))?;
        let status = res.status().as_u16();
        let text = res.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(api_error(status, &text).into());
        }
        Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text)? })
    }

    /// Retries what Agent37 calls transient (a sleep checkpoint being written, a wake in flight),
    /// with backoff, for up to about two minutes.
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>, timeout: Duration) -> Result<Value> {
        let mut delay = Duration::from_secs(2);
        let mut waited = Duration::ZERO;
        loop {
            match self.send(method.clone(), path, body.as_ref(), timeout).await {
                Err(e) if e.downcast_ref::<ApiError>().is_some_and(ApiError::transient) && waited < Duration::from_secs(120) => {
                    tracing::info!("Agent37 {path}: {e}; retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                    waited += delay;
                    delay = (delay * 2).min(Duration::from_secs(15));
                }
                r => return r,
            }
        }
    }

    pub async fn billing(&self) -> Result<Billing> {
        let v = self.send(reqwest::Method::GET, "/billing", None, Duration::from_secs(20)).await?;
        Ok(serde_json::from_value(v)?)
    }

    /// A new VM with a public heartbeat port and auto-sleep as the backstop for a closed laptop.
    pub async fn create(&self, name: &str, size: &crate::machine::provision::SizeInfo, heartbeat_port: u16) -> Result<Instance> {
        let body = json!({
            "template": TEMPLATE,
            "type": "vm",
            "name": name,
            "resources": { "cpu": size.cpu, "memory": size.memory, "disk": crate::machine::provision::DISK_GB },
            "metadata": { "app": "t3agent37" },
            "auto_sleep": true,
            "idle_timeout_seconds": crate::machine::provision::IDLE_TIMEOUT_SECS,
            "public_ports": [{ "port": heartbeat_port, "label": "t3agent37 heartbeat" }],
        });
        // Not retried: a create that failed late, or timed out here, may have left an instance.
        match self.send(reqwest::Method::POST, "/instances", Some(&body), Duration::from_secs(300)).await {
            Ok(v) => Ok(serde_json::from_value(v)?),
            Err(e) => {
                self.delete_strays(name).await;
                Err(e)
            }
        }
    }

    /// Deletes what a failed create of `name` may have left: this app's instances by that name.
    async fn delete_strays(&self, name: &str) {
        let Ok(all) = self.list().await else { return };
        for i in all.iter().filter(|i| {
            i.name.as_deref() == Some(name) && i.metadata.as_ref().is_some_and(|m| m["app"] == "t3agent37")
        }) {
            tracing::warn!("deleting {} left by a failed create", i.id);
            if let Err(e) = self.delete(&i.id).await {
                tracing::warn!("deleting {}: {e:#}", i.id);
            }
        }
    }

    /// `None` once Agent37 no longer knows the id.
    pub async fn get(&self, id: &str) -> Result<Option<Instance>> {
        match self.send(reqwest::Method::GET, &format!("/instances/{id}"), None, Duration::from_secs(20)).await {
            Ok(v) => Ok(Some(serde_json::from_value(v)?)),
            // Agent37's own answer, not an edge or proxy page that happens to be a 404.
            Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.status == 404 && a.code == "not_found") => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn list(&self) -> Result<Vec<Instance>> {
        let v = self.send(reqwest::Method::GET, "/instances", None, Duration::from_secs(20)).await?;
        Ok(serde_json::from_value(v["data"].clone())?)
    }

    /// Gone only when Agent37 says so for that id: one bad answer must not retire a machine.
    pub async fn confirmed_gone(&self, id: &str) -> bool {
        match self.get(id).await {
            Ok(None) => true,
            Ok(Some(i)) => matches!(i.status.as_str(), "deleted" | "deleting"),
            Err(_) => false,
        }
    }

    /// Starts a stopped machine or wakes a sleeping one. Returns once Agent37 has it running or
    /// is finishing a move on its own (`202`); the caller waits for T3 either way.
    pub async fn start(&self, id: &str) -> Result<()> {
        self.call(reqwest::Method::POST, &format!("/instances/{id}/start"), None, Duration::from_secs(300)).await.map(drop)
    }

    /// Sleeps the machine now, memory and running processes kept. Returns once it is asleep.
    pub async fn sleep(&self, id: &str) -> Result<()> {
        self.call(reqwest::Method::POST, &format!("/instances/{id}/sleep"), None, Duration::from_secs(300)).await.map(drop)
    }

    /// Only on a running machine; a VM cold-boots onto its new shape.
    pub async fn resize(&self, id: &str, cpu: u32, memory: u32) -> Result<()> {
        let body = json!({ "cpu": cpu, "memory": memory });
        self.call(reqwest::Method::POST, &format!("/instances/{id}/resize"), Some(body), Duration::from_secs(300)).await.map(drop)
    }

    pub async fn set_auto_sleep(&self, id: &str, on: bool) -> Result<()> {
        let body = json!({ "auto_sleep": on, "idle_timeout_seconds": crate::machine::provision::IDLE_TIMEOUT_SECS });
        self.call(reqwest::Method::PATCH, &format!("/instances/{id}"), Some(body), Duration::from_secs(30)).await.map(drop)
    }

    /// Gives `port` a permanent public URL. Without a prefix its hostname is a random 20-character
    /// slug, so the URL is its own credential.
    pub async fn add_public_port(&self, id: &str, port: u16, label: &str) -> Result<String> {
        let body = json!({ "port": port, "label": label });
        let v = self.call(reqwest::Method::POST, &format!("/instances/{id}/public-ports"), Some(body), Duration::from_secs(30)).await?;
        v["url"].as_str().map(str::to_owned).ok_or_else(|| anyhow!("Agent37 returned no public URL"))
    }

    /// Revokes `port`'s public URL; the edge stops routing it within seconds.
    pub async fn delete_public_port(&self, id: &str, port: u16) -> Result<()> {
        match self.call(reqwest::Method::DELETE, &format!("/instances/{id}/public-ports/{port}"), None, Duration::from_secs(30)).await {
            Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.status == 404) => Ok(()),
            r => r.map(drop),
        }
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        match self.call(reqwest::Method::DELETE, &format!("/instances/{id}"), None, Duration::from_secs(120)).await {
            Err(e) if e.downcast_ref::<ApiError>().is_some_and(|a| a.status == 404 && a.code == "not_found") => Ok(()),
            r => r.map(drop),
        }
    }

    /// Runs `script` with bash as root. An exec wakes a sleeping machine: never call it for a
    /// machine that should stay asleep.
    pub async fn exec(&self, id: &str, script: &str, timeout_secs: u64) -> Result<ExecResult> {
        // Agent37 runs it with `sh -c` and waits up to 15 minutes; the deadline is ours. Root has
        // no profile on the image, so the tools provisioning installs (t3, claude, codex, gh in
        // ~/.local/bin) are put on PATH here.
        let script = format!("export PATH=\"$HOME/.local/bin:$PATH\"\n{script}");
        let body = json!({
            "command": format!("exec bash -lc {}", shell_quote(&script)),
            "user": "root",
        });
        let url = format!("{SANDBOX}/instances/{id}/exec");
        let mut delay = Duration::from_secs(2);
        let mut waited = Duration::ZERO;
        loop {
            let res = self
                .http
                .post(&url)
                .header("X-Agent37-Key", &self.key)
                .json(&body)
                .timeout(Duration::from_secs(timeout_secs.min(900) + 60))
                .send()
                .await
                .context("running a command on the machine")?;
            let status = res.status().as_u16();
            let text = res.text().await.unwrap_or_default();
            if (200..300).contains(&status) {
                return Ok(serde_json::from_str(&text).with_context(|| format!("unexpected exec reply: {text}"))?);
            }
            let err = api_error(status, &text);
            // A refused command never started, so a retry cannot run it twice. A 504 can have:
            // the command keeps running on the machine. A 409 means a sleep is being written:
            // a retry would wake the VM, so the caller decides.
            if err.transient() && status != 504 && status != 409 && waited < Duration::from_secs(120) {
                tokio::time::sleep(delay).await;
                waited += delay;
                delay = (delay * 2).min(Duration::from_secs(15));
                continue;
            }
            return Err(err.into());
        }
    }

    /// Runs `script` and fails unless it exits 0. Returns stdout.
    pub async fn exec_ok(&self, id: &str, script: &str, timeout_secs: u64) -> Result<String> {
        let r = self.exec(id, script, timeout_secs).await?;
        if r.exit_code != Some(0) {
            let detail = if r.stderr.trim().is_empty() { r.stdout.trim() } else { r.stderr.trim() };
            bail!("command on the machine failed ({:?}): {detail}", r.exit_code);
        }
        Ok(r.stdout)
    }

    /// Spend per instance over whole UTC days, `from` to `to` inclusive. Compute settles hourly,
    /// so the current hour shows up within the next one.
    pub async fn usage(&self, from: chrono::NaiveDate, to: chrono::NaiveDate) -> Result<HashMap<String, Usage>> {
        let path = format!("/usage?from={}&to={}", from.format("%Y-%m-%d"), to.format("%Y-%m-%d"));
        let v = self.send(reqwest::Method::GET, &path, None, Duration::from_secs(30)).await?;
        let mut out = HashMap::new();
        for i in v["instances"].as_array().into_iter().flatten() {
            let Some(id) = i["id"].as_str() else { continue };
            out.insert(
                id.to_owned(),
                Usage {
                    dollars: i["total_micros"].as_f64().unwrap_or(0.0) / 1e6,
                    awake_minutes: i["awake_minutes"].as_u64().unwrap_or(0),
                },
            );
        }
        Ok(out)
    }
}

/// One single-quoted shell word.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_agent37_app_hosts_get_the_key() {
        assert!(is_app_url("https://ab12cd34ef-3773.agent37.app/ws"));
        assert!(is_app_url("wss://ab12cd34ef-3773.agent37.app/ws?x=1"));
        assert!(is_app_url("https://sandbox.agent37.app/v1/instances/x/exec"));
        assert!(!is_app_url("https://evilagent37.app/"));
        assert!(!is_app_url("https://agent37.app.evil.com/"));
        assert!(!is_app_url("http://localhost:47731/"));
        assert!(!is_app_url("https://api.agent37.com/v1/billing"));
    }

    #[test]
    fn reads_both_error_shapes() {
        let e = api_error(409, r#"{"error":{"code":"try_again","message":"checkpoint in progress"}}"#);
        assert_eq!((e.code.as_str(), e.transient()), ("try_again", true));
        let e = api_error(503, r#"{"error":"wake_timeout","message":"slow"}"#);
        assert_eq!((e.code.as_str(), e.transient()), ("wake_timeout", true));
        let e = api_error(401, "nope");
        assert_eq!((e.code.as_str(), e.transient()), ("http_401", false));
    }

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
