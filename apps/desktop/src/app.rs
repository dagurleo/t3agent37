//! The app's model: machines, settings, setup checks. UI-agnostic; the tray and the dev CLI
//! both drive it.

use crate::agent37::Agent37;
use crate::machine::activity::{needs_you, ActivityView};
use crate::machine::{provision, Event, MachineRuntime, Power, Shared, WatchdogPhase, WatchdogStatus};
use crate::proxy;
use crate::store::{Settings, Spend, State, Store};
use crate::t3desktop::T3Desktop;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Clear of t3boat's ports (47731+): T3 Code may know a port from either app.
const FIRST_PORT: u16 = 47371;
/// Machines ever created here: deleted ones keep their port.
const PORTS: u16 = 1000;

/// Why the app cannot run yet, in the order the user has to fix it.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Setup {
    /// The first check is still running.
    #[default]
    Starting,
    Ready,
    /// No Agent37 API key on this computer yet.
    ConnectAgent37,
    InstallT3,
}

/// The key was checked with Agent37 when it was saved; a revoked one shows up as failing calls.
pub async fn check_setup(data_dir: &std::path::Path) -> (Setup, Option<Agent37>, Option<T3Desktop>) {
    let Some(a37) = Agent37::load(data_dir) else { return (Setup::ConnectAgent37, None, None) };
    let Some(t3) = T3Desktop::locate() else { return (Setup::InstallT3, Some(a37), None) };
    (Setup::Ready, Some(a37), Some(t3))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MachineState {
    Working,
    Idle,
    Sleeping,
    Asleep,
    Waking,
    Creating,
    Error,
    Missing,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineView {
    pub id: String,
    pub name: String,
    pub size: String,
    pub state: MachineState,
    pub detail: String,
    /// Set while `detail` counts down, so the UI can tick it between updates.
    pub countdown: Option<Countdown>,
    pub paired: bool,
    /// Agents that need a sign-in on this machine (done in T3 Code, per machine).
    pub signed_out: Vec<String>,
    /// Coding agents signed in on it (Claude, Codex; not GitHub).
    pub signed_in: Vec<String>,
    /// Its T3 projects as last seen, and those being added (with their progress).
    pub projects: Vec<crate::machine::projects::MachineProject>,
    /// When a "keep awake" runs out (unix ms); none when off or until stopped.
    pub keep_awake_until: Option<u64>,
    pub keep_awake_forever: bool,
    /// GitHub's one-time code while a GitHub sign-in waits for it.
    pub github_code: Option<String>,
    /// The agent whose sign-in runs, until it's done or canceled.
    pub signing_in: Option<String>,
    /// What it's doing, or did before it went to sleep (the details' "now"); machine_details
    /// gives it to agents.
    #[serde(skip)]
    pub activity: Option<ActivityView>,
}

/// `detail` as `template` with `{left}` filled in: the time left until `ends_at` (unix ms).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Countdown {
    pub template: &'static str,
    pub ends_at: u64,
}

impl Countdown {
    fn new(template: &'static str, secs: u64, from: SystemTime) -> Self {
        Self { template, ends_at: unix_ms_after(from, secs) }
    }

    fn text(&self, secs: u64) -> String {
        self.template.replace("{left}", &human_secs(secs))
    }
}

/// Saturates: `secs` comes from the machine and may be anything.
fn unix_ms_after(from: SystemTime, secs: u64) -> u64 {
    let from_ms = from.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    from_ms.saturating_add(secs.saturating_mul(1000))
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineUsage {
    pub today: f64,
    pub month: f64,
    pub awake_secs_today: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Costs {
    pub today: f64,
    pub month: f64,
    /// What is left in the Agent37 wallet, which every machine draws on.
    pub balance: Option<f64>,
}

/// A machine being created: it holds its name and local port until it exists.
struct Creating {
    name: String,
    step: String,
    port: u16,
    /// Its Agent37 instance, once there is one.
    sandbox: Option<String>,
}

struct Running {
    machine: Arc<MachineRuntime>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct App {
    pub shared: Arc<Shared>,
    store: Store,
    data_dir: PathBuf,
    machines: RwLock<Vec<Running>>,
    creating: Mutex<Vec<Creating>>,
    balance: Mutex<Option<f64>>,
    /// The low-wallet warning went out, and the balance hasn't been above its line since.
    wallet_warned: Mutex<bool>,
    /// By sandbox id, gone sandboxes included: `Spend`.
    spend: Mutex<HashMap<String, Spend>>,
    retired_ports: Mutex<Vec<u16>>,
    /// Sandboxes of creates the app didn't live to finish, until deleted.
    leftover_sandboxes: Mutex<Vec<String>>,
    /// t3agent37's machines on the account that this computer doesn't have, as last listed.
    elsewhere: Mutex<Vec<ElsewhereMachine>>,
}

/// A machine t3agent37 made on this Agent37 account that this computer doesn't have: one made on
/// another computer, or here before this app's data was lost. It can be added here.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ElsewhereMachine {
    pub id: String,
    pub name: String,
    /// None for a shape none of the sizes has.
    pub size: Option<String>,
    pub asleep: bool,
}

impl App {
    pub fn new(
        a37: Agent37,
        t3: T3Desktop,
        data_dir: PathBuf,
        events: mpsc::UnboundedSender<Event>,
    ) -> Result<Arc<Self>> {
        let store = Store::new(&data_dir);
        let state = store.load().context("reading t3agent37 state")?;
        // A machine saved meanwhile owns its sandbox.
        let leftover: Vec<String> = state
            .pending_sandboxes
            .into_iter()
            .filter(|s| !state.machines.iter().any(|m| &m.sandbox_id == s))
            .collect();
        let shared = Arc::new(Shared {
            a37,
            t3: t3.with_token_file(data_dir.join("t3-token")),
            settings: RwLock::new(state.settings.clone()),
            events,
            http: crate::agent37::EdgeClient::new()?,
        });
        let app = Arc::new(Self {
            shared,
            store,
            data_dir,
            machines: RwLock::new(Vec::new()),
            creating: Mutex::new(Vec::new()),
            balance: Mutex::new(None),
            wallet_warned: Mutex::new(false),
            spend: Mutex::new(state.spend),
            retired_ports: Mutex::new(state.retired_ports),
            leftover_sandboxes: Mutex::new(leftover),
            elsewhere: Mutex::new(Vec::new()),
        });
        for record in state.machines {
            app.spawn(app.runtime(record));
        }
        tokio::spawn(app.clone().delete_leftover_sandboxes());
        Ok(app)
    }

    async fn delete_leftover_sandboxes(self: Arc<Self>) {
        let sandboxes = self.leftover_sandboxes.lock().unwrap().clone();
        for id in sandboxes {
            self.delete_leftover(&id).await;
        }
    }

    /// Unfinished, unlisted and billed without limit: deleted, or the user told. Kept until
    /// Agent37 confirms it gone, so the next launch tries again.
    async fn delete_leftover(&self, id: &str) {
        let a37 = &self.shared.a37;
        if let Err(e) = a37.delete(id).await {
            if !a37.confirmed_gone(id).await {
                self.shared.notify(
                    "A VM may be left on Agent37",
                    format!("Couldn't delete {id} from an unfinished create ({e:#}); delete it in the Agent37 dashboard"),
                );
                return;
            }
        }
        self.leftover_sandboxes.lock().unwrap().retain(|s| s != id);
        self.shared.persist();
    }

    fn machine_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("machines").join(id)
    }

    fn runtime(&self, record: crate::store::Machine) -> Arc<MachineRuntime> {
        let dir = self.machine_dir(&record.id);
        Arc::new(MachineRuntime::new(self.shared.clone(), record, dir))
    }

    /// Starts the machine's proxy and background loop.
    fn spawn(&self, machine: Arc<MachineRuntime>) {
        let proxy_machine = machine.clone();
        let proxy = tokio::spawn(async move {
            let port = proxy_machine.record().local_port;
            if let Err(e) = proxy::serve(proxy_machine.clone()).await {
                tracing::error!("proxy on port {port} failed: {e:#}");
                proxy_machine.shared.notify(
                    "t3agent37",
                    format!("{} cannot listen on port {port}: {e}", proxy_machine.record().name),
                );
            }
        });
        let runner = tokio::spawn(machine.clone().run());
        machine.start_forwards();
        self.machines.write().unwrap().push(Running { machine, tasks: vec![proxy, runner] });
    }

    pub fn machines(&self) -> Vec<Arc<MachineRuntime>> {
        self.machines.read().unwrap().iter().map(|r| r.machine.clone()).collect()
    }

    /// By id, or by name ignoring case.
    pub fn machine(&self, machine: &str) -> Result<Arc<MachineRuntime>> {
        self.machines()
            .into_iter()
            .find(|m| m.id() == machine || m.record().name.eq_ignore_ascii_case(machine))
            .ok_or_else(|| anyhow!("no machine {machine}"))
    }

    pub fn settings(&self) -> Settings {
        self.shared.settings.read().unwrap().clone()
    }

    /// Only the event loop calls this on `Event::Persist`: one writer for the state file.
    pub fn save(&self) {
        // One lock at a time: a guard in a field initializer would live until the end of the `let`.
        let spend = self.spend.lock().unwrap().clone();
        let retired_ports = self.retired_ports.lock().unwrap().clone();
        let creating: Vec<_> = self.creating.lock().unwrap().iter().filter_map(|c| c.sandbox.clone()).collect();
        let mut pending_sandboxes = self.leftover_sandboxes.lock().unwrap().clone();
        pending_sandboxes.extend(creating);
        let state = State {
            machines: self.machines().iter().map(|m| m.record()).collect(),
            settings: self.settings(),
            spend,
            retired_ports,
            pending_sandboxes,
        };
        if let Err(e) = self.store.save(&state) {
            tracing::error!("saving state: {e:#}");
        }
    }

    pub fn update_settings(&self, settings: Settings) {
        *self.shared.settings.write().unwrap() = settings;
        self.shared.persist();
    }

    /// Claims the name and a local port for a new machine, so concurrent creates never share
    /// either.
    fn reserve(&self, name: &str) -> Result<u16> {
        let mut creating = self.creating.lock().unwrap();
        let records: Vec<_> = self.machines().iter().map(|m| m.record()).collect();
        let names = records.iter().map(|m| &m.name).chain(creating.iter().map(|c| &c.name));
        if names.into_iter().any(|n| n.eq_ignore_ascii_case(name)) {
            bail!("a machine called {name} already exists");
        }
        let retired = self.retired_ports.lock().unwrap();
        let used: Vec<u16> = records
            .iter()
            .map(|m| m.local_port)
            .chain(creating.iter().map(|c| c.port))
            .chain(retired.iter().copied())
            .collect();
        let port = (FIRST_PORT..FIRST_PORT + PORTS)
            .find(|p| !used.contains(p) && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
            .ok_or_else(|| anyhow!("no free local port"))?;
        creating.push(Creating { name: name.to_owned(), step: "Preparing".into(), port, sandbox: None });
        Ok(port)
    }

    /// Saved before provisioning starts: a quit or crash from here on leaves the VM billed.
    fn set_creating_sandbox(&self, name: &str, sandbox: &str) {
        if let Some(c) = self.creating.lock().unwrap().iter_mut().find(|c| c.name == name) {
            c.sandbox = Some(sandbox.to_owned());
        }
        self.shared.persist();
    }

    fn set_creating_step(&self, name: &str, step: &str) {
        if let Some(c) = self.creating.lock().unwrap().iter_mut().find(|c| c.name == name) {
            c.step = step.to_owned();
        }
        self.shared.changed();
    }

    /// Creates a machine, then adds this computer's T3 projects `projects` (ids) to it.
    pub async fn create_machine(self: &Arc<Self>, name: &str, size: &str, projects: Vec<String>) -> Result<Arc<MachineRuntime>> {
        if !valid_name(name) {
            bail!("machine names are 1-32 lowercase letters, digits or dashes, starting with a letter or digit");
        }
        let port = self.reserve(name)?;
        self.shared.changed();
        let result = async {
            let version = self.shared.t3.version().await?;
            let record = provision::create(
                &self.shared.a37,
                name,
                size,
                &version,
                port,
                |sandbox| self.set_creating_sandbox(name, sandbox),
                |step| self.set_creating_step(name, step),
            )
            .await?;
            // Counted from the start: usage settles hourly, and a machine deleted before its first
            // hour shows up only once it is gone.
            let (day, _) = periods();
            self.spend.lock().unwrap().insert(
                record.sandbox_id.clone(),
                Spend { machine: record.id.clone(), day, today: 0.0, awake_secs_today: 0, month: 0.0 },
            );
            let machine = self.runtime(record);
            // Before pairing: T3 Code's onboarding needs the descriptor and session right away.
            // On failure the machine's loop primes it again.
            if let Err(e) = machine.prime().await {
                tracing::warn!("priming {}: {e:#}", machine.id());
            }
            self.spawn(machine.clone());
            Ok::<_, anyhow::Error>(machine)
        }
        .await;
        // A failed create's VM becomes a leftover: deleted now, or retried at the next launch.
        let leftover = {
            let mut creating = self.creating.lock().unwrap();
            let sandbox = creating.iter().position(|c| c.name == name).and_then(|i| creating.remove(i).sandbox);
            sandbox.filter(|_| result.is_err())
        };
        self.leftover_sandboxes.lock().unwrap().extend(leftover.clone());
        self.shared.persist();
        self.shared.changed();
        if let Some(id) = leftover {
            self.delete_leftover(&id).await;
        }
        let machine = result?;
        let (app, m) = (self.clone(), machine.clone());
        tokio::spawn(async move {
            for id in projects {
                if let Err(e) = app.add_project(&m, &id).await {
                    app.shared.notify(format!("A project didn't reach {}", m.record().name), format!("{e:#}"));
                }
            }
        });
        // The machine exists either way; its row still offers "Add to T3 Code". The dev driver
        // can leave T3 Code alone (T3AGENT37_NO_AUTOPAIR) and pair by hand.
        if std::env::var_os("T3AGENT37_NO_AUTOPAIR").is_some() {
            return Ok(machine);
        }
        if let Err(e) = self.pair(&machine) {
            self.shared.notify(format!("Couldn't open T3 Code for {name}"), format!("{e:#}; use Add to T3 Code"));
        }
        Ok(machine)
    }

    /// The account's t3agent37 machines this computer doesn't have, as last listed.
    pub fn elsewhere(&self) -> Vec<ElsewhereMachine> {
        self.elsewhere.lock().unwrap().clone()
    }

    /// Lists the account's t3agent37 machines that this computer doesn't have, from Agent37
    /// (which never wakes them). Other VMs on the account aren't t3agent37's and stay out.
    pub async fn refresh_elsewhere(&self) {
        let instances = match self.shared.a37.list().await {
            Ok(list) => list,
            Err(e) => return tracing::debug!("listing the account's machines: {e:#}"),
        };
        let mut ours: HashSet<String> = self.machines().iter().map(|m| m.sandbox_id()).collect();
        ours.extend(self.leftover_sandboxes.lock().unwrap().iter().cloned());
        ours.extend(self.creating.lock().unwrap().iter().filter_map(|c| c.sandbox.clone()));
        let found: Vec<ElsewhereMachine> = instances
            .iter()
            .filter(|i| !ours.contains(&i.id))
            .filter_map(|i| {
                Some(ElsewhereMachine {
                    id: i.id.clone(),
                    name: crate::machine::shared::machine_name(i)?,
                    size: crate::machine::shared::size_of(i).map(str::to_owned),
                    asleep: i.status != "running",
                })
            })
            .collect();
        let mut elsewhere = self.elsewhere.lock().unwrap();
        if *elsewhere != found {
            *elsewhere = found;
            self.shared.changed();
        }
    }

    /// Adds a machine made on another computer (`elsewhere`, by id or name) to this one, which
    /// gets a T3 sign-in of its own there: the other computer keeps using it as before. Wakes it.
    pub async fn add_machine_here(self: &Arc<Self>, machine: &str) -> Result<Arc<MachineRuntime>> {
        use crate::machine::shared;
        let found = self
            .elsewhere()
            .into_iter()
            .find(|m| m.id == machine || m.name.eq_ignore_ascii_case(machine))
            .ok_or_else(|| anyhow!("your Agent37 account has no t3agent37 machine {machine} that this computer doesn't have"))?;
        let name = found.name.clone();
        let port = self.reserve(&name)?;
        self.set_creating_step(&name, "Adding from your account");
        let result = async {
            let a37 = &self.shared.a37;
            let gone = || anyhow!("{name} no longer exists on Agent37");
            if a37.get(&found.id).await?.ok_or_else(gone)?.status != "running" {
                self.set_creating_step(&name, "Waking it");
                a37.start(&found.id).await?;
            }
            self.set_creating_step(&name, "Signing this computer in");
            let adopted = shared::adopt(a37, &found.id).await?;
            let heartbeat = provision::heartbeat_url(a37, &found.id).await?;
            let instance = a37.get(&found.id).await?.ok_or_else(gone)?;
            let record = shared::record(&instance, &name, adopted, heartbeat, port);
            let (day, _) = periods();
            self.spend
                .lock()
                .unwrap()
                .entry(record.sandbox_id.clone())
                .or_insert_with(|| Spend { machine: record.id.clone(), day, today: 0.0, awake_secs_today: 0, month: 0.0 });
            let machine = self.runtime(record);
            // What it does for the other computer, shown here too.
            match machine.read_settings().await {
                Ok(settings) => machine.use_settings(settings).await,
                Err(e) => tracing::warn!("reading {name}'s settings: {e:#}"),
            }
            if let Err(e) = machine.prime().await {
                tracing::warn!("priming {}: {e:#}", machine.id());
            }
            self.spawn(machine.clone());
            Ok::<_, anyhow::Error>(machine)
        }
        .await;
        // Never a leftover to delete: the machine is the other computer's too.
        self.creating.lock().unwrap().retain(|c| c.name != name);
        if result.is_ok() {
            self.elsewhere.lock().unwrap().retain(|m| m.id != found.id);
        }
        self.shared.persist();
        self.shared.changed();
        let machine = result?;
        if std::env::var_os("T3AGENT37_NO_AUTOPAIR").is_some() {
            return Ok(machine);
        }
        if let Err(e) = self.pair(&machine) {
            self.shared.notify(format!("Couldn't open T3 Code for {name}"), format!("{e:#}; use Add to T3 Code"));
        }
        Ok(machine)
    }

    /// This computer's T3 projects that can go to a machine: those with a git remote.
    pub async fn local_projects(&self) -> Result<Vec<crate::projects::Project>> {
        self.shared.t3.projects().await
    }

    /// Adds this computer's T3 project `project` (its id, path or title) to the machine as T3
    /// Code's "Add to another machine" does: cloned from its remote, with its env files. Resolves
    /// once cloned.
    pub async fn add_project(&self, machine: &MachineRuntime, project: &str) -> Result<crate::projects::Project> {
        let mut project = self
            .local_projects()
            .await?
            .into_iter()
            .find(|p| p.id == project || p.path == project || p.title.eq_ignore_ascii_case(project))
            .ok_or_else(|| anyhow!("no project {project} with a git remote"))?;
        project.scripts = self.shared.t3.project_scripts(&project).await;
        let (files, mut failed) = self.shared.t3.files_to_copy(&project).await?;
        failed.extend(machine.add_project(&project, files).await?);
        if !failed.is_empty() {
            self.shared.notify(
                format!("Some of {}'s files didn't reach {}", project.title, machine.record().name),
                failed.join("\n"),
            );
        }
        Ok(project)
    }

    /// Opens T3 Code's own "add backend" confirmation for this machine.
    pub fn pair(&self, machine: &MachineRuntime) -> Result<()> {
        let code = machine.auth.new_pairing_code();
        let pairing_url = format!("http://localhost:{}/pair#token={code}", machine.record().local_port);
        open_url(&format!("t3code://pair?pairingUrl={}", urlencoding::encode(&pairing_url)))
    }

    /// Refuses while work runs on the machine, unless `discard`. T3 Code offers no way to remove
    /// a backend from outside, so a paired machine ends with T3 Code in front and a note on what
    /// to remove there.
    pub async fn delete_machine(&self, machine: &MachineRuntime, discard: bool) -> Result<()> {
        if !discard {
            machine.check_idle_before_delete().await?;
        }
        machine.remove_issue_hooks().await;
        let a37 = &self.shared.a37;
        let record = {
            // Held until retired: a later wake then sees Missing.
            let _waking = machine.waking.lock().await;
            let record = machine.record();
            if let Err(e) = a37.delete(&record.sandbox_id).await {
                // Already gone on Agent37 is fine; anything else keeps the machine.
                if !a37.confirmed_gone(&record.sandbox_id).await {
                    return Err(e);
                }
            }
            machine.retire();
            record
        };
        let id = record.id.as_str();
        let running = {
            let mut machines = self.machines.write().unwrap();
            let idx = machines.iter().position(|r| r.machine.id() == id);
            idx.map(|i| machines.remove(i))
        };
        if let Some(r) = running {
            r.tasks.iter().for_each(JoinHandle::abort);
        }
        let _ = std::fs::remove_dir_all(machine.dir());
        self.retired_ports.lock().unwrap().push(record.local_port);
        if let Err(e) = self.shared.t3.unlink(&record.environment_id).await {
            tracing::warn!("unlinking {id} from T3 Code: {e:#}");
        }
        self.shared.persist();
        self.shared.changed();
        if !record.client_tokens.is_empty() {
            self.shared.notify(
                format!("Remove {} from T3 Code", record.name),
                "T3 Code still lists it. Remove it there under Settings → Environments.",
            );
            let _ = self.shared.t3.activate();
        }
        Ok(())
    }

    pub fn views(&self) -> Vec<MachineView> {
        let mut views: Vec<MachineView> = self.machines().iter().map(|m| view(m)).collect();
        for c in self.creating.lock().unwrap().iter() {
            views.push(MachineView {
                id: String::new(),
                name: c.name.clone(),
                size: String::new(),
                state: MachineState::Creating,
                detail: c.step.clone(),
                countdown: None,
                paired: false,
                signed_out: Vec::new(),
                signed_in: Vec::new(),
                projects: Vec::new(),
                keep_awake_until: None,
                keep_awake_forever: false,
                github_code: None,
                signing_in: None,
                activity: None,
            });
        }
        views
    }

    /// Totals count every instance spent on this month, those of deleted machines included.
    pub fn costs(&self) -> Costs {
        let (day, month) = periods();
        let mut costs = Costs { balance: *self.balance.lock().unwrap(), ..Costs::default() };
        for spend in self.spend.lock().unwrap().values() {
            let u = usage(spend, day, month);
            costs.today += u.today;
            costs.month += u.month;
        }
        costs
    }

    fn usage(&self, machine: &MachineRuntime) -> MachineUsage {
        let (day, month) = periods();
        let id = machine.id();
        let mut total = MachineUsage::default();
        for spend in self.spend.lock().unwrap().values().filter(|s| s.machine == id) {
            let u = usage(spend, day, month);
            total.today += u.today;
            total.month += u.month;
            total.awake_secs_today += u.awake_secs_today;
        }
        total
    }

    /// Looks up this month's spend of every machine in one call: Agent37 reports the whole
    /// workspace by instance and keeps deleted ones listed. A failed lookup keeps the last known
    /// figures, never a fake zero.
    pub async fn refresh_costs(&self) {
        use chrono::Datelike;
        let (day, month) = periods();
        let a37 = &self.shared.a37;
        let today = chrono::Utc::now().date_naive();
        let first = today.with_day(1).unwrap_or(today);
        if let Ok((on_day, in_month)) = tokio::try_join!(a37.usage(today, today), a37.usage(first, today)) {
            let machines: HashMap<String, String> =
                self.machines().iter().map(|m| (m.sandbox_id(), m.id())).collect();
            let mut spend = self.spend.lock().unwrap();
            for (instance, used) in &in_month {
                // Only this app's machines, live or deleted: the workspace may run other VMs.
                let Some(machine) = machines.get(instance).cloned().or_else(|| spend.get(instance).map(|s| s.machine.clone()))
                else {
                    continue;
                };
                let today = on_day.get(instance).cloned().unwrap_or_default();
                spend.insert(
                    instance.clone(),
                    Spend { machine, day, today: today.dollars, awake_secs_today: today.awake_minutes * 60, month: used.dollars },
                );
            }
            spend.retain(|_, s| s.day >= month);
        }
        if let Ok(b) = a37.billing().await {
            *self.balance.lock().unwrap() = Some(b.balance_usd);
            self.check_wallet(b.balance_usd);
        }
        self.refresh_elsewhere().await;
        self.shared.persist();
        self.shared.changed();
    }

    /// Warns once when the wallet drops under the line set in Settings, and again only after it
    /// was topped up above it.
    fn check_wallet(&self, balance: f64) {
        let Some(line) = self.settings().wallet_alert_usd else { return };
        let mut warned = self.wallet_warned.lock().unwrap();
        if balance >= line {
            *warned = false;
        } else if !*warned {
            *warned = true;
            self.shared.notify(
                format!("Agent37 wallet: ${balance:.2} left"),
                "Machines stop waking once it runs out. Top it up in the Agent37 dashboard.",
            );
        }
    }
}

/// "at 14:30" today, "Tue 14:30" on another day.
fn local_time(t: chrono::DateTime<chrono::Utc>) -> String {
    let t = t.with_timezone(&chrono::Local);
    if t.date_naive() == chrono::Local::now().date_naive() {
        t.format("at %H:%M").to_string()
    } else {
        t.format("%a %H:%M").to_string()
    }
}

/// Starts of the current day and month, unix seconds. In UTC: Agent37 reports spend by UTC day.
fn periods() -> (i64, i64) {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc::now();
    let start = |day| {
        Utc
            .with_ymd_and_hms(now.year(), now.month(), day, 0, 0, 0)
            .single()
            .map(|t| t.timestamp())
            .unwrap_or_default()
    };
    (start(now.day()), start(1))
}

/// What `spend` counts toward the periods starting at `day` and `month`: nothing for a period
/// that began after its lookup.
fn usage(spend: &Spend, day: i64, month: i64) -> MachineUsage {
    let today = spend.day >= day;
    MachineUsage {
        today: if today { spend.today } else { 0.0 },
        month: if spend.day >= month { spend.month } else { 0.0 },
        awake_secs_today: if today { spend.awake_secs_today } else { 0 },
    }
}

/// What the details view (and agents) show about one machine, beyond its MachineView.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineDetails {
    pub id: String,
    pub cost_today: f64,
    pub cost_month: f64,
    pub awake_secs_today: u64,
    pub agents: Vec<AgentLogin>,
    pub t3_version: String,
    pub created_at: i64,
    /// T3 Connect is on: other devices reach it with the user's T3 account.
    pub connect: bool,
    /// T3 Connect's new devices wait for approval; None until read on the machine.
    pub connect_approval: Option<bool>,
    /// Agent activity goes to the T3 Code phone app; None until read on the machine.
    pub connect_push: Option<bool>,
    /// Servers T3 saw listening there when it last ran.
    pub servers: Vec<crate::machine::preview::LocalServer>,
    /// Its ports forwarded to this computer.
    pub previews: Vec<crate::store::Preview>,
    /// What its awake time cost per project, this UTC month and today, most first.
    pub project_costs: Vec<ProjectCost>,
    /// T3's webhook tasks on it, as last read there, with addresses that wake it.
    pub webhooks: Vec<Webhook>,
    /// Its start URL, while it's on.
    pub start: Option<StartInfo>,
    /// What it's doing: threads at work and what each is up to, what finished lately, what woke
    /// it, its load and busiest processes. Asleep, as it was before it went to sleep.
    pub activity: Option<ActivityView>,
}

/// A machine's start URL (`machine/start.rs`), which starts threads in its projects from anywhere.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartInfo {
    /// None until the machine has an inbox URL again, which it gets the next time it's up.
    pub url: Option<String>,
    /// The same without its token (`<inbox URL>/start`), which then goes in an
    /// `Authorization: Bearer` header; None as `url` is.
    pub address: Option<String>,
    /// The token alone, for that header.
    pub token: String,
    /// The model for a project with no default of its own: "Opus 5.5, high".
    pub default_model: Option<String>,
}

/// One of T3's webhook tasks on a machine.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Webhook {
    pub task_id: String,
    pub title: String,
    /// Its project's title, when the project is still there.
    pub project: Option<String>,
    pub enabled: bool,
    /// Deliveries must also be signed with its secret.
    pub signed: bool,
    /// Where to send its deliveries so they wake the machine. None until the machine has an
    /// inbox URL, which it gets the next time it's up.
    pub waking_url: Option<String>,
}

