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
const ISSUES_MJS: &str = include_str!("issues.mjs");
const START_MJS: &str = include_str!("start.mjs");
const ACTIVITY_MJS: &str = include_str!("activity.mjs");
const PEERS_MJS: &str = include_str!("peers.mjs");
/// This app's version, which provisioning leaves on the machine so an older t3agent37 on another
/// computer doesn't put its older scripts back.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const T3_PORT: u16 = 3773;
pub const WATCHDOG_PORT: u16 = 3774;
/// The watchdog answers its own heartbeat here, through a public URL: that traffic is what keeps
/// Agent37's auto-sleep off the VM while the watchdog wants it up.
pub const HEARTBEAT_PORT: u16 = 3775;
/// The wake link's port: a public URL of its own (so replacing it leaves the heartbeat alone)
/// that a phone opens to wake the machine. The watchdog answers it with a page and nothing else.
pub const WAKE_PORT: u16 = 3776;
/// The inbox's port (`issues.mjs`, in the watchdog): a public URL of its own, made when the first
/// repo's issues, webhook task or the start URL needs it. GitHub's webhooks deliver there, and
/// the start URL (`start.mjs`) is a path on it.
pub const INBOX_PORT: u16 = 3777;

/// Identifies the provisioning scripts; a machine provisioned with others gets them on wake.
pub fn revision() -> String {
    let scripts = PROVISION_SH.bytes().chain(WATCHDOG_MJS.bytes()).chain(ISSUES_MJS.bytes()).chain(START_MJS.bytes()).chain(ACTIVITY_MJS.bytes()).chain(PEERS_MJS.bytes());
    let hash = fnv1a(scripts.chain(SLEEP_AFTER_SECS.to_string().bytes()));
    format!("{hash:016x}")
}

/// FNV-1a: stable across builds, unlike std's hasher.
pub fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provisioned {
    pub environment_id: Option<String>,
    /// A newer t3agent37 (on another computer) provisioned the machine: its version. Nothing was
    /// changed, so as not to take its features away.
    #[serde(default)]
    pub skipped: Option<String>,
    /// The T3 version left running: the one asked for, or a newer one that T3 Code's own update
    /// put there (provision.sh never takes a machine back). Older scripts didn't say.
    pub t3_version: Option<String>,
    pub upstream_token: Option<String>,
}

/// Orders T3 versions as semver does: `0.1.12-nightly.20261121.5038` after `0.1.11` and before
/// `0.1.12`. One that doesn't parse (none recorded, say) comes before any that does.
pub fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    fn parse(v: &str) -> Option<([u64; 3], Vec<&str>)> {
        let v = v.split('+').next()?;
        let (core, pre) = match v.split_once('-') {
            Some((core, pre)) => (core, pre.split('.').collect()),
            None => (v, Vec::new()),
        };
        let mut n = core.split('.').map(|n| n.parse::<u64>().ok());
        let core = [n.next()??, n.next()??, n.next()??];
        n.next().is_none().then_some((core, pre))
    }
    match (parse(a), parse(b)) {
        (None, None) => Equal,
        (None, Some(_)) => Less,
        (Some(_), None) => Greater,
        (Some((a_core, a_pre)), Some((b_core, b_pre))) => a_core.cmp(&b_core).then_with(|| {
            match (a_pre.is_empty(), b_pre.is_empty()) {
                (true, true) => return Equal,
                (true, false) => return Greater,
                (false, true) => return Less,
                (false, false) => {}
            }
            for (x, y) in a_pre.iter().zip(&b_pre) {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Less,
                    (Err(_), Ok(_)) => Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Equal {
                    return order;
                }
            }
            a_pre.len().cmp(&b_pre.len())
        }),
    }
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

/// The most the provisioning command may be. Agent37's exec runs it as `bash -lc <command>`, one
/// argument, and Linux takes at most 128 KiB in one (`MAX_ARG_STRLEN`): over that, bash never
/// starts ("Argument list too long"), which broke creating and upgrading machines in 0.1.8 and
/// 0.1.9, at 142 KB. Kept well under, so a growing script fails this test instead.
const MAX_COMMAND: usize = 96 * 1024;

/// Sends the scripts inside the command (Agent37's exec takes no files), gzipped, and runs
/// `provision.sh`.
pub async fn run(
    a37: &Agent37,
    instance: &str,
    name: &str,
    t3_version: &str,
    watchdog_secret: &str,
    heartbeat_url: &str,
    mint_upstream_token: bool,
) -> Result<Provisioned> {
    if !heartbeat_url.starts_with("https://") || !agent37::is_app_url(heartbeat_url) {
        bail!("unexpected heartbeat URL: {heartbeat_url}");
    }
    let script = command(name, t3_version, watchdog_secret, heartbeat_url, mint_upstream_token)?;
    let out = a37.exec_ok(instance, &script, 600).await?;
    let line = out
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!("provisioning printed no result"))?;
    Ok(serde_json::from_str(line)?)
}

