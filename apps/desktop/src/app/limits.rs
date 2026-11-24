//! Spending limits: one a month per machine, and one on all of them together (Settings).
//!
//! Agent37 meters by the minute but settles hourly, so a machine's month goes by the larger of
//! what Agent37 billed so far and its awake time this month at its hourly rate (its watchdog's
//! count, which is live while it's up). Past 80% of a limit the user hears about it, once a month.
//! At the limit the machine is stopped, not put to sleep: a stopped machine stays off when its
//! URLs are called too (a GitHub delivery, the start URL, another machine, the wake link), and a
//! sleeping one doesn't. Nothing here wakes it again until the limit is raised or removed, or
//! the month is over.

use super::*;
use crate::store::{Machine, Pause};

/// The share of a limit that's warned about.
const WARN_AT: f64 = 0.8;

/// The current UTC month, `2026-11`: Agent37's months.
pub fn this_month() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

/// `r`'s awake time this month at its size's rate, from its watchdog's count.
fn awake_estimate(r: &Machine, month: &str) -> f64 {
    let Some(u) = &r.usage else { return 0.0 };
    if u.day.get(..7) != Some(month) {
        return 0.0;
    }
    let hourly = provision::size(&r.size).map_or(0.0, |s| s.hourly);
    u.month_secs() as f64 * hourly / 3600.0
}

/// What was spent: "$3.20", "$0.04", and under a cent as it is.
fn dollars(v: f64) -> String {
    if v > 0.0 && v < 0.01 { crate::store::dollars(v) } else { format!("${v:.2}") }
}

impl App {
    /// What the machine spent this month as best known now: what Agent37 billed, or its awake
    /// time at its rate when that's more (the last hour isn't billed yet).
    pub fn month_spend(&self, m: &MachineRuntime) -> f64 {
        self.usage(m).month.max(awake_estimate(&m.record(), &this_month()))
    }

    /// All machines together this month, deleted ones included, as `month_spend` counts.
    pub fn total_month_spend(&self) -> f64 {
        let month = this_month();
        let unbilled: f64 = self.machines().iter().map(|m| (awake_estimate(&m.record(), &month) - self.usage(m).month).max(0.0)).sum();
        self.costs().month + unbilled
    }

    /// Stops what reached a limit, warns about what nears one, and lets wake again what no
    /// longer is over one. Every minute, and after a limit changes.
    pub async fn check_limits(&self) {
        let month = this_month();
        let total_limit = self.settings().month_limit_usd;
        let total = self.total_month_spend();
        for m in self.machines() {
            let r = m.record();
            let Some(p) = r.upkeep.paused else { continue };
            let lifted = if p.month != month {
                true
            } else if p.total {
                total_limit.is_none_or(|l| total < l)
            } else {
                r.upkeep.month_limit_usd.is_none_or(|l| self.month_spend(&m) < l)
            };
            if lifted {
                m.update_record(|r| r.upkeep.paused = None);
                if p.month != month {
                    self.shared.notify(format!("{} can wake again", r.name), "A new month began, so the limit it reached is behind it.");
                }
            }
        }
        for m in self.machines() {
            let r = m.record();
            if r.upkeep.paused.is_some() || m.power() == Power::Missing {
                continue;
            }
            let Some(limit) = r.upkeep.month_limit_usd else { continue };
            let spent = self.month_spend(&m);
            if spent >= limit {
                self.pause(&m, Pause { month: month.clone(), limit_usd: limit, total: false }).await;
                self.shared.notify(
                    format!("{} stopped at its {} limit", r.name, crate::store::dollars(limit)),
                    format!("It spent {} this month, so it stays off, work and all, until you raise its limit or the month is over.", dollars(spent)),
                );
            } else if spent >= WARN_AT * limit && r.upkeep.limit_warned.as_deref() != Some(&month) {
                m.update_record(|r| r.upkeep.limit_warned = Some(month.clone()));
                self.shared.notify(
                    format!("{} is near its {} limit", r.name, crate::store::dollars(limit)),
                    format!("It spent {} this month. At {}, it's stopped until you raise the limit.", dollars(spent), dollars(limit)),
                );
            }
        }
        let Some(limit) = total_limit else { return };
        if total >= limit {
            let mut stopped = Vec::new();
            for m in self.machines() {
                if m.record().upkeep.paused.is_none() && m.power() != Power::Missing {
                    self.pause(&m, Pause { month: month.clone(), limit_usd: limit, total: true }).await;
                    stopped.push(m.record().name);
                }
            }
            if !stopped.is_empty() {
                self.shared.notify(
                    format!("Machines stopped at your {} limit", crate::store::dollars(limit)),
                    format!(
                        "They spent {} this month together, so {} stay off until you raise the limit in Settings or the month is over.",
                        dollars(total),
                        stopped.join(", ")
                    ),
                );
            }
        } else if total >= WARN_AT * limit {
            let warned = {
                let mut warned = self.total_limit_warned.lock().unwrap();
                warned.replace(month.clone()).as_deref() == Some(&month)
            };
            if !warned {
                self.shared.persist();
                self.shared.notify(
                    format!("Your machines are near your {} limit", crate::store::dollars(limit)),
                    format!("They spent {} this month together. At {}, every machine is stopped until you raise it.", dollars(total), dollars(limit)),
                );
            }
        }
    }