/// A project's share of a machine's awake time and what it cost at the machine's hourly rate.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCost {
    /// None for awake time no project's work explains: waking, setup, a wake for nothing.
    pub project_id: Option<String>,
    pub title: String,
    pub secs_today: u64,
    pub secs_month: u64,
    pub dollars_today: f64,
    pub dollars_month: f64,
}

/// `usage` priced at `hourly` dollars, for `today` (UTC, `YYYY-MM-DD`): one entry per project
/// with time this month, named from `projects` ("Removed project" when it's gone), most first,
/// then the time no project explains.
pub fn project_costs(usage: Option<&crate::store::ProjectUsage>, projects: &[crate::machine::projects::MachineProject], hourly: f64, today: &str) -> Vec<ProjectCost> {
    let Some(u) = usage else { return Vec::new() };
    // Counted on an earlier day: this month's and today's figures may not be this month's.
    let same_month = u.day.get(..7) == today.get(..7);
    let same_day = u.day == today;
    let dollars = |secs: u64| secs as f64 * hourly / 3600.0;
    let mut out: Vec<ProjectCost> = u
        .month
        .iter()
        .filter(|_| same_month)
        .filter(|(_, secs)| **secs > 0)
        .map(|(id, &secs_month)| {
            let secs_today = if same_day { u.today.get(id).copied().unwrap_or(0) } else { 0 };
            let (project_id, title) = match id.as_str() {
                "" => (None, "Other".to_owned()),
                id => (Some(id.to_owned()), projects.iter().find(|p| p.id == id).map_or("Removed project".to_owned(), |p| p.title.clone())),
            };
            ProjectCost { project_id, title, secs_today, secs_month, dollars_today: dollars(secs_today), dollars_month: dollars(secs_month) }
        })
        .collect();
    out.sort_by(|a, b| a.project_id.is_none().cmp(&b.project_id.is_none()).then(b.secs_month.cmp(&a.secs_month)));
    out
}

