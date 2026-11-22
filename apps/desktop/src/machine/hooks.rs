//! T3's webhook tasks (scheduled tasks a webhook triggers), at an address that wakes the machine.
//! T3 gives each one a URL through T3 Connect's relay, which can't wake a sleeping machine: the
//! relay holds deliveries until something else wakes it, and without T3 Connect there's no URL at
//! all. The machine's inbox (`issues.mjs`) takes deliveries at the task's own path on T3's server
//! (`/api/hooks/<task>/<token>`) through the inbox's public URL, so a delivery wakes the machine,
//! and passes them to T3, which checks the token, signature and filters as it does for its own.
//! The tasks are read from T3 while the machine runs and kept for while it sleeps.

use super::{provision, MachineRuntime, Power};
use crate::proxy::upstream;
use crate::store::HookTask;
use anyhow::Result;
use serde_json::{json, Value};
use std::time::Duration;

/// The webhook tasks in an item of T3's `scheduledTasks.subscribe`: `{tasks, timeZone}`.
pub fn hook_tasks(list: &Value) -> Vec<HookTask> {
    list["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["schedule"]["type"] == "webhook")
        .filter_map(|t| {
            Some(HookTask {
                id: t["id"].as_str()?.to_owned(),
                title: t["title"].as_str().unwrap_or("Webhook task").to_owned(),
                enabled: t["enabled"].as_bool().unwrap_or(true),
                project_id: t["projectId"].as_str().unwrap_or_default().to_owned(),
                path: t["webhook"]["path"].as_str().filter(|p| p.starts_with("/api/hooks/"))?.to_owned(),
                signed: t["webhook"]["hasSecret"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// Where a task's deliveries go to wake the machine: the inbox's public URL and the task's path.
pub fn waking_url(inbox_url: &str, task: &HookTask) -> String {
    format!("{}{}", inbox_url.trim_end_matches('/'), task.path)
}

impl MachineRuntime {
    /// Reads its webhook tasks from T3 when it runs (never wakes it), and keeps its inbox URL
    /// while it has any.
    pub async fn refresh_hooks(&self) {
        if self.power() != Power::Running {
            return;
        }
        let read = upstream::subscribe_until(
            &self.upstream_url(),
            &self.upstream_token(),
            "scheduledTasks.subscribe",
            json!({}),
            Duration::from_secs(20),
            |item| Some(item.clone()),
        )
        .await;
        let hooks = match read {
            Ok((Some(list), _)) => hook_tasks(&list),
            Ok((None, _)) => return,
            Err(e) => return tracing::debug!("reading {}'s webhook tasks: {e:#}", self.id()),
        };
        if hooks != self.record().hooks {
            self.update_record(|r| r.hooks = hooks.clone());
            self.shared.changed();
        }
        if let Err(e) = self.sync_inbox_url().await {
            tracing::warn!("{}'s inbox URL: {e:#}", self.id());
        }
    }

    /// The inbox's public URL exists while the machine has GitHub issues on, webhook tasks or
    /// its start URL, and goes with the last of them: nothing delivers there any more.
    pub(super) async fn sync_inbox_url(&self) -> Result<Option<String>> {
        let r = self.record();
        let wanted = !r.issues.is_empty() || !r.hooks.is_empty() || r.start.is_some();
        let (id, a37) = (self.sandbox_id(), &self.shared.a37);
        let url = match (wanted, r.inbox_url) {
            (true, Some(url)) => Some(url),
            (true, None) => {
                let url = provision::inbox_url(a37, &id).await?;
                self.update_record(|r| r.inbox_url = Some(url.clone()));
                Some(url)
            }
            (false, Some(_)) => {
                a37.delete_public_port(&id, provision::INBOX_PORT).await?;
                self.update_record(|r| r.inbox_url = None);
                None
            }
            (false, None) => None,
        };
        self.shared.changed();
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_tasks_and_their_waking_urls() {
        let list = json!({ "timeZone": "UTC", "tasks": [
            { "id": "scheduled-task:a", "title": "Triage Sentry alerts", "enabled": true, "projectId": "p1",
              "schedule": { "type": "webhook", "filters": [] },
              "webhook": { "path": "/api/hooks/scheduled-task%3Aa/tok", "url": "https://relay.t3.codes/v1/hooks/k/scheduled-task%3Aa/tok", "hasSecret": true } },
            { "id": "scheduled-task:b", "title": "Daily", "enabled": true, "projectId": "p1", "schedule": { "type": "fixed_time", "timeOfDay": "09:00" } },
            // No URL from T3 (T3 Connect off): its path still works through the inbox.
            { "id": "scheduled-task:c", "title": "Deploys", "enabled": false, "projectId": "p2",
              "schedule": { "type": "webhook" }, "webhook": { "path": "/api/hooks/scheduled-task%3Ac/tok2", "url": null, "hasSecret": false } },
        ] });
        let hooks = hook_tasks(&list);
        assert_eq!(hooks.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["Triage Sentry alerts", "Deploys"]);
        assert!(hooks[0].signed && !hooks[1].enabled);
        assert_eq!(waking_url("https://abc.agent37.app/", &hooks[0]), "https://abc.agent37.app/api/hooks/scheduled-task%3Aa/tok");
        assert!(hook_tasks(&json!({})).is_empty());
    }
}
