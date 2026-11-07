//! Read-only inventory from this chat's dedicated app-server, including its loaded children.
use super::{Pending, Shared};
use boblm_protocol::{
    Event,
    jobs::{BackgroundJob, JobSnapshot},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::AsyncWriteExt,
    process::ChildStdin,
    sync::{Mutex, mpsc, oneshot},
    time::timeout,
};

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
