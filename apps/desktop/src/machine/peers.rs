//! Machines that use each other: T3's linked environments between the user's machines, as the
//! user allows each pair. A link from one machine to another lets the first one's agents work on
//! the second with T3's own tools (`delegate_task`, the thread tools), with T3's access level
//! capping what they may do there: say, run a benchmark on a bigger machine. T3 makes the link
//! itself (`t3 environment link` with a pairing code from `t3 pair`); t3agent37 makes and
//! removes them, gives the machine linked to a way in that wakes it (its inbox, `peers.mjs`),
//! and makes each again before its 30 days run out.
//!
//! What the machines record is the truth: the links in the linking machine's T3, and whether the
//! other takes links (`peers.json`), read with its other settings (`shared.rs`), so every
//! computer shows the same. Only the user changes them, in the panel or the command: agents see
//! the links (`list_machines`), and can't make one.

use super::{MachineRuntime, provision};
use crate::agent37::shell_quote;
use crate::app::App;
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::sync::Arc;

/// What a machine's agents may do on one they use: T3's access levels, from least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Access {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "approval-required")]
    ApprovalRequired,
    #[serde(rename = "auto-accept-edits")]
    AutoAcceptEdits,
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "full-access")]
    FullAccess,
}

impl Access {
    pub const ALL: [Access; 5] = [Access::ReadOnly, Access::ApprovalRequired, Access::AutoAcceptEdits, Access::Auto, Access::FullAccess];

    /// T3's name for it, as `t3 environment link --access` takes it.
    pub fn t3(self) -> &'static str {
        match self {
            Access::ReadOnly => "read-only",
            Access::ApprovalRequired => "approval-required",
            Access::AutoAcceptEdits => "auto-accept-edits",
            Access::Auto => "auto",
            Access::FullAccess => "full-access",
        }
    }

    /// A word for the panel and the command.
    pub fn short(self) -> &'static str {
        match self {
            Access::ReadOnly => "read",
            Access::ApprovalRequired => "ask",
            Access::AutoAcceptEdits => "edits",
            Access::Auto => "auto",
            Access::FullAccess => "full",
        }
    }

    /// What it lets the other machine's agents do, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Access::ReadOnly => "read its threads, start none",
            Access::ApprovalRequired => "start threads that ask you before each change",
            Access::AutoAcceptEdits => "start threads that edit files, asking before commands",
            Access::Auto => "start threads in T3's auto mode",
            Access::FullAccess => "start threads that run anything",
        }
    }

    /// T3's name, or a short word, or a few aliases.
    pub fn parse(s: &str) -> Option<Access> {
        let s = s.trim().to_ascii_lowercase();
        Access::ALL.into_iter().find(|a| a.t3() == s || a.short() == s).or(match s.as_str() {
            "read" | "readonly" => Some(Access::ReadOnly),
            "approval" | "approve" => Some(Access::ApprovalRequired),
            "full" => Some(Access::FullAccess),
            _ => None,
        })
    }
}

/// A link's session on the machine it reaches is labeled like this: T3 names it after the
/// linking environment, which is the machine's name.
fn session_label(linker: &str) -> String {
    format!("T3 Code · {linker}")
}

/// Turns T3's "linked environments" on in its settings file (Settings → Labs), which T3 reads
/// again on its own. Both ends need it.
const LINKS_ON: &str = r#"node -e '
  const fs = require("fs"), p = process.env.HOME + "/.t3/userdata/settings.json";
  let d = {}, mode = 0o600;
  try { d = JSON.parse(fs.readFileSync(p, "utf8")); mode = fs.statSync(p).mode & 0o777; } catch {}
  if (d.linkedEnvironmentsEnabled !== true) {
    d.linkedEnvironmentsEnabled = true;
    fs.writeFileSync(p + ".t3agent37", JSON.stringify(d, null, 2), { mode });
    fs.renameSync(p + ".t3agent37", p);
  }
'"#;

/// The pairing code `t3 pair` printed: its `Token:` line.
fn pairing_code(out: &str) -> Option<String> {
    let code = out.lines().find_map(|l| l.trim().strip_prefix("Token:"))?.trim();
    (!code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))).then(|| code.to_owned())
}

/// The ids of the sessions a link from `linker` left on a machine (`t3 auth session list --json`),
/// oldest first.
fn link_sessions(list: &str, linker: &str) -> Vec<String> {
    let label = session_label(linker);
    let Ok(sessions) = serde_json::from_str::<serde_json::Value>(list) else { return Vec::new() };
    let mut found: Vec<(&str, &str)> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["subject"] == "mcp-client" && s["client"]["label"].as_str() == Some(label.as_str()))
        .filter_map(|s| Some((s["issuedAt"].as_str().unwrap_or_default(), s["sessionId"].as_str()?)))
        .collect();
    found.sort();
    found.into_iter().map(|(_, id)| id.to_owned()).collect()
}

