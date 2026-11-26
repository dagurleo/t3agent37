//! One remote machine at runtime: its power state on Agent37, its watchdog, waking it on demand
//! and capturing its final state when it goes to sleep.

pub mod activity;
mod capture;
mod connect;
pub mod env;
pub mod hooks;
pub mod issues;
mod link;
pub mod peers;
pub mod preview;
pub mod projects;
pub mod provision;
pub mod shared;
pub mod signin;
pub mod start;
mod work;

use crate::agent37::{Agent37, EdgeClient, Instance, Resources};
use crate::proxy::auth::Auth;
use crate::proxy::cache::{self, Cache};
use crate::proxy::classify;
use crate::proxy::threads;
use crate::proxy::upstream;
use crate::store::{Machine, ProjectUsage, Settings};
use crate::t3desktop::T3Desktop;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, watch};

const WATCHDOG_POLL: Duration = Duration::from_secs(5);
const A37_POLL: Duration = Duration::from_secs(60);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(20);
/// A wake that moves the VM to another host copies its disk: a couple of minutes per 10 GB.
const READY_TIMEOUT: Duration = Duration::from_secs(420);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(120);
/// After a failed wake, requests fail fast instead of each retrying the start.
const WAKE_COOLDOWN: Duration = Duration::from_secs(30);
/// The watchdog's limit too.
pub const MAX_KEEP_AWAKE_MINUTES: u32 = 24 * 60;
/// Agent37's state read this long ago (by the wall clock, which runs on while the laptop sleeps)
/// is too old to act on: the VM may have gone to sleep meanwhile, and a request to it would wake it.
const API_FRESH: Duration = Duration::from_secs(30);
/// A loop iteration this late means this computer slept.
const LOOP_GAP: Duration = Duration::from_secs(20);
/// A sleep that failed: leave the VM to Agent37's auto-sleep this long, without a request to it
/// (the watchdog polls would count as activity and hold auto-sleep off).
const SLEEP_BACKOFF: Duration = Duration::from_secs(provision::IDLE_TIMEOUT_SECS + 240);
/// T3 should answer seconds after a wake; this long without it, with the VM running, its boot
/// container may be gone, and provisioning again puts it back.
const REPAIR_AFTER: Duration = Duration::from_secs(60);
/// A machine woken for work due is up this much ahead of it: a wake that boots fresh is slow.
const DUE_LEAD: chrono::TimeDelta = chrono::TimeDelta::minutes(2);
const DUE_WAKE_RETRY: Duration = Duration::from_secs(5 * 60);
/// A running machine's link is checked this often: links can be turned on, fail or near expiry
/// while it stays up.
const LINK_CHECK: Duration = Duration::from_secs(10 * 60);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Power {
    Unknown,
    Starting,
    Running,
    Stopping,
    Stopped,
    /// The VM is gone on Agent37, or the machine was deleted here. Final.
    Missing,
}

