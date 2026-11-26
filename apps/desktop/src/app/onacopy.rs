//! Letting agents on this computer send work to copies of the base: the user's switch
//! (`Settings::agents_spawn`, "Agents may send work to copies"), which `spawn_machine` over MCP
//! obeys, and the `on-a-copy` skill it installs for Claude Code and Codex here, so a thread in
//! any project can say "on a copy: …" and get it done there (`on-a-copy.md`).

use super::*;
use std::path::Path;

const SKILL: &str = include_str!("on-a-copy.md");
const NAME: &str = "on-a-copy";
/// In every skill t3agent37 writes, so it never overwrites or removes one it didn't.
const MARK: &str = "Written by t3agent37";

/// Where the skill goes: Claude Code's and Codex's skill folders, for each installed here.
fn skill_dirs(home: &Path) -> Vec<PathBuf> {
    [".claude", ".codex"].iter().map(|d| home.join(d)).filter(|d| d.is_dir()).map(|d| d.join("skills").join(NAME)).collect()
}

/// Writes the skill (`on`), or removes the one t3agent37 wrote. Returns the folders it changed.
pub(super) fn sync_skill(home: &Path, on: bool) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    for dir in skill_dirs(home) {
        let file = dir.join("SKILL.md");
        let ours = std::fs::read_to_string(&file).ok().map(|t| t.contains(MARK));
        let done = match (on, ours) {
            (true, Some(false)) => {
                tracing::warn!("{} isn't t3agent37's: left as it is", file.display());
                continue;
            }
            (true, Some(true)) if std::fs::read_to_string(&file).is_ok_and(|t| t == SKILL) => continue,
            (true, _) => std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, SKILL)),
            // Its folder too, if that leaves it empty.
            (false, Some(true)) => std::fs::remove_file(&file).map(|()| {
                let _ = std::fs::remove_dir(&dir);
            }),
            (false, _) => continue,
        };
        match done {
            Ok(()) => changed.push(dir),
            Err(e) => tracing::warn!("{}: {e}", file.display()),
        }
    }
    changed
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

impl App {
    /// The skill as this version has it, while the switch is on. At start: never removed then,
    /// so a second t3agent37 with a data folder of its own (switch off) leaves it alone.
    pub(super) fn keep_skill_current(&self) {
        if self.settings().agents_spawn
            && let Some(home) = home()
        {
            sync_skill(&home, true);
        }
    }

    /// The switch changed: the skill follows it.
    pub(super) fn skill_follows(&self, on: bool) {
        if let Some(home) = home() {
            sync_skill(&home, on);
        }
    }

    /// Whether agents here may send work to copies (`on`: they may; None: says which).
    pub fn set_on_a_copy(&self, on: Option<bool>) -> String {
        if let Some(on) = on {
            let mut s = self.settings();
            s.agents_spawn = on;
            self.update_settings(s);
        }
        let installed: Vec<String> = home()
            .map(|h| skill_dirs(&h).into_iter().filter(|d| d.join("SKILL.md").is_file()).map(|d| format!("~/{}", d.strip_prefix(&h).unwrap_or(&d).display())).collect())
            .unwrap_or_default();
        if self.settings().agents_spawn {
            let mut text = "Agents here may send work to copies of the base: in a T3 Code thread in any project, \"on a copy: …\" or /on-a-copy runs the task on a copy with the project as it is, and brings the changes back.".to_owned();
            if self.bases().is_empty() {
                text.push_str(" There's no base yet: `t3agent37 base <machine> on` makes one.");
            }
            if !installed.is_empty() {
                text.push_str(&format!(" The skill that tells Claude and Codex how: {}.", installed.join(", ")));
            }
            text
        } else {
            "Agents here may not send work to copies (spawn_machine is refused). `t3agent37 on-a-copy on` lets them, with a skill for Claude and Codex.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_follows_the_switch_and_leaves_others_alone() {
        let home = std::env::temp_dir().join(format!("t3agent37-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        // Codex isn't installed: nothing goes there.
        assert_eq!(sync_skill(&home, true), [home.join(".claude/skills/on-a-copy")]);
        let file = home.join(".claude/skills/on-a-copy/SKILL.md");
        assert!(std::fs::read_to_string(&file).unwrap().starts_with("---\nname: on-a-copy\n"));
        assert!(!home.join(".codex").exists());
        // Again: nothing to change.
        assert!(sync_skill(&home, true).is_empty());
        sync_skill(&home, false);
        assert!(!home.join(".claude/skills/on-a-copy").exists());
        assert!(home.join(".claude/skills").is_dir());
        // One of the user's own by that name stays, on or off.
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "---\nname: on-a-copy\n---\nmine\n").unwrap();
        assert!(sync_skill(&home, true).is_empty());
        assert!(sync_skill(&home, false).is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "---\nname: on-a-copy\n---\nmine\n");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_skill_says_who_wrote_it() {
        assert!(SKILL.contains(MARK));
        assert!(SKILL.starts_with("---\nname: on-a-copy\ndescription: "));
    }
}
