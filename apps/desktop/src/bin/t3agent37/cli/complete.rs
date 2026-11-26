//! Shell completions: `t3agent37 completions zsh|bash|fish` prints a script that asks this
//! program what comes next (`t3agent37 __complete <shell> <words…>`), so the commands, their
//! options, machine names and projects come from the running t3agent37, never from a list
//! written into the script.

use super::client::Backend;
use super::{COMMANDS, command};
use serde_json::{Value, json};
use t3agent37_lib::machine::provision::SIZES;

/// What a command's words are, in order.
#[derive(Clone, Copy)]
enum Slot {
    Machine,
    /// A machine made on another computer (`add-here`).
    Elsewhere,
    /// One of this computer's T3 projects.
    LocalProject,
    /// A project on the machine named before it.
    MachineProject,
    Size,
    Command,
    Words(&'static [&'static str]),
}

const ON_OFF: Slot = Slot::Words(&["on", "off"]);

fn slots(command: &str) -> &'static [Slot] {
    use Slot::*;
    match command {
        "details" | "delete" | "wake" | "sleep" | "open" | "pair" | "connect" | "wake-link" | "port" => &[Machine],
        "add-here" => &[Elsewhere],
        "resize" => &[Machine, Size],
        "keep-awake" => &[Machine, Words(&["30", "60", "120", "off"])],
        "auto-sleep" => &[Machine, ON_OFF],
        "add-project" => &[Machine, LocalProject],
        "sign-in" => &[Machine, Words(&["claude", "codex", "github"])],
        "issues" => &[Machine, MachineProject, ON_OFF],
        "start-url" => &[Machine, Words(&["on", "off", "replace"])],
        "add-mcp" => &[Words(&["claude-code", "codex"])],
        "link" | "unlink" => &[Machine, Machine],
        "copy" | "bench" | "collect" => &[Machine],
        "base" => &[Machine, Words(&["on", "off", "refresh"])],
        "on-a-copy" => &[ON_OFF],
        "new-machines" => &[Words(&["start", "claude", "github", "codex", "setup"])],
        "delete-after" => &[Machine, Words(&["30m", "1h", "2h", "1d", "never"])],
        "limit" => &[Machine, Words(&["5", "10", "25", "50", "off"])],
        "env" => &[Machine, MachineProject],
        "links" => &[Machine],
        "completions" => &[Words(&["zsh", "bash", "fish"])],
        "help" => &[Command],
        _ => &[],
    }
}

/// The values a flag takes, where they're known.
fn flag_slot(flag: &str) -> Option<Slot> {
    match flag {
        "size" => Some(Slot::Size),
        "project" => Some(Slot::LocalProject),
        "effort" => Some(Slot::Words(&["low", "medium", "high", "xhigh", "max"])),
        "access" => Some(Slot::Words(&["read", "ask", "edits", "auto", "full"])),
        "sizes" => Some(Slot::Size),
        "delete-after" => Some(Slot::Words(&["30m", "1h", "2h", "4h", "1d"])),
        "base" => Some(Slot::Machine),
        "max-hours" => Some(Slot::Words(&["2", "4", "8", "24"])),
        "timeout" => Some(Slot::Words(&["10", "30", "60", "120"])),
        _ => None,
    }
}

/// `__complete <shell> <program> <words…> <current>`: what may come next, one per line, in the
/// shell's form. Quiet when it can't say: a shell's completion never shows an error.
pub async fn run(args: &[String]) -> i32 {
    let Some((shell, rest)) = args.split_first() else { return 0 };
    let words: Vec<String> = rest.iter().skip(1).map(|w| if shell == "bash" { unescape(w) } else { w.clone() }).collect();
    let Some((current, before)) = words.split_last() else { return 0 };
    let mut out = String::new();
    for (word, about) in candidates(before, current).await {
        if !word.starts_with(current.as_str()) {
            continue;
        }
        let line = match shell.as_str() {
            "zsh" if about.is_empty() => word.replace(':', "\\:"),
            "zsh" => format!("{}:{}", word.replace(':', "\\:"), about),
            "fish" if about.is_empty() => word,
            "fish" => format!("{word}\t{about}"),
            _ => escape(&word),
        };
        out.push_str(&line);
        out.push('\n');
    }
    print!("{out}");
    0
}

