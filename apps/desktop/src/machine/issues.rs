//! GitHub issues as threads on the machine. For each project the user turns issues on for, a
//! webhook on its GitHub repo sends issue events to the machine's inbox (`issues.mjs`, in the
//! watchdog) at a public URL of the machine, so a delivery wakes it. An issue that someone who
//! can push there gives the project's label becomes a T3 thread in a worktree and branch of its
//! own, with the project's chosen agent, told to open a pull request; reviews, `/agent` comments
//! and failed CI on that pull request go back to the same thread. The webhook is made with
//! the machine's own gh login, and the secret that signs its deliveries is made on the machine
//! and kept only there and on GitHub.

use super::{provision, MachineRuntime};
use crate::agent37::shell_quote;
pub use crate::store::IssueSettings;
use crate::store::IssueWatch;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};

/// GitHub's longest label name.
const MAX_LABEL: usize = 50;

/// `owner/name`, for a project whose repo key is `github.com/owner/name`.
pub fn github_repo(repo_key: &str) -> Option<String> {
    let mut parts = repo_key.strip_prefix("github.com/")?.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (parts.next().is_none() && !owner.is_empty() && !name.is_empty()).then(|| format!("{owner}/{name}"))
}

/// A model the machine's T3 offers, to work on issues with.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    /// T3's provider instance (`claudeAgent`) and its name (`Claude`).
    pub provider: String,
    pub provider_name: String,
    /// Its slug (`claude-opus-5-5`), name (`Claude Opus 5.5`) and T3's other names for it.
    pub model: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// How hard it thinks, if T3 lets that be chosen.
    pub effort: Option<Effort>,
}

/// A model's reasoning option: its id (`effort`), its levels (id, label), and T3's default.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Effort {
    pub id: String,
    pub levels: Vec<(String, String)>,
    pub default: Option<String>,
}

impl ModelChoice {
    /// Its name without its provider's: "Opus 5.5" for Claude's "Claude Opus 5.5".
    pub fn short_name(&self) -> &str {
        self.name.strip_prefix(&self.provider_name).map(str::trim_start).filter(|n| !n.is_empty()).unwrap_or(&self.name)
    }

    /// A reasoning level it takes: `level` if it's one of its own, else T3's default.
    pub fn level(&self, level: Option<&str>) -> Option<String> {
        let effort = self.effort.as_ref()?;
        level
            .filter(|l| effort.levels.iter().any(|(id, _)| id == l))
            .map(str::to_owned)
            .or_else(|| effort.default.clone())
    }

    /// Settings for working on issues labeled `label` with this model, at `level`.
    pub fn settings(&self, label: &str, level: Option<&str>) -> IssueSettings {
        let options = match (&self.effort, self.level(level)) {
            (Some(effort), Some(level)) => vec![format!("{}={level}", effort.id)],
            _ => Vec::new(),
        };
        IssueSettings { label: label.to_owned(), provider: self.provider.clone(), model: self.model.clone(), options }
    }

    fn is(&self, name: &str) -> bool {
        [&self.model, &self.name].into_iter().chain(&self.aliases).any(|n| n.eq_ignore_ascii_case(name))
    }
}

/// The current models of the machine's signed-in agents, from T3's providers there.
pub fn model_choices(providers: &[Value]) -> Vec<ModelChoice> {
    let mut out = Vec::new();
    for p in providers.iter().filter(|p| p["auth"]["status"] != "unauthenticated") {
        let (Some(provider), Some(provider_name)) = (p["instanceId"].as_str(), p["displayName"].as_str()) else { continue };
        for m in p["models"].as_array().into_iter().flatten().filter(|m| m["isLegacy"] != true) {
            let (Some(model), Some(name)) = (m["slug"].as_str(), m["name"].as_str()) else { continue };
            out.push(ModelChoice {
                provider: provider.into(),
                provider_name: provider_name.into(),
                model: model.into(),
                name: name.into(),
                aliases: m["aliases"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned)).collect(),
                effort: effort_of(m),
            });
        }
    }
    out
}

