//! Signing an agent in on a machine with the vendor's own CLI login. The login runs on the
//! machine and listens for its OAuth callback on the machine's localhost. This computer listens
//! on the same port and replays each callback request there over exec, so the browser's redirect
//! after "Authorize" lands on the machine: no code to paste, no port opened on the machine, and
//! the credential is created on the machine and stays there. This computer's own logins are
//! never touched.
//!
//! Each machine signs in on its own, so machines can use different accounts. Signing in again
//! over a login ("another account") replaces it only once the new one finishes: a sign-in left
//! unfinished keeps the old one.

use super::MachineRuntime;
use crate::agent37::{shell_quote, Agent37};
use crate::proxy::upstream;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Who an agent on a machine is signed in as, from T3's provider status as last recorded (so
/// it's known while the machine sleeps).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// The account's email, as T3 reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// The plan, short: "Ultra" for Claude Ultra, "ChatGPT Plus" for Codex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The plan's usage limits as T3 last checked them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub usage: Vec<UsageWindow>,
    /// When that was (unix ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_checked_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// T3's label: "Weekly", "Weekly · Fable".
    pub label: String,
    pub used_percent: f64,
    /// Unix ms.
    pub resets_at: Option<i64>,
}

/// The account of a T3 provider status (an entry of the server config's `providers`).
pub fn account(provider: &Value) -> Account {
    let auth = &provider["auth"];
    let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let name = provider["displayName"].as_str().unwrap_or_default();
    // "Claude Ultra Subscription" → "Ultra"; "ChatGPT Plus Subscription" → "ChatGPT Plus".
    let plan = text(&auth["label"]).or_else(|| text(&auth["type"])).map(|p| {
        let p = p.strip_suffix(" Subscription").unwrap_or(&p);
        let p = p.strip_prefix(name).and_then(|rest| rest.strip_prefix(' ')).unwrap_or(p);
        p.to_owned()
    });
    let limits = &provider["usageLimits"];
    let ms = |v: &Value| v.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp_millis());
    let usage: Vec<UsageWindow> = limits["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            Some(UsageWindow {
                label: text(&w["label"])?,
                used_percent: w["usedPercent"].as_f64()?,
                resets_at: ms(&w["resetsAt"]),
            })
        })
        .collect();
    Account {
        email: text(&auth["email"]),
        plan,
        usage_checked_at: if usage.is_empty() { None } else { ms(&limits["checkedAt"]) },
        usage,
    }
}

const URL_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    /// By the provider's display name, as `signedOut` lists it.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Claude" => Some(Self::Claude),
            "Codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn login(self) -> &'static str {
        match self {
            Self::Claude => "claude auth login",
            Self::Codex => "codex login",
        }
    }

    /// Exits 0 once signed in.
    fn status(self) -> &'static str {
        match self {
            Self::Claude => "claude auth status >/dev/null 2>&1",
            Self::Codex => "codex login status >/dev/null 2>&1",
        }
    }
}

