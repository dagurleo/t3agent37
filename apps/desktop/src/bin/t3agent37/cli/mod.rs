//! The `t3agent37` command: `t3agent37 list`, `t3agent37 wake skiff`, …. Each command asks the
//! t3agent37 running on this computer (the app, or `t3agent37 serve` in its place) through the
//! local address coding agents use for MCP; `serve` is that backend without the panel, and
//! `setup` saves the Agent37 key it needs. Long calls (creating a machine, a sign-in) show the
//! machine's progress while they run, and ^C only stops the waiting: the work goes on.

mod args;
mod client;
mod complete;
mod serve;
mod show;
mod update;

use anyhow::{Result, anyhow, bail};
use args::{Args, Flag};
use client::Backend;
use serde_json::{Value, json};
use std::io::{BufRead, IsTerminal, Write};
use std::time::Duration;
use t3agent37_lib::machine::provision::SIZES;
use t3agent37_lib::mcp::{self, Client};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether these arguments are a command, not the app's own: `--open`, or what macOS passes the
/// app when it starts it.
pub fn wants(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        None => false,
        Some("-h" | "--help" | "-V" | "--version") => true,
        Some(a) => !a.starts_with('-'),
    }
}

/// Runs the command in `args` (the program's arguments, without its name); the exit code.
pub fn main(args: Vec<String>) -> i32 {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("t3agent37").build().expect("a tokio runtime");
    match rt.block_on(run(args)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("t3agent37: {e:#}");
            1
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Start,
    Machines,
    Awake,
    Work,
    Reach,
}

struct Command {
    name: &'static str,
    alias: Option<&'static str>,
    usage: &'static str,
    about: &'static str,
    flags: &'static [Flag],
    group: Group,
}

const fn cmd(name: &'static str, usage: &'static str, about: &'static str, flags: &'static [Flag], group: Group) -> Command {
    Command { name, alias: None, usage, about, flags, group }
}

const COMMANDS: &[Command] = &[
    cmd("setup", "setup", "Save your Agent37 API key on this computer (asked for, hidden)", &[], Group::Start),
    cmd(
        "serve",
        "serve [--install | --uninstall]",
        "Run t3agent37 without the app: T3 Code's way to your machines, their sleeping and waking, \
         notifications. --install starts it at login (launchd, systemd), --uninstall stops that",
        &[("install", false), ("uninstall", false)],
        Group::Start,
    ),
    cmd("status", "status", "Whether t3agent37 runs here, its version, and your wallet", &[], Group::Start),
    cmd(
        "update",
        "update [--check] [--to <version>]",
        "Install the latest release of this command over this one. --check: only say whether there's one. --to: \
         that release instead (v0.1.9)",
        &[("check", false), ("to", true)],
        Group::Start,
    ),
    cmd(
        "completions",
        "completions <zsh | bash | fish>",
        "Tab completion for your shell, machine names included. zsh: t3agent37 completions zsh > \
         \"${fpath[1]}/_t3agent37\"; bash: source <(t3agent37 completions bash) in ~/.bashrc; fish: t3agent37 \
         completions fish > ~/.config/fish/completions/t3agent37.fish",
        &[],
        Group::Start,
    ),
    Command { alias: Some("ls"), ..cmd("list", "list [--json]", "Your machines, what each is doing, and what they cost", &[("json", false)], Group::Machines) },
    Command {
        alias: Some("show"),
        ..cmd(
            "details",
            "details <machine> [--json]",
            "One machine: its agents, what it's doing and finished, its load, projects and costs. Never wakes it",
            &[("json", false)],
            Group::Machines,
        )
    },
    cmd(
        "create",
        "create <name> [--size <size>] [--project <project>]... [--delete-after <time>]",
        "A new machine, added to T3 Code (you confirm there), with projects from this computer's T3 Code. \
         --delete-after (90m, 2h, 1d): a throwaway one, deleted then, or once its agents finish",
        &[("size", true), ("project", true), ("delete-after", true)],
        Group::Machines,
    ),
    cmd(
        "copy",
        "copy <machine> <name> [--size <size>] [--delete-after <time>]",
        "A new machine that's a copy of one as it is now: its projects and their files, work not pushed, its \
         sign-ins and tools, at its size or another, with a T3 identity of its own. A few minutes. --delete-after: \
         a throwaway copy",
        &[("size", true), ("delete-after", true)],
        Group::Machines,
    ),
    cmd(
        "bench",
        "bench <machine> [--sizes <size,size,...>] [--project <project>] [--keep] [--timeout <minutes>] -- <command>",
        "Time a command on copies of a machine at several sizes at once (small, large and huge unless --sizes \
         says), in a project's folder or its home, then delete the copies (--keep keeps them). Each copy bills \
         while it runs",
        &[("sizes", true), ("project", true), ("keep", false), ("timeout", true)],
        Group::Machines,
    ),
    cmd(
        "delete-after",
        "delete-after <machine> <time | never>",
        "Delete a machine some time from now (90m, 2h, 1d), or once its agents finish if they still work then; \
         never keeps it",
        &[],
        Group::Machines,
    ),
    cmd(
        "limit",
        "limit [<machine>] [<dollars> | off]",
        "A monthly spending limit, for one machine or (without one) all of them together: at it, machines are \
         stopped until you raise it or the month is over, and you hear about it at 80%. Alone: the limits",
        &[("json", false)],
        Group::Machines,
    ),
    cmd(
        "delete",
        "delete <machine> [--discard] [--yes]",
        "Delete a machine and everything on it. --discard: even while work runs there. --yes: no question",
        &[("discard", false), ("yes", false)],
        Group::Machines,
    ),
    cmd("resize", "resize <machine> <size>", "Change its size: right away if it's idle, else at its next wake", &[], Group::Machines),
    cmd("sizes", "sizes", "The sizes a machine can have, and what each costs awake", &[], Group::Machines),
    cmd(
        "link",
        "link <machine> <other> --access <read | ask | edits | auto | full>",
        "Let a machine's agents use another of your machines, through T3's linked environments: read its \
         threads (read), or start threads there that ask you before each change (ask), edit files but ask \
         before commands (edits), run in T3's auto mode (auto) or run anything (full). Wakes both; again \
         changes how far",
        &[("access", true)],
        Group::Machines,
    ),
    cmd("unlink", "unlink <machine> <other>", "Stop a machine using another: the link goes on both", &[], Group::Machines),
    cmd("links", "links [<machine>] [--json]", "Which of your machines may use which, and how far", &[("json", false)], Group::Machines),
    cmd("wake", "wake <machine>", "Wake it, and wait until T3 serves there", &[], Group::Awake),
    cmd("sleep", "sleep <machine>", "Put it to sleep now (refused while an agent works there)", &[], Group::Awake),
    cmd(
        "keep-awake",
        "keep-awake <machine> <minutes | off>",
        "Keep it up for some minutes, working or not; off ends that",
        &[],
        Group::Awake,
    ),
    cmd(
        "auto-sleep",
        "auto-sleep <machine> <on | off>",
        "off: it stays up until you turn this on again; on: it sleeps 5 minutes after its agents finish",
        &[],
        Group::Awake,
    ),
    cmd("projects", "projects [--json]", "This computer's T3 projects, which can go to a machine", &[("json", false)], Group::Work),
    cmd(
        "add-project",
        "add-project <machine> <project>",
        "Put one of them on a machine, .env files included, as T3 Code's \"Add to another machine\" does",
        &[],
        Group::Work,
    ),
    cmd(
        "sign-in",
        "sign-in <machine> <claude | codex | github> [--other-account], or sign-in <machine> --cancel",
        "Sign an agent, or GitHub, in on the machine, in your browser. --other-account: a private window. --cancel \
         stops a sign-in that waits (its old sign-in stays)",
        &[("other-account", false), ("cancel", false)],
        Group::Work,
    ),
    cmd(
        "env",
        "env <machine> <project> [--copy | --remove]",
        "A project's env files on the machine and the variables they set, names only. --copy copies this \
         computer's over them again; --remove deletes them there",
        &[("copy", false), ("remove", false)],
        Group::Work,
    ),
    cmd(
        "issues",
        "issues <machine> <project> <on | off> [--label <label>] [--model <model>] [--effort <effort>]",
        "GitHub issues with the label become threads there, which open pull requests",
        &[("label", true), ("model", true), ("effort", true)],
        Group::Work,
    ),
    cmd("open", "open <machine>", "Show it in T3 Code, at its latest thread", &[], Group::Reach),
    cmd("pair", "pair <machine>", "Ask T3 Code to add it again (after it was removed there)", &[], Group::Reach),
    cmd(
        "port",
        "port <machine> <port> [--open]",
        "Reach a port on the machine at localhost here (waking it); --open opens it in the browser",
        &[("open", false)],
        Group::Reach,
    ),
    cmd("connect", "connect <machine>", "Turn on T3 Connect, for your phone and other computers", &[], Group::Reach),
    cmd(
        "wake-link",
        "wake-link <machine> [--replace]",
        "The link that wakes it from a phone (a secret); --replace retires the old one",
        &[("replace", false)],
        Group::Reach,
    ),
    cmd(
        "start-url",
        "start-url <machine> [on | off | replace]",
        "The URL that starts threads there from anywhere (a secret): shown, turned on or off, replaced",
        &[],
        Group::Reach,
    ),
    cmd(
        "add-here",
        "add-here <machine>",
        "Use a machine made with t3agent37 on another computer from this one too",
        &[],
        Group::Reach,
    ),
    cmd(
        "add-mcp",
        "add-mcp <claude-code | codex>",
        "Let that coding agent on this computer manage your machines (t3agent37's MCP server)",
        &[],
        Group::Reach,
    ),
];

fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name || c.alias == Some(name))
}

