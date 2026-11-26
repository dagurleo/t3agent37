//! The app model of `t3agent37_lib` on a tokio runtime of its own, and the snapshot of it the
//! panel draws: startup, the backend's events, and the commands the panel runs.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use t3agent37_lib::agent37::{Agent37, KEYS_PAGE};
pub use t3agent37_lib::app::{AgentLogin, ElsewhereMachine, ProjectCost, StartInfo, Webhook};
use t3agent37_lib::app::{self, App, Costs, EnvAction, GITHUB, MachineView, Setup};
use t3agent37_lib::machine::peers::Access;
use t3agent37_lib::machine::Event;
use t3agent37_lib::machine::issues::{IssueSettings, ModelChoice};
pub use t3agent37_lib::machine::preview::LocalServer;
use t3agent37_lib::machine::provision::{SIZES, SizeInfo};
use t3agent37_lib::machine::signin::Agent;
pub use t3agent37_lib::machine::signin::Secret;
use t3agent37_lib::mcp::{self, Client};
use t3agent37_lib::projects::Project;
pub use t3agent37_lib::store::Preview;
use t3agent37_lib::store::Settings;

const COSTS_EVERY: Duration = Duration::from_secs(300);
/// While a machine's details stay open, how often they look again at what the machine doesn't
/// push to this app (the servers listening there, its agents, settings and webhook tasks). The
/// details load again on every change, every few seconds: each look on each would pile up
/// commands on the machine (the settings are a command over Agent37's exec) and keep it busy.
const DETAILS_REFRESH_EVERY: Duration = Duration::from_secs(60);
/// Opening the details looks again, unless they were just open.
const DETAILS_REOPEN_GAP: Duration = Duration::from_secs(5);

/// What the backend tells the GPUI side.
pub enum Signal {
    /// Something visible changed: take a new snapshot.
    Changed,
    Notify { title: String, body: String },
    /// Start at login as the saved settings say, applied once the app model is up.
    LoginItem(bool),
}

/// Whether each coding agent CLI has t3agent37's MCP server; `None`: that CLI is not installed.
#[derive(Clone, Default)]
pub struct McpStatus {
    pub claude_code: Option<bool>,
    pub codex: Option<bool>,
}

impl McpStatus {
    pub fn get(&self, client: Client) -> Option<bool> {
        match client {
            Client::ClaudeCode => self.claude_code,
            Client::Codex => self.codex,
        }
    }
}

/// The Agent37 workspace's wallet, as last read.
#[derive(Clone)]
pub struct Account {
    pub balance_usd: f64,
    pub memory_limit_gb: u32,
    pub past_due: bool,
}

/// Everything the panel shows, as of the last change.
#[derive(Clone)]
pub struct UiState {
    pub setup: Setup,
    pub setup_busy: bool,
    pub account: Option<Account>,
    pub machines: Vec<MachineView>,
    /// t3agent37 machines on the account that this computer doesn't have: from another computer.
    pub elsewhere: Vec<ElsewhereMachine>,
    pub costs: Costs,
    pub settings: Settings,
    /// What new machines get signed in with: no secrets, only what's saved.
    pub sign_ins: t3agent37_lib::app::SignInsSaved,
    pub sizes: &'static [SizeInfo],
    pub mcp: McpStatus,
}

/// What the details view shows beyond the machine's row. Never wakes it.
#[derive(Clone)]
pub struct Details {
    pub id: String,
    pub cost_today: f64,
    pub cost_month: f64,
    pub awake_secs_today: u64,
    /// Its agents' sign-ins and accounts, GitHub last.
    pub agents: Vec<AgentLogin>,
    pub t3_version: String,
    pub created_at: i64,
    pub connect: bool,
    pub connect_approval: Option<bool>,
    pub connect_push: Option<bool>,
    /// Servers listening on it, as T3 there last saw them.
    pub servers: Vec<LocalServer>,
    /// Its ports forwarded here.
    pub previews: Vec<Preview>,
    /// The models its signed-in agents offer, to work on GitHub issues with.
    pub issue_models: Vec<ModelChoice>,
    /// What its awake time cost per project this month, most first.
    pub project_costs: Vec<ProjectCost>,
    /// T3's webhook tasks on it, with addresses that wake it.
    pub webhooks: Vec<Webhook>,
    /// Its start URL, while it's on.
    pub start: Option<StartInfo>,
    /// The env files its projects got from this computer: names only.
    pub env: Vec<t3agent37_lib::store::ProjectEnv>,
}