    /// Records `p` and stops the machine. Under its wake lock, so no wake runs meanwhile, and
    /// one that comes later sees the pause and refuses.
    async fn pause(&self, m: &MachineRuntime, p: Pause) {
        let _waking = m.waking.lock().await;
        m.update_record(|r| r.upkeep.paused = Some(p));
        if let Err(e) = self.shared.a37.stop(&m.sandbox_id()).await {
            // Asleep in Agent37's cold storage, which can't be stopped without waking it first:
            // it stays asleep, and this app doesn't wake it.
            tracing::warn!("stopping {} at its limit: {e:#}", m.id());
        }
        let _ = m.refresh_power().await;
    }

    /// Sets the monthly limit of one machine, or of all of them together (`machine` None), in
    /// dollars; None removes it. A machine stopped at a limit that no longer holds can wake again;
    /// one already over the new limit is stopped.
    pub async fn set_limit(&self, machine: Option<&str>, usd: Option<f64>) -> Result<String> {
        if usd.is_some_and(|v| !(v.is_finite() && v > 0.0 && v <= 100_000.0)) {
            bail!("a limit is a number of dollars above 0");
        }
        let month_word = crate::store::month_names(&this_month()).0;
        let text = match machine {
            Some(machine) => {
                let m = self.machine(machine)?;
                let name = m.record().name;
                m.update_record(|r| {
                    r.upkeep.month_limit_usd = usd;
                    r.upkeep.limit_warned = None;
                });
                let spent = self.month_spend(&m);
                match usd {
                    Some(v) => format!("{name} may spend {} a month; it spent {} in {month_word}.", crate::store::dollars(v), dollars(spent)),
                    None => format!("{name} has no limit of its own now; it spent {} in {month_word}.", dollars(spent)),
                }
            }
            None => {
                let mut settings = self.settings();
                settings.month_limit_usd = usd;
                self.update_settings(settings);
                *self.total_limit_warned.lock().unwrap() = None;
                let spent = self.total_month_spend();
                match usd {
                    Some(v) => format!("Your machines may spend {} a month together; they spent {} in {month_word}.", crate::store::dollars(v), dollars(spent)),
                    None => format!("Your machines have no limit together now; they spent {} in {month_word}.", dollars(spent)),
                }
            }
        };
        let paused_before: HashSet<String> = self.machines().iter().filter(|m| m.record().upkeep.paused.is_some()).map(|m| m.id()).collect();
        self.check_limits().await;
        let mut notes = Vec::new();
        for m in self.machines() {
            let r = m.record();
            match (paused_before.contains(&r.id), r.upkeep.paused.is_some()) {
                (true, false) => notes.push(format!("{} can wake again.", r.name)),
                (false, true) => notes.push(format!("{} is over it already, so it's stopped.", r.name)),
                (true, true) if machine.is_none_or(|m| m.eq_ignore_ascii_case(&r.name) || m == r.id) => {
                    notes.push(format!("{} stays stopped: it's still over a limit.", r.name))
                }
                _ => {}
            }
        }
        Ok(std::iter::once(text).chain(notes).collect::<Vec<_>>().join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_time_counts_in_its_month_only() {
        let mut r: Machine = serde_json::from_value(serde_json::json!({
            "id": "m", "sandboxId": "m", "name": "m", "size": "large", "upstreamUrl": "", "upstreamToken": "",
            "watchdogUrl": "", "watchdogSecret": "", "localPort": 1, "t3Version": "", "provisionRevision": "",
            "environmentId": "", "clientTokens": [], "linkTokens": [], "createdAt": 0
        }))
        .unwrap();
        assert_eq!(awake_estimate(&r, "2026-11"), 0.0);
        r.usage = Some(crate::store::ProjectUsage { day: "2026-11-20".into(), today: Default::default(), month: [("a".into(), 7200), ("".into(), 3600)].into() });
        let large = provision::size("large").unwrap().hourly;
        assert!((awake_estimate(&r, "2026-11") - 3.0 * large).abs() < 1e-9);
        assert_eq!(awake_estimate(&r, "2026-12"), 0.0);
    }

    #[test]
    fn a_pause_says_what_lets_it_wake() {
        let own = Pause { month: "2026-11".into(), limit_usd: 10.0, total: false };
        assert_eq!(own.why("skiff"), "skiff reached its $10 limit for November and is stopped: raise its limit, or it can wake in December");
        let all = Pause { month: "2026-12".into(), limit_usd: 50.0, total: true };
        assert!(all.why("skiff").contains("$50 limit for December together") && all.why("skiff").ends_with("wake in January"));
        let tiny = Pause { month: "2026-11".into(), limit_usd: 0.0005, total: false };
        assert!(tiny.why("probe").contains("its $0.0005 limit"), "{}", tiny.why("probe"));
        assert_eq!((crate::store::dollars(10.0), crate::store::dollars(12.5), dollars(0.00075), dollars(0.0)), ("$10".into(), "$12.50".into(), "$0.00075".into(), "$0.00".into()));
    }
}
