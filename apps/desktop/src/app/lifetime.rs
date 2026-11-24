//! Throwaway machines: made with a time to go (`create bench --delete-after 2h`), they're
//! deleted then, or once their agents finish if they still work at that time: never under
//! running work. Ten minutes before, the user hears about it and can keep the machine. It takes
//! t3agent37 running here: a machine whose time came while this computer was off sleeps
//! meanwhile, billing its disk, and goes at the next start.

use super::*;

/// The "deleted soon" notice comes this long before, for machines that live longer than twice
/// that.
const WARN_BEFORE: i64 = 10 * 60;
/// The longest a machine can be given: a year.
const MAX_SECS: u64 = 365 * 24 * 3600;

/// "90m", "2h", "1h30m", "1d", "45" (minutes): seconds. None for anything else, or nothing.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(minutes) = s.parse::<u64>() {
        return (minutes > 0).then_some(minutes * 60).filter(|&v| v <= MAX_SECS);
    }
    let mut total = 0u64;
    let mut number = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let unit = match c {
            'd' => 86_400,
            'h' => 3_600,
            'm' => 60,
            _ => return None,
        };
        total = total.checked_add(number.parse::<u64>().ok()?.checked_mul(unit)?)?;
        number.clear();
    }
    (number.is_empty() && total > 0 && total <= MAX_SECS).then_some(total)
}

/// "2h 10m", "45m", "3d 4h": how long until a time, for people.
pub fn span(secs: i64) -> String {
    let m = (secs.max(0) as u64).div_ceil(60);
    match m {
        0..60 => format!("{m}m"),
        60..1440 if m.is_multiple_of(60) => format!("{}h", m / 60),
        60..1440 => format!("{}h {}m", m / 60, m % 60),
        _ if (m / 60).is_multiple_of(24) => format!("{}d", m / 1440),
        _ => format!("{}d {}h", m / 1440, (m / 60) % 24),
    }
}

impl App {
    /// Deletes the throwaway machines whose time came, unless work still runs there, and warns
    /// about those whose time comes soon. Every minute.
    pub async fn check_lifetimes(&self) {
        let now = chrono::Utc::now().timestamp();
        for m in self.machines() {
            let r = m.record();
            let Some(at) = r.upkeep.delete_at else { continue };
            if m.power() == Power::Missing {
                continue;
            }
            if now >= at {
                match self.delete_machine(&m, false).await {
                    Ok(()) => self.shared.notify(format!("Deleted {}", r.name), "Its time was up, as you set when you made it."),
                    Err(e) => {
                        tracing::info!("{} stays until its work is done: {e:#}", r.name);
                        if self.overdue_noted.lock().unwrap().insert(r.id.clone()) {
                            self.shared.notify(
                                format!("{}'s time is up", r.name),
                                "Work still runs there, so it's deleted once that finishes. To keep it, open its details.",
                            );
                        }
                    }
                }
            } else if at - now <= WARN_BEFORE && !r.upkeep.delete_warned && at - r.created_at > 2 * WARN_BEFORE {
                m.update_record(|r| r.upkeep.delete_warned = true);
                self.shared.notify(
                    format!("{} is deleted in {}", r.name, span(at - now)),
                    format!("As you set when you made it. To keep it: its details, or `t3agent37 delete-after {} never`.", r.name),
                );
            }
        }
    }

    /// Gives a machine a time to go, `secs` from now, or none (`None`: it's kept).
    pub fn set_delete_after(&self, machine: &str, secs: Option<u64>) -> Result<String> {
        let m = self.machine(machine)?;
        if secs.is_some_and(|s| s == 0 || s > MAX_SECS) {
            bail!("a machine's time is from a minute to a year");
        }
        let at = secs.map(|s| chrono::Utc::now().timestamp() + s as i64);
        m.update_record(|r| {
            r.upkeep.delete_at = at;
            r.upkeep.delete_warned = false;
        });
        self.overdue_noted.lock().unwrap().remove(&m.id());
        let name = m.record().name;
        Ok(match secs {
            Some(s) => format!("{name} is deleted in {}, or once its agents finish if they still work then.", span(s as i64)),
            None => format!("{name} is kept: it has no time to go any more."),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90m"), Some(5400));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("1d"), Some(86_400));
        assert_eq!(parse_duration("45"), Some(2700));
        for bad in ["", "0", "2x", "h", "1.5h", "-2h", "400d"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
        assert_eq!(span(45 * 60), "45m");
        assert_eq!(span(2 * 3600), "2h");
        assert_eq!(span(2 * 3600 + 600), "2h 10m");
        assert_eq!(span(3 * 86_400 + 4 * 3600), "3d 4h");
        assert_eq!(span(-5), "0m");
    }
}
