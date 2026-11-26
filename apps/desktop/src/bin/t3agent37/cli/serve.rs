//! `t3agent37 serve`: the app's backend without the panel, for a computer without the app (or
//! Linux, which has none). It's what T3 Code reaches the machines through (their proxies here),
//! what puts them to sleep and wakes them, and what the command and coding agents talk to (MCP).
//! `--install` starts it at login: a launchd agent on macOS, a systemd user service on Linux.
//! And `t3agent37 setup`, which saves the Agent37 key it needs.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use t3agent37_lib::agent37::{Agent37, KEYS_PAGE};
use t3agent37_lib::app::{self, App, Setup};
use t3agent37_lib::machine::Event;
use t3agent37_lib::mcp;
use t3agent37_lib::t3desktop::T3Desktop;

const COSTS_EVERY: Duration = Duration::from_secs(300);
/// The launchd agent's label and the systemd unit's name.
pub const SERVICE: &str = "dev.t3agent37.serve";
pub const UNIT: &str = "t3agent37.service";

/// Runs until ^C or a stop from launchd or systemd.
pub async fn serve() -> Result<i32> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        // Plain in a log file (launchd's) or the journal.
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let port = mcp::port();
    // One t3agent37 at a time: the one running serves MCP here.
    if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
        eprintln!("t3agent37 is already running on this computer (the app, or another `t3agent37 serve`): port {port} is taken.");
        return Ok(1);
    }
    let dir = app::data_dir();
    let (setup, a37, t3) = app::check_setup(&dir).await;
    match setup {
        Setup::ConnectAgent37 => bail!("there's no Agent37 API key on this computer yet: run `t3agent37 setup` first"),
        Setup::InstallT3 => bail!("T3 Code isn't installed here (there's no ~/.t3/bin/t3): install it from https://t3.codes and open it once"),
        _ => {}
    }
    let (a37, t3) = a37.zip(t3).ok_or_else(|| anyhow!("t3agent37 isn't set up"))?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = App::new(a37, t3, dir, tx).context("starting t3agent37")?;
    tokio::spawn(events(app.clone(), rx));
    tokio::spawn(background(app.clone()));
    let mcp_app = app.clone();
    let mut mcp = tokio::spawn(async move { mcp::serve(mcp_app).await });
    let count = app.views().len();
    tracing::info!(
        "t3agent37 {} serves {count} machine{} for T3 Code, and the command and coding agents at {}",
        env!("CARGO_PKG_VERSION"),
        if count == 1 { "" } else { "s" },
        mcp::url()
    );
    let result = tokio::select! {
        r = &mut mcp => match r {
            Ok(Err(e)) => Err(e),
            _ => Err(anyhow!("t3agent37's local address stopped")),
        },
        _ = stopped() => Ok(0),
    };
    app.save();
    tracing::info!("t3agent37 stopped; its machines carry on, and sleep as usual");
    result
}

/// ^C, or SIGTERM from launchd or systemd.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("a SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

/// The backend's events: saves, notifications, and each machine's state as it changes.
async fn events(app: Arc<App>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Event>) {
    let mut states: HashMap<String, String> = HashMap::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Persist => app.save(),
            Event::Notify { title, body } => {
                tracing::info!("{title}: {body}");
                notify(&title, &body);
            }
            Event::Changed => {
                for m in app.views() {
                    let state = serde_json::to_value(m.state).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
                    if states.get(&m.name) != Some(&state) {
                        tracing::info!("{}: {state} · {}", m.name, m.detail);
                        states.insert(m.name.clone(), state);
                    }
                }
            }
        }
    }
}

/// Costs change slowly; and the "awake for hours" alerts, and links between machines that run
/// out soon, made again.
async fn background(app: Arc<App>) {
    let alerts = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            alerts.every_minute().await;
        }
    });
    loop {
        app.refresh_costs().await;
        app.renew_links().await;
        tokio::time::sleep(COSTS_EVERY).await;
    }
}

/// A desktop notification where there's a way, besides the log: `osascript` on macOS (shown as
/// Script Editor's), `notify-send` on Linux.
fn notify(title: &str, body: &str) {
    if cfg!(target_os = "macos") {
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let script = format!("display notification {} with title {}", quote(body), quote(title));
        let _ = std::process::Command::new("osascript").args(["-e", &script]).spawn();
    } else if cfg!(target_os = "linux") {
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=t3agent37", title, body])
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// `t3agent37 setup`: checks the Agent37 API key with Agent37 and keeps it on this computer.
pub async fn setup() -> Result<i32> {
    let dir = app::data_dir();
    if Agent37::load(&dir).is_some() {
        println!("t3agent37 already has an Agent37 API key on this computer.");
        println!("To use another, remove {} first.", dir.join("agent37-key").display());
        return Ok(0);
    }
    if std::io::stdin().is_terminal() {
        println!("Make an API key in Agent37's dashboard, {KEYS_PAGE}, and paste it here.");
        println!("It stays on this computer (readable by you only) and never goes to a machine.");
    }
    let key = read_secret("Agent37 API key: ")?;
    if key.is_empty() {
        bail!("no key given");
    }
    Agent37::save(&dir, &key).await.context("checking the key with Agent37")?;
    println!("Saved: the key works.");
    if T3Desktop::locate().is_none() {
        println!("Now install T3 Code (https://t3.codes) and open it once: t3agent37 adds your machines to it.");
    }
    println!("Next: `t3agent37 serve` runs t3agent37 (`t3agent37 serve --install` starts it at login), then `t3agent37 create <name>`.");
    Ok(0)
}

/// A line from the terminal without echoing it, or from stdin when that isn't a terminal
/// (`t3agent37 setup < key.txt`).
pub(super) fn read_secret(prompt: &str) -> Result<String> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        return Ok(line.trim().to_owned());
    }
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let stty = |arg: &str| std::process::Command::new("stty").arg(arg).stdin(std::process::Stdio::inherit()).status();
    let hidden = stty("-echo").is_ok_and(|s| s.success());
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if hidden {
        let _ = stty("echo");
    }
    eprintln!();
    read?;
    Ok(line.trim().to_owned())
}

