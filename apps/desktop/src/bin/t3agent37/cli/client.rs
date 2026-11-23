//! The t3agent37 running on this computer, the app or `t3agent37 serve`, as the command reaches
//! it: the tools coding agents call over MCP, at its `/cli` address, which answers once a call
//! is done. An older t3agent37 has no `/cli`: its MCP tools answer within 40 s, then go on in
//! the background.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::time::Duration;
use t3agent37_lib::{agent37::Agent37, app, mcp};

#[derive(Clone)]
pub struct Backend {
    http: reqwest::Client,
    base: String,
}

impl Backend {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            // On this computer: never through a proxy from the environment.
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .expect("an HTTP client");
        Self { http, base: format!("http://127.0.0.1:{}", mcp::port()) }
    }

    /// A tool's answer (text: a sentence, or JSON for the lists), once it's done.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<String> {
        let res = self
            .http
            .post(format!("{}/cli", self.base))
            .json(&json!({ "name": tool, "arguments": arguments }))
            .send()
            .await
            .map_err(unreachable)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return self.call_mcp(tool, arguments).await;
        }
        let body: Value = res.json().await.context("reading t3agent37's answer")?;
        if body["ok"] == true {
            return Ok(body["text"].as_str().unwrap_or_default().to_owned());
        }
        bail!("{}", body["error"].as_str().unwrap_or("t3agent37 couldn't do that"))
    }

    /// A tool's JSON answer: the lists and the details.
    pub async fn json(&self, tool: &str, arguments: Value) -> Result<Value> {
        let text = self.call(tool, arguments).await?;
        serde_json::from_str(&text).with_context(|| format!("t3agent37's answer to {tool}"))
    }

    /// The running t3agent37's version, from its MCP handshake.
    pub async fn version(&self) -> Result<String> {
        let r = self.rpc("initialize", json!({ "protocolVersion": "2025-06-18" })).await?;
        Ok(r["serverInfo"]["version"].as_str().unwrap_or("?").to_owned())
    }

    async fn call_mcp(&self, tool: &str, arguments: Value) -> Result<String> {
        let r = self.rpc("tools/call", json!({ "name": tool, "arguments": arguments })).await?;
        let text = r["content"][0]["text"].as_str().unwrap_or_default().to_owned();
        if r["isError"] == true {
            bail!("{text}");
        }
        Ok(text)
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let res = self.http.post(format!("{}/mcp", self.base)).json(&body).send().await.map_err(unreachable)?;
        let r: Value = res.json().await.context("reading t3agent37's answer")?;
        if let Some(message) = r["error"]["message"].as_str() {
            bail!("{message}");
        }
        Ok(r["result"].clone())
    }
}

/// Nothing answers here: t3agent37 isn't running, or isn't set up, so it doesn't serve yet.
fn unreachable(e: reqwest::Error) -> anyhow::Error {
    if !e.is_connect() {
        return anyhow!("t3agent37 didn't answer: {e}");
    }
    if Agent37::load(&app::data_dir()).is_none() {
        return anyhow!("t3agent37 isn't set up on this computer yet: run `t3agent37 setup`.");
    }
    anyhow!(
        "t3agent37 isn't running on this computer. Start it with `t3agent37 serve` (or `t3agent37 serve --install` to \
         start it at login), or open the t3agent37 app."
    )
}
