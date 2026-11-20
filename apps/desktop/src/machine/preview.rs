//! A machine's ports on this computer: `localhost:<port>` here reaches that port on the machine,
//! so a dev server an agent started there opens in the user's own browser, devtools and all.
//! Each connection rides T3's own preview tunnel (`/api/preview/tunnel?port=`, a WebSocket of raw
//! bytes to the machine's loopback), through Agent37's edge like the rest of T3: nothing new runs
//! on the machine and no port is opened there.
//!
//! Waking follows the proxy's rule: only what a person does wakes a sleeping machine. A page
//! opened or reloaded in a browser (a top-level navigation) wakes it and waits; anything else a
//! page sends on its own (a dev server's reconnect pings, a fetch) gets its connection closed, so
//! a tab left open never wakes it again and again. A new connection counts as using the machine;
//! traffic on a long-lived one (a hot-reload socket) doesn't, so watching a page doesn't keep it up.

use super::{MachineRuntime, Power, WatchdogPhase};
use crate::proxy::upstream;
use crate::store::Preview;
use anyhow::{bail, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{self, Message};

/// A server T3 found listening on the machine.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalServer {
    pub port: u16,
    pub process: Option<String>,
    /// The thread whose agent or terminal started it.
    pub thread_id: Option<String>,
}

/// One forwarded port: its listener here, which stops when this is dropped.
pub struct Forward {
    pub local_port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The machine's own: T3, the watchdog, its heartbeat and the wake link.
const OWN_PORTS: [u16; 4] = [3773, 3774, 3775, 3776];
/// t3agent37's own here: the MCP server and the machines' proxies.
const RESERVED_HERE: std::ops::Range<u16> = 47370..48371;
const SERVERS_FRESH: Duration = Duration::from_secs(10);
/// How long a new connection gets to send its request before it counts as not HTTP.
const HEAD_WAIT: Duration = Duration::from_millis(800);
const HEAD_MAX: usize = 16 * 1024;
const CHUNK: usize = 64 * 1024;

impl MachineRuntime {
    /// Brings back the forwards saved on its record, at launch.
    pub fn start_forwards(self: &Arc<Self>) {
        for p in self.record().previews {
            if let Err(e) = self.bind_forward(p.port, Some(p.local_port)) {
                tracing::warn!("forwarding port {} of {}: {e:#}", p.port, self.id());
            }
        }
    }

    /// Makes `port` on the machine reachable at `localhost:<the returned port>` here: the same
    /// number when it's free, so URLs a project has configured (OAuth callbacks, say) work.
    pub fn forward(self: &Arc<Self>, port: u16) -> Result<u16> {
        if port == 0 || OWN_PORTS.contains(&port) {
            bail!("port {port} is t3agent37's own on the machine");
        }
        if let Some(f) = self.forwards.lock().unwrap().get(&port) {
            return Ok(f.local_port);
        }
        let saved = self.record().previews.iter().find(|p| p.port == port).map(|p| p.local_port);
        let local_port = self.bind_forward(port, saved)?;
        self.update_record(|r| {
            r.previews.retain(|p| p.port != port);
            r.previews.push(Preview { port, local_port });
        });
        Ok(local_port)
    }

    pub fn unforward(&self, port: u16) {
        self.forwards.lock().unwrap().remove(&port);
        self.update_record(|r| r.previews.retain(|p| p.port != port));
    }

    pub(super) fn stop_forwards(&self) {
        self.forwards.lock().unwrap().clear();
    }

