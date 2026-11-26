//! What new machines start with. Sign-ins saved on this computer (`sign-ins.json` in the app's
//! data folder, readable by the user only): Claude's long-lived token, GitHub tokens (several,
//! for an account and its organizations), and an OpenAI API key for Codex. Every machine
//! t3agent37 creates gets them: Claude's token before T3 first starts there, then GitHub's (for
//! a spawn, the one that reaches its repo) and Codex's. Then the user's own setup script, if
//! Settings names one, runs there.
//!
//! Spawned machines (`spawn.rs`) start that way, as new machines, unless the user picked a base
//! to copy (`Settings::spawn_from`): a new machine is up in about half a minute and has the
//! latest tools, a copy of a base has whatever was installed on it.

use super::remote;
use super::*;
use crate::machine::signin::{self, Secret};
use crate::store::write_private;
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, Instant};

const FILE: &str = "sign-ins.json";
/// The longest the user's setup script may run on a new machine.
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The biggest setup script sent over.
const MAX_SETUP: u64 = 256 * 1024;

/// The sign-ins saved for new machines. Not `Debug`: it holds the secrets.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedSignIns {
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_token: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    github: Vec<SavedGithub>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_key: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedGithub {
    /// Whose it is, as GitHub said when it was saved.
    login: String,
    token: String,
    /// As GitHub said: `2027-02-20 12:00:00 UTC`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires: Option<String>,
}

impl SavedGithub {
    /// `@you, gh's sign-in`, without the token.
    fn describe(&self) -> String {
        let kind = if self.token.starts_with("github_pat_") {
            "a fine-grained token"
        } else if self.token.starts_with("gho_") {
            "gh's sign-in"
        } else {
            "a classic token"
        };
        let expires = self.expires.as_deref().map(|e| format!(", until {}", e.split(' ').next().unwrap_or(e))).unwrap_or_default();
        format!("@{}, {kind}{expires}", self.login)
    }
}

/// What's saved, without the secrets: for the panel and `t3agent37 new-machines`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInsSaved {
    pub claude: bool,
    /// Each GitHub token, described (`@you, gh's sign-in`), in the order they're tried.
    pub github: Vec<String>,
    pub codex: bool,
}

/// A change to the saved sign-ins.
pub enum SignInChange {
    Claude(Option<Secret>),
    Github(Secret),
    /// The GitHub token with this number (from 1), as `new-machines` lists them.
    GithubRemove(Option<usize>),
    Codex(Option<Secret>),
}

impl SavedSignIns {
    pub(super) fn load(dir: &Path) -> Self {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("reading the saved sign-ins: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&self, dir: &Path) -> Result<()> {
        write_private(&dir.join(FILE), &serde_json::to_vec_pretty(self)?)
    }

    pub fn saved(&self) -> SignInsSaved {
        SignInsSaved { claude: self.claude_token.is_some(), github: self.github.iter().map(SavedGithub::describe).collect(), codex: self.openai_key.is_some() }
    }

    /// Run on a new VM before T3 first starts: Claude's token where boot.sh gives it to T3.
    fn prepare_script(&self) -> Option<String> {
        let token = self.claude_token.as_ref()?;
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("CLAUDE_CODE_OAUTH_TOKEN={token}\n"));
        Some(format!("mkdir -p /root/.t3agent37 && umask 077 && echo {b64} | base64 -d > /root/.t3agent37/agent.env"))
    }