impl MachineRuntime {
    /// `another`: with another account than the browser's own sign-in, so the page opens in a
    /// private window where the browser has one. Over an existing login, this is how the
    /// machine switches accounts.
    pub async fn sign_in(&self, agent: Agent, another: bool) -> Result<()> {
        let _running = SignInRunning::start(self, &format!("{agent:?}"));
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        // Signed in already: the status says so throughout, so only the login's end counts.
        let replacing = a37.exec(&id, agent.status(), 30).await?.exit_code == Some(0);
        // The CLI hands the sign-in URL to $BROWSER; this one records it instead. `exit` says
        // how the login ended.
        let dir = a37
            .exec_ok(
                &id,
                &format!(
                    "d=$(mktemp -d); printf '#!/bin/sh\\necho \"$1\" > %s/url\\n' \"$d\" > \"$d/browser\"; \
                     chmod +x \"$d/browser\"; \
                     (BROWSER=\"$d/browser\" nohup script -qfec '{}' \"$d/log\" >/dev/null 2>&1; echo $? > \"$d/exit\") \
                     >/dev/null 2>&1 & echo \"$d\"",
                    agent.login()
                ),
                30,
            )
            .await?;
        let dir = dir.lines().last().unwrap_or_default().trim().to_owned();
        // The machine must not fall asleep while the person is in the browser.
        let result = self.kept_awake(self.finish_sign_in(agent, &dir, replacing, another)).await;
        let _ = a37
            .exec(&id, &format!("rm -rf '{dir}'; pkill -f '{}' || true", agent.login()), 30)
            .await;
        result?;
        // T3 re-checks the provider and tells T3 Code; the sign-in nudge goes away with it, and
        // the recorded config learns it too.
        let refreshed = upstream::call(
            &self.upstream_url(),
            &self.upstream_token(),
            "server.refreshProviders",
            serde_json::json!({}),
        )
        .await;
        if let Ok(result) = refreshed {
            self.cache.lock().unwrap().fold_providers(&result["providers"]);
            self.save_cache();
        }
        Ok(())
    }

    async fn finish_sign_in(&self, agent: Agent, dir: &str, replacing: bool, another: bool) -> Result<()> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let started = Instant::now();
        let url = loop {
            let out = a37.exec(&id, &format!("cat '{dir}/url' 2>/dev/null"), 30).await?;
            if let Some(url) = out.stdout.split_whitespace().find(|w| w.starts_with("https://")) {
                break url.to_owned();
            }
            if started.elapsed() > URL_TIMEOUT {
                bail!("the {agent:?} login did not start on the machine");
            }
            self.pause(Duration::from_millis(300)).await?;
        };
        let port = callback_port(&url).ok_or_else(|| anyhow!("the {agent:?} login has no local callback"))?;
        let _relay = Relay::start(a37.clone(), id.clone(), port)
            .await
            .with_context(|| format!("relaying port {port} for the {agent:?} login"))?;
        self.open_sign_in_page(&format!("{agent:?}"), &url, another)?;
        // Either signal: the login exiting, or (for a first sign-in) the status turning.
        let check = if replacing {
            format!("cat '{dir}/exit' 2>/dev/null")
        } else {
            format!("cat '{dir}/exit' 2>/dev/null; {} && echo in", agent.status())
        };
        loop {
            self.pause(Duration::from_secs(3)).await?;
            let out = a37.exec(&id, &check, 30).await?;
            let mut words = out.stdout.split_whitespace();
            match words.next() {
                Some("0" | "in") => return Ok(()),
                Some(code) => bail!("the {agent:?} login on the machine ended without signing in (exit {code})"),
                None => {}
            }
            if started.elapsed() > LOGIN_TIMEOUT {
                bail!("the {agent:?} sign-in was not finished in the browser");
            }
        }
    }

    /// Stops the sign-in that runs here, if any, within a moment: the login on the machine is
    /// stopped and its old sign-in, if it had one, stays.
    pub fn cancel_sign_in(&self) {
        if self.signing_in().is_some() {
            self.sign_in_canceled.store(true, Ordering::SeqCst);
            self.sign_in_wake.notify_waiters();
        }
    }

    /// A sign-in's wait between looks at the machine; a cancel ends it early, as an error.
    async fn pause(&self, d: Duration) -> Result<()> {
        if !self.sign_in_canceled.load(Ordering::SeqCst) {
            tokio::select! {
                _ = tokio::time::sleep(d) => {}
                _ = self.sign_in_wake.notified() => {}
            }
        }
        if self.sign_in_canceled.load(Ordering::SeqCst) {
            bail!("{CANCELED}");
        }
        Ok(())
    }

    /// Opens a sign-in page here: in a private window for another account, so the browser's
    /// own sign-in doesn't answer for it.
    fn open_sign_in_page(&self, agent: &str, url: &str, another: bool) -> Result<()> {
        if !another {
            return crate::app::open_url(url);
        }
        if !crate::app::open_url_private(url)? {
            self.shared.notify(
                format!("{agent} sign-in for {}", self.record().name),
                "Your browser can't be opened in a private window from here. If the page shows the account you're \
                 signed in with there, switch accounts on the page or sign out first.",
            );
        }
        Ok(())
    }
}

