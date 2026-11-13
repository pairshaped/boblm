//! Deliver durable result and error reports from a subagent to its parent (ADR 0043).
use crate::{app::App, role_spawn};
use boblm_store::{QuietCheck, Store, SubagentReport};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};

pub(crate) fn report_message_id(child: &str, result_seq: i64) -> String {
    let digest = Sha256::digest(format!("boblm:subagent-report:{child}:{result_seq}"));
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn idle_text(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else {
        let hours = seconds / 3_600;
        let minutes = seconds % 3_600 / 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{minutes}m")
        }
    }
}

fn truncate_utf8(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

const REVIEW_GRADE_HINT: &str =
    "Optional: grade this review with grade_subagent (good, ok or poor).";

fn report_content(
    child_id: &str,
    child_name: &str,
    text: &str,
    open: &[serde_json::Value],
    tpm_limited: bool,
    error: Option<(&str, &str)>,
    grade_hint: bool,
) -> String {
    let label = if child_name.trim().is_empty() {
        child_id.to_owned()
    } else {
        format!("{} ({child_id})", child_name.trim())
    };
    let prefix = if error.is_some_and(|(_, kind)| kind == "subagent_runaway_limit") {
        String::new()
    } else if error.is_some() {
        format!(
            "Subagent {label} hit an error. Fix the cause and use ping_peer with this subagent's session ID to try again, or use archive_peer and start another subagent. The error was:\n\n"
        )
    } else if tpm_limited {
        format!(
            "Subagent {label} stopped because its model provider reached a tokens-per-minute rate limit. Its review may be incomplete. In a few seconds, use ping_peer with this subagent's session ID to ask it to continue. Nothing was resent automatically. The provider's error details are not shown.\n\n"
        )
    } else {
        format!("Subagent {label} finished. Its result follows.\n\n")
    };
    let mut lines = Vec::new();
    for agent in open {
        let id = agent["sessionId"].as_str().unwrap_or("unknown");
        let name = agent["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(id);
        let role = agent["role"].as_str().unwrap_or("subagent");
        let idle = agent["idleSeconds"].as_u64().unwrap_or(0);
        // Reviewers carry no model or effort: reviews are graded blind.
        let on = match agent["model"].as_str() {
            Some(_) => {
                let model = agent["modelLabel"]
                    .as_str()
                    .or_else(|| agent["model"].as_str())
                    .unwrap_or("unknown");
                let effort = agent["effort"].as_str().unwrap_or("default");
                format!(" on {model} at {effort}")
            }
            None => String::new(),
        };
        lines.push(format!(
            "- {name} ({id}), {role}{on}, idle {}",
            idle_text(idle)
        ));
    }
    let open_text = if lines.is_empty() {
        "None.".to_owned()
    } else {
        lines.join("\n")
    };
    // Only a finished review carries the hint; it names no model or provider (ADR 0048).
    let hint = if grade_hint && error.is_none() && !tpm_limited {
        format!("\n\n{REVIEW_GRADE_HINT}")
    } else {
        String::new()
    };
    let suffix = format!(
        "{hint}\n\nOpen subagents:\n{open_text}\n\n{}",
        role_spawn::ReplyTrailer::SubagentReport.text()
    );
    let mut body = if let Some((error, "subagent_runaway_limit")) = error {
        error.trim().to_owned()
    } else if let Some((error, _)) = error {
        format!("\"{}\"", error.trim())
    } else if tpm_limited {
        // The raw error names the model, and reviews are graded blind.
        String::new()
    } else {
        text.trim().to_owned()
    };
    let pointer = "\n\nThe result was shortened. Use inspect_peer with this subagent's session ID to read the full history.";
    let mut content = format!("{prefix}{body}{suffix}");
    if content.len() > boblm_tuning::MAX_MESSAGE_BYTES {
        let available = boblm_tuning::MAX_MESSAGE_BYTES
            .saturating_sub(prefix.len() + pointer.len() + suffix.len());
        truncate_utf8(&mut body, available);
        content = format!("{prefix}{body}{pointer}{suffix}");
    }
    content
}

/// Report text and the optional assistant-text cursor used to suppress repeated results.
fn build_report(
    store: &Store,
    report: &SubagentReport,
    grading_enabled: bool,
) -> Result<(String, Option<i64>), String> {
    let reader = store
        .reader()
        .map_err(|error| format!("Could not read the subagent report: {error}."))?;
    if let Some(accepted) = reader
        .accepted_subagent_report(
            &report.child_session,
            report.result_seq,
            &report_message_id(&report.child_session, report.result_seq),
        )
        .map_err(|error| format!("Could not read the accepted subagent report: {error}."))?
    {
        return Ok(accepted);
    }
    let sessions = reader
        .sessions()
        .map_err(|error| format!("Could not read the subagent chat: {error}."))?;
    let child = sessions
        .iter()
        .find(|session| session.id == report.child_session)
        .ok_or("The subagent chat no longer exists, so its report cannot be built.")?;
    let open = role_spawn::open_subagents(store, &report.parent_session)?;
    if let Some(error) = report.error_message.as_deref() {
        return Ok((
            report_content(
                &child.id,
                &child.name,
                "",
                &open,
                false,
                Some((error, &report.source_kind)),
                false,
            ),
            None,
        ));
    }
    let text = reader
        .last_assistant_text(&report.child_session)
        .map_err(|error| format!("Could not read the subagent's final text: {error}."))?
        .unwrap_or_else(|| "The subagent finished without a final text reply.".to_owned());
    let text_seq = reader
        .last_assistant_text_seq(&report.child_session)
        .map_err(|error| format!("Could not read the subagent's final text: {error}."))?;
    let turn_events = reader.subagent_turn_events(&report.child_session, report.result_seq)
        .map_err(|error| format!("Could not read the subagent's turn outcome: {error}. Inspect its chat before continuing."))?;
    let mut failed = false;
    let mut tpm_error = false;
    for payload in turn_events {
        let event: serde_json::Value = serde_json::from_str(&payload)
            .map_err(|error| format!("Could not read a stored subagent event: {error}. Inspect its chat before continuing."))?;
        if event["type"] == "result" {
            failed = event["is_error"] == true;
        } else {
            let text = event["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let text = text.trim_start();
            if (text.starts_with("Codex stopped this turn with an error:")
                || text.starts_with("Codex reported an error:"))
                && boblm_tuning::is_api_tpm_rate_limit(text)
            {
                tpm_error = true;
            }
        }
    }
    Ok((
        report_content(
            &child.id,
            &child.name,
            &text,
            &open,
            failed && tpm_error,
            None,
            grading_enabled && child.role.as_deref() == Some("reviewer"),
        ),
        text_seq,
    ))
}

fn child_quiet(status: &boblm_runner::Status) -> bool {
    !status.turn_active
        && !status.stopping
        && status.queued == 0
        && status.dispatching == 0
        && status.uncertain == 0
        && !status.queue_held
}

impl App {
    /// Deliver error reports immediately and result reports once the child is quiet. The
    /// durable outbox and source-event UUID prevent duplicate parent input after a restart.
    pub(crate) async fn deliver_subagent_reports(self: &Arc<Self>) {
        let now = crate::schedule::now_ms();
        self.deliver_subagent_reports_at(now).await;
    }

    async fn deliver_subagent_reports_at(self: &Arc<Self>, now: i64) {
        let store = self.store.clone();
        let Ok(Ok(reports)) = tokio::task::spawn_blocking(move || {
            // A result that landed after the last delivery with no new text repeats it.
            store.supersede_repeated_subagent_reports(now)?;
            store.reader()?.due_subagent_reports(now)
        })
        .await
        else {
            return;
        };
        // Several results queued before the child was quiet are one report: the newest, which
        // is built from the child's final text. The older rows are superseded, not delivered.
        let mut newest: HashMap<String, i64> = HashMap::new();
        for report in reports
            .iter()
            .filter(|report| report.source_kind == "result")
        {
            let seq = newest.entry(report.child_session.clone()).or_default();
            *seq = (*seq).max(report.result_seq);
        }
        for (child, seq) in &newest {
            let store = self.store.clone();
            let (child, seq) = (child.clone(), *seq);
            if let Err(error) = tokio::task::spawn_blocking(move || {
                store.supersede_older_subagent_reports(&child, seq, now)
            })
            .await
            .unwrap_or_else(|error| Err(boblm_store::Error::Io(std::io::Error::other(error))))
            {
                eprintln!(
                    "Older subagent reports could not be marked superseded: {error}. They stay pending and the server will try again."
                );
                return;
            }
        }

        let grading_enabled = self.subagent_grading_enabled();
        for report in reports.into_iter().filter(|report| {
            report.source_kind != "result" || newest[&report.child_session] == report.result_seq
        }) {
            if report.source_kind == "result" {
                let Ok(handle) = self.handle(&report.child_session).await else {
                    self.fail_report(&report, "The subagent chat is no longer available.")
                        .await;
                    continue;
                };
                let Ok(status) = handle.status().await else {
                    continue;
                };
                if !child_quiet(&status) {
                    continue;
                }
                let child = report.child_session.clone();
                let seq = report.result_seq;
                let check = self
                    .blocking("check the subagent's background jobs", move |store| {
                        let check = store.reader()?.job_quiet_check(
                            &child,
                            status.running,
                            status.jobs.as_ref(),
                            now,
                        )?;
                        if let QuietCheck::RecentJobsUntil(until) = check
                            && let Some(jobs) = status.jobs.as_ref()
                        {
                            store.defer_subagent_report(&child, seq, until, jobs)?;
                        }
                        Ok(check)
                    })
                    .await;
                if !matches!(check, Ok(QuietCheck::Quiet)) {
                    continue;
                }
            }
            let store = self.store.clone();
            let report_for_build = report.clone();
            let (content, text_seq) = match tokio::task::spawn_blocking(move || {
                let (content, text_seq) = build_report(&store, &report_for_build, grading_enabled)?;
                store
                    .remember_subagent_report_text_seq(
                        &report_for_build.child_session,
                        report_for_build.result_seq,
                        text_seq,
                    )
                    .map_err(|error| {
                        format!("Could not save the subagent report's text cursor: {error}.")
                    })?;
                Ok::<_, String>((content, text_seq))
            })
            .await
            {
                Ok(Ok(content)) => content,
                Ok(Err(error)) => {
                    self.fail_report(&report, &error).await;
                    continue;
                }
                Err(error) => {
                    self.fail_report(&report, &format!("The report could not be built: {error}."))
                        .await;
                    continue;
                }
            };
            let id = report_message_id(&report.child_session, report.result_seq);
            let parent = match self.handle(&report.parent_session).await {
                Ok(parent) => parent,
                Err(_) => {
                    self.fail_report(&report, "The parent chat is no longer available.")
                        .await;
                    continue;
                }
            };
            match parent
                .send_from_peer(&id, &content, &report.child_session)
                .await
            {
                Ok(_) => {
                    let store = self.store.clone();
                    let child = report.child_session.clone();
                    let seq = report.result_seq;
                    let now = crate::schedule::now_ms();
                    if let Err(error) = tokio::task::spawn_blocking(move || {
                        store.mark_subagent_report_delivered(&child, seq, now, text_seq)?;
                        // A result queued during the delivery repeats this text unless the
                        // child wrote something new after it.
                        store.supersede_repeated_subagent_reports(now).map(|_| ())
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(boblm_store::Error::Io(std::io::Error::other(error)))
                    }) {
                        eprintln!(
                            "The subagent report was sent but its outbox row could not be marked delivered: {error}. The server will retry it with the same message ID."
                        );
                    }
                }
                Err(error) => {
                    self.fail_report(&report, &format!("{error}")).await;
                }
            }
        }
    }

    async fn fail_report(&self, report: &SubagentReport, error: &str) {
        let store = self.store.clone();
        let child = report.child_session.clone();
        let seq = report.result_seq;
        let parent = report.parent_session.clone();
        let message = error.to_owned();
        let now = crate::schedule::now_ms();
        let id = report_message_id(&report.child_session, report.result_seq);
        let record = store.clone();
        let record_child = child.clone();
        let record_message = message.clone();
        if let Err(mark) = tokio::task::spawn_blocking(move || {
            store.mark_subagent_report_failed(&child, seq, now, &message)
        })
        .await
        .unwrap_or_else(|error| Err(boblm_store::Error::Io(std::io::Error::other(error))))
        {
            eprintln!(
                "The subagent report failure could not be recorded: {mark}. Parent: {parent}."
            );
            return;
        }
        if let Err(record_error) = tokio::task::spawn_blocking(move || {
            record.record_event(
                &record_child,
                "subagent_report_failed",
                serde_json::json!({
                    "report_id": id,
                    "parent_session_id": parent,
                    "result_seq": seq,
                    "message": record_message,
                }),
                now,
            )
        })
        .await
        .unwrap_or_else(|error| Err(boblm_store::Error::Io(std::io::Error::other(error))))
        {
            eprintln!("The subagent report failure event could not be stored: {record_error}.");
        }
        eprintln!(
            "A subagent report could not be delivered to its parent: {error}. It will not be retried automatically."
        );
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
    use std::{io, sync::Mutex};
    use tokio::sync::mpsc;

    type SendHook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
    struct Fake {
        events: mpsc::UnboundedReceiver<Event>,
        inputs: mpsc::UnboundedSender<(String, String, Option<Priority>)>,
        on_send: SendHook,
    }
    #[async_trait]
    impl Driver for Fake {
        fn wakes_itself_on_background_job_completion(&self) -> bool {
            // This harness models Claude, which supplies its own completion wake.
            true
        }

        async fn send(
            &mut self,
            uuid: &str,
            content: &str,
            priority: Option<Priority>,
        ) -> io::Result<()> {
            if let Some(hook) = self.on_send.lock().unwrap().take() {
                hook();
            }
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
    struct Factory(Mutex<Option<Fake>>, Mutex<Option<Fake>>);
    #[async_trait]
    impl DriverFactory for Factory {
        async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
            if session.id != "parent" {
                if let Some(child) = self.1.lock().unwrap().take() {
                    return Ok(Box::new(child));
                }
                std::fs::read_dir(&session.working_directory).map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!(
                            "Could not start the fake child in {}: {error}",
                            session.working_directory
                        ),
                    )
                })?;
                panic!("the fake child must have a missing working folder");
            }
            Ok(Box::new(
                self.0
                    .lock()
                    .unwrap()
                    .take()
                    .expect("one fake parent child"),
            ))
        }
    }
    struct Harness {
        _temp: tempfile::TempDir,
        _events: mpsc::UnboundedSender<Event>,
        store: Store,
        app: Arc<App>,
        child: String,
        child_events: mpsc::UnboundedSender<Event>,
        inputs: mpsc::UnboundedReceiver<(String, String, Option<Priority>)>,
        on_send: SendHook,
    }

    #[tokio::test]
    async fn runaway_report_preserves_the_parent_decision_and_suppresses_bare_result() {
        let h = Harness::new().await;
        h.store
            .record_event(
                &h.child,
                "agent_event",
                json!({"type":"token_usage", "model":"fixture",
                "input_tokens": boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS + 1,
                "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0}),
                10,
            )
            .unwrap();
        assert!(h.store.pause_runaway_subagent_round(&h.child, 11).unwrap());
        let report = h
            .store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .into_iter()
            .find(|r| r.source_kind == "subagent_runaway_limit")
            .unwrap();
        let (content, text_seq) = build_report(&h.store, &report, false).unwrap();
        assert!(content.starts_with("Worker was paused after using "));
        assert!(content.contains("Decide whether it is still making real progress"));
        assert!(content.contains("a new round starts with a fresh limit"));
        assert!(content.contains("ask the owner with the ask tool before continuing"));
        assert!(!content.contains("Fix the cause"));
        assert!(content.contains("Open subagents:"));
        assert!(text_seq.is_none());
        h.store
            .record_event(&h.child, "agent_event", json!({"type":"result"}), 12)
            .unwrap();
        h.store.supersede_repeated_subagent_reports(13).unwrap();
        assert!(
            !h.store
                .reader()
                .unwrap()
                .pending_subagent_reports()
                .unwrap()
                .iter()
                .any(|r| r.source_kind == "result")
        );
    }

    impl Harness {
        async fn new() -> Self {
            Self::with_live_child(true).await
        }
        async fn with_live_child(live: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("db.sqlite")).unwrap();
            store
                .create_session(
                    "parent",
                    temp.path().join("missing-worktree").to_str().unwrap(),
                    1,
                )
                .unwrap();
            let project = store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .into_iter()
                .find(|session| session.id == "parent")
                .and_then(|session| session.project_id)
                .unwrap();
            let child = "10000000-0000-4000-8000-000000000801".to_owned();
            store
                .spawn_peer(PeerSpawn {
                    request_id: child.clone(),
                    caller: "parent".into(),
                    arguments: format!(r#"{{"title":"Worker","requestId":"{child}"}}"#),
                    project_id: project,
                    agent: "claude".into(),
                    model: "sonnet".into(),
                    effort: "high".into(),
                    title: "Worker".into(),
                    content: "work".into(),
                    created_at: 2,
                    parent_session_id: Some("parent".into()),
                    root_session_id: Some("parent".into()),
                    role: Some("implementer".into()),
                    resolved: None,
                    working_directory: None,
                    account_key: None,
                })
                .unwrap();
            store.transition(&child, "queued", "started").unwrap();
            let (events, event_rx) = mpsc::unbounded_channel();
            let (input_tx, inputs) = mpsc::unbounded_channel();
            let on_send = SendHook::default();
            let (child_events, child_rx) = mpsc::unbounded_channel();
            let (child_inputs, mut child_input_rx) = mpsc::unbounded_channel();
            let app = App::open(
                store.clone(),
                Arc::new(Factory(
                    Mutex::new(Some(Fake {
                        events: event_rx,
                        inputs: input_tx,
                        on_send: on_send.clone(),
                    })),
                    Mutex::new(live.then_some(Fake {
                        events: child_rx,
                        inputs: child_inputs,
                        on_send: SendHook::default(),
                    })),
                )),
                "subagent-report-fixture-token".into(),
            )
            .await
            .unwrap();
            app.handle_for_tests(&child).await.resume().await.unwrap();
            if live {
                let handle = app.handle_for_tests(&child).await;
                let uuid = "10000000-0000-4000-8000-000000000899";
                handle.send(uuid, "initialize fake provider").await.unwrap();
                child_input_rx.recv().await.unwrap();
                child_events
                    .send(Event::CommandLifecycle {
                        command_uuid: uuid.into(),
                        state: boblm_protocol::LifecycleState::Completed,
                    })
                    .unwrap();
                child_events
                    .send(
                        serde_json::from_value(json!({"type":"result","subtype":"success"}))
                            .unwrap(),
                    )
                    .unwrap();
                loop {
                    let status = handle.status().await.unwrap();
                    if !status.turn_active && status.dispatching == 0 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                // Remove initialization input and its report so each test starts with only
                // the original child task and a known, empty provider inventory.
                let db = rusqlite::Connection::open(temp.path().join("db.sqlite")).unwrap();
                db.execute("DELETE FROM subagent_reports", []).unwrap();
                db.execute(
                    "DELETE FROM messages WHERE uuid = '10000000-0000-4000-8000-000000000899'",
                    [],
                )
                .unwrap();
            }
            Self {
                _temp: temp,
                _events: events,
                store,
                app,
                child,
                child_events,
                inputs,
                on_send,
            }
        }
        async fn jobs(&self, snapshot: &boblm_protocol::jobs::JobSnapshot, first_seen: i64) {
            self.store
                .record_event(
                    &self.child,
                    "agent_event",
                    json!({"type":"background_jobs", "snapshot":snapshot}),
                    first_seen,
                )
                .unwrap();
            self.child_events
                .send(Event::BackgroundJobs {
                    snapshot: snapshot.clone(),
                })
                .unwrap();
            let handle = self.app.handle_for_tests(&self.child).await;
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while handle.status().await.unwrap().jobs.as_ref() != Some(snapshot) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }

        fn result(&self) -> i64 {
            self.result_saying("All done from the worker.")
        }
        /// One finished turn: the child's final text, then its result event. Returns the
        /// newest queued report's result sequence.
        fn result_saying(&self, text: &str) -> i64 {
            self.store
                .record_event(
                    &self.child,
                    "agent_event",
                    json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}}),
                    3,
                )
                .unwrap();
            self.store
                .record_event(
                    &self.child,
                    "agent_event",
                    json!({"type":"result","subtype":"success","is_error":false}),
                    4,
                )
                .unwrap();
            self.store
                .reader()
                .unwrap()
                .subagent_reports()
                .unwrap()
                .last()
                .unwrap()
                .result_seq
        }
    }

    #[tokio::test]
    async fn a_child_start_failure_reports_once_while_held_even_after_resumes() {
        let mut h = Harness::with_live_child(false).await;
        let child = h.app.handle_for_tests(&h.child).await;
        child
            .send("10000000-0000-4000-8000-000000000802", "follow up")
            .await
            .unwrap();
        let status = child.status().await.unwrap();
        assert!(status.queue_held);
        assert_eq!(status.queued, 1);
        h.app.deliver_subagent_reports().await;
        let (_, content, _) =
            tokio::time::timeout(std::time::Duration::from_secs(1), h.inputs.recv())
                .await
                .expect("a held child must report its startup error")
                .unwrap();
        assert!(content.contains("hit an error"));
        assert!(content.contains("could not be started"));
        assert!(content.contains(h._temp.path().join("missing-worktree").to_str().unwrap()));
        assert!(content.contains("ping_peer"));
        assert!(content.contains("archive_peer"));
        assert!(content.contains("Open subagents:"));
        assert!(content.contains("No reply is needed"));
        for _ in 0..3 {
            child.resume().await.unwrap();
            h.app.deliver_subagent_reports().await;
        }
        let reports = h.store.reader().unwrap().subagent_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].state, "delivered");
        assert_eq!(
            h.store
                .reader()
                .unwrap()
                .messages("parent", 0, i64::MAX)
                .unwrap()
                .len(),
            1
        );
        assert!(h.inputs.try_recv().is_err());
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn error_deduplication_resets_only_after_a_started_turn_or_result() {
        let h = Harness::new().await;
        for (index, recovery) in [
            json!({"type":"command_lifecycle","state":"started","command_uuid":"work"}),
            json!({"type":"system","subtype":"turn_started"}),
            json!({"type":"result","is_error":false}),
        ]
        .into_iter()
        .enumerate()
        {
            for _ in 0..2 {
                h.store
                    .record_event(
                        &h.child,
                        "model_error",
                        json!({"message":"The model could not be applied. Choose another model."}),
                        5,
                    )
                    .unwrap();
            }
            h.store
                .record_event(&h.child, "agent_event", recovery, 6)
                .unwrap();
            h.store
                .record_event(
                    &h.child,
                    "model_error",
                    json!({"message":"The model could not be applied. Choose another model."}),
                    7,
                )
                .unwrap();
            let reports = h.store.reader().unwrap().subagent_reports().unwrap();
            assert_eq!(
                reports
                    .iter()
                    .filter(|report| report.source_kind == "model_error")
                    .count(),
                index + 2
            );
        }
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_timeout_without_a_message_reports_the_cause_and_deduplicates() {
        let mut h = Harness::new().await;
        for _ in 0..2 {
            h.store
                .record_event(&h.child, "delivery_timeout", json!({}), 5)
                .unwrap();
        }
        h.app.deliver_subagent_reports().await;
        let (_, content, _) = h.inputs.recv().await.unwrap();
        assert!(content.contains("hit an error"));
        assert!(content.contains("did not confirm message consumption before the deadline"));
        assert!(content.contains("Unconfirmed messages are held"));
        assert_eq!(
            h.store.reader().unwrap().subagent_reports().unwrap().len(),
            1
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_result_without_new_text_after_an_error_sends_nothing_but_a_new_answer_does() {
        let mut h = Harness::new().await;
        // A previous result's text cursor must not suppress a later error.
        h.result();
        h.app.deliver_subagent_reports().await;
        h.inputs.recv().await.unwrap();
        h.store
            .record_event(
                &h.child,
                "model_error",
                json!({"message":"The model could not be applied. Choose another model."}),
                5,
            )
            .unwrap();
        // The bare result may land before the scheduler sees the error.
        h.store
            .record_event(
                &h.child,
                "agent_event",
                json!({"type":"result","is_error":true}),
                6,
            )
            .unwrap();
        h.app.deliver_subagent_reports().await;
        let reports = h.store.reader().unwrap().subagent_reports().unwrap();
        assert_eq!(
            reports
                .iter()
                .map(|report| report.state.as_str())
                .collect::<Vec<_>>(),
            ["delivered", "delivered", "superseded"]
        );
        let parent = h
            .store
            .reader()
            .unwrap()
            .messages("parent", 0, i64::MAX)
            .unwrap();
        assert_eq!(parent.len(), 2);
        assert!(parent[1].content.contains("hit an error"));
        h.result_saying("Recovered and completed the work.");
        h.app.deliver_subagent_reports().await;
        h.app.deliver_subagent_reports().await;
        let parent = h
            .store
            .reader()
            .unwrap()
            .messages("parent", 0, i64::MAX)
            .unwrap();
        assert_eq!(parent.len(), 3);
        assert!(
            parent[2]
                .content
                .contains("Recovered and completed the work.")
        );
        assert!(parent[2].content.contains("finished"));
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn provider_result_failures_queue_only_one_report() {
        for cause in [
            "Rate limit reached for the model on tokens per min (TPM). Try again in 1s.",
            "You exceeded your current quota. Check your plan and billing details.",
            "The account has insufficient credits. Check your billing details.",
        ] {
            let h = Harness::new().await;
            h.store.record_event(&h.child, "agent_event", json!({"type":"assistant", "message":{"content":[{"type":"text", "text":format!("Codex stopped this turn with an error: {cause}")}]}}), 3).unwrap();
            h.store
                .record_event(
                    &h.child,
                    "agent_event",
                    json!({"type":"result","is_error":true}),
                    4,
                )
                .unwrap();
            h.app.deliver_subagent_reports().await;
            h.app.deliver_subagent_reports().await;
            assert_eq!(
                h.store.reader().unwrap().subagent_reports().unwrap().len(),
                1
            );
            assert_eq!(
                h.store
                    .reader()
                    .unwrap()
                    .messages("parent", 0, i64::MAX)
                    .unwrap()
                    .len(),
                1
            );
            h.app.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_tpm_limited_report_explains_how_to_continue_and_does_not_replay() {
        let h = Harness::new().await;
        h.store.record_event(&h.child, "agent_event", json!({
            "type":"assistant", "message":{"content":[{"type":"text","text":
            "Codex stopped this turn with an error: Rate limit reached for gpt-6-luna in organization org-example on tokens per min (TPM): Limit 2000000, Used 1800000, Requested 240000. Please try again in 1.2s."
        }]}}), 3).unwrap();
        h.store
            .record_event(
                &h.child,
                "agent_event",
                json!({
                    "type":"result", "subtype":"error_during_execution", "is_error":true
                }),
                4,
            )
            .unwrap();
        let report = h
            .store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .pop()
            .unwrap();
        let (text, _) = build_report(&h.store, &report, false).unwrap();
        assert!(
            text.contains(
                "stopped because its model provider reached a tokens-per-minute rate limit"
            )
        );
        assert!(text.contains("review may be incomplete"));
        assert!(!text.contains("OpenAI") && !text.contains("luna"), "{text}");
        assert!(text.contains("In a few seconds, use ping_peer"));
        assert!(text.contains("Nothing was resent automatically"));
        assert!(!text.contains("finished. Its result follows"));
        assert_eq!(
            h.store
                .reader()
                .unwrap()
                .messages(&h.child, 0, i64::MAX)
                .unwrap()
                .len(),
            1
        );

        // A subsequent successful turn must not inherit an earlier TPM failure.
        h.result_saying("Review completed after an explicit ping.");
        let report = h
            .store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .pop()
            .unwrap();
        let (text, _) = build_report(&h.store, &report, false).unwrap();
        assert!(text.contains("finished. Its result follows"));
        assert!(!text.contains("tokens-per-minute rate limit"));
    }

    #[tokio::test]
    async fn a_report_delivers_with_a_long_lived_job() {
        let mut h = Harness::new().await;
        let snapshot = boblm_protocol::jobs::JobSnapshot {
            available: true,
            tasks: vec![boblm_protocol::jobs::BackgroundJob {
                id: "docs-server".into(),
                command: "serve docs".into(),
                agent_key: None,
            }],
        };
        h.jobs(
            &snapshot,
            crate::schedule::now_ms()
                - boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64
                - 1,
        )
        .await;
        h.result();
        h.app.deliver_subagent_reports().await;
        assert_eq!(
            h.store.reader().unwrap().subagent_reports().unwrap()[0].state,
            "delivered"
        );
        assert!(
            h.inputs
                .recv()
                .await
                .unwrap()
                .1
                .contains("All done from the worker.")
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn recent_jobs_defer_until_the_youngest_ages_out_and_restart_delivers_once() {
        let mut h = Harness::new().await;
        let now = crate::schedule::now_ms();
        let window = boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64;
        let job = |id: &str| boblm_protocol::jobs::BackgroundJob {
            id: id.into(),
            command: "serve docs".into(),
            agent_key: None,
        };
        let mut snapshot = boblm_protocol::jobs::JobSnapshot {
            available: true,
            tasks: vec![job("old"), job("older-recent")],
        };
        h.jobs(
            &boblm_protocol::jobs::JobSnapshot {
                available: true,
                tasks: vec![job("old")],
            },
            now - window - 1,
        )
        .await;
        h.jobs(&snapshot, now - 10).await;
        snapshot.tasks.push(job("youngest"));
        h.jobs(&snapshot, now).await;
        let seq = h.result();
        h.app.deliver_subagent_reports_at(now).await;
        let reopened = Store::open(h.store.path()).unwrap();
        assert_eq!(
            reopened
                .reader()
                .unwrap()
                .pending_subagent_reports()
                .unwrap()[0]
                .state,
            "pending"
        );
        assert!(
            reopened
                .reader()
                .unwrap()
                .due_subagent_reports(now + window)
                .unwrap()
                .is_empty()
        );
        // Repeated observations, even in a different order, cannot cause per-tick retries.
        snapshot.tasks.reverse();
        h.jobs(&snapshot, now + 1).await;
        assert!(
            reopened
                .reader()
                .unwrap()
                .due_subagent_reports(now + window)
                .unwrap()
                .is_empty()
        );
        h.app.deliver_subagent_reports_at(now + window).await;
        assert!(h.inputs.try_recv().is_err());
        let deadline = now + window + 1;
        assert_eq!(
            reopened
                .reader()
                .unwrap()
                .due_subagent_reports(deadline)
                .unwrap()[0]
                .result_seq,
            seq
        );
        h.app.deliver_subagent_reports_at(deadline).await;
        let (id, content, _) = h.inputs.recv().await.unwrap();
        assert_eq!(id, report_message_id(&h.child, seq));
        assert!(content.contains("All done from the worker."));
        h._events
            .send(Event::CommandLifecycle {
                command_uuid: id.clone(),
                state: boblm_protocol::LifecycleState::Started,
            })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while reopened
                .reader()
                .unwrap()
                .messages("parent", 0, i64::MAX)
                .unwrap()[0]
                .delivery_state
                != "started"
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Lose only outbox settlement, as a web restart between handoff and marking it would.
        let db = rusqlite::Connection::open(h.store.path()).unwrap();
        db.execute("UPDATE subagent_reports SET state = 'pending'", [])
            .unwrap();
        db.execute(
            "UPDATE sessions SET name = 'Renamed after acceptance' WHERE id = ?1",
            [&h.child],
        )
        .unwrap();
        h.app.deliver_subagent_reports_at(deadline).await;
        assert_eq!(
            reopened.reader().unwrap().subagent_reports().unwrap()[0].state,
            "delivered"
        );
        assert_eq!(
            reopened
                .reader()
                .unwrap()
                .messages("parent", 0, i64::MAX)
                .unwrap()
                .len(),
            1
        );
        assert!(h.inputs.try_recv().is_err());
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_recent_job_finishing_early_releases_a_deferred_report() {
        for remaining_old_job in [false, true] {
            let mut h = Harness::new().await;
            let now = crate::schedule::now_ms();
            let mut snapshot = boblm_protocol::jobs::JobSnapshot {
                available: true,
                tasks: vec![boblm_protocol::jobs::BackgroundJob {
                    id: "old".into(),
                    command: "serve".into(),
                    agent_key: None,
                }],
            };
            h.jobs(
                &snapshot,
                now - boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 - 1,
            )
            .await;
            snapshot.tasks.push(boblm_protocol::jobs::BackgroundJob {
                id: "recent".into(),
                command: "build".into(),
                agent_key: None,
            });
            h.jobs(&snapshot, now).await;
            h.result();
            h.app.deliver_subagent_reports_at(now).await;
            assert!(
                h.store
                    .reader()
                    .unwrap()
                    .due_subagent_reports(now)
                    .unwrap()
                    .is_empty()
            );
            snapshot.tasks.pop();
            if !remaining_old_job {
                snapshot.tasks.clear();
            }
            h.jobs(&snapshot, now + 1).await;
            assert_eq!(
                h.store
                    .reader()
                    .unwrap()
                    .due_subagent_reports(now)
                    .unwrap()
                    .len(),
                1
            );
            h.app.deliver_subagent_reports_at(now + 1).await;
            assert!(
                h.inputs
                    .recv()
                    .await
                    .unwrap()
                    .1
                    .contains("All done from the worker.")
            );
            h.app.deliver_subagent_reports_at(i64::MAX).await;
            assert!(h.inputs.try_recv().is_err());
            h.app.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn old_jobs_still_require_an_available_inventory_and_known_ages() {
        for unknown_age in [false, true] {
            let mut h = Harness::new().await;
            let now = crate::schedule::now_ms();
            let mut snapshot = boblm_protocol::jobs::JobSnapshot {
                available: true,
                tasks: vec![boblm_protocol::jobs::BackgroundJob {
                    id: "server".into(),
                    command: "serve".into(),
                    agent_key: None,
                }],
            };
            h.jobs(
                &snapshot,
                now - boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 - 1,
            )
            .await;
            if !unknown_age {
                snapshot.available = false;
                h.jobs(&snapshot, now).await;
            }
            h.result();
            if unknown_age {
                rusqlite::Connection::open(h.store.path())
                    .unwrap()
                    .execute("DELETE FROM idle_nudge_jobs", [])
                    .unwrap();
            }
            h.app.deliver_subagent_reports_at(now).await;
            h.app.deliver_subagent_reports_at(i64::MAX).await;
            assert_eq!(
                h.store.reader().unwrap().subagent_reports().unwrap()[0].state,
                "pending"
            );
            assert!(h.inputs.try_recv().is_err());
            h.app.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn an_inventory_change_before_deferral_cannot_hide_an_early_completion() {
        let h = Harness::new().await;
        let now = crate::schedule::now_ms();
        let mut snapshot = boblm_protocol::jobs::JobSnapshot {
            available: true,
            tasks: vec![boblm_protocol::jobs::BackgroundJob {
                id: "build".into(),
                command: "build docs".into(),
                agent_key: None,
            }],
        };
        h.jobs(&snapshot, now).await;
        let seq = h.result();
        let stale = snapshot.clone();
        snapshot.tasks.clear();
        h.jobs(&snapshot, now + 1).await;
        h.store
            .defer_subagent_report(
                &h.child,
                seq,
                now + boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 + 1,
                &stale,
            )
            .unwrap();
        assert_eq!(
            h.store.reader().unwrap().due_subagent_reports(now).unwrap()[0].result_seq,
            seq
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_new_result_supersedes_a_deferred_report() {
        let mut h = Harness::new().await;
        let now = crate::schedule::now_ms();
        let snapshot = boblm_protocol::jobs::JobSnapshot {
            available: true,
            tasks: vec![boblm_protocol::jobs::BackgroundJob {
                id: "server".into(),
                command: "serve".into(),
                agent_key: None,
            }],
        };
        h.jobs(&snapshot, now).await;
        h.result();
        h.app.deliver_subagent_reports_at(now).await;
        h.result_saying("A newer final answer.");
        h.app.deliver_subagent_reports_at(now).await;
        h.app
            .deliver_subagent_reports_at(
                now + boblm_tuning::IDLE_NUDGE_RECENT_JOB_WINDOW.as_millis() as i64 + 1,
            )
            .await;
        assert!(
            h.inputs
                .recv()
                .await
                .unwrap()
                .1
                .contains("A newer final answer.")
        );
        let reports = h.store.reader().unwrap().subagent_reports().unwrap();
        assert_eq!(reports[0].state, "superseded");
        assert_eq!(reports[1].state, "delivered");
        assert!(h.inputs.try_recv().is_err());
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_result_waits_for_a_known_available_empty_job_snapshot() {
        // A child with no provider process has no jobs, so its report is delivered.
        let mut stopped = Harness::with_live_child(false).await;
        stopped.result();
        stopped.app.deliver_subagent_reports().await;
        assert!(stopped.inputs.recv().await.is_some());

        let mut h = Harness::new().await;
        for available in [false, true] {
            h.child_events
                .send(Event::BackgroundJobs {
                    snapshot: boblm_protocol::jobs::JobSnapshot {
                        tasks: vec![],
                        available,
                    },
                })
                .unwrap();
            let handle = h.app.handle_for_tests(&h.child).await;
            loop {
                if handle
                    .status()
                    .await
                    .unwrap()
                    .jobs
                    .as_ref()
                    .is_some_and(|j| j.available == available)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            h.result();
            h.app.deliver_subagent_reports().await;
            if available {
                assert!(h.inputs.recv().await.is_some());
            } else {
                assert!(h.inputs.try_recv().is_err());
                assert!(
                    !h.store
                        .reader()
                        .unwrap()
                        .pending_subagent_reports()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    #[tokio::test]
    async fn a_quiet_child_report_reaches_the_parent_once_with_open_subagents() {
        let mut h = Harness::new().await;
        let seq = h.result();
        h.app.deliver_subagent_reports().await;
        let (id, content, _) = h.inputs.recv().await.expect("parent input");
        assert_eq!(id, report_message_id(&h.child, seq));
        assert!(content.contains("All done from the worker."));
        assert!(content.contains("Worker"));
        assert!(content.contains("Open subagents:"));
        assert!(content.contains("idle "));
        assert!(content.contains("No reply is needed"));
        let report = &h.store.reader().unwrap().subagent_reports().unwrap()[0];
        assert_eq!(report.state, "delivered");
        h.app.deliver_subagent_reports().await;
        assert!(h.inputs.try_recv().is_err());
    }

    #[tokio::test]
    async fn two_results_before_the_child_is_quiet_deliver_one_report_for_the_newest() {
        let mut h = Harness::new().await;
        let older = h.result_saying("First answer.");
        let newer = h.result_saying("Second answer.");
        assert_ne!(older, newer);
        h.app.deliver_subagent_reports().await;
        let (id, content, _) = h.inputs.recv().await.expect("parent input");
        assert_eq!(id, report_message_id(&h.child, newer));
        assert!(content.contains("Second answer."));
        assert!(
            h.inputs.try_recv().is_err(),
            "the older result sends nothing"
        );
        let states: Vec<(i64, String)> = h
            .store
            .reader()
            .unwrap()
            .subagent_reports()
            .unwrap()
            .into_iter()
            .map(|report| (report.result_seq, report.state))
            .collect();
        assert_eq!(
            states,
            [
                (older, "superseded".to_owned()),
                (newer, "delivered".to_owned())
            ]
        );
        h.app.deliver_subagent_reports().await;
        assert!(h.inputs.try_recv().is_err());
    }

    /// Queue a result from inside the in-flight delivery, as the child finishing again would.
    fn finish_during_delivery(h: &Harness, text: Option<&'static str>) {
        let (store, child) = (h.store.clone(), h.child.clone());
        *h.on_send.lock().unwrap() = Some(Box::new(move || {
            if let Some(text) = text {
                store
                    .record_event(
                        &child,
                        "agent_event",
                        json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}}),
                        5,
                    )
                    .unwrap();
            }
            store
                .record_event(
                    &child,
                    "agent_event",
                    json!({"type":"result","subtype":"success","is_error":false}),
                    6,
                )
                .unwrap();
        }));
    }

    #[tokio::test]
    async fn a_result_queued_during_delivery_without_new_text_sends_nothing() {
        let mut h = Harness::new().await;
        h.result();
        finish_during_delivery(&h, None);
        h.app.deliver_subagent_reports().await;
        h.inputs.recv().await.expect("first report");
        h.app.deliver_subagent_reports().await;
        assert!(
            h.inputs.try_recv().is_err(),
            "no new text, no second report"
        );
        let states: Vec<String> = h
            .store
            .reader()
            .unwrap()
            .subagent_reports()
            .unwrap()
            .into_iter()
            .map(|report| report.state)
            .collect();
        assert_eq!(states, ["delivered", "superseded"]);
    }

    #[tokio::test]
    async fn a_result_queued_during_delivery_with_new_text_sends_a_second_report() {
        let mut h = Harness::new().await;
        h.result();
        finish_during_delivery(&h, Some("Follow-up answer."));
        h.app.deliver_subagent_reports().await;
        h.inputs.recv().await.expect("first report");
        h.app.deliver_subagent_reports().await;
        // The parent is mid-turn on the first report, so the second waits in its queue and
        // never reaches the fake driver; the outbox state shows it was accepted.
        let states: Vec<String> = h
            .store
            .reader()
            .unwrap()
            .subagent_reports()
            .unwrap()
            .into_iter()
            .map(|report| report.state)
            .collect();
        assert_eq!(states, ["delivered", "delivered"]);
    }

    #[tokio::test]
    async fn a_failed_parent_delivery_is_recorded_and_never_retried() {
        let mut h = Harness::new().await;
        h.store.set_archived("parent", true).unwrap();
        h.result();
        h.app.deliver_subagent_reports().await;
        let report = &h.store.reader().unwrap().subagent_reports().unwrap()[0];
        assert_eq!(report.state, "failed");
        assert!(
            report
                .error
                .as_deref()
                .is_some_and(|error| !error.is_empty())
        );
        let failed = h
            .store
            .reader()
            .unwrap()
            .events_since(&h.child, 0, 100)
            .unwrap()
            .into_iter()
            .any(|event| event.kind == "subagent_report_failed");
        assert!(failed, "the child history must surface the failed report");
        h.app.deliver_subagent_reports().await;
        assert!(h.inputs.try_recv().is_err());
        assert_eq!(
            h.store.reader().unwrap().subagent_reports().unwrap()[0].state,
            "failed"
        );
    }

    #[test]
    fn an_oversized_result_is_shortened_with_an_inspect_pointer() {
        let text = "x".repeat(boblm_tuning::MAX_MESSAGE_BYTES + 100);
        let content = report_content("child", "Worker", &text, &[], false, None, false);
        assert!(content.len() <= boblm_tuning::MAX_MESSAGE_BYTES);
        assert!(content.contains("inspect_peer"));
        assert!(content.contains("No reply is needed"));
    }

    const HINT: &str = "Optional: grade this review with grade_subagent (good, ok or poor).";

    fn set_child_role(h: &Harness, role: &str) {
        let db = rusqlite::Connection::open(h._temp.path().join("db.sqlite")).unwrap();
        db.execute("UPDATE sessions SET role = ?1", [role]).unwrap();
    }

    fn pending_result(h: &Harness) -> boblm_store::SubagentReport {
        h.result();
        h.store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .into_iter()
            .find(|r| r.source_kind == "result")
            .unwrap()
    }

    #[tokio::test]
    async fn only_a_reviewer_result_carries_the_grading_hint_while_grading_is_on() {
        for (role, expected) in [
            ("reviewer", true),
            ("implementer", false),
            ("planner", false),
        ] {
            let h = Harness::new().await;
            set_child_role(&h, role);
            let report = pending_result(&h);
            let (content, _) = build_report(&h.store, &report, true).unwrap();
            assert_eq!(content.contains(HINT), expected, "{role}: {content}");
            // Only reviewers are blind; the open list names the model of other roles.
            for name in ["sonnet", "haiku", "luna", "openai", "claude", "codex"]
                .into_iter()
                .filter(|_| role == "reviewer")
            {
                assert!(
                    !content.to_lowercase().contains(name),
                    "{name} in {content}"
                );
            }
        }
        let h = Harness::new().await;
        set_child_role(&h, "reviewer");
        let report = pending_result(&h);
        let (content, _) = build_report(&h.store, &report, false).unwrap();
        assert!(!content.contains("grade_subagent"));
    }

    #[tokio::test]
    async fn a_delivered_reviewer_report_has_the_hint_and_disabling_grading_removes_it() {
        for enabled in [true, false] {
            let mut h = Harness::new().await;
            set_child_role(&h, "reviewer");
            h.app.set_subagent_grading_enabled_for_tests(enabled);
            h.result();
            h.app.deliver_subagent_reports().await;
            let (_, content, _) = h.inputs.recv().await.expect("parent input");
            assert_eq!(content.contains(HINT), enabled, "{content}");
        }
    }

    #[test]
    fn error_reports_never_carry_the_grading_hint() {
        let content = report_content(
            "child",
            "Worker",
            "",
            &[],
            false,
            Some(("The turn failed.", "driver_error")),
            true,
        );
        assert!(!content.contains("grade_subagent"));
    }
}