/// `serve --install`: t3agent37 starts at login, and now.
pub fn install() -> Result<i32> {
    let exe = std::env::current_exe().context("finding this program")?;
    let path = std::env::var("PATH").unwrap_or_default();
    let busy = std::net::TcpListener::bind(("127.0.0.1", mcp::port())).is_err();
    if cfg!(target_os = "macos") {
        let plist = launch_agent()?;
        let log = dirs::home_dir().context("no home folder")?.join("Library/Logs/t3agent37.log");
        std::fs::create_dir_all(plist.parent().unwrap())?;
        std::fs::write(&plist, launchd_plist(&exe, &path, &log))?;
        let domain = format!("gui/{}", uid()?);
        // Again over an older one: out first.
        let _ = std::process::Command::new("launchctl").args(["bootout", &domain]).arg(&plist).stderr(std::process::Stdio::null()).status();
        run("launchctl", &["bootstrap", &domain, &plist.to_string_lossy()])?;
        println!("t3agent37 now starts at login ({}).", plist.display());
        println!("Its log: {}", log.display());
    } else if cfg!(target_os = "linux") {
        let unit = systemd_unit_path()?;
        std::fs::create_dir_all(unit.parent().unwrap())?;
        std::fs::write(&unit, systemd_unit(&exe, &path))?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", UNIT])?;
        println!("t3agent37 now starts at login ({}).", unit.display());
        println!("Its log: journalctl --user -u t3agent37");
    } else {
        bail!("--install is for macOS and Linux; run `t3agent37 serve` yourself here");
    }
    if busy {
        println!(
            "Another t3agent37 runs here already (the app?), so this one stops at once: quit that one, and turn off its \
             Start at login, for this one to take over."
        );
    }
    Ok(0)
}

/// `serve --uninstall`: no more starting at login; stops the one it started.
pub fn uninstall() -> Result<i32> {
    if cfg!(target_os = "macos") {
        let plist = launch_agent()?;
        if !plist.exists() {
            println!("t3agent37 wasn't set to start at login.");
            return Ok(0);
        }
        let domain = format!("gui/{}", uid()?);
        let _ = std::process::Command::new("launchctl").args(["bootout", &domain]).arg(&plist).status();
        std::fs::remove_file(&plist)?;
    } else if cfg!(target_os = "linux") {
        let unit = systemd_unit_path()?;
        if !unit.exists() {
            println!("t3agent37 wasn't set to start at login.");
            return Ok(0);
        }
        let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", UNIT]).status();
        std::fs::remove_file(&unit)?;
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    } else {
        bail!("--uninstall is for macOS and Linux");
    }
    println!("t3agent37 no longer starts at login, and the one it started has stopped.");
    Ok(0)
}

fn launch_agent() -> Result<PathBuf> {
    Ok(dirs::home_dir().context("no home folder")?.join(format!("Library/LaunchAgents/{SERVICE}.plist")))
}

fn systemd_unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir().context("no config folder")?.join("systemd/user").join(UNIT))
}

pub fn uid() -> Result<String> {
    let out = std::process::Command::new("id").arg("-u").output().context("running id -u")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program).args(args).status().with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Started at login and kept up if it crashes, not when it stops by itself (another
/// t3agent37 already runs, say). With the PATH it was installed from, for the agents' CLIs.
fn launchd_plist(exe: &std::path::Path, path: &str, log: &std::path::Path) -> String {
    let (exe, path, log) = (xml(&exe.to_string_lossy()), xml(path), xml(&log.to_string_lossy()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{SERVICE}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>serve</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path}</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>Crashed</key>
        <true/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

fn systemd_unit(exe: &std::path::Path, path: &str) -> String {
    // systemd takes `%` as a specifier and quotes for spaces.
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"));
    format!(
        "[Unit]\n\
         Description=t3agent37: T3 Code machines on Agent37 Cloud that sleep when idle\n\
         Wants=network-online.target\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={} serve\n\
         Environment={}\n\
         Restart=on-abnormal\n\
         RestartSec=30\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        quote(&exe.to_string_lossy()),
        quote(&format!("PATH={path}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_files() {
        let plist = launchd_plist(std::path::Path::new("/Users/a b/bin/t3agent37"), "/usr/bin:/a&b", std::path::Path::new("/tmp/t.log"));
        assert!(plist.contains("<string>/Users/a b/bin/t3agent37</string>\n        <string>serve</string>"));
        assert!(plist.contains("<string>/usr/bin:/a&amp;b</string>"));
        let unit = systemd_unit(std::path::Path::new("/home/a b/.local/bin/t3agent37"), "/usr/bin:/100%");
        assert!(unit.contains("ExecStart=\"/home/a b/.local/bin/t3agent37\" serve\n"), "{unit}");
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/100%%\"\n"), "{unit}");
    }
}
