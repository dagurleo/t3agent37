//! Persistent app state: machines, settings and this month's spend, one JSON file in the app
//! data dir.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    /// Never changes: the id of the Agent37 instance the machine was created on.
    pub id: String,
    /// Agent37 instance it runs on, e.g. `ab12cd34ef`. The same as `id`: Agent37 keeps a disk
    /// until the instance is deleted, so a machine never moves to another one.
    pub sandbox_id: String,
    pub name: String,
    pub size: String,
    /// HTTPS URL of the T3 server on the VM, through Agent37's edge (which wants the key).
    pub upstream_url: String,
    /// Bearer token the proxy uses upstream. Never leaves this computer except to that server.
    pub upstream_token: String,
    /// HTTPS URL of the watchdog on the VM, through Agent37's edge.
    pub watchdog_url: String,
    pub watchdog_secret: String,
    /// The VM's public heartbeat URL, which its watchdog calls to hold off Agent37's auto-sleep.
    #[serde(default)]
    pub heartbeat_url: String,
    /// Loopback port the proxy serves this machine on. Stable, T3 Code stores it.
    pub local_port: u16,
    /// T3 server version installed on the machine.
    pub t3_version: String,
    /// `provision::revision()` of the scripts the machine runs.
    pub provision_revision: String,
    pub environment_id: String,
    /// Bearer tokens the proxy issued to T3 Code when it paired.
    pub client_tokens: Vec<String>,
    /// Bearer tokens the machine's server issued to T3 servers that linked to it through the
    /// proxy. The proxy only recognizes them: a call bearing one may wake the machine.
    pub link_tokens: Vec<LinkToken>,
    pub created_at: i64,
    /// When T3 on the machine next starts work on its own (an automation, a held webhook run, an
    /// agent resuming after a usage limit), as its watchdog last said: ms since epoch.
    #[serde(default)]
    pub next_due_at: Option<i64>,
    /// T3 Connect is on there: other devices reach it with the user's T3 account.
    #[serde(default)]
    pub connect: bool,
    /// The GitHub account the machine's gh (and so its git) is signed in to, as last checked.
    #[serde(default)]
    pub github_login: Option<String>,
    /// Whether `github_login` was ever checked: unchecked, None means "don't know yet".
    #[serde(default)]
    pub github_checked: bool,
    /// What its GitHub token can push to, when it's a fine-grained one, as the machine tested it;
    /// None for gh's own sign-in, which reaches every repository the account can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_reach: Option<GithubReach>,
    /// When that token expires, as GitHub said when it was given: `2027-02-20 12:00:00 UTC`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_token_expires: Option<String>,
    /// T3 Connect: new devices wait for approval from one already connected. None until read.
    #[serde(default)]
    pub connect_approval: Option<bool>,
    /// T3 Connect: agent activity goes to the T3 Code phone app as push notifications and Live
    /// Activities. None until read.
    #[serde(default)]
    pub connect_push: Option<bool>,
    /// Its ports forwarded to this computer, in the order they were added.
    #[serde(default)]
    pub previews: Vec<Preview>,
    /// Its projects whose GitHub issues it works on (`machine/issues.rs`).
    #[serde(default)]
    pub issues: Vec<IssueWatch>,
    /// Its awake time by project, as its watchdog last said (kept for while it sleeps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProjectUsage>,
    /// Its inbox's public URL (`issues.mjs`), while it has GitHub issues on or webhook tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_url: Option<String>,
    /// T3's webhook tasks on it, as last read there (`machine/hooks.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookTask>,
    /// Its start URL, while it's on (`machine/start.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<StartLink>,
    /// T3's links from this machine to others (its agents may work there), as last read from it
    /// (`machine/peers.rs`): the user's other machines among them, and any the user made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<PeerLink>,
    /// Whether it takes links from the user's other machines: its inbox passes them to its T3.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accepts_links: bool,
    /// Its spending limit, when it's a throwaway one, what it was copied from, and its
    /// projects' env files.
    #[serde(flatten, default)]
    pub upkeep: Upkeep,
}