/// T3's reasoning select for a model, without the levels it only puts in the prompt
/// (Claude's "ultrathink").
fn effort_of(model: &Value) -> Option<Effort> {
    let d = model["capabilities"]["optionDescriptors"].as_array()?.iter().find(|d| {
        d["type"] == "select" && (d["id"].as_str().is_some_and(|id| id.to_ascii_lowercase().contains("effort")) || d["label"] == "Reasoning")
    })?;
    let injected: Vec<&str> = d["promptInjectedValues"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let options = d["options"].as_array()?.iter().filter(|o| o["id"].as_str().is_some_and(|id| !injected.contains(&id)));
    let levels: Vec<(String, String)> = options
        .clone()
        .filter_map(|o| Some((o["id"].as_str()?.to_owned(), o["label"].as_str().or(o["id"].as_str())?.to_owned())))
        .collect();
    let default = options.clone().find(|o| o["isDefault"] == true).and_then(|o| o["id"].as_str()).map(str::to_owned);
    let id = d["id"].as_str()?.to_owned();
    (!levels.is_empty()).then_some(Effort { id, levels, default })
}

impl IssueSettings {
    /// The level of a reasoning option `id`, as `options` holds it.
    pub fn level(&self, id: &str) -> Option<&str> {
        self.options.iter().find_map(|o| o.strip_prefix(id)?.strip_prefix('='))
    }

    /// Its model among `choices`.
    pub fn choice<'a>(&self, choices: &'a [ModelChoice]) -> Option<&'a ModelChoice> {
        choices.iter().find(|c| c.provider == self.provider && c.model == self.model)
    }

    /// "Opus 5.5, high": its model and how hard it thinks, as the machine names them.
    pub fn agent(&self, choices: &[ModelChoice]) -> String {
        let Some(choice) = self.choice(choices) else {
            return self.options.iter().fold(self.model.clone(), |s, o| format!("{s}, {}", o.split_once('=').map_or(o.as_str(), |(_, v)| v)));
        };
        let level = choice.effort.as_ref().and_then(|e| {
            let id = self.level(&e.id)?;
            e.levels.iter().find(|(l, _)| l == id).map(|(_, label)| label.to_lowercase())
        });
        match level {
            Some(level) => format!("{}, {level}", choice.short_name()),
            None => choice.short_name().to_owned(),
        }
    }

    /// These settings with another label, model or level. A model is looked up among `choices`
    /// by its slug, name or T3's other names for it; a level that model doesn't take is refused.
    pub fn with(mut self, choices: &[ModelChoice], label: Option<&str>, model: Option<&str>, level: Option<&str>) -> Result<Self> {
        if let Some(label) = label {
            self.label = label.to_owned();
        }
        self.label = check_label(&self.label)?;
        if model.is_none() && level.is_none() {
            return Ok(self);
        }
        let named = |c: &ModelChoice| format!("{} ({})", c.model, c.provider_name);
        let choice = match model {
            _ if choices.is_empty() => bail!("T3 Code hasn't shown t3agent37 that machine's models yet. Open the machine in T3 Code once, then try again."),
            Some(model) => choices.iter().find(|c| c.is(model)),
            None => self.choice(choices),
        }
        .ok_or_else(|| anyhow!("Pick one of the machine's models: {}", choices.iter().map(named).collect::<Vec<_>>().join(", ")))?;
        let level = match (level, &choice.effort) {
            (Some(level), Some(effort)) => Some(
                effort
                    .levels
                    .iter()
                    .find(|(id, label)| id.eq_ignore_ascii_case(level) || label.eq_ignore_ascii_case(level))
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| anyhow!("{} takes {}", choice.name, effort.levels.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", ")))?,
            ),
            (Some(_), None) => bail!("{} has no reasoning levels to choose from", choice.name),
            // The level it had, if the new model takes it.
            (None, Some(effort)) => self.level(&effort.id).map(str::to_owned),
            (None, None) => None,
        };
        Ok(choice.settings(&self.label, level.as_deref()))
    }
}

/// A label name GitHub takes and the inbox can look up: trimmed, 1 to 50 characters, no comma
/// (GitHub's issue search splits labels on commas).
pub fn check_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        bail!("The label needs a name");
    }
    if label.chars().count() > MAX_LABEL {
        bail!("GitHub labels are {MAX_LABEL} characters at most");
    }
    if label.contains(',') {
        bail!("The label can't have a comma in it");
    }
    Ok(label.to_owned())
}

impl MachineRuntime {
    /// The models its agents offer, as T3 there last listed them. Never wakes it. The recorded
    /// list can lose a provider's models for a while (seen once, for about a minute after T3
    /// re-checked its providers), so the last full list stands in for an empty one.
    pub fn issue_models(&self) -> Vec<ModelChoice> {
        let now = model_choices(&self.cache.lock().unwrap().providers());
        let mut seen = self.models_seen.lock().unwrap();
        if now.is_empty() {
            return seen.clone();
        }
        seen.clone_from(&now);
        now
    }

