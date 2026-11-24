//! Headless driver for development: runs the app model and proxies without the tray.
//! Reads commands from stdin: list | create <name> [size] | pair <id> | wake <id> | sleep <id> |
//! keep <id> [minutes] (keep awake; none: until stopped, 0 ends it) | connect <id> | delete <id> | costs |
//! code <id> (a pairing URL, T3 Code left alone) | state <id> | github <id> [another] (sign its git in to GitHub; another: in a private window) |
//! elsewhere (the account's t3agent37 machines this data folder doesn't have) | adopt <id|name> (adds one) |
//! settings <id> (what the machine keeps for every computer: issues, start URL) | sync <id> (reads them again)

use std::sync::Arc;
use t3agent37_lib::app::{self, App, Setup};
use t3agent37_lib::machine::Event;
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info,t3agent37_lib=debug".into()))
        .init();
    let dir = std::env::var("T3AGENT37_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| dirs::data_dir().unwrap().join("t3agent37-dev"));
    // The key is read from `<dir>/agent37-key`; T3AGENT37_KEY_FILE checks and saves one there.
    if let Ok(file) = std::env::var("T3AGENT37_KEY_FILE") {
        t3agent37_lib::agent37::Agent37::save(&dir, &std::fs::read_to_string(file)?).await?;
    }
    let (setup, a37, t3) = app::check_setup(&dir).await;
    if setup != Setup::Ready {
        anyhow::bail!("setup incomplete: {setup:?}");
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let app = App::new(a37.unwrap(), t3.unwrap(), dir, tx)?;
    let saver = app.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Persist => saver.save(),
                Event::Changed => {}
                Event::Notify { title, body } => println!("NOTIFY {title}: {body}"),
            }
        }
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(e) = command(&app, &words).await {
                println!("ERROR {e:#}");
            }
        });
    }
    std::future::pending::<()>().await;
    Ok(())
}

async fn command(app: &Arc<App>, words: &[String]) -> anyhow::Result<()> {
    let arg = |i: usize| words.get(i).cloned().unwrap_or_default();
    match words.first().map(String::as_str) {
        Some("list") => {
            for v in app.views() {
                println!("{} {} [{:?}] {} paired={}", v.id, v.name, v.state, v.detail, v.paired);
            }
        }
        Some("create") => {
            let size = words.get(2).cloned().unwrap_or_else(|| "small".into());
            let m = app.create_machine(&arg(1), &size, Vec::new(), None).await?;
            println!("CREATED {} on port {}", m.id(), m.record().local_port);
        }
        Some("pair") => app.pair(&*app.machine(&arg(1))?)?,
        // A pairing URL for the machine's proxy, without opening T3 Code.
        Some("code") => {
            let m = app.machine(&arg(1))?;
            println!("PAIRING http://localhost:{}/pair#token={}", m.record().local_port, m.auth.new_pairing_code());
        }
        Some("state") => {
            let m = app.machine(&arg(1))?;
            println!("POWER {:?} WATCHDOG {:?}", m.power(), m.watchdog_status().map(|w| (w.phase, w.busy, w.idle_secs)));
        }
        Some("upgrade") => app.machine(&arg(1))?.upgrade_if_needed().await?,
        Some("elsewhere") => {
            app.refresh_elsewhere().await;
            for m in app.elsewhere() {
                println!("{} {} size={:?} asleep={}", m.id, m.name, m.size, m.asleep);
            }
            println!("elsewhere: {} machine(s)", app.elsewhere().len());
        }
        Some("adopt") => {
            let m = app.add_machine_here(&arg(1)).await?;
            println!("adopted {} ({}) on port {}", m.record().name, m.id(), m.record().local_port);
        }
        Some("sync") => app.machine(&arg(1))?.refresh_settings().await,
        Some("settings") => {
            let r = app.machine(&arg(1))?.record();
            let issues: Vec<String> = r.issues.iter().map(|w| format!("{} {} {}", w.repo, w.settings.label, w.settings.model)).collect();
            println!("issues: {issues:?} start: {} inbox: {}", r.start.is_some(), r.inbox_url.is_some());
        }
        Some("wake") => app.machine(&arg(1))?.ensure_awake().await?,
        Some("sleep") => app.machine(&arg(1))?.sleep_now().await?,
        Some("connect") => app.machine(&arg(1))?.turn_on_connect().await?,
        Some("github") => app.machine(&arg(1))?.sign_in_github(arg(2) == "another").await?,
        Some("keep") => app.machine(&arg(1))?.keep_awake(words.get(2).map(|m| m.parse()).transpose()?).await?,
        Some("delete") => app.delete_machine(&*app.machine(&arg(1))?, arg(2) == "discard").await?,
        Some("costs") => {
            app.refresh_costs().await;
            println!("{:?}", app.costs());
        }
        _ => println!("?"),
    }
    Ok(())
}
