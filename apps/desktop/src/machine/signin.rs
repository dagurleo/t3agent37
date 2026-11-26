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
        self.refresh_providers().await;
        Ok(())
    }

    /// T3 re-checks its agents' sign-ins and tells T3 Code (a sign-in nudge goes away with it),
    /// and the recorded config learns them too.
    pub(crate) async fn refresh_providers(&self) {
        let refreshed = upstream::call(&self.upstream_url(), &self.upstream_token(), "server.refreshProviders", serde_json::json!({})).await;
        if let Ok(result) = refreshed {
            self.cache.lock().unwrap().fold_providers(&result["providers"]);
            self.save_cache();
        }
    }

    /// A new machine's gh signed in with a token saved on this computer (`app/newmachine.rs`),
    /// which GitHub took when it was saved. git uses it from there. The machine is up.
    pub async fn put_github_token(&self, token: &Secret, expires: Option<String>) -> Result<()> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&token.0);
        let login = format!("echo {} | base64 -d | gh auth login --hostname github.com --git-protocol https --with-token", shell_quote(&b64));
        self.shared.a37.exec_ok(&self.sandbox_id(), &login, 60).await.context("signing gh in")?;
        self.refresh_github().await;
        self.update_record(|r| r.github_token_expires = expires);
        Ok(())
    }

    /// A new machine's Codex signed in with an OpenAI API key saved on this computer. Codex's
    /// own login renews itself, so machines sharing one could sign each other out; a key doesn't
    /// change. The machine is up.
    pub async fn put_openai_key(&self, key: &Secret) -> Result<()> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&key.0);
        let login = format!("echo {} | base64 -d | codex login --with-api-key", shell_quote(&b64));
        self.shared.a37.exec_ok(&self.sandbox_id(), &login, 60).await.context("signing Codex in")?;
        self.refresh_providers().await;
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

/// A secret that never shows in logs or debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// What a fine-grained token needs for what machines do: clone, push (workflow files too) and
/// open pull requests, comment and label for GitHub issues, make their webhook, and read CI.
/// Fine-grained tokens can't read the Checks API at all, so CI comes from Actions and statuses.
const GITHUB_TOKEN_PERMISSIONS: &str =
    "contents=write&pull_requests=write&issues=write&workflows=write&actions=read&statuses=read&repository_hooks=write&metadata=read";

/// GitHub's page for a new fine-grained token for this machine: its name, description, 90 days
/// and the permissions filled in. The repositories are picked there; GitHub takes no template
/// for those.
pub fn github_token_page(machine: &str) -> String {
    let name: String = format!("t3agent37 {machine}").chars().take(40).collect();
    let description = format!(
        "For t3agent37's machine {machine}: its agents clone, push and open pull requests only in the repositories picked here."
    );
    format!(
        "https://github.com/settings/personal-access-tokens/new?name={}&description={}&expires_in=90&{GITHUB_TOKEN_PERMISSIONS}",
        urlencoding::encode(&name),
        urlencoding::encode(&description)
    )
}

/// What `claude setup-token` prints: `sk-ant-oat01-…`. Only characters that go into a shell
/// file as they are.
pub fn check_claude_token(token: &str) -> Result<String> {
    let token = token.trim();
    let ok = token.starts_with("sk-ant-")
        && (30..=512).contains(&token.len())
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("that isn't a token from `claude setup-token` (it starts with sk-ant-)");
    }
    Ok(token.to_owned())
}

/// An OpenAI API key, for Codex: `sk-…` (`sk-proj-…` for a project's).
pub fn check_openai_key(key: &str) -> Result<String> {
    let key = key.trim();
    let ok = key.starts_with("sk-") && (20..=512).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("that isn't an OpenAI API key (it starts with sk-)");
    }
    Ok(key.to_owned())
}

/// A token for a machine's gh: a fine-grained personal access token (`github_pat_…`), or a
/// classic one (`ghp_…`), which reaches every repository like gh's own sign-in.
pub fn check_token_shape(token: &str) -> Result<String> {
    let token = token.trim();
    let known = token.starts_with("github_pat_") || token.starts_with("ghp_");
    if !known || token.len() < 36 || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("That isn't a GitHub token. A fine-grained one starts with github_pat_");
    }
    Ok(token.to_owned())
}

/// A GitHub token saved for new machines: as `check_token_shape`, or gh's own sign-in (`gho_…`,
/// what `gh auth token` prints).
pub fn check_saved_github_token(token: &str) -> Result<String> {
    let t = token.trim();
    if t.starts_with("gho_") && t.len() >= 36 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(t.to_owned());
    }
    check_token_shape(t)
}

/// What GitHub says about a token: whose it is and when it expires.
#[derive(Clone, Debug, PartialEq)]
pub struct GithubToken {
    pub login: String,
    pub expires: Option<String>,
}

