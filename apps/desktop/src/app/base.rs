//! Bases: a machine set up once (Claude, Codex and GitHub signed in, the user's tools, any
//! projects with their dependencies) and kept stopped, so copies start from it as it is
//! (`spawn.rs`, when the user picks it over new machines: `Settings::spawn_from`). Agent37 copies a stopped machine without touching it. Stopped, it bills its
//! disk alone, and nothing wakes it: not its URLs, not a copy's first heartbeat, not a call
//! from T3 Code (`ensure_awake` refuses). `refresh` alone starts it: its projects are brought up
//! to date and set up again, this computer's agent files go over, then it stops again.

use super::remote;
use super::*;
use crate::agent37::shell_quote;
use crate::store::Base;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// The longest a refresh may take on the machine.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(45 * 60);

impl App {
    /// The machines that are bases.
    pub fn bases(&self) -> Vec<Arc<MachineRuntime>> {
        self.machines().into_iter().filter(|m| m.record().upkeep.base.is_some() && m.power() != Power::Missing).collect()
    }

    /// The base to copy from: `from`, or the only one there is.
    pub fn base_for(&self, from: Option<&str>) -> Result<Arc<MachineRuntime>> {
        if let Some(from) = from {
            let m = self.machine(from)?;
            if m.record().upkeep.base.is_none() {
                bail!("{} isn't a base: `t3agent37 base {} on` makes it one", m.record().name, m.record().name);
            }
            return Ok(m);
        }
        match self.bases().as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!(
                "there's no base yet: set up a machine once (Claude, GitHub and your tools on it), then \
                 `t3agent37 base <machine> on` keeps it stopped for copies to start from"
            ),
            many => bail!("say which base: {}", many.iter().map(|m| m.record().name).collect::<Vec<_>>().join(", ")),
        }
    }

    /// Makes a machine a base (`on`), stopping it, or a machine to work on again. One with work
    /// running stays as it is.
    pub async fn set_base(&self, machine: &str, on: bool) -> Result<String> {
        let m = self.machine(machine)?;
        let r = m.record();
        let name = r.name.clone();
        if !on {
            if r.upkeep.base.is_none() {
                return Ok(format!("{name} isn't a base."));
            }
            m.update_record(|r| r.upkeep.base = None);
            let mut settings = self.settings();
            let mut text = format!("{name} is a machine to work on again: it wakes when used, as the others do. Copies made from it keep what they had.");
            if settings.spawn_from.as_deref() == Some(name.as_str()) {
                settings.spawn_from = None;
                self.update_settings(settings);
                text.push_str(" Spawned machines start as new machines now.");
            }
            return Ok(text);
        }
        if r.upkeep.base.is_some() {
            return Ok(format!("{name} is a base already."));
        }
        if r.upkeep.spawned.is_some() {
            bail!("{name} is a copy made for a checkout here, and goes when its work is done: make a base of a machine you set up");
        }
        if m.power() == Power::Running {
            if m.watchdog_get("/status?fresh").await.is_ok_and(|s| s.busy) {
                bail!("work is running on {name}: make it a base once that's done");
            }
            // Its projects as they are now: stopped, it isn't read again until a refresh, and
            // the list should say what copies get (one cleaned out by hand kept its old seven).
            m.refresh_projects().await;
        }
        m.update_record(|r| {
            r.upkeep.base = Some(Base { since: chrono::Utc::now().timestamp(), refreshed_at: None });
            r.upkeep.delete_at = None;
        });
        if let Err(e) = self.stop_base(&m).await {
            m.update_record(|r| r.upkeep.base = None);
            return Err(e.context(format!("stopping {name}")));
        }
        let start = if self.settings().spawn_from.as_deref() == Some(name.as_str()) {
            "Spawned machines (`spawn`, `run`, on a copy) start as copies of it.".to_owned()
        } else {
            format!("`t3agent37 new-machines start {name}` makes spawned machines (`spawn`, `run`, on a copy) copies of it, or `--base {name}` one of them.")
        };
        Ok(format!(
            "{name} is a base: stopped, billing its disk alone, and nothing wakes it. {start} `t3agent37 base {name} refresh` \
             brings any projects on it up to date."
        ))
    }

    /// Stops it, under its wake lock so no wake runs meanwhile. One asleep in Agent37's cold
    /// storage has to be started first.
    async fn stop_base(&self, m: &MachineRuntime) -> Result<()> {
        let _waking = m.waking.lock().await;
        let a37 = &self.shared.a37;
        let id = m.sandbox_id();
        if let Err(e) = a37.stop(&id).await {
            tracing::info!("stopping {}: {e:#}; starting it first", m.id());
            a37.start(&id).await?;
            a37.stop(&id).await?;
        }
        let _ = m.refresh_power().await;
        Ok(())
    }

    /// Starts a base, brings its projects up to date (each checkout that has no changes of its
    /// own follows its branch's upstream) and sets them up again (dependencies from their
    /// lockfiles), copies this computer's agent files over, then stops it again.
    pub async fn refresh_base(&self, machine: &str) -> Result<String> {
        let m = self.machine(machine)?;
        let name = m.record().name;
        if m.record().upkeep.base.is_none() {
            bail!("{name} isn't a base: `t3agent37 base {name} on` makes it one");
        }
        m.base_open.store(true, Ordering::SeqCst);
        let done = async {
            m.ensure_awake().await?;
            m.refresh_projects().await;
            let roots: Vec<(String, String)> = m.projects().into_iter().filter_map(|p| Some((p.title, p.root?))).collect();
            let out = m.kept_awake(refresh_projects_on(&m, &roots)).await?;
            let synced = m.kept_awake(self.send_agent_files(&m)).await?;
            Ok::<_, anyhow::Error>((out, synced))
        }
        .await;
        m.base_open.store(false, Ordering::SeqCst);
        let stopped = self.stop_base(&m).await;
        let (out, synced) = done?;
        stopped.context("stopping it again")?;
        m.update_record(|r| {
            if let Some(b) = &mut r.upkeep.base {
                b.refreshed_at = Some(chrono::Utc::now().timestamp());
            }
        });
        let mut text = format!("{name} is up to date and stopped again.");
        if !out.trim().is_empty() {
            text.push('\n');
            text.push_str(out.trim());
        }
        if let Some(s) = synced {
            text.push('\n');
            text.push_str(&s);
        }
        Ok(text)
    }
}

