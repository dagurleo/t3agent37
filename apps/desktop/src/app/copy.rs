//! Copies of a machine, through Agent37's fork: a new machine with the other's disk as it is
//! now (its projects and their files, its agents' and GitHub's sign-ins, whatever was installed),
//! at any size, with a T3 identity of its own. And `bench`: one command timed on copies at
//! several sizes, which are deleted after.
//!
//! A fork boots fresh from the copied disk, so the boot container starts T3 and the watchdog
//! there as the machine they were copied from: its environment id, its T3 sessions and T3
//! Connect's credentials, its watchdog's heartbeat URL. `FRESH_IDENTITY` stops them before
//! anything else and takes all that away, then provisioning starts T3 there as a new server.

use super::*;
use crate::machine::provision::SizeInfo;
use std::time::{Duration, Instant};

/// Run on the copy before provisioning. Stops what boot.sh started (boot.sh's own loops first,
/// so nothing comes back), then removes what made it the other machine: T3's environment id,
/// its secrets (session signing keys, T3 Connect's credentials), its sessions, pairings and
/// links to other servers, and t3agent37's own per-machine files (the watchdog's token and
/// config, GitHub issues, the start URL, links taken, awake time, the version that set it up).
/// Its projects, threads, settings and sign-ins stay.
const FRESH_IDENTITY: &str = r#"set -u
dir=/root/.t3agent37 u=/root/.t3/userdata
# Docker starts the boot container as it comes up: once it answers, the container is there to remove.
for _ in $(seq 1 90); do docker info >/dev/null 2>&1 && break; sleep 1; done
docker rm -f t3agent37-boot >/dev/null 2>&1 || true
stop_all() {
  for p in $(pgrep -f '^/bin/bash /root/.t3agent37/boot.sh'); do kill -KILL "$p" 2>/dev/null || true; done
  for n in t3 watchdog; do
    pid="$(cat "$dir/$n.pid" 2>/dev/null || true)"
    [ -n "$pid" ] && kill -KILL -- "-$pid" 2>/dev/null || true
  done
}
stop_all; sleep 1; stop_all
for _ in $(seq 1 40); do
  curl -sf -m 2 -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment || break
  sleep 0.5
done
if curl -sf -m 2 -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment; then
  echo "T3 kept running on the copy" >&2; exit 1
fi
rm -f "$u/environment-id" "$u/environment-id.recovery" "$u/anonymous-id" "$u/server-runtime.json"
rm -rf "$u/secrets"
[ -f "$u/statev2.sqlite" ] && node --no-warnings -e '
  const { DatabaseSync } = require("node:sqlite");
  const db = new DatabaseSync(process.argv[1]);
  for (const t of ["auth_sessions", "auth_pairing_links", "auth_connect_devices", "peer_environment_links"]) {
    try { db.exec(`DELETE FROM ${t}`); } catch {}
  }
  db.close();
' "$u/statev2.sqlite"
rm -f "$dir/config.json" "$dir/issues.json" "$dir/issues-state.json" "$dir/start.json" "$dir/start-threads.json" \
  "$dir/peers.json" "$dir/usage.json" "$dir/app-version" "$dir/t3.pid" "$dir/watchdog.pid"
echo fresh
"#;

/// How long Agent37 may take to finish a fork that outran its request.
const FORK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