    fn bind_forward(self: &Arc<Self>, port: u16, saved: Option<u16>) -> Result<u16> {
        let listener = bind_local(port, saved)?;
        listener.set_nonblocking(true)?;
        let local_port = listener.local_addr()?.port();
        let listener = TcpListener::from_std(listener)?;
        // Weak: the listener lives in the machine, and must not keep a deleted one alive.
        let machine: Weak<Self> = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                let Some(m) = machine.upgrade() else { return };
                tokio::spawn(async move {
                    if let Err(e) = m.serve_preview(client, port).await {
                        tracing::debug!("preview of port {port}: {e:#}");
                    }
                });
            }
        });
        self.forwards.lock().unwrap().insert(port, Forward { local_port, task });
        Ok(local_port)
    }

    /// Up and staying up: a connection goes straight through.
    fn up_for_preview(&self) -> bool {
        self.power() == Power::Running
            && !self.is_stopping()
            && self.watchdog_status().is_none_or(|w| w.phase == WatchdogPhase::Active)
    }

    async fn serve_preview(self: Arc<Self>, mut client: TcpStream, port: u16) -> Result<()> {
        let _ = client.set_nodelay(true);
        let name = self.record().name;
        let mut head = Vec::new();
        if !self.up_for_preview() {
            read_head(&mut client, &mut head).await;
            match kind(&head) {
                Kind::Page => {
                    tracing::info!("waking {} for a page on port {port}", self.id());
                    if let Err(e) = self.ensure_awake().await {
                        return respond(&mut client, "503 Service Unavailable", &format!("{name} could not wake: {e:#}")).await;
                    }
                }
                // Closed without an answer: a reconnecting page retries quietly, and an answer
                // of any kind would tell it the server is back (Vite then reloads, which wakes).
                Kind::Background | Kind::NotHttp => return Ok(()),
            }
        }
        let target = format!("/api/preview/tunnel?port={port}");
        let ws = match upstream::connect(&self.upstream_url(), &self.upstream_token(), &target).await {
            Ok(ws) => ws,
            Err(e) => {
                if head.is_empty() {
                    read_head(&mut client, &mut head).await;
                }
                if kind(&head) != Kind::NotHttp {
                    let text = match refusal(&e) {
                        Some(Refusal::Port) => format!("Nothing is listening on port {port} on {name}."),
                        Some(Refusal::Off) => format!("T3's preview tunnel is turned off on {name} (T3 Code settings)."),
                        None => format!("Couldn't reach port {port} on {name}: {e:#}"),
                    };
                    respond(&mut client, "502 Bad Gateway", &text).await?;
                }
                return Ok(());
            }
        };
        self.keep_up();
        pipe(client, ws, head).await
    }

    /// The servers T3 saw listening on the machine when it last looked. Never wakes it.
    pub fn local_servers(&self) -> Vec<LocalServer> {
        self.servers.lock().unwrap().0.clone()
    }

    /// Asks T3 on the machine again which servers listen there, if it runs and the last look is
    /// a few seconds old.
    pub async fn refresh_servers(&self) {
        if !self.up_for_background() {
            return;
        }
        {
            let mut servers = self.servers.lock().unwrap();
            if servers.1.is_some_and(|t| t.elapsed() < SERVERS_FRESH) {
                return;
            }
            servers.1 = Some(Instant::now());
        }
        let read = upstream::subscribe_until(
            &self.upstream_url(),
            &self.upstream_token(),
            "subscribeDiscoveredLocalServers",
            json!({}),
            Duration::from_secs(8),
            |item| Some(item.clone()),
        )
        .await;
        let list = match read {
            Ok((Some(item), _)) | Ok((None, item)) => {
                tracing::debug!("servers on {}: {}", self.id(), item.to_string().chars().take(600).collect::<String>());
                parse_servers(&item)
            }
            Err(e) => return tracing::debug!("reading servers on {}: {e:#}", self.id()),
        };
        let changed = {
            let mut servers = self.servers.lock().unwrap();
            let changed = servers.0 != list;
            servers.0 = list;
            changed
        };
        if changed {
            self.shared.changed();
        }
    }
}

