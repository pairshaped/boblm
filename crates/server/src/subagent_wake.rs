//! Supply the idle-turn wake-up that Codex's native subagent mailbox omits.
use crate::{
    app::App,
    subagent_history::{codex_meta, records},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct Completion {
    id: String,
    child: String,
    task: String,
    notified: bool,
    fallback_sent: bool,
}
struct NativeReceipt {
    task: String,
    notified: bool,
}

fn completion_message_id(parent: &str, completion: &str, stage: &str) -> String {
    let digest = Sha256::digest(format!(
        "boblm:subagent-completion:{parent}:{completion}:{stage}"
    ));
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

#[derive(Default)]
struct Parent {
    offset: u64,
    discarding: bool,
    seen: BTreeSet<String>,
    native_receipts: BTreeSet<String>,
    unconsumed_native: BTreeMap<String, NativeReceipt>,
    pending: VecDeque<Completion>,
}
impl Parent {
    fn record(&mut self, record: &Value, since: i64) {
        let payload = &record["payload"];
        if record["type"] == "response_item"
            && ((payload["type"] == "message" && payload["role"] == "assistant")
                || matches!(
                    payload["type"].as_str(),
                    Some("function_call" | "custom_tool_call")
                ))
        {
            // A native receipt can be appended after the model's final answer. Only
            // subsequent model output proves it has had a chance to consume that input.
            self.unconsumed_native.clear();
        }
        if record["type"] == "event_msg" && payload["type"] == "item_completed" {
            let item = &payload["item"];
            if item["type"] == "SubAgentActivity"
                && item["kind"] == "completed"
                && record["timestamp"]
                    .as_str()
                    .and_then(crate::schedule::parse_rfc3339)
                    .is_some_and(|time| time >= since)
                && let (Some(id), Some(child), Some(task)) = (
                    item["id"].as_str(),
                    item["agent_thread_id"].as_str(),
                    item["agent_path"].as_str(),
                )
                && self.seen.insert(id.into())
            {
                self.pending.push_back(Completion {
                    id: id.into(),
                    child: child.into(),
                    task: task.into(),
                    notified: false,
                    fallback_sent: false,
                });
            }
        } else if record["type"] == "response_item"
            && payload["type"] == "agent_message"
            && let Some(author) = payload["author"].as_str()
            && payload["content"].as_array().is_some_and(|blocks| {
                blocks.iter().any(|b| {
                    b["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("Message Type: FINAL_ANSWER\n"))
                })
            })
            && let Some(id) = payload["id"].as_str()
            && self.native_receipts.insert(id.into())
            && let Some(index) = self.pending.iter().position(|c| c.task == author)
        {
            self.pending.remove(index);
            self.unconsumed_native.insert(
                id.into(),
                NativeReceipt {
                    task: author.into(),
                    notified: false,
                },
            );
        }
    }

    fn read(&mut self, path: &Path, since: i64) -> io::Result<bool> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(boblm_tuning::SUBAGENT_TRANSCRIPT_READ_BYTES as u64)
            .read_to_end(&mut bytes)?;
        let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let mut start = 0;
        if self.discarding {
            if let Some(newline) = bytes.iter().position(|b| *b == b'\n') {
                start = newline + 1;
                self.discarding = false;
            } else {
                self.offset += bytes.len() as u64;
                return Ok(false);
            }
        }
        for line in bytes[start..end.max(start)].split(|b| *b == b'\n') {
            if let Ok(record) = serde_json::from_slice::<Value>(line) {
                self.record(&record, since);
            }
        }
        self.offset += end as u64;
        if end == 0 && bytes.len() == boblm_tuning::SUBAGENT_TRANSCRIPT_READ_BYTES {
            // Oversized provider metadata must not pin the cursor forever.
            self.discarding = true;
            self.offset += bytes.len() as u64;
        }
        // Never dispatch while unread or incomplete records could contain native delivery.
        Ok(bytes.len() < boblm_tuning::SUBAGENT_TRANSCRIPT_READ_BYTES && end == bytes.len())
    }
}

pub(crate) struct Observer {
    root: PathBuf,
    since: i64,
    parents: BTreeMap<String, Parent>,
}
impl Observer {
    pub(crate) fn new() -> Self {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
            });
        Self {
            root: home.join("sessions"),
            since: crate::schedule::now_ms(),
            parents: BTreeMap::new(),
        }
    }
    fn scan(&mut self, parent: &str) -> io::Result<Vec<(String, String, String)>> {
        let (path, _) = codex_meta(&self.root, parent)?;
        let state = self.parents.entry(parent.into()).or_default();
        if !state.read(&path, self.since)? {
            return Ok(Vec::new());
        }
        let mut ready = Vec::new();
        for completion in &state.pending {
            if completion.fallback_sent {
                continue;
            }
            let Ok((path, meta)) = codex_meta(&self.root, &completion.child) else {
                continue;
            };
            // Only direct children can report to this parent. Nested agents report to theirs.
            if meta["parent_thread_id"] != parent {
                continue;
            }
            let Some(turn) = completion.id.strip_prefix("subagent-completed-") else {
                continue;
            };
            let window = records(&path, None)?;
            if let Some(final_message) = window.rows.iter().rev().find_map(|(_, record)| {
                let payload = &record["payload"];
                (record["type"] == "event_msg"
                    && payload["type"] == "task_complete"
                    && payload["turn_id"] == turn)
                    .then(|| {
                        payload["last_agent_message"]
                            .as_str()
                            .unwrap_or("The subagent completed without a final message.")
                    })
            }) {
                let id = completion_message_id(
                    parent,
                    &completion.id,
                    if completion.notified {
                        "body"
                    } else {
                        "pointer"
                    },
                );
                let mut content = format!(
                    "Subagent task {} completed.\n\n{final_message}",
                    completion.task
                );
                if content.len() > boblm_tuning::MAX_MESSAGE_BYTES {
                    let suffix = "\n\nThe final message was shortened. Open the subagent transcript for the full result.";
                    let mut end = boblm_tuning::MAX_MESSAGE_BYTES - suffix.len();
                    while !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    content.truncate(end);
                    content.push_str(suffix);
                }
                if !completion.notified {
                    content = format!(
                        "Subagent {} finished; its result follows. Run `true` once to receive the queued message at a tool boundary, then act on it. If no result arrives, end your turn for the fallback.",
                        completion.task
                    );
                }
                ready.push((completion.id.clone(), id, content));
            }
        }
        for (receipt, native) in &state.unconsumed_native {
            if !native.notified {
                ready.push((receipt.clone(), completion_message_id(parent, receipt, "native"),
                    format!("Subagent {} finished. Its final result is now in your context; read it and continue.", native.task)));
            }
        }
        Ok(ready)
    }
}