impl App {
    /// Makes `name`, a copy of the machine `source` as its disk is now, at `size` (the source's
    /// when None), and adds it to T3 Code unless `pair` is false. `delete_after`: a throwaway
    /// one, deleted that many seconds from now (`lifetime.rs`).
    pub async fn copy_machine(
        self: &Arc<Self>,
        source: &str,
        name: &str,
        size: Option<&str>,
        delete_after: Option<u64>,
        pair: bool,
    ) -> Result<Arc<MachineRuntime>> {
        if !valid_name(name) {
            bail!("machine names are 1-32 lowercase letters, digits or dashes, starting with a letter or digit");
        }
        let from = self.machine(source)?;
        let from_record = from.record();
        let size = size.unwrap_or(&from_record.size).to_owned();
        let size_info = provision::size(&size)?;
        let port = self.reserve(name)?;
        self.set_creating_step(name, &format!("Copying {} on Agent37", from_record.name));
        // The copy's watchdog calls the original's heartbeat once as it boots, which wakes the
        // original if it slept: it's put back to sleep after.
        let was_asleep = self.shared.a37.get(&from_record.sandbox_id).await.ok().flatten().is_some_and(|i| i.status != "running");
        let result = async {
            let version = self.shared.t3.version().await?;
            let instance = self.fork(&from_record.sandbox_id, name, size_info).await?;
            let id = instance.id.clone();
            self.set_creating_step(name, "Giving it a T3 sign-in of its own");
            self.shared.a37.exec_ok(&id, FRESH_IDENTITY, 120).await.context("giving the copy an identity of its own")?;
            if was_asleep {
                let from = from.clone();
                tokio::spawn(async move { from.back_to_sleep().await });
            }
            let heartbeat = provision::heartbeat_url(&self.shared.a37, &id).await?;
            self.set_creating_step(name, "Starting T3 there");
            let secret = crate::proxy::auth::random_token();
            let provisioned = provision::run(&self.shared.a37, &id, name, &version, &secret, &heartbeat, true).await?;
            let mut record = provision::new_record(&id, name, &size, port, provisioned, secret, heartbeat, &version)?;
            // The same disk: the same GitHub sign-in.
            record.github_login = from_record.github_login.clone();
            record.github_checked = from_record.github_checked;
            record.github_reach = from_record.github_reach.clone();
            record.github_token_expires = from_record.github_token_expires.clone();
            record.upkeep.copied_from = Some(from_record.name.clone());
            record.upkeep.env = from_record.upkeep.env.clone();
            record.upkeep.delete_at = delete_after.map(|s| chrono::Utc::now().timestamp() + s as i64);
            let (day, _) = periods();
            self.spend
                .lock()
                .unwrap()
                .insert(record.sandbox_id.clone(), Spend { machine: record.id.clone(), day, today: 0.0, awake_secs_today: 0, month: 0.0 });
            let machine = self.runtime(record);
            if let Err(e) = machine.prime().await {
                tracing::warn!("priming {}: {e:#}", machine.id());
            }
            self.spawn(machine.clone());
            Ok::<_, anyhow::Error>(machine)
        }
        .await;
        let leftover = {
            let mut creating = self.creating.lock().unwrap();
            let sandbox = creating.iter().position(|c| c.name == name).and_then(|i| creating.remove(i).sandbox);
            sandbox.filter(|_| result.is_err())
        };
        self.leftover_sandboxes.lock().unwrap().extend(leftover.clone());
        self.shared.persist();
        self.shared.changed();
        if let Some(id) = leftover {
            self.delete_leftover(&id).await;
        }
        let machine = result?;
        if pair && std::env::var_os("T3AGENT37_NO_AUTOPAIR").is_none()
            && let Err(e) = self.pair(&machine)
        {
            self.shared.notify(format!("Couldn't open T3 Code for {name}"), format!("{e:#}; use Add to T3 Code"));
        }
        Ok(machine)
    }

    /// Agent37's fork of `sandbox`, once it runs. Recorded as being created as soon as Agent37
    /// gives it an id, so a failure from there deletes it.
    async fn fork(&self, sandbox: &str, name: &str, size: &SizeInfo) -> Result<crate::agent37::Instance> {
        let a37 = &self.shared.a37;
        let instance = a37.fork(sandbox, &provision::instance_name(name), size).await.context("copying the VM on Agent37")?;
        self.set_creating_sandbox(name, &instance.id);
        let started = Instant::now();
        let mut instance = instance;
        while instance.status != "running" {
            match instance.status.as_str() {
                "failed" | "deleting" | "deleted" => bail!("Agent37's copy ended {}", instance.status),
                _ if started.elapsed() > FORK_TIMEOUT => bail!("Agent37's copy didn't finish in {} minutes", FORK_TIMEOUT.as_secs() / 60),
                _ => {}
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            instance = a37.get(&instance.id).await?.ok_or_else(|| anyhow!("Agent37's copy is gone"))?;
        }
        Ok(instance)
    }

    /// Runs `command` (bash, in `project`'s folder on the source or its home) on a copy of
    /// `source` at each of `sizes` at once, times it there, then deletes the copies (unless
    /// `keep`). A copy's run stops waiting after `timeout_mins`. Returns the results, fastest
    /// first, as a table.
    pub async fn bench(
        self: &Arc<Self>,
        source: &str,
        sizes: &[String],
        project: Option<&str>,
        command: &str,
        keep: bool,
        timeout_mins: u32,
    ) -> Result<String> {
        let from = self.machine(source)?;
        let from_name = from.record().name;
        if command.trim().is_empty() {
            bail!("nothing to run");
        }
        if sizes.is_empty() {
            bail!("no sizes to run it on");
        }
        for s in sizes {
            provision::size(s)?;
        }
        let root = match project {
            Some(p) => Some(
                from.projects()
                    .into_iter()
                    .find(|x| x.id == p || x.title.eq_ignore_ascii_case(p))
                    .and_then(|x| x.root)
                    .ok_or_else(|| anyhow!("{from_name} has no project {p} with a folder"))?,
            ),
            None => None,
        };
        let names: Vec<String> = sizes.iter().map(|s| self.free_name(&format!("{}-{s}", from_name.chars().take(24).collect::<String>()))).collect();
        let runs = sizes.iter().zip(&names).map(|(size, name)| {
            let app = self.clone();
            let (size, name, root, command) = (size.clone(), name.clone(), root.clone(), command.to_owned());
            let source = from_name.clone();
            async move { app.bench_one(&source, &size, &name, root.as_deref(), &command, keep, timeout_mins).await }
        });
        let mut results = futures_util::future::join_all(runs).await;
        results.sort_by(|a, b| a.secs.unwrap_or(f64::MAX).total_cmp(&b.secs.unwrap_or(f64::MAX)));
        Ok(bench_table(&from_name, project, command, &results, keep))
    }

    /// `base`, or `base-2`, `base-3`… when that's taken.
    fn free_name(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.machines().iter().any(|m| m.record().name.eq_ignore_ascii_case(n))
                || self.creating.lock().unwrap().iter().any(|c| c.name.eq_ignore_ascii_case(n))
        };
        let base = base.trim_end_matches('-');
        (1..)
            .map(|i| if i == 1 { base.to_owned() } else { format!("{base}-{i}") })
            .find(|n| !taken(n))
            .expect("a free name")
    }