/// Its gh login shows as one more agent: what its git pushes and clones private repos with.
pub const GITHUB: &str = "GitHub";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogin {
    pub name: String,
    pub signed_in: bool,
    /// Who it's signed in as, as last seen: email, plan and usage limits (Claude, Codex).
    #[serde(flatten)]
    pub account: crate::machine::signin::Account,
    /// GitHub's account, `@login`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// GitHub: the repositories its token can push to, when it's a fine-grained one, and that
    /// in words ("only skildingur, tantalum", "all 85 of octocat's repos").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repos: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<String>,
    /// GitHub: when that token expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
}

impl App {
    /// Never wakes the machine.
    pub fn details(&self, machine: &str) -> Result<MachineDetails> {
        let machine = self.machine(machine)?;
        let id = machine.id();
        let record = machine.record();
        let usage = self.usage(&machine);
        let providers = machine.cache.lock().unwrap().providers();
        let activity = activity_view(&machine);
        Ok(MachineDetails {
            id,
            cost_today: usage.today,
            cost_month: usage.month,
            awake_secs_today: usage.awake_secs_today,
            // Only agents whose status is known, so this agrees with `signed_out` in the list.
            agents: providers
                .iter()
                .filter_map(|p| {
                    let name = p["displayName"].as_str()?.to_owned();
                    let signed_in = match p["auth"]["status"].as_str()? {
                        "authenticated" => true,
                        "unauthenticated" => false,
                        _ => return None,
                    };
                    let account = if signed_in { crate::machine::signin::account(p) } else { Default::default() };
                    Some(AgentLogin { name, signed_in, account, login: None, repos: None, reach: None, expires: None })
                })
                .chain(record.github_checked.then(|| AgentLogin {
                    name: GITHUB.into(),
                    signed_in: record.github_login.is_some(),
                    account: Default::default(),
                    login: record.github_login.clone(),
                    repos: record.github_reach.as_ref().map(|r| r.repos.clone()),
                    reach: record.github_reach.as_ref().map(crate::store::GithubReach::describe),
                    expires: record.github_token_expires.clone(),
                }))
                .collect(),
            t3_version: record.t3_version,
            created_at: record.created_at,
            connect: record.connect,
            connect_approval: record.connect_approval,
            connect_push: record.connect_push,
            servers: machine.local_servers(),
            project_costs: project_costs(
                record.usage.as_ref(),
                &machine.projects(),
                crate::machine::provision::size(&record.size).map_or(0.0, |s| s.hourly),
                &chrono::Utc::now().format("%Y-%m-%d").to_string(),
            ),
            webhooks: {
                let projects = machine.projects();
                record
                    .hooks
                    .iter()
                    .map(|h| Webhook {
                        task_id: h.id.clone(),
                        title: h.title.clone(),
                        project: projects.iter().find(|p| p.id == h.project_id).map(|p| p.title.clone()),
                        enabled: h.enabled,
                        signed: h.signed,
                        waking_url: record.inbox_url.as_deref().map(|u| crate::machine::hooks::waking_url(u, h)),
                    })
                    .collect()
            },
            start: record.start.as_ref().map(|s| StartInfo {
                url: machine.start_url(),
                address: machine.start_parts().map(|(address, _)| address),
                token: s.token.clone(),
                default_model: machine.start_agent(),
            }),
            previews: record.previews,
            activity,
        })
    }

