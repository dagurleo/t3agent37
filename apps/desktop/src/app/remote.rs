//! Files and long commands on a machine over Agent37's exec, which takes one command of under
//! 128 KiB (CONTRIBUTING.md) and gives up after 15 minutes: files go in and out in pieces, and
//! a long command runs in the background as a job, read back as it goes.

use crate::agent37::{shell_quote, Agent37};
use anyhow::{bail, Context, Result};
use base64::Engine;

/// Base64 characters a command carries in, well under exec's limit.
const IN_PIECE: usize = 64 * 1024;
/// Bytes a command hands back, base64 on the way.
pub const OUT_PIECE: usize = 48 * 1024;
/// Where t3agent37's jobs keep their script, output, exit code and process group.
pub const JOBS: &str = "/root/.t3agent37/jobs";

/// Writes `bytes` to `path` on the machine.
pub async fn upload(a37: &Agent37, id: &str, path: &str, bytes: &[u8]) -> Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let part = format!("{path}.b64");
    a37.exec_ok(id, &format!("mkdir -p \"$(dirname {p})\" && : > {part}", p = shell_quote(path), part = shell_quote(&part)), 30).await?;
    // Base64 is ASCII, so any cut is on a character.
    for piece in b64.as_bytes().chunks(IN_PIECE) {
        let piece = std::str::from_utf8(piece).expect("base64 is ASCII");
        a37.exec_ok(id, &format!("printf %s '{piece}' >> {}", shell_quote(&part)), 60).await.context("sending a file")?;
    }
    a37.exec_ok(id, &format!("base64 -d {part} > {p} && rm -f {part}", p = shell_quote(path), part = shell_quote(&part)), 60).await?;
    Ok(())
}

/// `path` on the machine, at most `max` bytes.
pub async fn download(a37: &Agent37, id: &str, path: &str, max: u64) -> Result<Vec<u8>> {
    let size: u64 = a37.exec_ok(id, &format!("stat -c %s {}", shell_quote(path)), 30).await?.trim().parse().context("the file's size")?;
    if size > max {
        bail!("{} MB to bring back, more than {} MB", size / 1_000_000, max / 1_000_000);
    }
    let mut out = Vec::with_capacity(size as usize);
    let pieces = size.div_ceil(OUT_PIECE as u64);
    for i in 0..pieces {
        let b64 = a37
            .exec_ok(id, &format!("dd if={} bs={OUT_PIECE} skip={i} count=1 status=none | base64 -w0", shell_quote(path)), 60)
            .await
            .context("reading a file back")?;
        out.extend(base64::engine::general_purpose::STANDARD.decode(b64.trim()).context("the file's contents")?);
    }
    if out.len() as u64 != size {
        bail!("read {} bytes of {size}", out.len());
    }
    Ok(out)
}

/// Starts `script` (bash) on the machine in the background as job `tag`, in its own process
/// group, so it can outlast an exec and be stopped. Its output goes to `<tag>.out`, its exit
/// code to `<tag>.done`.
pub async fn start_job(a37: &Agent37, id: &str, tag: &str, script: &str) -> Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(script);
    if b64.len() > IN_PIECE {
        bail!("the job's script is too long to send");
    }
    let j = format!("{JOBS}/{tag}");
    let start = format!(
        "mkdir -p {JOBS} && rm -f {j}.out {j}.done && echo {b64} | base64 -d > {j}.sh\n\
         setsid nohup bash -c 'echo $$ > {j}.pid; bash -l {j}.sh > {j}.out 2>&1; echo $? > {j}.done; rm -f {j}.pid' >/dev/null 2>&1 < /dev/null &\n\
         echo started"
    );
    a37.exec_ok(id, &start, 60).await?;
    Ok(())
}

/// What job `tag` wrote from byte `offset` on (at most a piece), and its exit code once done.
pub async fn job_output(a37: &Agent37, id: &str, tag: &str, offset: u64) -> Result<(Vec<u8>, Option<i32>)> {
    let j = format!("{JOBS}/{tag}");
    // The exit code first: output written before it is all there when it's read after.
    let script = format!(
        "cat {j}.done 2>/dev/null || echo running; tail -c +{} {j}.out 2>/dev/null | head -c {OUT_PIECE} | base64 -w0",
        offset + 1
    );
    let out = a37.exec_ok(id, &script, 60).await?;
    let (state, b64) = out.split_once('\n').unwrap_or((out.as_str(), ""));
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).context("the job's output")?;
    let code = match state.trim() {
        "running" => None,
        s => Some(s.parse().with_context(|| format!("the job's exit code: {s}"))?),
    };
    Ok((bytes, code))
}

/// Stops job `tag` and whatever it started.
pub async fn stop_job(a37: &Agent37, id: &str, tag: &str) -> Result<()> {
    let j = format!("{JOBS}/{tag}");
    // Its whole session, which `setsid` made it the leader of: GNU timeout (as `run` uses) moves
    // into a process group of its own, so killing the job's group left the command running
    // (2026-11-25). Killed, it never writes its exit code: 130, as for ^C.
    a37.exec_ok(id, &stop_script(&j), 30).await?;
    Ok(())
}

fn stop_script(j: &str) -> String {
    format!(
        "pid=$(cat {j}.pid 2>/dev/null)\n\
         if [ \"${{pid:-0}}\" -gt 1 ] 2>/dev/null; then\n\
           pkill -TERM -s \"$pid\"; sleep 1; pkill -KILL -s \"$pid\"\n\
         fi\n\
         [ -f {j}.done ] || echo 130 > {j}.done; rm -f {j}.pid; true"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job started as `start_job` starts one, running a command under `timeout` as `run` does,
    /// stops whole: the command and what it started.
    #[test]
    fn a_stopped_job_takes_its_command_with_it() {
        if !std::process::Command::new("setsid").arg("true").status().is_ok_and(|s| s.success())
            || !std::process::Command::new("pkill").arg("-V").output().is_ok_and(|o| o.status.success())
        {
            return eprintln!("no setsid or pkill here (macOS): skipped");
        }
        let dir = std::env::temp_dir().join(format!("t3agent37-job-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let j = dir.join("t").to_string_lossy().into_owned();
        std::fs::write(format!("{j}.sh"), "timeout -k 10 600 bash -c 'sleep 600 & wait'\n").unwrap();
        let start = format!("setsid nohup bash -c 'echo $$ > {j}.pid; bash -l {j}.sh > {j}.out 2>&1; echo $? > {j}.done; rm -f {j}.pid' >/dev/null 2>&1 < /dev/null &");
        std::process::Command::new("bash").arg("-c").arg(&start).status().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let sid = std::fs::read_to_string(format!("{j}.pid")).unwrap().trim().to_owned();
        let alive = || std::process::Command::new("pgrep").args(["-s", &sid]).output().unwrap().status.success();
        assert!(alive());
        std::process::Command::new("bash").arg("-c").arg(stop_script(&j)).status().unwrap();
        assert!(!alive(), "something of the job's is still running");
        assert_eq!(std::fs::read_to_string(format!("{j}.done")).unwrap().trim(), "130");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