    /// Turns issue work on or off for one of the machine's projects. On takes `settings`, or
    /// keeps the project's own if it's on already, else the defaults; when it's on already,
    /// that saves them. Wakes the machine: the webhook is made and removed from there.
    pub async fn set_issues(&self, project_id: &str, on: bool, settings: Option<IssueSettings>) -> Result<()> {
        let name = self.record().name;
        let project = self
            .projects()
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or_else(|| anyhow!("that project isn't on {name}"))?;
        let repo = project
            .repo_key
            .as_deref()
            .and_then(github_repo)
            .ok_or_else(|| anyhow!("{} isn't on GitHub", project.title))?;
        // Checked before waking it.
        let settings = match (on, settings.or(project.issues)) {
            (false, _) => None,
            (true, Some(s)) => Some(IssueSettings { label: check_label(&s.label)?, ..s }),
            (true, None) => Some(IssueSettings::default()),
        };
        if on && self.record().github_login.is_none() {
            bail!("Sign {name} in to GitHub first: it makes the webhook and opens the pull requests with that account.");
        }
        self.ensure_awake().await?;
        // Older scripts have no inbox. Updating restarts the watchdog only, never T3, but wait
        // for agents anyway, as keep-awake does.
        if self.record().provision_revision != provision::revision() {
            if self.watchdog_get("/status?fresh").await?.busy {
                bail!("{name} needs a quick update first; try again once its agents finish");
            }
            self.kept_awake(self.upgrade_if_needed()).await?;
        }
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        if let Some(settings) = settings {
            // Asked of Agent37 each time: the webhook must get the URL that works now.
            let url = provision::inbox_url(a37, &id).await?;
            self.update_record(|r| r.inbox_url = Some(url.clone()));
            let agent_name = settings.choice(&self.issue_models()).map(|c| c.name.clone());
            let input = json!({
                "repo": repo,
                "url": url,
                "projectId": project.id,
                "label": settings.label,
                "provider": settings.provider,
                "model": settings.model,
                "options": settings.options,
                "agentName": agent_name,
            });
            let out = self.kept_awake(a37.exec_ok(&id, &command("on", &input), 120)).await?;
            let hook_id = out.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()).and_then(|v| v["hookId"].as_u64());
            self.update_record(|r| {
                r.issues.retain(|w| w.repo != repo);
                r.issues.push(IssueWatch { project_id: project.id.clone(), repo: repo.clone(), hook_id, settings });
            });
        } else {
            self.kept_awake(a37.exec_ok(&id, &command("off", &json!({ "repo": repo })), 60)).await?;
            self.update_record(|r| r.issues.retain(|w| w.repo != repo));
            // The URL stays while webhook tasks use it.
            if let Err(e) = self.sync_inbox_url().await {
                tracing::warn!("{}'s inbox URL: {e:#}", self.id());
            }
        }
        self.shared.changed();
        Ok(())
    }

    /// Before a delete: the webhooks go, so GitHub doesn't keep delivering to a machine that's
    /// gone. Best effort; a webhook left behind only fails its deliveries.
    pub async fn remove_issue_hooks(&self) {
        let watches = self.record().issues;
        if watches.is_empty() {
            return;
        }
        if let Err(e) = self.ensure_awake().await {
            return tracing::warn!("leaving {}'s webhooks on GitHub: {e:#}", self.id());
        }
        for w in watches {
            if let Err(e) = self.shared.a37.exec_ok(&self.sandbox_id(), &command("off", &json!({ "repo": w.repo })), 60).await {
                tracing::warn!("removing {}'s webhook from {}: {e:#}", self.id(), w.repo);
            }
        }
    }
}