    /// Forwards `port` on the machine to this computer and returns its address here,
    /// `http://localhost:<port>`. `open`: in the default browser too, which wakes the machine.
    pub async fn open_port(&self, machine: &Arc<MachineRuntime>, port: u16, open: bool) -> Result<String> {
        let local = machine.forward(port)?;
        let url = format!("http://localhost:{local}");
        if open {
            open_url(&url)?;
        }
        Ok(url)
    }

    /// Warns about a machine that has been awake for `awake_alert_hours` in a row, once per
    /// stretch, with what holds it up.
    pub fn check_awake_alerts(&self) {
        let Some(hours) = self.settings().awake_alert_hours else { return };
        for m in self.machines() {
            let Some(since) = m.awake_since() else { continue };
            let up = since.elapsed().unwrap_or_default();
            if up.as_secs() < u64::from(hours) * 3600 || std::mem::replace(&mut *m.awake_warned.lock().unwrap(), true) {
                continue;
            }
            let record = m.record();
            let why = match m.watchdog_status() {
                Some(w) if w.working_threads + w.working_subagents > 0 => "an agent is still working",
                Some(w) if w.keep_awake_forever => "auto sleep is off",
                Some(w) if w.keep_awake_secs > 0 => "it's kept awake",
                Some(w) if w.cpu_busy => "something is busy on its CPU (a build or a job)",
                Some(w) if w.agents_unknown => "T3 isn't answering there, so it can't tell if agents work",
                _ => "it's in use",
            };
            let spent = provision::size(&record.size).map(|s| format!(", about ${:.2} so far", s.hourly * up.as_secs_f64() / 3600.0)).unwrap_or_default();
            self.shared.notify(
                format!("{} has been awake {}h", record.name, up.as_secs() / 3600),
                format!("It's up because {why}{spent}. Sleep it from t3agent37 if that's not on purpose."),
            );
        }
    }

