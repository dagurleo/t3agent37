//! What the panel does with a machine: its actions, which one is
//! obvious now, and the words and numbers it shows.

use std::time::{SystemTime, UNIX_EPOCH};

use t3agent37_lib::app::{MachineState as State, MachineView};
use t3agent37_lib::machine::provision::SizeInfo;

use crate::backend::Cmd;
use crate::pixel::MachineState;

/// The rooftop's state for a machine's.
pub fn pixel_state(s: State) -> MachineState {
    match s {
        State::Working => MachineState::Working,
        State::Idle => MachineState::Idle,
        State::Sleeping => MachineState::Sleeping,
        State::Asleep => MachineState::Asleep,
        State::Waking => MachineState::Waking,
        State::Creating => MachineState::Creating,
        State::Error => MachineState::Error,
        State::Missing => MachineState::Missing,
        State::Unknown => MachineState::Unknown,
    }
}

/// Stable key: creating rows have no id yet.
pub fn machine_key(m: &MachineView) -> String {
    if m.id.is_empty() { format!("new:{}", m.name) } else { m.id.clone() }
}

/// A backend command for one machine, with what its button says.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub label: String,
    /// Shown on the row while it runs.
    pub busy: &'static str,
    /// A solid button when it's the primary one; outline otherwise.
    pub solid: bool,
    pub cmd: Cmd,
}

fn action(label: impl Into<String>, busy: &'static str, cmd: Cmd) -> Action {
    Action { label: label.into(), busy, solid: false, cmd }
}

/// An entry of a row's ⋯ menu: an action, or opening the project picker in its details.
#[derive(Clone, Debug)]
pub enum MenuItem {
    Act(Action),
    PickProject,
}

pub const PICK_PROJECT: &str = "Add a project…";

pub fn pair() -> Action {
    Action { solid: true, ..action("Add to T3 Code", "Sending…", Cmd::Open { thread: None }) }
}
pub fn repair() -> Action {
    action("Add to T3 Code again", "Sending…", Cmd::Pair)
}
pub fn wake() -> Action {
    action("Wake now", "Waking…", Cmd::Wake)
}
pub fn retry() -> Action {
    action("Try waking", "Waking…", Cmd::Wake)
}
pub fn sleep() -> Action {
    action("Sleep now", "Sleeping…", Cmd::Sleep)
}
pub fn connect() -> Action {
    action("Turn on", "Approve in browser…", Cmd::TurnOnConnect)
}
pub fn wake_link() -> Action {
    action("Copy link", "Copying…", Cmd::WakeLink { renew: false })
}
pub fn new_wake_link() -> Action {
    action("New link", "Replacing…", Cmd::WakeLink { renew: true })
}
pub fn remove() -> Action {
    action("Remove", "Removing…", Cmd::Delete { discard: true })
}
pub fn delete() -> Action {
    action("Delete", "Deleting…", Cmd::Delete { discard: false })
}
pub fn delete_anyway() -> Action {
    action("Delete anyway", "Deleting…", Cmd::Delete { discard: true })
}
pub fn sign_in_to(agent: &str) -> Action {
    action(format!("Sign in {agent}"), "Signing in…", Cmd::SignIn(agent.into()))
}
pub fn resize_to(size: &str) -> Action {
    action(format!("Resize to {size}"), "Resizing…", Cmd::Resize(size.into()))
}
pub fn connect_approval(on: bool) -> Action {
    action(if on { "Approve new devices" } else { "Let new devices in" }, "Changing device approval…", Cmd::ConnectApproval(on))
}
pub fn connect_push(on: bool) -> Action {
    action(if on { "Push to phone" } else { "Stop pushing to phone" }, "Changing phone push…", Cmd::ConnectPush(on))
}
pub fn open_port(port: u16) -> Action {
    action(format!("Open :{port}"), "Opening…", Cmd::OpenPort(port))
}
pub fn close_port(port: u16) -> Action {
    action(format!("Stop forwarding :{port}"), "Stopping…", Cmd::ClosePort(port))
}