/// The provisioning command: each script gzipped, then base64 (nothing in it reaches the shell
/// but those letters), unpacked into a folder of its own, then `provision.sh` run from there.
fn command(name: &str, t3_version: &str, watchdog_secret: &str, heartbeat_url: &str, mint_upstream_token: bool) -> Result<String> {
    let files = [
        ("provision.sh", PROVISION_SH),
        ("watchdog.mjs", WATCHDOG_MJS),
        ("issues.mjs", ISSUES_MJS),
        ("start.mjs", START_MJS),
        ("activity.mjs", ACTIVITY_MJS),
        ("peers.mjs", PEERS_MJS),
    ];
    let mut script = String::from("set -e; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT\n");
    for (file, text) in files {
        script.push_str(&format!("echo {} | base64 -d | gunzip > \"$d/{file}\"\n", gzipped(text)));
    }
    script.push_str(&format!(
        "T3A37_SECRET={s} bash \"$d/provision.sh\" {v} {sleep} {hb} {mint} {name} {app}\n",
        v = shell_word(t3_version)?,
        s = shell_word(watchdog_secret)?,
        sleep = SLEEP_AFTER_SECS,
        hb = agent37::shell_quote(heartbeat_url),
        mint = u8::from(mint_upstream_token),
        name = shell_word(name)?,
        app = shell_word(APP_VERSION)?,
    ));
    if script.len() > MAX_COMMAND {
        bail!("the provisioning command is {} KB, over the {} KB a machine takes", script.len() / 1024, MAX_COMMAND / 1024);
    }
    Ok(script)
}

/// `text` gzipped, in base64.
fn gzipped(text: &str) -> String {
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(text.as_bytes()).expect("gzip in memory");
    base64::engine::general_purpose::STANDARD.encode(gz.finish().expect("gzip in memory"))
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

/// The GitHub inbox's public URL, made on first use. Its random hostname isn't what guards it:
/// the inbox acts only on deliveries signed with a webhook's secret.
pub async fn inbox_url(a37: &Agent37, instance: &str) -> Result<String> {
    let found = a37.get(instance).await?.ok_or_else(|| anyhow!("the machine no longer exists on Agent37"))?;
    match found.public_ports.iter().find(|p| p.port == INBOX_PORT) {
        Some(p) => Ok(p.url.clone()),
        None => a37.add_public_port(instance, INBOX_PORT, "t3agent37 inbox").await,
    }
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
    new_record(&id, name, size, local_port, provisioned, secret, heartbeat, t3_version)
}

/// The record of a machine just provisioned on `instance`, with the watchdog `secret` it was
/// given: a new one, or a copy (`app/copy.rs`).
#[allow(clippy::too_many_arguments)]
pub fn new_record(
    instance: &str,
    name: &str,
    size: &str,
    local_port: u16,
    provisioned: Provisioned,
    secret: String,
    heartbeat: String,
    t3_version: &str,
) -> Result<Machine> {
    let (upstream_url, watchdog_url) = urls(instance);
    Ok(Machine {
        id: instance.to_owned(),
        sandbox_id: instance.to_owned(),
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
        t3_version: provisioned.t3_version.unwrap_or_else(|| t3_version.to_owned()),
        provision_revision: revision(),
        environment_id: provisioned.environment_id.ok_or_else(|| anyhow!("provisioning gave no environment id"))?,
        client_tokens: Vec::new(),
        link_tokens: Vec::new(),
        created_at: chrono::Utc::now().timestamp(),
        next_due_at: None,
        connect: false,
        github_login: None,
        github_checked: false,
        github_reach: None,
        github_token_expires: None,
        connect_approval: None,
        connect_push: None,
        previews: Vec::new(),
        issues: Vec::new(),
        usage: None,
        inbox_url: None,
        hooks: Vec::new(),
        start: None,
        links: Vec::new(),
        accepts_links: false,
        upkeep: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::{MAX_COMMAND, cmp_versions, command};
    use std::cmp::Ordering::*;

    #[test]
    fn provisioning_fits_in_one_argument() {
        let script = command("raven", "0.1.12-nightly.20261122.5071", "abcdefghijklmnopqrstuvwxyz0123456789", "https://ve92hic1ra-3775.agent37.app", true).unwrap();
        assert!(script.len() < MAX_COMMAND, "{} bytes", script.len());
        // Quoted for `bash -lc`, as Agent37's exec gets it: still under Linux's 128 KiB.
        assert!(crate::agent37::shell_quote(&script).len() < 128 * 1024);
        // Each script comes back whole.
        use base64::Engine;
        use std::io::Read;
        let line = script.lines().find(|l| l.ends_with("\"$d/watchdog.mjs\"")).unwrap();
        let packed = line.strip_prefix("echo ").unwrap().split(' ').next().unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&base64::engine::general_purpose::STANDARD.decode(packed).unwrap()[..]).read_to_string(&mut text).unwrap();
        assert_eq!(text, super::WATCHDOG_MJS);
    }

    #[test]
    fn versions_order_as_semver() {
        let ordered = [
            "",
            "0.1.10",
            "0.1.11",
            "0.1.12-nightly.20261120.4994",
            "0.1.12-nightly.20261121.5038",
            "0.1.12-nightly.20261121.10000",
            "0.1.12-preview.20261111.4547",
            "0.1.12",
            "0.2.0",
        ];
        for (i, a) in ordered.iter().enumerate() {
            for (j, b) in ordered.iter().enumerate() {
                assert_eq!(cmp_versions(a, b), i.cmp(&j), "{a} vs {b}");
            }
        }
        assert_eq!(cmp_versions("0.1.12+build.1", "0.1.12"), Equal);
        assert_eq!(cmp_versions("not a version", "0.0.1"), Less);
    }
}