    /// Brings T3 Code forward on `thread`, or just to the front; an unpaired machine gets the
    /// "add backend" dialog instead.
    pub fn open_in_t3(&self, machine: &MachineRuntime, thread: Option<&str>) -> Result<()> {
        let record = machine.record();
        if record.client_tokens.is_empty() {
            return self.pair(machine);
        }
        match thread {
            Some(thread) => self.shared.t3.open_thread(&record.environment_id, thread),
            None => self.shared.t3.activate(),
        }
    }
}

fn view(m: &MachineRuntime) -> MachineView {
    let r = m.record();
    let watchdog = m.watchdog_status();
    // Before the struct below: a lock taken in one of its fields is held until it's built, and
    // this takes the same ones.
    let activity = activity_view(m);
    let mut countdown = None;
    let mut count_down = |template, secs, from| {
        let c = Countdown::new(template, secs, from);
        let text = c.text(secs);
        countdown = Some(c);
        text
    };
    use MachineState::*;
    let (state, detail) = match (m.power(), &watchdog) {
        (Power::Missing, _) => (Missing, "Deleted on Agent37".to_owned()),
        (_, _) if m.last_error().is_some() => (Error, m.last_error().unwrap_or_default()),
        (Power::Running, Some(w)) if w.agents_unknown => (Working, "T3 not answering · stays up 30m max".to_owned()),
        (Power::Running, Some(w)) if w.cpu_busy => (Working, busy_cpu_line(w)),
        (Power::Running, Some(w)) if w.busy => (Working, working_line(w)),
        (Power::Running, Some(w)) if w.phase == WatchdogPhase::Active && w.keep_awake_forever => {
            (Idle, "Awake until you stop it".to_owned())
        }
        (Power::Running, Some(w)) if w.phase == WatchdogPhase::Active && w.keep_awake_secs > 0 => {
            (Idle, count_down("Kept awake · {left} left", w.keep_awake_secs, w.received))
        }
        (Power::Running, Some(w)) if w.phase == WatchdogPhase::Active && w.work_due => {
            let due = w.next_due_at.and_then(chrono::DateTime::from_timestamp_millis);
            (Idle, due.map_or("Idle · scheduled work due".to_owned(), |t| format!("Idle · scheduled work {}", local_time(t))))
        }
        (Power::Running, Some(w)) if w.phase == WatchdogPhase::Active => {
            let left = w.sleep_after_secs.saturating_sub(w.idle_secs);
            (Idle, count_down("Idle · sleeps in {left}", left, w.received))
        }
        (Power::Running, Some(_)) | (Power::Stopping, _) => (Sleeping, "Going to sleep".to_owned()),
        (Power::Running, None) => (Idle, "Running".to_owned()),
        (Power::Starting, _) => match m.upgrading() {
            Some(version) => (Waking, format!("Updating T3 to {version}")),
            None => (Waking, "Waking up".to_owned()),
        },
        (Power::Stopped, _) => match m.scheduled_wake() {
            Some(due) => (Asleep, format!("Asleep · wakes {} for scheduled work", local_time(due))),
            None => (Asleep, "Asleep · bills disk only".to_owned()),
        },
        (Power::Unknown, _) => (Unknown, "Checking".to_owned()),
    };
    let running_watchdog = watchdog.as_ref().filter(|_| m.power() == Power::Running);
    MachineView {
        id: r.id.clone(),
        name: r.name.clone(),
        size: r.size.clone(),
        state,
        detail,
        countdown,
        paired: !r.client_tokens.is_empty(),
        signed_out: {
            let mut out = m.cache.lock().unwrap().signed_out_providers();
            if r.github_checked && r.github_login.is_none() {
                out.push(GITHUB.into());
            }
            out
        },
        signed_in: m.cache.lock().unwrap().signed_in_providers(),
        projects: m.projects(),
        keep_awake_until: running_watchdog
            .filter(|w| w.keep_awake_secs > 0)
            .map(|w| unix_ms_after(w.received, w.keep_awake_secs)),
        keep_awake_forever: running_watchdog.is_some_and(|w| w.keep_awake_forever),
        github_code: m.github_code(),
        signing_in: m.signing_in(),
        activity,
    }
}

