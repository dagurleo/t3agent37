//! The machine's start URL: a path on its inbox's public URL (`start.mjs`, served by the inbox in
//! the watchdog) that starts agent threads in any of its projects, reports on them and takes
//! follow-ups, over plain HTTPS. It's for an assistant such as Grok, a phone shortcut or a script:
//! anything that can send a web request, with no T3 sign-in. A request to it wakes a sleeping
//! machine. The token in the URL is its only key, so the URL is a secret: whoever has it acts as
//! the user. Replacing it retires the old one.
//!
//! The machine resolves requests itself, while this app may be closed, so the app writes it the
//! settings it needs: the token, the inbox's URL, the machine's models (as T3 there last listed
//! them) and the model for a project with no default of its own. They go again whenever they
//! change, when the machine comes up.

use super::issues::ModelChoice;
use super::{provision, MachineRuntime, Power};
use crate::agent37::shell_quote;
use crate::proxy::auth::random_token;
use crate::store::{IssueSettings, StartLink};
use anyhow::{bail, Result};
use base64::Engine;
use serde_json::{json, Value};

/// The start URL's path on the inbox, before its token.
pub const START_PATH: &str = "/start/";

/// `<inbox URL>/start/<token>`.
pub fn start_url(inbox_url: &str, token: &str) -> String {
    format!("{}{START_PATH}{token}", inbox_url.trim_end_matches('/'))
}

/// `<inbox URL>/start`: the start URL without its token, which then goes in an
/// `Authorization: Bearer` header.
pub fn start_address(inbox_url: &str) -> String {
    format!("{}/start", inbox_url.trim_end_matches('/'))
}

/// The model for a thread in a project with no default model of its own: Claude Opus 5.5 at high
/// effort, as for issues, when the machine offers it; else its first model at T3's default level.
pub fn default_agent(choices: &[ModelChoice]) -> Option<IssueSettings> {
    let best = IssueSettings::default();
    if best.choice(choices).is_some() {
        return Some(best);
    }
    choices.first().map(|c| c.settings("", None))
}

/// What `start.mjs on` takes.
fn settings(token: &str, url: &str, choices: &[ModelChoice]) -> Value {
    let agent = default_agent(choices).map(|s| json!({ "provider": s.provider, "model": s.model, "options": s.options }));
    let models: Vec<Value> = choices
        .iter()
        .map(|c| {
            json!({
                "provider": c.provider,
                "providerName": c.provider_name,
                "model": c.model,
                "name": c.name,
                "aliases": c.aliases,
                "effort": c.effort.as_ref().map(|e| json!({ "id": e.id, "levels": e.levels, "default": e.default })),
            })
        })
        .collect();
    json!({ "token": token, "url": url, "agent": agent, "models": models })
}

fn fingerprint(settings: &Value) -> u64 {
    provision::fnv1a(settings.to_string().bytes())
}

impl MachineRuntime {
    /// Its start URL, while it's on.
    pub fn start_url(&self) -> Option<String> {
        let r = self.record();
        Some(start_url(r.inbox_url.as_deref()?, &r.start?.token))
    }

    /// Its start URL without the token, and the token, while it's on.
    pub fn start_parts(&self) -> Option<(String, String)> {
        let r = self.record();
        Some((start_address(r.inbox_url.as_deref()?), r.start?.token))
    }

    /// The model its start URL uses for a project with none of its own: "Opus 5.5, high".
    pub fn start_agent(&self) -> Option<String> {
        let choices = self.issue_models();
        default_agent(&choices).map(|s| s.agent(&choices))
    }