/// A command for one machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Brings T3 Code forward, into the thread if given; an unpaired machine gets the dialog.
    Open { thread: Option<String> },
    Pair,
    Wake,
    Sleep,
    TurnOnConnect,
    /// Copies the wake link; `renew` replaces it.
    WakeLink { renew: bool },
    /// Minutes; `None`: until stopped (auto sleep off); 0 ends it.
    KeepAwake(Option<u32>),
    Resize(String),
    /// `agent`: a name as `signed_out` lists it. `another`: another account than the browser's,
    /// in a private window; over a sign-in, it switches the machine's account.
    SignIn { agent: String, another: bool },
    /// Stops the sign-in that runs on it; the old sign-in stays.
    CancelSignIn,
    /// `discard`: delete even while work runs on it.
    Delete { discard: bool },
    /// T3 Connect: new devices wait for approval.
    ConnectApproval(bool),
    /// T3 Connect: agent activity to the phone app.
    ConnectPush(bool),
    /// Forwards a port of the machine here and opens it in the browser; resolves to its URL.
    OpenPort(u16),
    /// Stops forwarding a port.
    ClosePort(u16),
    /// GitHub issues given a label in one of its projects (by id) become threads there, with
    /// `settings` (on: the project's own, or the defaults, when `None`).
    Issues { project: String, on: bool, settings: Option<IssueSettings> },
    /// The start URL on (resolves to it, for the clipboard) or off; `replace`: a new one.
    Start { on: bool, replace: bool },
    /// Signs its GitHub in with a token the user made (one limited to some repositories);
    /// resolves to who and how many repositories, for the toast.
    GithubToken(Secret),
    /// Lets its agents use another of the user's machines (by id) as far as `access` says, or
    /// not at all (`None`).
    Link { to: String, to_name: String, access: Option<Access> },
    /// Its own monthly spending limit in dollars, or none; resolves to what it spent, for the
    /// toast.
    Limit(Option<f64>),
    /// Deleted this many seconds from now, or kept (`None`).
    DeleteAfter(Option<u64>),
    /// A project's env files: this computer's copied over again, or taken off it; resolves to
    /// what was done.
    Env { project: String, action: EnvAction },
    /// Makes it a base (`Some(true)`, stopping it), a machine to work on again (`Some(false)`),
    /// or refreshes the base (`None`); resolves to what was done.
    Base(Option<bool>),
    /// Brings a spawned copy's work back into its checkout and deletes it; resolves to what came.
    Collect,
}

pub type Res<T = ()> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

struct Inner {
    data_dir: PathBuf,
    app: Mutex<Option<Arc<App>>>,
    setup: Mutex<Setup>,
    busy: Mutex<bool>,
    account: Mutex<Option<Account>>,
    mcp: Mutex<McpStatus>,
    /// One `init` at a time: the startup check and "Check again" must not both build an App.
    init_lock: tokio::sync::Mutex<()>,
    /// When each machine's details last looked again at the machine.
    details_refreshed: Mutex<HashMap<String, Instant>>,
    signals: UnboundedSender<Signal>,
}

impl Inner {
    fn app(&self) -> Res<Arc<App>> {
        self.app.lock().unwrap().clone().ok_or_else(|| "t3agent37 is not set up yet".to_owned())
    }

    fn changed(&self) {
        let _ = self.signals.unbounded_send(Signal::Changed);
    }

    fn notify(&self, title: impl Into<String>, body: impl Into<String>) {
        let _ = self.signals.unbounded_send(Signal::Notify { title: title.into(), body: body.into() });
    }
}

/// The backend: cheap to clone, every clone the same.
#[derive(Clone)]
pub struct Backend {
    rt: Arc<tokio::runtime::Runtime>,
    inner: Arc<Inner>,
}

impl Backend {
    /// Starts the runtime and the setup check; `signals` hears about every change.
    pub fn start(data_dir: PathBuf, signals: UnboundedSender<Signal>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("t3agent37").build().expect("tokio runtime");
        let inner = Arc::new(Inner {
            data_dir,
            app: Mutex::new(None),
            setup: Mutex::new(Setup::Starting),
            busy: Mutex::new(false),
            account: Mutex::new(None),
            mcp: Mutex::new(McpStatus::default()),
            init_lock: tokio::sync::Mutex::new(()),
            details_refreshed: Mutex::new(HashMap::new()),
            signals,
        });
        rt.spawn(init(inner.clone()));
        Self { rt: Arc::new(rt), inner }
    }