/// The port asked for when it's free here (and on IPv6 too: a browser tries `localhost` there
/// first), else the next few, else any.
fn bind_local(port: u16, saved: Option<u16>) -> Result<std::net::TcpListener> {
    let wanted = saved.into_iter().chain((0..10).filter_map(|i| port.checked_add(i)));
    for p in wanted.filter(|p| *p >= 1024 && !RESERVED_HERE.contains(p)) {
        if taken_here(p) {
            continue;
        }
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", p)) {
            return Ok(listener);
        }
    }
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?)
}

/// Something here already answers on `port`: binding it anyway could shadow it.
fn taken_here(port: u16) -> bool {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    let wait = Duration::from_millis(150);
    [SocketAddr::from((Ipv4Addr::LOCALHOST, port)), SocketAddr::from((Ipv6Addr::LOCALHOST, port))]
        .iter()
        .any(|addr| std::net::TcpStream::connect_timeout(addr, wait).is_ok())
}

#[derive(Debug, PartialEq)]
enum Kind {
    /// A page a person opens or reloads in a browser.
    Page,
    /// HTTP a page or a program sends on its own.
    Background,
    NotHttp,
}

/// What a connection's first bytes are.
fn kind(head: &[u8]) -> Kind {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (method, _target, version) = (first.next().unwrap_or_default(), first.next(), first.next());
    let http = !method.is_empty() && method.bytes().all(|b| b.is_ascii_uppercase()) && version.is_some_and(|v| v.starts_with("HTTP/1."));
    if !http {
        return Kind::NotHttp;
    }
    let header = |name: &str| {
        lines.clone().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_ascii_lowercase())
        })
    };
    let navigates = header("sec-fetch-mode").as_deref() == Some("navigate") && header("sec-fetch-dest").as_deref() == Some("document");
    if method == "GET" && navigates { Kind::Page } else { Kind::Background }
}

