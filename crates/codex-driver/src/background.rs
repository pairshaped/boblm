//! Read-only inventory from this chat's dedicated app-server, including its loaded children.
use super::{Pending, Shared};
use boblm_protocol::{
    Event,
    jobs::{BackgroundJob, JobCompletion, JobSnapshot},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::AsyncWriteExt,
    process::ChildStdin,
    sync::{Mutex, mpsc, oneshot},
    time::timeout,
};

/// Oldest first, bounded by both age and count. Updated even for child threads.
#[derive(Default)]
pub(super) struct Completions(std::collections::VecDeque<(String, JobCompletion)>);

pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl Completions {
    fn prune(&mut self, at: i64) {
        let age = i64::try_from(boblm_tuning::BACKGROUND_COMPLETION_MAX_AGE.as_millis())
            .unwrap_or(i64::MAX);
        self.0
            .retain(|(_, completion)| at.saturating_sub(completion.reported_at) <= age);
    }

    pub(super) fn observe(&mut self, method: &str, params: &Value, at: i64) {
        self.prune(at);
        let item = &params["item"];
        if method != "item/completed"
            || !matches!(
                item["type"].as_str(),
                Some("commandExecution" | "CommandExecution")
            )
            || !matches!(
                item["source"].as_str(),
                Some(
                    "unified_exec_startup"
                        | "unified_exec_interaction"
                        | "unifiedExecStartup"
                        | "unifiedExecInteraction"
                )
            )
        {
            return;
        }
        let Some(thread) = params["threadId"].as_str() else {
            return;
        };
        let Some(process) = item
            .get("processId")
            .or_else(|| item.get("process_id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(status) = item["status"].as_str() else {
            return;
        };
        let code = item.get("exitCode").or_else(|| item.get("exit_code"));
        let exit_code = code.and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()));
        let id = format!("{thread}:{process}");
        self.0.retain(|(key, _)| key != &id);
        self.0.push_back((
            id,
            JobCompletion {
                exit_code,
                status: status.to_owned(),
                reported_at: at,
            },
        ));
        while self.0.len() > boblm_tuning::BACKGROUND_COMPLETION_LIMIT {
            self.0.pop_front();
        }
    }

    pub(super) fn get(&mut self, id: &str, at: i64) -> Option<JobCompletion> {
        self.prune(at);
        self.0
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value.clone())
    }
}

struct Inventory {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: i64,
}
impl Inventory {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ()> {
        self.next_id -= 1;
        let id = self.next_id;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);
        let result = timeout(boblm_tuning::BACKGROUND_REQUEST_TIMEOUT, async {
            let mut bytes = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))
                .map_err(|_| ())?;
            bytes.push(b'\n');
            {
                let mut stdin = self.stdin.lock().await;
                stdin.write_all(&bytes).await.map_err(|_| ())?;
                stdin.flush().await.map_err(|_| ())?;
            }
            rx.await.map_err(|_| ())?.map_err(|_| ())
        })
        .await
        .map_err(|_| ())
        .and_then(|v| v);
        self.pending.lock().expect("pending lock").remove(&id);
        result
    }

    async fn pages(&mut self, method: &str, mut params: Value) -> Result<Vec<Value>, ()> {
        let mut rows = Vec::new();
        let mut cursors = std::collections::HashSet::new();
        loop {
            let page = self.request(method, params.clone()).await?;
            rows.extend(page["data"].as_array().ok_or(())?.iter().cloned());
            if rows.len() > boblm_tuning::BACKGROUND_JOB_SCAN_LIMIT {
                return Err(());
            }
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(cursor)
                    if cursors.len() < boblm_tuning::BACKGROUND_JOB_SCAN_LIMIT
                        && cursors.insert(cursor.to_owned()) =>
                {
                    params["cursor"] = json!(cursor)
                }
                Some(_) => return Err(()),
                None => return Ok(rows),
            }
        }
    }

    async fn scan(&mut self, root: &str) -> Result<Vec<BackgroundJob>, ()> {
        let threads = self.pages("thread/loaded/list", json!({})).await?;
        let mut tasks = Vec::new();
        for thread in threads {
            let thread = thread.as_str().ok_or(())?;
            for job in self
                .pages(
                    "thread/backgroundTerminals/list",
                    json!({"threadId":thread}),
                )
                .await?
            {
                tasks.push(BackgroundJob {
                    id: format!("{thread}:{}", job["processId"].as_str().ok_or(())?),
                    command: job["command"].as_str().ok_or(())?.to_owned(),
                    agent_key: (thread != root).then(|| thread.to_owned()),
                });
                if tasks.len() > boblm_tuning::BACKGROUND_JOB_SCAN_LIMIT {
                    return Err(());
                }
            }
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        tasks.dedup_by(|a, b| a.id == b.id);
        Ok(tasks)
    }
}