    /// The GitHub token for a machine working on `repo_key` (`github.com/owner/name`): the first
    /// that can push there, else the first that can read it, else the first. GitHub has no list
    /// of what a token reaches, so its git service is asked (from this computer; nothing changes).
    async fn github_for(&self, repo_key: Option<&str>) -> Option<SavedGithub> {
        let repo = repo_key.and_then(|k| k.strip_prefix("github.com/")).map(|r| r.trim_end_matches(".git"));
        let (Some(repo), [_, _, ..]) = (repo, self.github.as_slice()) else { return self.github.first().cloned() };
        let client = reqwest::Client::builder().user_agent("t3agent37").timeout(Duration::from_secs(15)).build().ok()?;
        let reaches = |token: String, service: &'static str| {
            let req = client.get(format!("https://github.com/{repo}.git/info/refs?service={service}")).basic_auth("x-access-token", Some(token));
            async move { req.send().await.is_ok_and(|r| r.status().is_success()) }
        };
        let mut readable = None;
        for g in &self.github {
            if reaches(g.token.clone(), "git-receive-pack").await {
                return Some(g.clone());
            }
            if readable.is_none() && reaches(g.token.clone(), "git-upload-pack").await {
                readable = Some(g.clone());
            }
        }
        readable.or_else(|| self.github.first().cloned())
    }
}

/// What spawned machines start as: a new machine, or a copy of a base.
pub(super) enum Start {
    New,
    Base(Arc<MachineRuntime>),
}

impl Start {
    /// The base's name, or empty for a new machine (`Spawned::base`).
    pub(super) fn base_name(&self) -> String {
        match self {
            Start::New => String::new(),
            Start::Base(b) => b.record().name,
        }
    }

    /// `a new machine`, `a copy of base`.
    pub(super) fn describe(base: &str) -> String {
        if base.is_empty() { "a new machine".into() } else { format!("a copy of {base}") }
    }
}

/// What a new machine was signed in with, and what went wrong.
#[derive(Default)]
pub(super) struct Notes {
    pub done: Vec<String>,
    pub problems: Vec<String>,
}

impl Notes {
    /// `Signed in: Claude, GitHub as @you. Codex: …` for a spawn's answer.
    pub(super) fn text(&self) -> String {
        let mut text = String::new();
        if !self.done.is_empty() {
            text.push_str(&format!("Signed in there: {}.", self.done.join(", ")));
        }
        for p in &self.problems {
            text.push_str(&format!(" {p}"));
        }
        text.trim().to_owned()
    }
}

impl App {
    pub fn sign_ins_saved(&self) -> SignInsSaved {
        self.sign_ins.lock().unwrap().saved()
    }

    /// Saves, replaces or removes a sign-in for new machines. A GitHub token is checked with
    /// GitHub first; one already saved is replaced.
    pub async fn change_sign_in(&self, change: SignInChange) -> Result<String> {
        let mut s = self.sign_ins.lock().unwrap().clone();
        let text = match change {
            SignInChange::Claude(Some(t)) => {
                s.claude_token = Some(signin::check_claude_token(&t.0)?);
                "Claude's token is saved: new machines' Claude uses it.".to_owned()
            }
            SignInChange::Claude(None) => {
                s.claude_token = None;
                "Claude's token is forgotten here: new machines start with Claude signed out.".to_owned()
            }
            SignInChange::Codex(Some(k)) => {
                s.openai_key = Some(signin::check_openai_key(&k.0)?);
                "The OpenAI API key is saved: new machines' Codex uses it (billed per use, not on a ChatGPT plan).".to_owned()
            }
            SignInChange::Codex(None) => {
                s.openai_key = None;
                "The OpenAI API key is forgotten here: new machines start with Codex signed out.".to_owned()
            }
            SignInChange::Github(t) => {
                let token = Secret(signin::check_saved_github_token(&t.0)?);
                let info = signin::check_github_token(&token).await?;
                let saved = SavedGithub { login: info.login, token: token.0, expires: info.expires };
                let text = format!("GitHub is saved for new machines: {}.", saved.describe());
                match s.github.iter_mut().find(|g| g.token == saved.token) {
                    Some(g) => *g = saved,
                    None => s.github.push(saved),
                }
                text
            }
            SignInChange::GithubRemove(n) => {
                let i = match (n, s.github.len()) {
                    (_, 0) => bail!("no GitHub token is saved"),
                    (None, 1) => 0,
                    (None, _) => bail!("say which: {}", numbered(&s.saved().github)),
                    (Some(n), len) if (1..=len).contains(&n) => n - 1,
                    (Some(n), _) => bail!("there's no GitHub token {n}: {}", numbered(&s.saved().github)),
                };
                let gone = s.github.remove(i);
                format!("Forgot {}: GitHub tokens already on machines stay there.", gone.describe())
            }
        };
        s.save(&self.data_dir)?;
        *self.sign_ins.lock().unwrap() = s;
        self.shared.changed();
        Ok(text)
    }

