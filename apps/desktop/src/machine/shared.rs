//! A machine used from several computers, each with t3agent37 on the same Agent37 account. What
//! the machine does is the same for all of them; what each app knows about it comes from the
//! machine itself: its GitHub issue settings and its start URL are read there whenever it comes
//! up (`refresh_settings`), so a change made on one computer shows on the others.
//!
//! A machine made on another computer shows here as one to add (`App::elsewhere`). Adding it
//! (`adopt`) wakes it once and gives this computer a T3 sign-in of its own there, next to the
//! other computer's. The watchdog's key is read from the machine rather than made anew, which
//! would lock the other computer out of it.

use super::{provision, MachineRuntime};
use crate::agent37::{shell_quote, Agent37, Instance};
use crate::store::{IssueSettings, IssueWatch, Machine, StartLink};
use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

/// Agent37 names the instances t3agent37 makes this way (`provision::instance_name`).
pub const INSTANCE_PREFIX: &str = "t3agent37 · ";

/// The machine's name, for an instance t3agent37 made; None for any other.
pub fn machine_name(instance: &Instance) -> Option<String> {
    let name = instance.name.as_deref()?.strip_prefix(INSTANCE_PREFIX)?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Its size by its resources; None for a shape no size has.
pub fn size_of(instance: &Instance) -> Option<&'static str> {
    let r = &instance.resources;
    provision::SIZES.iter().find(|s| s.cpu == r.cpu && s.memory == r.memory).map(|s| s.id)
}

/// What another computer's machine tells this one as it's added.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adopted {
    /// The watchdog's key, shared by every computer that uses the machine.
    pub secret: String,
    pub name: Option<String>,
    /// This computer's own T3 sign-in there.
    pub upstream_token: String,
    pub environment_id: String,
    pub t3_version: Option<String>,
}

/// On a running machine: a T3 sign-in for this computer, and what it needs to run the machine.
/// Fails for a machine still being made, which has no watchdog yet.
pub async fn adopt(a37: &Agent37, instance: &str) -> Result<Adopted> {
    let label = shell_quote("t3agent37 (another computer)");
    let script = format!(
        r#"set -e
d="$HOME/.t3agent37"
[ -f "$d/config.json" ] || {{ echo '{{"notReady":true}}'; exit 0; }}
for _ in $(seq 1 60); do curl -sf -m 5 -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment && break; sleep 1; done
token="$("$HOME/.local/bin/t3" auth session issue --token-only --ttl 3650d --label {label})"
descriptor="$(curl -sf -m 5 http://127.0.0.1:3773/.well-known/t3/environment)"
TOKEN="$token" DESCRIPTOR="$descriptor" node -e '
  const fs = require("fs"), d = process.env.HOME + "/.t3agent37";
  const read = (f) => {{ try {{ return fs.readFileSync(d + "/" + f, "utf8").trim(); }} catch {{ return null; }} }};
  const descriptor = JSON.parse(process.env.DESCRIPTOR);
  console.log(JSON.stringify({{
    secret: JSON.parse(read("config.json")).secret,
    name: read("name"),
    upstreamToken: process.env.TOKEN,
    environmentId: descriptor.environmentId,
    t3Version: descriptor.serverVersion ?? null,
  }}));
'
"#
    );
    let out = a37.exec_ok(instance, &script, 120).await?;
    let line = out.lines().rev().find(|l| l.starts_with('{')).ok_or_else(|| anyhow!("the machine said nothing"))?;
    if line.contains("\"notReady\"") {
        bail!("it's still being set up on the other computer; try again in a minute");
    }
    let adopted: Adopted = serde_json::from_str(line)?;
    if adopted.secret.is_empty() || adopted.upstream_token.is_empty() {
        bail!("the machine's setup is incomplete");
    }
    Ok(adopted)
}

/// The settings a machine keeps for every computer: its repos' issue settings (webhook secrets
/// left out) and its start URL's token.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MachineSettings {
    #[serde(default)]
    pub repos: Vec<RepoSettings>,
    pub start_token: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoSettings {
    pub repo: String,
    pub project_id: String,
    pub hook_id: Option<u64>,
    /// Missing on machines set up before they could be chosen: the defaults then.
    pub label: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
}