impl App {
    pub(crate) async fn wake_completed_subagents(&self, observer: &mut Option<Observer>) {
        if self.external_runners {
            return;
        }
        let store = self.store.clone();
        let Ok(Ok(sessions)) =
            tokio::task::spawn_blocking(move || store.reader()?.sessions()).await
        else {
            return;
        };
        for session in sessions {
            if session.agent != "codex" || session.archived {
                continue;
            }
            let Some(parent) = session.agent_session_id else {
                continue;
            };
            let Ok(handle) = self.handle(&session.id).await else {
                continue;
            };
            wake_parent(&handle, parent, observer).await;
        }
    }
}

/// Lives with the runner, so idle-parent wakes continue while HTTP is restarting.
pub(crate) async fn wake_parent(
    handle: &boblm_runner::SessionHandle,
    parent: String,
    observer: &mut Option<Observer>,
) {
    let Ok(status) = handle.status().await else {
        return;
    };
    if !status.running {
        return;
    }
    let Some(mut state) = observer.take() else {
        return;
    };
    let lookup = parent.clone();
    let result = tokio::task::spawn_blocking(move || {
        let result = state.scan(&lookup);
        (state, result)
    })
    .await;
    let Ok((state, result)) = result else { return };
    *observer = Some(state);
    match result {
        Ok(ready) if !status.turn_active => {
            for (completion, id, content) in ready {
                match handle.send_subagent_result(&id, &content).await {
                    Ok(true) => {
                        if let Some(state) =
                            observer.as_mut().and_then(|o| o.parents.get_mut(&parent))
                            && let Some(completion) =
                                state.pending.iter_mut().find(|c| c.id == completion)
                        {
                            // Keep the receipt in FIFO order until Codex drains its mailbox.
                            if completion.notified {
                                completion.fallback_sent = true;
                            } else {
                                completion.notified = true;
                            }
                        } else if let Some(native) = observer
                            .as_mut()
                            .and_then(|o| o.parents.get_mut(&parent))
                            .and_then(|p| p.unconsumed_native.get_mut(&completion))
                        {
                            native.notified = true;
                        }
                    }
                    Ok(false) => break,
                    Err(error) => {
                        eprintln!(
                            "The subagent result could not be queued: {error}. The server will check its message receipt on the next pass."
                        );
                        break;
                    }
                }
            }
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "Subagent completions could not be checked: {error}. The server will check again on its next pass."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use boblm_protocol::{Event, LifecycleState, Priority};
    use boblm_runner::{Driver, DriverFactory};
    use boblm_store::{NewSession, Session, Store};
    use serde_json::json;
    use std::{
        fs::{self, OpenOptions},
        io::Write,
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
    struct Factory(Mutex<Option<Fake>>);
    #[async_trait]
    impl DriverFactory for Factory {
        async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
            Ok(Box::new(
                self.0.lock().unwrap().take().expect("one fake child"),
            ))
        }
    }
    struct Harness {
        _temp: tempfile::TempDir,
        app: Arc<App>,
        observer: Option<Observer>,
        parent: PathBuf,
        child: PathBuf,
        events: mpsc::UnboundedSender<Event>,
        inputs: mpsc::UnboundedReceiver<(String, String, Option<Priority>)>,
    }
    impl Harness {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("db.sqlite")).unwrap();
            store.add_project("p", "/fixture", "fixture", 0).unwrap();
            store
                .create_session_in(NewSession {
                    id: "s",
                    name: "",
                    project_id: "p",
                    agent: "codex",
                    model: "gpt-6-luna",
                    effort: Some("low"),
                    created_at: 0,
                })
                .unwrap();
            store.set_agent_session("s", "parent").unwrap();
            let (events, event_rx) = mpsc::unbounded_channel();
            let (input_tx, inputs) = mpsc::unbounded_channel();
            let app = App::open(
                store,
                Arc::new(Factory(Mutex::new(Some(Fake {
                    events: event_rx,
                    inputs: input_tx,
                })))),
                "subagent-fixture-token".into(),
            )
            .await
            .unwrap();
            let root = temp.path().join("sessions");
            fs::create_dir_all(&root).unwrap();
            let parent = root.join("rollout-parent.jsonl");
            let child = root.join("rollout-child.jsonl");
            fs::write(
                &parent,
                format!(
                    "{}\n",
                    json!({"type":"session_meta","payload":{"id":"parent"}})
                ),
            )
            .unwrap();
            fs::write(&child, format!("{}\n",json!({"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent"}}))).unwrap();
            Self {
                _temp: temp,
                app,
                observer: Some(Observer {
                    root,
                    since: 0,
                    parents: BTreeMap::new(),
                }),
                parent,
                child,
                events,
                inputs,
            }
        }
        fn append(path: &Path, record: Value) {
            writeln!(
                OpenOptions::new().append(true).open(path).unwrap(),
                "{record}"
            )
            .unwrap();
        }
        fn complete(&self, turn: &str, text: &str) {
            Self::append(
                &self.child,
                json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":text}}),
            );
            Self::append(
                &self.parent,
                json!({"timestamp":"2026-10-27T19:55:17.249Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"SubAgentActivity","id":format!("subagent-completed-{turn}"),"kind":"completed","agent_thread_id":"child","agent_path":"/root/wake_test"}}}),
            );
        }
        fn native(&self) {
            Self::append(
                &self.parent,
                json!({"type":"response_item","payload":{"type":"agent_message","id":"native-final-1","author":"/root/wake_test","recipient":"/root","content":[{"type":"input_text","text":"Message Type: FINAL_ANSWER\nTask name: /root\nSender: /root/wake_test\nPayload:\nOK"}]}}),
            );
        }
        async fn tick(&mut self) {
            tokio::time::advance(boblm_tuning::SCHEDULER_TICK).await;
            self.app.wake_completed_subagents(&mut self.observer).await;
            self.app.handle("s").await.unwrap().status().await.unwrap();
        }
        async fn begin(&mut self) {
            let handle = self.app.handle("s").await.unwrap();
            handle
                .send("00000000-0000-4000-8000-000000000001", "spawn a child")
                .await
                .unwrap();
            self.inputs.recv().await.unwrap();
            self.events
                .send(Event::CommandLifecycle {
                    command_uuid: "00000000-0000-4000-8000-000000000001".into(),
                    state: LifecycleState::Started,
                })
                .unwrap();
        }
        async fn idle(&mut self) {
            Self::append(
                &self.parent,
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"fake final answer"}]}}),
            );
            self.events
                .send(Event::Result {
                    subtype: Some("success".into()),
                    is_error: Some(false),
                })
                .unwrap();
            let handle = self.app.handle("s").await.unwrap();
            for _ in 0..1000 {
                if !handle.status().await.unwrap().turn_active {
                    return;
                }
                tokio::task::yield_now().await;
            }
            panic!("fake result was not consumed");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn subagent_completion_wakes_idle_parent_once_and_duplicate_records_do_not_redeliver() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.idle().await;
        h.complete("turn-child", "OK");
        h.tick().await;
        let input = h
            .inputs
            .try_recv()
            .expect("idle parent must receive the child's result");
        assert!(input.1.contains("/root/wake_test"));
        assert!(
            !input.1.contains("OK"),
            "wake pointer must not copy the result"
        );
        assert_eq!(input.2, None);
        h.native();
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: input.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await;
        h.complete("turn-child", "OK");
        h.tick().await;
        h.tick().await;
        assert!(
            h.inputs.try_recv().is_err(),
            "duplicate completion woke parent again"
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn subagent_completion_mid_turn_defers_to_native_delivery() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.complete("turn-child", "OK");
        h.tick().await;
        assert!(h.inputs.try_recv().is_err());
        h.native();
        h.idle().await;
        h.tick().await;
        h.complete("turn-child", "OK");
        h.tick().await;
        assert!(
            h.inputs.try_recv().is_err(),
            "native result was also sent as a wake"
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn subagent_completion_before_idle_without_native_delivery_is_woken_after_turn() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.complete("turn-child", "OK");
        h.tick().await;
        h.idle().await;
        h.tick().await;
        let pointer = h.inputs.try_recv().unwrap();
        assert!(!pointer.1.contains("OK"));
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: pointer.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await;
        h.tick().await;
        let fallback = h.inputs.try_recv().unwrap();
        assert!(fallback.1.ends_with("OK"));
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: fallback.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await;
        h.tick().await;
        assert!(
            h.inputs.try_recv().is_err(),
            "fallback result must be delivered only once"
        );
        h.app.shutdown().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn subagent_completion_rechecks_idle_in_runner_and_keeps_stopped_chats_stopped() {
        let mut h = Harness::new().await;
        h.begin().await;
        let handle = h.app.handle("s").await.unwrap();
        let id = "00000000-0000-4000-8000-000000000032";
        assert!(!handle.send_subagent_result(id, "result").await.unwrap());
        h.idle().await;
        assert!(handle.send_subagent_result(id, "result").await.unwrap());
        handle.status().await.unwrap();
        let input = h.inputs.try_recv().unwrap();
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: input.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await;
        assert!(handle.send_subagent_result(id, "result").await.unwrap());
        handle.status().await.unwrap();
        assert!(h.inputs.try_recv().is_err());
        handle.stop().await.unwrap();
        assert!(
            !handle
                .send_subagent_result("00000000-0000-4000-8000-000000000033", "another")
                .await
                .unwrap()
        );
        assert!(handle.status().await.unwrap().stopped);
        h.app.shutdown().await.unwrap();
    }

    #[test]
    fn subagent_completion_reader_handles_partial_and_oversized_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("records.jsonl");
        fs::write(
            &path,
            vec![b'x'; boblm_tuning::SUBAGENT_TRANSCRIPT_READ_BYTES + 20],
        )
        .unwrap();
        let mut state = Parent::default();
        assert!(!state.read(&path, 0).unwrap());
        assert!(!state.read(&path, 0).unwrap());
        Harness::append(&path, json!({"type":"unknown"}));
        assert!(state.read(&path, 0).unwrap());
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"unfinished\":")
            .unwrap();
        assert!(!state.read(&path, 0).unwrap());
    }
    #[tokio::test(start_paused = true)]
    async fn subagent_completion_native_receipt_after_fallback_does_not_hide_next_completion() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.idle().await;
        h.complete("first", "OK");
        h.tick().await;
        let input = h.inputs.try_recv().unwrap();
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: input.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.complete("second", "OK");
        h.native();
        h.idle().await;
        h.tick().await;
        assert!(
            h.inputs.try_recv().is_ok(),
            "first native receipt hid the second task's result"
        );
        h.app.shutdown().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn subagent_completion_duplicate_native_receipt_does_not_hide_next_completion() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.complete("first", "OK");
        h.native();
        h.complete("second", "OK");
        h.native(); // Same provider message ID, replayed after the next completion.
        h.idle().await;
        h.tick().await;
        assert!(
            h.inputs.try_recv().is_ok(),
            "replayed receipt hid a new completion"
        );
        h.app.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn subagent_completion_fallback_is_bounded_and_preserves_utf8() {
        let mut h = Harness::new().await;
        let text = "é".repeat(boblm_tuning::MAX_MESSAGE_BYTES);
        h.complete("large", &text);
        let observer = h.observer.as_mut().unwrap();
        let pointer = observer.scan("parent").unwrap();
        assert_eq!(pointer.len(), 1);
        assert!(!pointer[0].2.contains("é"));
        observer.parents.get_mut("parent").unwrap().pending[0].notified = true;
        let fallback = observer.scan("parent").unwrap();
        assert_ne!(
            pointer[0].1, fallback[0].1,
            "stages need distinct durable message IDs"
        );
        assert!(fallback[0].2.len() <= boblm_tuning::MAX_MESSAGE_BYTES);
        assert!(fallback[0].2.ends_with("the full result."));
        h.app.shutdown().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn subagent_completion_native_result_after_final_gets_a_pointer_without_copying_body() {
        let mut h = Harness::new().await;
        h.begin().await;
        h.idle().await;
        h.complete("first", "OK");
        h.tick().await;
        let pointer = h.inputs.try_recv().unwrap();
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: pointer.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await; // The model finishes before Codex appends its mailbox receipt.
        h.native();
        h.tick().await;
        let pointer = h
            .inputs
            .try_recv()
            .expect("late native input must get a consuming turn");
        assert!(pointer.1.contains("now in your context"));
        assert!(
            !pointer.1.contains("OK"),
            "never copy the full native result"
        );
        h.events
            .send(Event::CommandLifecycle {
                command_uuid: pointer.0,
                state: LifecycleState::Started,
            })
            .unwrap();
        h.idle().await;
        h.tick().await;
        assert!(h.inputs.try_recv().is_err());
        h.app.shutdown().await.unwrap();
    }
}