/// How t3agent37 looks after a machine beyond running it: its spending limit (and whether it was
/// stopped at one), when a throwaway machine is deleted, and the env files its projects got.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Upkeep {
    /// Dollars a UTC month, as Agent37 bills it: stopped there (`app/limits.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_limit_usd: Option<f64>,
    /// Stopped for reaching a limit, its own or the one on all machines: not woken until that
    /// limit is raised or the month is over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    /// The UTC month (`2026-11`) it was warned about nearing its own limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_warned: Option<String>,
    /// A throwaway machine: deleted at this time (unix seconds), or once its agents finish if they
    /// still work then (`app/lifetime.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_at: Option<i64>,
    /// The "deleted soon" notification went out for `delete_at`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub delete_warned: bool,
    /// The machine it was copied from, by name (`app/copy.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied_from: Option<String>,
    /// The env files each of its projects got from this computer, as last copied: names only,
    /// never values (`machine/env.rs`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<ProjectEnv>,
    /// A base: kept stopped, so copies start from it as it is (`app/base.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    /// A copy of a base made for a checkout on this computer (`app/spawn.rs`): its work comes
    /// back there, or it runs that checkout's commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawned: Option<Spawned>,
}

/// A machine kept stopped for copies to start from: signed in, with the user's projects and
/// tools. Nothing wakes it but `refresh`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Base {
    /// When it became one (unix seconds).
    pub since: i64,
    /// When a refresh last brought it up to date (unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<i64>,
}

/// A machine made for a checkout on this computer, new or a copy of a base: given its files
/// (`sent`, on top of `head`), it works there for a thread here (`runner` false) or runs the
/// checkout's commands (`t3agent37 run`, `runner` true).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Spawned {
    /// The checkout's folder on this computer (its git top level).
    pub checkout: String,
    /// The repository's folder on the machine.
    pub root: String,
    /// Its T3 project there.
    pub project_id: String,
    /// The branch it works on there.
    pub branch: String,
    /// The checkout's HEAD when it was given its files.
    pub head: String,
    /// What it was given: the checkout's files then, as a commit on `head`, kept here as
    /// `refs/t3agent37/<machine>/sent`.
    pub sent: String,
    /// The base it's a copy of; empty for a new machine.
    pub base: String,
    pub runner: bool,
    /// Seen with nothing running there since then (unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_since: Option<i64>,
    /// Its work came back here, to this branch of the checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected: Option<String>,
}

/// Why a machine is stopped and kept off.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    /// The UTC month it reached the limit in, `2026-11`: the next one lets it wake again.
    pub month: String,
    pub limit_usd: f64,
    /// The limit on all machines together, not its own.
    pub total: bool,
}

impl Pause {
    /// Why the machine stays off, and what lets it wake.
    pub fn why(&self, name: &str) -> String {
        let (this, next) = month_names(&self.month);
        if self.total {
            format!(
                "your machines reached their {} limit for {this} together, so {name} is stopped: raise the limit in Settings, or it can wake in {next}",
                dollars(self.limit_usd)
            )
        } else {
            format!("{name} reached its {} limit for {this} and is stopped: raise its limit, or it can wake in {next}", dollars(self.limit_usd))
        }
    }
}

/// An amount of dollars as people write it: "$10", "$12.50", and a fraction of a cent as it is
/// ("$0.0005"), not "$0.00".
pub fn dollars(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("${v:.0}")
    } else if v >= 0.01 {
        format!("${v:.2}")
    } else {
        format!("${}", (v * 1e6).round() / 1e6)
    }
}

/// "November" and "December" for `2026-11`.
pub fn month_names(month: &str) -> (String, String) {
    let first = chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").unwrap_or_default();
    let next = first.checked_add_months(chrono::Months::new(1)).unwrap_or(first);
    (first.format("%B").to_string(), next.format("%B").to_string())
}

/// A project's env files on a machine, as t3agent37 last copied them there.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnv {
    /// T3's project id on the machine.
    pub project_id: String,
    pub files: Vec<EnvFile>,
    /// Unix seconds.
    pub copied_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvFile {
    /// Relative to the project's folder.
    pub path: String,
    /// The variables it sets, in order: their names, never their values.
    pub names: Vec<String>,
    /// A hash of its contents, to tell whether this computer's copy changed since.
    pub hash: String,
}

/// One of T3's links from a machine to another T3 server, as T3 records it there.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PeerLink {
    /// The other server's environment id: a machine's `environment_id`.
    pub environment_id: String,
    /// T3's access level: what the linking machine's agents may do there.
    pub access: String,
    pub linked_at: String,
    /// When the link's token runs out (RFC 3339); it can't be renewed, only made again.
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// What a fine-grained GitHub token can push to, as its machine tested it repository by
/// repository (`machine/signin.rs`). GitHub doesn't say which repositories such a token reaches:
/// `/user/repos` lists the account's, whatever the token.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GithubReach {
    /// A short hash of the token it was tested for, so a new token is tested again; empty when
    /// the test failed, so it's tried again.
    pub token: String,
    pub repos: Vec<String>,
    /// The one owner every one of whose repositories it reaches ("All repositories").
    #[serde(default)]
    pub all_of: Option<String>,
}