    pub fn state(&self) -> UiState {
        let inner = &self.inner;
        let app = inner.app.lock().unwrap().clone();
        UiState {
            setup: *inner.setup.lock().unwrap(),
            setup_busy: *inner.busy.lock().unwrap(),
            account: inner.account.lock().unwrap().clone(),
            machines: app.as_ref().map(|a| a.views()).unwrap_or_default(),
            elsewhere: app.as_ref().map(|a| a.elsewhere()).unwrap_or_default(),
            costs: app.as_ref().map(|a| a.costs()).unwrap_or_default(),
            settings: app.as_ref().map(|a| a.settings()).unwrap_or_default(),
            sign_ins: app.as_ref().map(|a| a.sign_ins_saved()).unwrap_or_default(),
            sizes: SIZES,
            mcp: inner.mcp.lock().unwrap().clone(),
        }
    }

    /// Runs `job` on the runtime; the returned future can be awaited from GPUI.
    fn run<T, F, J>(&self, job: J) -> impl Future<Output = Res<T>> + use<T, F, J>
    where
        T: Send + 'static,
        F: Future<Output = Res<T>> + Send + 'static,
        J: FnOnce(Arc<Inner>) -> F,
    {
        let handle = self.rt.spawn(job(self.inner.clone()));
        async move { handle.await.map_err(|e| format!("{e}"))? }
    }

    /// Resolves to the wake link for `Cmd::WakeLink` and the start URL for `Cmd::Start` (for the
    /// clipboard), else to `None`.
    pub fn act(&self, id: String, cmd: Cmd) -> impl Future<Output = Res<Option<String>>> + use<> {
        self.run(move |inner| act(inner, id, cmd))
    }