/// GitHub's device sign-in lasts 15 minutes.
const GITHUB_TIMEOUT: Duration = Duration::from_secs(15 * 60);

impl MachineRuntime {
    /// Signs the machine's gh, which its git uses for GitHub, in to the user's GitHub account with
    /// GitHub's device flow: the page opens here, the code is on the clipboard and in a
    /// notification, and the token is created on the machine and stays there. `another`: the
    /// page opens in a private window (see `sign_in`). Over an existing login gh adds the new
    /// account and makes it the one in use; the old one is then signed out.
    pub async fn sign_in_github(&self, another: bool) -> Result<()> {
        let _running = SignInRunning::start(self, crate::app::GITHUB);
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let before = self.record().github_login;
        // In the background, so its code can be read while it waits; `exit` says how it ended.
        let dir = a37
            .exec_ok(
                &id,
                "d=$(mktemp -d); \
                 (gh auth login --hostname github.com --git-protocol https --web --skip-ssh-key \
                    --scopes repo,read:org,workflow > \"$d/log\" 2>&1; echo $? > \"$d/exit\") \
                 </dev/null >/dev/null 2>&1 & echo \"$d\"",
                30,
            )
            .await?;
        let dir = dir.lines().last().unwrap_or_default().trim().to_owned();
        let result = self.kept_awake(self.finish_github(&dir, another)).await;
        if self.github_code.lock().unwrap().take().is_some() {
            self.shared.changed();
        }
        let _ = a37.exec(&id, &format!("rm -rf '{dir}'; pkill -f 'gh auth login' || true"), 30).await;
        result?;
        self.refresh_github().await;
        let after = self.record().github_login;
        if let Some(old) = before.filter(|old| after.is_some() && after.as_ref() != Some(old)) {
            let out = a37
                .exec(&id, &format!("gh auth logout --hostname github.com --user {}", shell_quote(&old)), 30)
                .await;
            if !matches!(out, Ok(ref r) if r.exit_code == Some(0)) {
                tracing::warn!("signing {old} out of GitHub on {}: {out:?}", self.id());
            }
        }
        Ok(())
    }

    async fn finish_github(&self, dir: &str, another: bool) -> Result<()> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let started = Instant::now();
        let mut shown = false;
        loop {
            self.pause(Duration::from_secs(2)).await?;
            let out = a37.exec(&id, &format!("cat '{dir}/exit' 2>/dev/null; echo ---; cat '{dir}/log'"), 30).await?;
            let (exit, log) = out.stdout.split_once("---").unwrap_or_default();
            if !shown && let Some(code) = device_code(log) {
                shown = true;
                // In the panel for as long as the sign-in waits, and on the clipboard to start.
                *self.github_code.lock().unwrap() = Some(code.clone());
                self.shared.changed();
                copy_to_clipboard(&code);
                self.shared.notify(
                    format!("GitHub code for {}", self.record().name),
                    format!("Enter {code} on the GitHub page that just opened. It's on your clipboard and in t3agent37's panel."),
                );
                self.open_sign_in_page("GitHub", "https://github.com/login/device", another)?;
            }
            match exit.trim() {
                "" => {}
                "0" => return Ok(()),
                _ => bail!("GitHub sign-in failed: {}", log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output")),
            }
            if started.elapsed() > GITHUB_TIMEOUT {
                bail!("the GitHub sign-in was not finished in the browser");
            }
        }
    }