impl GithubReach {
    /// "all 85 of dagurleo's repos", "only skildingur, tantalum and 2 more", "no repos".
    pub fn describe(&self) -> String {
        if self.token.is_empty() {
            return "limited to some repos (not checked yet)".into();
        }
        if let Some(owner) = &self.all_of {
            return format!("all {} of {owner}'s repos", self.repos.len());
        }
        let names: Vec<&str> = self.repos.iter().map(|r| r.rsplit('/').next().unwrap_or(r)).collect();
        match names.len() {
            0 => "can't push to any repo".into(),
            1..=3 => format!("only {}", names.join(", ")),
            n => format!("only {}, {} and {} more", names[0], names[1], n - 2),
        }
    }
}

/// A machine's start URL, `<inbox URL>/start/<token>`: the token makes it a secret.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartLink {
    pub token: String,
    /// A hash of the settings last written on the machine, so they go again when its models change.
    #[serde(default)]
    pub sent: u64,
}

/// One of T3's scheduled tasks that a webhook triggers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookTask {
    pub id: String,
    pub title: String,
    pub enabled: bool,
    pub project_id: String,
    /// Its path on T3's server, `/api/hooks/<task>/<token>`. The token makes it a secret.
    pub path: String,
    /// Deliveries must also be signed with its secret.
    pub signed: bool,
}

/// Awake time by project, as a machine's watchdog meters it: seconds per project id this UTC
/// month and today, "" for time no project's work explains (waking, setup, a wake for nothing).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectUsage {
    /// The UTC day it was counted up to, `YYYY-MM-DD`.
    pub day: String,
    pub today: BTreeMap<String, u64>,
    pub month: BTreeMap<String, u64>,
}

impl ProjectUsage {
    pub fn month_secs(&self) -> u64 {
        self.month.values().sum()
    }
}

/// A project whose GitHub repo sends its issues to the machine through a webhook.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IssueWatch {
    /// T3's project id on the machine.
    pub project_id: String,
    /// `owner/name` on GitHub.
    pub repo: String,
    pub hook_id: Option<u64>,
    /// The label and the agent. Missing in records from before they could be chosen.
    #[serde(flatten)]
    pub settings: IssueSettings,
}

/// Which label hands a project's issue to the machine, and the agent that works on it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct IssueSettings {
    pub label: String,
    /// T3's provider instance: `claudeAgent`, `codex`, ….
    pub provider: String,
    /// The model's slug there, e.g. `claude-opus-5-5`.
    pub model: String,
    /// T3's model options as `id=value`, e.g. `effort=high`.
    pub options: Vec<String>,
}

impl Default for IssueSettings {
    /// Claude Code at its best, on issues labeled `agent`.
    fn default() -> Self {
        Self { label: "agent".into(), provider: "claudeAgent".into(), model: "claude-opus-5-5".into(), options: vec!["effort=high".into()] }
    }
}

/// A port on the machine that `localhost:local_port` reaches here.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub port: u16,
    pub local_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkToken {
    pub token: String,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub start_at_login: bool,
    /// A warning when the Agent37 wallet drops below this many dollars; None: never.
    pub wallet_alert_usd: Option<f64>,
    /// A warning when a machine has been awake this many hours in a row; None: never.
    pub awake_alert_hours: Option<u32>,
    /// A notification when a machine's agents finish.
    pub notify_finished: bool,
    /// A project added to a machine gets its setup scripts run, or its dependencies installed.
    pub set_up_projects: bool,
    /// The panel's top: the pixel-art rooftop with its walking agents, instead of the still
    /// skyline.
    pub animated_rooftop: bool,
    /// How big the panel opens.
    pub panel_size: PanelSize,
    /// Dollars a UTC month for all machines together, as Agent37 bills them: every machine is
    /// stopped there; None: no limit.
    pub month_limit_usd: Option<f64>,
    /// Files and folders under the home folder (`.claude/skills`) that copies of a base get from
    /// this computer: the agents' settings as they are here now.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub base_sync: Vec<String>,
    /// Agents on this computer may send work to copies of the base (`spawn_machine`), with the
    /// `on-a-copy` skill for Claude Code and Codex here. The user's to turn on: copies cost.
    pub agents_spawn: bool,
    /// What spawned machines (`spawn`, `run`) start as: a copy of this base, by name, or a new
    /// machine with the sign-ins saved here (`app/newmachine.rs`) when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_from: Option<String>,
    /// A shell script on this computer that every new machine runs once it's made, before its
    /// projects arrive: the user's own tools (a Rust toolchain, say).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_setup: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_at_login: true,
            wallet_alert_usd: Some(5.0),
            awake_alert_hours: Some(3),
            notify_finished: true,
            set_up_projects: true,
            animated_rooftop: false,
            panel_size: PanelSize::Standard,
            month_limit_usd: None,
            base_sync: Vec::new(),
            agents_spawn: false,
            spawn_from: None,
            machine_setup: None,
        }
    }
}