/// `issues.mjs` as a command on the machine, its input on stdin.
fn command(verb: &str, input: &Value) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(input.to_string());
    format!("echo {} | base64 -d | node --no-warnings \"$HOME/.t3agent37/issues.mjs\" {verb}", shell_quote(&b64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_repos_take_issues() {
        assert_eq!(github_repo("github.com/octocat/hello-world").as_deref(), Some("octocat/hello-world"));
        assert_eq!(github_repo("gitlab.com/a/b"), None);
        assert_eq!(github_repo("github.com/a"), None);
        assert_eq!(github_repo("github.com/a/b/c"), None);
    }

    /// The shape T3 0.1.12 gives its providers in the server config.
    fn providers() -> Vec<Value> {
        let effort = |default: &str| {
            json!({ "optionDescriptors": [
                { "id": "effort", "label": "Reasoning", "type": "select", "promptInjectedValues": ["ultrathink"], "options": [
                    { "id": "low", "label": "Low" }, { "id": "medium", "label": "Medium", "isDefault": default == "medium" },
                    { "id": "high", "label": "High", "isDefault": default == "high" }, { "id": "max", "label": "Max" },
                    { "id": "ultrathink", "label": "Ultrathink" } ] },
                { "id": "fastMode", "label": "Fast Mode", "type": "boolean" } ] })
        };
        vec![
            json!({ "instanceId": "codex", "displayName": "Codex", "auth": { "status": "unauthenticated" },
                "models": [{ "slug": "gpt-6", "name": "GPT-6", "capabilities": effort("medium") }] }),
            json!({ "instanceId": "claudeAgent", "displayName": "Claude", "auth": { "status": "authenticated" }, "models": [
                { "slug": "claude-opus-5-5", "name": "Claude Opus 5.5", "aliases": ["opus"], "capabilities": effort("medium") },
                { "slug": "claude-sonnet-5-5", "name": "Claude Sonnet 5.5", "aliases": ["sonnet"], "capabilities": effort("high") },
                { "slug": "claude-opus-4-8", "name": "Claude Opus 4.8", "isLegacy": true, "capabilities": effort("high") },
                { "slug": "claude-haiku-4-5", "name": "Claude Haiku 4.5", "capabilities": { "optionDescriptors": [
                    { "id": "thinking", "label": "Thinking", "type": "boolean" } ] } } ] }),
        ]
    }

    #[test]
    fn models_are_the_signed_in_agents_current_ones() {
        let choices = model_choices(&providers());
        let names: Vec<_> = choices.iter().map(ModelChoice::short_name).collect();
        assert_eq!(names, ["Opus 5.5", "Sonnet 5.5", "Haiku 4.5"]);
        let effort = choices[0].effort.as_ref().unwrap();
        assert_eq!(effort.id, "effort");
        assert_eq!(effort.levels.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["low", "medium", "high", "max"]);
        assert_eq!(effort.default.as_deref(), Some("medium"));
        assert_eq!(choices[2].effort, None);
    }

    #[test]
    fn settings_change_within_what_the_machine_offers() {
        let choices = model_choices(&providers());
        let start = IssueSettings::default();
        assert_eq!(start.agent(&choices), "Opus 5.5, high");
        // A new model keeps the level when it takes it.
        let s = start.clone().with(&choices, Some(" claude "), Some("sonnet"), None).unwrap();
        assert_eq!(s, IssueSettings { label: "claude".into(), provider: "claudeAgent".into(), model: "claude-sonnet-5-5".into(), options: vec!["effort=high".into()] });
        let s = s.with(&choices, None, None, Some("Max")).unwrap();
        assert_eq!(s.options, ["effort=max"]);
        assert_eq!(s.agent(&choices), "Sonnet 5.5, max");
        // One without levels drops them; a level it doesn't take is refused.
        let s = s.with(&choices, None, Some("claude-haiku-4-5"), None).unwrap();
        assert!(s.options.is_empty());
        assert!(s.clone().with(&choices, None, None, Some("high")).is_err());
        assert!(start.clone().with(&choices, None, Some("opus"), Some("ultrathink")).is_err());
        // Signed-out agents' and legacy models aren't offered.
        assert!(start.clone().with(&choices, None, Some("gpt-6"), None).is_err());
        assert!(start.clone().with(&choices, None, Some("claude-opus-4-8"), None).is_err());
        // A label alone needs no model list.
        assert_eq!(start.clone().with(&[], Some("todo"), None, None).unwrap().label, "todo");
        assert!(start.with(&[], None, Some("opus"), None).is_err());
    }

    /// The inbox's rules for feedback, the start URL's and links' (`issues.test.mjs`,
    /// `start.test.mjs`, `peers.test.mjs`), when Node is here.
    #[test]
    fn inbox_rules() {
        let run = std::process::Command::new("node")
            .args(["--test", "src/machine/issues.test.mjs", "src/machine/start.test.mjs", "src/machine/peers.test.mjs"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output();
        match run {
            Ok(out) => assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
            Err(e) => eprintln!("node isn't here ({e}): the inbox's tests didn't run"),
        }
    }

    #[test]
    fn labels_github_takes() {
        assert_eq!(check_label("  ready for agent ").unwrap(), "ready for agent");
        assert!(check_label(" ").is_err());
        assert!(check_label("a,b").is_err());
        assert!(check_label(&"x".repeat(51)).is_err());
        assert!(check_label(&"é".repeat(50)).is_ok());
    }

    #[test]
    fn watches_saved_before_settings_get_the_old_ones() {
        let old: IssueWatch = serde_json::from_value(json!({ "projectId": "p", "repo": "a/b", "hookId": 7 })).unwrap();
        assert_eq!(old.settings, IssueSettings::default());
        let new = IssueWatch { settings: IssueSettings { label: "claude".into(), ..Default::default() }, ..old };
        let back: IssueWatch = serde_json::from_value(serde_json::to_value(&new).unwrap()).unwrap();
        assert_eq!(back, new);
    }
}