    /// Who the machine's gh is signed in as, kept on its record so the details view can say it
    /// without waking the machine. Only for a running machine: an exec would wake it.
    pub(super) async fn refresh_github(&self) {
        if !self.up_for_background() {
            return;
        }
        let out = self.shared.a37.exec(&self.sandbox_id(), "gh api user --jq .login 2>/dev/null", 30).await;
        let login = match out {
            Ok(r) if r.exit_code == Some(0) => Some(r.stdout.trim().to_owned()).filter(|l| !l.is_empty()),
            Ok(_) => None,
            Err(e) => return tracing::warn!("reading {}'s GitHub login: {e:#}", self.id()),
        };
        let record = self.record();
        if !record.github_checked || record.github_login != login {
            self.update_record(|m| {
                m.github_checked = true;
                m.github_login = login;
            });
        }
    }
}

/// What a canceled sign-in fails with.
pub const CANCELED: &str = "the sign-in was canceled";

/// Marks a sign-in as running on its machine for as long as it lives, for the panel (and its
/// Cancel) to see.
struct SignInRunning<'a>(&'a MachineRuntime);

impl<'a> SignInRunning<'a> {
    fn start(machine: &'a MachineRuntime, agent: &str) -> Self {
        machine.sign_in_canceled.store(false, Ordering::SeqCst);
        *machine.signing_in.lock().unwrap() = Some(agent.to_owned());
        machine.shared.changed();
        Self(machine)
    }
}

impl Drop for SignInRunning<'_> {
    fn drop(&mut self) {
        *self.0.signing_in.lock().unwrap() = None;
        self.0.sign_in_canceled.store(false, Ordering::SeqCst);
        self.0.shared.changed();
    }
}

/// gh's one-time code, `XXXX-XXXX`.
fn device_code(log: &str) -> Option<String> {
    log.split(|c: char| c.is_whitespace() || c == ':').map(str::trim).find(|w| {
        w.len() == 9
            && w.as_bytes()[4] == b'-'
            && w.chars().filter(|c| *c != '-').all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    }).map(str::to_owned)
}

pub(crate) fn copy_to_clipboard(text: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Listens on `port` here for as long as it lives, and answers each request with what the same
/// request gets from `port` on the machine's localhost.
struct Relay(tokio::task::JoinHandle<()>);

impl Drop for Relay {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Relay {
    async fn start(a37: Agent37, instance: String, port: u16) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| anyhow!("port {port} is in use on this computer ({e}); finish or close the other sign-in first"))?;
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let (a37, instance) = (a37.clone(), instance.clone());
            async move { replay(&a37, &instance, port, &uri).await }
        });
        Ok(Self(tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        })))
    }
}

/// A browser callback is a plain GET whose path and query carry the code: that is all that goes
/// over, base64 so nothing in it reaches the shell.
async fn replay(a37: &Agent37, instance: &str, port: u16, uri: &axum::http::Uri) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let b64 = base64::engine::general_purpose::STANDARD.encode(path);
    let script = format!(
        "p=$(echo {} | base64 -d); curl -s -i --max-time 20 \"http://127.0.0.1:{port}$p\"",
        shell_quote(&b64)
    );
    match a37.exec(instance, &script, 40).await {
        Ok(r) if r.exit_code == Some(0) => from_curl(&r.stdout).unwrap_or_else(|| {
            (axum::http::StatusCode::BAD_GATEWAY, "the sign-in on the machine gave no answer").into_response()
        }),
        Ok(r) => (axum::http::StatusCode::BAD_GATEWAY, format!("the sign-in on the machine is gone: {}", r.stderr.trim()))
            .into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, format!("couldn't reach the machine: {e:#}")).into_response(),
    }
}

/// `curl -i` output back into a response: status, the headers a browser needs here, and body.
fn from_curl(out: &str) -> Option<axum::response::Response> {
    let (head, body) = out.split_once("\r\n\r\n").or_else(|| out.split_once("\n\n"))?;
    let mut lines = head.lines();
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut res = axum::response::Response::builder().status(status);
    for line in lines {
        let Some((k, v)) = line.split_once(':') else { continue };
        if matches!(k.trim().to_ascii_lowercase().as_str(), "location" | "content-type") {
            res = res.header(k.trim(), v.trim());
        }
    }
    res.body(axum::body::Body::from(body.to_owned())).ok()
}

