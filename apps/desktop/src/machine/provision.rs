//! Creating and upgrading machines: everything that runs on the VM goes through `provision.sh`,
//! which is idempotent.

use crate::agent37::{self, Agent37};
use crate::proxy::auth::random_token;
use crate::store::Machine;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::Deserialize;

const PROVISION_SH: &str = include_str!("provision.sh");
const WATCHDOG_MJS: &str = include_str!("watchdog.mjs");
pub const T3_PORT: u16 = 3773;
pub const WATCHDOG_PORT: u16 = 3774;
/// The watchdog answers its own heartbeat here, through a public URL: that traffic is what keeps
/// Agent37's auto-sleep off the VM while the watchdog wants it up.
pub const HEARTBEAT_PORT: u16 = 3775;
/// The wake link's port: a public URL of its own (so replacing it leaves the heartbeat alone)
/// that a phone opens to wake the machine. The watchdog answers it with a page and nothing else.
pub const WAKE_PORT: u16 = 3776;

/// Identifies the provisioning scripts; a machine provisioned with others gets them on wake.
pub fn revision() -> String {
    let hash = fnv1a(PROVISION_SH.bytes().chain(WATCHDOG_MJS.bytes()).chain(SLEEP_AFTER_SECS.to_string().bytes()));
    format!("{hash:016x}")
}

/// FNV-1a: stable across builds, unlike std's hasher.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provisioned {
    pub environment_id: String,
    pub upstream_token: Option<String>,
}

/// Machine sizes: Agent37's VM shapes. `hourly` is compute while awake (Agent37's monthly VM
/// price over 730 hours, disk taken out); asleep, a machine bills its disk alone.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeInfo {
    pub id: &'static str,
    pub spec: &'static str,
    pub hourly: f64,
    #[serde(skip)]
    pub cpu: u32,
    #[serde(skip)]
    pub memory: u32,
}

pub const SIZES: &[SizeInfo] = &[
    SizeInfo { id: "small", spec: "2 vCPU · 4 GB", hourly: 0.0151, cpu: 2, memory: 4 },
    SizeInfo { id: "default", spec: "4 vCPU · 8 GB", hourly: 0.0301, cpu: 4, memory: 8 },
    SizeInfo { id: "large", spec: "4 vCPU · 16 GB", hourly: 0.0493, cpu: 4, memory: 16 },
    SizeInfo { id: "xlarge", spec: "4 vCPU · 32 GB", hourly: 0.0877, cpu: 4, memory: 32 },
    SizeInfo { id: "huge", spec: "8 vCPU · 64 GB", hourly: 0.1753, cpu: 8, memory: 64 },
];

pub fn size(id: &str) -> Result<&'static SizeInfo> {
    SIZES.iter().find(|s| s.id == id).ok_or_else(|| anyhow!("unknown size {id}"))
}

/// The machine's disk. Room for a few repos, their dependencies and Docker images; it bills
/// \$0.09 per GB-month, asleep too, and can only grow.
pub const DISK_GB: u32 = 20;

/// How long a machine stays up after the last agent turn and the last action through t3agent37.
/// Long enough to read a result and type a follow-up without waiting for a wake; idle minutes
/// cost under a cent even on the large size.
pub const SLEEP_AFTER_SECS: u64 = 300;

/// Agent37's auto-sleep, the backstop when this app isn't running: the VM sleeps once nothing has
/// moved through its URLs this long. Its minimum; the watchdog's heartbeat holds it off.
pub const IDLE_TIMEOUT_SECS: u64 = 300;

/// The machine's name on Agent37. Only ever shown there.
pub fn instance_name(name: &str) -> String {
    format!("t3agent37 · {name}")
}