/// Asks GitHub about a token from this computer, so a bad one is caught before the machine wakes.
pub async fn check_github_token(token: &Secret) -> Result<GithubToken> {
    let client = reqwest::Client::builder().user_agent("t3agent37").timeout(Duration::from_secs(20)).build()?;
    let get = |path: &str| {
        client
            .get(format!("https://api.github.com{path}"))
            .bearer_auth(&token.0)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
    };
    let res = get("/user").send().await.context("asking GitHub about the token")?;
    match res.status().as_u16() {
        200 => {}
        401 => bail!("GitHub doesn't take that token: check that all of it was copied and that it hasn't expired"),
        code => bail!("GitHub answered {code} about that token"),
    }
    let expires = res.headers().get("github-authentication-token-expiration").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let login = res.json::<Value>().await?["login"].as_str().ok_or_else(|| anyhow!("GitHub didn't say whose token it is"))?.to_owned();
    Ok(GithubToken { login, expires })
}

/// Who the machine's gh is, and for a fine-grained token a short hash of it (so a new one is
/// tested again): `login`, then `limited <hash>` or `full`.
const GITHUB_STATE: &str = r#"login="$(gh api user --jq .login 2>/dev/null)" || exit 1
echo "$login"
t="$(gh auth token 2>/dev/null)"
case "$t" in
  github_pat_*) echo "limited $(printf '%s' "$t" | sha256sum | cut -c1-16)" ;;
  *) echo full ;;
esac"#;

/// What `GITHUB_STATE` printed: the login, and the token's hash when it's a fine-grained one.
fn parse_github_state(out: &str) -> Option<(String, Option<String>)> {
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let login = lines.next()?.to_owned();
    let hash = lines.next().and_then(|l| l.strip_prefix("limited ")).map(str::to_owned);
    Some((login, hash))
}

/// Tests which repositories gh's token can push to: GitHub's list of the account's
/// repositories (whatever the token reaches), each asked for git's push service, which answers
/// 200 only to a token that can push there. Nothing is changed. The token goes to curl in a file
/// only root can read, never on a command line. Prints `owner <name> <count>` for every owner in
/// the list, then `push <repo>` for each it can push to.
const GITHUB_REACH: &str = r#"d="$(mktemp -d)"; trap 'rm -rf "$d"' EXIT
printf 'user = "x-access-token:%s"
' "$(gh auth token)" > "$d/auth"; chmod 600 "$d/auth"
gh api --paginate '/user/repos?per_page=100&affiliation=owner,collaborator,organization_member' --jq '.[].full_name' > "$d/all" || exit 1
cut -d/ -f1 "$d/all" | sort | uniq -c | while read -r n o; do echo "owner $o $n"; done
xargs -P 16 -I{} sh -c 'c=$(curl -s -m 20 -K "$1" -o /dev/null -w "%{http_code}" "https://github.com/$2.git/info/refs?service=git-receive-pack"); [ "$c" = 200 ] && echo "push $2"' _ "$d/auth" {} < "$d/all"
true"#;