/// Opens the machine's latest thread in T3 Code, named so the button says where it goes.
pub fn open_in(thread: Option<String>) -> Action {
    let label = if thread.is_some() { "Open last thread in T3 Code" } else { "Open T3 Code" };
    action(label, "Opening…", Cmd::Open { thread })
}

/// "Stay awake for…" choices, in minutes.
pub const STAY_AWAKE: [u32; 4] = [15, 30, 60, 120];

/// A one-time "stay up at least this long"; 0 stops it early.
pub fn keep_awake(minutes: u32) -> Action {
    if minutes == 0 {
        action("Stop staying awake", "Stopping…", Cmd::KeepAwake(Some(0)))
    } else {
        action(format!("Stay awake for {}", duration(minutes as u64 * 60)), "Waking…", Cmd::KeepAwake(Some(minutes)))
    }
}

/// The auto sleep switch: off keeps the machine up until it's turned back on.
pub fn auto_sleep(on: bool) -> Action {
    if on {
        action("Turn on auto sleep", "Turning on auto sleep…", Cmd::KeepAwake(Some(0)))
    } else {
        action("Turn off auto sleep", "Turning off auto sleep…", Cmd::KeepAwake(None))
    }
}

/// The one obvious thing to do with a machine right now, if any.
pub fn primary_action(m: &MachineView) -> Option<Action> {
    match m.state {
        // wakes by itself when used in T3 Code; opening it lives in the details view
        State::Working | State::Idle | State::Asleep => (!m.paired).then(pair),
        State::Error => Some(retry()),
        State::Missing => Some(remove()),
        // transitional: the agent and the detail line say what's happening
        _ => None,
    }
}

/// Secondary actions for the ⋯ menu (delete is always added by the menu).
pub fn menu_actions(m: &MachineView, sizes: &[SizeInfo]) -> Vec<MenuItem> {
    if m.id.is_empty() || m.state == State::Missing {
        return Vec::new();
    }
    let mut out = Vec::new();
    if m.state == State::Asleep {
        out.push(MenuItem::Act(wake()));
    }
    if matches!(m.state, State::Working | State::Idle) {
        out.push(MenuItem::Act(sleep()));
    }
    out.push(MenuItem::PickProject);
    if m.paired {
        out.push(MenuItem::Act(repair()));
    }
    out.extend(sizes.iter().filter(|s| s.id != m.size).map(|s| MenuItem::Act(resize_to(s.id))));
    out
}

/// Wake or sleep, whichever applies now; none mid-transition.
pub fn power_action(m: &MachineView) -> Option<Action> {
    match m.state {
        State::Asleep => Some(wake()),
        State::Working | State::Idle => Some(sleep()),
        _ => None,
    }
}

/// When a resize takes effect.
pub fn resize_hint(m: &MachineView) -> &'static str {
    match m.state {
        State::Asleep => "Applies on its next wake",
        State::Working => "Applies at a later wake that finds it idle, never mid-turn",
        State::Idle => "Reboots onto the new size now (about a minute)",
        _ => "Billed only while awake",
    }
}

/// Up and added to T3 Code, but nothing to work on yet: the last step of setting it up.
pub fn needs_project(m: &MachineView) -> bool {
    m.paired && m.projects.is_empty() && !matches!(m.state, State::Creating | State::Missing | State::Error)
}