const SETTINGS_SCRIPT: &str = r#"node -e '
  const fs = require("fs"), d = process.env.HOME + "/.t3agent37";
  const read = (f) => { try { return JSON.parse(fs.readFileSync(d + "/" + f, "utf8")); } catch { return null; } };
  const repos = Object.entries(read("issues.json")?.repos ?? {}).map(([repo, w]) => ({
    repo, projectId: w.projectId, hookId: w.hookId ?? null, label: w.label ?? null,
    provider: w.provider ?? null, model: w.model ?? null, options: w.options ?? null,
  }));
  console.log(JSON.stringify({ repos, startToken: read("start.json")?.token ?? null }));
'"#;

/// The record with the machine's own settings: what it does is what the app shows.
pub fn apply(record: &mut Machine, settings: MachineSettings) {
    let default = IssueSettings::default();
    record.issues = settings
        .repos
        .into_iter()
        .map(|w| IssueWatch {
            project_id: w.project_id,
            repo: w.repo,
            hook_id: w.hook_id,
            settings: match (w.provider, w.model) {
                (Some(provider), Some(model)) => IssueSettings {
                    label: w.label.unwrap_or_else(|| default.label.clone()),
                    provider,
                    model,
                    options: w.options.unwrap_or_default(),
                },
                _ => IssueSettings { label: w.label.unwrap_or_else(|| default.label.clone()), ..default.clone() },
            },
        })
        .collect();
    let sent = |token: &str| record.start.as_ref().filter(|s| s.token == token).map_or(0, |s| s.sent);
    record.start = settings.start_token.map(|token| StartLink { sent: sent(&token), token });
}

/// A record for a machine made on another computer, as it's added here.
pub fn record(instance: &Instance, name: &str, adopted: Adopted, heartbeat_url: String, local_port: u16) -> Machine {
    let (upstream_url, watchdog_url) = provision::urls(&instance.id);
    Machine {
        id: instance.id.clone(),
        sandbox_id: instance.id.clone(),
        name: name.to_owned(),
        size: size_of(instance).unwrap_or("default").to_owned(),
        upstream_url,
        upstream_token: adopted.upstream_token,
        watchdog_url,
        watchdog_secret: adopted.secret,
        heartbeat_url,
        local_port,
        t3_version: adopted.t3_version.unwrap_or_default(),
        // Unknown: its first wake from here provisions it, which leaves a newer app's scripts be.
        provision_revision: String::new(),
        environment_id: adopted.environment_id,
        client_tokens: Vec::new(),
        link_tokens: Vec::new(),
        created_at: instance.created.unwrap_or_else(|| chrono::Utc::now().timestamp()),
        next_due_at: None,
        connect: false,
        github_login: None,
        github_checked: false,
        github_reach: None,
        github_token_expires: None,
        connect_approval: None,
        connect_push: None,
        previews: Vec::new(),
        issues: Vec::new(),
        usage: None,
        inbox_url: instance.public_ports.iter().find(|p| p.port == provision::INBOX_PORT).map(|p| p.url.clone()),
        hooks: Vec::new(),
        start: None,
    }
}

impl MachineRuntime {
    /// Reads the settings the machine keeps for every computer (another one may have changed
    /// them) into the record. Only while it runs: never wakes it.
    pub async fn refresh_settings(&self) {
        if !self.up_for_background() {
            return;
        }
        match self.read_settings().await {
            Ok(settings) => self.use_settings(settings).await,
            Err(e) => tracing::debug!("reading {}'s settings: {e:#}", self.id()),
        }
    }

    pub(crate) async fn read_settings(&self) -> Result<MachineSettings> {
        let out = self.shared.a37.exec_ok(&self.sandbox_id(), SETTINGS_SCRIPT, 30).await?;
        let line = out.lines().rev().find(|l| l.starts_with('{')).ok_or_else(|| anyhow!("no settings"))?;
        Ok(serde_json::from_str(line)?)
    }

