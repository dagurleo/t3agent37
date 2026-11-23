//! The machine's T3 projects, and adding one the way T3 Code's "Add to another machine" does:
//! `projectClone.start` on the machine's server, which clones and registers the project and
//! shows its progress in T3 Code, then the env files written in with `projects.writeFile`.

use super::MachineRuntime;
use crate::agent37::shell_quote;
use crate::projects::{self, Project};
use crate::proxy::upstream;
use crate::store::IssueSettings;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Long enough for a large repository on a slow link.
const CLONE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// A setup still going after this goes on unwatched.
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SETUP_SH: &str = include_str!("setup-project.sh");

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineProject {
    /// T3's project id on the machine; while t3boat adds it, the local project's id.
    pub id: String,
    pub title: String,
    pub repo_key: Option<String>,
    /// Its folder on the machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Set while t3boat adds it: how far that is, as T3 Code words clone progress.
    pub status: Option<String>,
    /// Set when its GitHub issues become threads here (`issues.rs`): the label that hands one
    /// over, and the agent that works on it.
    pub issues: Option<IssueSettings>,
}

impl MachineRuntime {
    /// Its projects as last seen: on the machine when it last ran or slept. Never wakes it.
    pub fn projects(&self) -> Vec<MachineProject> {
        let adding = self.adding.lock().unwrap().clone();
        let watched = self.record().issues;
        let mut out: Vec<MachineProject> = self
            .projects
            .lock()
            .unwrap()
            .iter()
            .filter(|p| !adding.iter().any(|a| a.repo_key.is_some() && a.repo_key == p.repo_key))
            .cloned()
            .map(|mut p| {
                p.issues = watched.iter().find(|w| w.project_id == p.id).map(|w| w.settings.clone());
                p
            })
            .collect();
        out.extend(adding);
        out
    }

    pub(super) fn set_projects_from(&self, shell: &Value) {
        let scratch = self.cache.lock().unwrap().scratch_root();
        let projects = projects::live(shell)
            // T3 Code lists its scratch folder as "No project", not as a project.
            .filter(|p| scratch.is_none() || p["workspaceRoot"].as_str() != scratch.as_deref())
            .map(|p| MachineProject {
                id: p["id"].as_str().unwrap_or_default().to_owned(),
                title: p["title"].as_str().unwrap_or_default().to_owned(),
                repo_key: projects::repo_key(p),
                root: p["workspaceRoot"].as_str().map(str::to_owned),
                status: None,
                issues: None,
            })
            .collect();
        *self.projects.lock().unwrap() = projects;
        self.shared.changed();
    }

    /// Reads the projects from the machine if it runs.
    pub async fn refresh_projects(&self) {
        if !self.up_for_background() {
            return;
        }
        match self.shell_snapshot().await {
            Ok(shell) => self.set_projects_from(&shell),
            Err(e) => tracing::warn!("reading projects on {}: {e:#}", self.id()),
        }
    }

    /// Clones `project` from this computer's T3 Code onto the machine and copies `files` (path,
    /// contents) into it. Resolves when the clone is done; the project shows in T3 Code, with
    /// the clone's progress, as soon as it starts.
    pub async fn add_project(&self, project: &Project, files: Vec<(String, String)>) -> Result<Vec<String>> {
        // Already on it (cloned there by hand or by an agent, say): what it lacks of the
        // project's files comes along, and nothing else changes.
        let existing = self.projects.lock().unwrap().iter().find(|p| p.repo_key.as_deref() == Some(&project.repo_key)).cloned();
        if let Some(existing) = existing {
            let root = existing.root.ok_or_else(|| anyhow!("{} is already on {}", project.title, self.record().name))?;
            self.ensure_awake().await?;
            return Ok(self.kept_awake(self.write_files(project, &root, files, true)).await);
        }
        {
            let mut adding = self.adding.lock().unwrap();
            if adding.iter().any(|a| a.repo_key.as_deref() == Some(&project.repo_key)) {
                bail!("{} is already being added", project.title);
            }
            adding.push(MachineProject {
                id: project.id.clone(),
                title: project.title.clone(),
                repo_key: Some(project.repo_key.clone()),
                root: None,
                status: Some("Waking the machine".into()),
                issues: None,
            });
        }
        self.shared.changed();
        let result = async {
            self.ensure_awake().await?;
            self.kept_awake(self.clone_project(project, files)).await
        }
        .await;
        self.adding.lock().unwrap().retain(|a| a.repo_key.as_deref() != Some(&project.repo_key));
        self.refresh_projects().await;
        self.shared.changed();
        result
    }