    /// What spawns start as: `new`, or a base by name.
    pub fn set_spawn_from(&self, from: &str) -> Result<String> {
        let from = from.trim();
        let mut settings = self.settings();
        let text = if from == "new" {
            settings.spawn_from = None;
            "Spawned machines start as new machines, signed in with what's saved here.".to_owned()
        } else {
            let b = self.base_for(Some(from))?;
            let name = b.record().name;
            settings.spawn_from = Some(name.clone());
            format!("Spawned machines start as copies of {name}, with its sign-ins and tools.")
        };
        self.update_settings(settings);
        Ok(text)
    }

    /// The setup script new machines run (a path on this computer), or none.
    pub fn set_machine_setup(&self, path: Option<&str>) -> Result<String> {
        let mut settings = self.settings();
        let text = match path {
            Some(p) => {
                let p = std::path::absolute(p.trim()).with_context(|| p.to_owned())?;
                let meta = std::fs::metadata(&p).with_context(|| p.display().to_string())?;
                if !meta.is_file() || meta.len() > MAX_SETUP {
                    bail!("{} isn't a script of at most {} KB", p.display(), MAX_SETUP / 1024);
                }
                let shown = p.display().to_string();
                settings.machine_setup = Some(shown.clone());
                format!("New machines run {shown} once they're made (bash, as root, in its home), before their projects arrive.")
            }
            None => {
                settings.machine_setup = None;
                "New machines run no setup script of yours.".to_owned()
            }
        };
        self.update_settings(settings);
        Ok(text)
    }

    /// What new machines start with, for `t3agent37 new-machines`.
    pub fn new_machines(&self) -> String {
        let settings = self.settings();
        let saved = self.sign_ins_saved();
        let mut lines = vec![match &settings.spawn_from {
            None => "Spawned machines (spawn, run, on a copy) start as new machines.".to_owned(),
            Some(b) => format!("Spawned machines (spawn, run, on a copy) start as copies of {b}; `new-machines start new` makes them new ones."),
        }];
        lines.push("Saved here for new machines:".into());
        lines.push(format!("  Claude: {}", if saved.claude { "a long-lived token" } else { "nothing (`new-machines claude` saves one)" }));
        if saved.github.is_empty() {
            lines.push("  GitHub: nothing (`new-machines github --from-gh` saves this computer's gh sign-in)".into());
        } else {
            for (i, g) in saved.github.iter().enumerate() {
                lines.push(format!("  GitHub {}: {g}", i + 1));
            }
        }
        lines.push(format!("  Codex: {}", if saved.codex { "an OpenAI API key" } else { "nothing: it starts signed out (`new-machines codex` saves an API key)" }));
        lines.push(match &settings.machine_setup {
            Some(p) => format!("Then they run {p}."),
            None => "They run no setup script of yours (`new-machines setup <script>`).".into(),
        });
        if !settings.base_sync.is_empty() {
            lines.push(format!(
                "Spawned machines get {} from here.",
                settings.base_sync.iter().map(|p| format!("~/{p}")).collect::<Vec<_>>().join(", ")
            ));
        }
        lines.join("\n")
    }

    /// What spawns start as: `from` (`new`, or a base), or as Settings says.
    pub(super) fn start_for(&self, from: Option<&str>) -> Result<Start> {
        match from.map(str::trim).filter(|f| !f.is_empty()) {
            Some("new") => Ok(Start::New),
            Some(b) => self.base_for(Some(b)).map(Start::Base),
            None => match self.settings().spawn_from {
                None => Ok(Start::New),
                Some(b) => self
                    .base_for(Some(&b))
                    .map(Start::Base)
                    .with_context(|| format!("spawns start as copies of {b} (`t3agent37 new-machines start new` makes them new machines)")),
            },
        }
    }