fn help() -> String {
    let mut out = format!(
        "t3agent37 {VERSION}: T3 Code machines on your Agent37 Cloud account that sleep when nobody works.\n\n\
         Usage: t3agent37 <command> [options]. `t3agent37 help <command>` says more.\n"
    );
    let groups = [
        (Group::Start, "Getting going"),
        (Group::Machines, "Machines"),
        (Group::Awake, "Awake and asleep"),
        (Group::Work, "Projects, agents, GitHub"),
        (Group::Reach, "Reaching a machine"),
    ];
    for (group, title) in groups {
        out.push_str(&format!("\n{}\n", show::bold(title)));
        for c in COMMANDS.iter().filter(|c| c.group == group) {
            let about = c.about.split(". ").next().unwrap_or(c.about);
            out.push_str(&format!("  {:13} {about}\n", c.name));
        }
    }
    out.push_str(
        "\nA machine is its name or id, from `t3agent37 list`. Every command but setup, serve, update, sizes, completions \
         and add-mcp talks to the t3agent37 running here: the app, or `t3agent37 serve`.\n",
    );
    out
}

fn help_for(c: &Command) -> String {
    let alias = c.alias.map(|a| format!(" (or {a})")).unwrap_or_default();
    format!("Usage: t3agent37 {}{alias}\n\n{}.\n", c.usage, c.about)
}

