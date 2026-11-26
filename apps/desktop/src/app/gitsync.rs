//! A checkout on this computer to a machine and back, with git. Its files as they are (commits
//! not pushed, changes not committed, new files) go as one commit on its HEAD, in a bundle
//! without what its remotes have, which the machine fetches itself. What the machine's agent
//! did comes back the same way: its files as a commit, bundled without the history this
//! computer has. The checkout's own index, branches and files are never touched, except by an
//! `apply` asked for.

use crate::agent37::shell_quote;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The most a bundle may weigh, either way.
pub const MAX_BUNDLE: u64 = 25 * 1024 * 1024;

/// Who the snapshot commits are by: they're t3agent37's, and a checkout without a git identity
/// set can make them too.
const WHO: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "t3agent37"),
    ("GIT_AUTHOR_EMAIL", "t3agent37@localhost"),
    ("GIT_COMMITTER_NAME", "t3agent37"),
    ("GIT_COMMITTER_EMAIL", "t3agent37@localhost"),
];

async fn git_bytes(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<Vec<u8>> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .await
        .context("running git")?;
    if !out.status.success() {
        bail!("git {}: {}", args.first().copied().unwrap_or_default(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

async fn git(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_bytes(dir, args, env).await?).trim().to_owned())
}

fn temp_file(what: &str) -> PathBuf {
    std::env::temp_dir().join(format!("t3agent37-{what}-{}-{:08x}", std::process::id(), rand::random::<u32>()))
}

/// A checkout's files as they were at one moment.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The checkout's top folder.
    pub top: PathBuf,
    /// The folder asked for, below `top` ("" for the top).
    pub below: String,
    pub head: String,
    /// The branch checked out, if one is.
    pub branch: Option<String>,
    /// Its files as they were: a commit on `head`.
    pub commit: String,
    /// They differ from `head`'s: changes not committed, or new files.
    pub changed: bool,
    /// `github.com/owner/name`, from its `origin`.
    pub repo_key: Option<String>,
}

impl Snapshot {
    pub fn short_head(&self) -> &str {
        &self.head[..self.head.len().min(7)]
    }
}

/// The checkout `folder` is in, with its files as they are now (ignored ones aside), taken with
/// an index of its own.
pub async fn snapshot(folder: &Path) -> Result<Snapshot> {
    let folder = folder.canonicalize().with_context(|| format!("no folder {}", folder.display()))?;
    let top = git(&folder, &["rev-parse", "--show-toplevel"], &[]).await.map_err(|_| anyhow!("{} isn't in a git checkout", folder.display()))?;
    let top = PathBuf::from(top).canonicalize()?;
    let below = folder.strip_prefix(&top).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let head = git(&top, &["rev-parse", "--verify", "HEAD"], &[]).await.map_err(|_| anyhow!("{} has no commit yet", top.display()))?;
    let branch = git(&top, &["symbolic-ref", "--short", "-q", "HEAD"], &[]).await.ok().filter(|b| !b.is_empty());
    // A copy of its index, so `add` only reads what changed and the real one stays as it is.
    let index = top.join(git(&top, &["rev-parse", "--git-path", "index"], &[]).await?);
    let tmp = temp_file("index");
    let _ = std::fs::copy(&index, &tmp);
    let tmp_str = tmp.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", tmp_str.as_str())];
    let tree = async {
        git(&top, &["add", "-A"], &env).await?;
        git(&top, &["write-tree"], &env).await
    }
    .await;
    let _ = std::fs::remove_file(&tmp);
    let tree = tree?;
    let head_tree = git(&top, &["rev-parse", "HEAD^{tree}"], &[]).await?;
    let commit = git(&top, &["commit-tree", &tree, "-p", &head, "-m", "t3agent37: the checkout's files"], &WHO).await?;
    let origin = git(&top, &["remote", "get-url", "origin"], &[]).await.ok();
    Ok(Snapshot {
        top,
        below,
        branch,
        commit,
        changed: tree != head_tree,
        repo_key: origin.as_deref().and_then(crate::machine::projects::repo_key_of),
        head,
    })
}

/// `commit` as a bundle under `refname` (kept in the checkout), without what its remotes have:
/// what a machine that fetched them still needs.
pub async fn bundle(top: &Path, refname: &str, commit: &str) -> Result<Vec<u8>> {
    git(top, &["update-ref", refname, commit], &[]).await?;
    let file = temp_file("bundle");
    let path = file.to_string_lossy().into_owned();
    let mut args = vec!["bundle", "create", path.as_str(), refname];
    if !git(top, &["for-each-ref", "--count=1", "refs/remotes"], &[]).await?.is_empty() {
        args.extend(["--not", "--remotes"]);
    }
    let made = git(top, &args, &[]).await;
    let bytes = std::fs::read(&file);
    let _ = std::fs::remove_file(&file);
    made?;
    let bytes = bytes?;
    if bytes.len() as u64 > MAX_BUNDLE {
        bail!(
            "the checkout's files not on its remote come to {} MB, more than {} MB to send: push its branch first, or check for big files that aren't ignored",
            bytes.len() / 1_000_000,
            MAX_BUNDLE / 1_000_000
        );
    }
    Ok(bytes)
}

/// Run on the machine, in the repository `root` is in: the bundle checked out as `branch`, with
/// HEAD at `head` and the files as the checkout's. What the machine had there before (changes,
/// files that aren't ignored) goes; ignored ones (dependencies, build output) stay. Prints the
/// repository's folder.
pub fn apply_script(root: &str, bundle: &str, refname: &str, branch: &str, head: &str) -> String {
    format!(
        "set -e\ncd {root}\ncd \"$(git rev-parse --show-toplevel)\"\n\
         git fetch -q origin 2>/dev/null || echo \"t3agent37: couldn't fetch from origin; going by what the machine had\" >&2\n\
         git fetch -q -f {bundle} {spec}\n\
         git reset -q --hard\ngit clean -fdq\n\
         git checkout -q -f -B {branch} refs/t3agent37/sent\n\
         git reset -q --mixed {head}\npwd\n",
        bundle = shell_quote(bundle),
        spec = shell_quote(&format!("{refname}:refs/t3agent37/sent")),
        branch = shell_quote(branch),
        head = shell_quote(head),
    )
}

/// Run on the machine, in the repository `root` is in: its files as they are now (commits,
/// changes, new files) as a commit on its HEAD, bundled at `out` without `head`'s history,
/// which the checkout here has. Prints `same` when nothing changed from what it was given.
pub fn collect_script(root: &str, out: &str, head: &str, machine: &str) -> String {
    format!(
        "set -e\ncd {root}\ncd \"$(git rev-parse --show-toplevel)\"\n\
         export GIT_AUTHOR_NAME=t3agent37 GIT_AUTHOR_EMAIL=t3agent37@localhost GIT_COMMITTER_NAME=t3agent37 GIT_COMMITTER_EMAIL=t3agent37@localhost\n\
         idx=$(mktemp); cp \"$(git rev-parse --git-path index)\" \"$idx\" 2>/dev/null || true\n\
         GIT_INDEX_FILE=\"$idx\" git add -A\ntree=$(GIT_INDEX_FILE=\"$idx\" git write-tree); rm -f \"$idx\"\n\
         if [ \"$tree\" = \"$(git rev-parse 'refs/t3agent37/sent^{{tree}}')\" ] && [ \"$(git rev-parse HEAD)\" = {head} ]; then echo same; exit 0; fi\n\
         commit=$(git commit-tree \"$tree\" -p HEAD -m {message})\n\
         git update-ref refs/t3agent37/result \"$commit\"\n\
         rm -f {out}; git bundle create {out} refs/t3agent37/result --not {head} >/dev/null 2>&1\necho bundle\n",
        head = shell_quote(head),
        out = shell_quote(out),
        message = shell_quote(&format!("t3agent37: {machine}'s work")),
    )
}

/// The branch a machine's work goes to in the checkout at `top`: `t3agent37/<machine>`, or
/// `t3agent37-<machine>` where a branch named `t3agent37` leaves no room for that (git keeps
/// branches as files: `t3agent37` can't be one and a folder too). This repo has one.
pub async fn result_branch(top: &Path, machine: &str) -> String {
    match git(top, &["rev-parse", "--verify", "-q", "refs/heads/t3agent37"], &[]).await {
        Ok(_) => format!("t3agent37-{machine}"),
        Err(_) => format!("t3agent37/{machine}"),
    }
}

/// Takes a machine's work from `bundle` into the checkout: `refs/t3agent37/<machine>/result`,
/// and a new branch on it, `result_branch` (or `-2`, `-3`… when that's taken), or the one there
/// already with the same files: a second collect of the same work (its snapshot commit has a time
/// of its own) made a second branch (2026-11-25). Returns the branch.
pub async fn take_result(top: &Path, machine: &str, bundle: &[u8]) -> Result<String> {
    let file = temp_file("result");
    std::fs::write(&file, bundle)?;
    let path = file.to_string_lossy().into_owned();
    let result_ref = format!("refs/t3agent37/{machine}/result");
    let fetched = git(top, &["fetch", "-q", path.as_str(), &format!("+refs/t3agent37/result:{result_ref}")], &[]).await;
    let _ = std::fs::remove_file(&file);
    fetched?;
    let result = git(top, &["rev-parse", &result_ref], &[]).await?;
    let files = git(top, &["rev-parse", &format!("{result_ref}^{{tree}}")], &[]).await?;
    let first = result_branch(top, machine).await;
    for i in 1.. {
        let branch = if i == 1 { first.clone() } else { format!("{first}-{i}") };
        match git(top, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")], &[]).await {
            Ok(at) if at == result => return Ok(branch),
            Ok(_) if git(top, &["rev-parse", &format!("refs/heads/{branch}^{{tree}}")], &[]).await.is_ok_and(|t| t == files) => return Ok(branch),
            Ok(_) => continue,
            Err(_) => {
                git(top, &["branch", &branch, &result], &[]).await?;
                return Ok(branch);
            }
        }
    }
    unreachable!()
}

const DIFF: [&str; 4] = ["--no-color", "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/"];

/// What changed from `from` to `to`, as `git diff --stat` says it.
pub async fn changes(top: &Path, from: &str, to: &str) -> Result<String> {
    let mut args = vec!["diff", "--stat=100"];
    args.extend(DIFF);
    args.extend([from, to]);
    git(top, &args, &[]).await
}

/// The changes from `from` to `to` made to the checkout's files as they are now, all or none:
/// an error, changing nothing, when they don't fit.
/// Makes the changes from `from` to `to` to the checkout's files, all or none. `false`: they're
/// there already (a second collect of the same work), so nothing was done.
pub async fn apply(top: &Path, from: &str, to: &str) -> Result<bool> {
    let mut args = vec!["diff", "--binary"];
    args.extend(DIFF);
    args.extend([from, to]);
    let patch = git_bytes(top, &args, &[]).await?;
    if patch.is_empty() {
        return Ok(true);
    }
    let file = temp_file("patch");
    std::fs::write(&file, &patch)?;
    let path = file.to_string_lossy().into_owned();
    let there = git(top, &["apply", "--check", "-R", "--whitespace=nowarn", path.as_str()], &[]).await.is_ok();
    let applied = if there { Ok(false) } else { git(top, &["apply", "--whitespace=nowarn", path.as_str()], &[]).await.map(|_| true) };
    let _ = std::fs::remove_file(&file);
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, script: &str) {
        let ok = std::process::Command::new("bash").arg("-c").arg(script).current_dir(dir).envs(WHO).status().unwrap().success();
        assert!(ok, "{script}");
    }

    /// The whole trip on two local repositories: a checkout here with a commit not pushed, a
    /// change and a new file, given to a "machine" that has only what the remote has; its work
    /// comes back as a branch and onto the checkout's files.
    #[tokio::test]
    async fn a_checkout_goes_there_and_its_work_comes_back() {
        let root = temp_file("test");
        std::fs::create_dir_all(&root).unwrap();
        sh(
            &root,
            "git init -q -b main origin && cd origin && echo one > a.txt && git add . && git commit -qm one && cd .. && \
             git clone -q origin here && git clone -q origin machine && cd here && echo two >> a.txt && git commit -qam two && \
             echo three >> a.txt && echo new > b.txt && printf 'target/\\n' > .gitignore && mkdir target && echo big > target/x",
        );
        let here = root.join("here");
        let snap = snapshot(&here.join(".")).await.unwrap();
        assert!(snap.changed && snap.branch.as_deref() == Some("main") && snap.below.is_empty());
        let bundle_bytes = bundle(&snap.top, "refs/t3agent37/m/sent", &snap.commit).await.unwrap();
        // The real index is untouched: b.txt is still untracked there.
        assert_eq!(git(&here, &["status", "--porcelain"], &[]).await.unwrap(), "M a.txt\n?? .gitignore\n?? b.txt");
        let machine = root.join("machine");
        std::fs::write(root.join("sent.bundle"), &bundle_bytes).unwrap();
        let checkout_there = apply_script(&shell_quote(&machine.to_string_lossy()), &root.join("sent.bundle").to_string_lossy(), "refs/t3agent37/m/sent", "main", &snap.head);
        sh(&machine, &checkout_there);
        assert_eq!(std::fs::read_to_string(machine.join("a.txt")).unwrap(), "one\ntwo\nthree\n");
        assert_eq!(std::fs::read_to_string(machine.join("b.txt")).unwrap(), "new\n");
        assert_eq!(git(&machine, &["rev-parse", "HEAD"], &[]).await.unwrap(), snap.head);
        // Nothing done there yet: nothing comes back.
        let out = root.join("result.bundle").to_string_lossy().into_owned();
        let collect = collect_script(&shell_quote(&machine.to_string_lossy()), &out, &snap.head, "m");
        let said = std::process::Command::new("bash").arg("-c").arg(&collect).current_dir(&machine).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&said.stdout).trim(), "same");
        // The machine's agent commits a fix and leaves another change.
        sh(&machine, "echo fixed > c.txt && git add c.txt && git commit -qm fix && echo four >> a.txt");
        let said = std::process::Command::new("bash").arg("-c").arg(&collect).current_dir(&machine).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&said.stdout).trim(), "bundle", "{}", String::from_utf8_lossy(&said.stderr));
        let branch = take_result(&snap.top, "m", &std::fs::read(&out).unwrap()).await.unwrap();
        assert_eq!(branch, "t3agent37/m");
        let stat = changes(&snap.top, "refs/t3agent37/m/sent", "refs/t3agent37/m/result").await.unwrap();
        assert!(stat.contains("a.txt") && stat.contains("c.txt") && !stat.contains("b.txt"), "{stat}");
        apply(&snap.top, "refs/t3agent37/m/sent", "refs/t3agent37/m/result").await.unwrap();
        assert_eq!(std::fs::read_to_string(here.join("a.txt")).unwrap(), "one\ntwo\nthree\nfour\n");
        assert_eq!(std::fs::read_to_string(here.join("c.txt")).unwrap(), "fixed\n");
        // Again, now that it's there: nothing to do.
        assert!(!apply(&snap.top, "refs/t3agent37/m/sent", "refs/t3agent37/m/result").await.unwrap());
        assert_eq!(std::fs::read_to_string(here.join("a.txt")).unwrap(), "one\ntwo\nthree\nfour\n");
        // Taken again, it's the same branch, even as a commit of its own made later.
        assert_eq!(take_result(&snap.top, "m", &std::fs::read(&out).unwrap()).await.unwrap(), "t3agent37/m");
        let later = std::process::Command::new("bash").arg("-c").arg(&collect).current_dir(&machine).env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&later.stdout).trim(), "bundle", "{}", String::from_utf8_lossy(&later.stderr));
        assert_eq!(take_result(&snap.top, "m", &std::fs::read(&out).unwrap()).await.unwrap(), "t3agent37/m");
        // With a branch named t3agent37, one beside it.
        git(&snap.top, &["branch", "-D", "t3agent37/m"], &[]).await.unwrap();
        git(&snap.top, &["branch", "t3agent37"], &[]).await.unwrap();
        assert_eq!(take_result(&snap.top, "m", &std::fs::read(&out).unwrap()).await.unwrap(), "t3agent37-m");
        let _ = std::fs::remove_dir_all(&root);
    }
}