pub(super) async fn monitor(
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    shared: Arc<Shared>,
    events: mpsc::WeakSender<Event>,
) {
    // Negative IDs do not overlap normal command RPCs or the usage request (-1).
    let mut inventory = Inventory {
        stdin,
        pending,
        next_id: -10,
    };
    let mut snapshot = JobSnapshot::default();
    let mut previous = None;
    let mut tick = tokio::time::interval(boblm_tuning::BACKGROUND_POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let root = shared.thread.lock().expect("thread lock").clone();
        let Some(root) = root else { continue };
        // Bound the entire scan as well as each request; partial inventories never clear jobs.
        match timeout(
            boblm_tuning::BACKGROUND_REQUEST_TIMEOUT,
            inventory.scan(&root),
        )
        .await
        {
            Ok(Ok(tasks)) => {
                snapshot = JobSnapshot {
                    tasks,
                    available: true,
                }
            }
            _ => snapshot.available = false,
        }
        inventory
            .pending
            .lock()
            .expect("pending lock")
            .retain(|id, _| *id >= -10);
        if previous.as_ref() != Some(&snapshot) {
            let Some(events) = events.upgrade() else {
                break;
            };
            if events
                .send(Event::BackgroundJobs {
                    snapshot: snapshot.clone(),
                })
                .await
                .is_err()
            {
                break;
            }
            previous = Some(snapshot.clone());
        } else if events.strong_count() == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(process: usize) -> Value {
        json!({"threadId":"thread", "item":{
            "type":"CommandExecution", "process_id":process.to_string(),
            "source":"unified_exec_startup", "status":"failed", "exit_code":"143"
        }})
    }

    #[test]
    fn background_completion_cache_parses_both_wire_shapes_and_missing_codes() {
        let mut cache = Completions::default();
        cache.observe("item/completed", &completion(1), 1000);
        assert_eq!(
            cache.get("thread:1", 1000),
            Some(JobCompletion {
                exit_code: Some(143),
                status: "failed".into(),
                reported_at: 1000,
            })
        );
        let mut camel = json!({"threadId":"child", "item":{
            "type":"commandExecution", "processId":"2", "source":"unifiedExecStartup",
            "status":"completed", "exitCode":0
        }});
        cache.observe("item/completed", &camel, 2000);
        assert_eq!(cache.get("child:2", 2000).unwrap().exit_code, Some(0));
        for code in [Value::Null, json!("not an integer")] {
            camel["item"]["exitCode"] = code;
            cache.observe("item/completed", &camel, 2000);
            assert_eq!(cache.get("child:2", 2000).unwrap().exit_code, None);
        }
        camel["item"].as_object_mut().unwrap().remove("exitCode");
        cache.observe("item/completed", &camel, 2000);
        assert_eq!(cache.get("child:2", 2000).unwrap().exit_code, None);
    }

    #[test]
    fn background_completion_cache_ignores_unrelated_or_malformed_notifications() {
        let mut cache = Completions::default();
        cache.observe("item/started", &completion(1), 1000);
        for field in ["source", "process_id", "status"] {
            let mut params = completion(1);
            params["item"].as_object_mut().unwrap().remove(field);
            cache.observe("item/completed", &params, 1000);
        }
        let mut params = completion(1);
        params["item"]["source"] = json!("foreground");
        cache.observe("item/completed", &params, 1000);
        params = completion(1);
        params["item"]["type"] = json!("agentMessage");
        cache.observe("item/completed", &params, 1000);
        assert!(cache.get("thread:1", 1000).is_none());
    }

    #[test]
    fn background_completion_cache_evicts_oldest_and_expired_records() {
        let mut cache = Completions::default();
        let limit = boblm_tuning::BACKGROUND_COMPLETION_LIMIT;
        for process in 0..=limit {
            cache.observe("item/completed", &completion(process), 1000);
        }
        assert_eq!(cache.0.len(), limit);
        assert!(cache.get("thread:0", 1000).is_none());
        assert!(cache.get(&format!("thread:{limit}"), 1000).is_some());
        // Updating a job moves it to the newest position without growing the cache.
        cache.observe("item/completed", &completion(1), 2000);
        cache.observe("item/completed", &completion(limit + 1), 2000);
        assert!(cache.get("thread:1", 2000).is_some());
        assert!(cache.get("thread:2", 2000).is_none());
        let age = i64::try_from(boblm_tuning::BACKGROUND_COMPLETION_MAX_AGE.as_millis()).unwrap();
        assert!(cache.get("thread:1", 2000 + age).is_some());
        assert!(cache.get("thread:1", 2001 + age).is_none());
        assert!(cache.0.is_empty());
        cache.observe("item/completed", &completion(1), 1000);
        cache.observe("item/completed", &completion(2), 1001 + age);
        assert_eq!(cache.0.len(), 1, "insertion also removes expired records");
    }
}