/// Reads up to the end of an HTTP request's head, or what comes in `HEAD_WAIT`.
async fn read_head(client: &mut TcpStream, head: &mut Vec<u8>) {
    let deadline = tokio::time::Instant::now() + HEAD_WAIT;
    let mut buf = [0u8; 4096];
    while head.len() < HEAD_MAX && !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match tokio::time::timeout_at(deadline, client.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => head.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
}

enum Refusal {
    /// Nothing listens on the port.
    Port,
    /// The tunnel is turned off in T3's settings there.
    Off,
}

fn refusal(e: &anyhow::Error) -> Option<Refusal> {
    e.chain().find_map(|c| match c.downcast_ref::<tungstenite::Error>() {
        Some(tungstenite::Error::Http(res)) if res.headers().contains_key("x-t3-tunnel-port-refused") => Some(Refusal::Port),
        Some(tungstenite::Error::Http(res)) if res.headers().contains_key("x-t3-tunnel-disabled") => Some(Refusal::Off),
        _ => None,
    })
}

/// A short page, then the connection closes.
async fn respond(client: &mut TcpStream, status: &str, text: &str) -> Result<()> {
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta name=color-scheme content=\"light dark\"><title>t3agent37</title>\
         <p style=\"font:15px/1.5 -apple-system,system-ui,sans-serif;margin:48px auto;max-width:520px;padding:0 16px\">{escaped}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(response.as_bytes()).await?;
    let _ = client.shutdown().await;
    Ok(())
}

/// Bytes both ways until either end closes, or the machine goes silent (asleep).
async fn pipe(client: TcpStream, ws: upstream::Upstream, head: Vec<u8>) -> Result<()> {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (mut from_client, mut to_client) = client.into_split();
    if !head.is_empty() {
        ws_tx.send(Message::binary(head)).await?;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let up = async move {
        let mut ping = tokio::time::interval(upstream::PING);
        ping.tick().await;
        loop {
            tokio::select! {
                chunk = rx.recv() => match chunk {
                    Some(chunk) => ws_tx.send(Message::binary(chunk)).await?,
                    None => {
                        let _ = ws_tx.send(Message::Close(None)).await;
                        return anyhow::Ok(());
                    }
                },
                _ = ping.tick() => ws_tx.send(Message::Ping(Vec::new().into())).await?,
            }
        }
    };
    let read_client = async move {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = from_client.read(&mut buf).await?;
            if n == 0 || tx.send(buf[..n].to_vec()).await.is_err() {
                return anyhow::Ok(());
            }
        }
    };
    let down = async move {
        loop {
            match tokio::time::timeout(upstream::SILENCE, ws_rx.next()).await {
                Ok(Some(Ok(Message::Binary(b)))) => to_client.write_all(&b).await?,
                Ok(Some(Ok(Message::Text(t)))) => to_client.write_all(t.as_bytes()).await?,
                Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                Ok(Some(Ok(_))) => {}
            }
        }
        let _ = to_client.shutdown().await;
        anyhow::Ok(())
    };
    tokio::select! {
        r = async { tokio::try_join!(up, read_client) } => r.map(drop),
        r = down => r,
    }
}

/// T3's `DiscoveredLocalServerList` (or its servers alone), without the machine's own ports.
fn parse_servers(v: &Value) -> Vec<LocalServer> {
    let list = v["servers"].as_array().or(v.as_array()).cloned().unwrap_or_default();
    let mut out: Vec<LocalServer> = Vec::new();
    for s in list {
        let Some(port) = s["port"].as_u64().and_then(|p| u16::try_from(p).ok()) else { continue };
        if OWN_PORTS.contains(&port) || out.iter().any(|o| o.port == port) {
            continue;
        }
        out.push(LocalServer {
            port,
            process: s["processName"].as_str().map(str::to_owned),
            thread_id: s["agent"]["threadId"].as_str().or(s["terminal"]["threadId"].as_str()).map(str::to_owned),
        });
    }
    out.sort_by_key(|s| s.port);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_page_opened_in_a_browser_wakes() {
        let page = b"GET / HTTP/1.1\r\nHost: localhost:5173\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n\r\n";
        assert_eq!(kind(page), Kind::Page);
        let ping = b"GET / HTTP/1.1\r\nHost: localhost:5173\r\nAccept: text/x-vite-ping\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: empty\r\n\r\n";
        assert_eq!(kind(ping), Kind::Background);
        let socket = b"GET /?token=x HTTP/1.1\r\nUpgrade: websocket\r\nSec-Fetch-Mode: websocket\r\n\r\n";
        assert_eq!(kind(socket), Kind::Background);
        let frame = b"GET /a HTTP/1.1\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: iframe\r\n\r\n";
        assert_eq!(kind(frame), Kind::Background);
        assert_eq!(kind(b"curl-less garbage"), Kind::NotHttp);
        assert_eq!(kind(b""), Kind::NotHttp);
        assert_eq!(kind(&[0, 0, 0, 8, 4, 210, 22, 47]), Kind::NotHttp);
    }

    #[test]
    fn reads_t3s_server_list() {
        let v = json!({"servers": [
            {"host": "127.0.0.1", "port": 5173, "url": "http://localhost:5173", "processName": "node", "pid": 9, "terminal": null, "agent": {"threadId": "t1"}},
            {"host": "0.0.0.0", "port": 3773, "url": "http://localhost:3773", "processName": "t3", "pid": 2, "terminal": null},
            {"host": "::1", "port": 3000, "url": "http://localhost:3000", "processName": null, "pid": null, "terminal": {"threadId": "t2", "terminalId": "x"}},
        ], "scannedAt": "2026-11-20T00:00:00Z"});
        let servers = parse_servers(&v);
        assert_eq!(servers.iter().map(|s| s.port).collect::<Vec<_>>(), [3000, 5173]);
        assert_eq!(servers[0].thread_id.as_deref(), Some("t2"));
        assert_eq!(servers[1].process.as_deref(), Some("node"));
    }
}