/// Each project's checkout brought up to date and set up again, as a job; what each says.
async fn refresh_projects_on(m: &MachineRuntime, roots: &[(String, String)]) -> Result<String> {
    use base64::Engine;
    let setup = base64::engine::general_purpose::STANDARD.encode(crate::machine::projects::SETUP_SH);
    let mut script = format!("export PATH=\"$HOME/.local/bin:$PATH\"\necho {setup} | base64 -d > /root/.t3agent37/setup-project.sh\n");
    for (title, root) in roots {
        let t = shell_quote(title);
        script.push_str(&format!(
            "( cd {root} || exit 0\n\
               if [ -z \"$(git status --porcelain --untracked-files=no 2>/dev/null)\" ] && git fetch -q origin 2>/dev/null && git merge -q --ff-only '@{{u}}' >/dev/null 2>&1; then\n\
                 echo {t}\": up to date at $(git rev-parse --short HEAD)\"\n\
               else echo {t}\": left as it was (changes of its own, or no upstream)\"; fi\n\
               bash /root/.t3agent37/setup-project.sh >/tmp/t3agent37-setup.log 2>&1; c=$?\n\
               [ $c = 1 ] && echo {t}\": its setup failed: $(tail -n 1 /tmp/t3agent37-setup.log)\"\n\
               true )\n",
            root = crate::machine::projects::home_path(root),
        ));
    }
    let a37 = &m.shared.a37;
    let id = m.sandbox_id();
    let tag = format!("refresh-{}", chrono::Utc::now().timestamp());
    remote::start_job(a37, &id, &tag, &script).await?;
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (_, code) = remote::job_output(a37, &id, &tag, u64::MAX - 1).await?;
        if code.is_some() {
            return a37.exec_ok(&id, &format!("cat {}/{tag}.out", remote::JOBS), 60).await;
        }
        if started.elapsed() > REFRESH_TIMEOUT {
            let _ = remote::stop_job(a37, &id, &tag).await;
            bail!("the projects' setup took over {} minutes", REFRESH_TIMEOUT.as_secs() / 60);
        }
    }
}