    #[allow(clippy::too_many_arguments)]
    async fn bench_one(
        self: &Arc<Self>,
        source: &str,
        size: &str,
        name: &str,
        root: Option<&str>,
        command: &str,
        keep: bool,
        timeout_mins: u32,
    ) -> BenchResult {
        let started = Instant::now();
        let hourly = provision::size(size).map_or(0.0, |s| s.hourly);
        let mut result = BenchResult { size: size.to_owned(), machine: name.to_owned(), secs: None, exit_code: None, tail: Vec::new(), error: None, cost: 0.0 };
        // A copy the app doesn't live to delete goes on its own, an hour after its run would.
        let lifetime = u64::from(timeout_mins) * 60 + 3600;
        let machine = match self.copy_machine(source, name, Some(size), Some(lifetime), false).await {
            Ok(m) => m,
            Err(e) => {
                result.error = Some(format!("{e:#}"));
                return result;
            }
        };
        match run_timed(&machine, root, command, timeout_mins).await {
            Ok((code, secs, tail)) => {
                result.exit_code = Some(code);
                result.secs = Some(secs);
                result.tail = tail;
            }
            Err(e) => result.error = Some(format!("{e:#}")),
        }
        if keep {
            let _ = machine.keep_awake(Some(0)).await;
        } else if let Err(e) = self.delete_machine(&machine, true).await {
            result.error.get_or_insert(format!("deleting it: {e:#}"));
        }
        result.cost = started.elapsed().as_secs_f64() * hourly / 3600.0;
        result
    }
}

/// One size's run in `bench`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchResult {
    pub size: String,
    pub machine: String,
    /// How long the command took on the copy, by its own clock.
    pub secs: Option<f64>,
    pub exit_code: Option<i32>,
    /// Its output's last lines.
    pub tail: Vec<String>,
    pub error: Option<String>,
    /// About what the copy cost, made to deleted.
    pub cost: f64,
}