/// The machine has this repository, or is getting it.
pub fn has_repo(m: &MachineView, repo_key: &str) -> bool {
    m.projects.iter().any(|p| p.repo_key.as_deref() == Some(repo_key))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Whole seconds until `ends_at` (unix ms), 0 once past.
pub fn secs_left(ends_at: u64) -> u64 {
    ends_at.saturating_sub(now_ms()).div_ceil(1000)
}

/// The machine's detail line, its countdown ticked to now.
pub fn detail(m: &MachineView) -> String {
    match &m.countdown {
        Some(c) => c.template.replace("{left}", &time_left(secs_left(c.ends_at))),
        None => m.detail.clone(),
    }
}

/// "2h 14m", "2h", "38m", "0m"
pub fn duration(secs: u64) -> String {
    let m = secs / 60;
    if m < 60 {
        format!("{m}m")
    } else if m % 60 > 0 {
        format!("{}h {}m", m / 60, m % 60)
    } else {
        format!("{}h", m / 60)
    }
}

/// "42s", "4m", "1h 5m"; minutes round up. Matches the backend's `human_secs`.
pub fn time_left(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let m = secs.div_ceil(60);
    if m < 60 { format!("{m}m") } else { format!("{}h {}m", m / 60, m % 60) }
}

/// "/Users/you/Dev/app" → "~/Dev/app"
pub fn home_path(path: &str) -> String {
    for root in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(root) {
            return match rest.find('/') {
                Some(i) if i > 0 => format!("~{}", &rest[i..]),
                None if !rest.is_empty() => "~".into(),
                _ => path.into(),
            };
        }
    }
    path.into()
}

/// "Nov 20, 2026"
pub fn short_date(unix_secs: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(unix_secs, 0) {
        chrono::LocalResult::Single(d) => d.format("%b %-d, %Y").to_string(),
        _ => String::new(),
    }
}

pub fn money(n: f64) -> String {
    format!("${n:.2}")
}

pub fn hourly(n: f64) -> String {
    if n < 0.1 { format!("${n:.3}/h") } else { format!("${n:.2}/h") }
}

pub fn size_of<'a>(sizes: &'a [SizeInfo], id: &str) -> Option<&'a SizeInfo> {
    sizes.iter().find(|s| s.id == id)
}

/// Codenames for new machines.
const NAMES: [&str; 12] = ["raven", "falcon", "kestrel", "lynx", "fox", "heron", "magpie", "otter", "sparrow", "jackal", "osprey", "badger"];

pub fn suggest_name(taken: &[String]) -> String {
    let used = |n: &str| taken.iter().any(|t| t == n);
    if let Some(free) = NAMES.iter().find(|n| !used(n)) {
        return free.to_string();
    }
    (2..).map(|i| format!("raven-{i}")).find(|n| !used(n)).unwrap()
}

/// `^[a-z0-9][a-z0-9-]{0,31}$`
pub fn valid_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let mut chars = name.chars();
    chars.next().is_some_and(ok) && name.len() <= 32 && chars.all(|c| ok(c) || c == '-')
}

/// Lowercase, spaces and underscores to dashes, nothing else outside `[a-z0-9-]`, 32 at most.
pub fn clean_name(raw: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in raw.to_lowercase().chars() {
        if c.is_whitespace() || c == '_' {
            gap = true;
            continue;
        }
        if gap {
            out.push('-');
            gap = false;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            out.push(c);
        }
    }
    if gap {
        out.push('-');
    }
    out.chars().take(32).collect()
}

/// "Claude", "Claude and Codex", "A, B and C" (the joiners before each name).
pub fn joiners(n: usize) -> impl Iterator<Item = &'static str> {
    (0..n).map(move |i| if i == 0 { "" } else if i == n - 1 { " and " } else { ", " })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(clean_name("My Box_2!"), "my-box-2");
        assert_eq!(clean_name("a  b"), "a-b");
        assert!(valid_name("raven-2"));
        assert!(!valid_name("-raven"));
        assert!(!valid_name(""));
        assert_eq!(suggest_name(&["raven".into()]), "falcon");
    }

    #[test]
    fn times() {
        assert_eq!(duration(0), "0m");
        assert_eq!(duration(2 * 3600), "2h");
        assert_eq!(duration(8040), "2h 14m");
        assert_eq!(time_left(42), "42s");
        assert_eq!(time_left(61), "2m");
        assert_eq!(time_left(3900), "1h 5m");
        assert_eq!(hourly(0.0301), "$0.030/h");
        assert_eq!(hourly(0.1753), "$0.18/h");
    }

    #[test]
    fn paths() {
        assert_eq!(home_path("/Users/you/Dev/app"), "~/Dev/app");
        assert_eq!(home_path("/Users/you"), "~");
        assert_eq!(home_path("/opt/x"), "/opt/x");
    }
}
