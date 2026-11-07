//! Subagent lifetime: the idle archive backstop and the archived-subagent prune (ADR 0043).
use crate::app::{ApiError, App};
use std::sync::atomic::Ordering;

fn millis(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn quiet(status: &boblm_runner::Status) -> bool {
    !status.archived
        && !status.turn_active
        && !status.stopping
        && status.queued == 0
        && status.dispatching == 0
        && status.uncertain == 0
        && status
            .jobs
            .as_ref()
            .is_none_or(|jobs| jobs.tasks.is_empty())
}

impl App {
    /// Run the backstop archive and, at most once per interval, the prune.
    /// Returns how many subagents were archived and how many were deleted.
    pub(crate) async fn sweep_subagent_lifetime(
        &self,
        now: i64,
    ) -> Result<(usize, usize), ApiError> {
        let archived = self.archive_idle_subagents(now).await?;
        let pruned = self.prune_archived_subagents(now).await?;
        Ok((archived, pruned))
    }

    async fn archive_idle_subagents(&self, now: i64) -> Result<usize, ApiError> {
        let cutoff = now.saturating_sub(millis(boblm_tuning::SUBAGENT_IDLE_ARCHIVE_AGE));
        let store = self.store.clone();
        let candidates =
            tokio::task::spawn_blocking(move || store.reader()?.idle_subagents(cutoff))
                .await
                .map_err(ApiError::internal)??;
        let mut archived = 0;
        for (id, _) in candidates {
            let handle = match self.handle(&id).await {
                Ok(handle) => handle,
                Err(_) => continue,
            };
            let status = match handle.status().await {
                Ok(status) => status,
                Err(error) => {
                    eprintln!(
                        "Could not read subagent {} before archiving it: {error}. It will be checked again.",
                        boblm_protocol::client::shown(&id)
                    );
                    continue;
                }
            };
            if !quiet(&status) {
                continue;
            }
            let store = self.store.clone();
            let child = id.clone();
            let blocked = tokio::task::spawn_blocking(move || {
                let reader = store.reader()?;
                Ok::<_, boblm_store::Error>(
                    reader.pending_question_count(&child)? > 0
                        || reader.has_pending_subagent_report(&child)?,
                )
            })
            .await
            .map_err(ApiError::internal)??;
            if blocked {
                continue;
            }
            if let Err(error) = handle.archive(true).await {
                eprintln!(
                    "Could not archive idle subagent {}: {error}. It will be checked again.",
                    boblm_protocol::client::shown(&id)
                );
                continue;
            }
            archived += 1;
        }
        Ok(archived)
    }

    async fn prune_archived_subagents(&self, now: i64) -> Result<usize, ApiError> {
        let next = self.subagent_prune_at.load(Ordering::Relaxed);
        if now < next {
            return Ok(0);
        }
        self.subagent_prune_at.store(
            now.saturating_add(millis(boblm_tuning::SUBAGENT_PRUNE_INTERVAL)),
            Ordering::Relaxed,
        );
        let cutoff = now.saturating_sub(millis(boblm_tuning::SUBAGENT_PRUNE_AGE));
        let store = self.store.clone();
        let candidates =
            tokio::task::spawn_blocking(move || store.reader()?.prunable_subagents(cutoff))
                .await
                .map_err(ApiError::internal)??;
        let mut pruned = 0;
        for id in candidates {
            let store = self.store.clone();
            let child = id.clone();
            let pending = tokio::task::spawn_blocking(move || {
                store.reader()?.has_pending_subagent_report(&child)
            })
            .await
            .map_err(ApiError::internal)??;
            if pending {
                continue;
            }
            if let Err(error) = crate::app::delete_chat(self, id.clone()).await {
                eprintln!(
                    "Could not prune archived subagent {}: {error}. It will be checked again.",
                    boblm_protocol::client::shown(&id)
                );
                continue;
            }
            pruned += 1;
        }
        Ok(pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use boblm_protocol::{Event, Priority};
    use boblm_runner::{Driver, DriverFactory};
    use boblm_store::{PeerSpawn, Session, Store};
    use serde_json::json;
    use std::{
        io,
        sync::{Arc, Mutex},
    };
    use tokio::sync::mpsc;

    struct Fake {
        events: mpsc::UnboundedReceiver<Event>,
        inputs: mpsc::UnboundedSender<(String, String, Option<Priority>)>,
    }
    #[async_trait]
    impl Driver for Fake {
        async fn send(
            &mut self,
            uuid: &str,
            content: &str,
            priority: Option<Priority>,
        ) -> io::Result<()> {
            self.inputs
                .send((uuid.into(), content.into(), priority))
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
    struct Factory(Mutex<Vec<Fake>>);
    #[async_trait]
    impl DriverFactory for Factory {
        async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
            let mut fakes = self.0.lock().unwrap();
            if fakes.is_empty() {
                return Err(io::Error::other("unexpected spawn"));
            }
            Ok(Box::new(fakes.remove(0)))
        }
    }
    struct Harness {
        _temp: tempfile::TempDir,
        store: Store,
        app: Arc<App>,
        child: String,
        inputs: mpsc::UnboundedReceiver<(String, String, Option<Priority>)>,
    }
    impl Harness {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("lifetime.sqlite")).unwrap();
            store.create_session("root", "/fake", 1).unwrap();
            let project = store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .into_iter()
                .find(|session| session.id == "root")
                .and_then(|session| session.project_id)
                .unwrap();
            let child = "10000000-0000-4000-8000-000000000901".to_owned();
            store
                .spawn_peer(PeerSpawn {
                    request_id: child.clone(),
                    caller: "root".into(),
                    arguments: format!(r#"{{"title":"Worker","requestId":"{child}"}}"#),
                    project_id: project,
                    agent: "claude".into(),
                    model: "sonnet".into(),
                    effort: "high".into(),
                    title: "Worker".into(),
                    content: "work".into(),
                    created_at: 2,
                    parent_session_id: Some("root".into()),
                    root_session_id: Some("root".into()),
                    role: Some("implementer".into()),
                    resolved: None,
                    working_directory: None,
                    account_key: None,
                })
                .unwrap();
            store.transition(&child, "queued", "started").unwrap();
            let (_events, event_rx) = mpsc::unbounded_channel();
            let (input_tx, inputs) = mpsc::unbounded_channel();
            let app = App::open(
                store.clone(),
                Arc::new(Factory(Mutex::new(vec![Fake {
                    events: event_rx,
                    inputs: input_tx,
                }]))),
                "subagent-lifetime-token".into(),
            )
            .await
            .unwrap();
            Self {
                _temp: temp,
                store,
                app,
                child,
                inputs,
            }
        }
        fn activity(&self, at: i64) {
            self.store
                .record_event(
                    &self.child,
                    "agent_event",
                    json!({"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}),
                    at,
                )
                .unwrap();
        }
        fn archived(&self) -> bool {
            self.store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .into_iter()
                .find(|session| session.id == self.child)
                .unwrap()
                .archived
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_subagent_is_archived_after_the_backstop_and_pruned_three_days_later() {
        let h = Harness::new().await;
        let start = crate::schedule::now_ms();
        h.activity(start);
        let (archived, _) = h
            .app
            .sweep_subagent_lifetime(start + millis(boblm_tuning::SUBAGENT_IDLE_ARCHIVE_AGE) - 1)
            .await
            .unwrap();
        assert_eq!(archived, 0);
        let archived_at = start + millis(boblm_tuning::SUBAGENT_IDLE_ARCHIVE_AGE);
        let (archived, _) = h.app.sweep_subagent_lifetime(archived_at).await.unwrap();
        assert_eq!(archived, 1);
        assert!(h.archived());

        let archive_time = h
            .store
            .reader()
            .unwrap()
            .events_since(&h.child, 0, 100)
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "archive_changed")
            .map(|event| event.created_at)
            .unwrap();
        let (_, pruned) = h
            .app
            .sweep_subagent_lifetime(archive_time + millis(boblm_tuning::SUBAGENT_PRUNE_AGE) - 1)
            .await
            .unwrap();
        assert_eq!(pruned, 0);
        let (_, pruned) = h
            .app
            .sweep_subagent_lifetime(
                archive_time
                    + millis(boblm_tuning::SUBAGENT_PRUNE_AGE)
                    + millis(boblm_tuning::SUBAGENT_PRUNE_INTERVAL),
            )
            .await
            .unwrap();
        assert_eq!(pruned, 1);
        assert!(h.store.reader().unwrap().session_deleted(&h.child).unwrap());
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn queued_input_open_questions_and_pending_reports_block_the_backstop() {
        let mut h = Harness::new().await;
        let start = crate::schedule::now_ms();
        h.activity(start);
        let now = start + millis(boblm_tuning::SUBAGENT_IDLE_ARCHIVE_AGE) + 1;
        let handle = h.app.handle(&h.child).await.unwrap();
        handle
            .send("00000000-0000-4000-8000-000000000001", "still working")
            .await
            .unwrap();
        assert!(h.inputs.try_recv().is_err() || true);
        let (archived, _) = h.app.sweep_subagent_lifetime(now).await.unwrap();
        assert_eq!(archived, 0, "queued input must keep a subagent open");
        h.app.shutdown().await.unwrap();

        let h = Harness::new().await;
        h.activity(start);
        h.store
            .ask(
                "20000000-0000-4000-8000-000000000001",
                &h.child,
                "Which way?",
                &[],
                start,
            )
            .unwrap();
        let (archived, _) = h.app.sweep_subagent_lifetime(now).await.unwrap();
        assert_eq!(archived, 0, "an open question must keep a subagent open");
        h.app.shutdown().await.unwrap();

        let h = Harness::new().await;
        h.store
            .record_event(
                &h.child,
                "agent_event",
                json!({"type":"result","subtype":"success","is_error":false}),
                start,
            )
            .unwrap();
        let (archived, _) = h.app.sweep_subagent_lifetime(now).await.unwrap();
        assert_eq!(archived, 0, "a pending report must keep a subagent open");
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_pending_report_blocks_the_prune_and_roots_are_never_pruned() {
        let h = Harness::new().await;
        let start = crate::schedule::now_ms();
        h.store
            .record_event(
                &h.child,
                "agent_event",
                json!({"type":"result","subtype":"success","is_error":false}),
                start,
            )
            .unwrap();
        h.store.set_archived(&h.child, true).unwrap();
        h.store.set_archived("root", true).unwrap();
        let later = start + millis(boblm_tuning::SUBAGENT_PRUNE_AGE) + 1;
        let (_, pruned) = h.app.sweep_subagent_lifetime(later).await.unwrap();
        assert_eq!(pruned, 0);
        assert!(!h.store.reader().unwrap().session_deleted(&h.child).unwrap());
        assert!(!h.store.reader().unwrap().session_deleted("root").unwrap());
        h.app.shutdown().await.unwrap();
    }
}