/// The port of a `redirect_uri=http://localhost:<port>/...` (or 127.0.0.1) parameter.
fn callback_port(url: &str) -> Option<u16> {
    let url = url::Url::parse(url).ok()?;
    let redirect = url.query_pairs().find(|(k, _)| k == "redirect_uri")?.1;
    let redirect = url::Url::parse(&redirect).ok()?;
    matches!(redirect.host_str()?, "localhost" | "127.0.0.1").then_some(())?;
    redirect.port()
}

#[cfg(test)]
mod tests {
    use super::{account, callback_port, device_code, from_curl};
    use serde_json::json;

    #[test]
    fn reads_the_account_from_t3s_provider_status() {
        let claude = json!({
            "displayName": "Claude",
            "auth": {"status": "authenticated", "type": "Claude Ultra", "label": "Claude Ultra Subscription", "email": "you@example.com"},
            "usageLimits": {"checkedAt": "2026-11-21T15:36:39.206Z", "windows": [
                {"id": "seven_day", "kind": "weekly", "label": "Weekly", "usedPercent": 35, "resetsAt": "2026-11-24T23:59:59.706Z"},
                {"id": "seven_day_fable", "kind": "weekly", "label": "Weekly · Fable", "usedPercent": 48.5},
            ]},
        });
        let a = account(&claude);
        assert_eq!(a.email.as_deref(), Some("you@example.com"));
        assert_eq!(a.plan.as_deref(), Some("Ultra"));
        assert_eq!(a.usage.len(), 2);
        assert_eq!((a.usage[1].label.as_str(), a.usage[1].used_percent, a.usage[1].resets_at), ("Weekly · Fable", 48.5, None));
        assert_eq!(a.usage[0].resets_at, Some(1795564799706));
        assert_eq!(a.usage_checked_at, Some(1795275399206));
        let codex = json!({"displayName": "Codex", "auth": {"status": "authenticated", "label": "ChatGPT Plus Subscription"}, "usageLimits": {"windows": []}});
        let a = account(&codex);
        assert_eq!((a.email, a.plan.as_deref(), a.usage.len(), a.usage_checked_at), (None, Some("ChatGPT Plus"), 0, None));
        assert_eq!(account(&json!({"displayName": "Codex", "auth": {"status": "unauthenticated"}})), Default::default());
    }

    #[test]
    fn finds_ghs_device_code() {
        let log = "! First copy your one-time code: 3F2A-9BC1\nOpen this URL to continue in your web browser: https://github.com/login/device\n";
        assert_eq!(device_code(log).as_deref(), Some("3F2A-9BC1"));
        assert_eq!(device_code("nothing here 2026-11-18"), None);
    }

    #[test]
    fn turns_curl_output_into_a_response() {
        let res = from_curl("HTTP/1.1 302 Found\r\nLocation: https://x.test/ok\r\nX-Other: 1\r\n\r\n").unwrap();
        assert_eq!(res.status(), 302);
        assert_eq!(res.headers()["location"], "https://x.test/ok");
        assert!(res.headers().get("x-other").is_none());
        assert!(from_curl("garbage").is_none());
    }

    #[test]
    fn finds_the_local_callback_port() {
        let claude = "https://claude.com/cai/oauth/authorize?code=true&redirect_uri=http%3A%2F%2Flocalhost%3A43327%2Fcallback&state=x";
        let codex = "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback";
        let manual = "https://claude.com/cai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
        assert_eq!(callback_port(claude), Some(43327));
        assert_eq!(callback_port(codex), Some(1455));
        assert_eq!(callback_port(manual), None);
    }
}
