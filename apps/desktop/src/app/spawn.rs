//! Machines for a checkout on this computer: new ones, signed in with what's saved here
//! (`newmachine.rs`), or copies of a base, as the user chose. `spawn` makes one for a thread
//! here: the checkout's files go over as they are (`gitsync.rs`), the machine is linked into T3
//! Code for agents here, and the thread hands it work with T3's own `delegate_task`, which
//! reports back to it. `collect` brings its work back into the checkout and deletes it; one
//! left with nothing running for half an hour has its work brought back to a branch, and goes.
//! `run` keeps one machine per checkout for its commands (builds, tests): the files go over
//! before each, and the output comes back as it's written.

use super::newmachine::Start;
use super::{gitsync, remote};
use super::*;
use crate::agent37::shell_quote;
use crate::machine::projects::{home_path, MachineProject};
use crate::store::Spawned;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

/// A spawned copy with nothing running there this long has its work brought back and goes.
pub const IDLE_MINUTES: i64 = 30;
/// Hours a copy lives at most unless asked otherwise: the safety net if this app isn't there to
/// see it idle.
const MAX_HOURS: u64 = 24;
/// Where a copy keeps the files it got and the work it hands back.
const SENT: &str = "/root/.t3agent37/spawn/sent.bundle";
const RESULT: &str = "/root/.t3agent37/spawn/result.bundle";
/// The most of this computer's agent files sent along.
const MAX_AGENT_FILES: usize = 5 * 1024 * 1024;

/// A spawn for an agent, kept after the MCP call stopped waiting for it, for `spawn_status`.
#[derive(Clone, Debug)]
pub struct SpawnJob {
    pub checkout: String,
    pub started: Instant,
    pub machine: Option<String>,
    pub result: Option<std::result::Result<String, String>>,
}

