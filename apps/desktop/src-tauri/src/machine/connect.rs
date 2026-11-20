//! T3 Connect: T3's own way for other devices (a phone, a browser) to reach a machine with the
//! user's T3 account, through a tunnel the machine dials out. It is set up on the machine itself
//! with T3's `t3 up --connect`, which t3agent37 runs there: its sign-in page opens here. T3 applies
//! the link as it starts, so t3agent37 then restarts it, and once the tunnel is up it turns on the
//! one setting a sleeping machine needs.

use super::MachineRuntime;
use anyhow::{bail, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// T3's device sign-in lasts 10 minutes; then the restart and the first tunnel connection.
const SETUP_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const URL_TIMEOUT: Duration = Duration::from_secs(120);
/// After the restart: the link with T3's cloud, then the tunnel's first connection.
const LINK_TIMEOUT: Duration = Duration::from_secs(120);

impl MachineRuntime {
    /// Resolves once other devices can reach the machine through T3 Connect, after the user
    /// signed in to their T3 account in the browser.
    pub async fn turn_on_connect(&self) -> Result<()> {
        self.ensure_awake().await?;
        // `t3 up` restarts T3 to start the tunnel, which stops running agent turns.
        if self.watchdog_get("/status?fresh").await?.busy {
            bail!("an agent is working; turn T3 Connect on once it finishes");
        }
        self.kept_awake(async {
            self.run_connect_setup().await?;
            // T3 links the machine as it starts. `t3 up` restarts T3's own background service for
            // that, but here T3 runs under t3agent37's boot hook instead, so restart it here.
            self.restart_t3().await?;
            self.wait_connect_linked().await
        })
        .await?;
        // A webhook sent while the machine sleeps is then held by T3 Connect's relay and run as
        // it wakes, instead of failing.
        let state = self.connect_api(Method::GET, "link-state", None).await?;
        let preferences = json!({
            "publishAgentActivity": state["publishAgentActivity"].as_bool().unwrap_or(false),
            "holdWebhooksWhileOffline": true,
        });
        self.connect_api(Method::POST, "preferences", Some(preferences)).await?;
        self.update_record(|r| r.connect = true);
        Ok(())
    }

    /// The machine's wake link, for a phone that reaches it through T3 Connect: T3 Connect can't
    /// wake a sleeping machine, and this link does, then holds it up long enough to pick it in
    /// T3 Code. It wakes and nothing else, so using the machine still takes the user's own T3
    /// sign-in. `renew` replaces it; the old link stops working.
    pub async fn wake_link(&self, renew: bool) -> Result<String> {
        // Older watchdogs don't answer the link: take this app's scripts first, as keep-awake does.
        if self.record().provision_revision != super::provision::revision() {
            self.ensure_awake().await?;
            if self.watchdog_get("/status?fresh").await?.busy {
                bail!("needs a quick update first; try again once its agents finish");
            }
            self.kept_awake(self.upgrade_if_needed()).await?;
        }
        super::provision::wake_url(&self.shared.a37, &self.sandbox_id(), renew).await
    }

    /// Reads whether T3 Connect is on, for a machine that just woke: it can be turned on or off
    /// on the machine too.
    pub(super) async fn refresh_connect(&self) {
        if !self.up_for_background() {
            return;
        }
        match self.connect_api(Method::GET, "link-state", None).await {
            Ok(state) => {
                let on = state["linked"] == true && state["managedTunnelActive"] == true;
                if self.record().connect != on {
                    self.update_record(|r| r.connect = on);
                }
            }
            Err(e) => tracing::warn!("reading T3 Connect state of {}: {e:#}", self.id()),
        }
    }

    async fn run_connect_setup(&self) -> Result<()> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        // In the background, so the sign-in page can be read while it waits; `exit` says how it ended.
        let dir = a37
            .exec_ok(
                &id,
                "d=$(mktemp -d); \
                 (\"$HOME/.local/bin/t3\" up --connect --headless --yes > \"$d/log\" 2>&1; echo $? > \"$d/exit\") \
                 </dev/null >/dev/null 2>&1 & echo \"$d\"",
                30,
            )
            .await?;
        let dir = dir.lines().last().unwrap_or_default().trim().to_owned();
        let result = self.finish_connect_setup(&dir).await;
        let _ = a37.exec(&id, &format!("rm -rf '{dir}'"), 30).await;
        result
    }

    async fn finish_connect_setup(&self, dir: &str) -> Result<()> {
        let id = self.sandbox_id();
        let a37 = &self.shared.a37;
        let started = Instant::now();
        let mut opened = false;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let out = a37
                .exec(&id, &format!("cat '{dir}/exit' 2>/dev/null; echo ---; cat '{dir}/log'"), 30)
                .await?;
            let (exit, log) = out.stdout.split_once("---").unwrap_or_default();
            if !opened && let Some(url) = log.split_whitespace().find(|w| w.starts_with("https://")) {
                crate::app::open_url(url)?;
                opened = true;
            }
            match exit.trim() {
                "" => {}
                "0" => return Ok(()),
                _ => bail!("T3 Connect setup failed: {}", last_line(log)),
            }
            if !opened && started.elapsed() > URL_TIMEOUT {
                bail!("T3 Connect setup did not reach its sign-in: {}", last_line(log));
            }
            if started.elapsed() > SETUP_TIMEOUT {
                bail!("the T3 Connect sign-in was not finished in the browser");
            }
        }
    }

    /// Restarts the T3 server on the machine and waits until it answers again. boot.sh starts a
    /// new one as soon as the old one exits.
    async fn restart_t3(&self) -> Result<()> {
        let out = self
            .shared
            .a37
            .exec(
                &self.sandbox_id(),
                "d=/root/.t3agent37; pid=$(cat $d/t3.pid 2>/dev/null); [ -n \"$pid\" ] || { echo 'T3 is not running'; exit 1; }; \
                 kill -TERM -- -$pid 2>/dev/null; \
                 for _ in $(seq 1 120); do [ \"$(cat $d/t3.pid 2>/dev/null)\" != \"$pid\" ] && break; sleep 0.5; done; \
                 [ \"$(cat $d/t3.pid 2>/dev/null)\" != \"$pid\" ] || kill -KILL -- -$pid 2>/dev/null; \
                 for _ in $(seq 1 240); do curl -sf -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment && exit 0; sleep 0.5; done; \
                 echo 'T3 did not come back'; exit 1",
                240,
            )
            .await?;
        if out.exit_code != Some(0) {
            bail!("restarting T3 for T3 Connect: {}", last_line(&out.stdout));
        }
        Ok(())
    }

    /// Waits for the restarted server to link the machine and bring its tunnel up.
    async fn wait_connect_linked(&self) -> Result<()> {
        let started = Instant::now();
        loop {
            // the server is up again, but its link and tunnel come a moment later
            if let Ok(state) = self.connect_api(Method::GET, "link-state", None).await
                && state["linked"] == true
                && state["managedTunnelActive"] == true
            {
                return Ok(());
            } else if started.elapsed() > LINK_TIMEOUT {
                let state = self.connect_api(Method::GET, "link-state", None).await.unwrap_or_default();
                let reason = state["reachability"]["reason"].as_str().unwrap_or("its tunnel did not come up");
                bail!("T3 Connect is not reachable yet: {reason}");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// T3's `/api/connect/*` routes on the machine.
    async fn connect_api(&self, method: Method, route: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}/api/connect/{route}", self.upstream_url());
        let mut req = self
            .shared
            .http
            .request(method, url)
            .bearer_auth(self.upstream_token())
            .timeout(Duration::from_secs(15));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await?;
        let status = res.status();
        if !status.is_success() {
            // T3 says what was wrong in `message`, e.g. "Link this environment to T3 Connect first."
            let text = res.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if message.is_empty() {
                bail!("T3 Connect {route}: HTTP {status}");
            }
            bail!("T3 Connect {route}: {message} (HTTP {})", status.as_u16());
        }
        Ok(res.json().await?)
    }
}

fn last_line(log: &str) -> &str {
    log.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no output")
}