    /// Returns the files that could not be copied, with their reason.
    async fn clone_project(&self, project: &Project, files: Vec<(String, String)>) -> Result<Vec<String>> {
        let (url, token) = (self.upstream_url(), self.upstream_token());
        let config = upstream::call(&url, &token, "server.getConfig", json!({})).await?;
        let destination = projects::clone_destination(&config["settings"], project);
        // A checkout of it there already, that T3 doesn't know as this repo (a project whose
        // clone failed, then cloned by hand): it lacks only the project's files.
        if self.is_checkout_of(&destination, &project.repo_key).await {
            return Ok(self.write_files(project, &destination, files, true).await);
        }
        let new_id = uuid();
        let started = upstream::call(
            &url,
            &token,
            "projectClone.start",
            json!({
                "projectId": new_id,
                "title": project.title,
                "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "remoteUrl": projects::https_clone_url(&project.remote_url),
                "destinationPath": destination,
            }),
        )
        .await
        .map_err(|e| explain_clone_failure(e, &self.record().name))?;
        // The server clones for the existing project when one already owns the destination.
        let project_id = started["projectId"].as_str().unwrap_or_default().to_owned();
        let cwd = started["cwd"].as_str().unwrap_or_default().to_owned();
        self.set_adding_status(&project.repo_key, "Connecting");

        let (outcome, _) = upstream::subscribe_until(&url, &token, "subscribeProjectClones", json!({}), CLONE_TIMEOUT, |clones| {
            let clone = clones.as_array()?.iter().find(|c| c["projectId"] == project_id.as_str())?;
            match clone["phase"].as_str()? {
                "done" => Some(Ok(())),
                "failed" => Some(Err(clone["error"].as_str().unwrap_or("the clone failed").to_owned())),
                "cancelled" => Some(Err("the clone was cancelled in T3 Code".to_owned())),
                _ => {
                    self.set_adding_status(&project.repo_key, &progress(clone));
                    None
                }
            }
        })
        .await?;
        match outcome {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                // T3 keeps the project it made for the clone, empty: it goes, so a try once the
                // machine can reach the repo starts clean.
                if project_id == new_id {
                    self.remove_empty_project(&project_id, &cwd).await;
                }
                return Err(explain_clone_failure(anyhow::anyhow!(e), &self.record().name));
            }
            None => bail!("T3 Code stopped reporting the clone"),
        }

