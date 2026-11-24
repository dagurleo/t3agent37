//! How the command shows what t3agent37 answers: the machine list, a machine's details, the
//! projects. It reads the same JSON the MCP tools give agents, so an app of another version
//! answers too: what's missing is left out.

use serde_json::Value;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bold and dim in a terminal that takes them, unless `NO_COLOR` is set.
fn styled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        !cfg!(test)
            && std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |t| t != "dumb")
    })
}

pub fn bold(s: &str) -> String {
    if styled() { format!("\x1b[1m{s}\x1b[0m") } else { s.to_owned() }
}

pub fn dim(s: &str) -> String {
    if styled() { format!("\x1b[2m{s}\x1b[0m") } else { s.to_owned() }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// "42s", "12m", "1h 5m".
pub fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

/// "just now", "4m ago", "2h ago", "3d ago".
pub fn ago(ms: u64, now: u64) -> String {
    let secs = now.saturating_sub(ms) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// "14:05" today, else "Mon 14:05", in local time.
pub fn clock(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ms as i64).single() else { return "?".into() };
    if t.date_naive() == Local::now().date_naive() { t.format("%H:%M").to_string() } else { t.format("%a %H:%M").to_string() }
}

pub fn money(dollars: f64) -> String {
    format!("${dollars:.2}")
}

fn megabytes(mb: u64) -> String {
    if mb >= 1024 { format!("{:.1} GB", mb as f64 / 1024.) } else { format!("{mb} MB") }
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn n(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn f(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// A machine by name (any case) or id, in `list_machines`' answer.
pub fn find<'a>(machines: &'a Value, machine: &str) -> Option<&'a Value> {
    list(machines, "machines").iter().find(|m| s(m, "id") == Some(machine) || s(m, "name").is_some_and(|n| n.eq_ignore_ascii_case(machine)))
}

/// Its list line ("Asleep · bills disk only"), with any countdown in it as of now.
pub fn detail_line(m: &Value, now: u64) -> String {
    let countdown = m.get("countdown").filter(|c| !c.is_null());
    if let Some((template, ends)) = countdown.and_then(|c| Some((s(c, "template")?, n(c, "endsAt")?))) {
        return template.replace("{left}", &duration(ends.saturating_sub(now).div_ceil(1000)));
    }
    s(m, "detail").unwrap_or_default().to_owned()
}

/// What a long call shows while it runs: the machine's line and any project being added.
pub fn progress(m: &Value, now: u64) -> String {
    let mut line = format!("{}: {}", s(m, "name").unwrap_or("?"), detail_line(m, now));
    for p in list(m, "projects") {
        if let (Some(title), Some(status)) = (s(p, "title"), s(p, "status")) {
            let _ = write!(line, " · {title}: {status}");
        }
    }
    line
}

/// `t3agent37 list`.
pub fn machines(v: &Value, now: u64) -> String {
    let mut out = String::new();
    let rows = list(v, "machines");
    if rows.is_empty() {
        out.push_str("No machines yet: `t3agent37 create <name>` makes one.\n");
    }
    let width = |key: &str| rows.iter().map(|m| s(m, key).unwrap_or_default().chars().count()).max().unwrap_or(0);
    let (name_w, size_w, state_w) = (width("name"), width("size"), width("state"));
    for m in rows {
        let name = s(m, "name").unwrap_or("?");
        let _ = writeln!(
            out,
            "{}{}  {:size_w$}  {:state_w$}  {}",
            bold(name),
            " ".repeat(name_w - name.chars().count()),
            s(m, "size").unwrap_or_default(),
            s(m, "state").unwrap_or_default(),
            detail_line(m, now),
        );
        let pad = " ".repeat(name_w + 2);
        if let Some(code) = s(m, "githubCode") {
            let _ = writeln!(out, "{pad}GitHub code {}: enter it on the page that opened", bold(code));
        }
        let signed_out: Vec<&str> = list(m, "signedOut").iter().filter_map(Value::as_str).collect();
        if !signed_out.is_empty() {
            let _ = writeln!(out, "{pad}{}", dim(&format!("not signed in: {} (t3agent37 sign-in {name} <agent>)", signed_out.join(", "))));
        }
        if m["keepAwakeForever"] == true {
            let _ = writeln!(out, "{pad}{}", dim("kept awake until auto sleep is on again"));
        } else if let Some(until) = n(m, "keepAwakeUntil") {
            let _ = writeln!(out, "{pad}{}", dim(&format!("kept awake until {}", clock(until))));
        }
    }
    let elsewhere = list(v, "elsewhere");
    if !elsewhere.is_empty() {
        let _ = writeln!(out, "\n{}", bold("on your agent37 account, made on another computer"));
        let width = |key: &str| elsewhere.iter().map(|m| s(m, key).unwrap_or("?").chars().count()).max().unwrap_or(0);
        let (name_w, size_w) = (width("name"), width("size"));
        for m in elsewhere {
            let name = s(m, "name").unwrap_or("?");
            let state = if m["asleep"] == true { "asleep" } else { "awake " };
            let _ = writeln!(
                out,
                "{name:name_w$}  {:size_w$}  {state}  {}",
                s(m, "size").unwrap_or("?"),
                dim(&format!("t3agent37 add-here {name}"))
            );
        }
    }
    let costs = &v["costs"];
    let mut line = format!("today {} · this month {}", money(f(costs, "today").unwrap_or(0.)), money(f(costs, "month").unwrap_or(0.)));
    if let Some(balance) = f(costs, "balance") {
        let _ = write!(line, " · wallet {}", money(balance));
    }
    let _ = writeln!(out, "\n{}", dim(&line));
    out
}

/// `t3agent37 limit`: the limit on all machines and each one's, with what they spent this month.
pub fn limits(v: &Value) -> String {
    let mut out = String::new();
    let all = &v["limit"];
    let spent = money(f(all, "spent").unwrap_or(0.));
    let _ = match f(all, "monthUsd") {
        Some(limit) => writeln!(out, "All machines: {} a month · spent {spent} this month", t3agent37_lib::store::dollars(limit)),
        None => writeln!(out, "All machines: no limit · spent {spent} this month {}", dim("(t3agent37 limit 50 sets one)")),
    };
    for m in list(v, "machines") {
        out.push_str(&limit_of(m));
    }
    out
}

/// One machine's limit: "skiff: $10.00 a month · spent $3.20", and why it's stopped if it is.
pub fn limit_of(m: &Value) -> String {
    let name = s(m, "name").unwrap_or("?");
    let spent = money(f(m, "monthSpent").unwrap_or(0.));
    let mut line = match f(m, "monthLimit") {
        Some(limit) => format!("{name}: {} a month · spent {spent}", t3agent37_lib::store::dollars(limit)),
        None => format!("{name}: no limit of its own · spent {spent}"),
    };
    if let Some(why) = s(m, "paused") {
        let _ = write!(line, "\n  {}", bold(&format!("Stopped: {why}.")));
    }
    line.push('\n');
    line
}

/// `t3agent37 details`: `row` from the machine list, `d` from machine_details.
pub fn details(row: &Value, d: &Value, now: u64) -> String {
    let mut out = String::new();
    let name = s(row, "name").unwrap_or("?");
    let _ = writeln!(out, "{} · {} · {}", bold(name), s(row, "size").unwrap_or("?"), s(row, "state").unwrap_or("?"));
    let _ = writeln!(out, "{}", detail_line(row, now));
    let mut about = vec![format!("T3 {}", s(d, "t3Version").unwrap_or("?"))];
    about.push(if d["connect"] == true { "T3 Connect on".into() } else { "T3 Connect off".into() });
    if let Some(made) = d.get("createdAt").and_then(Value::as_i64).and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
        about.push(format!("made {}", made.format("%Y-%m-%d")));
    }
    if let Some(from) = s(row, "copiedFrom") {
        about.push(format!("a copy of {from}"));
    }
    let _ = writeln!(out, "{}", dim(&about.join(" · ")));
    if let Some(code) = s(row, "githubCode") {
        let _ = writeln!(out, "GitHub code {}: enter it on the page that opened", bold(code));
    }
    if let Some(why) = s(row, "paused") {
        let _ = writeln!(out, "{}", bold(&format!("Stopped: {why}.")));
    }
    if let Some(at) = row.get("deleteAt").and_then(Value::as_i64) {
        let left = at - (now / 1000) as i64;
        let when = if left > 0 { format!("in {}", t3agent37_lib::app::span(left)) } else { "once its work is done".into() };
        let _ = writeln!(out, "Deleted {when} (t3agent37 delete-after {name} never keeps it).");
    }

    section(&mut out, "agents", None);
    let agents = list(d, "agents");
    let width = agents.iter().filter_map(|a| s(a, "name")).map(str::len).max().unwrap_or(0);
    for a in agents {
        let agent = s(a, "name").unwrap_or("?");
        let mut parts = Vec::new();
        if a["signedIn"] != true {
            parts.push(dim(&format!("not signed in: t3agent37 sign-in {name} {}", agent.to_lowercase())));
        }
        parts.extend(s(a, "plan").map(str::to_owned));
        parts.extend(s(a, "email").map(str::to_owned));
        parts.extend(s(a, "login").map(|l| format!("@{l}")));
        parts.extend(s(a, "reach").map(str::to_owned));
        parts.extend(s(a, "expires").map(|e| format!("token expires {e}")));
        for u in list(a, "usage") {
            if let (Some(label), Some(used)) = (s(u, "label"), f(u, "usedPercent")) {
                parts.push(format!("{label} {used:.0}%"));
            }
        }
        let _ = writeln!(out, "  {agent:width$}  {}", parts.join(" · "));
    }

    if let Some(a) = d.get("activity").filter(|a| a.is_object()) {
        activity(&mut out, a, now);
    }

    let projects = list(row, "projects");
    if !projects.is_empty() {
        section(&mut out, "projects", None);
        for p in projects {
            let mut line = s(p, "title").unwrap_or("?").to_owned();
            if let Some(status) = s(p, "status") {
                let _ = write!(line, " · {status}");
            }
            let env: Vec<String> = list(d, "env")
                .iter()
                .filter(|e| s(e, "projectId") == s(p, "id"))
                .flat_map(|e| list(e, "files"))
                .map(|f| format!("{} ({})", s(f, "path").unwrap_or("?"), list(f, "names").iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")))
                .collect();
            if !env.is_empty() {
                let _ = write!(line, " · {}", dim(&format!("env: {}", env.join("; "))));
            }
            if let Some(i) = p.get("issues").filter(|i| i.is_object()) {
                let options: Vec<&str> = list(i, "options").iter().filter_map(Value::as_str).collect();
                let model = s(i, "model").unwrap_or("?");
                let agent = if options.is_empty() { model.to_owned() } else { format!("{model} ({})", options.join(", ")) };
                let _ = write!(line, " · {}", dim(&format!("GitHub issues labeled \"{}\" go to {agent}", s(i, "label").unwrap_or("agent"))));
            }
            let _ = writeln!(out, "  {line}");
        }
    }

    section(&mut out, "costs", None);
    let awake = n(d, "awakeSecsToday").map(|s| format!(" (awake {})", duration(s))).unwrap_or_default();
    let _ = writeln!(
        out,
        "  today {}{awake} · this month {}",
        money(f(d, "costToday").unwrap_or(0.)),
        money(f(d, "costMonth").unwrap_or(0.))
    );
    if let Some(limit) = f(row, "monthLimit") {
        let _ = writeln!(out, "  {}", dim(&format!("spent {} of its {} limit this month", money(f(row, "monthSpent").unwrap_or(0.)), t3agent37_lib::store::dollars(limit))));
    }
    for p in list(d, "projectCosts") {
        let _ = writeln!(
            out,
            "  {}",
            dim(&format!(
                "{}: today {} · this month {}",
                s(p, "title").unwrap_or("?"),
                money(f(p, "dollarsToday").unwrap_or(0.)),
                money(f(p, "dollarsMonth").unwrap_or(0.))
            ))
        );
    }

    let servers = list(d, "servers");
    let previews = list(d, "previews");
    if !servers.is_empty() || !previews.is_empty() {
        section(&mut out, "previews", None);
        for p in previews {
            if let (Some(port), Some(local)) = (n(p, "port"), n(p, "localPort")) {
                let _ = writeln!(out, "  {port} at http://localhost:{local}");
            }
        }
        for sv in servers {
            let port = n(sv, "port").unwrap_or(0);
            if previews.iter().any(|p| n(p, "port") == Some(port)) {
                continue;
            }
            let process = s(sv, "process").map(|p| format!(" ({p})")).unwrap_or_default();
            let _ = writeln!(out, "  {port}{process} {}", dim(&format!("t3agent37 port {name} {port} --open")));
        }
    }

    let (uses, used_by) = (list(row, "links"), list(row, "usedBy"));
    if !uses.is_empty() || !used_by.is_empty() {
        section(&mut out, "other machines", None);
        for l in uses {
            let _ = writeln!(out, "  uses {}", link_line(l));
        }
        for l in used_by {
            let _ = writeln!(out, "  used by {}", link_line(l));
        }
    }

    let hooks = list(d, "webhooks");
    let start = d.get("start").filter(|s| s.is_object());
    if !hooks.is_empty() || start.is_some() {
        section(&mut out, "from outside", None);
        for h in hooks {
            let project = s(h, "project").map(|p| format!(" · {p}")).unwrap_or_default();
            let off = if h["enabled"] == false { " · off" } else { "" };
            let _ = writeln!(out, "  webhook: {}{project}{off}", s(h, "title").unwrap_or("?"));
        }
        if !hooks.is_empty() {
            let _ = writeln!(out, "  {}", dim(&format!("their waking URLs: t3agent37 details {name} --json")));
        }
        if start.is_some() {
            let _ = writeln!(out, "  start URL on {}", dim(&format!("(t3agent37 start-url {name} shows it)")));
        }
    }
    out
}

/// T3's access level as `t3agent37 link --access` takes it.
fn access_word(access: &str) -> &str {
    match access {
        "read-only" => "read",
        "approval-required" => "ask",
        "auto-accept-edits" => "edits",
        "full-access" => "full",
        other => other,
    }
}

/// "faktor · full · until 23 Dec", and the link's error if it has one.
fn link_line(l: &Value) -> String {
    let until = s(l, "expiresAt").and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| format!(" · until {}", t.with_timezone(&chrono::Local).format("%-d %b")));
    let error = s(l, "lastError").map(|e| format!(" · {e}")).unwrap_or_default();
    format!("{} · {}{}{error}", s(l, "machine").unwrap_or("?"), access_word(s(l, "access").unwrap_or("?")), until.unwrap_or_default())
}

/// `t3agent37 links`: which machines may use which, for all of them or one.
pub fn links(v: &Value, machine: Option<&str>) -> String {
    let mut out = String::new();
    for m in list(v, "machines") {
        let name = s(m, "name").unwrap_or("?");
        if machine.is_some_and(|w| s(m, "id") != Some(w) && !name.eq_ignore_ascii_case(w)) {
            continue;
        }
        for l in list(m, "links") {
            let _ = writeln!(out, "{} can use {}", bold(name), link_line(l));
        }
        if machine.is_some() {
            for l in list(m, "usedBy") {
                let _ = writeln!(out, "{} can use {} · {}", bold(s(l, "machine").unwrap_or("?")), name, access_word(s(l, "access").unwrap_or("?")));
            }
        }
    }
    if out.is_empty() {
        out = format!(
            "{}\n",
            match machine {
                Some(m) => format!("{m} uses no other machine, and none uses it."),
                None => "No machine uses another. `t3agent37 link <machine> <other> --access …` lets one.".into(),
            }
        );
    }
    out
}

/// "now" (while it's up) and "finished".
fn activity(out: &mut String, a: &Value, now: u64) {
    let live = a["live"] == true;
    if live {
        let woke = n(a, "wokeAt").map(|at| match s(a, "wokeFor") {
            Some(why) => format!("woke {} for {why}", clock(at)),
            None => format!("woke {}", clock(at)),
        });
        section(out, "now", woke.as_deref());
        let threads = list(a, "threads");
        if threads.is_empty() {
            let _ = writeln!(out, "  {}", dim("No agent is working right now."));
        }
        for t in threads {
            let needs_you = t["needsYou"] == true;
            let title = s(t, "title").unwrap_or("Untitled thread");
            let state = s(t, "state").unwrap_or("working");
            let mut line = format!("{} {} · {}", if needs_you { "!" } else { "•" }, title, if needs_you { bold(state) } else { state.to_owned() });
            for part in [s(t, "project"), s(t, "agent")].into_iter().flatten() {
                let _ = write!(line, " · {part}");
            }
            match n(t, "subagents").unwrap_or(0) {
                0 => {}
                1 => line.push_str(" · 1 subagent"),
                k => {
                    let _ = write!(line, " · {k} subagents");
                }
            }
            if let Some(at) = n(t, "startedAt") {
                let _ = write!(line, " · {}", duration(now.saturating_sub(at) / 1000));
            }
            let _ = writeln!(out, "  {line}");
            if let Some(doing) = s(t, "doing") {
                let _ = writeln!(out, "    ▸ {doing}");
            }
            if let Some(said) = s(t, "said") {
                let _ = writeln!(out, "    {}", dim(&format!("“{said}”")));
            }
            for b in list(t, "background").iter().filter_map(Value::as_str) {
                let _ = writeln!(out, "    {}", dim(&format!("↳ {b}")));
            }
        }
        if let Some(l) = a.get("load").filter(|l| l.is_object()) {
            let mut parts = vec![format!("cpu {:.1} of {} cores", f(l, "cores").unwrap_or(0.), n(l, "cpus").unwrap_or(0))];
            if let (Some(used), Some(total)) = (n(l, "memUsedMb"), n(l, "memTotalMb")) {
                parts.push(format!("memory {} of {}", megabytes(used), megabytes(total)));
            }
            if let (Some(used), Some(total)) = (f(l, "diskUsedGb"), f(l, "diskTotalGb")) {
                parts.push(format!("disk {used:.1} of {total:.1} GB"));
            }
            let _ = writeln!(out, "  {}", parts.join(" · "));
        }
        let top: Vec<String> = list(a, "top")
            .iter()
            .take(3)
            .map(|p| format!("{} {}%", s(p, "name").unwrap_or("?"), n(p, "cpu").unwrap_or(0)))
            .collect();
        if !top.is_empty() {
            let _ = writeln!(out, "  {}", dim(&format!("busiest: {}", top.join(" · "))));
        }
    }
    let as_of = (!live).then(|| n(a, "asOf").map(|t| format!("before it slept, {}", clock(t)))).flatten();
    section(out, "finished", as_of.as_deref());
    let finished = list(a, "finished");
    if finished.is_empty() {
        let _ = writeln!(out, "  {}", dim("Nothing finished in the last day."));
    }
    for t in finished {
        let status = match s(t, "status").unwrap_or("done") {
            "completed" => "done",
            "rolled_back" => "rolled back",
            other => other,
        };
        let mut line = format!("{status:8} {}", s(t, "title").unwrap_or("Untitled thread"));
        if let Some(p) = s(t, "project") {
            let _ = write!(line, " · {p}");
        }
        if let Some(at) = n(t, "endedAt") {
            let _ = write!(line, " · {}", ago(at, now));
        }
        let _ = writeln!(out, "  {line}");
        if let Some(e) = s(t, "error") {
            let _ = writeln!(out, "           {}", dim(e));
        }
    }
}

fn section(out: &mut String, title: &str, extra: Option<&str>) {
    let _ = match extra {
        Some(extra) => writeln!(out, "\n{} {}", bold(title), dim(&format!("· {extra}"))),
        None => writeln!(out, "\n{}", bold(title)),
    };
}

/// `t3agent37 projects`: this computer's T3 projects that can go to a machine.
pub fn projects(v: &Value) -> String {
    let rows = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return "No T3 projects with a git remote on this computer.\n".into();
    }
    let width = rows.iter().filter_map(|p| s(p, "title")).map(|t| t.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for p in rows {
        let title = s(p, "title").unwrap_or("?");
        let _ = writeln!(out, "{}{}  {}", bold(title), " ".repeat(width - title.chars().count()), dim(s(p, "repoKey").unwrap_or_default()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_795_402_600_000;

    fn machines_json() -> Value {
        json!({
            "machines": [
                {"id": "ve92hic1ra", "name": "skiff", "size": "large", "state": "working", "detail": "Working · Fix login",
                 "signedOut": [], "projects": [{"title": "Tantalum", "status": "Cloning 40%"}], "keepAwakeForever": false},
                {"id": "ssuxzy3xa0", "name": "faktor", "size": "default", "state": "idle", "detail": "Idle · sleeps in 4m",
                 "countdown": {"template": "Idle · sleeps in {left}", "endsAt": NOW + 125_000},
                 "signedOut": ["Claude"], "githubCode": "ABCD-1234", "keepAwakeUntil": null},
            ],
            "elsewhere": [{"id": "x", "name": "raven", "size": "small", "asleep": true}],
            "costs": {"today": 0.084, "month": 0.862, "balance": 9.3669},
        })
    }

    #[test]
    fn the_machine_list() {
        let v = machines_json();
        let out = machines(&v, NOW);
        assert!(out.contains("skiff   large    working  Working · Fix login"), "{out}");
        // The countdown as of now, not as the answer said it.
        assert!(out.contains("faktor  default  idle     Idle · sleeps in 2m"), "{out}");
        assert!(out.contains("GitHub code ABCD-1234"));
        assert!(out.contains("not signed in: Claude (t3agent37 sign-in faktor <agent>)"));
        assert!(out.contains("raven  small  asleep  t3agent37 add-here raven"));
        assert!(out.ends_with("today $0.08 · this month $0.86 · wallet $9.37\n"), "{out}");
        assert_eq!(find(&v, "SKIFF").and_then(|m| s(m, "id")), Some("ve92hic1ra"));
        assert_eq!(find(&v, "ssuxzy3xa0").and_then(|m| s(m, "name")), Some("faktor"));
        assert!(find(&v, "nope").is_none());
        assert_eq!(progress(&v["machines"][0], NOW), "skiff: Working · Fix login · Tantalum: Cloning 40%");
        assert!(machines(&json!({"machines": [], "costs": {}}), NOW).starts_with("No machines yet"));
    }

    #[test]
    fn a_machines_details() {
        let row = &machines_json()["machines"][0];
        let d = json!({
            "costToday": 0.0622, "costMonth": 0.7615, "awakeSecsToday": 3840,
            "agents": [
                {"name": "Claude", "signedIn": true, "email": "a@example.com", "plan": "Ultra", "usage": [{"label": "Weekly", "usedPercent": 39.0}]},
                {"name": "Codex", "signedIn": false},
                {"name": "GitHub", "signedIn": true, "login": "octocat"},
            ],
            "t3Version": "0.1.12", "createdAt": 1795052701, "connect": true,
            "servers": [{"port": 3000, "process": "next-server"}], "previews": [],
            "projectCosts": [{"title": "Grok", "dollarsToday": 0.03, "dollarsMonth": 0.057}],
            "webhooks": [{"title": "Sentry alert", "project": "Tantalum", "enabled": true, "wakingUrl": "https://secret.example/hook"}],
            "start": {"url": "https://secret.example/start/token", "token": "token"},
            "activity": {"live": true, "asOf": NOW, "wokeAt": NOW - 60_000, "wokeFor": "GitHub (you/repo)",
                "threads": [{"title": "Fix login", "state": "needs your approval", "needsYou": true, "project": "Tantalum",
                    "agent": "Opus 5.5, high", "subagents": 2, "startedAt": NOW - 720_000, "background": ["bun run dev"]}],
                "finished": [{"title": "Old", "status": "failed", "project": "Grok", "endedAt": NOW - 7_200_000, "error": "Usage limit reached"}],
                "load": {"cores": 1.84, "cpus": 4, "memUsedMb": 1260, "memTotalMb": 7900, "diskUsedGb": 6.1, "diskTotalGb": 19.6},
                "top": [{"name": "next build", "cpu": 180}, {"name": "T3 server", "cpu": 12}]},
        });
        let out = details(row, &d, NOW);
        for want in [
            "skiff · large · working",
            "T3 0.1.12 · T3 Connect on · made 2026-11-",
            "  Claude  Ultra · a@example.com · Weekly 39%",
            "  Codex   not signed in: t3agent37 sign-in skiff codex",
            "  GitHub  @octocat",
            "! Fix login · needs your approval · Tantalum · Opus 5.5, high · 2 subagents · 12m",
            "    ↳ bun run dev",
            "cpu 1.8 of 4 cores · memory 1.2 GB of 7.7 GB · disk 6.1 of 19.6 GB",
            "busiest: next build 180% · T3 server 12%",
            "failed   Old · Grok · 2h ago",
            "Usage limit reached",
            "  Tantalum · Cloning 40%",
            "today $0.06 (awake 1h 4m) · this month $0.76",
            "Grok: today $0.03 · this month $0.06",
            "3000 (next-server) t3agent37 port skiff 3000 --open",
            "webhook: Sentry alert · Tantalum",
            "start URL on",
        ] {
            assert!(out.contains(want), "missing {want:?} in\n{out}");
        }
        // Secrets stay in --json.
        assert!(!out.contains("secret.example"), "{out}");
        // Asleep: no "now", and "finished" as of the sleep.
        let mut asleep = d.clone();
        asleep["activity"]["live"] = json!(false);
        let out = details(row, &asleep, NOW);
        assert!(!out.contains("busiest") && out.contains("finished · before it slept"), "{out}");
    }

    #[test]
    fn times() {
        assert_eq!((duration(42), duration(720), duration(3840)), ("42s".into(), "12m".into(), "1h 4m".into()));
        assert_eq!(ago(NOW - 30_000, NOW), "just now");
        assert_eq!(ago(NOW - 3 * 86_400_000, NOW), "3d ago");
        assert_eq!(money(0.005), "$0.01");
        assert_eq!(megabytes(536), "536 MB");
    }

    #[test]
    fn which_machines_use_which() {
        let link = json!({"machine": "faktor", "machineId": "ssuxzy3xa0", "access": "full-access", "expiresAt": "2026-12-23T14:02:32.162Z"});
        let v = json!({"machines": [
            {"id": "ve92hic1ra", "name": "skiff", "links": [link], "usedBy": []},
            {"id": "ssuxzy3xa0", "name": "faktor", "links": [], "usedBy": [{"machine": "skiff", "machineId": "ve92hic1ra", "access": "full-access", "expiresAt": "2026-12-23T14:02:32.162Z"}]},
        ]});
        let all = links(&v, None);
        assert!(all.starts_with("skiff can use faktor · full · until 23 Dec"), "{all}");
        assert_eq!(all.lines().count(), 1);
        assert!(links(&v, Some("faktor")).starts_with("skiff can use faktor · full"));
        assert!(links(&json!({"machines": []}), None).starts_with("No machine uses another"));
        let error = json!({"machine": "faktor", "access": "read-only", "lastError": "unreachable"});
        assert_eq!(link_line(&error), "faktor · read · unreachable");
    }
}