    pub fn create_machine(&self, name: String, size: String, projects: Vec<String>, delete_after: Option<u64>) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move { inner.app()?.create_machine(&name, &size, projects, delete_after).await.map(drop).map_err(err) })
    }

    /// A copy of the machine `source` (by id) named `name`, at `size`.
    pub fn copy_machine(&self, source: String, name: String, size: String, delete_after: Option<u64>) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move { inner.app()?.copy_machine(&source, &name, Some(&size), delete_after, true).await.map(drop).map_err(err) })
    }

    /// The monthly limit on all machines together, or none.
    pub fn set_total_limit(&self, usd: Option<f64>) -> impl Future<Output = Res<String>> + use<> {
        self.run(move |inner| async move { inner.app()?.set_limit(None, usd).await.map_err(err) })
    }

    /// Adds a machine made with t3agent37 on another computer to this one; resolves to its name.
    pub fn add_machine_here(&self, id: String) -> impl Future<Output = Res<String>> + use<> {
        self.run(move |inner| async move { inner.app()?.add_machine_here(&id).await.map(|m| m.record().name).map_err(err) })
    }

    /// This computer's T3 projects that can go to a machine.
    pub fn local_projects(&self) -> impl Future<Output = Res<Vec<Project>>> + use<> {
        self.run(|inner| async move { inner.app()?.local_projects().await.map_err(err) })
    }

    pub fn add_project(&self, id: String, project: String) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move {
            let app = inner.app()?;
            let machine = app.machine(&id).map_err(err)?;
            app.add_project(&machine, &project).await.map(drop).map_err(err)
        })
    }

    /// The thread "Open" goes to; never wakes the machine.
    pub fn latest_thread(&self, id: String) -> impl Future<Output = Res<Option<String>>> + use<> {
        self.run(move |inner| async move { Ok(inner.app()?.machine(&id).map_err(err)?.latest_thread().await) })
    }

    /// For the details view; never wakes the machine. `opened`: the view just opened, so it
    /// looks again at the servers listening on the machine, its agents, settings and webhook
    /// tasks, when it runs; while it stays open, once a minute (`DETAILS_REFRESH_EVERY`).
    pub fn details(&self, id: &str, opened: bool) -> Res<Details> {
        let app = self.inner.app()?;
        let d = app.details(id).map_err(err)?;
        let mut issue_models = Vec::new();
        if let Ok(machine) = app.machine(id) {
            issue_models = machine.issue_models();
        }
        let gap = if opened { DETAILS_REOPEN_GAP } else { DETAILS_REFRESH_EVERY };
        let due = {
            let mut last = self.inner.details_refreshed.lock().unwrap();
            let due = last.get(id).is_none_or(|at| at.elapsed() >= gap);
            if due {
                last.insert(id.to_owned(), Instant::now());
            }
            due
        };
        if let Some(machine) = app.machine(id).ok().filter(|_| due) {
            self.rt.spawn(async move {
                machine.refresh_servers().await;
                machine.refresh_agents().await;
                // Another computer may have changed them.
                machine.refresh_settings().await;
                machine.refresh_hooks().await;
            });
        }
        Ok(Details {
            id: d.id,
            cost_today: d.cost_today,
            cost_month: d.cost_month,
            awake_secs_today: d.awake_secs_today,
            agents: d.agents,
            t3_version: d.t3_version,
            created_at: d.created_at,
            connect: d.connect,
            connect_approval: d.connect_approval,
            connect_push: d.connect_push,
            servers: d.servers,
            previews: d.previews,
            issue_models,
            project_costs: d.project_costs,
            webhooks: d.webhooks,
            start: d.start,
            env: d.env,
        })
    }

    /// Saves the settings; the login item is the caller's (it lives on the main thread).
    pub fn update_settings(&self, settings: Settings) -> Res {
        self.inner.app()?.update_settings(settings);
        self.inner.changed();
        Ok(())
    }

    /// "openKeys" | "installT3" | "recheck"
    pub fn setup_action(&self, action: &'static str) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move {
            match action {
                "openKeys" => app::open_url(KEYS_PAGE).map_err(err)?,
                "installT3" => app::open_url("https://t3.codes").map_err(err)?,
                _ => {}
            }
            let app = inner.app.lock().unwrap().clone();
            match app {
                None => init(inner.clone()).await,
                Some(app) => {
                    refresh_account(&inner, &app.shared.a37).await;
                    inner.changed();
                }
            }
            Ok(())
        })
    }

    /// Checks the pasted Agent37 API key with Agent37, keeps it on this computer, and starts.
    pub fn save_api_key(&self, key: String) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move {
            if inner.app.lock().unwrap().is_some() {
                return Err("t3agent37 already has a key; quit and remove it to use another".into());
            }
            let set_busy = |busy| {
                *inner.busy.lock().unwrap() = busy;
                inner.changed();
            };
            set_busy(true);
            let saved = Agent37::save(&inner.data_dir, &key).await;
            set_busy(false);
            saved.map_err(err)?;
            init(inner.clone()).await;
            Ok(())
        })
    }

    /// Lets a coding agent CLI control t3agent37 (registers the MCP server in its config).
    pub fn add_mcp(&self, client: Client) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move {
            let result = mcp::register(client).await.map_err(err);
            refresh_mcp(inner).await;
            result
        })
    }

    /// Looks again which coding agents have the MCP server (the panel opening).
    pub fn refresh_mcp(&self) {
        self.rt.spawn(refresh_mcp(self.inner.clone()));
    }
}

