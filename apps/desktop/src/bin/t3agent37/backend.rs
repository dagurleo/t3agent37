//! The app model of `t3agent37_lib` on a tokio runtime of its own, and the snapshot of it the
//! panel draws: startup, the backend's events, and the commands the panel runs.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use t3agent37_lib::agent37::{Agent37, KEYS_PAGE};
pub use t3agent37_lib::app::AgentLogin;
use t3agent37_lib::app::{self, App, Costs, GITHUB, MachineView, Setup};
use t3agent37_lib::machine::Event;
use t3agent37_lib::machine::issues::{IssueSettings, ModelChoice};
pub use t3agent37_lib::machine::preview::LocalServer;
use t3agent37_lib::machine::provision::{SIZES, SizeInfo};
use t3agent37_lib::machine::signin::Agent;
use t3agent37_lib::mcp::{self, Client};
use t3agent37_lib::projects::Project;
pub use t3agent37_lib::store::Preview;
use t3agent37_lib::store::Settings;

const COSTS_EVERY: Duration = Duration::from_secs(300);

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
    pub costs: Costs,
    pub settings: Settings,
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
            costs: app.as_ref().map(|a| a.costs()).unwrap_or_default(),
            settings: app.as_ref().map(|a| a.settings()).unwrap_or_default(),
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

    /// Resolves to the wake link for `Cmd::WakeLink` (for the clipboard), else to `None`.
    pub fn act(&self, id: String, cmd: Cmd) -> impl Future<Output = Res<Option<String>>> + use<> {
        self.run(move |inner| act(inner, id, cmd))
    }

    pub fn create_machine(&self, name: String, size: String, projects: Vec<String>) -> impl Future<Output = Res> + use<> {
        self.run(move |inner| async move { inner.app()?.create_machine(&name, &size, projects).await.map(drop).map_err(err) })
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

    /// For the details view; never wakes the machine. Looks again at the servers listening on
    /// it when it runs.
    pub fn details(&self, id: &str) -> Res<Details> {
        let app = self.inner.app()?;
        let d = app.details(id).map_err(err)?;
        let mut issue_models = Vec::new();
        if let Ok(machine) = app.machine(id) {
            issue_models = machine.issue_models();
            self.rt.spawn(async move { machine.refresh_servers().await });
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
        Cmd::Open { thread } => app.open_in_t3(&machine, thread.as_deref()),
        Cmd::Pair => app.pair(&machine),
        Cmd::Wake => machine.ensure_awake().await,
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

/// Costs and the account change slowly; refreshed on a timer.
async fn background(inner: Arc<Inner>, app: Arc<App>) {
    let alerts = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            alerts.check_awake_alerts();
        }
    });
    loop {
        app.refresh_costs().await;
        refresh_account(&inner, &app.shared.a37).await;
        inner.changed();
        tokio::time::sleep(COSTS_EVERY).await;
    }
}
