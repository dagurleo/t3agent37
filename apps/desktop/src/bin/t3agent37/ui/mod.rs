//! The panel: the rooftop strip, one view under it (the machine list, a
//! machine's details, a new machine, settings, or setup), and a toast. It lives as long as
//! the app; closing the panel only hides its window, so a sign-in running in the browser or
//! a half-typed name is still there on the next click.

mod details;
mod forms;
mod header;
mod kit;
mod list;
mod now;
mod rooftop;
mod settings;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3agent37_lib::app::{MachineView, Setup};
use t3agent37_lib::mcp::Client;
use t3agent37_lib::projects::Project;

use crate::backend::{Backend, Cmd, Details, UiState};
use crate::model::{Action, machine_key};
use crate::pixel::{Px, Theme};
use crate::theme::{self, BODY, MONO};
use kit::Icon;

const FPS: u64 = 12;
/// The strip's slide between full and compact, as `--strip-ease`.
const STRIP_EASE: Duration = Duration::from_millis(220);

/// The tabs of a machine's details (`details::details_tabs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailsTab {
    Activity,
    Projects,
    Accounts,
    Connections,
}

impl DetailsTab {
    const ALL: [DetailsTab; 4] = [DetailsTab::Activity, DetailsTab::Projects, DetailsTab::Accounts, DetailsTab::Connections];
}

#[derive(Clone, PartialEq, Debug)]
pub enum View {
    List,
    New,
    Settings,
    /// That machine's details, by id.
    Machine(String),
}

/// Something fetched: still coming, there, or failed with a message.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

struct Toast {
    text: String,
    error: bool,
    until: Instant,
}

/// The details view's "+ Add a project…" list.
struct Picker {
    query: Entity<InputState>,
    active: usize,
    _sub: Subscription,
}

/// The new machine form.
struct NewForm {
    name: Entity<InputState>,
    size: String,
    picked: Vec<String>,
    /// A copy of this machine (id, name) instead of a new one.
    copy_of: Option<(String, String)>,
    /// A throwaway machine: deleted this many seconds after it's made.
    delete_after: Option<u64>,
    _sub: Subscription,
}

pub struct Panel {
    backend: Backend,
    state: UiState,
    view: View,
    /// Which agent the strip calls out: the hovered row.
    hover: Option<String>,
    /// Per machine key: the busy label of its running command.
    pending: HashMap<String, &'static str>,
    /// Per machine key: why its last delete was refused.
    delete_errors: HashMap<String, String>,
    toast: Option<Toast>,
    /// The row whose ⋯ menu is open, and whether its delete is armed.
    menu: Option<String>,
    menu_confirm: bool,
    /// The menu last closed by a click outside it, and when: that click may be on its button.
    menu_closed: Option<(String, Instant)>,
    /// A two-step button armed for 3 s: its key.
    armed: Option<(String, Instant)>,