/// The panel's sizes. It grows down and to both sides of the menu-bar item, and never past the
/// screen.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PanelSize {
    #[default]
    Standard,
    Large,
    Huge,
}

impl PanelSize {
    pub const ALL: [PanelSize; 3] = [PanelSize::Standard, PanelSize::Large, PanelSize::Huge];

    /// Width and height in points.
    pub fn dims(self) -> (f32, f32) {
        match self {
            PanelSize::Standard => (380., 560.),
            PanelSize::Large => (460., 760.),
            PanelSize::Huge => (580., 940.),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PanelSize::Standard => "Standard",
            PanelSize::Large => "Large",
            PanelSize::Huge => "Huge",
        }
    }

    /// The next size up or down, staying at the ends.
    pub fn step(self, up: bool) -> PanelSize {
        let i = PanelSize::ALL.iter().position(|s| *s == self).unwrap_or(0);
        let j = if up { (i + 1).min(PanelSize::ALL.len() - 1) } else { i.saturating_sub(1) };
        PanelSize::ALL[j]
    }
}

/// The choices Settings offers.
pub const WALLET_ALERTS: [Option<f64>; 4] = [None, Some(5.0), Some(10.0), Some(25.0)];
pub const AWAKE_ALERTS: [Option<u32>; 4] = [None, Some(1), Some(3), Some(8)];
/// Monthly limits, for all machines (Settings) and for one (its details).
pub const TOTAL_LIMITS: [Option<f64>; 6] = [None, Some(10.0), Some(25.0), Some(50.0), Some(100.0), Some(250.0)];
pub const MACHINE_LIMITS: [Option<f64>; 6] = [None, Some(2.0), Some(5.0), Some(10.0), Some(25.0), Some(50.0)];

/// One instance's spend as last looked up. Kept after the instance is gone (its machine deleted)
/// so the totals still count it until its month is over.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    /// Id of the machine that ran on the sandbox.
    pub machine: String,
    /// Start of the local day of the lookup, unix seconds.
    pub day: i64,
    /// Dollars since `day`.
    pub today: f64,
    pub awake_secs_today: u64,
    /// Dollars since the start of `day`'s month.
    pub month: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub machines: Vec<Machine>,
    pub settings: Settings,
    /// By instance id.
    pub spend: HashMap<String, Spend>,
    /// Local ports of deleted machines, never given out again: T3 Code may still list one (it
    /// cannot be told to forget a backend), and would reach a new machine there as the old.
    pub retired_ports: Vec<u16>,
    /// Instances of creates still running: deleted at the next start if the app quit or crashed
    /// before the create finished.
    pub pending_sandboxes: Vec<String>,
    /// The UTC month (`2026-11`) the user was warned about nearing the limit on all machines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_limit_warned: Option<String>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("state.json"),
        }
    }

    pub fn load(&self) -> Result<State> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// The file holds bearer tokens.
    pub fn save(&self, state: &State) -> Result<()> {
        write_private(&self.path, &serde_json::to_vec_pretty(state)?)
    }
}

/// Atomic write of a file only its owner can read, never readable by others even briefly.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().expect("path has a parent"))?;
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all(bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Removes everything in `dir` but `keep`, whatever an interrupted write left behind included.
pub fn sweep(dir: &Path, keep: &HashSet<PathBuf>) {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    for path in entries.map(|e| e.path()).filter(|p| !keep.contains(p)) {
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
    }
}