async fn run(raw: Vec<String>) -> Result<i32> {
    let Some((name, rest)) = raw.split_first() else {
        print!("{}", help());
        return Ok(0);
    };
    match name.as_str() {
        "help" | "-h" | "--help" => {
            match rest.first() {
                Some(name) => print!("{}", help_for(command(name).ok_or_else(|| anyhow!("no command {name}: `t3agent37 help` lists them"))?)),
                None => print!("{}", help()),
            }
            return Ok(0);
        }
        "version" | "-V" | "--version" => {
            println!("t3agent37 {VERSION}");
            return Ok(0);
        }
        // What the shells' completion scripts ask; not in the help.
        "__complete" => return Ok(complete::run(rest).await),
        _ => {}
    }
    let c = command(name).ok_or_else(|| anyhow!("no command {name}: `t3agent37 help` lists them"))?;
    let mut a = Args::parse(rest, c.flags).map_err(|e| anyhow!("{e}\nUsage: t3agent37 {}", c.usage))?;
    if a.flag("help") {
        print!("{}", help_for(c));
        return Ok(0);
    }
    let usage = |e: String| anyhow!("{e}\nUsage: t3agent37 {}", c.usage);
    let b = Backend::new();
    let text = match c.name {
        "setup" => {
            a.done().map_err(usage)?;
            return serve::setup().await;
        }
        "serve" => {
            a.done().map_err(usage)?;
            return match (a.flag("install"), a.flag("uninstall")) {
                (true, true) => Err(usage("--install or --uninstall, not both".into())),
                (true, false) => serve::install(),
                (false, true) => serve::uninstall(),
                (false, false) => serve::serve().await,
            };
        }
        "status" => {
            a.done().map_err(usage)?;
            let version = b.version().await?;
            let list = b.json("list_machines", json!({})).await?;
            let count = list["machines"].as_array().map_or(0, Vec::len);
            let mut line = format!("t3agent37 {version} runs here, with {count} machine{}", if count == 1 { "" } else { "s" });
            if let Some(balance) = list["costs"]["balance"].as_f64() {
                line.push_str(&format!("; your Agent37 wallet has {} left", show::money(balance)));
            }
            println!("{line}.");
            if version != VERSION {
                println!("{}", show::dim(&format!("(this command is {VERSION})")));
            }
            return Ok(0);
        }
        "list" => {
            a.done().map_err(usage)?;
            let v = b.json("list_machines", json!({})).await?;
            return print_json_or(&a, &v, |v| show::machines(v, show::now_ms()));
        }
        "details" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            let (list, d) = tokio::try_join!(b.json("list_machines", json!({})), b.json("machine_details", json!({ "machine": machine })))?;
            if a.flag("json") {
                return print_json_or(&a, &d, |_| String::new());
            }
            let row = show::find(&list, &machine).cloned().unwrap_or(Value::Null);
            print!("{}", show::details(&row, &d, show::now_ms()));
            return Ok(0);
        }
        "projects" => {
            a.done().map_err(usage)?;
            let v = b.json("list_projects", json!({})).await?;
            return print_json_or(&a, &v, show::projects);
        }
        "update" => {
            a.done().map_err(usage)?;
            return update::update(a.flag("check"), a.value("to")).await;
        }
        "completions" => {
            let shell = a.word("zsh, bash or fish").map_err(usage)?;
            a.done().map_err(usage)?;
            let script = complete::script(&shell).ok_or_else(|| usage(format!("no completions for {shell}: zsh, bash or fish")))?;
            print!("{script}");
            return Ok(0);
        }
        "links" => {
            let machine = a.maybe_word();
            a.done().map_err(usage)?;
            let v = b.json("list_machines", json!({})).await?;
            if let Some(m) = machine.as_deref().filter(|m| show::find(&v, m).is_none()) {
                bail!("no machine {m}: `t3agent37 list` lists them");
            }
            return print_json_or(&a, &v, |v| show::links(v, machine.as_deref()));
        }
        "link" => {
            let from = a.word("a machine").map_err(usage)?;
            let to = a.word("the machine it may use").map_err(usage)?;
            a.done().map_err(usage)?;
            let access = a.value("access").ok_or_else(|| usage("say how far with --access: read, ask, edits, auto or full".into()))?;
            if t3agent37_lib::machine::peers::Access::parse(access).is_none() {
                return Err(usage(format!("no access {access}: read, ask, edits, auto or full")));
            }
            if from.eq_ignore_ascii_case(&to) {
                bail!("{from}'s agents work on it already");
            }
            eprintln!("{}", show::dim("Waking both and linking them; about half a minute."));
            b.call("link_machines", json!({ "from": from, "to": to, "access": access })).await?
        }
        "unlink" => {
            let from = a.word("a machine").map_err(usage)?;
            let to = a.word("the machine it uses").map_err(usage)?;
            a.done().map_err(usage)?;
            b.call("unlink_machines", json!({ "from": from, "to": to })).await?
        }
        "sizes" => {
            a.done().map_err(usage)?;
            for s in SIZES {
                println!("{:8} {:16} ${:.3} an hour awake", s.id, s.spec, s.hourly);
            }
            println!("{}", show::dim("Asleep, any size bills its 20 GB disk alone, about $1.80 a month."));
            return Ok(0);
        }
        "add-mcp" => {
            let which = a.word("claude-code or codex").map_err(usage)?;
            a.done().map_err(usage)?;
            let client = match which.as_str() {
                "claude-code" | "claude" => Client::ClaudeCode,
                "codex" => Client::Codex,
                other => return Err(usage(format!("no coding agent {other}"))),
            };
            mcp::register(client).await?;
            format!("{which} can manage your machines now, through t3agent37 at {}.", mcp::url())
        }
        "create" => {
            let name = a.word("a name").map_err(usage)?;
            a.done().map_err(usage)?;
            let mut args = json!({ "name": name, "projects": a.values("project") });
            if let Some(size) = a.value("size") {
                args["size"] = json!(size);
            }
            if let Some(time) = a.value("delete-after") {
                args["deleteAfterMinutes"] = json!(minutes(time).map_err(usage)?);
            }
            eprintln!("{}", show::dim("Creating it takes about a minute; ^C stops waiting, not the creating."));
            watched(&b, &name, b.call("create_machine", args)).await?
        }
        "copy" => {
            let machine = a.word("the machine to copy").map_err(usage)?;
            let name = a.word("a name for the copy").map_err(usage)?;
            a.done().map_err(usage)?;
            let mut args = json!({ "machine": machine, "name": name });
            if let Some(size) = a.value("size") {
                args["size"] = json!(size);
            }
            if let Some(time) = a.value("delete-after") {
                args["deleteAfterMinutes"] = json!(minutes(time).map_err(usage)?);
            }
            eprintln!("{}", show::dim("Copying takes a few minutes; ^C stops waiting, not the copying."));
            watched(&b, &name, b.call("copy_machine", args)).await?
        }
        "bench" => {
            let machine = a.word("a machine").map_err(usage)?;
            let mut words = Vec::new();
            while let Some(w) = a.maybe_word() {
                words.push(w);
            }
            let command = words.join(" ");
            if command.trim().is_empty() {
                return Err(usage("say what to run after --: t3agent37 bench skiff -- cargo build --release".into()));
            }
            let sizes: Vec<String> = a.value("sizes").unwrap_or("small,large,huge").split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            for size in &sizes {
                if !SIZES.iter().any(|s| s.id == size) {
                    return Err(usage(format!("no size {size}: `t3agent37 sizes` lists them")));
                }
            }
            let timeout: u32 = match a.value("timeout") {
                Some(t) => t.parse().ok().filter(|t| (1..=1440).contains(t)).ok_or_else(|| usage(format!("{t}: a number of minutes, up to 1440")))?,
                None => 60,
            };
            let hourly: f64 = sizes.iter().filter_map(|id| SIZES.iter().find(|s| s.id == id)).map(|s| s.hourly).sum();
            let copies = if sizes.len() == 1 { format!("a {} copy", sizes[0]) } else { format!("{} copies ({})", sizes.len(), sizes.join(", ")) };
            let after = if a.flag("keep") { "they're kept after" } else { "they're deleted after all the same" };
            eprintln!(
                "{}",
                show::dim(&format!(
                    "Making {copies} of {machine} and running it there: a few minutes, at ${hourly:.3} an hour while they run. ^C stops waiting, not the copies: {after}."
                ))
            );
            let mut args = json!({ "machine": machine, "sizes": sizes, "command": command, "keep": a.flag("keep"), "timeoutMinutes": timeout });
            if let Some(p) = a.value("project") {
                args["project"] = json!(p);
            }
            let names: Vec<String> = sizes.iter().map(|s| format!("{}-{s}", machine.chars().take(24).collect::<String>())).collect();
            watched_all(&b, &names, b.call("bench", args)).await?
        }
        "delete-after" => {
            let machine = a.word("a machine").map_err(usage)?;
            let time = a.word("a time (90m, 2h, 1d) or never").map_err(usage)?;
            a.done().map_err(usage)?;
            let minutes = match time.as_str() {
                "never" | "off" => 0,
                t => minutes(t).map_err(usage)?,
            };
            b.call("delete_after", json!({ "machine": machine, "minutes": minutes })).await?
        }
        "limit" => {
            let first = a.maybe_word();
            let second = a.maybe_word();
            a.done().map_err(usage)?;
            let dollars = |w: &str| -> Result<Option<f64>> {
                match w {
                    "off" | "none" => Ok(None),
                    w => w.trim_start_matches('$').parse::<f64>().ok().filter(|v| *v > 0.0).map(Some).ok_or_else(|| usage(format!("{w}: a number of dollars, or off"))),
                }
            };
            let is_amount = |w: &str| w == "off" || w == "none" || w.trim_start_matches('$').parse::<f64>().is_ok();
            let (machine, value) = match (first, second) {
                (None, _) => {
                    let v = b.json("list_machines", json!({})).await?;
                    return print_json_or(&a, &v, show::limits);
                }
                (Some(w), None) if is_amount(&w) => (None, dollars(&w)?),
                (Some(m), None) => {
                    let v = b.json("list_machines", json!({})).await?;
                    let row = show::find(&v, &m).ok_or_else(|| anyhow!("no machine {m}: `t3agent37 list` lists them"))?;
                    print!("{}", show::limit_of(row));
                    return Ok(0);
                }
                (Some(m), Some(w)) => (Some(m), dollars(&w)?),
            };
            b.call("spending_limit", json!({ "machine": machine, "dollars": value })).await?
        }
        "env" => {
            let machine = a.word("a machine").map_err(usage)?;
            let project = a.word("a project on it").map_err(usage)?;
            a.done().map_err(usage)?;
            let action = match (a.flag("copy"), a.flag("remove")) {
                (true, true) => return Err(usage("--copy or --remove, not both".into())),
                (true, false) => "copy",
                (false, true) => "remove",
                (false, false) => "list",
            };
            let call = b.call("project_env", json!({ "machine": machine, "project": project, "action": action }));
            if action == "list" { call.await? } else { watched(&b, &machine, call).await? }
        }
        "delete" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            if !a.flag("yes") && !confirm(&machine)? {
                println!("Kept {machine}.");
                return Ok(1);
            }
            b.call("delete_machine", json!({ "machine": machine, "discard": a.flag("discard") })).await?
        }
        "resize" => {
            let machine = a.word("a machine").map_err(usage)?;
            let size = a.word("a size (t3agent37 sizes)").map_err(usage)?;
            a.done().map_err(usage)?;
            watched(&b, &machine, b.call("resize_machine", json!({ "machine": machine, "size": size }))).await?
        }
        "wake" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            watched(&b, &machine, b.call("wake_machine", json!({ "machine": machine }))).await?
        }
        "sleep" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            b.call("sleep_machine", json!({ "machine": machine })).await?
        }
        "keep-awake" => {
            let machine = a.word("a machine").map_err(usage)?;
            let how = a.word("minutes, or off").map_err(usage)?;
            a.done().map_err(usage)?;
            let minutes: u32 = match how.as_str() {
                "off" => 0,
                m => m.parse().map_err(|_| usage(format!("{m} isn't a number of minutes")))?,
            };
            watched(&b, &machine, b.call("keep_awake", json!({ "machine": machine, "minutes": minutes }))).await?
        }
        "auto-sleep" => {
            let machine = a.word("a machine").map_err(usage)?;
            let on = on_off(&a.word("on or off").map_err(usage)?).map_err(usage)?;
            a.done().map_err(usage)?;
            watched(&b, &machine, b.call("set_auto_sleep", json!({ "machine": machine, "on": on }))).await?
        }
        "add-project" => {
            let machine = a.word("a machine").map_err(usage)?;
            let project = a.word("a project (t3agent37 projects)").map_err(usage)?;
            a.done().map_err(usage)?;
            watched(&b, &machine, b.call("add_project", json!({ "machine": machine, "project": project }))).await?
        }
        "sign-in" if a.flag("cancel") => {
            let machine = a.word("a machine").map_err(usage)?;
            let _agent = a.maybe_word();
            a.done().map_err(usage)?;
            b.call("cancel_sign_in", json!({ "machine": machine })).await?
        }
        "sign-in" => {
            let machine = a.word("a machine").map_err(usage)?;
            let agent = match a.word("claude, codex or github").map_err(usage)?.to_lowercase().as_str() {
                "claude" | "claude-code" => "Claude",
                "codex" => "Codex",
                "github" | "gh" => "GitHub",
                other => return Err(usage(format!("no agent {other}: claude, codex or github"))),
            };
            a.done().map_err(usage)?;
            eprintln!(
                "{}",
                show::dim(&format!(
                    "The sign-in page opens in your browser. ^C stops waiting, not the sign-in: `t3agent37 sign-in {machine} --cancel` stops that."
                ))
            );
            let args = json!({ "machine": machine, "agent": agent, "anotherAccount": a.flag("other-account") });
            watched(&b, &machine, b.call("sign_in_agent", args)).await?
        }
        "issues" => {
            let machine = a.word("a machine").map_err(usage)?;
            let project = a.word("a project on it").map_err(usage)?;
            let on = on_off(&a.word("on or off").map_err(usage)?).map_err(usage)?;
            a.done().map_err(usage)?;
            let mut args = json!({ "machine": machine, "project": project, "on": on });
            for key in ["label", "model", "effort"] {
                if let Some(v) = a.value(key) {
                    args[key] = json!(v);
                }
            }
            watched(&b, &machine, b.call("github_issues", args)).await?
        }
        "open" | "pair" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            b.call(if c.name == "open" { "open_in_t3" } else { "add_to_t3" }, json!({ "machine": machine })).await?
        }
        "connect" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            eprintln!("{}", show::dim("T3's sign-in page opens in your browser; approve it there."));
            watched(&b, &machine, b.call("turn_on_connect", json!({ "machine": machine }))).await?
        }
        "port" => {
            let machine = a.word("a machine").map_err(usage)?;
            let port: u16 = a.word("a port").map_err(usage)?.parse().map_err(|_| usage("a port is a number, 1 to 65535".into()))?;
            a.done().map_err(usage)?;
            let args = json!({ "machine": machine, "port": port, "open": a.flag("open") });
            watched(&b, &machine, b.call("open_port", args)).await?
        }
        "wake-link" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            b.call("wake_link", json!({ "machine": machine, "renew": a.flag("replace") })).await?
        }
        "start-url" => {
            let machine = a.word("a machine").map_err(usage)?;
            let what = a.maybe_word();
            a.done().map_err(usage)?;
            let mut args = json!({ "machine": machine });
            match what.as_deref() {
                None => {}
                Some("on") => args["on"] = json!(true),
                Some("off") => args["on"] = json!(false),
                Some("replace") => args["replace"] = json!(true),
                Some(other) => return Err(usage(format!("{other}: on, off or replace"))),
            }
            watched(&b, &machine, b.call("start_url", args)).await?
        }
        "add-here" => {
            let machine = a.word("a machine").map_err(usage)?;
            a.done().map_err(usage)?;
            watched(&b, &machine, b.call("add_machine_here", json!({ "machine": machine }))).await?
        }
        other => bail!("{other} isn't done yet"),
    };
    println!("{}", to_you(&text));
    Ok(0)
}