    // the details view
    details: Option<Details>,
    details_failing: bool,
    /// (machine id, its latest thread) once looked up.
    latest_thread: Option<(String, Option<String>)>,
    stay_awake_open: bool,
    more_open: bool,
    signing_in: Option<String>,
    /// Machines whose sign-in the user canceled, until it has stopped.
    canceled_sign_in: std::collections::HashSet<String>,
    /// The auto sleep switch as clicked, until the machine says so too.
    power_want: Option<bool>,
    /// A T3 Connect switch as clicked ("approval" or "push"), until the machine says so too.
    connect_want: Option<(&'static str, bool)>,
    /// A project's issues switch as clicked (project id, on), until the machine says so too.
    issues_want: Option<(String, bool)>,
    /// A project's issue settings being picked, to turn issues on or change them.
    issues_form: Option<details::IssuesForm>,
    /// The port whose Open runs.
    opening_port: Option<u16>,
    /// The previews section's "port" field, while it's open.
    port_input: Option<(Entity<InputState>, Subscription)>,
    /// The field for a limited GitHub token, open for one machine (its id). Masked.
    github_token_input: Option<(String, Entity<InputState>, Subscription)>,
    picker: Option<Picker>,

    new_form: Option<NewForm>,
    /// T3 Code's projects for the picker and the new machine form, fetched on each open.
    projects: Load<Vec<Project>>,
    /// Favicons by project id.
    favicons: HashMap<String, Option<Arc<Image>>>,

    // settings and setup
    mcp_adding: Option<Client>,
    login_want: Option<bool>,
    key_input: Entity<InputState>,

    // drawing
    started: Instant,
    /// Images painted last frame: dropped from GPUI's atlas once the next is made.
    shown: Vec<Arc<RenderImage>>,
    /// Strip height animation: from, to, since.
    strip: (f32, f32, Instant),
    /// The look the strip was last drawn in (the animated rooftop or not), so a change in
    /// Settings takes effect at once.
    rooftop_on: Option<bool>,
    roof_scroll: ScrollHandle,
    details_scroll: ScrollHandle,
    /// The details' tab, Activity when a machine opens.
    details_tab: DetailsTab,
    /// Whether "finished" lists everything, not just the latest few.
    finished_all: bool,
    /// The screenshot aid's state (`screenshot_aid`).
    screenshot_opened: bool,
    screenshot_paged: Instant,
    /// The agent the wide roof last scrolled to.
    roof_followed: Option<String>,
    /// Bumped on each view change, so the new view rises in.
    view_serial: u64,
    visible: bool,
    was_active: bool,
    reduce_motion: bool,
    applied_theme: Option<Theme>,
    focus: FocusHandle,
    _tick: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1000 / FPS)).await;
                let alive = this.update(cx, |this, cx| {
                    if this.toast.as_ref().is_some_and(|t| Instant::now() >= t.until) {
                        this.toast = None;
                        cx.notify();
                    }
                    if this.armed.as_ref().is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(3)) {
                        this.armed = None;
                        cx.notify();
                    }
                    if this.visible && !this.reduce_motion {
                        cx.notify();
                    }
                    this.screenshot_aid(cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let subscriptions = vec![
            // Clicked away (another app, the desktop): the panel goes, as menu-bar panels do.
            cx.observe_window_activation(window, |this: &mut Self, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active && this.visible && std::env::var_os("T3AGENT37_KEEP_PANEL_OPEN").is_none() {
                    this.hide(window, cx);
                }
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("sk_live_…").masked(true));
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.subscribe_in(&key_input, window, |this: &mut Self, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_key(window, cx);
            }
            cx.notify();
        }));
        let focus = cx.focus_handle();
        let state = backend.state();
        Self {
            backend,
            state,
            view: View::List,
            hover: None,
            pending: HashMap::new(),
            delete_errors: HashMap::new(),
            toast: None,
            menu: None,
            menu_confirm: false,
            menu_closed: None,
            armed: None,
            details: None,
            details_failing: false,
            latest_thread: None,
            stay_awake_open: false,
            more_open: false,
            signing_in: None,
            canceled_sign_in: Default::default(),
            power_want: None,
            connect_want: None,
            issues_want: None,
            issues_form: None,
            opening_port: None,
            port_input: None,
            github_token_input: None,
            picker: None,
            new_form: None,
            projects: Load::Loading,
            favicons: HashMap::new(),
            mcp_adding: None,
            login_want: None,
            key_input,
            started: Instant::now(),
            shown: Vec::new(),
            strip: (rooftop::FULL, rooftop::FULL, Instant::now()),
            rooftop_on: None,
            roof_scroll: ScrollHandle::new(),
            details_scroll: ScrollHandle::new(),
            details_tab: DetailsTab::Activity,
            finished_all: false,
            screenshot_opened: false,
            screenshot_paged: Instant::now(),
            roof_followed: None,
            view_serial: 0,
            visible: false,
            was_active: false,
            reduce_motion: crate::macos::reduce_motion(),
            applied_theme: None,
            focus,
            _tick: tick,
            _subscriptions: subscriptions,
        }
    }

    /// Shown or hidden by the app. Shown: keyboard focus, a fresh look at the coding agents.
    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        if visible {
            self.was_active = false;
            self.reduce_motion = crate::macos::reduce_motion();
            self.focus_root(window, cx);
            self.backend.refresh_mcp();
        } else {
            self.menu = None;
            self.menu_confirm = false;
            self.stay_awake_open = false;
        }
        cx.notify();
    }

    fn focus_root(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.view {
            View::New => self.new_form.as_ref().map(|f| f.name.clone()),
            View::Machine(_) => self.picker.as_ref().map(|p| p.query.clone()),
            _ if self.state.setup == Setup::ConnectAgent37 => Some(self.key_input.clone()),
            _ => None,
        };
        match input {
            Some(input) => input.update(cx, |s, cx| s.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    /// Hidden by a click away, Esc, or the menu-bar item.
    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(false, window, cx);
        crate::hide_window(window, cx);
    }

    /// A new snapshot from the backend.
    pub fn refresh(&mut self, state: UiState, cx: &mut Context<Self>) {
        self.state = state;
        if let View::Machine(id) = &self.view {
            if self.state.machines.iter().any(|m| &m.id == id) {
                self.load_details(false, cx);
            } else {
                // deleted while its details were open
                self.view = View::List;
                self.view_serial += 1;
            }
        }
        if self.power_want.is_some() && !self.pending.values().any(|b| b.contains("auto sleep")) {
            self.power_want = None;
        }
        if self.connect_want.is_some() && !self.pending.values().any(|b| b.contains("device approval") || b.contains("phone push")) {
            self.connect_want = None;
        }
        if self.issues_want.is_some() && !self.pending.values().any(|b| b.contains("issue")) {
            self.issues_want = None;
        }
        if self.login_want == Some(self.state.settings.start_at_login) {
            self.login_want = None;
        }
        cx.notify();
    }

    pub fn tooltip(state: &UiState) -> String {
        use t3agent37_lib::app::MachineState::*;
        let working = state.machines.iter().filter(|m| m.state == Working).count();
        let awake = state.machines.iter().filter(|m| matches!(m.state, Idle | Working | Waking)).count();
        match (working, awake) {
            (0, 0) => "t3agent37 · all machines asleep".into(),
            (0, n) => format!("t3agent37 · {n} awake"),
            (w, _) => format!("t3agent37 · {w} working"),
        }
    }

    fn machine(&self, key: &str) -> Option<&MachineView> {
        self.state.machines.iter().find(|m| machine_key(m) == key)
    }

    fn toast(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        let until = Instant::now() + Duration::from_secs(if error { 6 } else { 4 });
        self.toast = Some(Toast { text: text.into(), error, until });
        cx.notify();
    }

    /// For screenshots of a machine's details, which are taller than the panel: with
    /// `T3AGENT37_OPEN_MACHINE=<name>` the panel opens on that machine's details, and with
    /// `T3AGENT37_PAGE_EVERY=<secs>` it then pages down through each tab every so many seconds,
    /// on to the next tab after the end. Neither does anything otherwise.
    fn screenshot_aid(&mut self, cx: &mut Context<Self>) {
        if let Ok(name) = std::env::var("T3AGENT37_OPEN_MACHINE")
            && !self.screenshot_opened
            && let Some(id) = self.state.machines.iter().find(|m| m.name.eq_ignore_ascii_case(&name)).map(|m| m.id.clone())
        {
            self.screenshot_opened = true;
            self.view = View::Machine(id);
            self.view_serial += 1;
            self.load_details(true, cx);
            cx.notify();
        }
        let Some(every) = std::env::var("T3AGENT37_PAGE_EVERY").ok().and_then(|s| s.parse::<f64>().ok()) else { return };
        if !matches!(self.view, View::Machine(_)) || self.screenshot_paged.elapsed().as_secs_f64() < every {
            return;
        }
        self.screenshot_paged = Instant::now();
        let (offset, max, page) = (self.details_scroll.offset(), self.details_scroll.max_offset(), self.details_scroll.bounds().size.height);
        // Past the end of a tab: the next tab, from its top.
        if -offset.y >= max.y - px(1.) {
            let i = DetailsTab::ALL.iter().position(|t| *t == self.details_tab).unwrap_or(0);
            self.details_tab = DetailsTab::ALL[(i + 1) % DetailsTab::ALL.len()];
            self.details_scroll.set_offset(point(px(0.), px(0.)));
        } else {
            self.details_scroll.set_offset(point(px(0.), (offset.y - page * 0.85).max(-max.y)));
        }
        cx.notify();
    }

    /// Shows another view; each starts fresh.
    fn go(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        self.go_picking(view, false, window, cx)
    }

    /// `pick_project`: a machine's details, opened with the project picker up.
    fn go_picking(&mut self, view: View, pick_project: bool, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(view, View::Machine(_)) && view != self.view {
            self.details_tab = if pick_project { DetailsTab::Projects } else { DetailsTab::Activity };
            self.finished_all = false;
        }
        let compact = |v: &View| *v != View::List;
        if compact(&view) != compact(&self.view) {
            let now = self.strip_height();
            self.strip = (now, self.strip_target(&view), Instant::now());
        }
        self.menu = None;
        self.menu_confirm = false;
        self.armed = None;
        self.hover = None;
        self.stay_awake_open = false;
        self.more_open = false;
        self.signing_in = None;
        self.power_want = None;
        self.connect_want = None;
        self.issues_want = None;
        self.issues_form = None;
        self.opening_port = None;
        self.port_input = None;
        self.github_token_input = None;
        self.picker = None;
        self.new_form = None;
        self.details = None;
        self.details_failing = false;
        self.view = view.clone();
        self.view_serial += 1;
        match view {
            View::Machine(id) => {
                self.load_details(true, cx);
                if self.latest_thread.as_ref().is_none_or(|(of, _)| *of != id) {
                    self.latest_thread = None;
                    let lookup = self.backend.latest_thread(id.clone());
                    cx.spawn(async move |this, cx| {
                        let thread = lookup.await.unwrap_or(None);
                        let _ = this.update(cx, |this, cx| {
                            this.latest_thread = Some((id, thread));
                            cx.notify();
                        });
                    })
                    .detach();
                }
                if pick_project {
                    self.open_picker(window, cx);
                }
            }
            View::New => self.open_new_form(window, cx),
            _ => {}
        }
        self.focus_root(window, cx);
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go(View::List, window, cx);
    }

    /// Esc: the innermost open thing closes first; on the list it closes the panel.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu = None;
            self.menu_confirm = false;
        } else if self.stay_awake_open {
            self.stay_awake_open = false;
        } else if self.port_input.is_some() {
            self.port_input = None;
            self.focus.focus(window, cx);
        } else if self.github_token_input.is_some() {
            self.github_token_input = None;
            self.focus.focus(window, cx);
        } else if self.issues_form.is_some() {
            self.issues_form = None;
            self.focus.focus(window, cx);
        } else if self.picker.is_some() {
            self.picker = None;
            self.focus.focus(window, cx);
        } else if self.view != View::List && self.state.setup == Setup::Ready {
            self.back(window, cx);
        } else {
            self.hide(window, cx);
        }
        cx.notify();
    }

    /// `opened`: the view just opened (else it's a refresh, on every change).
    fn load_details(&mut self, opened: bool, cx: &mut Context<Self>) {
        let View::Machine(id) = &self.view else { return };
        match self.backend.details(id, opened) {
            Ok(d) => {
                self.details = Some(d);
                self.details_failing = false;
            }
            Err(e) => {
                if !self.details_failing {
                    self.details_failing = true;
                    self.toast(format!("Couldn't load details: {e}"), true, cx);
                }
            }
        }
    }

    /// Fetches T3 Code's projects afresh.
    fn load_projects(&mut self, cx: &mut Context<Self>) {
        self.projects = Load::Loading;
        let fetch = self.backend.local_projects();
        cx.spawn(async move |this, cx| {
            let result = fetch.await;
            let _ = this.update(cx, |this, cx| {
                this.projects = match result {
                    Ok(list) => Load::Ready(list),
                    Err(e) => Load::Failed(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    // ── two-step buttons ──

    fn is_armed(&self, key: &str) -> bool {
        self.armed.as_ref().is_some_and(|(k, _)| k == key)
    }

    /// First call arms `key` for 3 s; a second call within that runs it (true).
    fn arm(&mut self, key: &str) -> bool {
        if self.is_armed(key) {
            self.armed = None;
            return true;
        }
        self.armed = Some((key.into(), Instant::now()));
        false
    }

    // ── running commands ──

    /// Stops the sign-in that runs on `m`; its own `track` then says so.
    fn cancel_sign_in(&mut self, m: &MachineView, cx: &mut Context<Self>) {
        self.canceled_sign_in.insert(machine_key(m));
        let job = self.backend.act(m.id.clone(), Cmd::CancelSignIn);
        cx.spawn(async move |_, _| {
            let _ = job.await;
        })
        .detach();
        cx.notify();
    }

    /// Runs a machine command, its busy label on the row meanwhile; says how it went.
    fn track(&mut self, m: &MachineView, action: Action, cx: &mut Context<Self>) {
        let key = machine_key(m);
        let (name, paired) = (m.name.clone(), m.paired);
        if let Cmd::Delete { discard: false } = action.cmd {
            self.delete_errors.remove(&key);
        }
        self.pending.insert(key.clone(), action.busy);
        cx.notify();
        let job = self.backend.act(m.id.clone(), action.cmd.clone());
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.pending.remove(&key);
                let accept_hint = format!("Switch to T3 Code to accept \"{name}\"");
                match (result, &action.cmd) {
                    (Err(e), Cmd::Delete { discard: false }) => {
                        this.delete_errors.insert(key, e);
                    }
                    (Err(_), Cmd::SignIn { agent, .. }) if this.canceled_sign_in.remove(&key) => {
                        this.toast(format!("{agent} sign-in canceled. {name} keeps the sign-in it had."), false, cx)
                    }
                    (Err(e), _) => this.toast(format!("{name}: {e}"), true, cx),
                    (Ok(who), Cmd::SignIn { agent, .. }) => {
                        this.canceled_sign_in.remove(&key);
                        let text = match who {
                            Some(who) if action.busy == crate::model::SWITCHING => format!("{agent} on {name} is now {who}"),
                            Some(who) => format!("{agent} is signed in on {name} as {who}"),
                            None => format!("{agent} is signed in on {name}"),
                        };
                        this.toast(text, false, cx)
                    }
                    (Ok(_), Cmd::TurnOnConnect) => this.toast(format!("{name} is reachable from your other devices"), false, cx),
                    // What the backend said: what it spent, when it goes, which files.
                    (Ok(Some(text)), Cmd::Limit(_) | Cmd::DeleteAfter(_) | Cmd::Env { .. } | Cmd::Base(_) | Cmd::Collect) => this.toast(text, false, cx),
                    (Ok(_), Cmd::Link { to_name, access, .. }) => {
                        let text = match access {
                            Some(a) => format!("{name}'s agents can use {to_name} now: {}", a.about()),
                            None => format!("{name} no longer uses {to_name}"),
                        };
                        this.toast(text, false, cx)
                    }
                    (Ok(url), Cmd::WakeLink { renew }) => {
                        if let Some(url) = url {
                            cx.write_to_clipboard(ClipboardItem::new_string(url));
                        }
                        let text = if *renew {
                            "New wake link copied. The old one no longer works.".to_string()
                        } else {
                            format!("Wake link copied. Open it on your phone to wake {name}.")
                        };
                        this.toast(text, false, cx);
                    }
                    (Ok(who), Cmd::GithubToken(_)) => {
                        let who = who.unwrap_or_default();
                        this.toast(format!("GitHub on {name} is now {who}"), false, cx)
                    }
                    (Ok(url), Cmd::Start { on, replace }) => {
                        if let Some(url) = url {
                            cx.write_to_clipboard(ClipboardItem::new_string(url));
                        }
                        let text = match (on, replace) {
                            (false, _) => format!("{name}'s start URL is off. The old URL no longer works."),
                            (true, true) => "New start URL copied. The old one no longer works.".to_string(),
                            (true, false) => format!("Start URL copied. Give it to Grok or a script to start threads on {name}."),
                        };
                        this.toast(text, false, cx);
                    }
                    (Ok(Some(url)), Cmd::OpenPort(port)) => {
                        let here = url.trim_start_matches("http://");
                        this.toast(format!("Opened {here} · port {port} on {name}"), false, cx);
                    }
                    (Ok(_), Cmd::Issues { on: true, settings: Some(s), .. }) => {
                        let saved = if action.busy.starts_with("Saving") { "Saved. " } else { "" };
                        this.toast(format!("{saved}Label a GitHub issue \"{}\" and {name} opens a pull request", s.label), false, cx)
                    }
                    (Ok(_), Cmd::Open { .. }) if !paired => this.toast(accept_hint, false, cx),
                    (Ok(_), Cmd::Pair) => this.toast(accept_hint, false, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Not a tracked action: a clone can take minutes, and its progress shows on the machine.
    fn add_project(&mut self, m: &MachineView, p: &Project, cx: &mut Context<Self>) {
        let job = self.backend.add_project(m.id.clone(), p.id.clone());
        let (machine, title) = (m.name.clone(), p.title.clone());
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.toast(format!("{title} is on {machine}"), false, cx),
                Err(e) => this.toast(format!("Couldn't add {title} to {machine}: {e}"), true, cx),
            });
        })
        .detach();
    }

    /// A project's favicon (a `data:` URL from T3 Code) as an image, made once.
    fn favicon(&mut self, p: &Project) -> Option<Arc<Image>> {
        self.favicons
            .entry(p.id.clone())
            .or_insert_with(|| {
                use base64::Engine as _;
                let url = p.icon.as_deref()?;
                let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
                let format = match meta.split(';').next()? {
                    "image/png" => ImageFormat::Png,
                    "image/svg+xml" => ImageFormat::Svg,
                    "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
                    "image/gif" => ImageFormat::Gif,
                    "image/webp" => ImageFormat::Webp,
                    "image/x-icon" | "image/vnd.microsoft.icon" | "image/ico" => ImageFormat::Ico,
                    "image/bmp" => ImageFormat::Bmp,
                    _ => return None,
                };
                let bytes = if meta.ends_with(";base64") {
                    base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?
                } else {
                    urlencoding_decode(data).into_bytes()
                };
                Some(Arc::new(Image::from_bytes(format, bytes)))
            })
            .clone()
    }

    /// A buffer of scene pixels as an image, `k` device pixels each.
    fn image(&mut self, px: &Px, k: usize) -> Arc<RenderImage> {
        let bytes = px.to_bgra(k);
        let buffer = image::RgbaImage::from_raw(px.w as u32 * k as u32, px.h as u32 * k as u32, bytes).expect("sized to fit");
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        self.shown.push(image.clone());
        image
    }

    /// Seconds on the animation clock; `None` holds every picture still (Reduce motion).
    fn clock(&self) -> Option<f64> {
        (!self.reduce_motion).then(|| self.started.elapsed().as_secs_f64())
    }

    /// Seconds for the blinking bits (dots, skeletons), which run even when still.
    fn t(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

/// `%XX` escapes of a non-base64 data URL.
fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in std::mem::take(&mut self.shown) {
            cx.drop_image(old, Some(window));
        }
        let theme = theme::theme_of(window);
        let tk = theme::tokens(theme);
        if self.applied_theme != Some(theme) {
            theme::apply_to_kit(&tk, cx);
            self.applied_theme = Some(theme);
        }

        let body: AnyElement = match self.state.setup {
            Setup::Ready => match self.view.clone() {
                View::List => self.render_list(&tk, window, cx).into_any_element(),
                View::New => self.render_new(&tk, window, cx).into_any_element(),
                View::Settings => self.render_settings(&tk, window, cx).into_any_element(),
                View::Machine(id) => match self.state.machines.iter().find(|m| m.id == id).cloned() {
                    Some(m) => self.render_details(&m, &tk, window, cx).into_any_element(),
                    None => div().into_any_element(),
                },
            },
            _ => self.render_setup(&tk, window, cx).into_any_element(),
        };
        let strip = self.render_strip(theme, &tk, window, cx);
        let costs = (self.state.setup == Setup::Ready && !matches!(self.view, View::Machine(_))).then(|| self.render_costs(&tk));
        let serial = self.view_serial;
        // each view rises in alongside the sky sliding
        let body = div().id(("view", serial)).relative().flex().flex_col().flex_1().min_h_0().child(body);
        let body = if self.reduce_motion {
            body.into_any_element()
        } else {
            body.with_animation(("rise", serial), Animation::new(STRIP_EASE).with_easing(ease_out_quint()), |el, d| el.opacity(d).top(px(6. * (1. - d))))
                .into_any_element()
        };

        let toast = self.toast.as_ref().map(|t| {
            div()
                .id("toast")
                .absolute()
                .left(px(12.))
                .right(px(12.))
                .bottom(px(64.))
                .flex()
                .items_start()
                .gap(px(8.))
                .px(px(12.))
                .py(px(9.))
                .bg(tk.fg)
                .text_color(tk.bg)
                .font_family(MONO)
                .text_size(px(11.5))
                .line_height(relative(1.4))
                .shadow(kit::hard_shadow(&tk))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toast = None;
                    cx.notify();
                }))
                .when(t.error, |d| d.child(div().mt(px(2.)).child(kit::icon(Icon::Alert, 12., tk.danger))))
                .child(div().flex_1().min_w_0().child(t.text.clone()))
        });

        div()
            .id("panel")
            .track_focus(&self.focus)
            .key_context("Panel")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let k = &event.keystroke;
                if k.key == "escape" {
                    this.escape(window, cx);
                }
                // ⌘+ and ⌘− resize the panel, ⌘0 puts it back.
                if k.modifiers.platform && this.state.setup == Setup::Ready {
                    let size = this.state.settings.panel_size;
                    match k.key.as_str() {
                        "=" | "+" => this.set_panel_size(size.step(true), window, cx),
                        "-" => this.set_panel_size(size.step(false), window, cx),
                        "0" => this.set_panel_size(t3agent37_lib::store::PanelSize::Standard, window, cx),
                        _ => {}
                    }
                }
            }))
            // Esc in a text field: the field has its own binding, so it's caught on the way in.
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.escape(window, cx);
                cx.stop_propagation();
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(tk.bg)
            .text_color(tk.fg)
            .font_family(BODY)
            .font_features(FontFeatures(Arc::new(vec![("cv11".into(), 1), ("ss01".into(), 1)])))
            .text_size(px(13.))
            .line_height(relative(1.4))
            .child(strip)
            .child(body)
            .children(costs)
            .children(toast)
    }
}

/// Text in a field, its first or only line.
fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