async fn candidates(before: &[String], current: &str) -> Vec<(String, String)> {
    let Some((name, rest)) = before.split_first() else {
        let mut all: Vec<(String, String)> = COMMANDS.iter().map(|c| (c.name.to_owned(), summary(c.about))).collect();
        all.push(("help".into(), "A command's usage".into()));
        return all;
    };
    let Some(c) = command(name).map(|c| c.name).or((name == "help").then_some("help")) else { return Vec::new() };
    let flags = command(c).map(|c| c.flags).unwrap_or_default();
    // The flag before this word, if it takes a value.
    if let Some(flag) = rest.last().and_then(|w| w.strip_prefix("--")).filter(|f| flags.iter().any(|(n, v)| n == f && *v)) {
        return match flag_slot(flag) {
            Some(slot) => values(slot, rest).await,
            None => Vec::new(),
        };
    }
    if current.starts_with('-') {
        return flags.iter().map(|(n, _)| (format!("--{n}"), String::new())).chain([("--help".to_owned(), String::new())]).collect();
    }
    let index = positional(rest, flags).len();
    match slots(c).get(index) {
        Some(&slot) => values(slot, rest).await,
        None => Vec::new(),
    }
}

/// The words given so far, without the options and their values.
fn positional<'a>(rest: &'a [String], flags: &[(&str, bool)]) -> Vec<&'a str> {
    let mut words = Vec::new();
    let mut it = rest.iter();
    while let Some(w) = it.next() {
        match w.strip_prefix("--") {
            Some(f) if !f.contains('=') && flags.iter().any(|(n, v)| *n == f && *v) => {
                it.next();
            }
            Some(_) => {}
            None if w.starts_with('-') && w.len() > 1 => {}
            None => words.push(w.as_str()),
        }
    }
    words
}

async fn values(slot: Slot, rest: &[String]) -> Vec<(String, String)> {
    let b = Backend::quick();
    let named = |v: &Value, key: &str, about: &str| -> Vec<(String, String)> {
        v[key]
            .as_array()
            .map(|a| a.iter().filter_map(|m| Some((m["name"].as_str()?.to_owned(), m[about].as_str().unwrap_or_default().to_owned()))).collect())
            .unwrap_or_default()
    };
    match slot {
        Slot::Words(words) => words.iter().map(|w| (w.to_string(), String::new())).collect(),
        Slot::Size => SIZES.iter().map(|s| (s.id.to_owned(), format!("{} · ${:.3} an hour awake", s.spec, s.hourly))).collect(),
        Slot::Command => COMMANDS.iter().map(|c| (c.name.to_owned(), summary(c.about))).collect(),
        Slot::Machine => b.json("list_machines", json!({})).await.map(|v| named(&v, "machines", "detail")).unwrap_or_default(),
        Slot::Elsewhere => b.json("list_machines", json!({})).await.map(|v| named(&v, "elsewhere", "size")).unwrap_or_default(),
        Slot::LocalProject => b
            .json("list_projects", json!({}))
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .map(|a| a.iter().filter_map(|p| Some((p["title"].as_str()?.to_owned(), p["repoKey"].as_str().unwrap_or_default().to_owned()))).collect())
            .unwrap_or_default(),
        Slot::MachineProject => {
            let Some(machine) = rest.iter().find(|w| !w.starts_with('-')) else { return Vec::new() };
            let Ok(list) = b.json("list_machines", json!({})).await else { return Vec::new() };
            let Some(m) = super::show::find(&list, machine) else { return Vec::new() };
            m["projects"]
                .as_array()
                .map(|a| a.iter().filter_map(|p| Some((p["title"].as_str()?.to_owned(), String::new()))).collect())
                .unwrap_or_default()
        }
    }
}

/// A command's description up to its first full stop, as `help` lists it.
fn summary(about: &str) -> String {
    about.split(". ").next().unwrap_or(about).to_owned()
}