/// What the machine is doing, from its watchdog's last report: live while it runs.
fn activity_view(m: &MachineRuntime) -> Option<ActivityView> {
    let (activity, at) = m.activity()?;
    Some(activity.view(at, m.power() == Power::Running, &m.projects(), &m.issue_models()))
}

/// The UI's rule: `^[a-z0-9][a-z0-9-]{0,31}$`.
fn valid_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    name.len() <= 32 && name.starts_with(ok) && name.chars().all(|c| ok(c) || c == '-')
}

/// "Working · 2 threads · 3 subagents".
/// A thread waiting for the user says so: "Needs you · Fix login: needs your approval". One
/// thread at work is named: "Working · Fix login · 2 subagents".
fn working_line(w: &WatchdogStatus) -> String {
    let live = w.activity.as_ref().map(|a| a.working.as_slice()).unwrap_or_default();
    if let Some(t) = live.iter().find(|t| needs_you(&t.state)) {
        let title = if t.title.is_empty() { "a thread" } else { &t.title };
        return format!("Needs you · {title}: {}", t.state);
    }
    let count = |n: u32, one: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {one}s")),
    };
    let threads = match live {
        [t] if w.working_threads == 1 && !t.title.is_empty() => Some(t.title.clone()),
        _ => count(w.working_threads, "thread"),
    };
    std::iter::once("Working".to_owned())
        .chain(threads)
        .chain(count(w.working_subagents, "subagent"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "Working · busy CPU · next build": what's using it, when the watchdog says.
fn busy_cpu_line(w: &WatchdogStatus) -> String {
    match w.activity.as_ref().and_then(|a| a.top.first()) {
        Some(p) => format!("Working · busy CPU · {}", p.name),
        None => "Working · busy CPU".to_owned(),
    }
}

/// "42s", "4m", "1h 5m"; minutes round up. The UI's `timeLeft` matches it.
fn human_secs(s: u64) -> String {
    let m = s.div_ceil(60);
    match (s, m) {
        (0..60, _) => format!("{s}s"),
        (_, 0..60) => format!("{m}m"),
        _ => format!("{}h {}m", m / 60, m % 60),
    }
}

/// On Linux through GLib, as `gio open` does: `xdg-open` outside GNOME and KDE takes a scheme
/// with a digit in it, like `t3code:`, for a file name.
#[cfg(target_os = "linux")]
pub fn open_url(url: &str) -> Result<()> {
    use gtk::gio;
    gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>)
        .map_err(|e| anyhow::anyhow!("could not open {url}: {e}"))
}

#[cfg(not(target_os = "linux"))]
pub fn open_url(url: &str) -> Result<()> {
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).status()
    } else {
        // No shell: `cmd /C start` would run anything after a `&` in the URL.
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).status()
    }?;
    if !status.success() {
        bail!("could not open {url}");
    }
    Ok(())
}

