//! A project's env files on a machine (`.env`, `.env.local`, `.dev.vars`, …): they come from this
//! computer's copy of the project when it's added, and again whenever the user asks, over what's
//! there. t3agent37 keeps the files' names and the names of the variables they set, never their
//! values, so the details can say what a machine's agents have without waking it.

use super::MachineRuntime;
use crate::agent37::shell_quote;
use crate::projects::Project;
use crate::store::{EnvFile, ProjectEnv};
use anyhow::{Result, anyhow};

/// The variables an env file sets, in order: `KEY=…` and `export KEY=…` lines, without
/// comments, and without the lines inside a quoted value that runs over several (a PEM key).
pub fn variable_names(contents: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut open_quote: Option<char> = None;
    for line in contents.lines() {
        if let Some(q) = open_quote {
            if line.contains(q) {
                open_quote = None;
            }
            continue;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let ok = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-';
        if key.is_empty() || !key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') || !key.chars().all(ok) {
            continue;
        }
        let value = value.trim_start();
        if let Some(q) = value.chars().next().filter(|c| *c == '"' || *c == '\'')
            && !value[1..].contains(q)
        {
            open_quote = Some(q);
        }
        if !names.iter().any(|n| n == key) {
            names.push(key.to_owned());
        }
    }
    names
}

/// A short hash of a file's contents, to tell a changed copy from the one last copied.
pub fn content_hash(contents: &str) -> String {
    format!("{:016x}", super::provision::fnv1a(contents.bytes()))
}

/// What arrived of `files` (path below the project, contents), without their values: those
/// not in `failed` (each starts with its path), by their path on the machine, below its checkout.
pub fn arrived(project: &Project, files: &[(String, String)], failed: &[String]) -> Vec<EnvFile> {
    files
        .iter()
        .filter(|(path, _)| !failed.iter().any(|e| e.starts_with(&format!("{path}:"))))
        .map(|(path, contents)| EnvFile { path: on_machine(project, path), names: variable_names(contents), hash: content_hash(contents) })
        .collect()
}

fn on_machine(project: &Project, path: &str) -> String {
    if project.subdirectory.is_empty() { path.to_owned() } else { format!("{}/{path}", project.subdirectory) }
}

impl MachineRuntime {
    /// The env files its project got from this computer, as last copied. Never wakes it.
    pub fn env_of(&self, project_id: &str) -> Option<ProjectEnv> {
        self.record().upkeep.env.into_iter().find(|e| e.project_id == project_id)
    }

    /// Records the env files its project `project_id` has from this computer now.
    pub fn note_env(&self, project_id: &str, files: Vec<EnvFile>) {
        let entry = ProjectEnv { project_id: project_id.to_owned(), files, copied_at: chrono::Utc::now().timestamp() };
        self.update_record(|r| {
            r.upkeep.env.retain(|e| e.project_id != project_id);
            if !entry.files.is_empty() {
                r.upkeep.env.push(entry);
            }
        });
    }

    /// Writes `files` into its project at `root` over what's there, and records them. Wakes it.
    /// Returns what couldn't be written, with why.
    pub async fn copy_env(&self, project_id: &str, root: &str, local: &Project, files: Vec<(String, String)>) -> Result<Vec<String>> {
        self.ensure_awake().await?;
        let failed = self.kept_awake(self.write_files(local, root, files.clone(), false)).await;
        self.note_env(project_id, arrived(local, &files, &failed));
        Ok(failed)
    }

    /// Deletes the env files t3agent37 copied into its project at `root`, and forgets them. Wakes
    /// it. Returns how many were there.
    pub async fn remove_env(&self, project_id: &str, root: &str) -> Result<usize> {
        let env = self.env_of(project_id).ok_or_else(|| anyhow!("no env files t3agent37 copied there"))?;
        self.ensure_awake().await?;
        let paths: Vec<String> = env.files.iter().map(|f| shell_quote(&f.path)).collect();
        let script = format!("cd {} || exit 1\nrm -f -- {}\n", super::projects::home_path(root), paths.join(" "));
        self.shared.a37.exec_ok(&self.sandbox_id(), &script, 30).await?;
        self.update_record(|r| r.upkeep.env.retain(|e| e.project_id != project_id));
        Ok(env.files.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_never_values() {
        let file = "# comment\nDATABASE_URL=postgres://u:p@h/db\nexport STRIPE_KEY='sk_live_123'\n\
                    PRIVATE_KEY=\"-----BEGIN KEY-----\nMIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Qu==\n-----END KEY-----\"\n\
                    EMPTY=\n  SPACED = x\n1BAD=x\nnot a line\nDATABASE_URL=again\n";
        assert_eq!(variable_names(file), ["DATABASE_URL", "STRIPE_KEY", "PRIVATE_KEY", "EMPTY", "SPACED"]);
        assert_ne!(content_hash("A=1"), content_hash("A=2"));
    }
}