/// The tools' answers speak to a coding agent about "the user"; here, the user reads them.
fn to_you(text: &str) -> String {
    text.replace("asks the user to add it as a backend; tell them to confirm.", "asks you to add it as a backend: confirm it there.")
        .replace("asks the user to add", "asks you to add")
        .replace("; tell them to confirm.", ": confirm it there.")
        .replace("the user's other devices reach it with their T3 account", "your other devices reach it with your T3 account")
        .replace("until the user removes it there", "until you remove it there")
}

/// "90m", "2h", "1d" as minutes, rounded up.
fn minutes(time: &str) -> Result<u64, String> {
    t3agent37_lib::app::parse_duration(time).map(|s| s.div_ceil(60)).ok_or_else(|| format!("{time}: a time such as 90m, 2h or 1d"))
}

fn on_off(word: &str) -> Result<bool, String> {
    match word {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("{other}: on or off")),
    }
}

fn print_json_or(a: &Args, v: &Value, human: impl Fn(&Value) -> String) -> Result<i32> {
    if a.flag("json") {
        println!("{}", serde_json::to_string_pretty(v)?);
    } else {
        print!("{}", human(v));
    }
    Ok(0)
}

/// "Delete raven?" answered by typing its name.
fn confirm(machine: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("deleting {machine} needs --yes when there's no terminal to ask in");
    }
    print!("Delete {machine} and everything on it, including work not pushed anywhere? Type its name to go ahead: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case(machine))
}

