//! A single follow-up after a quiet root turn started by an automated wake (ADR 0046).
use crate::app::App;
use boblm_runner::Status;
use boblm_store::Store;
use sha2::{Digest, Sha256};

fn message_id(session: &str, result_seq: i64) -> String {
    let digest = Sha256::digest(format!("boblm:idle-nudge:{session}:{result_seq}"));
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn automated_start(store: &Store, session: &str, result_seq: i64) -> boblm_store::Result<bool> {
    let reader = store.reader()?;
    let Some(input) = reader.idle_nudge_input(session, result_seq)? else {
        return Ok(false);
    };
    if let Some(sender) = input.sender.as_deref() {
        if sender == session {
            // The runner authenticates itself for inventory completions and native subagent wakes.
            return Ok(input.content.as_deref().is_some_and(|text| {
                text.starts_with("Background command is no longer listed as running:")
                    || text.starts_with("Subagent ")
            }));
        }
        if let Some(uuid) = input.uuid {
            return Ok(reader
                .idle_nudge_report_sources(sender, session)?
                .into_iter()
                .any(|seq| crate::subagent_report::report_message_id(sender, seq) == uuid));
        }
    }
    let payload: serde_json::Value =
        serde_json::from_str(&input.payload).map_err(std::io::Error::other)?;
    // Unknown origins never trigger a nudge. The preserved Claude subtype is explicit evidence.
    Ok(payload["type"] == "system" && payload["subtype"] == "task_notification")
}

fn quiet(status: &Status) -> bool {
    !status.turn_active
        && !status.stopping
        && !status.stopped
        && !status.archived
        && status.queued == 0
        && status.dispatching == 0
        && status.uncertain == 0
        && status.rejected == 0
        && !status.queue_held
}

// An idle child can remain held while waiting for its parent. Only work that can
// still produce a report blocks the root's opportunity.
fn child_quiet(status: &Status) -> bool {
    !status.turn_active && !status.stopping && status.queued == 0 && status.dispatching == 0
}

impl App {
    async fn idle_nudge_status_quiet(
        &self,
        session: &str,
        status: Status,
        child: bool,
        now: i64,
    ) -> Result<bool, String> {
        if !(if child {
            child_quiet(&status)
        } else {
            quiet(&status)
        }) {
            return Ok(false);
        }
        let session = session.to_owned();
        self.blocking("check the idle nudge's job ages", move |store| {
            store.reader()?.idle_nudge_jobs_quiet(
                &session,
                status.running,
                status.jobs.as_ref(),
                now,
            )
        })
        .await
    }

    pub(crate) async fn run_idle_nudges(&self, now: i64) -> Result<(), String> {
        let cutoff = now.saturating_sub(boblm_tuning::IDLE_NUDGE_DELAY.as_millis() as i64);
        let due = self
            .blocking("read due idle nudges", move |store| {
                store.reader()?.due_idle_nudges(cutoff)
            })
            .await?;
        for nudge in due {
            let session = nudge.session.clone();
            let seq = nudge.result_seq;
            let id = message_id(&session, seq);
            let handle = self.handle(&session).await.ok();
            let mut allowed = false;
            if nudge.state == "pending" {
                let root_quiet = if let Some(handle) = &handle {
                    match handle.status().await {
                        Ok(status) => {
                            self.idle_nudge_status_quiet(&session, status, false, now)
                                .await?
                        }
                        Err(_) => false,
                    }
                } else {
                    false
                };
                if root_quiet {
                    let root = session.clone();
                    allowed = self
                        .blocking("read the idle turn's origin", move |store| {
                            automated_start(&store, &root, seq)
                        })
                        .await?;
                    if allowed {
                        let root = session.clone();
                        let children = self
                            .blocking("read open subagents", move |store| {
                                Ok(store
                                    .reader()?
                                    .sessions()?
                                    .into_iter()
                                    .filter(|s| {
                                        !s.archived
                                            && (s.parent_session_id.as_deref() == Some(&root)
                                                || s.root_session_id.as_deref() == Some(&root))
                                    })
                                    .map(|s| s.id)
                                    .collect::<Vec<_>>())
                            })
                            .await?;
                        for child in children {
                            let child_quiet = match self.handle(&child).await {
                                Ok(handle) => match handle.status().await {
                                    Ok(status) => {
                                        self.idle_nudge_status_quiet(&child, status, true, now)
                                            .await?
                                    }
                                    Err(_) => false,
                                },
                                Err(_) => false,
                            };
                            if !child_quiet {
                                allowed = false;
                                break;
                            }
                        }
                    }
                }
            }
            let root = session.clone();
            let uuid = id.clone();
            let accepted = self
                .blocking("settle the idle nudge", move |store| {
                    store.prepare_idle_nudge(&root, seq, &uuid, allowed, now)
                })
                .await?;
            if accepted && let Some(handle) = handle {
                // The store already accepted the automated message. This identical receipt
                // refreshes the ordinary queue without clearing a Stop or releasing a hold.
                handle.send_automated(&id, boblm_tuning::IDLE_NUDGE_TEXT).await.map_err(|error| format!("The idle nudge could not reach its runner: {error}. Its accepted message will keep the same ID."))?;
                self.blocking("record the idle nudge's handoff", move |store| {
                    store.finish_idle_nudge(&session, seq, &id)
                })
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use boblm_protocol::{
        Event, LifecycleState, Priority,
        jobs::{BackgroundJob, JobSnapshot},
    };
    use boblm_runner::{Driver, DriverFactory, SessionHandle};
    use boblm_store::{PeerSpawn, Session};
    use std::{
        io,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::mpsc;

    struct Fake {
        events: mpsc::UnboundedReceiver<Event>,
        inputs: mpsc::UnboundedSender<(String, String)>,
    }
    #[async_trait]
    impl Driver for Fake {
        async fn send(&mut self, uuid: &str, content: &str, _: Option<Priority>) -> io::Result<()> {
            self.inputs
                .send((uuid.into(), content.into()))
                .map_err(io::Error::other)
        }
        async fn next_event(&mut self) -> Option<Event> {
            self.events.recv().await
        }
        async fn configure(&mut self, _: &str, _: Option<&str>) -> io::Result<()> {
            Ok(())
        }
        async fn abort(&mut self) -> io::Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct Factory(Mutex<std::collections::HashMap<String, Fake>>);
    #[async_trait]
    impl DriverFactory for Factory {
        async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
            Ok(Box::new(
                self.0
                    .lock()
                    .unwrap()
                    .remove(&session.id)
                    .expect("fake child"),
            ))
        }
    }
    struct Harness {
        _temp: tempfile::TempDir,
        store: Store,
        app: Arc<App>,
        handle: SessionHandle,
        factory: Arc<Factory>,
        events: mpsc::UnboundedSender<Event>,
        inputs: mpsc::UnboundedReceiver<(String, String)>,
    }
    impl Harness {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("db.sqlite")).unwrap();
            store
                .create_session("root", temp.path().to_str().unwrap(), 1)
                .unwrap();
            let (events, rx) = mpsc::unbounded_channel();
            let (tx, inputs) = mpsc::unbounded_channel();
            let factory = Arc::new(Factory(Mutex::new(std::collections::HashMap::from([(
                "root".into(),
                Fake {
                    events: rx,
                    inputs: tx,
                },
            )]))));
            let app = App::open(
                store.clone(),
                factory.clone(),
                "idle-nudge-test-token".into(),
            )
            .await
            .unwrap();
            let handle = app.handle("root").await.unwrap();
            Self {
                _temp: temp,
                store,
                app,
                handle,
                factory,
                events,
                inputs,
            }
        }
        async fn spawn_child(&self) -> String {
            let project = self
                .store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .into_iter()
                .find(|s| s.id == "root")
                .unwrap()
                .project_id
                .unwrap();
            let child = uuid::Uuid::new_v4().to_string();
            let created_at = crate::schedule::now_ms();
            let request = || PeerSpawn {
                request_id: child.clone(),
                caller: "root".into(),
                arguments: "{}".into(),
                project_id: project.clone(),
                agent: "claude".into(),
                model: "sonnet".into(),
                effort: "high".into(),
                title: "Child".into(),
                content: "work".into(),
                created_at,
                parent_session_id: Some("root".into()),
                root_session_id: Some("root".into()),
                role: Some("implementer".into()),
                resolved: None,
                working_directory: None,
                account_key: None,
            };
            self.store.spawn_peer(request()).unwrap();
            self.app.spawn_peer(request()).await.unwrap();
            child
        }
        async fn input(&mut self) -> (String, String) {
            tokio::time::timeout(Duration::from_secs(3), self.inputs.recv())
                .await
                .unwrap()
                .unwrap()
        }
        async fn finish(&self, uuid: &str) -> i64 {
            self.events
                .send(Event::CommandLifecycle {
                    command_uuid: uuid.into(),
                    state: LifecycleState::Started,
                })
                .unwrap();
            self.events
                .send(Event::BackgroundJobs {
                    snapshot: JobSnapshot {
                        available: true,
                        tasks: vec![],
                    },
                })
                .unwrap();
            self.events
                .send(Event::Result {
                    subtype: Some("success".into()),
                    is_error: Some(false),
                })
                .unwrap();
            self.settled().await
        }
        async fn settled(&self) -> i64 {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let status = self.handle.status().await.unwrap();
                    let due = self
                        .store
                        .reader()
                        .unwrap()
                        .due_idle_nudges(i64::MAX)
                        .unwrap();
                    if !status.turn_active
                        && status.dispatching == 0
                        && status.queued == 0
                        && let Some(last) = due.last()
                    {
                        // The status barrier follows event application; yield once so its
                        // corresponding summary can commit before the scheduler's read.
                        tokio::task::yield_now().await;
                        break last.result_seq;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap()
        }
        fn due_time(&self, seq: i64) -> i64 {
            self.store
                .reader()
                .unwrap()
                .due_idle_nudges(i64::MAX)
                .unwrap()
                .into_iter()
                .find(|n| n.result_seq == seq)
                .unwrap()
                .created_at
                + boblm_tuning::IDLE_NUDGE_DELAY.as_millis() as i64
        }
        async fn job_turn(&mut self) -> i64 {
            self.handle
                .send(&uuid::Uuid::new_v4().to_string(), "work")
                .await
                .unwrap();
            let (owner, _) = self.input().await;
            self.events
                .send(Event::CommandLifecycle {
                    command_uuid: owner,
                    state: LifecycleState::Started,
                })
                .unwrap();
            self.events
                .send(Event::BackgroundJobs {
                    snapshot: JobSnapshot {
                        available: true,
                        tasks: vec![BackgroundJob {
                            id: "job".into(),
                            command: "check".into(),
                            agent_key: None,
                        }],
                    },
                })
                .unwrap();
            self.events
                .send(Event::Result {
                    subtype: None,
                    is_error: Some(false),
                })
                .unwrap();
            self.events
                .send(Event::BackgroundJobs {
                    snapshot: JobSnapshot {
                        available: true,
                        tasks: vec![],
                    },
                })
                .unwrap();
            let (notice, text) = self.input().await;
            assert!(text.starts_with("Background command is no longer listed"));
            self.finish(&notice).await
        }
        fn nudge_count(&self) -> usize {
            self.store
                .reader()
                .unwrap()
                .messages("root", 0, i64::MAX)
                .unwrap()
                .into_iter()
                .filter(|m| m.content == boblm_tuning::IDLE_NUDGE_TEXT)
                .count()
        }
    }

    fn job_snapshot(ids: &[&str]) -> JobSnapshot {
        JobSnapshot {
            available: true,
            tasks: ids
                .iter()
                .map(|id| BackgroundJob {
                    id: (*id).into(),
                    command: (*id).into(),
                    agent_key: None,
                })
                .collect(),
        }
    }

    async fn publish_jobs(
        handle: &SessionHandle,
        events: &mpsc::UnboundedSender<Event>,
        snapshot: JobSnapshot,
    ) {
        events
            .send(Event::BackgroundJobs {
                snapshot: snapshot.clone(),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if handle.status().await.unwrap().jobs.as_ref() == Some(&snapshot) {
                    tokio::task::yield_now().await;
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn idle_nudge_late_tool_result_before_job_notice_fires() {
        let mut h = Harness::new().await;
        // The fake driver uses Codex's translated assistant/user event format.
        let conn = rusqlite::Connection::open(h.store.path()).unwrap();
        conn.execute("UPDATE sessions SET agent = 'codex' WHERE id = 'root'", [])
            .unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        h.handle.send(&owner, "work").await.unwrap();
        h.input().await;
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: owner,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.events
            .send(Event::Assistant {
                message: serde_json::json!({"content":[{"type":"tool_use","id":"exec-old","name":"exec_command","input":{}}]}),
            })
            .unwrap();
        h.events
            .send(Event::BackgroundJobs {
                snapshot: job_snapshot(&["job"]),
            })
            .unwrap();
        h.events
            .send(Event::Result {
                subtype: None,
                is_error: Some(false),
            })
            .unwrap();
        h.events
            .send(Event::User {
                message: serde_json::json!({"content":[{"type":"tool_result","tool_use_id":"exec-old","is_error":true,"content":"\u{1b}]0;\u{7}"}]}),
            })
            .unwrap();
        h.events
            .send(Event::BackgroundJobs {
                snapshot: job_snapshot(&[]),
            })
            .unwrap();
        let (notice, text) = h.input().await;
        assert!(text.starts_with("Background command is no longer listed as running:"));
        let seq = h.finish(&notice).await;
        assert!(automated_start(&h.store, "root", seq).unwrap());
        h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
        assert_eq!(h.nudge_count(), 1);
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_long_lived_job_does_not_block() {
        let mut h = Harness::new().await;
        let seq = h.job_turn().await;
        let now = h.due_time(seq);
        let snapshot = job_snapshot(&["preview"]);
        h.store
            .record_event(
                "root",
                "agent_event",
                serde_json::to_value(Event::BackgroundJobs {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
                now - boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 - 1,
            )
            .unwrap();
        publish_jobs(&h.handle, &h.events, snapshot.clone()).await;
        // Providers keep publishing unchanged inventories without a fresh actor summary.
        h.store
            .record_event(
                "root",
                "agent_event",
                serde_json::to_value(Event::BackgroundJobs { snapshot }).unwrap(),
                now,
            )
            .unwrap();
        h.app.run_idle_nudges(now).await.unwrap();
        assert_eq!(h.nudge_count(), 1);
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_recent_jobs_and_mixed_inventory_block_at_both_checks() {
        for mixed in [false, true] {
            for acceptance_only in [false, true] {
                let mut h = Harness::new().await;
                let seq = h.job_turn().await;
                let now = h.due_time(seq);
                if mixed {
                    h.store
                        .record_event(
                            "root",
                            "agent_event",
                            serde_json::to_value(Event::BackgroundJobs {
                                snapshot: job_snapshot(&["preview"]),
                            })
                            .unwrap(),
                            now - boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 - 1,
                        )
                        .unwrap();
                }
                let snapshot = job_snapshot(if mixed {
                    &["preview", "build"]
                } else {
                    &["build"]
                });
                publish_jobs(&h.handle, &h.events, snapshot).await;
                if acceptance_only {
                    assert!(
                        !h.store
                            .prepare_idle_nudge("root", seq, &message_id("root", seq), true, now)
                            .unwrap()
                    );
                } else {
                    h.app.run_idle_nudges(now).await.unwrap();
                }
                assert_eq!(
                    h.nudge_count(),
                    0,
                    "mixed: {mixed}, acceptance only: {acceptance_only}"
                );
            }
        }
    }

    #[tokio::test]
    async fn idle_nudge_job_update_after_status_read_blocks_acceptance() {
        for unavailable in [false, true] {
            let mut h = Harness::new().await;
            let seq = h.job_turn().await;
            let now = h.due_time(seq);
            let mut snapshot = job_snapshot(&["new-build"]);
            snapshot.available = !unavailable;
            // The event has committed but its actor summary has not. Acceptance must
            // reject the earlier quiet status even when it was read successfully.
            h.store
                .record_event(
                    "root",
                    "agent_event",
                    serde_json::to_value(Event::BackgroundJobs { snapshot }).unwrap(),
                    now,
                )
                .unwrap();
            assert!(
                !h.store
                    .prepare_idle_nudge("root", seq, &message_id("root", seq), true, now)
                    .unwrap()
            );
            assert_eq!(h.nudge_count(), 0);
        }
    }

    #[tokio::test]
    async fn idle_nudge_restart_preserves_recent_job_window() {
        let mut h = Harness::new().await;
        let seq = h.job_turn().await;
        let now = h.due_time(seq);
        let snapshot = job_snapshot(&["preview"]);
        // Pin first observation independently of wall time and later duplicate snapshots.
        h.store
            .record_event(
                "root",
                "agent_event",
                serde_json::to_value(Event::BackgroundJobs {
                    snapshot: snapshot.clone(),
                })
                .unwrap(),
                now,
            )
            .unwrap();
        publish_jobs(&h.handle, &h.events, snapshot.clone()).await;
        let reopened = Store::open(h.store.path()).unwrap();
        let window = boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64;
        for check_at in [now, now + window - 1, now + window] {
            assert!(
                !reopened
                    .reader()
                    .unwrap()
                    .idle_nudge_jobs_quiet("root", true, Some(&snapshot), check_at)
                    .unwrap()
            );
        }
        // Reopen web-owned storage while keeping the existing fake runner alive.
        assert!(
            reopened
                .reader()
                .unwrap()
                .idle_nudge_jobs_quiet("root", true, Some(&snapshot), now + window + 1)
                .unwrap()
        );
        assert!(
            reopened
                .prepare_idle_nudge(
                    "root",
                    seq,
                    &message_id("root", seq),
                    true,
                    now + window + 1
                )
                .unwrap()
        );
        h.app.run_idle_nudges(now + window + 1).await.unwrap();
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_child_job_age_guards_scheduler_and_acceptance() {
        for recent in [false, true] {
            for acceptance_only in [false, true] {
                let mut h = Harness::new().await;
                let child = h.spawn_child().await;
                let (events, rx) = mpsc::unbounded_channel();
                let (tx, mut inputs) = mpsc::unbounded_channel();
                h.factory.0.lock().unwrap().insert(
                    child.clone(),
                    Fake {
                        events: rx,
                        inputs: tx,
                    },
                );
                let handle = h.app.handle(&child).await.unwrap();
                handle.resume().await.unwrap();
                let (id, _) = tokio::time::timeout(Duration::from_secs(3), inputs.recv())
                    .await
                    .unwrap()
                    .unwrap();
                events
                    .send(Event::CommandLifecycle {
                        command_uuid: id,
                        state: LifecycleState::Started,
                    })
                    .unwrap();
                let snapshot = job_snapshot(&["preview"]);
                let first_seen = crate::schedule::now_ms()
                    - if recent {
                        0
                    } else {
                        boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 + 1
                    };
                h.store
                    .record_event(
                        &child,
                        "agent_event",
                        serde_json::to_value(Event::BackgroundJobs {
                            snapshot: snapshot.clone(),
                        })
                        .unwrap(),
                        first_seen,
                    )
                    .unwrap();
                publish_jobs(&handle, &events, snapshot).await;
                events
                    .send(Event::Result {
                        subtype: None,
                        is_error: Some(false),
                    })
                    .unwrap();
                let report = tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        if !handle.status().await.unwrap().turn_active {
                            let reports = h
                                .store
                                .reader()
                                .unwrap()
                                .pending_subagent_reports()
                                .unwrap();
                            if let Some(report) =
                                reports.into_iter().find(|r| r.child_session == child)
                            {
                                break report;
                            }
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                h.store
                    .mark_subagent_report_delivered(
                        &child,
                        report.result_seq,
                        crate::schedule::now_ms(),
                        None,
                    )
                    .unwrap();
                let seq = h.job_turn().await;
                let now = h.due_time(seq);
                if acceptance_only {
                    assert_eq!(
                        h.store
                            .prepare_idle_nudge("root", seq, &message_id("root", seq), true, now)
                            .unwrap(),
                        !recent
                    );
                } else {
                    h.app.run_idle_nudges(now).await.unwrap();
                }
                assert_eq!(
                    h.nudge_count(),
                    usize::from(!recent),
                    "recent: {recent}, acceptance only: {acceptance_only}"
                );
            }
        }
    }

    #[tokio::test]
    async fn idle_nudge_after_job_notice_once_and_never_a_chain() {
        let mut h = Harness::new().await;
        let seq = h.job_turn().await;
        assert!(automated_start(&h.store, "root", seq).unwrap());
        // Claude's usage control replies and repeated empty job snapshots do not
        // republish an unchanged status and cannot make a quiet snapshot stale.
        for payload in [
            serde_json::json!({"type":"control_response","response":{}}),
            serde_json::json!({"type":"background_jobs","snapshot":{"available":true,"tasks":[]}}),
        ] {
            h.store
                .record_event("root", "agent_event", payload, crate::schedule::now_ms())
                .unwrap();
        }
        h.app.run_idle_nudges(h.due_time(seq) - 1).await.unwrap();
        assert_eq!(h.nudge_count(), 0);
        h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
        let (id, text) = h.input().await;
        assert_eq!(text, boblm_tuning::IDLE_NUDGE_TEXT);
        assert_eq!(id, message_id("root", seq));
        assert!(!h.store.reader().unwrap().human_input(&id).unwrap());
        let nudge_seq = h.finish(&id).await;
        assert!(!automated_start(&h.store, "root", nudge_seq).unwrap());
        h.app.run_idle_nudges(h.due_time(nudge_seq)).await.unwrap();
        h.app.run_idle_nudges(i64::MAX).await.unwrap();
        assert_eq!(h.nudge_count(), 1);
    }

    #[tokio::test]
    async fn idle_nudge_excludes_owner_peer_and_scheduled_turns() {
        for origin in ["owner", "peer", "schedule"] {
            let mut h = Harness::new().await;
            let id = uuid::Uuid::new_v4().to_string();
            match origin {
                "owner" => h.handle.send(&id, "work").await.unwrap(),
                "peer" => h.handle.send_from_peer(&id, "work", "peer").await.unwrap(),
                _ => h
                    .handle
                    .send_automated(&id, "Scheduled prompt: work")
                    .await
                    .unwrap(),
            };
            h.input().await;
            let seq = h.finish(&id).await;
            h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
            assert_eq!(h.nudge_count(), 0, "{origin}");
        }
    }

    #[tokio::test]
    async fn idle_nudge_drops_blocked_opportunities_permanently() {
        for blocker in [
            "job",
            "unavailable",
            "question",
            "queued",
            "uncertain",
            "held",
            "stop",
            "owner",
            "peer",
        ] {
            let mut h = Harness::new().await;
            let seq = h.job_turn().await;
            match blocker {
                "job" | "unavailable" => {
                    h.events
                        .send(Event::BackgroundJobs {
                            snapshot: JobSnapshot {
                                available: blocker != "unavailable",
                                tasks: if blocker == "job" {
                                    vec![BackgroundJob {
                                        id: "next".into(),
                                        command: "work".into(),
                                        agent_key: None,
                                    }]
                                } else {
                                    vec![]
                                },
                            },
                        })
                        .unwrap();
                    tokio::time::timeout(Duration::from_secs(3), async {
                        loop {
                            let status = h.handle.status().await.unwrap();
                            if status
                                .jobs
                                .is_some_and(|j| !j.available || !j.tasks.is_empty())
                            {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                }
                "question" => {
                    h.store.ask("q", "root", "Choose?", &[], 1).unwrap();
                }
                "queued" | "uncertain" => {
                    h.store
                        .append(boblm_store::NewMessage {
                            uuid: "held",
                            session: "root",
                            role: "user",
                            content: "pending",
                            delivery_state: if blocker == "queued" {
                                "queued"
                            } else {
                                "uncertain"
                            },
                            created_at: crate::schedule::now_ms(),
                        })
                        .unwrap();
                }
                "held" => {
                    h.handle.stop().await.unwrap();
                    h.store.set_runner_stopped("root", false).unwrap();
                }
                "stop" => {
                    h.handle.stop().await.unwrap();
                }
                "owner" => {
                    h.handle
                        .send(&uuid::Uuid::new_v4().to_string(), "new work")
                        .await
                        .unwrap();
                }
                "peer" => {
                    h.handle
                        .send_from_peer(&uuid::Uuid::new_v4().to_string(), "new work", "peer")
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let now = h.due_time(seq);
            h.app.run_idle_nudges(now).await.unwrap();
            assert_eq!(h.nudge_count(), 0, "{blocker}");
            assert!(
                !h.store
                    .reader()
                    .unwrap()
                    .due_idle_nudges(i64::MAX)
                    .unwrap()
                    .iter()
                    .any(|n| n.result_seq == seq),
                "{blocker}"
            );
        }
    }

    #[tokio::test]
    async fn idle_nudge_native_claude_wake_and_duplicate_completed_ack() {
        let mut h = Harness::new().await;
        let owner = uuid::Uuid::new_v4().to_string();
        h.handle.send(&owner, "work").await.unwrap();
        h.input().await;
        let old = h.finish(&owner).await;
        // A completion for the preceding turn must not turn the native wake into owner input.
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: owner,
                state: LifecycleState::Completed,
            })
            .unwrap();
        h.events
            .send(Event::System {
                subtype: Some("task_notification".into()),
                session_id: None,
                status: Some("completed".into()),
            })
            .unwrap();
        h.events
            .send(Event::Result {
                subtype: None,
                is_error: Some(false),
            })
            .unwrap();
        let seq = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let seq = h.settled().await;
                if seq > old {
                    break seq;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_restart_acceptance_and_handoff_are_idempotent() {
        let mut h = Harness::new().await;
        let seq = h.job_turn().await;
        let id = message_id("root", seq);
        let now = h.due_time(seq);
        // Simulate losing the server after durable acceptance, before notifying the live runner.
        assert!(
            h.store
                .prepare_idle_nudge("root", seq, &id, true, now)
                .unwrap()
        );
        let reopened = Store::open(h.store.path()).unwrap();
        assert!(
            reopened
                .prepare_idle_nudge("root", seq, &id, false, now)
                .unwrap()
        );
        h.app.run_idle_nudges(now).await.unwrap();
        assert_eq!(h.input().await.0, id);
        // Simulate a crash after handoff by presenting the same accepted message again.
        assert!(
            !h.handle
                .send_automated(&id, boblm_tuning::IDLE_NUDGE_TEXT)
                .await
                .unwrap()
        );
        reopened.finish_idle_nudge("root", seq, &id).unwrap();
        h.app.run_idle_nudges(now).await.unwrap();
        assert_eq!(h.nudge_count(), 1);
        assert!(h.inputs.try_recv().is_err());
    }

    #[tokio::test]
    async fn idle_nudge_subagent_report_is_a_trigger_but_subagent_results_are_not_candidates() {
        let mut h = Harness::new().await;
        // Start and finish an owner turn so the root has a quiet provider with empty jobs.
        let owner = uuid::Uuid::new_v4().to_string();
        h.handle.send(&owner, "work").await.unwrap();
        h.input().await;
        h.finish(&owner).await;
        let child = h.spawn_child().await;
        h.store
            .record_event(
                &child,
                "agent_event",
                serde_json::json!({"type":"result"}),
                2,
            )
            .unwrap();
        assert!(
            h.store
                .reader()
                .unwrap()
                .due_idle_nudges(i64::MAX)
                .unwrap()
                .iter()
                .all(|n| n.session != child)
        );
        let report = h
            .store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .pop()
            .unwrap();
        let id = crate::subagent_report::report_message_id(&child, report.result_seq);
        h.store
            .mark_subagent_report_delivered(&child, report.result_seq, 3, None)
            .unwrap();
        h.store.set_archived(&child, true).unwrap();
        h.handle
            .send_from_peer(&id, "Subagent Child finished.", &child)
            .await
            .unwrap();
        h.input().await;
        let seq = h.finish(&id).await;
        assert!(automated_start(&h.store, "root", seq).unwrap());
        h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_waits_for_mid_turn_children_and_pending_reports() {
        for report_pending in [false, true] {
            let mut h = Harness::new().await;
            let seq = h.job_turn().await;
            let child = h.spawn_child().await;
            let (events, rx) = mpsc::unbounded_channel();
            let (tx, mut inputs) = mpsc::unbounded_channel();
            h.factory.0.lock().unwrap().insert(
                child.clone(),
                Fake {
                    events: rx,
                    inputs: tx,
                },
            );
            let handle = h.app.handle(&child).await.unwrap();
            handle.resume().await.unwrap();
            let (uuid, _) = tokio::time::timeout(Duration::from_secs(3), inputs.recv())
                .await
                .unwrap()
                .unwrap();
            events
                .send(Event::CommandLifecycle {
                    command_uuid: uuid,
                    state: LifecycleState::Started,
                })
                .unwrap();
            events
                .send(Event::BackgroundJobs {
                    snapshot: JobSnapshot {
                        available: true,
                        tasks: vec![],
                    },
                })
                .unwrap();
            if report_pending {
                events
                    .send(Event::Result {
                        subtype: None,
                        is_error: Some(false),
                    })
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let status = handle.status().await.unwrap();
                    let report = h
                        .store
                        .reader()
                        .unwrap()
                        .has_pending_subagent_report(&child)
                        .unwrap();
                    if status.turn_active != report_pending && report == report_pending {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
            assert_eq!(h.nudge_count(), 0, "pending report: {report_pending}");
        }
    }

    #[tokio::test]
    async fn idle_nudge_allows_an_idle_child_waiting_for_its_parent() {
        let mut h = Harness::new().await;
        let child = h.spawn_child().await;
        h.store.transition(&child, "queued", "started").unwrap();
        // Recovery can hold an empty child queue. This is not a future parent wake.
        let child_handle = h.app.handle(&child).await.unwrap();
        child_handle
            .send_from_peer(&child, "work", "root")
            .await
            .unwrap();
        assert!(child_handle.status().await.unwrap().queue_held);
        let seq = h.job_turn().await;
        h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
        assert_eq!(h.input().await.1, boblm_tuning::IDLE_NUDGE_TEXT);
    }

    #[tokio::test]
    async fn idle_nudge_observes_legacy_native_subagent_lifecycles() {
        for codex in [false, true] {
            for completed in [false, true] {
                let mut h = Harness::new().await;
                let call = if codex {
                    serde_json::json!({"type":"tool_use","id":"native","name":"SubagentActivity",
                        "input":{"agentPath":"/root/child","agentThreadId":"native-thread","kind":"started"}})
                } else {
                    serde_json::json!({"type":"tool_use","id":"native","name":"Agent","input":{}})
                };
                h.store
                    .record_event(
                        "root",
                        "agent_event",
                        serde_json::json!({"type":"assistant","message":{"content":[call]}}),
                        1,
                    )
                    .unwrap();
                if completed {
                    let result = if codex {
                        serde_json::json!({"type":"tool_use","id":"native-done","name":"SubagentActivity",
                            "input":{"agentPath":"/root/child","agentThreadId":"native-thread","kind":"completed"}})
                    } else {
                        serde_json::json!({"type":"tool_result","tool_use_id":"native","content":"done"})
                    };
                    h.store
                        .record_event(
                            "root",
                            "agent_event",
                            serde_json::json!({"type":"user","message":{"content":[result]}}),
                            2,
                        )
                        .unwrap();
                }
                let seq = h.job_turn().await;
                h.app.run_idle_nudges(h.due_time(seq)).await.unwrap();
                assert_eq!(
                    h.nudge_count(),
                    usize::from(completed),
                    "Codex: {codex}, completed: {completed}"
                );
            }
        }
    }
}