    /// Run on a new VM before T3 first starts (`provision::create`).
    pub(super) fn prepare_new(&self) -> Option<String> {
        self.sign_ins.lock().unwrap().prepare_script()
    }

    /// Refused before a spawn makes a new machine no agent could work on.
    pub(super) fn check_agent_saved(&self) -> Result<()> {
        let saved = self.sign_ins_saved();
        if !saved.claude && !saved.codex {
            bail!(
                "new machines have no agent to work with: save Claude's long-lived token here first (`claude setup-token`, then \
                 `t3agent37 new-machines claude`), or an OpenAI API key for Codex (`t3agent37 new-machines codex`). Or spawn copies \
                 of a base instead: `t3agent37 new-machines start <base>`"
            );
        }
        Ok(())
    }

    /// A new machine, up: GitHub signed in with the saved token that reaches `repo_key` (or the
    /// first), Codex with the saved key. Claude has its token from `prepare_new` already.
    pub(super) async fn sign_in_new(&self, m: &MachineRuntime, repo_key: Option<&str>) -> Notes {
        let saved = self.sign_ins.lock().unwrap().clone();
        let mut notes = Notes::default();
        if saved.claude_token.is_some() {
            notes.done.push("Claude (the saved token)".into());
        }
        if let Some(g) = saved.github_for(repo_key).await {
            let who = g.login.clone();
            match m.put_github_token(&Secret(g.token), g.expires).await {
                Ok(()) => notes.done.push(format!("GitHub as @{who}")),
                Err(e) => notes.problems.push(format!("GitHub isn't signed in there: {e:#}.")),
            }
        }
        if let Some(key) = saved.openai_key {
            match m.put_openai_key(&Secret(key)).await {
                Ok(()) => notes.done.push("Codex (the saved API key)".into()),
                Err(e) => notes.problems.push(format!("Codex isn't signed in there: {e:#}.")),
            }
        } else {
            m.refresh_providers().await;
        }
        notes
    }

    /// The user's setup script (Settings), run on a new machine as a job, waited for. None when
    /// there's none; else what it came to.
    pub(super) async fn run_machine_setup(&self, m: &MachineRuntime) -> Option<std::result::Result<String, String>> {
        let path = self.settings().machine_setup?;
        let name = m.record().name;
        let result = m.kept_awake(run_setup(&self.shared.a37, &m.sandbox_id(), &path)).await;
        Some(match result {
            Ok(secs) => Ok(format!("Your setup script ran there ({secs} s).")),
            Err(e) => Err(format!("Your setup script ({path}) failed on {name}: {e:#}.")),
        })
    }
}

/// `1. @you, gh's sign-in; 2. …`
fn numbered(items: &[String]) -> String {
    items.iter().enumerate().map(|(i, g)| format!("{}. {g}", i + 1)).collect::<Vec<_>>().join("; ")
}