        let failed = self.write_files(project, &cwd, files, false).await;
        if let Err(e) = self.copy_scripts(project, &project_id).await {
            tracing::warn!("copying {}'s scripts to {}: {e:#}", project.title, self.id());
        }
        if self.shared.settings.read().unwrap().set_up_projects {
            self.set_up(project, &cwd).await;
        }
        Ok(failed)
    }

    /// Writes the project's files (its `.env` files) into its folder `cwd` on the machine;
    /// `missing_only`: those the machine doesn't have yet, so none edited there is overwritten.
    /// Returns those that couldn't be written, with why.
    async fn write_files(&self, project: &Project, cwd: &str, files: Vec<(String, String)>, missing_only: bool) -> Vec<String> {
        let (url, token) = (self.upstream_url(), self.upstream_token());
        let relative = |path: &str| if project.subdirectory.is_empty() { path.to_owned() } else { format!("{}/{path}", project.subdirectory) };
        let present: Vec<String> = if missing_only && !files.is_empty() {
            let checks: String = files.iter().map(|(p, _)| format!("[ -e {} ] && echo {};\n", shell_quote(&relative(p)), shell_quote(p))).collect();
            let script = format!("cd {} || exit 0\n{checks}true", home_path(cwd));
            match self.shared.a37.exec_ok(&self.sandbox_id(), &script, 30).await {
                Ok(out) => out.lines().map(str::to_owned).collect(),
                // Can't tell: nothing is written over.
                Err(e) => return vec![format!("checking which files {} has: {e:#}", self.record().name)],
            }
        } else {
            Vec::new()
        };
        let mut failed = Vec::new();
        for (path, contents) in files.into_iter().filter(|(p, _)| !present.contains(p)) {
            let written = upstream::call(&url, &token, "projects.writeFile", json!({ "cwd": cwd, "relativePath": relative(&path), "contents": contents })).await;
            if let Err(e) = written {
                failed.push(format!("{path}: {e:#}"));
            }
        }
        failed
    }

    /// Whether `folder` on the machine is a git checkout of the repo `repo_key`
    /// (`github.com/owner/name`).
    async fn is_checkout_of(&self, folder: &str, repo_key: &str) -> bool {
        let script = format!("git -C {} remote get-url origin 2>/dev/null", home_path(folder));
        match self.shared.a37.exec(&self.sandbox_id(), &script, 30).await {
            Ok(r) if r.exit_code == Some(0) => repo_key_of(r.stdout.trim()).is_some_and(|k| k.eq_ignore_ascii_case(repo_key)),
            _ => false,
        }
    }

    /// Takes a project whose clone failed off the machine: T3's record of it (`t3 project
    /// remove`, which refuses one with threads) and its folder if it's empty. Best effort.
    async fn remove_empty_project(&self, project_id: &str, cwd: &str) {
        let mut script = format!("t3 project remove {}", shell_quote(project_id));
        if !cwd.is_empty() {
            script.push_str(&format!(" && rmdir {} 2>/dev/null; true", home_path(cwd)));
        }
        if let Err(e) = self.shared.a37.exec_ok(&self.sandbox_id(), &script, 60).await {
            tracing::warn!("removing the empty project {project_id} from {}: {e:#}", self.id());
        }
    }

    /// The project's T3 scripts (Run, Test, its setup...) onto the machine's copy, which T3's
    /// clone leaves without them; T3 there then runs its setup ones for each new worktree too.
    /// Clients can't change a project's own scripts, so they go where T3 Code's script editor
    /// puts them: the server's per-project overrides. A patch merges by project, so others stay.
    async fn copy_scripts(&self, project: &Project, project_id: &str) -> Result<()> {
        if project.scripts.is_empty() || project_id.is_empty() {
            return Ok(());
        }
        let (url, token) = (self.upstream_url(), self.upstream_token());
        let shell = self.shell_snapshot().await?;
        let own = projects::live(&shell).find(|p| p["id"] == project_id).map(|p| p["scripts"].clone());
        let settings = upstream::call(&url, &token, "server.getSettings", json!({})).await?;
        let overridden = settings["projectScriptOverrides"][project_id].is_array();
        if overridden || own.as_ref().and_then(Value::as_array).is_some_and(|s| !s.is_empty()) {
            return Ok(());
        }
        let patch = json!({ "patch": { "projectScriptOverrides": { project_id: project.scripts } } });
        upstream::call(&url, &token, "server.updateSettings", patch).await?;
        Ok(())
    }

    /// Runs the project's setup on the machine, in its folder (`setup-project.sh`): its T3
    /// setup scripts, or its dependencies from its lockfiles. Progress shows on its row; a
    /// failure is a notification, never a failed add: the project is there either way.
    async fn set_up(&self, project: &Project, cwd: &str) {
        let scripts: Vec<String> = project
            .scripts
            .iter()
            .filter(|s| s["runOnWorktreeCreate"] == true)
            .filter_map(|s| {
                let command = s["command"].as_str()?.replace(['\n', '\t'], " ");
                let name = s["name"].as_str().unwrap_or("setup").replace(['\n', '\t'], " ");
                Some(format!("{name}\t{command}"))
            })
            .collect();
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        let script = format!(
            "d=$HOME/.t3agent37/setup; mkdir -p \"$d\"; f=\"$d/$(date +%s)-$RANDOM\"; \
             echo {} | base64 -d > \"$f.sh\"; echo {} | base64 -d > \"$f.scripts\"; \
             cd {} || exit 1; \
             (bash \"$f.sh\" \"$f.scripts\" > \"$f.log\" 2>&1; echo $? > \"$f.exit\") </dev/null >/dev/null 2>&1 & echo \"$f\"",
            shell_quote(&b64(SETUP_SH)),
            shell_quote(&b64(&scripts.join("\n"))),
            shell_quote(cwd),
        );
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        let started = match a37.exec_ok(&id, &script, 30).await {
            Ok(out) => out.lines().last().unwrap_or_default().trim().to_owned(),
            Err(e) => return tracing::warn!("setting up {} on {}: {e:#}", project.title, self.id()),
        };
        self.set_adding_status(&project.repo_key, "Setting up");
        let outcome = self.kept_awake(self.wait_set_up(&project.repo_key, &started)).await;
        let name = self.record().name;
        match outcome {
            Ok(None) => {}
            Ok(Some(tail)) => self.shared.notify(format!("Setting up {} on {name} failed", project.title), tail),
            Err(e) => self.shared.notify(format!("Setting up {} on {name}", project.title), format!("{e:#}")),
        }
    }

    /// Follows a setup until it ends: None when it went fine, else its last lines.
    async fn wait_set_up(&self, repo_key: &str, run: &str) -> Result<Option<String>> {
        let started = Instant::now();
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let out = a37
                .exec(&id, &format!("cat '{run}.exit' 2>/dev/null; echo ---; grep '^t3agent37-step: ' '{run}.log' | tail -n 1; echo ---; tail -n 12 '{run}.log'"), 30)
                .await?;
            let mut parts = out.stdout.splitn(3, "---\n");
            let (exit, step, tail) = (parts.next().unwrap_or_default().trim(), parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
            if let Some(step) = step.trim().strip_prefix("t3agent37-step: ") {
                self.set_adding_status(repo_key, &format!("Setting up · {step}"));
            }
            match exit {
                "" => {}
                "0" | "3" => return Ok(None),
                _ => {
                    let tail: String = tail.lines().filter(|l| !l.starts_with("t3agent37-step: ")).collect::<Vec<_>>().join("\n");
                    let tail = tail.trim();
                    let cut = tail.char_indices().rev().nth(399).map_or(0, |(i, _)| i);
                    return Ok(Some(tail[cut..].to_owned()));
                }
            }
            if started.elapsed() > SETUP_TIMEOUT {
                bail!("still running after {} minutes; it goes on in the background (log: {run}.log)", SETUP_TIMEOUT.as_secs() / 60);
            }
        }
    }

    fn set_adding_status(&self, repo_key: &str, status: &str) {
        if let Some(a) = self.adding.lock().unwrap().iter_mut().find(|a| a.repo_key.as_deref() == Some(repo_key)) {
            a.status = Some(status.to_owned());
        }
        self.shared.changed();
    }
}