/// Opens `url` in a private window of the default browser, where nobody is signed in, so a
/// sign-in there can be any account. Chrome, Edge, Brave, Vivaldi, Opera and Firefox take a flag
/// for it; Safari and others can't be asked, so they get `url` as usual. True: private.
pub fn open_url_private(url: &str) -> Result<bool> {
    #[cfg(target_os = "macos")]
    if let Some((bundle, flag)) = default_browser().and_then(|b| private_flag(&b).map(|f| (b, f))) {
        // `-n` so the flag reaches a browser that's already running.
        let opened = std::process::Command::new("open")
            .args(["-n", "-b", &bundle, "--args", flag, url])
            .status()
            .is_ok_and(|s| s.success());
        if opened {
            return Ok(true);
        }
    }
    open_url(url)?;
    Ok(false)
}

/// Whether `open_url_private` gets a private window from the default browser. Looked up once.
pub fn private_window_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    #[cfg(target_os = "macos")]
    return *AVAILABLE.get_or_init(|| default_browser().is_some_and(|b| private_flag(&b).is_some()));
    #[cfg(not(target_os = "macos"))]
    return *AVAILABLE.get_or_init(|| false);
}

/// The bundle id of the app that opens https links, from Launch Services' settings. None for
/// the system default, Safari.
#[cfg(target_os = "macos")]
fn default_browser() -> Option<String> {
    let plist = dirs::home_dir()?
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let out = std::process::Command::new("plutil")
        .args(["-extract", "LSHandlers", "json", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    let handlers: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    handlers
        .as_array()?
        .iter()
        .find(|h| h["LSHandlerURLScheme"] == "https")?["LSHandlerRoleAll"]
        .as_str()
        .map(str::to_owned)
}

/// The command-line flag that opens a private window in this browser, if it has one.
fn private_flag(bundle: &str) -> Option<&'static str> {
    let b = bundle.to_ascii_lowercase();
    let chromium = ["com.google.chrome", "org.chromium.", "com.brave.browser", "com.vivaldi.vivaldi"];
    if chromium.iter().any(|p| b.starts_with(p)) {
        Some("--incognito")
    } else if b.starts_with("com.microsoft.edgemac") {
        Some("--inprivate")
    } else if b.starts_with("com.operasoftware.opera") {
        Some("--private")
    } else if b.starts_with("org.mozilla.") {
        Some("--private-window")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn private_window_flags() {
        assert_eq!(super::private_flag("com.google.Chrome"), Some("--incognito"));
        assert_eq!(super::private_flag("com.google.chrome.canary"), Some("--incognito"));
        assert_eq!(super::private_flag("com.microsoft.edgemac"), Some("--inprivate"));
        assert_eq!(super::private_flag("org.mozilla.firefox"), Some("--private-window"));
        assert_eq!(super::private_flag("com.apple.Safari"), None);
        assert_eq!(super::private_flag("company.thebrowser.Browser"), None);
    }

    #[test]
    fn project_costs() {
        use crate::machine::projects::MachineProject;
        use crate::store::ProjectUsage;
        let project = |id: &str, title: &str| MachineProject { id: id.into(), title: title.into(), repo_key: None, root: None, status: None, issues: None };
        let projects = [project("a", "Alpha"), project("b", "Beta")];
        let usage = ProjectUsage {
            day: "2026-11-22".into(),
            today: [("a".into(), 1800), ("".into(), 60)].into(),
            month: [("a".into(), 7200), ("b".into(), 10800), ("".into(), 600), ("gone".into(), 360), ("idle".into(), 0)].into(),
        };
        let costs = super::project_costs(Some(&usage), &projects, 0.05, "2026-11-22");
        let titles: Vec<_> = costs.iter().map(|c| c.title.as_str()).collect();
        // Most first, the time no project explains last, nothing for none.
        assert_eq!(titles, ["Beta", "Alpha", "Removed project", "Other"]);
        assert_eq!((costs[1].secs_today, costs[1].secs_month), (1800, 7200));
        assert!((costs[1].dollars_month - 0.10).abs() < 1e-9);
        assert!((costs[1].dollars_today - 0.025).abs() < 1e-9);
        assert_eq!(costs[3].project_id, None);
        // Counted yesterday: nothing today. Last month: nothing this month.
        let costs = super::project_costs(Some(&usage), &projects, 0.05, "2026-11-23");
        assert!(costs.iter().all(|c| c.secs_today == 0) && costs[0].secs_month == 10800);
        assert!(super::project_costs(Some(&usage), &projects, 0.05, "2026-12-01").is_empty());
        assert!(super::project_costs(None, &projects, 0.05, "2026-11-22").is_empty());
    }

    #[test]
    fn valid_name() {
        for ok in ["dev", "0", "a-b-", &"a".repeat(32)] {
            assert!(super::valid_name(ok), "{ok}");
        }
        for bad in ["", "-dev", "Dev", "my dev", "dév", &"a".repeat(33)] {
            assert!(!super::valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn spend_counts_only_in_its_periods() {
        let spend =
            crate::store::Spend { machine: "m".into(), day: 100, today: 1.0, awake_secs_today: 60, month: 5.0 };
        let same_day = super::usage(&spend, 100, 0);
        assert_eq!((same_day.today, same_day.awake_secs_today, same_day.month), (1.0, 60, 5.0));
        let later_day = super::usage(&spend, 200, 0);
        assert_eq!((later_day.today, later_day.awake_secs_today, later_day.month), (0.0, 0, 5.0));
        let later_month = super::usage(&spend, 300, 300);
        assert_eq!((later_month.today, later_month.month), (0.0, 0.0));
    }
}