/// A word as bash puts it on the command line: what the shell would split or expand, escaped.
fn escape(word: &str) -> String {
    let mut out = String::new();
    for ch in word.chars() {
        if !(ch.is_alphanumeric() || "-_./:=@%+,".contains(ch)) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A word as bash passes it while it's typed (`T3\ 37`): the escapes taken out.
fn unescape(word: &str) -> String {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.extend(chars.next()),
            '\'' | '"' => {}
            ch => out.push(ch),
        }
    }
    out
}

/// `t3agent37 completions <shell>`: the script for that shell.
pub fn script(shell: &str) -> Option<&'static str> {
    Some(match shell {
        "zsh" => ZSH,
        "bash" => BASH,
        "fish" => FISH,
        _ => return None,
    })
}

const ZSH: &str = r#"#compdef t3agent37
# t3agent37's completions for zsh. Put this file on your $fpath as _t3agent37, or add
# `source <(t3agent37 completions zsh)` to ~/.zshrc after compinit.
_t3agent37() {
  local out
  out=$(command t3agent37 __complete zsh "${(@Q)words[1,CURRENT]}" 2>/dev/null) || return 1
  [[ -n $out ]] || return 1
  local -a items
  items=("${(@f)out}")
  _describe 't3agent37' items
}
if [[ "${funcstack[1]}" == "_t3agent37" ]]; then
  _t3agent37 "$@"
else
  compdef _t3agent37 t3agent37
fi
"#;

const BASH: &str = r#"# t3agent37's completions for bash. Add `source <(t3agent37 completions bash)` to ~/.bashrc,
# or put this file in bash-completion's completions folder as t3agent37.
_t3agent37() {
  local IFS=$'\n'
  COMPREPLY=($(command t3agent37 __complete bash "${COMP_WORDS[@]:0:COMP_CWORD+1}" 2>/dev/null))
}
complete -o default -F _t3agent37 t3agent37
"#;

const FISH: &str = r#"# t3agent37's completions for fish. Save as ~/.config/fish/completions/t3agent37.fish, or run
# `t3agent37 completions fish | source`.
function __t3agent37_complete
    set -l words (commandline -opc)
    set -l current (commandline -ct)
    command t3agent37 __complete fish $words "$current" 2>/dev/null
end
complete -c t3agent37 -f -a '(__t3agent37_complete)'
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn what_comes_next() {
        let names = |v: Vec<(String, String)>| v.into_iter().map(|(w, _)| w).collect::<Vec<_>>();
        let first = names(candidates(&[], "").await);
        assert!(first.contains(&"add-project".to_owned()) && first.contains(&"help".to_owned()) && first.contains(&"completions".to_owned()));
        assert!(!first.iter().any(|w| w.starts_with("__")));
        assert_eq!(names(candidates(&words(&["resize", "raven"]), "").await)[0], "small");
        assert_eq!(names(candidates(&words(&["create", "raven", "--size"]), "").await)[0], "small");
        assert_eq!(names(candidates(&words(&["issues", "raven", "Tantalum"]), "").await), ["on", "off"]);
        assert_eq!(names(candidates(&words(&["issues", "raven", "--label", "x", "Tantalum"]), "").await), ["on", "off"]);
        assert_eq!(names(candidates(&words(&["sign-in", "raven"]), "--").await), ["--other-account", "--cancel", "--token", "--no-token", "--help"]);
        assert_eq!(names(candidates(&words(&["help"]), "").await).len(), COMMANDS.len());
        assert!(candidates(&words(&["sizes"]), "").await.is_empty());
        assert!(candidates(&words(&["nonsense"]), "").await.is_empty());
    }

    #[test]
    fn bash_words() {
        assert_eq!(escape("T3 37"), "T3\\ 37");
        assert_eq!(escape("it's"), "it\\'s");
        assert_eq!(unescape("T3\\ 37"), "T3 37");
        assert_eq!(unescape("'T3 37"), "T3 37");
        for shell in ["zsh", "bash", "fish"] {
            assert!(script(shell).unwrap().contains("__complete"));
        }
    }
}