impl MachineRuntime {
    /// Lets the user's other machines link to this one, or stops that: its inbox passes their
    /// links to T3 while `on`, through a public URL that wakes it. Resolves to that URL.
    async fn take_links(&self, on: bool) -> Result<Option<String>> {
        self.ensure_awake().await?;
        if on && self.record().provision_revision != provision::revision() {
            // The inbox learned links in these scripts; taking them restarts T3, so never mid-turn.
            if self.watchdog_get("/status?fresh").await?.busy {
                bail!("{} needs a quick update first; try again once its agents finish", self.record().name);
            }
            self.kept_awake(self.upgrade_if_needed()).await?;
        }
        let file = format!("{{\"accept\": {on}}}");
        let script = format!(
            "umask 077; printf '%s\\n' {} > \"$HOME/.t3agent37/peers.json\"{}",
            shell_quote(&file),
            if on { format!("\n{LINKS_ON}") } else { String::new() }
        );
        self.shared.a37.exec_ok(&self.sandbox_id(), &script, 60).await?;
        self.update_record(|r| r.accepts_links = on);
        self.sync_inbox_url().await
    }

    /// A one-time code for a link from `linker` to this machine.
    async fn pairing_code_for(&self, linker: &str) -> Result<String> {
        let label = format!("{linker} (t3agent37)");
        let out = self.shared.a37.exec_ok(&self.sandbox_id(), &format!("t3 pair --label {} --ttl 10m", shell_quote(&label)), 60).await?;
        pairing_code(&out).ok_or_else(|| anyhow!("T3 on {} gave no pairing code", self.record().name))
    }

    /// Links this machine to the T3 at `url` with `code`, at `access`; a link it has to that
    /// environment already goes first, so this changes the access too.
    async fn link(&self, environment_id: &str, url: &str, code: &str, access: Access) -> Result<()> {
        let script = format!(
            "{LINKS_ON}\nt3 environment unlink {env} >/dev/null 2>&1; t3 environment link {url} --pairing-code {code} --access {access} 2>&1",
            env = shell_quote(environment_id),
            url = shell_quote(url),
            code = shell_quote(code),
            access = access.t3(),
        );
        let out = self.shared.a37.exec(&self.sandbox_id(), &script, 120).await?;
        if out.exit_code != Some(0) || !out.stdout.contains("Linked") {
            let said = out.stdout.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no answer");
            bail!("T3 on {} didn't link: {said}", self.record().name);
        }
        Ok(())
    }

    async fn unlink(&self, environment_id: &str) -> Result<()> {
        // Gone already is fine: T3 says so and exits non-zero.
        let _ = self.shared.a37.exec(&self.sandbox_id(), &format!("t3 environment unlink {}", shell_quote(environment_id)), 60).await?;
        Ok(())
    }

    /// Revokes the sessions links from `linker` left here, all of them or all but the newest
    /// (the link just made). Resolves to how many went.
    async fn revoke_links_from(&self, linker: &str, keep_newest: bool) -> Result<usize> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let list = a37.exec_ok(&id, "t3 auth session list --json", 60).await.context("listing T3's sessions")?;
        let mut sessions = link_sessions(list.trim(), linker);
        if keep_newest {
            sessions.pop();
        }
        for session in &sessions {
            a37.exec_ok(&id, &format!("t3 auth session revoke {}", shell_quote(session)), 60).await?;
        }
        Ok(sessions.len())
    }
}

/// A machine's links to the user's other machines, by name, for the list and the details.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    /// The other machine's name and id.
    pub machine: String,
    pub machine_id: String,
    /// T3's access level.
    pub access: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// When a link is made again: this long before it runs out.
const RENEW_BEFORE: chrono::Duration = chrono::Duration::days(5);
/// After a try that failed (the other machine busy, say), the next one waits this long: each
/// try wakes the other machine.
const RENEW_RETRY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// When each link (`<from id>><to id>`) was last made again, or tried.
static RENEWED: std::sync::Mutex<Option<std::collections::HashMap<String, std::time::Instant>>> = std::sync::Mutex::new(None);

impl App {
    /// `from`'s links to the user's other machines, and the machines that may use `from`.
    pub fn links_of(&self, from: &MachineRuntime) -> (Vec<LinkView>, Vec<LinkView>) {
        let machines = self.machines();
        let me = from.record();
        let out = me
            .links
            .iter()
            .filter_map(|l| {
                let to = machines.iter().map(|m| m.record()).find(|r| r.environment_id == l.environment_id && r.id != me.id)?;
                Some(LinkView { machine: to.name, machine_id: to.id, access: l.access.clone(), expires_at: l.expires_at.clone(), last_error: l.last_error.clone() })
            })
            .collect();
        let by = machines
            .iter()
            .map(|m| m.record())
            .filter(|r| r.id != me.id)
            .filter_map(|r| {
                let l = r.links.iter().find(|l| l.environment_id == me.environment_id)?;
                Some(LinkView { machine: r.name.clone(), machine_id: r.id.clone(), access: l.access.clone(), expires_at: l.expires_at.clone(), last_error: l.last_error.clone() })
            })
            .collect();
        (out, by)
    }