/// Sends the script at `path` here to the machine and runs it there as root, in its home; the
/// seconds it took, or its last lines.
async fn run_setup(a37: &crate::agent37::Agent37, id: &str, path: &str) -> Result<u64> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    if bytes.len() as u64 > MAX_SETUP {
        bail!("it's over {} KB", MAX_SETUP / 1024);
    }
    let file = "/root/.t3agent37/machine-setup.sh";
    remote::upload(a37, id, file, &bytes).await?;
    let tag = format!("setup-{}", chrono::Utc::now().timestamp());
    remote::start_job(a37, id, &tag, &format!("cd /root && bash {file}")).await?;
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let (_, code) = remote::job_output(a37, id, &tag, u64::MAX - 1).await?;
        match code {
            Some(0) => return Ok(started.elapsed().as_secs()),
            Some(c) => {
                let tail = a37.exec_ok(id, &format!("tail -n 3 {}/{tag}.out", remote::JOBS), 30).await.unwrap_or_default();
                bail!("exit code {c}: {}", tail.trim().replace('\n', " · "));
            }
            None if started.elapsed() > SETUP_TIMEOUT => {
                let _ = remote::stop_job(a37, id, &tag).await;
                bail!("it ran over {} minutes", SETUP_TIMEOUT.as_secs() / 60);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(login: &str, token: &str, expires: Option<&str>) -> SavedGithub {
        SavedGithub { login: login.into(), token: token.into(), expires: expires.map(str::to_owned) }
    }

    #[test]
    fn what_is_saved_is_said_without_the_secrets() {
        let s = SavedSignIns {
            claude_token: Some("sk-ant-oat01-secret".into()),
            github: vec![gh("you", "gho_secretsecret", None), gh("you", "github_pat_secret", Some("2027-11-23 10:00:00 UTC"))],
            openai_key: None,
        };
        let saved = s.saved();
        assert_eq!(saved.github, ["@you, gh's sign-in", "@you, a fine-grained token, until 2027-11-23"]);
        assert!(saved.claude && !saved.codex);
        let shown = format!("{saved:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn claudes_token_is_there_before_t3_starts() {
        assert!(SavedSignIns::default().prepare_script().is_none());
        let s = SavedSignIns { claude_token: Some("sk-ant-oat01-abc".into()), ..Default::default() };
        let script = s.prepare_script().unwrap();
        // Only base64 reaches the shell, and the file is the owner's alone.
        assert!(!script.contains("sk-ant") && script.contains("umask 077") && script.ends_with("> /root/.t3agent37/agent.env"));
        let b64 = script.split("echo ").nth(1).unwrap().split(' ').next().unwrap();
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-abc\n");
    }

    #[test]
    fn saved_sign_ins_survive_a_restart_readable_by_their_owner_only() {
        let dir = std::env::temp_dir().join(format!("t3agent37-sign-ins-{}", std::process::id()));
        let s = SavedSignIns { openai_key: Some("sk-proj-abcdefghijklmnopqrstuvwxyz".into()), github: vec![gh("you", "ghp_x", None)], ..Default::default() };
        s.save(&dir).unwrap();
        let back = SavedSignIns::load(&dir);
        assert_eq!(back.saved(), s.saved());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Nothing saved, or a file it can't read: nothing.
        std::fs::write(dir.join(FILE), "not json").unwrap();
        assert!(SavedSignIns::load(&dir).saved().github.is_empty());
        assert!(!SavedSignIns::load(&dir.join("none")).saved().codex);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn one_github_token_needs_no_asking() {
        let s = SavedSignIns { github: vec![gh("you", "ghp_only", None)], ..Default::default() };
        assert_eq!(s.github_for(Some("github.com/you/app")).await.unwrap().token, "ghp_only");
        assert!(SavedSignIns::default().github_for(Some("github.com/you/app")).await.is_none());
        // Not on GitHub: the first.
        let two = SavedSignIns { github: vec![gh("a", "ghp_1", None), gh("b", "ghp_2", None)], ..Default::default() };
        assert_eq!(two.github_for(Some("gitlab.com/you/app")).await.unwrap().token, "ghp_1");
        assert_eq!(two.github_for(None).await.unwrap().token, "ghp_1");
    }

    #[test]
    fn spawns_say_what_they_start_as() {
        assert_eq!(Start::describe(""), "a new machine");
        assert_eq!(Start::describe("base"), "a copy of base");
        let notes = Notes { done: vec!["Claude (the saved token)".into(), "GitHub as @you".into()], problems: vec!["Codex isn't signed in there: no.".into()] };
        assert_eq!(notes.text(), "Signed in there: Claude (the saved token), GitHub as @you. Codex isn't signed in there: no.");
        assert_eq!(Notes::default().text(), "");
    }
}