/// What `GITHUB_REACH` printed, as a reach for the token with hash `token`.
fn parse_reach(out: &str, token: &str) -> crate::store::GithubReach {
    let mut owners = std::collections::BTreeMap::new();
    let mut repos = Vec::new();
    for line in out.lines() {
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("owner"), Some(owner), Some(n)) => {
                owners.insert(owner.to_owned(), n.parse::<usize>().unwrap_or(0));
            }
            (Some("push"), Some(repo), None) => repos.push(repo.to_owned()),
            _ => {}
        }
    }
    repos.sort_by_key(|r| r.to_lowercase());
    let owner_of = |r: &String| r.split('/').next().unwrap_or_default().to_owned();
    let only = repos.first().map(owner_of).filter(|o| repos.iter().all(|r| owner_of(r) == *o));
    // Every repository of one owner, and more than a handful: "All repositories".
    let all_of = only.filter(|o| repos.len() > 3 && owners.get(o) == Some(&repos.len()));
    crate::store::GithubReach { token: token.to_owned(), repos, all_of }
}

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
        self.sign_out_github(before).await;
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

    /// Signs the machine's gh in with a token the user made on GitHub: a fine-grained one limited
    /// to some repositories, so the machine's agents reach only those. Checked with GitHub from
    /// here first, then put in gh on the machine (git uses it from there); the account it had
    /// before is signed out. Wakes the machine.
    /// Resolves to who it is now and what it reaches, for a toast.
    pub async fn sign_in_github_token(&self, token: &Secret) -> Result<String> {
        let token = Secret(check_token_shape(&token.0)?);
        let info = check_github_token(&token).await?;
        let _running = SignInRunning::start(self, crate::app::GITHUB);
        self.ensure_awake().await?;
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        let before = self.record().github_login;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&token.0);
        let login = format!("echo {} | base64 -d | gh auth login --hostname github.com --git-protocol https --with-token", shell_quote(&b64));
        self.kept_awake(a37.exec_ok(&id, &login, 60)).await.context("signing gh in on the machine")?;
        self.kept_awake(self.refresh_github()).await;
        self.update_record(|r| r.github_token_expires = info.expires.clone());
        self.sign_out_github(before).await;
        self.shared.changed();
        let reach = self.record().github_reach.map_or_else(|| "every repo it can reach".to_owned(), |r| r.describe());
        Ok(format!("@{}, {reach}", info.login))
    }

    /// Claude on the machine signed in with a long-lived token from `claude setup-token` (made
    /// on any computer, in the browser), or back on its own login (`None`). The token goes in
    /// `agent.env`, which boot.sh loads into T3's environment, so the agents T3 runs use it over
    /// any login. Claude's own login is put aside meanwhile: a base's copies all start with it,
    /// and one copy refreshing it could sign the others out. Restarts T3, so not while an agent
    /// works there. Wakes the machine.
    pub async fn set_claude_token(&self, token: Option<&Secret>) -> Result<String> {
        let token = token.map(|t| check_claude_token(&t.0)).transpose()?;
        let name = self.record().name;
        self.ensure_awake().await?;
        if self.watchdog_get("/status?fresh").await?.busy {
            bail!("an agent works on {name}, and T3 restarts for this: try again once it's done");
        }
        // boot.sh loads agent.env from this version's scripts on.
        self.kept_awake(self.upgrade_if_needed()).await?;
        let (d, c) = ("/root/.t3agent37", "/root/.claude/.credentials.json");
        let script = match &token {
            Some(t) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(format!("CLAUDE_CODE_OAUTH_TOKEN={t}\n"));
                format!(
                    "umask 077; echo {} | base64 -d > {d}/agent.env.new && mv -f {d}/agent.env.new {d}/agent.env; \
                     [ -f {c} ] && mv -f {c} {c}.t3agent37-off; true",
                    shell_quote(&b64)
                )
            }
            None => format!("rm -f {d}/agent.env; [ -f {c}.t3agent37-off ] && [ ! -f {c} ] && mv -f {c}.t3agent37-off {c}; true"),
        };
        let a37 = &self.shared.a37;
        a37.exec_ok(&self.sandbox_id(), &script, 30).await.context("putting the token on the machine")?;
        self.kept_awake(self.restart_t3()).await?;
        self.shared.changed();
        Ok(match token {
            Some(_) => format!(
                "Claude on {name} uses the long-lived token now. Its own login is put aside, so copies of {name} share the token \
                 without signing each other out. `sign-in {name} claude --no-token` goes back to the login."
            ),
            None => format!("Claude on {name} is back on its own login, if it had one."),
        })
    }

    /// Which repositories a fine-grained token on the machine can push to (`GITHUB_REACH`).
    /// A failed test leaves the hash empty, so it's tried again.
    async fn test_github_reach(&self, hash: &str) -> crate::store::GithubReach {
        match self.shared.a37.exec(&self.sandbox_id(), GITHUB_REACH, 180).await {
            Ok(r) if r.exit_code == Some(0) => parse_reach(&r.stdout, hash),
            other => {
                tracing::warn!("testing {}'s GitHub token: {other:?}", self.id());
                crate::store::GithubReach::default()
            }
        }
    }

    /// The agents' sign-in status read from the machine's T3 itself, when this app has none: T3
    /// Code shows it a machine's config only once it has opened the machine, so a new one had no
    /// agents to sign in. Only while it runs: never wakes it.
    pub async fn refresh_agents(&self) {
        if !self.up_for_background() || !self.cache.lock().unwrap().providers().is_empty() {
            return;
        }
        match upstream::call(&self.upstream_url(), &self.upstream_token(), "server.getConfig", serde_json::json!({})).await {
            Ok(config) => {
                self.cache.lock().unwrap().fold_providers(&config["providers"]);
                self.save_cache();
                self.shared.changed();
            }
            Err(e) => tracing::debug!("reading {}'s agents: {e:#}", self.id()),
        }
    }

    /// After a sign-in as another account: the old one is signed out of gh, which kept it.
    async fn sign_out_github(&self, before: Option<String>) {
        let after = self.record().github_login;
        let Some(old) = before.filter(|old| after.is_some() && after.as_ref() != Some(old)) else { return };
        let out = self.shared.a37.exec(&self.sandbox_id(), &format!("gh auth logout --hostname github.com --user {}", shell_quote(&old)), 30).await;
        if !matches!(out, Ok(ref r) if r.exit_code == Some(0)) {
            tracing::warn!("signing {old} out of GitHub on {}: {out:?}", self.id());
        }
    }

    /// Who the machine's gh is signed in as, and the repositories a limited token reaches, kept
    /// on its record so the details view can say them without waking the machine. Only for a
    /// running machine: an exec would wake it.
    pub(super) async fn refresh_github(&self) {
        if !self.up_for_background() {
            return;
        }
        let out = self.shared.a37.exec(&self.sandbox_id(), GITHUB_STATE, 30).await;
        let (login, hash) = match out {
            Ok(r) if r.exit_code == Some(0) => parse_github_state(&r.stdout).map_or((None, None), |(l, hash)| (Some(l), hash)),
            Ok(_) => (None, None),
            Err(e) => return tracing::warn!("reading {}'s GitHub login: {e:#}", self.id()),
        };
        let record = self.record();
        // A token is tested once: again only when it changes, or the last test failed.
        let reach = match hash {
            None => None,
            Some(hash) => match record.github_reach.clone() {
                Some(known) if known.token == hash => Some(known),
                _ => Some(self.test_github_reach(&hash).await),
            },
        };
        if !record.github_checked || record.github_login != login || record.github_reach != reach {
            let limited = reach.is_some();
            self.update_record(|m| {
                m.github_checked = true;
                m.github_login = login;
                m.github_reach = reach;
                // gh's own sign-in has no expiry to show.
                if !limited {
                    m.github_token_expires = None;
                }
            });
            self.shared.changed();
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

/// On the clipboard, where there's a way: `pbcopy` on macOS; on Linux Wayland's `wl-copy`, else
/// `xclip` or `xsel`, whichever is installed. Best effort.
pub(crate) fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "linux") {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    } else {
        &[]
    };
    for (program, args) in tools {
        let Ok(mut child) = std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().is_ok_and(|s| s.success()) {
            return;
        }
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
    use super::{account, callback_port, check_saved_github_token, check_token_shape, device_code, from_curl, github_token_page, parse_github_state, parse_reach, Secret};
    use serde_json::json;

    #[test]
    fn limited_github_tokens() {
        let page = github_token_page("skiff");
        assert!(page.starts_with("https://github.com/settings/personal-access-tokens/new?name=t3agent37%20skiff&description="));
        assert!(page.contains("&expires_in=90&contents=write&") && page.contains("&repository_hooks=write&") && !page.contains("checks="));
        // GitHub's limit for a name is 40 characters.
        assert!(github_token_page(&"a".repeat(32)).contains(&format!("name=t3agent37%20{}&", "a".repeat(30))));
        let fine = format!("github_pat_{}", "A1_b".repeat(20));
        assert_eq!(check_token_shape(&format!("  {fine}\n")).unwrap(), fine);
        assert!(check_token_shape(&format!("ghp_{}", "x".repeat(36))).is_ok());
        assert!(check_token_shape("github_pat_short").is_err());
        assert!(check_token_shape(&format!("gho_{}", "x".repeat(36))).is_err());
        // gh's own sign-in is for new machines, not for "Some repos only…".
        assert!(check_saved_github_token(&format!("gho_{}", "x".repeat(36))).is_ok());
        assert!(check_saved_github_token(&format!("gho_{}; x", "x".repeat(36))).is_err());
        assert!(check_saved_github_token(&fine).is_ok());
        assert!(check_token_shape(&format!("github_pat_{}; rm -rf /", "x".repeat(40))).is_err());
        assert_eq!(format!("{:?}", Secret(fine)), "Secret(…)");
        assert_eq!(parse_github_state("me\nlimited 0123456789abcdef\n"), Some(("me".into(), Some("0123456789abcdef".into()))));
        assert_eq!(parse_github_state("me\nfull\n"), Some(("me".into(), None)));
        assert_eq!(parse_github_state(""), None);
        // One owner's repos picked: listed by name.
        let picked = parse_reach("owner acme 12\nowner me 85\npush acme/web\npush acme/api\n", "h");
        assert_eq!((picked.repos.clone(), picked.all_of.clone()), (vec!["acme/api".to_owned(), "acme/web".to_owned()], None));
        assert_eq!(picked.describe(), "only api, web");
        // Every one of an owner's repos: "All repositories".
        let mut all = String::from("owner me 5\nowner acme 2\n");
        for r in ["a", "b", "c", "d", "e"] {
            all.push_str(&format!("push me/{r}\n"));
        }
        let all = parse_reach(&all, "h");
        assert_eq!(all.all_of.as_deref(), Some("me"));
        assert_eq!(all.describe(), "all 5 of me's repos");
        assert_eq!(parse_reach("owner me 3\n", "h").describe(), "can't push to any repo");
        assert_eq!(crate::store::GithubReach::default().describe(), "limited to some repos (not checked yet)");
    }

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
