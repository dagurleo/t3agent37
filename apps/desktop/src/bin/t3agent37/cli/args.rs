//! A command's arguments: words in order, and `--flag`, `--flag value` or `--flag=value`. Each
//! command says which flags it takes and which of them take a value.

/// A flag a command takes: its name, and whether a value follows it.
pub type Flag = (&'static str, bool);

#[derive(Debug, PartialEq)]
pub struct Args {
    words: std::collections::VecDeque<String>,
    flags: Vec<(&'static str, Option<String>)>,
}

impl Args {
    pub fn parse(raw: &[String], known: &[Flag]) -> Result<Self, String> {
        let mut words = std::collections::VecDeque::new();
        let mut flags = Vec::new();
        let mut it = raw.iter();
        while let Some(arg) = it.next() {
            if arg == "--" {
                words.extend(it.by_ref().cloned());
                break;
            }
            let Some(flag) = arg.strip_prefix("--") else {
                if arg.len() > 1 && arg.starts_with('-') && arg != "-h" {
                    return Err(format!("unknown option {arg}"));
                }
                if arg == "-h" {
                    flags.push(("help", None));
                } else {
                    words.push_back(arg.clone());
                }
                continue;
            };
            let (name, inline) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (flag, None),
            };
            let Some(&(name, valued)) = known.iter().chain(&[("help", false)]).find(|(n, _)| *n == name) else {
                return Err(format!("unknown option --{name}"));
            };
            let value = match (valued, inline) {
                (true, Some(v)) => Some(v),
                (true, None) => Some(it.next().cloned().ok_or_else(|| format!("--{name} needs a value"))?),
                (false, None) => None,
                (false, Some(_)) => return Err(format!("--{name} takes no value")),
            };
            flags.push((name, value));
        }
        Ok(Self { words, flags })
    }

    /// Whether the flag was given.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| *n == name)
    }

    /// The flag's value; the last one when it's given more than once.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(n, _)| *n == name).and_then(|(_, v)| v.as_deref())
    }

    /// Every value of a flag that can be given more than once (`--project a --project b`).
    pub fn values(&self, name: &str) -> Vec<String> {
        self.flags.iter().filter(|(n, _)| *n == name).filter_map(|(_, v)| v.clone()).collect()
    }

    /// The next word, which the command needs: `what` names it in the error.
    pub fn word(&mut self, what: &str) -> Result<String, String> {
        self.words.pop_front().ok_or_else(|| format!("missing {what}"))
    }

    /// The next word, if there is one.
    pub fn maybe_word(&mut self) -> Option<String> {
        self.words.pop_front()
    }

    /// No words left over.
    pub fn done(&self) -> Result<(), String> {
        match self.words.front() {
            Some(extra) => Err(format!("unexpected {extra}")),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &[&str], known: &[Flag]) -> Result<Args, String> {
        Args::parse(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>(), known)
    }

    #[test]
    fn words_and_flags() {
        let known = [("size", true), ("project", true), ("json", false)];
        let mut a = parse(&["raven", "--size", "large", "--project=Tantalum", "--project", "cork", "--json"], &known).unwrap();
        assert_eq!(a.word("name").unwrap(), "raven");
        assert_eq!(a.value("size"), Some("large"));
        assert_eq!(a.values("project"), ["Tantalum", "cork"]);
        assert!(a.flag("json") && !a.flag("help"));
        assert!(a.done().is_ok());
        assert_eq!(a.word("machine"), Err("missing machine".into()));
        // After `--`, a word that looks like a flag is a word.
        let mut a = parse(&["--", "--odd"], &known).unwrap();
        assert_eq!(a.word("name").unwrap(), "--odd");
        assert!(parse(&["-h"], &[]).unwrap().flag("help"));
        assert!(parse(&["--help"], &[]).unwrap().flag("help"));
    }

    #[test]
    fn refuses_what_it_doesnt_know() {
        let known = [("size", true), ("yes", false)];
        assert_eq!(parse(&["--colour"], &known), Err("unknown option --colour".into()));
        assert_eq!(parse(&["-x"], &known), Err("unknown option -x".into()));
        assert_eq!(parse(&["--size"], &known), Err("--size needs a value".into()));
        assert_eq!(parse(&["--yes=1"], &known), Err("--yes takes no value".into()));
        let a = parse(&["skiff", "extra"], &known).unwrap();
        let mut a = a;
        a.word("machine").unwrap();
        assert_eq!(a.done(), Err("unexpected extra".into()));
    }
}