    /// Turns the start URL on and returns it, or off. On again keeps the URL; `replace` gives it
    /// a new token, and the old URL stops working at once. Wakes the machine.
    pub async fn set_start(&self, on: bool, replace: bool) -> Result<Option<String>> {
        let name = self.record().name;
        self.ensure_awake().await?;
        // Older scripts have no start URL. Updating restarts the watchdog only, never T3, but
        // wait for agents anyway, as keep-awake does.
        if self.record().provision_revision != provision::revision() {
            if self.watchdog_get("/status?fresh").await?.busy {
                bail!("{name} needs a quick update first; try again once its agents finish");
            }
            self.kept_awake(self.upgrade_if_needed()).await?;
        }
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        if !on {
            self.kept_awake(a37.exec_ok(&id, &command("off", &json!({})), 60)).await?;
            self.update_record(|r| r.start = None);
            // The inbox's URL stays while issues or webhook tasks use it.
            if let Err(e) = self.sync_inbox_url().await {
                tracing::warn!("{}'s inbox URL: {e:#}", self.id());
            }
            self.shared.changed();
            return Ok(None);
        }
        let token = match self.record().start {
            Some(s) if !replace => s.token,
            _ => random_token(),
        };
        let url = provision::inbox_url(a37, &id).await?;
        self.update_record(|r| r.inbox_url = Some(url.clone()));
        let input = settings(&token, &url, &self.issue_models());
        self.kept_awake(a37.exec_ok(&id, &command("on", &input), 60)).await?;
        let sent = fingerprint(&input);
        self.update_record(|r| r.start = Some(StartLink { token: token.clone(), sent }));
        self.shared.changed();
        Ok(Some(start_url(&url, &token)))
    }

    /// When it comes up: the start URL's settings again, if its models or inbox URL changed.
    /// Never wakes it.
    pub async fn refresh_start(&self) {
        let r = self.record();
        let (Some(start), Some(url)) = (r.start, r.inbox_url) else { return };
        let choices = self.issue_models();
        // Not known yet: what the machine has stands.
        if self.power() != Power::Running || choices.is_empty() {
            return;
        }
        let input = settings(&start.token, &url, &choices);
        let sent = fingerprint(&input);
        if sent == start.sent {
            return;
        }
        match self.shared.a37.exec_ok(&self.sandbox_id(), &command("on", &input), 60).await {
            Ok(_) => self.update_record(|r| {
                if let Some(s) = r.start.as_mut() {
                    s.sent = sent;
                }
            }),
            Err(e) => tracing::warn!("updating {}'s start URL settings: {e:#}", self.id()),
        }
    }
}

/// `start.mjs` as a command on the machine, its input on stdin.
fn command(verb: &str, input: &Value) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(input.to_string());
    format!("echo {} | base64 -d | node --no-warnings \"$HOME/.t3agent37/start.mjs\" {verb}", shell_quote(&b64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::issues::{model_choices, Effort};

    fn choice(provider: &str, model: &str, name: &str, effort: bool) -> ModelChoice {
        ModelChoice {
            provider: provider.into(),
            provider_name: if provider == "codex" { "Codex".into() } else { "Claude".into() },
            model: model.into(),
            name: name.into(),
            aliases: Vec::new(),
            effort: effort.then(|| Effort { id: "effort".into(), levels: vec![("low".into(), "Low".into()), ("high".into(), "High".into())], default: Some("low".into()) }),
        }
    }

    #[test]
    fn the_default_model_is_opus_at_high_when_offered() {
        let opus = choice("claudeAgent", "claude-opus-5-5", "Claude Opus 5.5", true);
        let gpt = choice("codex", "gpt-6", "GPT-6", true);
        let both = [gpt.clone(), opus];
        assert_eq!(default_agent(&both).unwrap().agent(&both), "Opus 5.5, high");
        // Codex alone: its first model, at T3's default level.
        let codex = [gpt];
        let agent = default_agent(&codex).unwrap();
        assert_eq!((agent.model.as_str(), agent.options.as_slice()), ("gpt-6", ["effort=low".to_owned()].as_slice()));
        assert!(default_agent(&model_choices(&[])).is_none());
    }

    #[test]
    fn settings_for_the_machine() {
        let choices = [choice("claudeAgent", "claude-opus-5-5", "Claude Opus 5.5", true), choice("claudeAgent", "claude-haiku-4-5", "Claude Haiku 4.5", false)];
        let s = settings("tok", "https://abc.agent37.app", &choices);
        assert_eq!(s["agent"], json!({ "provider": "claudeAgent", "model": "claude-opus-5-5", "options": ["effort=high"] }));
        assert_eq!(s["models"][0]["effort"]["levels"], json!([["low", "Low"], ["high", "High"]]));
        assert_eq!(s["models"][1]["effort"], Value::Null);
        assert_eq!(start_url("https://abc.agent37.app/", "tok"), "https://abc.agent37.app/start/tok");
        assert_eq!(start_address("https://abc.agent37.app/"), "https://abc.agent37.app/start");
        // A change in the models is a change to send.
        assert_ne!(fingerprint(&s), fingerprint(&settings("tok", "https://abc.agent37.app", &choices[..1])));
    }
}