    /// Lets `from`'s agents use `to` with `access`, or not at all (`None`). Wakes both. Resolves
    /// to a sentence for the user.
    pub async fn set_link(&self, from: &str, to: &str, access: Option<Access>) -> Result<String> {
        let (a, b) = (self.machine(from)?, self.machine(to)?);
        if a.id() == b.id() {
            bail!("{}'s agents work on it already", a.record().name);
        }
        let (ra, rb) = (a.record(), b.record());
        let Some(access) = access else {
            a.ensure_awake().await?;
            a.unlink(&rb.environment_id).await?;
            b.ensure_awake().await?;
            b.revoke_links_from(&ra.name, false).await?;
            a.refresh_settings().await;
            // The last machine that used it gone: its inbox no longer passes links.
            let used = self.machines().iter().any(|m| m.id() != a.id() && m.record().links.iter().any(|l| l.environment_id == rb.environment_id));
            if !used {
                b.take_links(false).await?;
            }
            return Ok(format!("{} can't use {} any more: the link is gone on both.", ra.name, rb.name));
        };
        let url = b.take_links(true).await?.ok_or_else(|| anyhow!("{} has no public address for links", rb.name))?;
        a.ensure_awake().await?;
        let code = b.pairing_code_for(&ra.name).await?;
        a.link(&rb.environment_id, &url, &code, access).await?;
        // A link made before this one (a renewal, another access) leaves a session there: gone.
        b.revoke_links_from(&ra.name, true).await?;
        a.refresh_settings().await;
        Ok(format!(
            "{} can use {} now ({}: {}). Its agents work there with T3's own tools, such as delegate_task with {}'s environment.",
            ra.name,
            rb.name,
            access.short(),
            access.about(),
            rb.name
        ))
    }

    /// Makes again the links between the user's machines that run out soon, from machines that
    /// are up (never waking one for this; the one linked to wakes). A link that can't be made
    /// stays as it was until the next try.
    pub async fn renew_links(self: &Arc<Self>) {
        let now = chrono::Utc::now();
        for a in self.machines() {
            if !a.up_for_background() {
                continue;
            }
            let (links, _) = self.links_of(&a);
            for l in links {
                let due = chrono::DateTime::parse_from_rfc3339(&l.expires_at).is_ok_and(|t| t.with_timezone(&chrono::Utc) - now < RENEW_BEFORE);
                let Some(access) = Access::parse(&l.access) else { continue };
                let key = format!("{}>{}", a.id(), l.machine_id);
                let recent = RENEWED.lock().unwrap().get_or_insert_default().get(&key).is_some_and(|t| t.elapsed() < RENEW_RETRY);
                if due && !recent {
                    RENEWED.lock().unwrap().get_or_insert_default().insert(key, std::time::Instant::now());
                    let name = a.record().name;
                    match self.set_link(&a.id(), &l.machine_id, Some(access)).await {
                        Ok(_) => tracing::info!("made {name}'s link to {} again", l.machine),
                        Err(e) => tracing::warn!("making {name}'s link to {} again: {e:#}", l.machine),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels() {
        for a in Access::ALL {
            assert_eq!(Access::parse(a.t3()), Some(a));
            assert_eq!(Access::parse(a.short()), Some(a));
        }
        assert_eq!(Access::parse("Full"), Some(Access::FullAccess));
        assert_eq!(Access::parse("sudo"), None);
    }

    #[test]
    fn t3s_pairing_output() {
        // As `t3 pair` printed it on faktor (T3 0.1.12-nightly.20261122.5071), a QR code above.
        let out = "▀▀▀▀▀▀▀ ▀ ▀\nPairing with faktor (http://127.0.0.1:3773).\nPairing URL: http://127.0.0.1:3773/pair#token=Ab3dEf6hIj9k\nApp link: t3code://pair?x\nToken: Ab3dEf6hIj9k\nExpires: 2026-11-23T14:17:17.942Z\n";
        assert_eq!(pairing_code(out).as_deref(), Some("Ab3dEf6hIj9k"));
        assert_eq!(pairing_code("Token: a'; rm -rf /"), None);
        assert_eq!(pairing_code("no token here"), None);
    }

    #[test]
    fn a_links_sessions_on_the_other_machine() {
        // As `t3 auth session list --json` listed them on faktor, after skiff linked to it twice.
        let list = r#"[
          {"sessionId":"desk","subject":"mcp-client","client":{"label":"T3 Code · Dagur's MacBook Pro (2)"},"issuedAt":"2026-11-22T13:20:18.192Z"},
          {"sessionId":"new","subject":"mcp-client","client":{"label":"T3 Code · skiff"},"issuedAt":"2026-11-23T14:02:32.158Z"},
          {"sessionId":"old","subject":"mcp-client","client":{"label":"T3 Code · skiff"},"issuedAt":"2026-11-20T09:00:00.000Z"},
          {"sessionId":"app","subject":"cli-issued-session","client":{"label":"T3 Code · skiff"},"issuedAt":"2026-11-22T13:20:07.072Z"}
        ]"#;
        assert_eq!(link_sessions(list, "skiff"), ["old", "new"]);
        assert_eq!(link_sessions(list, "faktor"), Vec::<String>::new());
        assert_eq!(link_sessions("not json", "skiff"), Vec::<String>::new());
    }
}