impl Power {
    /// Asleep and stopped both bill the disk alone and both wake with a start; asleep keeps the
    /// VM's memory, so its T3 server runs on as if nothing happened.
    fn from_instance(i: &Instance) -> Self {
        match i.status.as_str() {
            "running" => Power::Running,
            "sleeping" | "stopped" => Power::Stopped,
            "stopping" => Power::Stopping,
            "provisioning" | "starting" | "restarting" | "updating" | "waking" => Power::Starting,
            "deleting" | "deleted" => Power::Missing,
            // A lifecycle action failed: a start is what brings it back, so it counts as down.
            "failed" => Power::Stopped,
            _ => Power::Unknown,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub sequence: u64,
    pub threads: HashMap<String, u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WatchdogPhase {
    Active,
    Draining,
    Stopping,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogStatus {
    pub phase: WatchdogPhase,
    pub busy: bool,
    /// Agent runs in flight.
    pub working_threads: u32,
    pub working_subagents: u32,
    /// The top-level threads working, at most 10. Older watchdogs don't say.
    #[serde(default)]
    pub working: Vec<WorkingThread>,
    /// Busy only because of CPU use (a build or job outside an agent turn).
    pub cpu_busy: bool,
    /// T3 is not answering, so the watchdog keeps it up (at most 30 min) in case an agent works.
    pub agents_unknown: bool,
    /// Seconds left of a "keep awake"; 0 when off or until stopped.
    pub keep_awake_secs: u64,
    /// Kept awake until someone stops it: no auto sleep at all.
    pub keep_awake_forever: bool,
    pub idle_secs: u64,
    pub sleep_after_secs: u64,
    /// When T3 next starts work on its own, ms since epoch. Older watchdogs don't say.
    #[serde(default)]
    pub next_due_at: Option<i64>,
    /// That is soon: the machine waits for it instead of sleeping.
    #[serde(default)]
    pub work_due: bool,
    pub manifest: Option<Manifest>,
    /// Awake time by project this month and today. Older watchdogs don't say.
    #[serde(default)]
    pub usage: Option<ProjectUsage>,
    /// What it's doing: its agents' threads, what they finished, its load. Older watchdogs don't say.
    #[serde(default)]
    pub activity: Option<activity::Activity>,
    /// When this app got the status: what its second counts are relative to.
    #[serde(skip, default = "SystemTime::now")]
    pub received: SystemTime,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorkingThread {
    pub id: String,
    pub title: String,
}

/// Agents this long without work have finished, not paused between two turns.
const FINISHED_AFTER: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub enum Event {
    /// Something visible changed.
    Changed,
    /// A machine record changed and must be saved.
    Persist,
    Notify { title: String, body: String },
}

/// What every machine shares: the Agent37 account, T3 Code, settings and the app's event channel.
pub struct Shared {
    pub a37: Agent37,
    pub t3: T3Desktop,
    pub settings: RwLock<Settings>,
    pub events: mpsc::UnboundedSender<Event>,
    /// To the machines themselves, through Agent37's edge.
    pub http: EdgeClient,
}

impl Shared {
    pub fn changed(&self) {
        let _ = self.events.send(Event::Changed);
    }
    /// Saved by the event loop, the state file's one writer.
    pub fn persist(&self) {
        let _ = self.events.send(Event::Persist);
    }
    pub fn notify(&self, title: impl Into<String>, body: impl Into<String>) {
        let _ = self.events.send(Event::Notify { title: title.into(), body: body.into() });
    }
}

pub struct MachineRuntime {
    pub shared: Arc<Shared>,
    record: RwLock<Machine>,
    dir: PathBuf,
    pub cache: Mutex<Cache>,
    pub auth: Auth,
    power: watch::Sender<Power>,
    stopped_at: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    /// The VM's shape as Agent37 last reported it.
    resources: Mutex<Option<Resources>>,
    /// When Agent37's API last told us the VM's state (wall clock).
    api_seen: Mutex<Option<SystemTime>>,
    /// Until then, after a failed sleep, nothing is sent to the VM by the run loop.
    sleep_backoff: Mutex<Option<SystemTime>>,
    watchdog: Mutex<Option<WatchdogStatus>>,
    /// One wake at a time; a delete takes it too, so no restore swaps the sandbox under it.
    pub(crate) waking: tokio::sync::Mutex<()>,
    /// One capture at a time: "sleep now" and the background poll can see the same drain.
    capturing: tokio::sync::Mutex<()>,
    /// One sleep request at a time.
    sleeping: tokio::sync::Mutex<()>,
    last_heartbeat: Mutex<Option<Instant>>,
    /// When the watchdog was seen stopping the machine; cleared once Boat reports it stopped or
    /// the watchdog is active again.
    stop_seen: Mutex<Option<Instant>>,
    /// The last failed wake: its message and when.
    wake_failure: Mutex<Option<(String, Instant)>>,
    /// Its T3 projects as last read, and those t3boat is adding.
    projects: Mutex<Vec<projects::MachineProject>>,
    adding: Mutex<Vec<projects::MachineProject>>,
    /// GitHub's one-time code while its sign-in waits for it, for the panel to show.
    github_code: Mutex<Option<String>>,
    /// The agent whose sign-in runs (`signin.rs`), and its cancel: the flag, and a wake for the
    /// sign-in's wait.
    pub(super) signing_in: Mutex<Option<String>>,
    pub(super) sign_in_canceled: std::sync::atomic::AtomicBool,
    pub(super) sign_in_wake: tokio::sync::Notify,
    /// The T3 version being provisioned while `upgrade` runs, for the panel to say so.
    upgrading: Mutex<Option<String>>,
    /// Since when it has been up without a break (wall clock), and whether that stretch was
    /// already warned about.
    awake_since: Mutex<Option<SystemTime>>,
    pub(crate) awake_warned: Mutex<bool>,
    /// The models its agents offered when last listed (`issues.rs`), for while T3's list is
    /// briefly empty.
    pub(crate) models_seen: Mutex<Vec<issues::ModelChoice>>,
    /// Its ports forwarded to this computer, by port on the machine.
    forwards: Mutex<HashMap<u16, preview::Forward>>,
    /// The servers T3 found listening on it, as last read, and when.
    servers: Mutex<(Vec<preview::LocalServer>, Option<Instant>)>,
    /// What it was doing at the watchdog's last report, and when that was: kept while it sleeps.
    last_activity: Mutex<Option<(activity::Activity, SystemTime)>>,
    /// Why this app is waking it, for the watchdog's "woke for" (`wake_for`).
    wake_why: Mutex<Option<&'static str>>,
    /// A base is up for its refresh: it may wake (`app/base.rs`).
    pub(crate) base_open: std::sync::atomic::AtomicBool,
}

/// Marks a provisioning as running for as long as it's held, so the panel can say so.
struct Upgrading<'a>(&'a MachineRuntime);

impl<'a> Upgrading<'a> {
    fn start(m: &'a MachineRuntime, version: &str) -> Self {
        *m.upgrading.lock().unwrap() = Some(version.to_owned());
        m.shared.changed();
        Self(m)
    }
}

impl Drop for Upgrading<'_> {
    fn drop(&mut self) {
        *self.0.upgrading.lock().unwrap() = None;
        self.0.shared.changed();
    }
}

impl MachineRuntime {
    pub fn new(shared: Arc<Shared>, record: Machine, dir: PathBuf) -> Self {
        let cache = Cache::load(&cache::cache_path(&dir));
        let shell = cache.capture.as_ref().map(|c| c.shell.clone());
        let this = Self {
            shared,
            record: RwLock::new(record),
            dir,
            cache: Mutex::new(cache),
            auth: Auth::default(),
            power: watch::channel(Power::Unknown).0,
            stopped_at: Mutex::new(None),
            resources: Mutex::new(None),
            api_seen: Mutex::new(None),
            sleep_backoff: Mutex::new(None),
            watchdog: Mutex::new(None),
            waking: tokio::sync::Mutex::new(()),
            capturing: tokio::sync::Mutex::new(()),
            sleeping: tokio::sync::Mutex::new(()),
            last_heartbeat: Mutex::new(None),
            stop_seen: Mutex::new(None),
            wake_failure: Mutex::new(None),
            projects: Mutex::new(Vec::new()),
            adding: Mutex::new(Vec::new()),
            github_code: Mutex::new(None),
            signing_in: Mutex::new(None),
            sign_in_canceled: Default::default(),
            base_open: Default::default(),
            sign_in_wake: tokio::sync::Notify::new(),
            upgrading: Mutex::new(None),
            awake_since: Mutex::new(None),
            awake_warned: Mutex::new(false),
            models_seen: Mutex::new(Vec::new()),
            forwards: Mutex::new(HashMap::new()),
            servers: Mutex::new((Vec::new(), None)),
            last_activity: Mutex::new(None),
            wake_why: Mutex::new(None),
        };
        if let Some(shell) = shell {
            this.set_projects_from(&shell);
        }
        this
    }

    pub fn record(&self) -> Machine {
        self.record.read().unwrap().clone()
    }

    pub fn update_record(&self, f: impl FnOnce(&mut Machine)) {
        f(&mut self.record.write().unwrap());
        self.shared.persist();
        self.shared.changed();
    }

    pub fn id(&self) -> String {
        self.record.read().unwrap().id.clone()
    }

    /// The Agent37 instance the machine runs on.
    pub fn sandbox_id(&self) -> String {
        self.record.read().unwrap().sandbox_id.clone()
    }

    /// Where this machine's cache, chats and assets are kept on this computer.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn power(&self) -> Power {
        *self.power.borrow()
    }

    pub fn subscribe_power(&self) -> watch::Receiver<Power> {
        self.power.subscribe()
    }

    /// Deleted by this app: nothing may wake it or write to its dir again. Taken under the cache
    /// lock, so no `save_cache` that missed it is still writing.
    pub fn retire(&self) {
        let _cache = self.cache.lock().unwrap();
        self.set_power(Power::Missing);
        self.stop_forwards();
    }

    /// Since when it has been up without a break; none while it sleeps.
    pub fn awake_since(&self) -> Option<SystemTime> {
        *self.awake_since.lock().unwrap()
    }

    /// GitHub's one-time code while its sign-in waits for the user to enter it.
    pub fn github_code(&self) -> Option<String> {
        self.github_code.lock().unwrap().clone()
    }

    /// The T3 version the machine is being provisioned with right now.
    pub fn upgrading(&self) -> Option<String> {
        self.upgrading.lock().unwrap().clone()
    }

    /// The agent whose sign-in runs here, as `signed_out` names it.
    pub fn signing_in(&self) -> Option<String> {
        self.signing_in.lock().unwrap().clone()
    }

    /// Missing is final: an Agent37 answer or a wake still in flight from before must not revive a
    /// deleted machine.
    fn set_power(&self, p: Power) {
        let changed = self.power.send_if_modified(|cur| {
            if *cur == Power::Missing || *cur == p {
                return false;
            }
            *cur = p;
            true
        });
        if changed {
            if p != Power::Running {
                *self.watchdog.lock().unwrap() = None;
            }
            // A resize or a wake still in progress keeps the stretch going: the VM bills
            // through them.
            match p {
                Power::Running => {
                    self.awake_since.lock().unwrap().get_or_insert_with(SystemTime::now);
                }
                Power::Stopped | Power::Missing => {
                    *self.awake_since.lock().unwrap() = None;
                    *self.awake_warned.lock().unwrap() = false;
                }
                _ => {}
            }
            self.shared.changed();
        }
    }

    pub fn watchdog_status(&self) -> Option<WatchdogStatus> {
        self.watchdog.lock().unwrap().clone()
    }

    /// What it was doing at the watchdog's last report, and when that came: while it's up, as of
    /// seconds ago; asleep, as it went to sleep. None until a watchdog that reports it was seen.
    pub fn activity(&self) -> Option<(activity::Activity, SystemTime)> {
        self.last_activity.lock().unwrap().clone()
    }

    fn keep_activity(&self, s: &WatchdogStatus) {
        if let Some(a) = &s.activity {
            *self.last_activity.lock().unwrap() = Some((a.clone(), s.received));
        }
    }

    /// `ensure_awake`, saying what for: the machine's details then say what woke it ("T3 Code",
    /// "scheduled work"). A wake that was already under way keeps its own reason.
    pub async fn wake_for(&self, why: &'static str) -> Result<()> {
        if self.power() != Power::Running {
            self.wake_why.lock().unwrap().get_or_insert(why);
        }
        let result = self.ensure_awake().await;
        // Used by the wake, or not needed: either way not for a later one.
        self.wake_why.lock().unwrap().take();
        result
    }

    /// Long enough for any stop to finish; a stop still pending after this failed.
    const STOP_SETTLE: Duration = Duration::from_secs(120);

    pub fn last_error(&self) -> Option<String> {
        self.wake_failure.lock().unwrap().as_ref().map(|(e, _)| e.clone())
    }

    /// The machine is about to freeze or frozen: user actions must wait for it and wake it again.
    pub fn is_stopping(&self) -> bool {
        matches!(self.power(), Power::Stopping | Power::Stopped) || self.stop_pending()
    }

    fn stop_pending(&self) -> bool {
        self.stop_seen.lock().unwrap().is_some_and(|t| t.elapsed() < Self::STOP_SETTLE)
    }

    pub fn save_cache(&self) {
        // Serialized and written under the lock: one writer at a time for the file.
        let result = {
            let cache = self.cache.lock().unwrap();
            if self.power() == Power::Missing {
                return;
            }
            cache.to_bytes().and_then(|b| crate::store::write_private(&cache::cache_path(&self.dir), &b))
        };
        if let Err(e) = result {
            tracing::warn!("saving cache for {}: {e:#}", self.id());
        }
    }

    /// Reads the VM's state from Agent37's API, which never wakes it.
    pub async fn refresh_power(&self) -> Result<Power> {
        let id = self.sandbox_id();
        let mut found = self.shared.a37.get(&id).await?;
        if found.is_none() {
            // Missing is final: one answer must not retire a machine.
            tokio::time::sleep(Duration::from_secs(2)).await;
            found = self.shared.a37.get(&id).await?;
        }
        *self.api_seen.lock().unwrap() = Some(SystemTime::now());
        match found {
            Some(instance) => {
                *self.stopped_at.lock().unwrap() = instance.slept_at();
                *self.resources.lock().unwrap() = Some(instance.resources.clone());
                let mut p = Power::from_instance(&instance);
                if p == Power::Stopped {
                    *self.stop_seen.lock().unwrap() = None;
                } else if p == Power::Running && self.stop_pending() {
                    // The sleep was asked for and has not landed yet.
                    p = Power::Stopping;
                } else if p == Power::Running && self.waking.try_lock().is_err() {
                    // A wake is waiting for T3 to answer; it publishes Running once it does.
                    return Ok(p);
                }
                self.set_power(p);
                Ok(p)
            }
            None => {
                self.set_power(Power::Missing);
                Ok(Power::Missing)
            }
        }
    }

    /// Waits out transitional states, so callers see Running or Stopped (or Missing). A wake
    /// publishes its own outcome; the run loop resolves the rest.
    pub async fn settled_power(&self) -> Power {
        // "Running" from before this computer slept may be a VM asleep since: callers go on to
        // talk to it, which would wake it.
        if self.power() == Power::Running && !self.api_fresh() && self.waking.try_lock().is_ok() {
            let _ = self.refresh_power().await;
        }
        let mut rx = self.subscribe_power();
        let settled = |p: &Power| matches!(p, Power::Running | Power::Stopped | Power::Missing);
        let _ = tokio::time::timeout(SETTLE_TIMEOUT, rx.wait_for(settled)).await;
        self.power()
    }

    fn api_fresh(&self) -> bool {
        let seen = *self.api_seen.lock().unwrap();
        seen.is_some_and(|t| t.elapsed().is_ok_and(|e| e < API_FRESH))
    }

    fn sleep_backed_off(&self) -> bool {
        let until = *self.sleep_backoff.lock().unwrap();
        until.is_some_and(|t| SystemTime::now() < t)
    }

    pub fn upstream_url(&self) -> String {
        self.record.read().unwrap().upstream_url.trim_end_matches('/').to_owned()
    }

    pub fn upstream_token(&self) -> String {
        self.record.read().unwrap().upstream_token.clone()
    }

    /// The machine's own descriptor, never an edge error page. Cached for while the machine
    /// sleeps. A request reaching a sleeping VM wakes it: only for a machine that should be up.
    pub async fn fetch_descriptor(&self) -> Option<Value> {
        let url = format!("{}/.well-known/t3/environment", self.upstream_url());
        let res = self.shared.http.get(url).timeout(Duration::from_secs(3)).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().await.ok()?;
        if v["environmentId"].as_str() != Some(&self.record().environment_id) {
            return None;
        }
        self.cache.lock().unwrap().descriptor = Some(v.clone());
        // T3 Code updates machines itself too ("Update machines"): what the machine serves counts.
        if let Some(version) = v["serverVersion"].as_str().filter(|s| *s != self.record().t3_version) {
            self.server_changed(version.to_owned());
        }
        Some(v)
    }

    /// Brings the machine up if needed and returns once its T3 server answers and will stay up.
    /// Concurrent callers share one wake.
    pub async fn ensure_awake(&self) -> Result<()> {
        let _guard = self.waking.lock().await;
        // Checked under the lock: a delete holding it retires the machine.
        if self.power() == Power::Missing {
            bail!("the machine no longer exists on Agent37");
        }
        // Stopped at a spending limit: nothing wakes it until the limit is raised or the month
        // is over (`app/limits.rs`). Not a failed wake, so no notification for it either.
        if let Some(p) = self.record().upkeep.paused {
            bail!("{}", p.why(&self.record().name));
        }
        // A base stays stopped, as copies start from it, until a refresh opens it.
        if self.record().upkeep.base.is_some() && !self.base_open.load(std::sync::atomic::Ordering::SeqCst) {
            let name = self.record().name;
            bail!(
                "{name} is a base: it stays stopped so copies start from it as it is. `t3agent37 base {name} refresh` brings it \
                 up to date, `t3agent37 base {name} off` makes it a machine to work on again"
            );
        }
        if let Some((e, at)) = self.wake_failure.lock().unwrap().clone() {
            if at.elapsed() < WAKE_COOLDOWN {
                return Err(anyhow!(e));
            }
        }
        if self.power() == Power::Running && !self.is_stopping() && self.fetch_descriptor().await.is_some() {
            if self.watchdog_status().is_some_and(|w| w.phase == WatchdogPhase::Draining) {
                // Someone needs it after all: call the sleep off before it stops.
                self.send_activity(false).await;
            }
            // Too late if the watchdog is already stopping: wake_locked waits for it and resumes.
            if !self.is_stopping() {
                if self.wake_failure.lock().unwrap().take().is_some() {
                    self.shared.changed();
                }
                // A message accepted while another caller's wake was finishing.
                self.flush_deferred_commands().await;
                return Ok(());
            }
        }
        let result = self.wake_locked().await;
        let was_failing = self.wake_failure.lock().unwrap().is_some();
        match &result {
            Ok(()) => *self.wake_failure.lock().unwrap() = None,
            Err(e) => {
                let msg = format!("{e:#}");
                if !was_failing {
                    self.shared.notify(format!("{} could not wake", self.record().name), msg.clone());
                }
                *self.wake_failure.lock().unwrap() = Some((msg, Instant::now()));
                let _ = self.refresh_power().await;
            }
        }
        self.shared.changed();
        result
    }

    async fn wake_locked(&self) -> Result<()> {
        let id = self.id();
        let started = Instant::now();
        let fresh = self.fresh_capture().is_some();
        self.set_power(Power::Starting);
        let mut resumed = false;
        loop {
            match self.refresh_power().await? {
                Power::Running => break,
                Power::Stopped => {
                    tracing::info!("waking {id}");
                    self.set_power(Power::Starting);
                    self.shared.a37.start(&self.sandbox_id()).await?;
                    resumed = true;
                    break;
                }
                Power::Missing => bail!("the machine no longer exists on Agent37"),
                _ => {}
            }
            if started.elapsed() > READY_TIMEOUT {
                bail!("the machine is stuck between states on Agent37");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // From here on the machine runs: whatever it slept with is history. A sleep the proxy
        // did not see may have outdated the recorded reads too.
        if !fresh {
            self.cache.lock().unwrap().bump_epoch();
        }
        self.drop_capture();
        self.set_power(Power::Starting);
        self.wait_ready_or_repair(started).await?;
        if resumed {
            // T3 serves now and may have started work on its own; restarting it would kill that.
            if self.watchdog_get("/status?fresh").await.is_ok_and(|s| s.busy) {
                tracing::info!("{id} is busy: upgrade and resize deferred to the next wake");
            } else {
                if let Err(e) = self.upgrade_if_needed().await {
                    tracing::warn!("upgrading {id}: {e:#}");
                }
                // A size picked while it slept: a resize needs it running.
                if let Err(e) = self.resize_locked().await {
                    tracing::warn!("resizing {id}: {e:#}");
                }
            }
        }
        // Before Running is published: T3 Code reconnects then, and must find the messages
        // accepted while asleep ahead of anything it sends next.
        self.on_ready().await;
        self.set_power(Power::Running);
        tracing::info!("{id} awake after {:?}", started.elapsed());
        self.send_activity(resumed).await;
        Ok(())
    }

    /// T3 answers after a wake: sends what waited for it and refreshes the session.
    async fn on_ready(&self) {
        self.flush_deferred_commands().await;
        if let Err(e) = self.prime().await {
            tracing::warn!("refreshing {}'s session: {e:#}", self.id());
        }
    }

    /// Fills what the proxy must answer before the machine can sleep: descriptor and session.
    pub async fn prime(&self) -> Result<()> {
        self.fetch_descriptor().await.ok_or_else(|| anyhow!("T3 on the machine did not answer"))?;
        let session = self
            .shared
            .http
            .get(format!("{}/api/auth/session", self.upstream_url()))
            .bearer_auth(self.upstream_token())
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json::<Map<String, Value>>()
            .await
            .context("T3 on the machine sent a malformed auth session")?;
        self.cache.lock().unwrap().auth_session = Some(session);
        self.save_cache();
        Ok(())
    }

    /// `wait_ready`, but a VM that runs while T3 doesn't answer for a minute is provisioned again
    /// once: its boot container may be gone (an agent removed every container, say), and nothing
    /// else would bring T3 back.
    async fn wait_ready_or_repair(&self, started: Instant) -> Result<()> {
        let mut repaired = false;
        while self.fetch_descriptor().await.is_none() {
            if !repaired && started.elapsed() > REPAIR_AFTER && self.refresh_power().await? == Power::Running {
                repaired = true;
                tracing::warn!("T3 on {} does not answer: provisioning it again", self.id());
                if let Err(e) = self.upgrade(true).await {
                    tracing::warn!("repairing {}: {e:#}", self.id());
                }
            }
            if started.elapsed() > READY_TIMEOUT {
                bail!("T3 on the machine did not answer");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        Ok(())
    }

    async fn wait_ready(&self, started: Instant) -> Result<()> {
        while self.fetch_descriptor().await.is_none() {
            if started.elapsed() > READY_TIMEOUT {
                bail!("T3 on the machine did not answer");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        Ok(())
    }

    /// Keeps the machine on the desktop app's T3 version and on this app's scripts.
    pub async fn upgrade_if_needed(&self) -> Result<()> {
        self.upgrade(false).await
    }

    /// `force`: provisions again even when nothing changed, to repair the machine.
    async fn upgrade(&self, force: bool) -> Result<()> {
        let desktop = self.shared.t3.version().await?;
        let revision = provision::revision();
        let record = self.record();
        // T3 Code's own update may have taken the machine past the desktop app's version. It
        // never goes back: T3's database doesn't follow, and T3 refuses remote downgrades too.
        let target = match provision::cmp_versions(&desktop, &record.t3_version) {
            std::cmp::Ordering::Greater => desktop,
            _ => record.t3_version.clone(),
        };
        if !force && target == record.t3_version && revision == record.provision_revision {
            return Ok(());
        }
        tracing::info!("upgrading {} (T3 {target}, scripts {revision})", self.id());
        let _upgrading = Upgrading::start(self, &target);
        let heartbeat = match record.heartbeat_url.as_str() {
            "" => {
                let url = provision::heartbeat_url(&self.shared.a37, &record.sandbox_id).await?;
                self.update_record(|m| m.heartbeat_url = url.clone());
                url
            }
            url => url.to_owned(),
        };
        let provisioned = provision::run(
            &self.shared.a37,
            &record.sandbox_id,
            &record.name,
            &target,
            &record.watchdog_secret,
            &heartbeat,
            false,
        )
        .await?;
        if let Some(newer) = provisioned.skipped {
            // Recorded as done, so it isn't tried on every wake; a newer app here tries again.
            tracing::info!("{} has scripts from t3agent37 {newer}, newer than this one: left as they are", self.id());
            self.update_record(|m| m.provision_revision = revision);
            return Ok(());
        }
        let version = provisioned.t3_version.unwrap_or(target);
        self.update_record(|m| m.provision_revision = revision);
        self.cache.lock().unwrap().bump_epoch();
        if version != self.record().t3_version {
            self.server_changed(version);
        }
        self.wait_ready(Instant::now()).await
    }

    /// The machine's T3 runs another version now: after an upgrade here, or T3 Code's own update.
    /// What was recorded from the old server (streams, stored chats) is in its encoding.
    fn server_changed(&self, version: String) {
        tracing::info!("{} runs T3 {version}", self.id());
        self.update_record(|m| m.t3_version = version);
        {
            let mut cache = self.cache.lock().unwrap();
            cache.bump_epoch();
            cache.streams.clear();
        }
        threads::sweep(&self.dir, &HashSet::new());
        self.save_cache();
    }

    async fn flush_deferred_commands(&self) {
        // Left queued until delivered: the run loop retries a failed send.
        let commands = self.cache.lock().unwrap().deferred_commands.clone();
        if commands.is_empty() {
            return;
        }
        let requests = commands
            .iter()
            .map(|c| ("orchestration.dispatchCommand".to_owned(), c.clone()))
            .collect();
        match upstream::call_many(&self.upstream_url(), &self.upstream_token(), requests).await {
            Ok(results) => {
                self.cache.lock().unwrap().deferred_commands.retain(|c| !commands.contains(c));
                // T3 Code was told they were accepted: a refused message is said here or nowhere.
                for (command, result) in commands.iter().zip(results) {
                    if let Err(e) = result
                        && command["type"] == "message.dispatch"
                    {
                        tracing::warn!("message to {} refused: {e}", self.id());
                        self.shared.notify(format!("{}: a message was not sent", self.record().name), e);
                    }
                }
            }
            // Resent on the next try; the server dedupes them by commandId.
            Err(e) => {
                tracing::warn!("sending deferred commands: {e:#}");
                return;
            }
        }
        self.save_cache();
    }

    /// Someone used the machine: it stays awake, and recorded answers may be outdated.
    pub fn user_action(self: &Arc<Self>) {
        self.cache.lock().unwrap().bump_epoch();
        self.keep_up();
    }

    /// Someone uses the machine outside T3 (a forwarded port): it stays awake, at most one call
    /// to its watchdog per `HEARTBEAT_EVERY`.
    pub fn keep_up(self: &Arc<Self>) {
        if self.power() != Power::Running {
            return;
        }
        {
            let mut last = self.last_heartbeat.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < HEARTBEAT_EVERY) {
                return;
            }
            *last = Some(Instant::now());
        }
        let this = self.clone();
        tokio::spawn(async move { this.send_activity(false).await });
    }

    /// `resumed`: this app just resumed the sandbox. The watchdog was frozen mid-stop and, after
    /// a short sleep, cannot tell: it would refuse activity and report a stop for another 90 s.
    async fn send_activity(&self, resumed: bool) {
        *self.last_heartbeat.lock().unwrap() = Some(Instant::now());
        // Going quiet is not final on the VM: activity calls it off there, so a failure here
        // only means the watchdog wasn't reached. The run loop's next look decides.
        let by = if resumed { self.wake_why.lock().unwrap().take() } else { None };
        match self.watchdog_post("/activity", json!({ "resumed": resumed, "by": by })).await {
            Ok(s) => {
                self.keep_activity(&s);
                *self.watchdog.lock().unwrap() = Some(s);
                self.shared.changed();
            }
            Err(e) => tracing::debug!("activity for {}: {e:#}", self.id()),
        }
    }

    /// The watchdog went quiet: the VM sleeps now instead of in a few minutes, when Agent37's
    /// auto-sleep would take it. Never during a wake (the status then predates it). On failure,
    /// the VM is left alone for a while so auto-sleep can still take it.
    async fn sleep_vm(&self) {
        if self.waking.try_lock().is_err() || self.sleep_backed_off() {
            return;
        }
        let Ok(_one) = self.sleeping.try_lock() else { return };
        let id = self.sandbox_id();
        tracing::info!("sleeping {}", self.id());
        match self.shared.a37.sleep(&id).await {
            Ok(()) => *self.sleep_backoff.lock().unwrap() = None,
            Err(e) => {
                tracing::warn!("sleeping {}: {e:#}; leaving it to auto-sleep", self.id());
                *self.sleep_backoff.lock().unwrap() = Some(SystemTime::now() + SLEEP_BACKOFF);
                *self.stop_seen.lock().unwrap() = None;
            }
        }
        let _ = self.refresh_power().await;
    }

    pub(crate) async fn watchdog_get(&self, path: &str) -> Result<WatchdogStatus> {
        self.watchdog_request(reqwest::Method::GET, path, None).await
    }

    async fn watchdog_post(&self, path: &str, body: Value) -> Result<WatchdogStatus> {
        self.watchdog_request(reqwest::Method::POST, path, Some(body)).await
    }

    async fn watchdog_request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<WatchdogStatus> {
        let (url, secret) = {
            let m = self.record.read().unwrap();
            (format!("{}{path}", m.watchdog_url.trim_end_matches('/')), m.watchdog_secret.clone())
        };
        // These routes reply after the watchdog's own T3 check, which allows itself 10 s.
        let refreshes = matches!(path, "/status?fresh" | "/drain" | "/sleep-ack");
        let timeout = Duration::from_secs(if refreshes { 15 } else { 4 });
        let mut req = self.shared.http.request(method, url).bearer_auth(secret).timeout(timeout);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await?;
        let status = res.status();
        if status.as_u16() == 409 {
            let value: Value = res.json().await.unwrap_or_default();
            bail!("{}", value["error"].as_str().unwrap_or("not now"));
        }
        if !status.is_success() {
            bail!("watchdog {path}: HTTP {status}");
        }
        Ok(res.json().await?)
    }

    /// "Sleep now": asks the watchdog to drain. Refused while an agent works.
    pub async fn sleep_now(&self) -> Result<()> {
        if !self.awake_now().await? {
            return Ok(());
        }
        let s = self.watchdog_post("/drain", json!({})).await?;
        // Any use now calls the sleep off: the next user action must reach the watchdog.
        *self.last_heartbeat.lock().unwrap() = None;
        self.on_watchdog_status(s).await;
        Ok(())
    }

    /// Keeps the machine up for `minutes` whether or not anyone uses it (`None`: until stopped);
    /// `Some(0)` ends it. The watchdog enforces it, so it holds with the laptop closed too. Wakes a
    /// sleeping machine to start.
    pub async fn keep_awake(&self, minutes: Option<u32>) -> Result<()> {
        if minutes.is_some_and(|m| m > MAX_KEEP_AWAKE_MINUTES) {
            bail!("keep awake takes at most {MAX_KEEP_AWAKE_MINUTES} minutes, or until stopped");
        }
        // A stop ends any hold, so a machine that isn't running has none to end.
        if minutes == Some(0) && !self.awake_now().await? {
            return Ok(());
        }
        self.ensure_awake().await?;
        // Older scripts lack keep-awake: take this app's now, which restarts T3, so never
        // mid-turn. Ending a hold works with any watchdog that has one.
        if minutes != Some(0) && self.record().provision_revision != provision::revision() {
            if self.watchdog_get("/status?fresh").await?.busy {
                bail!("needs a quick update first; try again once its agents finish");
            }
            self.kept_awake(self.upgrade_if_needed()).await?;
        }
        let body = json!({ "minutes": minutes });
        let s = match self.watchdog_post("/keep-awake", body.clone()).await {
            Ok(s) => s,
            // Not reached because this app was putting it to sleep: ending it is then already
            // done; anything else waits for the sleep, wakes the machine and asks again.
            Err(_) if self.is_stopping() || self.sleeping.try_lock().is_err() => {
                if minutes == Some(0) {
                    return Ok(());
                }
                self.ensure_awake().await?;
                self.watchdog_post("/keep-awake", body).await?
            }
            Err(e) => return Err(e),
        };
        self.on_watchdog_status(s).await;
        Ok(())
    }

    /// New size. Agent37 resizes only a running VM, which then boots fresh: an idle machine is
    /// moved right away, keeping any "keep awake"; a working one (an agent or a build), or one
    /// asleep, moves at its next wake that finds it idle.
    pub async fn resize(&self, size: &str) -> Result<()> {
        provision::size(size)?;
        if self.record().size == size {
            return Ok(());
        }
        self.update_record(|m| m.size = size.to_owned());
        if self.settled_power().await != Power::Running {
            return Ok(());
        }
        let hold = match self.watchdog_get("/status?fresh").await {
            Ok(s) if !s.busy => match (s.keep_awake_forever, s.keep_awake_secs) {
                (true, _) => Some(None),
                (false, 0) => None,
                (false, secs) => Some(Some(secs.div_ceil(60) as u32)),
            },
            _ => return Ok(()),
        };
        {
            let _guard = self.waking.lock().await;
            self.resize_locked().await?;
        }
        self.send_activity(true).await;
        // The fresh boot ended any hold: it is taken again.
        if let Some(minutes) = hold {
            self.keep_awake(minutes).await?;
        }
        Ok(())
    }

    /// Moves a running, idle machine onto its recorded size, if it isn't on it. Under `waking`:
    /// requests wait for the reboot as for any wake.
    async fn resize_locked(&self) -> Result<()> {
        let want = provision::size(&self.record().size)?;
        if self.resources.lock().unwrap().is_none() {
            self.refresh_power().await?;
        }
        let Some(have) = self.resources.lock().unwrap().clone() else { return Ok(()) };
        if have.cpu == want.cpu && have.memory == want.memory {
            return Ok(());
        }
        let started = Instant::now();
        tracing::info!("resizing {} to {}", self.id(), want.spec);
        self.set_power(Power::Starting);
        if let Err(e) = self.shared.a37.resize(&self.sandbox_id(), want.cpu, want.memory).await {
            // Refused (the workspace's memory limit, no host with room): it runs on as it was.
            let _ = self.refresh_power().await;
            return Err(e);
        }
        self.cache.lock().unwrap().bump_epoch();
        loop {
            // The new shape first: until then the server answering is the one about to reboot.
            let moved = self.resources.lock().unwrap().as_ref().is_some_and(|r| r.cpu == want.cpu && r.memory == want.memory);
            if self.refresh_power().await? == Power::Running && moved && self.fetch_descriptor().await.is_some() {
                break;
            }
            if started.elapsed() > READY_TIMEOUT {
                bail!("T3 on the machine did not come back after the resize");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        self.on_ready().await;
        self.set_power(Power::Running);
        Ok(())
    }

    /// For actions on a running machine: true when it runs, false when it sleeps (or is gone).
    async fn awake_now(&self) -> Result<bool> {
        match self.settled_power().await {
            Power::Running => Ok(true),
            Power::Stopped | Power::Missing => Ok(false),
            _ => bail!("still waking; try again in a moment"),
        }
    }

    async fn on_watchdog_status(&self, s: WatchdogStatus) {
        if s.busy {
            self.cache.lock().unwrap().bump_epoch();
        }
        if s.phase == WatchdogPhase::Active {
            // Back in use after a drain: an earlier capture no longer describes the machine.
            self.drop_capture();
            *self.stop_seen.lock().unwrap() = None;
        }
        if self.record.read().unwrap().next_due_at != s.next_due_at {
            self.update_record(|r| r.next_due_at = s.next_due_at);
        }
        if let Some(usage) = &s.usage {
            // Saved for while it sleeps: on a new day, each minute more, and as it goes to sleep.
            let saved = self.record.read().unwrap().usage.clone();
            let due = saved.as_ref().is_none_or(|old| {
                old.day != usage.day || usage.month_secs() >= old.month_secs() + 60 || (s.phase != WatchdogPhase::Active && old != usage)
            });
            if due {
                self.update_record(|r| r.usage = Some(usage.clone()));
            }
        }
        let manifest = s.manifest.clone();
        let phase = s.phase;
        self.keep_activity(&s);
        *self.watchdog.lock().unwrap() = Some(s);
        self.shared.changed();
        match (phase, manifest) {
            (WatchdogPhase::Draining, Some(m)) => {
                {
                    // Whoever waited here finds the capture done, and must not redo it: it may
                    // be confirmed by now.
                    let _capturing = self.capturing.lock().await;
                    if self.captured_sequence() != Some(m.sequence) {
                        self.drop_capture();
                        if let Err(e) = self.capture(&m).await {
                            tracing::warn!("capture failed for {}: {e:#}", self.id());
                            return;
                        }
                    }
                }
                // Still draining means the watchdog couldn't stop yet (T3 not answering): leave the
                // retry to the regular poll instead of re-acking in a loop.
                match self.watchdog_post("/sleep-ack", json!({ "sequence": m.sequence })).await {
                    Ok(s) if s.phase != WatchdogPhase::Draining => Box::pin(self.on_watchdog_status(s)).await,
                    _ => {}
                }
            }
            // Stopping without a manifest (the watchdog could not read it) still stops: only
            // the capture goes unconfirmed.
            (WatchdogPhase::Stopping, m) => {
                // A wake in progress: this status was read before the watchdog learnt of it.
                if self.waking.try_lock().is_err() {
                    return;
                }
                if m.is_some_and(|m| self.captured_sequence() == Some(m.sequence)) {
                    self.confirm_capture();
                }
                self.mark_stopping();
                self.sleep_vm().await;
            }
            _ => {}
        }
    }

    /// For background reads and checks: only while it runs and isn't about to sleep. Each goes to
    /// the VM's URLs or its exec, and either would wake a VM that went to sleep meanwhile.
    pub(super) fn up_for_background(&self) -> bool {
        self.power() == Power::Running && !self.is_stopping() && self.sleeping.try_lock().is_ok()
    }

    /// Known before Agent37 says so: new sockets and wakes wait for the sleep instead of reaching
    /// a machine that is about to freeze.
    fn mark_stopping(&self) {
        // From this attempt: an older one that failed must not make this one look settled.
        if !self.stop_pending() {
            *self.stop_seen.lock().unwrap() = Some(Instant::now());
        }
        self.set_power(Power::Stopping);
    }

    /// Background loop: follows the machine's power state and its watchdog.
    pub async fn run(self: Arc<Self>) {
        let unprimed = {
            let c = self.cache.lock().unwrap();
            c.descriptor.is_none() || c.auth_session.is_none()
        };
        if unprimed {
            // Without these T3 Code cannot even connect while the machine sleeps.
            let primed = async {
                self.ensure_awake().await?;
                self.prime().await
            };
            if let Err(e) = primed.await {
                tracing::warn!("priming {}: {e:#}", self.id());
            }
        }
        // By the wall clock: Instant stands still while this computer sleeps.
        let mut last_a37_poll: Option<SystemTime> = None;
        let mut last_loop: Option<SystemTime> = None;
        let mut last_due_wake: Option<Instant> = None;
        let mut last_link_check: Option<Instant> = None;
        let mut watchdog_failures = 0u32;
        let mut was_busy = false;
        let mut was_running = false;
        // The threads last seen working, and since when none are.
        let mut last_working: Vec<WorkingThread> = Vec::new();
        let mut idle_since: Option<Instant> = None;
        loop {
            let transitioning = matches!(self.power(), Power::Stopping | Power::Starting | Power::Unknown);
            let now = SystemTime::now();
            let woke_here = last_loop.is_some_and(|t| now.duration_since(t).unwrap_or_default() > LOOP_GAP);
            last_loop = Some(now);
            // The VM decides on its own and may have gone to sleep since the last look; a
            // request to its watchdog would wake it, so Agent37's API is asked first.
            let winding_down = self.watchdog_status().is_some_and(|w| w.phase != WatchdogPhase::Active);
            let stale = last_a37_poll.is_none_or(|t| t.elapsed().map_or(true, |e| e >= A37_POLL));
            if transitioning || woke_here || winding_down || stale || watchdog_failures >= 2 {
                last_a37_poll = Some(now);
                watchdog_failures = 0;
                if let Err(e) = self.refresh_power().await {
                    tracing::warn!("agent37 status of {}: {e:#}", self.id());
                }
            }
            if self.power() == Power::Missing {
                return;
            }
            let due = self.scheduled_wake().is_some_and(|due| chrono::Utc::now() >= due - DUE_LEAD);
            if (due || self.has_queued_wake())
                && last_due_wake.is_none_or(|t| t.elapsed() >= DUE_WAKE_RETRY)
            {
                last_due_wake = Some(Instant::now());
                tracing::info!("waking {} for work due", self.id());
                if let Err(e) = self.wake_for("scheduled work").await {
                    tracing::warn!("waking {} for work due: {e:#}", self.id());
                }
            }
            let running = self.power() == Power::Running;
            let came_up = running && !was_running;
            was_running = running;
            // After a failed sleep, quiet: auto-sleep needs the VM's URLs left alone.
            if running && !self.sleep_backed_off() {
                if came_up || last_link_check.is_none_or(|t| t.elapsed() >= LINK_CHECK) {
                    last_link_check = Some(Instant::now());
                    let this = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.ensure_linked().await {
                            tracing::warn!("linking {} into T3 Code: {e:#}", this.id());
                        }
                        if came_up {
                            this.refresh_projects().await;
                            this.refresh_connect().await;
                            this.refresh_github().await;
                            this.refresh_agents().await;
                            // Before the hooks: they keep the inbox URL only for what's on.
                            this.refresh_settings().await;
                            this.refresh_hooks().await;
                            this.refresh_start().await;
                        }
                    });
                }
                // Retries a send that failed during the wake.
                self.flush_deferred_commands().await;
                match self.watchdog_get("/status").await {
                    Ok(s) => {
                        watchdog_failures = 0;
                        if was_busy && !s.busy {
                            // The turn's last edits may have landed after the last busy poll.
                            self.cache.lock().unwrap().bump_epoch();
                            let this = self.clone();
                            tokio::spawn(async move { this.refresh_projects().await });
                        }
                        was_busy = s.busy;
                        self.follow_agents(&s, &mut last_working, &mut idle_since);
                        if !s.agents_unknown && self.wake_failure.lock().unwrap().take().is_some() {
                            // A failed wake came up after all: T3 answers.
                            self.shared.changed();
                            let this = self.clone();
                            tokio::spawn(async move { this.on_ready().await });
                        }
                        self.on_watchdog_status(s).await;
                    }
                    Err(_) => watchdog_failures += 1,
                }
            }
            tokio::time::sleep(WATCHDOG_POLL).await;
        }
    }

    /// Says when agents that worked have all finished: once none has worked for
    /// `FINISHED_AFTER`, so a pause between two turns doesn't count.
    fn follow_agents(&self, s: &WatchdogStatus, last_working: &mut Vec<WorkingThread>, idle_since: &mut Option<Instant>) {
        if s.agents_unknown {
            return;
        }
        if s.working_threads + s.working_subagents > 0 {
            // The latest names, but a thread that finished while another runs stays in the list.
            for t in &s.working {
                if !last_working.iter().any(|w| w.id == t.id) {
                    last_working.push(t.clone());
                }
            }
            if last_working.is_empty() {
                last_working.push(WorkingThread { id: String::new(), title: String::new() });
            }
            *idle_since = None;
            return;
        }
        if last_working.is_empty() {
            return;
        }
        let since = *idle_since.get_or_insert_with(Instant::now);
        if since.elapsed() < FINISHED_AFTER {
            return;
        }
        let finished = std::mem::take(last_working);
        *idle_since = None;
        if !self.shared.settings.read().unwrap().notify_finished {
            return;
        }
        // A spawned copy's work reaches the thread that asked for it, and the copy goes after:
        // that it finished is no news (it said "sleeps in 5 minutes", 2026-11-25).
        if self.record().upkeep.spawned.is_some() {
            return;
        }
        let name = self.record().name;
        let titles: Vec<String> = finished.iter().filter(|t| !t.title.is_empty()).map(|t| format!("“{}”", t.title)).collect();
        let what = match titles.len() {
            0 => "Its agents are done.".to_owned(),
            1 => format!("{} is done.", titles[0]),
            n => format!("{n} threads are done: {}.", titles.join(", ")),
        };
        let then = if s.keep_awake_forever || s.keep_awake_secs > 0 {
            " It stays awake as you asked."
        } else {
            " It sleeps in 5 minutes unless you use it."
        };
        self.shared.notify(format!("{name} finished"), format!("{what}{then}"));
    }

    /// When work T3 starts on its own (an automation, ...) comes due on a sleeping machine, which
    /// wakes for it as an always-on server would run it. Only work due after it went to sleep:
    /// what came due while it ran had its chance.
    pub fn scheduled_wake(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let due = chrono::DateTime::from_timestamp_millis(self.record.read().unwrap().next_due_at?)?;
        let stopped_at = (*self.stopped_at.lock().unwrap())?;
        (self.power() == Power::Stopped && due > stopped_at).then_some(due)
    }

    /// A message T3 Code was told is sent, still queued on a sleeping machine: its wake failed
    /// or never ran.
    fn has_queued_wake(&self) -> bool {
        self.power() == Power::Stopped
            && self.cache.lock().unwrap().deferred_commands.iter().any(|c| {
                c["type"].as_str().is_some_and(|k| classify::WAKING_COMMANDS.contains(&k))
            })
    }

}