/// T3's `projectCloneProgressSummary`: "Receiving objects · 42% · 1.2 MiB/s".
fn progress(clone: &Value) -> String {
    let stage = match clone["stage"].as_str().unwrap_or_default() {
        "counting" => "Counting objects",
        "receiving" => "Receiving objects",
        "resolving" => "Resolving deltas",
        "checkout" => "Checking out files",
        _ => "Connecting",
    };
    let mut parts = vec![stage.to_owned()];
    if let Some(percent) = clone["percent"].as_u64() {
        parts.push(format!("{percent}%"));
    }
    if clone["stalled"] == true {
        parts.push("Stalled".into());
    } else if let Some(detail) = clone["detail"].as_str().filter(|d| !d.is_empty()) {
        parts.push(detail.to_owned());
    }
    parts.join(" · ")
}

/// A clone the git host turned away: the machine's git uses its own gh login, which t3agent37
/// signs in per machine.
fn explain_clone_failure(e: anyhow::Error, machine: &str) -> anyhow::Error {
    let text = format!("{e:#}");
    let auth = ["Authentication failed", "could not read Username", "Permission denied", "Repository not found", "terminal prompts disabled"]
        .iter()
        .any(|s| text.contains(s));
    if auth {
        anyhow::anyhow!("{text}\n{machine} isn't signed in to a GitHub account with access to this repo: sign in to GitHub in its details.")
    } else {
        e
    }
}

/// A folder for the machine's shell: `~/x` as `"$HOME"/'x'`, anything else quoted.
fn home_path(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("\"$HOME\"/{}", shell_quote(rest)),
        None if path == "~" => "\"$HOME\"".to_owned(),
        None => shell_quote(path),
    }
}

/// `github.com/owner/name` for a git remote URL, in any of its forms.
fn repo_key_of(remote: &str) -> Option<String> {
    let https = projects::https_clone_url(remote);
    let rest = https.split_once("://").map_or(https.as_str(), |(_, r)| r);
    let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
    let key = rest.trim_end_matches('/').trim_end_matches(".git").to_lowercase();
    (key.matches('/').count() >= 2).then_some(key)
}

/// A random v4 UUID, the form of T3's project ids.
fn uuid() -> String {
    let b: [u8; 16] = rand::random();
    let hex = |r: std::ops::Range<usize>| b[r].iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-4{}-{:x}{}-{}",
        hex(0..4),
        hex(4..6),
        &hex(6..8)[1..],
        8 | (b[8] & 0x3),
        &hex(8..10)[1..],
        hex(10..16)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_keys_from_remotes_and_paths_for_the_shell() {
        for remote in [
            "https://github.com/Faktor-LLC/faktor-client.git",
            "https://github.com/Faktor-LLC/faktor-client",
            "git@github.com:Faktor-LLC/faktor-client.git",
            "ssh://git@github.com/Faktor-LLC/faktor-client.git",
            "https://x-access-token@github.com/Faktor-LLC/faktor-client.git",
        ] {
            assert_eq!(repo_key_of(remote).as_deref(), Some("github.com/faktor-llc/faktor-client"), "{remote}");
        }
        assert_eq!(repo_key_of(""), None);
        assert_eq!(home_path("~/faktor-client"), "\"$HOME\"/'faktor-client'");
        assert_eq!(home_path("/root/it's"), "'/root/it'\\''s'");
    }

    #[test]
    fn uuids_are_v4() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')));
    }

    #[test]
    fn words_progress_like_t3() {
        let c = serde_json::json!({"stage": "receiving", "percent": 42, "detail": "1.2 MiB/s", "stalled": false});
        assert_eq!(progress(&c), "Receiving objects · 42% · 1.2 MiB/s");
    }
}