/// Starts `command` on the machine in the background (so it may outlast an exec's 15 minutes),
/// kept awake meanwhile, and waits for it: its exit code, seconds and last lines of output.
async fn run_timed(m: &MachineRuntime, root: Option<&str>, command: &str, timeout_mins: u32) -> Result<(i32, f64, Vec<String>)> {
    use base64::Engine;
    m.keep_awake(Some(timeout_mins.saturating_add(10).min(1440))).await?;
    let cd = root.map(crate::machine::projects::home_path).unwrap_or_else(|| "\"$HOME\"".into());
    let script = format!("cd {cd} || exit 97\n{command}\n");
    let b64 = base64::engine::general_purpose::STANDARD.encode(script);
    let start = format!(
        "d=/root/.t3agent37; mkdir -p $d; rm -f $d/bench.done $d/bench.out\n\
         echo {b64} | base64 -d > $d/bench.sh\n\
         setsid nohup bash -c 's=$(date +%s.%N); bash -l /root/.t3agent37/bench.sh > /root/.t3agent37/bench.out 2>&1; c=$?; \
         echo \"$c $s $(date +%s.%N)\" > /root/.t3agent37/bench.done' >/dev/null 2>&1 < /dev/null &\necho started"
    );
    let a37 = &m.shared.a37;
    let id = m.sandbox_id();
    a37.exec_ok(&id, &start, 60).await?;
    let deadline = Instant::now() + Duration::from_secs(u64::from(timeout_mins) * 60);
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let done = a37.exec_ok(&id, "cat /root/.t3agent37/bench.done 2>/dev/null || true", 30).await.unwrap_or_default();
        let parts: Vec<&str> = done.split_whitespace().collect();
        if let [code, s, e] = parts[..] {
            let (code, s, e): (i32, f64, f64) = (code.parse()?, s.parse()?, e.parse()?);
            let tail = a37.exec_ok(&id, "tail -n 5 /root/.t3agent37/bench.out 2>/dev/null || true", 30).await.unwrap_or_default();
            return Ok((code, e - s, tail.lines().map(|l| l.chars().take(200).collect()).collect()));
        }
        if Instant::now() > deadline {
            bail!("still running after {timeout_mins} minutes");
        }
    }
}

/// "3m 12s", "58.4s".
fn took(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let s = secs.round() as u64;
        format!("{}m {}s", s / 60, s % 60)
    }
}

fn bench_table(source: &str, project: Option<&str>, command: &str, results: &[BenchResult], keep: bool) -> String {
    let place = project.map(|p| format!(" in {p}")).unwrap_or_default();
    let mut out = format!("`{}`{place} on copies of {source}:\n", command.trim());
    out.push_str(&format!("  {:8} {:16} {:>9}  {:>4}\n", "size", "", "time", "exit"));
    for r in results {
        let spec = provision::size(&r.size).map(|s| s.spec).unwrap_or_default();
        let time = r.secs.map(took).unwrap_or_else(|| "–".into());
        let exit = r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "–".into());
        out.push_str(&format!("  {:8} {:16} {:>9}  {:>4}\n", r.size, spec, time, exit));
    }
    let timed: Vec<&BenchResult> = results.iter().filter(|r| r.secs.is_some() && r.exit_code == Some(0)).collect();
    if let (Some(fast), Some(slow)) = (timed.first(), timed.last())
        && timed.len() > 1
        && fast.secs.unwrap_or(0.0) > 0.0
    {
        out.push_str(&format!("{} was {:.1}× as fast as {}. ", fast.size, slow.secs.unwrap_or(0.0) / fast.secs.unwrap_or(1.0), slow.size));
    }
    let cost: f64 = results.iter().map(|r| r.cost).sum();
    out.push_str(&if keep {
        format!("The copies cost about ${cost:.2} so far, and are kept: {}.\n", results.iter().map(|r| r.machine.as_str()).collect::<Vec<_>>().join(", "))
    } else {
        format!("The copies cost about ${cost:.2}, and are deleted.\n")
    });
    for r in results {
        if let Some(e) = &r.error {
            out.push_str(&format!("{}: {e}\n", r.size));
        }
        if !r.tail.is_empty() && (r.exit_code != Some(0) || results.len() == 1 || r.size == results[0].size) {
            out.push_str(&format!("--- {} (last lines)\n{}\n", r.size, r.tail.join("\n")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_results_read_as_a_table() {
        let r = |size: &str, secs: Option<f64>, code: Option<i32>| BenchResult {
            size: size.into(),
            machine: format!("skiff-{size}"),
            secs,
            exit_code: code,
            tail: vec!["done".into()],
            error: None,
            cost: 0.01,
        };
        let out = bench_table("skiff", Some("Tantalum"), "cargo build", &[r("huge", Some(30.0), Some(0)), r("small", Some(95.0), Some(0))], false);
        assert!(out.starts_with("`cargo build` in Tantalum on copies of skiff:\n"), "{out}");
        assert!(out.contains("huge     8 vCPU · 64 GB       30.0s     0"), "{out}");
        assert!(out.contains("1m 35s"), "{out}");
        assert!(out.contains("huge was 3.2× as fast as small. The copies cost about $0.02, and are deleted."), "{out}");
        // The fastest run's last lines; a failed run's too.
        assert!(out.contains("--- huge (last lines)\ndone") && !out.contains("--- small"), "{out}");
    }
}