/// Runs `call` and shows what `machine` does meanwhile, as the machine list says (its line, a
/// project being added, a GitHub code to enter), on stderr: in place in a terminal, else a line
/// for each change.
async fn watched(b: &Backend, machine: &str, call: impl Future<Output = Result<String>>) -> Result<String> {
    let tty = std::io::stderr().is_terminal();
    let watch = async {
        let mut last = String::new();
        let mut code_shown = false;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let Ok(list) = b.json("list_machines", json!({})).await else { continue };
            let Some(m) = show::find(&list, machine) else { continue };
            if let Some(code) = m["githubCode"].as_str().filter(|_| !code_shown) {
                code_shown = true;
                clear(tty);
                eprintln!("GitHub code {}: enter it on the GitHub page that opened (it's on your clipboard too).", show::bold(code));
                last.clear();
            }
            let line = show::progress(m, show::now_ms());
            if line != last {
                if tty {
                    let width = terminal_width().saturating_sub(1);
                    eprint!("\r\x1b[2K{}", show::dim(&line.chars().take(width).collect::<String>()));
                } else {
                    eprintln!("{line}");
                }
                last = line;
            }
        }
    };
    let result = tokio::select! {
        r = call => r,
        _ = watch => unreachable!("watching doesn't end"),
    };
    clear(tty);
    result
}

