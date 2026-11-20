//! Signing an agent in on a machine with the vendor's own CLI login. The login runs on the
//! machine and listens for its OAuth callback on the machine's localhost. This computer listens
//! on the same port and replays each callback request there over exec, so the browser's redirect
//! after "Authorize" lands on the machine: no code to paste, no port opened on the machine, and
//! the credential is created on the machine and stays there. This computer's own logins are
//! never touched.

use super::MachineRuntime;
use crate::agent37::{shell_quote, Agent37};
use crate::proxy::upstream;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use std::time::{Duration, Instant};

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
    pub async fn sign_in(&self, agent: Agent) -> Result<()> {
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        // The CLI hands the sign-in URL to $BROWSER; this one records it instead.
        let dir = a37
            .exec_ok(
                &id,
                &format!(
                    "d=$(mktemp -d); printf '#!/bin/sh\\necho \"$1\" > %s/url\\n' \"$d\" > \"$d/browser\"; \
                     chmod +x \"$d/browser\"; \
                     BROWSER=\"$d/browser\" nohup script -qfec '{}' \"$d/log\" >/dev/null 2>&1 & echo \"$d\"",
                    agent.login()
                ),
                30,
            )
            .await?;
        let dir = dir.lines().last().unwrap_or_default().trim().to_owned();
        // The machine must not fall asleep while the person is in the browser.
        let result = self.kept_awake(self.finish_sign_in(agent, &dir)).await;
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

    async fn finish_sign_in(&self, agent: Agent, dir: &str) -> Result<()> {
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
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let port = callback_port(&url).ok_or_else(|| anyhow!("the {agent:?} login has no local callback"))?;
        let _relay = Relay::start(a37.clone(), id.clone(), port)
            .await
            .with_context(|| format!("relaying port {port} for the {agent:?} login"))?;
        crate::app::open_url(&url)?;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if a37.exec(&id, agent.status(), 30).await?.exit_code == Some(0) {
                return Ok(());
            }
            if started.elapsed() > LOGIN_TIMEOUT {
                bail!("the {agent:?} sign-in was not finished in the browser");
            }
        }
    }
}

/// GitHub's device sign-in lasts 15 minutes.
const GITHUB_TIMEOUT: Duration = Duration::from_secs(15 * 60);

impl MachineRuntime {
    /// Signs the machine's gh, which its git uses for GitHub, in to the user's GitHub account with
    /// GitHub's device flow: the page opens here, the code is on the clipboard and in a
    /// notification, and the token is created on the machine and stays there.
    pub async fn sign_in_github(&self) -> Result<()> {
        self.ensure_awake().await?;
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
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
        let result = self.kept_awake(self.finish_github(&dir)).await;
        let _ = a37.exec(&id, &format!("rm -rf '{dir}'; pkill -f 'gh auth login' || true"), 30).await;
        result?;
        self.refresh_github().await;
        Ok(())
    }

    async fn finish_github(&self, dir: &str) -> Result<()> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let started = Instant::now();
        let mut shown = false;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let out = a37.exec(&id, &format!("cat '{dir}/exit' 2>/dev/null; echo ---; cat '{dir}/log'"), 30).await?;
            let (exit, log) = out.stdout.split_once("---").unwrap_or_default();
            if !shown && let Some(code) = device_code(log) {
                shown = true;
                copy_to_clipboard(&code);
                self.shared.notify(
                    format!("GitHub code for {}", self.record().name),
                    format!("Enter {code} on the GitHub page that just opened (it is on your clipboard)"),
                );
                crate::app::open_url("https://github.com/login/device")?;
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
    use super::{callback_port, device_code, from_curl};

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
