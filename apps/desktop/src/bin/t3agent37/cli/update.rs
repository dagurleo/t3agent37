//! `t3agent37 update`: the latest release's build of the command for this computer, checked
//! against its `.sha256` and moved into place over this program, as `install.sh` does. A copy
//! that something else keeps up to date is left alone: the app's own program (the app updates
//! itself), Homebrew's (`brew upgrade`), and a build from source.

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use t3agent37_lib::machine::provision::cmp_versions;

const REPO: &str = "dagurleo/t3agent37";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `to`: a release's tag (`v0.1.9`) instead of the latest, even an older one.
pub async fn update(check: bool, to: Option<&str>) -> Result<i32> {
    let exe = std::env::current_exe().context("finding this program")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if let Some(why) = kept_elsewhere(&exe) {
        bail!("{why}");
    }
    let http = reqwest::Client::builder()
        .user_agent(format!("t3agent37/{VERSION}"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()?;
    let tag = match to {
        Some(tag) => format!("v{}", tag.trim_start_matches('v')),
        None => latest().await?,
    };
    let version = tag.trim_start_matches('v');
    if to.is_none() && cmp_versions(version, VERSION).is_le() {
        println!("t3agent37 {VERSION} is the latest.");
        return Ok(0);
    }
    if check {
        println!("t3agent37 {version} is out; this is {VERSION}. `t3agent37 update` installs it.");
        return Ok(0);
    }

    let asset = asset()?;
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
    eprintln!("{}", super::show::dim(&format!("downloading {url}")));
    let archive = download(&http, &url).await.with_context(|| format!("getting t3agent37 {version}"))?;
    let sums = String::from_utf8(download(&http, &format!("{url}.sha256")).await?).context("reading the checksum")?;
    let want = sums.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    let got = format!("{:x}", Sha256::digest(&archive));
    if want != got {
        bail!("the download doesn't match its checksum; nothing was changed");
    }

    // Unpacked next to this program, so the move into place is a rename on one disk: a
    // t3agent37 running from the old file (`serve`) keeps running.
    let dir = exe.parent().ok_or_else(|| anyhow!("{} has no folder", exe.display()))?;
    let work = dir.join(format!(".t3agent37-update-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| cant_write(dir, e))?;
    let result = install(&archive, &work, &exe);
    let _ = std::fs::remove_dir_all(&work);
    result?;

    let now = std::process::Command::new(&exe).arg("--version").output().context("running the new t3agent37")?;
    let now = String::from_utf8_lossy(&now.stdout).trim().to_owned();
    println!("Updated: {now} (was {VERSION}), at {}.", exe.display());
    if let Some(service) = restart_service() {
        println!("Restarted {service}, so it runs the new version.");
    }
    Ok(0)
}

fn install(archive: &[u8], work: &Path, exe: &Path) -> Result<()> {
    let tgz = work.join("t3agent37.tar.gz");
    std::fs::write(&tgz, archive)?;
    let status = std::process::Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(work).status().context("running tar")?;
    let new = work.join("t3agent37");
    if !status.success() || !new.is_file() {
        bail!("the download isn't a t3agent37 build; nothing was changed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&new, exe).map_err(|e| cant_write(exe.parent().unwrap_or(exe), e))
}

fn cant_write(dir: &Path, e: std::io::Error) -> anyhow::Error {
    anyhow!(
        "can't write to {} ({e}). Install it somewhere of yours instead: \
         curl -fsSL https://raw.githubusercontent.com/{REPO}/main/install.sh | sh",
        dir.display()
    )
}

/// Why something else updates the program at `exe`, if it does.
fn kept_elsewhere(exe: &Path) -> Option<String> {
    let path = exe.to_string_lossy();
    if path.contains(".app/Contents/MacOS/") {
        return Some(
            "this is the t3agent37 app's own program, which updates with the app: right-click its menu-bar icon → \
             Check for Updates…"
                .into(),
        );
    }
    if path.contains("/Cellar/") {
        return Some("Homebrew installed this t3agent37: `brew upgrade t3agent37` updates it.".into());
    }
    if path.contains("/target/debug/") || path.contains("/target/release/") || path.contains("/target/cli/") {
        return Some(format!("{} is a build from source: build it again to update it.", exe.display()));
    }
    None
}

/// The latest release's tag: where GitHub's `releases/latest` sends a browser (no API, so no
/// rate limit).
async fn latest() -> Result<String> {
    let http = reqwest::Client::builder()
        .user_agent(format!("t3agent37/{VERSION}"))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let res = http.get(format!("https://github.com/{REPO}/releases/latest")).send().await.context("asking GitHub for the latest release")?;
    let location = res.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok()).unwrap_or_default();
    let tag = location.rsplit_once("/tag/").map(|(_, t)| t.to_owned()).filter(|t| t.starts_with('v'));
    tag.ok_or_else(|| anyhow!("GitHub didn't say which release is the latest ({})", res.status()))
}

async fn download(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let res = http.get(url).send().await.with_context(|| format!("downloading {url}"))?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("no such release ({url} isn't there; https://github.com/{REPO}/releases lists them)");
    }
    if !res.status().is_success() {
        bail!("downloading {url}: {}", res.status());
    }
    Ok(res.bytes().await.with_context(|| format!("downloading {url}"))?.to_vec())
}

/// The release's file for this computer, as `install.sh` picks it.
fn asset() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => "t3agent37-macos.tar.gz",
        ("linux", "x86_64") => "t3agent37-linux-x86_64.tar.gz",
        ("linux", "aarch64") => "t3agent37-linux-arm64.tar.gz",
        (os, arch) => bail!("t3agent37 has no build for {arch} {os}"),
    })
}

/// `serve` as `serve --install` set it up, restarted if it runs, so it runs the new program.
fn restart_service() -> Option<String> {
    let quiet = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    if cfg!(target_os = "macos") {
        let plist: PathBuf = dirs::home_dir()?.join(format!("Library/LaunchAgents/{}.plist", super::serve::SERVICE));
        if !plist.exists() {
            return None;
        }
        let uid = super::serve::uid().ok()?;
        let job = format!("gui/{uid}/{}", super::serve::SERVICE);
        // Only one that runs: launchd's copy stops at once while the app runs, and stays stopped.
        let running = std::process::Command::new("launchctl")
            .args(["print", &job])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == "state = running"));
        (running && quiet("launchctl", &["kickstart", "-k", &job])).then(|| "t3agent37 serve (launchd)".to_owned())
    } else {
        let unit = super::serve::UNIT;
        (quiet("systemctl", &["--user", "is-active", "--quiet", unit]) && quiet("systemctl", &["--user", "restart", unit]))
            .then(|| "t3agent37 serve (systemd)".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_that_update_themselves() {
        let kept = |p: &str| kept_elsewhere(Path::new(p)).is_some();
        assert!(kept("/Applications/t3agent37.app/Contents/MacOS/t3agent37"));
        assert!(kept("/opt/homebrew/Cellar/t3agent37/0.1.10/bin/t3agent37"));
        assert!(kept("/home/linuxbrew/.linuxbrew/Cellar/t3agent37/0.1.10/bin/t3agent37"));
        assert!(kept("/Users/a/t3agent37/apps/desktop/target/debug/t3agent37"));
        assert!(!kept("/Users/a/.local/bin/t3agent37"));
        assert!(!kept("/usr/local/bin/t3agent37"));
    }
}