    pub(crate) async fn use_settings(&self, settings: MachineSettings) {
        let before = self.record();
        let mut after = before.clone();
        apply(&mut after, settings);
        let needs_inbox = !after.issues.is_empty() || after.start.is_some();
        if after.issues != before.issues || after.start != before.start {
            self.update_record(|r| {
                r.issues = after.issues.clone();
                r.start = after.start.clone();
            });
            self.shared.changed();
        }
        // Another computer may have made the inbox's URL.
        if needs_inbox && before.inbox_url.is_none() {
            match provision::inbox_url(&self.shared.a37, &self.sandbox_id()).await {
                Ok(url) => {
                    self.update_record(|r| r.inbox_url = Some(url));
                    self.shared.changed();
                }
                Err(e) => tracing::warn!("{}'s inbox URL: {e:#}", self.id()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn instance(name: &str, cpu: u32, memory: u32) -> Instance {
        serde_json::from_value(json!({
            "id": "ab12cd34ef", "status": "sleeping", "name": name, "created": 1795011458,
            "resources": { "cpu": cpu, "memory": memory, "disk": 20 },
            "public_ports": [{ "port": 3775, "url": "https://hb.agent37.app" }, { "port": 3777, "url": "https://inbox.agent37.app" }],
        }))
        .unwrap()
    }

    #[test]
    fn only_t3agent37_instances_are_machines() {
        assert_eq!(machine_name(&instance("t3agent37 · skiff", 4, 16)).as_deref(), Some("skiff"));
        assert_eq!(machine_name(&instance("tantalum-web", 4, 16)), None);
        assert_eq!(machine_name(&instance("t3agent37 · ", 4, 16)), None);
        assert_eq!(size_of(&instance("t3agent37 · skiff", 4, 16)), Some("large"));
        assert_eq!(size_of(&instance("t3agent37 · skiff", 3, 16)), None);
    }

    #[test]
    fn a_machine_from_elsewhere_gets_its_own_record() {
        let adopted = Adopted {
            secret: "s".into(),
            name: Some("skiff".into()),
            upstream_token: "t".into(),
            environment_id: "env".into(),
            t3_version: Some("0.1.12".into()),
        };
        let r = record(&instance("t3agent37 · skiff", 4, 16), "skiff", adopted, "https://hb.agent37.app".into(), 47380);
        assert_eq!((r.size.as_str(), r.local_port, r.watchdog_secret.as_str()), ("large", 47380, "s"));
        assert_eq!(r.inbox_url.as_deref(), Some("https://inbox.agent37.app"));
        assert_eq!(r.created_at, 1795011458);
        assert!(r.provision_revision.is_empty() && r.client_tokens.is_empty());
    }

    #[test]
    fn the_machine_settings_win() {
        let adopted = Adopted { secret: "s".into(), name: None, upstream_token: "t".into(), environment_id: "e".into(), t3_version: None };
        let mut r = record(&instance("t3agent37 · skiff", 4, 16), "skiff", adopted, String::new(), 47380);
        r.start = Some(StartLink { token: "old".into(), sent: 7 });
        let settings: MachineSettings = serde_json::from_value(json!({
            "repos": [
                { "repo": "a/b", "projectId": "p1", "hookId": 5, "label": "claude", "provider": "claudeAgent", "model": "claude-sonnet-5-5", "options": ["effort=low"] },
                // Set up before settings could be chosen.
                { "repo": "c/d", "projectId": "p2", "hookId": null, "label": null, "provider": null, "model": null, "options": null },
            ],
            "startToken": "old",
        }))
        .unwrap();
        apply(&mut r, settings);
        assert_eq!(r.issues[0].settings, IssueSettings { label: "claude".into(), provider: "claudeAgent".into(), model: "claude-sonnet-5-5".into(), options: vec!["effort=low".into()] });
        assert_eq!(r.issues[1].settings, IssueSettings::default());
        // The same token keeps what was sent with it; another starts over; none is off.
        assert_eq!(r.start, Some(StartLink { token: "old".into(), sent: 7 }));
        apply(&mut r, MachineSettings { repos: Vec::new(), start_token: Some("new".into()) });
        assert_eq!((r.start.as_ref().unwrap().sent, r.issues.len()), (0, 0));
        apply(&mut r, MachineSettings::default());
        assert_eq!(r.start, None);
    }
}