/// `watched` for several machines at once (`bench`'s copies): a line each, as they change.
async fn watched_all(b: &Backend, machines: &[String], call: impl Future<Output = Result<String>>) -> Result<String> {
    let watch = async {
        let mut last: std::collections::HashMap<String, String> = Default::default();
        loop {
            tokio::time::sleep(Duration::from_millis(2000)).await;
            let Ok(list) = b.json("list_machines", json!({})).await else { continue };
            for name in machines {
                let Some(m) = show::find(&list, name) else { continue };
                let line = show::progress(m, show::now_ms());
                if last.get(name) != Some(&line) {
                    eprintln!("{}", show::dim(&line));
                    last.insert(name.clone(), line);
                }
            }
        }
    };
    tokio::select! {
        r = call => r,
        _ = watch => unreachable!("watching doesn't end"),
    }
}

fn clear(tty: bool) {
    if tty {
        eprint!("\r\x1b[2K");
    }
}

/// Columns, from `stty size` on the terminal (looked up once); 80 when that can't say.
fn terminal_width() -> usize {
    static WIDTH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *WIDTH.get_or_init(|| {
        let size = std::fs::File::open("/dev/tty")
            .ok()
            .and_then(|tty| std::process::Command::new("stty").arg("size").stdin(tty).output().ok())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
        size.and_then(|s| s.split_whitespace().nth(1)?.parse().ok()).filter(|w| *w > 20).unwrap_or(80)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_the_apps_own_arguments() {
        let words = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(wants(&words(&["list"])) && wants(&words(&["--help"])) && wants(&words(&["-V"])));
        // The app's own, and what macOS may pass it.
        assert!(!wants(&[]) && !wants(&words(&["--open"])) && !wants(&words(&["-psn_0_12345"])));
        for c in COMMANDS {
            assert_eq!(command(c.name).map(|f| f.name), Some(c.name));
            assert!(c.usage.starts_with(c.name), "{} usage", c.name);
        }
        assert_eq!(command("ls").map(|c| c.name), Some("list"));
        assert!(help().contains("add-project") && !help().contains("__complete"));
        assert_eq!(
            to_you("Created raven (x1). T3 Code now asks the user to add it as a backend; tell them to confirm."),
            "Created raven (x1). T3 Code now asks you to add it as a backend: confirm it there."
        );
        assert_eq!(to_you("T3 Code now asks the user to add raven; tell them to confirm."), "T3 Code now asks you to add raven: confirm it there.");
    }
}