async fn act(inner: Arc<Inner>, id: String, cmd: Cmd) -> Res<Option<String>> {
    let app = inner.app()?;
    let machine = app.machine(&id).map_err(err)?;
    let done = match cmd {
        Cmd::WakeLink { renew } => return machine.wake_link(renew).await.map(Some).map_err(err),
        Cmd::Start { on, replace } => return machine.set_start(on, replace).await.map_err(err),
        Cmd::GithubToken(token) => return machine.sign_in_github_token(&token).await.map(Some).map_err(err),
        Cmd::Link { to, access, .. } => return app.set_link(&id, &to, access).await.map(Some).map_err(err),
        Cmd::Limit(usd) => return app.set_limit(Some(&id), usd).await.map(Some).map_err(err),
        Cmd::DeleteAfter(secs) => return app.set_delete_after(&id, secs).map(Some).map_err(err),
        Cmd::Env { project, action } => return app.project_env(&id, &project, action).await.map(Some).map_err(err),
        Cmd::Base(Some(on)) => return app.set_base(&id, on).await.map(Some).map_err(err),
        Cmd::Base(None) => return app.refresh_base(&id).await.map(Some).map_err(err),
        Cmd::Collect => return app.collect(&id, true, false).await.map(Some).map_err(err),
        Cmd::Open { thread } => app.open_in_t3(&machine, thread.as_deref()),
        Cmd::Pair => app.pair(&machine),
        Cmd::Wake => machine.wake_for("Wake in t3agent37").await,
        Cmd::Sleep => machine.sleep_now().await,
        Cmd::TurnOnConnect => machine.turn_on_connect().await,
        Cmd::KeepAwake(minutes) => machine.keep_awake(minutes).await,
        Cmd::Resize(size) => machine.resize(&size).await,
        Cmd::SignIn { agent, another } => {
            if agent == GITHUB {
                machine.sign_in_github(another).await.map_err(err)?;
            } else {
                let a = Agent::from_name(&agent).ok_or_else(|| format!("{agent} has to be signed in from T3 Code's settings"))?;
                machine.sign_in(a, another).await.map_err(err)?;
            }
            // Who it's signed in as now, for the toast.
            let who = app.details(&id).ok().and_then(|d| {
                let a = d.agents.into_iter().find(|a| a.name == agent)?;
                a.account.email.or(a.login.map(|l| format!("@{l}")))
            });
            return Ok(who);
        }
        Cmd::CancelSignIn => {
            machine.cancel_sign_in();
            Ok(())
        }
        Cmd::Delete { discard } => app.delete_machine(&machine, discard).await,
        Cmd::ConnectApproval(on) => machine.set_connect_approval(on).await,
        Cmd::ConnectPush(on) => machine.set_connect_push(on).await,
        Cmd::OpenPort(port) => return app.open_port(&machine, port, true).await.map(Some).map_err(err),
        Cmd::ClosePort(port) => {
            machine.unforward(port);
            Ok(())
        }
        Cmd::Issues { project, on, settings } => machine.set_issues(&project, on, settings).await,
    };
    done.map(|_| None).map_err(err)
}

/// Checks for the Agent37 key and the T3 Code install; starts the app model once both are there.
async fn init(inner: Arc<Inner>) {
    let _init = inner.init_lock.lock().await;
    if inner.app.lock().unwrap().is_some() {
        return;
    }
    let (setup, a37, t3) = app::check_setup(&inner.data_dir).await;
    *inner.setup.lock().unwrap() = setup;
    if let Some(a37) = &a37 {
        refresh_account(&inner, a37).await;
    }
    if let (Setup::Ready, Some(a37), Some(t3)) = (setup, a37, t3) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        match App::new(a37, t3, inner.data_dir.clone(), tx) {
            Ok(app) => {
                *inner.app.lock().unwrap() = Some(app.clone());
                let _ = inner.signals.unbounded_send(Signal::LoginItem(app.settings().start_at_login));
                tokio::spawn(events(inner.clone(), app.clone(), rx));
                tokio::spawn(background(inner.clone(), app.clone()));
                let mcp_inner = inner.clone();
                let mcp_app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = mcp::serve(mcp_app).await {
                        mcp_inner.notify("t3agent37", format!("Coding agents cannot reach t3agent37: {e:#}"));
                    }
                });
                tokio::spawn(refresh_mcp(inner.clone()));
            }
            Err(e) => inner.notify("t3agent37 could not start", format!("{e:#}")),
        }
    }
    inner.changed();
}

async fn refresh_account(inner: &Inner, a37: &Agent37) {
    if let Ok(b) = a37.billing().await {
        *inner.account.lock().unwrap() =
            Some(Account { balance_usd: b.balance_usd, memory_limit_gb: b.memory_limit_gb, past_due: b.past_due });
    }
}

async fn refresh_mcp(inner: Arc<Inner>) {
    let status = |client| async move { if mcp::installed(client).await { Some(mcp::registered(client).await) } else { None } };
    let (claude_code, codex) = tokio::join!(status(Client::ClaudeCode), status(Client::Codex));
    *inner.mcp.lock().unwrap() = McpStatus { claude_code, codex };
    inner.changed();
}

async fn events(inner: Arc<Inner>, app: Arc<App>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Event>) {
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Persist => app.save(),
            Event::Changed => inner.changed(),
            Event::Notify { title, body } => {
                tracing::info!("notification: {title}: {body}");
                inner.notify(title, body)
            }
        }
    }
}

/// Costs and the account change slowly; refreshed on a timer. And links between machines that
/// run out soon are made again.
async fn background(inner: Arc<Inner>, app: Arc<App>) {
    let alerts = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            alerts.every_minute().await;
        }
    });
    loop {
        app.refresh_costs().await;
        refresh_account(&inner, &app.shared.a37).await;
        app.renew_links().await;
        inner.changed();
        tokio::time::sleep(COSTS_EVERY).await;
    }
}