/// `tantalum` for a checkout of github.com/you/Tantalum: a machine name's start.
fn short_name(snap: &gitsync::Snapshot) -> String {
    let from = snap.repo_key.as_deref().and_then(|k| k.rsplit('/').next()).map(str::to_owned).unwrap_or_else(|| {
        snap.top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    let mut name: String = from.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    name = name.trim_matches('-').chars().take(20).collect::<String>().trim_end_matches('-').to_owned();
    if name.is_empty() { "work".into() } else { name }
}

/// Waits up to half a minute for a copy to look idle. Its agent's turn just ended when its work
/// is collected, and the machine still looked busy a few seconds after (2026-11-25): the delete
/// was refused, and the agent's retry brought the work back a second time.
async fn settle(m: &MachineRuntime) {
    for _ in 0..6 {
        if m.power() != Power::Running || m.watchdog_get("/status?fresh").await.is_ok_and(|w| !w.busy) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

impl App {
    /// Makes a machine (new, or a copy of a base: `from`, else as Settings says) with the checkout
    /// `checkout` is in, as it is now, for a thread here to hand work. Returns what to do next.
    pub async fn spawn_machine(self: &Arc<Self>, checkout: &str, from: Option<&str>, size: Option<&str>, max_hours: Option<u64>) -> Result<String> {
        let job = {
            let mut jobs = self.spawn_jobs.lock().unwrap();
            jobs.retain(|j| j.started.elapsed() < Duration::from_secs(6 * 3600));
            jobs.push(SpawnJob { checkout: checkout.to_owned(), started: Instant::now(), machine: None, result: None });
            jobs.len() - 1
        };
        let result = self.spawn_inner(checkout, from, size, max_hours, job).await;
        if let Some(j) = self.spawn_jobs.lock().unwrap().get_mut(job) {
            j.result = Some(result.as_ref().map(Clone::clone).map_err(|e| format!("{e:#}")));
        }
        result
    }

    async fn spawn_inner(self: &Arc<Self>, checkout: &str, from: Option<&str>, size: Option<&str>, max_hours: Option<u64>, job: usize) -> Result<String> {
        let start = self.start_for(from)?;
        let snap = gitsync::snapshot(Path::new(checkout)).await?;
        let hours = max_hours.unwrap_or(MAX_HOURS).clamp(1, 7 * 24);
        let (m, notes) = self.make_machine(&snap, &start, size, hours, false).await?;
        let name = m.record().name;
        if let Some(j) = self.spawn_jobs.lock().unwrap().get_mut(job) {
            j.machine = Some(name.clone());
        }
        let set_up = async {
            let project = self.project_on(&m, &snap, &start).await?;
            let root = project.root.clone().ok_or_else(|| anyhow!("{} has no folder on {name}", project.title))?;
            let branch = self.give_checkout(&m, &snap, &root).await?;
            self.env_from_here(&m, &project).await;
            let synced = self.send_agent_files(&m).await?;
            m.update_record(|r| {
                r.upkeep.spawned = Some(Spawned {
                    checkout: snap.top.to_string_lossy().into_owned(),
                    root: root.clone(),
                    project_id: project.id.clone(),
                    branch: branch.clone(),
                    head: snap.head.clone(),
                    sent: snap.commit.clone(),
                    base: start.base_name(),
                    runner: false,
                    idle_since: None,
                    collected: None,
                })
            });
            let linked = if self.shared.t3.links_enabled() {
                m.ensure_linked().await.context("linking it into T3 Code for agents here")?;
                true
            } else {
                false
            };
            Ok::<_, anyhow::Error>((project, branch, synced, linked))
        }
        .await;
        let (project, branch, synced, linked) = match set_up {
            Ok(v) => v,
            Err(e) => {
                if let Err(d) = self.delete_machine(&m, true).await {
                    tracing::warn!("deleting {name} after a failed spawn: {d:#}");
                }
                return Err(e);
            }
        };
        let r = m.record();
        let what = if snap.changed { "with its changes not committed" } else { "with nothing uncommitted" };
        let mut text = format!(
            "{name} is ready: {} ({}) with {} as it is here, on the branch {branch} at {} {what}, in the project {} there.",
            Start::describe(&start.base_name()),
            r.size,
            snap.top.display(),
            snap.short_head(),
            project.title,
        );
        for s in [Some(notes), synced].into_iter().flatten().filter(|s| !s.is_empty()) {
            text.push(' ');
            text.push_str(&s);
        }
        if linked {
            text.push_str(&format!(
                "\nHand it the work with T3's delegate_task, target {{\"environmentId\": \"{}\", \"projectId\": \"{}\"}}: it runs in a thread \
                 there and reports back to yours. Then collect_machine {{\"machine\": \"{name}\"}} brings its changes into {} and deletes it.",
                r.environment_id,
                project.id,
                snap.top.display()
            ));
        } else {
            text.push_str(
                "\nT3 Code's linked environments are off, so agents here can't hand it work: turn them on in T3 Code's settings, then spawn again.",
            );
        }
        text.push_str(&format!(
            " Left with nothing running for {IDLE_MINUTES} minutes, its work goes to the branch {} there and it's deleted; it goes after {hours} hour{} whatever happens.",
            gitsync::result_branch(&snap.top, &name).await,
            if hours == 1 { "" } else { "s" }
        ));
        Ok(text)
    }

    /// Waits a while for the spawns under way, then says how each stands.
    pub async fn spawn_status(&self, wait: Duration) -> String {
        let until = Instant::now() + wait;
        while Instant::now() < until && self.spawn_jobs.lock().unwrap().iter().any(|j| j.result.is_none()) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let jobs = self.spawn_jobs.lock().unwrap().clone();
        if jobs.is_empty() {
            return "No spawns in the last hours.".into();
        }
        jobs.iter()
            .rev()
            .map(|j| {
                let who = j.machine.clone().unwrap_or_else(|| "a copy".into());
                match &j.result {
                    None => format!("{who} for {}: still being made ({} s so far).", j.checkout, j.started.elapsed().as_secs()),
                    Some(Ok(text)) => text.clone(),
                    Some(Err(e)) => format!("The spawn for {} failed: {e}", j.checkout),
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// A machine for the checkout, named after it, which goes after `hours` whatever happens: a
    /// copy of a base, or a new machine signed in with what's saved here, the user's setup script
    /// run there. With what to tell about how it was set up.
    async fn make_machine(self: &Arc<Self>, snap: &gitsync::Snapshot, start: &Start, size: Option<&str>, hours: u64, runner: bool) -> Result<(Arc<MachineRuntime>, String)> {
        let name = self.free_name(&format!("{}-{}", short_name(snap), if runner { "run" } else { "tmp" }));
        match start {
            Start::Base(base) => Ok((self.copy_machine(&base.record().name, &name, size, Some(hours * 3600), false).await?, String::new())),
            Start::New => {
                // Commands need no agent; work handed over does.
                if !runner {
                    self.check_agent_saved()?;
                }
                let m = self.create_bare(&name, size.unwrap_or("default"), Some(hours * 3600)).await?;
                let mut notes = self.sign_in_new(&m, snap.repo_key.as_deref()).await.text();
                if let Some(Ok(t) | Err(t)) = self.run_machine_setup(&m).await {
                    notes = format!("{notes} {t}").trim().to_owned();
                }
                Ok((m, notes))
            }
        }
    }

    /// The checkout's project on `m`: as the base had it, or cloned from GitHub now as T3 Code's
    /// "Add to another machine" does.
    async fn project_on(self: &Arc<Self>, m: &MachineRuntime, snap: &gitsync::Snapshot, start: &Start) -> Result<MachineProject> {
        let key = snap.repo_key.clone().ok_or_else(|| anyhow!("{} has no `origin` to find its project by", snap.top.display()))?;
        let find = || {
            m.projects()
                .into_iter()
                .find(|p| p.status.is_none() && p.root.is_some() && p.repo_key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(&key)))
        };
        m.refresh_projects().await;
        if let Some(p) = find() {
            return Ok(p);
        }
        let local = self.local_projects().await?.into_iter().find(|p| p.repo_key.eq_ignore_ascii_case(&key)).ok_or_else(|| match start {
            Start::Base(b) => anyhow!("{key} is neither on {} nor a project in T3 Code here: add it to {0}", b.record().name),
            Start::New => anyhow!("{key} isn't a project in T3 Code here: add {} to T3 Code first", snap.top.display()),
        })?;
        let added = self.add_project(m, &local.id).await;
        if let (Err(e), Start::New) = (&added, start)
            && self.sign_ins_saved().github.is_empty()
        {
            bail!("{e:#} (no GitHub sign-in is saved for new machines: `t3agent37 new-machines github --from-gh` saves this computer's)");
        }
        added?;
        m.refresh_projects().await;
        find().ok_or_else(|| anyhow!("{} didn't arrive on {}", local.title, m.record().name))
    }

    /// Sends the checkout's files to `m` and checks them out in its repository at `root`, on
    /// the checkout's branch (or `t3agent37/<machine>` for a detached HEAD). Returns the branch.
    async fn give_checkout(&self, m: &MachineRuntime, snap: &gitsync::Snapshot, root: &str) -> Result<String> {
        let name = m.record().name;
        let refname = format!("refs/t3agent37/{name}/sent");
        let bundle = gitsync::bundle(&snap.top, &refname, &snap.commit).await?;
        let id = m.sandbox_id();
        m.kept_awake(remote::upload(&self.shared.a37, &id, SENT, &bundle)).await.context("sending the checkout's files")?;
        let branch = snap.branch.clone().unwrap_or_else(|| format!("t3agent37/{name}"));
        let script = gitsync::apply_script(&home_path(root), SENT, &refname, &branch, &snap.head);
        self.shared.a37.exec_ok(&id, &script, 300).await.with_context(|| format!("checking the checkout's files out on {name}"))?;
        Ok(branch)
    }

    /// This computer's env files for the project, over what the base had.
    async fn env_from_here(&self, m: &MachineRuntime, project: &MachineProject) {
        if let Err(e) = self.project_env(&m.record().name, &project.id, EnvAction::Copy).await {
            tracing::info!("env files for {} on {}: {e:#}", project.title, m.id());
        }
    }

    /// This computer's agent files that Settings lists (`base_sync`), over the machine's.
    /// Says what went, if anything did.
    pub(super) async fn send_agent_files(&self, m: &MachineRuntime) -> Result<Option<String>> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
        let present: Vec<String> = self.settings().base_sync.into_iter().filter(|p| home.join(p).exists()).collect();
        if present.is_empty() {
            return Ok(None);
        }
        let out = tokio::process::Command::new("tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["--no-xattrs", "-czf", "-", "-C"])
            .arg(&home)
            .args(&present)
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .context("packing the agent files")?;
        if !out.status.success() {
            bail!("packing the agent files: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        if out.stdout.len() > MAX_AGENT_FILES {
            bail!("the agent files to send come to {} MB, more than {} MB", out.stdout.len() / 1_000_000, MAX_AGENT_FILES / 1_000_000);
        }
        let id = m.sandbox_id();
        let file = "/root/.t3agent37/agent-files.tgz";
        remote::upload(&self.shared.a37, &id, file, &out.stdout).await?;
        self.shared.a37.exec_ok(&id, &format!("tar -xzf {file} -C /root --no-same-owner && rm -f {file}"), 60).await.context("unpacking the agent files")?;
        Ok(Some(format!("This computer's {} went over.", present.iter().map(|p| format!("~/{p}")).collect::<Vec<_>>().join(", "))))
    }

    /// Brings a spawned copy's work back into the checkout it was made for: a branch
    /// `t3agent37/<machine>` there, and (with `apply`) its changes made to the checkout's files
    /// when they fit them as they are now. Then deletes it, unless `keep`. Refused while an
    /// agent works there.
    pub async fn collect(&self, machine: &str, apply: bool, keep: bool) -> Result<String> {
        let m = self.machine(machine)?;
        let r = m.record();
        let name = r.name.clone();
        let s = r.upkeep.spawned.clone().ok_or_else(|| anyhow!("{name} wasn't spawned for a checkout here: nothing to bring back"))?;
        if s.runner {
            bail!("{name} runs commands for {}: nothing comes back from it", s.checkout);
        }
        if !self.collecting.lock().unwrap().insert(r.id.clone()) {
            bail!("{name}'s work is being brought back already");
        }
        let result = self.collect_inner(&m, &s, apply, keep).await;
        self.collecting.lock().unwrap().remove(&r.id);
        result
    }

    async fn collect_inner(&self, m: &MachineRuntime, s: &Spawned, apply: bool, keep: bool) -> Result<String> {
        let name = m.record().name;
        if m.power() == Power::Running && m.watchdog_get("/status?fresh").await.is_ok_and(|w| w.working_threads > 0) {
            bail!("an agent still works on {name}: bring its work back once it's done (delegate_task reports back then)");
        }
        let top = Path::new(&s.checkout);
        if !top.exists() {
            bail!("{} isn't there any more to bring {name}'s work back to", s.checkout);
        }
        m.ensure_awake().await?;
        let a37 = &self.shared.a37;
        let id = m.sandbox_id();
        let said = m
            .kept_awake(a37.exec_ok(&id, &gitsync::collect_script(&home_path(&s.root), RESULT, &s.head, &name), 300))
            .await
            .with_context(|| format!("taking {name}'s work"))?;
        let mut text;
        let branch;
        if said.trim().ends_with("same") {
            text = format!("{name} changed nothing in {}.", s.checkout);
            branch = String::new();
        } else {
            let bytes = m.kept_awake(remote::download(a37, &id, RESULT, gitsync::MAX_BUNDLE)).await.with_context(|| format!("bringing {name}'s work back"))?;
            branch = gitsync::take_result(top, &name, &bytes).await?;
            let result_ref = format!("refs/t3agent37/{name}/result");
            let stat = gitsync::changes(top, &s.sent, &result_ref).await.unwrap_or_default();
            text = format!("{name}'s work is on the branch {branch} in {}:\n{stat}\n", s.checkout);
            if apply {
                match gitsync::apply(top, &s.sent, &result_ref).await {
                    Ok(true) => text.push_str(&format!("Its changes are made to the files in {} too, not committed.", s.checkout)),
                    Ok(false) => text.push_str(&format!("Its changes are in the files in {} already.", s.checkout)),
                    Err(e) => text.push_str(&format!(
                        "Its changes don't fit the files in {} as they are now, so they're only on the branch ({e:#}). \
                         `git diff {} {branch}` there shows them.",
                        s.checkout,
                        &s.sent[..s.sent.len().min(12)]
                    )),
                }
            }
        }
        m.update_record(|r| {
            if let Some(sp) = &mut r.upkeep.spawned {
                sp.collected = Some(branch.clone());
            }
        });
        if keep {
            text.push_str(&format!(" {name} is kept; it goes after {IDLE_MINUTES} minutes with nothing running."));
        } else {
            settle(m).await;
            match self.delete_machine(m, false).await {
                Ok(()) => text.push_str(&format!(" {name} is deleted.")),
                Err(e) => text.push_str(&format!(
                    " {name} stays for now ({e:#}); it goes by itself once nothing has run there for {IDLE_MINUTES} minutes, \
                     or collect_machine again then: its work is on that branch already."
                )),
            }
        }
        Ok(text.trim().to_owned())
    }

    /// Every minute: spawned copies with nothing running there for `IDLE_MINUTES` have their
    /// work brought back to a branch, and go.
    pub async fn check_spawns(&self) {
        let now = chrono::Utc::now().timestamp();
        for m in self.machines() {
            let r = m.record();
            let Some(s) = r.upkeep.spawned.clone() else { continue };
            if m.power() == Power::Missing || self.collecting.lock().unwrap().contains(&r.id) {
                continue;
            }
            let busy = m.power() == Power::Running && m.watchdog_status().is_none_or(|w| w.busy || w.keep_awake_secs > 0 || w.keep_awake_forever);
            let idle_since = match (busy, s.idle_since) {
                (true, None) => continue,
                (true, Some(_)) => None,
                (false, Some(t)) => Some(t),
                (false, None) => Some(now),
            };
            if idle_since != s.idle_since {
                m.update_record(|r| {
                    if let Some(sp) = &mut r.upkeep.spawned {
                        sp.idle_since = idle_since;
                    }
                });
            }
            let Some(since) = idle_since else { continue };
            if now - since < IDLE_MINUTES * 60 {
                continue;
            }
            if s.runner || s.collected.is_some() {
                match self.delete_machine(&m, false).await {
                    Ok(()) => tracing::info!("deleted {}: nothing ran there for {IDLE_MINUTES} minutes", r.name),
                    Err(e) => tracing::info!("{} stays: {e:#}", r.name),
                }
                continue;
            }
            match self.collect(&r.name, false, false).await {
                Ok(text) => self.shared.notify(format!("{}'s work came back", r.name), text),
                Err(e) => {
                    // Tried again in another half hour; it goes at its time otherwise.
                    m.update_record(|r| {
                        if let Some(sp) = &mut r.upkeep.spawned {
                            sp.idle_since = Some(now);
                        }
                    });
                    self.shared.notify(format!("Couldn't bring {}'s work back", r.name), format!("{e:#}"));
                }
            }
        }
    }

    /// Starts `command` for the checkout `checkout` is in, on its machine for commands (made the
    /// first time, new or from a base), with the checkout's files as they are now: in the same folder
    /// below the checkout's top, after any run before it is done. Answers the machine and the
    /// job to read with `run_output`.
    pub async fn run_start(self: &Arc<Self>, checkout: &str, command: &str, from: Option<&str>, size: Option<&str>, timeout_mins: u32) -> Result<Value> {
        if command.trim().is_empty() {
            bail!("nothing to run");
        }
        let snap = gitsync::snapshot(Path::new(checkout)).await?;
        let top = snap.top.to_string_lossy().into_owned();
        let existing = self.machines().into_iter().find(|m| {
            m.power() != Power::Missing && m.record().upkeep.spawned.as_ref().is_some_and(|s| s.runner && s.checkout == top)
        });
        let made = existing.is_none();
        let m = match existing {
            Some(m) => m,
            None => self.make_runner(&snap, from, size).await?,
        };
        let name = m.record().name;
        let s = m.record().upkeep.spawned.clone().ok_or_else(|| anyhow!("{name} lost its checkout"))?;
        let a37 = &self.shared.a37;
        // Kept up for the run, with time to spare: `run_output` lets it go once runs are done.
        m.keep_awake(Some(timeout_mins.saturating_add(10).min(24 * 60))).await?;
        let id = m.sandbox_id();
        let refname = format!("refs/t3agent37/{name}/sent");
        let bundle = gitsync::bundle(&snap.top, &refname, &snap.commit).await?;
        let tag = format!("run-{}-{:04x}", chrono::Utc::now().timestamp(), rand::random::<u16>());
        let bundle_path = format!("{}/{tag}.bundle", remote::JOBS);
        m.kept_awake(remote::upload(a37, &id, &bundle_path, &bundle)).await.context("sending the checkout's files")?;
        let branch = snap.branch.clone().unwrap_or_else(|| format!("t3agent37/{name}"));
        let apply = gitsync::apply_script(&home_path(&s.root), &bundle_path, &refname, &branch, &snap.head);
        let below = if snap.below.is_empty() { String::new() } else { format!("/{}", snap.below) };
        let script = format!(
            "exec 8>/root/.t3agent37/run.lock\n\
             flock -n 8 || {{ echo 't3agent37: waiting for the run before this one'; flock 8; }}\n\
             top=$(bash -c {apply} | tail -n 1)\n\
             rm -f {bundle}\n\
             [ -n \"$top\" ] || {{ echo \"t3agent37: couldn't check the checkout's files out on {name}\"; exit 97; }}\n\
             cd \"$top\"{below} || exit 97\n\
             timeout -k 10 {secs} bash -lc {command} 8>&-\n",
            apply = shell_quote(&apply),
            bundle = shell_quote(&bundle_path),
            below = shell_quote_below(&below),
            secs = u64::from(timeout_mins) * 60,
            command = shell_quote(command),
        );
        remote::start_job(a37, &id, &tag, &script).await?;
        Ok(json!({ "machine": name, "job": tag, "made": made, "base": s.base, "from": Start::describe(&s.base), "size": m.record().size }))
    }

    /// A machine to run the checkout's commands on, kept between runs.
    async fn make_runner(self: &Arc<Self>, snap: &gitsync::Snapshot, from: Option<&str>, size: Option<&str>) -> Result<Arc<MachineRuntime>> {
        let start = self.start_for(from)?;
        let (m, _) = self.make_machine(snap, &start, size, MAX_HOURS, true).await?;
        let set_up = async {
            let project = self.project_on(&m, snap, &start).await?;
            let root = project.root.clone().ok_or_else(|| anyhow!("{} has no folder there", project.title))?;
            self.env_from_here(&m, &project).await;
            self.send_agent_files(&m).await?;
            m.update_record(|r| {
                r.upkeep.spawned = Some(Spawned {
                    checkout: snap.top.to_string_lossy().into_owned(),
                    root,
                    project_id: project.id.clone(),
                    branch: String::new(),
                    head: snap.head.clone(),
                    sent: snap.commit.clone(),
                    base: start.base_name(),
                    runner: true,
                    idle_since: None,
                    collected: None,
                })
            });
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = set_up {
            let _ = self.delete_machine(&m, true).await;
            return Err(e);
        }
        Ok(m)
    }

    /// What run `job` on `machine` wrote from byte `offset`, base64, and its exit code once it's
    /// done. Lets the machine go once no run is left there.
    pub async fn run_output(&self, machine: &str, job: &str, offset: u64) -> Result<Value> {
        use base64::Engine;
        let m = self.machine(machine)?;
        let (bytes, code) = remote::job_output(&self.shared.a37, &m.sandbox_id(), job, offset).await?;
        let more = bytes.len() >= remote::OUT_PIECE;
        if code.is_some() && !more {
            let left = self
                .shared
                .a37
                .exec_ok(&m.sandbox_id(), &format!("ls {}/run-*.pid 2>/dev/null | wc -l", remote::JOBS), 30)
                .await
                .unwrap_or_default();
            if left.trim() == "0" {
                let _ = m.keep_awake(Some(0)).await;
            }
        }
        Ok(json!({ "output": base64::engine::general_purpose::STANDARD.encode(&bytes), "exitCode": code, "more": more }))
    }

    /// Stops run `job` on `machine`.
    pub async fn run_stop(&self, machine: &str, job: &str) -> Result<String> {
        let m = self.machine(machine)?;
        remote::stop_job(&self.shared.a37, &m.sandbox_id(), job).await?;
        Ok(format!("Stopped it on {}.", m.record().name))
    }

    /// Sets the agent files this computer gives spawned machines (`add`) or stops giving one.
    pub fn base_sync(&self, path: Option<&str>, add: bool) -> Result<String> {
        let mut settings = self.settings();
        if let Some(path) = path {
            let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
            let p = path.trim().trim_end_matches('/');
            let rel = p.strip_prefix("~/").map(str::to_owned).or_else(|| Path::new(p).strip_prefix(&home).ok().map(|r| r.to_string_lossy().into_owned()));
            let rel = rel.or_else(|| (!p.starts_with('/') && !p.is_empty()).then(|| p.to_owned())).ok_or_else(|| anyhow!("{path} isn't in your home folder"))?;
            if rel.split('/').any(|c| c == ".." || c.is_empty()) {
                bail!("{path}: a file or folder in your home folder, such as ~/.claude/skills");
            }
            if add {
                if !home.join(&rel).exists() {
                    bail!("there's no ~/{rel}");
                }
                if !settings.base_sync.contains(&rel) {
                    settings.base_sync.push(rel);
                }
            } else {
                settings.base_sync.retain(|p| *p != rel);
            }
            self.update_settings(settings.clone());
        }
        Ok(if settings.base_sync.is_empty() {
            "Spawned machines get none of this computer's agent files (`t3agent37 base sync ~/.claude/skills` adds one).".into()
        } else {
            format!(
                "Spawned machines (new ones and copies of bases) get these from this computer as they are when made, and bases at each refresh: {}.",
                settings.base_sync.iter().map(|p| format!("~/{p}")).collect::<Vec<_>>().join(", ")
            )
        })
    }
}

/// `"/sub dir"` as a shell word after `"$top"`, or nothing.
fn shell_quote_below(below: &str) -> String {
    if below.is_empty() { String::new() } else { shell_quote(below) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_named_after_the_repo() {
        let snap = |key: Option<&str>, top: &str| gitsync::Snapshot {
            top: top.into(),
            below: String::new(),
            head: "abc".into(),
            branch: None,
            commit: "def".into(),
            changed: false,
            repo_key: key.map(str::to_owned),
        };
        assert_eq!(short_name(&snap(Some("github.com/you/Tantalum"), "/x")), "tantalum");
        assert_eq!(short_name(&snap(None, "/Users/you/My App_2")), "my-app-2");
        assert_eq!(short_name(&snap(Some("github.com/you/a-very-long-repository-name-indeed"), "/x")), "a-very-long-reposito");
        assert_eq!(short_name(&snap(None, "/")), "work");
    }
}