/// Sends the scripts inside the command (Agent37's exec takes no files; together they are well
/// under its 1 MB and the kernel's 128 KB argument limits) and runs `provision.sh`.
pub async fn run(
    a37: &Agent37,
    instance: &str,
    name: &str,
    t3_version: &str,
    watchdog_secret: &str,
    heartbeat_url: &str,
    mint_upstream_token: bool,
) -> Result<Provisioned> {
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    if !heartbeat_url.starts_with("https://") || !agent37::is_app_url(heartbeat_url) {
        bail!("unexpected heartbeat URL: {heartbeat_url}");
    }
    let script = format!(
        "set -e; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT\n\
         echo {p} | base64 -d > \"$d/provision.sh\"; echo {w} | base64 -d > \"$d/watchdog.mjs\"\n\
         T3A37_SECRET={s} bash \"$d/provision.sh\" {v} {sleep} {hb} {mint} {name}\n",
        p = b64(PROVISION_SH),
        w = b64(WATCHDOG_MJS),
        v = shell_word(t3_version)?,
        s = shell_word(watchdog_secret)?,
        sleep = SLEEP_AFTER_SECS,
        hb = agent37::shell_quote(heartbeat_url),
        mint = u8::from(mint_upstream_token),
        name = shell_word(name)?,
    );
    let out = a37.exec_ok(instance, &script, 600).await?;
    let line = out
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!("provisioning printed no result"))?;
    Ok(serde_json::from_str(line)?)
}

/// Only `[A-Za-z0-9._-]` reaches the remote shell.
fn shell_word(s: &str) -> Result<&str> {
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        bail!("unsafe value for the remote shell: {s}");
    }
    Ok(s)
}

/// The machine's T3 server and watchdog, as this computer reaches them: Agent37's edge, with the
/// workspace key the app adds.
pub fn urls(instance: &str) -> (String, String) {
    (agent37::port_url(instance, T3_PORT), agent37::port_url(instance, WATCHDOG_PORT))
}

/// The heartbeat's public URL, made at create; an older machine gets one here.
pub async fn heartbeat_url(a37: &Agent37, instance: &str) -> Result<String> {
    let found = a37.get(instance).await?.ok_or_else(|| anyhow!("the machine no longer exists on Agent37"))?;
    match found.public_ports.iter().find(|p| p.port == HEARTBEAT_PORT) {
        Some(p) => Ok(p.url.clone()),
        None => a37.add_public_port(instance, HEARTBEAT_PORT, "t3agent37 heartbeat").await,
    }
}

/// The machine's wake link, made on first use; `renew` replaces it, and the old one stops
/// working. Agent37 wakes a sleeping VM for any request to it, before the watchdog sees it.
pub async fn wake_url(a37: &Agent37, instance: &str, renew: bool) -> Result<String> {
    let found = a37.get(instance).await?.ok_or_else(|| anyhow!("the machine no longer exists on Agent37"))?;
    if let Some(p) = found.public_ports.iter().find(|p| p.port == WAKE_PORT) {
        if !renew {
            return Ok(p.url.clone());
        }
        a37.delete_public_port(instance, WAKE_PORT).await?;
    }
    a37.add_public_port(instance, WAKE_PORT, "t3agent37 wake link").await
}

/// Creates a new machine end to end. Takes about a minute, most of it the installs.
/// On failure after `created`, the caller deletes the instance.
pub async fn create(
    a37: &Agent37,
    name: &str,
    size: &str,
    t3_version: &str,
    local_port: u16,
    created: impl FnOnce(&str),
    progress: impl Fn(&str),
) -> Result<Machine> {
    let size_info = self::size(size)?;
    progress("Creating the VM on Agent37");
    let instance = a37
        .create(&instance_name(name), size_info, HEARTBEAT_PORT)
        .await
        .context("creating the Agent37 VM")?;
    let id = instance.id.clone();
    created(&id);
    let heartbeat = match instance.public_ports.iter().find(|p| p.port == HEARTBEAT_PORT) {
        Some(p) => p.url.clone(),
        None => heartbeat_url(a37, &id).await?,
    };
    progress("Installing T3 Code");
    let secret = random_token();
    let provisioned = run(a37, &id, name, t3_version, &secret, &heartbeat, true).await?;
    let (upstream_url, watchdog_url) = urls(&id);
    Ok(Machine {
        id: id.clone(),
        sandbox_id: id,
        name: name.to_owned(),
        size: size.to_owned(),
        upstream_url,
        upstream_token: provisioned
            .upstream_token
            .ok_or_else(|| anyhow!("provisioning issued no token"))?,
        watchdog_url,
        watchdog_secret: secret,
        heartbeat_url: heartbeat,
        local_port,
        t3_version: t3_version.to_owned(),
        provision_revision: revision(),
        environment_id: provisioned.environment_id,
        client_tokens: Vec::new(),
        link_tokens: Vec::new(),
        created_at: chrono::Utc::now().timestamp(),
        next_due_at: None,
        connect: false,
        github_login: None,
        github_checked: false,
    })
}
