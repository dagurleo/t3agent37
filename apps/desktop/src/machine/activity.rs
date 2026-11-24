//! What a machine is doing, as `activity.mjs` reports it in the watchdog's status: its agents'
//! threads and what each is up to, what they finished lately, what woke it, and its load. The
//! details show it; the last report is kept while it sleeps, so they still say what it did last.

use super::issues::ModelChoice;
use super::projects::MachineProject;
use crate::store::IssueSettings;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// The watchdog's report. Older watchdogs send none.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Activity {
    pub working: Vec<LiveThread>,
    pub recent: Vec<FinishedThread>,
    pub load: Option<Load>,
    pub top: Vec<Process>,
    /// When it last woke (ms since epoch) and what woke it, as the watchdog saw it.
    pub woke_at: Option<i64>,
    pub woke_for: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveThread {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// T3's model options as `id=value`, e.g. `effort=high`.
    pub options: Vec<String>,
    /// "working", "needs your approval", "waits on subagents", ….
    pub state: String,
    pub started_at: Option<String>,
    pub subagents: u32,
    /// Commands it left running: a dev server, a watcher.
    pub background: Vec<String>,
    pub branch: Option<String>,
    /// What it's running right now ("running cargo test", "Read src/main.rs", "thinking"), and
    /// what its agent last said, cut short. Older watchdogs don't say.
    pub doing: Option<String>,
    pub said: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FinishedThread {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    /// completed, failed, stopped, cancelled, ….
    pub status: String,
    pub ended_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Load {
    /// Cores in use (smoothed over about 30 s) and in all.
    pub cores: f64,
    pub cpus: u32,
    pub mem_used_mb: Option<u64>,
    pub mem_total_mb: Option<u64>,
    /// The disk that persists (`/root`).
    pub disk_used_gb: Option<f64>,
    pub disk_total_gb: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Process {
    pub pid: u32,
    /// "T3 server", "Claude Code", or its command line: "next build".
    pub name: String,
    /// Share of one core over the watchdog's last tick, in percent.
    pub cpu: u32,
    pub mem_mb: u64,
}

/// What the details show: the report, with project and model names and times as unix ms.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    /// Read from the machine while it was up; false: as it was before it went to sleep.
    pub live: bool,
    /// When it was read (unix ms).
    pub as_of: u64,
    /// When it last woke (unix ms) and what woke it: "T3 Code", "GitHub (you/repo)", ….
    pub woke_at: Option<u64>,
    pub woke_for: Option<String>,
    pub threads: Vec<ThreadView>,
    pub finished: Vec<FinishedView>,
    pub load: Option<Load>,
    pub top: Vec<Process>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub id: String,
    pub title: String,
    pub project: Option<String>,
    /// "Opus 5.5, high".
    pub agent: Option<String>,
    pub state: String,
    /// It waits for the user: an approval, a question, a sign-in.
    pub needs_you: bool,
    pub started_at: Option<u64>,
    pub subagents: u32,
    pub background: Vec<String>,
    pub branch: Option<String>,
    /// What it's running now, and what its agent last said.
    pub doing: Option<String>,
    pub said: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinishedView {
    pub id: String,
    pub title: String,
    pub project: Option<String>,
    pub status: String,
    pub ended_at: Option<u64>,
    pub error: Option<String>,
}

/// States in which a thread waits for the user, as `activity.mjs` words them.
const NEEDS_YOU: [&str; 4] = ["needs your approval", "asks you something", "needs a sign-in", "waits for you"];

/// The thread waits for the user: an approval, a question, a sign-in.
pub fn needs_you(state: &str) -> bool {
    NEEDS_YOU.contains(&state)
}

impl Activity {
    /// The report as the details show it: projects by title, models by name.
    pub fn view(&self, as_of: SystemTime, live: bool, projects: &[MachineProject], models: &[ModelChoice]) -> ActivityView {
        let project = |id: &Option<String>| id.as_ref().and_then(|id| projects.iter().find(|p| &p.id == id)).map(|p| p.title.clone());
        let ms = |t: &Option<String>| {
            t.as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .and_then(|t| u64::try_from(t.timestamp_millis()).ok())
        };
        ActivityView {
            live,
            as_of: as_of.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            woke_at: self.woke_at.and_then(|t| u64::try_from(t).ok()),
            woke_for: self.woke_for.as_deref().map(wake_reason),
            threads: self
                .working
                .iter()
                .map(|t| ThreadView {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    project: project(&t.project_id),
                    agent: agent(t, models),
                    state: t.state.clone(),
                    needs_you: needs_you(&t.state),
                    started_at: ms(&t.started_at),
                    subagents: t.subagents,
                    background: t.background.clone(),
                    branch: t.branch.clone(),
                    doing: t.doing.clone(),
                    said: t.said.clone(),
                })
                .collect(),
            finished: self
                .recent
                .iter()
                .map(|t| FinishedView {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    project: project(&t.project_id),
                    status: t.status.clone(),
                    ended_at: ms(&t.ended_at),
                    error: t.error.clone(),
                })
                .collect(),
            load: self.load.clone(),
            top: self.top.clone(),
        }
    }
}

/// "Opus 5.5, high": the thread's model as the machine names it.
fn agent(t: &LiveThread, models: &[ModelChoice]) -> Option<String> {
    let (provider, model) = (t.provider.clone()?, t.model.clone()?);
    Some(IssueSettings { label: String::new(), provider, model, options: t.options.clone() }.agent(models))
}

/// What woke the machine, for people. The watchdog's reasons are terse ("user activity",
/// "issue you/repo#3"); the app's own say what it woke the machine for ("app: T3 Code").
pub fn wake_reason(raw: &str) -> String {
    if let Some(by) = raw.strip_prefix("app: ") {
        return by.to_owned();
    }
    if let Some(repo) = raw.strip_prefix("GitHub: ") {
        return format!("GitHub ({repo})");
    }
    match raw {
        "resumed by the app" => "t3agent37".into(),
        "user activity" | "used in T3" => "T3 Code".into(),
        "wake link" => "the wake link".into(),
        "keep awake" => "a keep-awake".into(),
        "agent working" => "an agent".into(),
        r if r.starts_with("issue ") || r.starts_with("feedback on ") || r.starts_with("news on ") => "GitHub".into(),
        r if r.contains("start URL") => "the start URL".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_watchdogs_report() {
        let a: Activity = serde_json::from_value(json!({
            "working": [{"id": "t1", "title": "Fix login", "projectId": "p1", "provider": "claudeAgent", "model": "claude-opus-5-5",
                "options": ["effort=high"], "state": "needs your approval", "startedAt": "2026-11-23T09:50:00Z", "subagents": 2,
                "background": ["bun run dev"], "branch": "t3/fix"}],
            "recent": [{"id": "t2", "title": "Old", "projectId": "gone", "status": "failed", "endedAt": "2026-11-23T09:00:00.000Z", "error": "boom"}],
            "load": {"cores": 1.84, "cpus": 4, "memUsedMb": 1260, "memTotalMb": 7936, "diskUsedGb": 6.1, "diskTotalGb": 19.6},
            "top": [{"pid": 10, "name": "next build", "cpu": 180, "memMb": 100}],
            "wokeAt": 1795396000000_i64, "wokeFor": "GitHub: you/repo", "lastUse": {"reason": "agent working", "at": 1}
        }))
        .unwrap();
        let projects = vec![MachineProject { id: "p1".into(), title: "Tantalum".into(), repo_key: None, root: None, status: None, issues: None }];
        let v = a.view(UNIX_EPOCH + std::time::Duration::from_millis(5), true, &projects, &[]);
        let t = &v.threads[0];
        assert_eq!((t.project.as_deref(), t.agent.as_deref(), t.needs_you), (Some("Tantalum"), Some("claude-opus-5-5, high"), true));
        assert_eq!(t.started_at, Some(1795427400000));
        assert_eq!(v.finished[0].project, None, "a project the machine no longer has");
        assert_eq!(v.finished[0].ended_at, Some(1795424400000));
        assert_eq!(v.woke_for.as_deref(), Some("GitHub (you/repo)"));
        assert_eq!((v.as_of, v.live), (5, true));
        // An older watchdog's status has no report; a partial one fills in.
        assert_eq!(serde_json::from_value::<Activity>(json!({})).unwrap(), Activity::default());
    }

    #[test]
    fn wake_reasons_for_people() {
        assert_eq!(wake_reason("app: T3 Code"), "T3 Code");
        assert_eq!(wake_reason("user activity"), "T3 Code");
        assert_eq!(wake_reason("issue you/repo#3"), "GitHub");
        assert_eq!(wake_reason("start URL: Tantalum"), "the start URL");
        assert_eq!(wake_reason("follow-up from the start URL"), "the start URL");
        assert_eq!(wake_reason("wake link"), "the wake link");
        assert_eq!(wake_reason("a webhook"), "a webhook");
    }
}
