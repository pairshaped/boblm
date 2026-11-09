use async_trait::async_trait;
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_runner::{Driver, DriverFactory, RunningSession};
use boblm_store::{NewSession, PeerSpawn, Session, Store};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
struct Input {
    uuid: String,
    content: String,
    priority: Option<Priority>,
    human: bool,
    model: String,
    effort: Option<String>,
}
struct Fake {
    model: String,
    effort: Option<String>,
    wait_for_tool_boundary: bool,
    wakes_itself_on_background_job_completion: bool,
    emit: mpsc::UnboundedSender<Event>,
    manual_stop: Arc<AtomicBool>,
    interrupts: Arc<AtomicUsize>,
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<Input>,
}
#[async_trait]
impl Driver for Fake {
    fn wakes_itself_on_background_job_completion(&self) -> bool {
        self.wakes_itself_on_background_job_completion
    }
    fn steers_wait_for_tool_boundary(&self) -> bool {
        self.wait_for_tool_boundary
    }
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        if content == "simulate broken pipe" {
            return Err(io::Error::other("test child closed its input pipe"));
        }
        self.inputs
            .send(Input {
                uuid: uuid.into(),
                content: content.into(),
                priority,
                human: false,
                model: self.model.clone(),
                effort: self.effort.clone(),
            })
            .map_err(io::Error::other)
    }
    async fn send_with_origin(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        human: bool,
    ) -> io::Result<()> {
        if content == "simulate broken pipe" {
            return Err(io::Error::other("test child closed its input pipe"));
        }
        self.inputs
            .send(Input {
                uuid: uuid.into(),
                content: content.into(),
                priority,
                human,
                model: self.model.clone(),
                effort: self.effort.clone(),
            })
            .map_err(io::Error::other)
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }
    async fn configure(&mut self, model: &str, effort: Option<&str>) -> io::Result<()> {
        self.model = model.to_owned();
        self.effort = effort.map(str::to_owned);
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        if !self.manual_stop.load(Ordering::SeqCst) {
            let _ = self.emit.send(Event::Result {
                subtype: Some("interrupted".into()),
                is_error: Some(false),
            });
        }
        Ok(())
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        self.events.close();
        Ok(())
    }
}
struct Factory {
    fake: Mutex<Option<Fake>>,
    account_id: Option<String>,
    spawn_delay: Mutex<Option<Duration>>,
}
#[async_trait]
impl DriverFactory for Factory {
    fn account_id(&self, _: &Session) -> Option<String> {
        self.account_id.clone()
    }
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        let delay = *self.spawn_delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        Ok(Box::new(self.fake.lock().unwrap().take().ok_or_else(
            || io::Error::other("unexpected second spawn"),
        )?))
    }
}
struct Harness {
    manual_stop: Arc<AtomicBool>,
    interrupts: Arc<AtomicUsize>,
    _temp: tempfile::TempDir,
    store: Store,
    factory: Arc<Factory>,
    events: mpsc::UnboundedSender<Event>,
    inputs: mpsc::UnboundedReceiver<Input>,
}
impl Harness {
    fn new() -> Self {
        Self::new_with_account(None)
    }
    fn new_with_account(account_id: Option<&str>) -> Self {
        Self::build(account_id, "claude")
    }
    fn new_with_agent(agent: &str) -> Self {
        Self::build(None, agent)
    }
    fn build(account_id: Option<&str>, agent: &str) -> Self {
        let temp =
            tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        if agent == "claude" {
            store.create_session("s", "/local/project", 1).unwrap();
        } else {
            store
                .add_project("project", "/local/project", "project", 1)
                .unwrap();
            store
                .create_session_in(NewSession {
                    name: "",
                    id: "s",
                    project_id: "project",
                    agent,
                    model: "deepseek-ai/DeepSeek-V4.1-Flash-1017",
                    effort: Some("high"),
                    created_at: 1,
                })
                .unwrap();
        }
        Self::from_store(temp, store, account_id, agent)
    }
    fn new_role(agent: &str, account_key: Option<&str>, step: &str) -> Self {
        let temp =
            tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        store.create_session("root", "/local/project", 1).unwrap();
        let project = store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.id == "root")
            .and_then(|session| session.project_id)
            .unwrap();
        let role = if step.starts_with("reviewer") {
            "reviewer"
        } else {
            "implementer"
        };
        let model = if agent == "pi" {
            "deepseek-ai/DeepSeek-V4.1-Flash-1017"
        } else if role == "reviewer" {
            "gpt-6-luna"
        } else {
            "gpt-6.1-sol"
        };
        let account = if step.contains("openai") {
            "openai-api"
        } else if step.contains("deepseek") {
            "deepseek"
        } else {
            "codex-plan"
        };
        let resolved = serde_json::json!({
            "role": role,
            "difficulty": "simple",
            "model": model,
            "effort": "high",
            "account": account,
            "step": step,
            "reason": "fixture",
        })
        .to_string();
        store
            .spawn_peer(PeerSpawn {
                request_id: "s".into(),
                caller: "root".into(),
                arguments: serde_json::json!({
                    "title": "Role chat",
                    "requestId": "s",
                    "role": role,
                    "difficulty": "simple",
                })
                .to_string(),
                project_id: project,
                agent: agent.into(),
                model: model.into(),
                effort: "high".into(),
                title: "Role chat".into(),
                content: "work".into(),
                created_at: now_ms(),
                parent_session_id: Some("root".into()),
                root_session_id: Some("root".into()),
                role: Some(role.into()),
                resolved: Some(resolved),
                working_directory: None,
                account_key: account_key.map(str::to_owned),
            })
            .unwrap();
        Self::from_store(temp, store, None, agent)
    }
    fn from_store(
        temp: tempfile::TempDir,
        store: Store,
        account_id: Option<&str>,
        agent: &str,
    ) -> Self {
        let (events, event_rx) = mpsc::unbounded_channel();
        let (input_tx, inputs) = mpsc::unbounded_channel();
        let manual_stop = Arc::new(AtomicBool::new(false));
        let interrupts = Arc::new(AtomicUsize::new(0));
        Self {
            manual_stop: manual_stop.clone(),
            interrupts: interrupts.clone(),
            _temp: temp,
            store,
            factory: Arc::new(Factory {
                fake: Mutex::new(Some(Fake {
                    model: "sonnet".into(),
                    effort: None,
                    wait_for_tool_boundary: matches!(agent, "pi" | "codex"),
                    wakes_itself_on_background_job_completion: agent == "claude",
                    emit: events.clone(),
                    manual_stop,
                    interrupts,
                    events: event_rx,
                    inputs: input_tx,
                })),
                account_id: account_id.map(str::to_owned),
                spawn_delay: Mutex::new(None),
            }),
            events,
            inputs,
        }
    }
    async fn start(&self) -> RunningSession {
        RunningSession::start(self.store.clone(), "s", self.factory.clone())
            .await
            .unwrap()
    }
    async fn input(&mut self) -> Input {
        // This watchdog bounds real SQLite/fsync work on a shared host. Ordering
        // assertions and the protocol's virtual-clock deadlines remain separate.
        tokio::time::timeout(Duration::from_secs(30), self.inputs.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn lifecycle(&self, uuid: &str, state: LifecycleState) {
        self.events
            .send(Event::CommandLifecycle {
                command_uuid: uuid.into(),
                state,
            })
            .unwrap();
    }
    fn result(&self) {
        self.events
            .send(Event::Result {
                subtype: Some("success".into()),
                is_error: Some(false),
            })
            .unwrap();
    }
}
const A: &str = "00000000-0000-4000-8000-000000000001";
const B: &str = "00000000-0000-4000-8000-000000000002";
const C: &str = "00000000-0000-4000-8000-000000000003";

#[tokio::test]
async fn quiet_runner_restart_releases_a_later_peer_message() {
    let mut h = Harness::new();
    let runner = h.start().await;
    runner.shutdown().await.unwrap();
    let replacement = h.start().await;
    let client = replacement.handle();
    assert!(!client.status().await.unwrap().queue_held);
    client
        .send_from_peer(A, "later peer work", "other")
        .await
        .unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0 && s.turn_active).await;
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn quiet_runner_restart_after_a_completed_turn_dispatches_peer_work() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "finish this").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    runner.shutdown().await.unwrap();
    let mut child = Harness::new();
    let replacement = RunningSession::start(h.store.clone(), "s", child.factory.clone())
        .await
        .unwrap();
    let client = replacement.handle();
    assert!(!client.status().await.unwrap().queue_held);
    client
        .send_from_peer(B, "peer after completed turn", "other")
        .await
        .unwrap();
    assert_eq!(child.input().await.uuid, B);
    child.lifecycle(B, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0 && s.turn_active).await;
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn runner_restart_during_a_consumed_active_turn_stays_held() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "still working").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    runner.shutdown().await.unwrap();
    let replacement = h.start().await;
    assert!(replacement.handle().status().await.unwrap().queue_held);
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn runner_restart_with_uncertain_or_dispatching_input_overrides_a_quiet_receipt() {
    for delivery in ["uncertain", "dispatching"] {
        let h = Harness::new();
        h.start().await.shutdown().await.unwrap();
        h.store
            .append(boblm_store::NewMessage {
                uuid: A,
                session: "s",
                role: "user",
                content: "unconfirmed",
                delivery_state: "queued",
                created_at: now_ms(),
            })
            .unwrap();
        h.store.transition(A, "queued", "dispatching").unwrap();
        if delivery == "uncertain" {
            h.store.transition(A, "dispatching", "uncertain").unwrap();
        }
        let replacement = h.start().await;
        let status = replacement.handle().status().await.unwrap();
        assert!(status.queue_held);
        assert_eq!((status.uncertain, status.dispatching), (1, 0));
        replacement.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn abrupt_runner_restart_cannot_reuse_a_consumed_quiet_receipt() {
    let h = Harness::new();
    h.start().await.shutdown().await.unwrap();
    let runner = h.start().await;
    assert!(!runner.handle().status().await.unwrap().queue_held);
    let client = runner.handle();
    drop(runner);
    while client.published_status().is_ok() {
        tokio::task::yield_now().await;
    }
    let replacement = h.start().await;
    assert!(replacement.handle().status().await.unwrap().queue_held);
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn runner_restart_without_a_quiet_receipt_stays_held() {
    let h = Harness::new();
    let runner = h.start().await;
    assert!(runner.handle().status().await.unwrap().queue_held);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn runner_restart_with_jobs_or_unknown_inventory_stays_held() {
    for available in [true, false] {
        let mut h = Harness::new();
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "start a job").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        h.events
            .send(Event::BackgroundJobs {
                snapshot: boblm_protocol::jobs::JobSnapshot {
                    available,
                    tasks: if available {
                        vec![boblm_protocol::jobs::BackgroundJob {
                            id: "job".into(),
                            command: "work".into(),
                            agent_key: None,
                        }]
                    } else {
                        vec![]
                    },
                },
            })
            .unwrap();
        h.result();
        wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
        runner.shutdown().await.unwrap();
        let replacement = h.start().await;
        assert!(replacement.handle().status().await.unwrap().queue_held);
        replacement.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn accepts_once_and_steers_two_clients_in_order() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    assert!(!client.status().await.unwrap().running);
    assert!(client.send(A, "first").await.unwrap());
    let first = h.input().await;
    assert_eq!(first.uuid, A);
    assert_eq!(first.content, "first");
    assert_eq!(first.priority, None);
    h.lifecycle(A, LifecycleState::Started);
    let (one, two) = tokio::join!(client.send(B, "one"), client.send(C, "two"));
    assert!(one.unwrap() && two.unwrap());
    let one = h.input().await;
    let two = h.input().await;
    assert_eq!((one.uuid.as_str(), two.uuid.as_str()), (B, C));
    assert_eq!(one.priority, Some(Priority::Now));
    assert_eq!(two.priority, Some(Priority::Now));
    assert!(!client.send(B, "one").await.unwrap());
    assert!(client.send(B, "changed").await.is_err());
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

async fn wait_agent_session(store: &Store, expected: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let stored = store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .into_iter()
                .find(|session| session.id == "s")
                .and_then(|session| session.agent_session_id);
            if stored.as_deref() == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the child's init event must be recorded");
}

async fn wait_status(
    client: &boblm_runner::SessionHandle,
    predicate: impl Fn(&boblm_runner::Status) -> bool,
) -> boblm_runner::Status {
    let mut updates = client.status_updates();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = updates.borrow().clone();
            if predicate(&status) {
                return status;
            }
            if updates.changed().await.is_err() {
                panic!("the runner stopped publishing status before the expected state");
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn restart_holds_uncertain_and_queued_until_an_owner_message() {
    let mut h = Harness::new();
    for (uuid, content) in [(A, "lost ack"), (B, "queued")] {
        h.store
            .append(boblm_store::NewMessage {
                uuid,
                session: "s",
                role: "user",
                content,
                delivery_state: "queued",
                created_at: now_ms(),
            })
            .unwrap();
    }
    h.store.transition(A, "queued", "dispatching").unwrap();
    h.store.set_agent_session("s", "resume-id").unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    let status = client.status().await.unwrap();
    assert!(!status.running);
    assert_eq!((status.queued, status.uncertain), (1, 1));
    assert!(h.inputs.try_recv().is_err());
    client.send(C, "owner is back").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    assert_eq!(h.input().await.uuid, C);
    assert!(h.inputs.try_recv().is_err());
    assert_eq!(client.status().await.unwrap().uncertain, 1);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn slash_waits_through_cutover_and_consumed_cancellation_never_requeues() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "steer").await.unwrap();
    h.input().await;
    client.send(C, "/compact").await.unwrap();
    h.result();
    h.lifecycle(A, LifecycleState::Cancelled);
    wait_status(&client, |s| !s.turn_active).await;
    assert!(h.inputs.try_recv().is_err());
    h.lifecycle(B, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    assert!(h.inputs.try_recv().is_err());
    h.result();
    let slash = h.input().await;
    assert_eq!(slash.uuid, C);
    assert_eq!(slash.priority, None);
    assert_eq!(client.status().await.unwrap().queued, 0);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn lone_completed_after_result_releases_waiting_slash() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "steer").await.unwrap();
    h.input().await;
    client.send(C, "/compact").await.unwrap();
    h.result();
    h.lifecycle(B, LifecycleState::Completed);
    assert_eq!(h.input().await.uuid, C);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejection_retries_at_idle_twice_then_holds_and_explicit_retry_resets_limit() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "steer").await.unwrap();
    h.input().await;
    h.lifecycle(B, LifecycleState::Refused);
    wait_status(&client, |s| s.queued == 1).await;
    assert!(h.inputs.try_recv().is_err());
    h.result();
    let retry = h.input().await;
    assert_eq!((retry.uuid.as_str(), retry.priority), (B, None));
    h.lifecycle(B, LifecycleState::Cancelled);
    assert_eq!(h.input().await.uuid, B);
    h.lifecycle(B, LifecycleState::Discarded);
    wait_status(&client, |s| s.rejected == 1).await;
    assert!(h.inputs.try_recv().is_err());
    let events = client.events_since(0, 1024).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "delivery_rejected"));
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(B)
            .unwrap()
            .unwrap()
            .retry_count,
        2
    );
    client.retry(B).await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(B)
            .unwrap()
            .unwrap()
            .retry_count,
        0
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn post_result_deadline_is_thirty_seconds_and_later_results_do_not_extend_it() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    h.result();
    wait_status(&client, |s| !s.turn_active).await;
    tokio::time::advance(Duration::from_secs(29)).await;
    h.result();
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(Duration::from_millis(1001)).await;
    let status = wait_status(&client, |s| s.uncertain == 1).await;
    assert!(!status.running);
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn initial_deadline_is_sixty_seconds_and_result_cannot_extend_it() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(50)).await;
    h.result();
    wait_status(&client, |s| !s.turn_active).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(Duration::from_millis(1001)).await;
    wait_status(&client, |s| s.uncertain == 1).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_holds_queue_and_archive_reopen_preserves_session_without_execution() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    client.send(B, "/compact").await.unwrap();
    client.stop().await.unwrap();
    let status = client.status().await.unwrap();
    assert_eq!((status.uncertain, status.queued), (1, 1));
    assert!(h.inputs.try_recv().is_err());
    client.archive(true).await.unwrap();
    let status = client.status().await.unwrap();
    assert!(status.archived && !status.running);
    client.archive(false).await.unwrap();
    assert!(!client.status().await.unwrap().running);
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

struct ChildFactory;
#[async_trait]
impl DriverFactory for ChildFactory {
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut command = tokio::process::Command::new("python3");
        command
            .arg("-u")
            .arg(root.join("../claude-driver/tests/fake_child.py"));
        if let Some(id) = &session.agent_session_id {
            command.env("EXPECT_RESUME", id);
        }
        let session = boblm_claude_driver::ClaudeSession::spawn_command(
            command,
            root,
            session.agent_session_id.as_deref(),
        )
        .await?;
        Ok(Box::new(boblm_runner::ClaudeDriver(Some(session))))
    }
}

#[tokio::test]
async fn fake_child_persists_init_and_resumes_after_archive_reopen() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let client = runner.handle();
    client.send(A, "started").await.unwrap();
    wait_agent_session(&h.store, "fake-session").await;
    client.archive(true).await.unwrap();
    assert!(client.status().await.unwrap().archived);
    client.send(B, "result-then-completed").await.unwrap();
    assert!(!client.status().await.unwrap().archived);
    wait_status(&client, |s| s.dispatching == 0 && !s.turn_active).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn fake_child_exit_makes_pending_uncertain_and_does_not_replay_queued_slash() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let client = runner.handle();
    client.send(A, "hang").await.unwrap();
    client.send(B, "/compact").await.unwrap();
    client.send(C, "exit").await.unwrap();
    let status = wait_status(&client, |s| s.uncertain == 2).await;
    assert!(!status.running);
    assert_eq!(status.queued, 1);
    runner.shutdown().await.unwrap();
}

async fn wait_child_event(
    client: &boblm_runner::SessionHandle,
    predicate: impl Fn(&serde_json::Value) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if client
                .events_since(0, 1024)
                .await
                .unwrap()
                .iter()
                .any(|event| {
                    event.kind == "agent_event"
                        && predicate(&serde_json::from_str(&event.payload).unwrap())
                })
            {
                // Recording precedes applying the batch. Cross the actor barrier too.
                client.status().await.unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn fake_child_self_woken_background_job_does_not_queue_a_duplicate_wake() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let client = runner.handle();
    client.send(A, "runner-self-woken-job").await.unwrap();
    wait_status(&client, |s| {
        s.jobs
            .as_ref()
            .is_some_and(|j| j.available && j.tasks.is_empty())
            && s.dispatching == 0
    })
    .await;
    let events = client.events_since(0, 1024).await.unwrap();
    let appended: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "message_appended")
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "only the original prompt belongs in the queue"
    );
    assert!(events.iter().any(|e| e.kind == "agent_event"
        && e.payload.contains("own-job")
        && e.payload.contains("background_jobs")));
    let status = client.status().await.unwrap();
    assert_eq!(
        (status.queued, status.dispatching, status.uncertain),
        (0, 0, 0)
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn fake_child_queued_steer_survives_a_tool_call_longer_than_the_ack_deadline() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let client = runner.handle();
    client.send(A, "runner-tool").await.unwrap();
    wait_child_event(&client, |e| e["message"]["content"][0]["id"] == "long-tool").await;
    client.send(B, "runner-queued-steer").await.unwrap();
    wait_child_event(&client, |e| {
        e["command_uuid"] == B && e["state"] == "queued"
    })
    .await;
    tokio::time::pause();
    tokio::time::advance(boblm_tuning::ACK_DEADLINE * 3).await;
    let status = client.status().await.unwrap();
    assert!(status.running && !status.queue_held);
    assert_eq!((status.dispatching, status.uncertain), (1, 0));
    assert_eq!(
        delivery_state(&h, B),
        "dispatching",
        "queued is not consumption"
    );
    tokio::time::resume();
    client.send(C, "runner-release-steer").await.unwrap();
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    assert_eq!(delivery_state(&h, B), "started");
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    let events = client.events_since(0, 1024).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| {
                let value: serde_json::Value = serde_json::from_str(&e.payload).unwrap();
                value["message"]["received_uuid"] == B
            })
            .count(),
        1,
        "the queued input is never replayed"
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn fake_child_unacknowledged_steer_still_times_out_during_a_tool_call() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let client = runner.handle();
    client.send(A, "runner-tool").await.unwrap();
    wait_child_event(&client, |e| e["message"]["content"][0]["id"] == "long-tool").await;
    client.send(B, "runner-unacknowledged-steer").await.unwrap();
    wait_child_event(&client, |e| e["message"]["received_uuid"] == B).await;
    tokio::time::pause();
    tokio::time::advance(boblm_tuning::ACK_DEADLINE + Duration::from_secs(1)).await;
    tokio::time::resume();
    let status = wait_status(&client, |s| s.uncertain == 1 && !s.running).await;
    assert!(status.queue_held);
    assert_eq!(delivery_state(&h, B), "uncertain");
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_ack_is_not_final_and_new_input_waits_for_result() {
    let mut h = Harness::new();
    h.manual_stop.store(true, Ordering::SeqCst);
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "running Bash").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    let mut stop = tokio::spawn({
        let client = client.clone();
        async move { client.stop().await }
    });
    h.events
        .send(Event::ControlResponse {
            response: serde_json::json!({"request_id":"interrupt", "subtype":"success"}),
        })
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut stop)
            .await
            .is_err()
    );
    let send = tokio::spawn({
        let client = client.clone();
        async move { client.send(B, "next").await }
    });
    assert!(client.status().await.unwrap().stopping);
    assert_eq!(client.status().await.unwrap().queued, 1);
    assert!(h.inputs.try_recv().is_err());
    h.result();
    stop.await.unwrap().unwrap();
    send.await.unwrap().unwrap();
    assert_eq!(h.input().await.uuid, B);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn result_before_interrupt_ack_completes_stop_without_waiting_for_ack() {
    let mut h = Harness::new();
    h.manual_stop.store(true, Ordering::SeqCst);
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "running Bash").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    let stop = tokio::spawn({
        let client = client.clone();
        async move { client.stop().await }
    });
    wait_status(&client, |s| s.stopping).await;
    h.result();
    tokio::time::timeout(Duration::from_secs(1), stop)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    h.events
        .send(Event::ControlResponse {
            response: serde_json::json!({"request_id":"interrupt", "subtype":"success"}),
        })
        .unwrap();
    assert!(client.status().await.unwrap().running);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_without_final_result_kills_after_bounded_wait() {
    let mut h = Harness::new();
    h.manual_stop.store(true, Ordering::SeqCst);
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "running Bash").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    let stop = tokio::spawn({
        let client = client.clone();
        async move { client.stop().await }
    });
    wait_status(&client, |s| s.stopping).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_millis(30001)).await;
    stop.await.unwrap().unwrap();
    assert!(!client.status().await.unwrap().running);
    assert!(
        client
            .events_since(0, 1024)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "stop_timeout")
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn slow_reader_and_reconnect_recover_every_event_once_by_cursor() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "one").await.unwrap();
    h.input().await;
    let first = client.events_since(0, 2).await.unwrap();
    let cursor = first.last().unwrap().seq;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "two").await.unwrap();
    h.input().await;
    h.lifecycle(B, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    runner.shutdown().await.unwrap();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(ChildFactory))
        .await
        .unwrap();
    let mut received = first.iter().map(|e| e.seq).collect::<Vec<_>>();
    let mut after = cursor;
    loop {
        let page = runner.handle().events_since(after, 2).await.unwrap();
        if page.is_empty() {
            break;
        }
        after = page.last().unwrap().seq;
        received.extend(page.iter().map(|e| e.seq));
    }
    let expected = h
        .store
        .reader()
        .unwrap()
        .events_since("s", 0, 1024)
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect::<Vec<_>>();
    assert_eq!(received, expected);
    assert_eq!(received, (1..=after).collect::<Vec<_>>());
    assert!(!runner.handle().status().await.unwrap().running);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn cursor_pages_are_repeatable_and_do_not_start_sessions() {
    use boblm_protocol::client::Sequence;
    use boblm_runner::events::cursor_page;
    let h = Harness::new();
    let runner = h.start().await;
    let handle = runner.handle();
    let first = cursor_page(&handle, "s", Sequence(0), 2).await.unwrap();
    let again = cursor_page(&handle, "s", Sequence(0), 2).await.unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
    let empty = cursor_page(&handle, "s", first.cursor, 2).await.unwrap();
    assert!(empty.events.is_empty());
    assert_eq!(empty.cursor, first.cursor);
    assert!(!empty.status.running);
    assert!(cursor_page(&handle, "s", Sequence(0), 0).await.is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn model_and_effort_change_between_turns_and_survive_stop() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "one").await.unwrap();
    let first = h.input().await;
    assert_eq!((first.model.as_str(), first.effort), ("sonnet", None));
    h.lifecycle(A, LifecycleState::Completed);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    client.set_model("opus", Some("high")).await.unwrap();
    client.send(B, "two").await.unwrap();
    let second = h.input().await;
    assert_eq!(
        (second.model.as_str(), second.effort),
        ("opus", Some("high".to_string()))
    );
    client.stop().await.unwrap();
    assert_eq!(client.status().await.unwrap().uncertain, 1);
    assert_eq!(client.status().await.unwrap().model, "opus");
    assert_eq!(
        client.status().await.unwrap().effort.as_deref(),
        Some("high")
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_holds_unacknowledged_steer_and_never_replays_it() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "unacknowledged steer").await.unwrap();
    assert_eq!(h.input().await.priority, Some(Priority::Now));
    client.send(C, "/compact").await.unwrap();
    client.stop().await.unwrap();
    let status = client.status().await.unwrap();
    assert_eq!(
        (status.dispatching, status.uncertain, status.queued),
        (0, 1, 1)
    );
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(A)
            .unwrap()
            .unwrap()
            .delivery_state,
        "started"
    );
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(B)
            .unwrap()
            .unwrap()
            .delivery_state,
        "uncertain"
    );
    assert!(client.retry(B).await.is_err());
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_device_stop_holds_input_accepted_during_the_first_stop() {
    let mut h = Harness::new();
    h.manual_stop.store(true, Ordering::SeqCst);
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "running").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    let stop_one = tokio::spawn({
        let client = client.clone();
        async move { client.stop().await }
    });
    wait_status(&client, |s| s.stopping).await;
    client.send(B, "next").await.unwrap();
    let stop_two = tokio::spawn({
        let client = client.clone();
        async move { client.stop().await }
    });
    wait_status(&client, |s| s.queue_held).await;
    h.result();
    stop_one.await.unwrap().unwrap();
    stop_two.await.unwrap().unwrap();
    assert_eq!(client.status().await.unwrap().queued, 1);
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn active_selection_is_pending_while_steers_keep_the_current_configuration() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "one").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.set_model("opus", Some("high")).await.unwrap();
    assert!(client.status().await.unwrap().selection_pending);
    client.send(B, "steer immediately").await.unwrap();
    let steer = h.input().await;
    assert_eq!(
        (steer.priority, steer.model.as_str(), steer.effort),
        (Some(Priority::Now), "sonnet", None)
    );
    h.lifecycle(B, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    client.send(C, "next turn").await.unwrap();
    let next = h.input().await;
    assert_eq!(
        (next.priority, next.model.as_str(), next.effort),
        (None, "opus", Some("high".to_string()))
    );
    assert!(!client.status().await.unwrap().selection_pending);
    let summaries = client
        .events_since(0, 1024)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "state_summary")
        .map(|e| serde_json::from_str::<serde_json::Value>(&e.payload).unwrap())
        .collect::<Vec<_>>();
    assert!(
        summaries
            .iter()
            .any(|s| s["selectionPending"] == true && s["model"] == "opus")
    );
    assert_eq!(summaries.last().unwrap()["selectionPending"], false);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn identical_retry_of_an_accepted_message_succeeds_on_an_archived_session() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    assert!(client.send(A, "one").await.unwrap());
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.archive(true).await.unwrap();
    assert!(client.status().await.unwrap().archived);
    assert!(!client.send(A, "one").await.unwrap());
    assert!(client.send(A, "changed").await.is_err());
    assert!(client.status().await.unwrap().archived);
    assert_eq!(client.status().await.unwrap().queued, 0);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn message_uuids_are_canonicalized_before_dedupe_and_storage() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    let upper = "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
    let lower = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    assert!(client.send(upper, "once").await.unwrap());
    assert_eq!(h.input().await.uuid, lower);
    for same in [
        lower.to_owned(),
        "aaaaaaaabbbb4ccc8dddeeeeeeeeeeee".to_owned(),
        format!("{{{upper}}}"),
        format!("urn:uuid:{upper}"),
    ] {
        assert!(!client.send(&same, "once").await.unwrap(), "{same}");
    }
    assert!(client.send(lower, "changed").await.is_err());
    assert!(client.send("not-a-uuid", "x").await.is_err());
    assert!(h.inputs.try_recv().is_err());
    let stored = h
        .store
        .reader()
        .unwrap()
        .messages("s", 1, i64::MAX)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].uuid, lower);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn unapplied_selection_survives_disconnect_and_restart() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "one").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.set_model("opus", Some("high")).await.unwrap();
    assert!(client.status().await.unwrap().selection_pending);
    client.archive(true).await.unwrap();
    let status = client.status().await.unwrap();
    assert!(!status.running);
    assert!(status.selection_pending, "disconnect must not clear it");
    runner.shutdown().await.unwrap();
    let restarted = h.start().await;
    let status = restarted.handle().status().await.unwrap();
    assert!(!status.running);
    assert!(status.selection_pending, "restart must not clear it");
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn late_started_for_an_uncertain_message_keeps_slash_commands_waiting() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client.send(B, "steer").await.unwrap();
    h.input().await;
    // Stop makes the unacknowledged steer uncertain; the fake interrupts the turn.
    client.stop().await.unwrap();
    assert_eq!(client.status().await.unwrap().uncertain, 1);
    assert!(!client.status().await.unwrap().turn_active);
    // Claude had in fact consumed it and begins a turn.
    h.lifecycle(B, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active).await;
    client.send(C, "/compact").await.unwrap();
    client.status().await.unwrap();
    assert!(
        h.inputs.try_recv().is_err(),
        "slash command dispatched mid-turn"
    );
    h.result();
    assert_eq!(h.input().await.uuid, C);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_follow_up_written_after_a_result_cannot_outlive_the_thirty_second_window() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    h.result();
    wait_status(&client, |s| !s.turn_active).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    client.send(B, "follow-up").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    tokio::time::advance(Duration::from_secs(19)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(Duration::from_millis(1001)).await;
    let status = wait_status(&client, |s| s.uncertain == 2).await;
    assert!(!status.running);
    runner.shutdown().await.unwrap();
}

fn upload(h: &Harness, id: &str, name: &str, content_type: &str, size: i64) {
    h.store
        .create_attachment(boblm_store::NewAttachment {
            id,
            session: "s",
            name,
            content_type,
            size,
            sha256: "0000000000000000000000000000000000000000000000000000000000000001",
            path: &format!("/uploads/{id}-file"),
            created_at: now_ms(),
        })
        .unwrap();
    h.store.mark_attachment_stored(id).unwrap();
}
const U1: &str = "10000000-0000-4000-8000-000000000001";
const U2: &str = "10000000-0000-4000-8000-000000000002";

#[tokio::test]
async fn attachments_are_listed_in_the_dispatched_text_and_steers_carry_them() {
    let mut h = Harness::new();
    upload(&h, U1, "Screen \"shot\".png", "image/png", 380_928);
    upload(&h, U2, "notes.txt", "text/plain", 12);
    let runner = h.start().await;
    let client = runner.handle();
    assert!(
        client
            .send_with_attachments(A, "look at this", &[U1.into()])
            .await
            .unwrap()
    );
    let first = h.input().await;
    assert_eq!(
        first.content,
        format!(
            "look at this\n\nAttached files:\n- /uploads/{U1}-file (image/png, 372 KiB, original name \"Screen \\\"shot\\\".png\")"
        )
    );
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    // Attachments alone are enough, and a steer carries them too.
    assert!(
        client
            .send_with_attachments(B, "", &[U2.into()])
            .await
            .unwrap()
    );
    let steer = h.input().await;
    assert_eq!(steer.priority, Some(Priority::Now));
    assert_eq!(
        steer.content,
        format!(
            "Attached files:\n- /uploads/{U2}-file (text/plain, 12 B, original name \"notes.txt\")"
        )
    );
    // The stored message keeps exactly what the owner typed.
    let stored = h.store.reader().unwrap().message(A).unwrap().unwrap();
    assert_eq!(stored.content, "look at this");
    // Identical retry is the same message; a different list conflicts.
    assert!(
        !client
            .send_with_attachments(A, "look at this", &[U1.into()])
            .await
            .unwrap()
    );
    assert!(
        client
            .send_with_attachments(A, "look at this", &[])
            .await
            .is_err()
    );
    assert!(
        client
            .send_with_attachments(C, "x", &[U1.into()])
            .await
            .is_err()
    );
    assert!(client.send_with_attachments(C, "", &[]).await.is_err());
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

fn init(id: &str) -> Event {
    Event::System {
        subtype: Some("init".into()),
        session_id: Some(id.into()),
        status: None,
    }
}

#[tokio::test]
async fn clear_waits_for_idle_and_a_new_conversation_id_is_recorded_once() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "remember this").await.unwrap();
    h.input().await;
    h.events.send(init("first")).unwrap();
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    // /clear is a slash command: it waits for the running turn instead of steering it.
    client.send(B, "/clear").await.unwrap();
    client.status().await.unwrap();
    assert!(h.inputs.try_recv().is_err(), "/clear must not steer");
    h.events.send(init("first")).unwrap();
    h.result();
    let clear = h.input().await;
    assert_eq!((clear.uuid.as_str(), clear.priority), (B, None));
    h.events.send(init("second")).unwrap();
    h.events.send(init("second")).unwrap();
    h.lifecycle(B, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    let changes: Vec<serde_json::Value> = client
        .events_since(0, 1024)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "agent_session_changed")
        .map(|e| serde_json::from_str(&e.payload).unwrap())
        .collect();
    assert_eq!(
        changes,
        [
            serde_json::json!({"agent_session_id":"first","previous":null}),
            serde_json::json!({"agent_session_id":"second","previous":"first"}),
        ]
    );
    assert_eq!(
        h.store.reader().unwrap().sessions().unwrap()[0]
            .agent_session_id
            .as_deref(),
        Some("second"),
        "the next resume continues the cleared conversation"
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn retry_lookup_rejects_consumed_input_and_other_chats() {
    let h = Harness::new();
    h.store.create_session("other", "/fake", 1).unwrap();
    let consumed = "00000000-0000-4000-8000-000000000091";
    let foreign = "00000000-0000-4000-8000-000000000092";
    for (uuid, session, state) in [(consumed, "s", "started"), (foreign, "other", "rejected")] {
        h.store
            .append(boblm_store::NewMessage {
                uuid,
                session,
                role: "user",
                content: "Do not resend",
                delivery_state: state,
                created_at: now_ms(),
            })
            .unwrap();
    }
    let runner = h.start().await;
    let client = runner.handle();
    let error = client.retry(consumed).await.unwrap_err().to_string();
    assert!(error.contains("already taken"), "{error}");
    let error = client.retry(foreign).await.unwrap_err().to_string();
    assert!(error.contains("no message"), "{error}");
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(foreign)
            .unwrap()
            .unwrap()
            .delivery_state,
        "rejected"
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn retry_targets_the_exact_spelling_the_browser_names() {
    let mut h = Harness::new();
    let upper = "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
    let lower = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    // Two rejected messages that differ only in case, as an old database could hold.
    for (uuid, content) in [(upper, "upper twin"), (lower, "lower twin")] {
        h.store
            .append(boblm_store::NewMessage {
                uuid,
                session: "s",
                role: "user",
                content,
                delivery_state: "rejected",
                created_at: now_ms(),
            })
            .unwrap();
    }
    let runner = h.start().await;
    let client = runner.handle();
    client.retry(upper).await.unwrap();
    assert_eq!(
        h.input().await.uuid,
        upper,
        "the uppercase twin is the one retried"
    );
    assert_eq!(
        client.status().await.unwrap().rejected,
        1,
        "the lowercase twin is untouched"
    );
    runner.shutdown().await.unwrap();
}

struct IdleDriver;
#[async_trait]
impl Driver for IdleDriver {
    async fn send(&mut self, _: &str, _: &str, _: Option<Priority>) -> io::Result<()> {
        Ok(())
    }
    async fn next_event(&mut self) -> Option<Event> {
        std::future::pending().await
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
struct CaptureResume(Arc<Mutex<Vec<Option<String>>>>);
#[async_trait]
impl DriverFactory for CaptureResume {
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
        self.0
            .lock()
            .unwrap()
            .push(session.agent_session_id.clone());
        Ok(Box::new(IdleDriver))
    }
}

#[tokio::test]
async fn after_a_clear_a_restart_resumes_the_new_conversation() {
    let h = Harness::new();
    h.store.set_agent_session("s", "before-clear").unwrap();
    h.store.set_agent_session("s", "after-clear").unwrap();
    let launched = Arc::new(Mutex::new(Vec::new()));
    let runner = RunningSession::start(
        h.store.clone(),
        "s",
        Arc::new(CaptureResume(launched.clone())),
    )
    .await
    .unwrap();
    runner.handle().send(A, "carry on").await.unwrap();
    wait_status(&runner.handle(), |s| s.running).await;
    assert_eq!(
        *launched.lock().unwrap(),
        [Some("after-clear".to_string())],
        "the new process resumes the conversation Claude reported last"
    );
    runner.shutdown().await.unwrap();
}

const SCHEDULED: &str = "Scheduled prompt (every 30 minutes):\n\nkeep going";

#[tokio::test]
async fn a_scheduled_prompt_waits_for_idle_instead_of_steering() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    client.send(B, SCHEDULED).await.unwrap();
    client.status().await.unwrap();
    assert!(
        h.inputs.try_recv().is_err(),
        "a scheduled prompt must not steer a running turn"
    );
    h.result();
    let nudge = h.input().await;
    assert_eq!((nudge.uuid.as_str(), nudge.priority), (B, None));
    assert_eq!(nudge.content, SCHEDULED);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_scheduled_prompt_does_not_release_a_held_queue() {
    let mut h = Harness::new();
    h.store
        .append(boblm_store::NewMessage {
            uuid: A,
            session: "s",
            role: "user",
            content: "queued before a restart",
            delivery_state: "queued",
            created_at: now_ms(),
        })
        .unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    assert!(client.status().await.unwrap().queue_held);
    client.send(B, SCHEDULED).await.unwrap();
    let status = client.status().await.unwrap();
    assert!(status.queue_held, "only the owner releases a held queue");
    assert_eq!(status.queued, 2);
    assert!(h.inputs.try_recv().is_err());
    // An ordinary message from the owner releases everything, in order.
    client.send(C, "go ahead").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_scheduled_prompt_wakes_an_idle_chat_whose_hold_is_only_from_a_restart() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    let status = client.status().await.unwrap();
    assert!(
        status.queue_held && status.queued == 0,
        "a fresh start holds an empty queue"
    );
    client.send(B, SCHEDULED).await.unwrap();
    let nudge = h.input().await;
    assert_eq!((nudge.uuid.as_str(), nudge.priority), (B, None));
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_scheduled_prompt_does_not_wake_a_chat_the_owner_stopped() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.turn_active && s.dispatching == 0).await;
    client.stop().await.unwrap();
    assert!(client.status().await.unwrap().stopped);
    client.send(B, SCHEDULED).await.unwrap();
    client.status().await.unwrap();
    assert!(
        h.inputs.try_recv().is_err(),
        "the owner's Stop is respected"
    );
    assert_eq!(client.status().await.unwrap().queued, 1);
    // The owner speaking again lifts it, and the nudge follows the owner's message.
    client.send(C, "carry on").await.unwrap();
    assert!(!client.status().await.unwrap().stopped);
    assert_eq!(h.input().await.uuid, B);
    runner.shutdown().await.unwrap();
}

struct NeverStarts;
#[async_trait]
impl DriverFactory for NeverStarts {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Err(io::Error::other("the binary is missing"))
    }
}

#[tokio::test]
async fn a_codex_chat_that_cannot_start_names_codex_not_claude() {
    let h = Harness::new();
    h.store
        .add_project(
            "50000000-0000-4000-8000-000000000001",
            "/local/codex",
            "codex",
            1,
        )
        .unwrap();
    h.store
        .create_session_in(boblm_store::NewSession {
            name: "",
            id: "x",
            project_id: "50000000-0000-4000-8000-000000000001",
            agent: "codex",
            model: "gpt-6-luna",
            effort: None,
            created_at: 2,
        })
        .unwrap();
    let runner = RunningSession::start(h.store.clone(), "x", Arc::new(NeverStarts))
        .await
        .unwrap();
    runner.handle().send(A, "hello").await.unwrap();
    wait_status(&runner.handle(), |s| s.queue_held && !s.running).await;
    let events = runner.handle().events_since(0, 100).await.unwrap();
    let message = events
        .iter()
        .find(|e| e.kind == "driver_error")
        .map(|e| {
            let payload: serde_json::Value = serde_json::from_str(&e.payload).unwrap();
            payload["message"].as_str().unwrap_or_default().to_owned()
        })
        .unwrap_or_default();
    assert!(
        message.starts_with("Codex could not be started"),
        "{message}"
    );
    assert!(!message.contains("Claude"), "{message}");
    assert!(message.contains("the binary is missing"), "{message}");
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failed_send_records_its_cause_and_holds_uncertain_input() {
    let h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "simulate broken pipe").await.unwrap();
    wait_status(&client, |s| s.uncertain == 1 && s.queue_held && !s.running).await;
    let events = client.events_since(0, 100).await.unwrap();
    let error = events
        .iter()
        .find(|e| e.kind == "driver_error")
        .expect("the send failure must be recorded");
    assert!(
        error.payload.contains("test child closed its input pipe"),
        "{}",
        error.payload
    );
    assert!(error.payload.contains(A), "{}", error.payload);
    runner.shutdown().await.unwrap();
}

fn compaction(subtype: &str) -> Event {
    Event::System {
        subtype: Some(subtype.into()),
        session_id: None,
        status: None,
    }
}

async fn wait_compaction_record(client: &boblm_runner::SessionHandle, marker: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if client
                .events_since(0, 100)
                .await
                .unwrap()
                .iter()
                .any(|event| event.kind == "agent_event" && event.payload.contains(marker))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("compaction event must be processed");
}

fn context(used_tokens: u64, window_tokens: u64) -> Event {
    Event::Usage {
        five_hour: None,
        weekly: None,
        credits: None,
        context: Some(boblm_protocol::usage::ContextUsage {
            used_tokens,
            window_tokens,
        }),
    }
}

fn result(subtype: &str, is_error: bool) -> Event {
    Event::Result {
        subtype: Some(subtype.into()),
        is_error: Some(is_error),
    }
}

#[tokio::test]
async fn pi_auto_compacts_at_the_threshold_before_queued_input() {
    let mut h = Harness::new_with_agent("pi");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    client
        .send(B, "Scheduled prompt: after compaction")
        .await
        .unwrap();
    assert_eq!(client.status().await.unwrap().queued, 1);
    h.events.send(context(99_000, 100_000)).unwrap();
    h.events.send(result("success", false)).unwrap();
    let compact = h.input().await;
    assert_eq!(compact.content, "/compact");
    assert_ne!(compact.uuid, B);
    h.lifecycle(&compact.uuid, LifecycleState::Started);
    h.events.send(compaction("compact_started")).unwrap();
    h.events.send(compaction("compact_boundary")).unwrap();
    h.events.send(result("success", false)).unwrap();
    assert_eq!(h.input().await.uuid, B);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn pi_does_not_auto_compact_below_the_threshold() {
    let mut h = Harness::new_with_agent("pi");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    h.events.send(context(97_000, 100_000)).unwrap();
    h.events.send(result("success", false)).unwrap();
    wait_status(&client, |status| {
        !status.turn_active && status.dispatching == 0 && status.queued == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn pi_overflow_compacts_then_resumes_once() {
    let mut h = Harness::new_with_agent("pi");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    h.events.send(result("context_overflow", true)).unwrap();
    let compact = h.input().await;
    assert_eq!(compact.content, "/compact");
    h.lifecycle(&compact.uuid, LifecycleState::Started);
    h.events.send(compaction("compact_started")).unwrap();
    h.events.send(compaction("compact_boundary")).unwrap();
    h.events.send(result("success", false)).unwrap();
    let resume = h.input().await;
    assert!(
        resume.content.starts_with("boblm: ") && resume.content.contains("continue"),
        "{}",
        resume.content
    );
    h.lifecycle(&resume.uuid, LifecycleState::Started);
    h.events.send(result("context_overflow", true)).unwrap();
    wait_status(&client, |status| {
        !status.turn_active && status.dispatching == 0 && status.queued == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        h.inputs.try_recv().is_err(),
        "a second overflow must not compact or resume again"
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn pi_auto_compaction_does_not_repeat_for_the_same_crossing() {
    let mut h = Harness::new_with_agent("pi");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    h.events.send(context(99_000, 100_000)).unwrap();
    h.events.send(result("success", false)).unwrap();
    let compact = h.input().await;
    assert_eq!(compact.content, "/compact");
    h.lifecycle(&compact.uuid, LifecycleState::Started);
    h.events.send(compaction("compact_started")).unwrap();
    h.events.send(compaction("compact_boundary")).unwrap();
    h.events.send(result("success", false)).unwrap();
    h.events.send(result("success", false)).unwrap();
    wait_status(&client, |status| {
        !status.turn_active && status.dispatching == 0 && status.queued == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn compaction_blocks_owner_and_peer_steering_until_boundary() {
    for status_signal in [false, true] {
        let mut h = Harness::new();
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "first").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        let start = if status_signal {
            Event::System {
                subtype: Some("status".into()),
                session_id: None,
                status: Some("compacting".into()),
            }
        } else {
            compaction("compact_started")
        };
        h.events.send(start).unwrap();
        wait_compaction_record(
            &client,
            if status_signal {
                "compacting"
            } else {
                "compact_started"
            },
        )
        .await;
        client.send(B, "owner follow-up").await.unwrap();
        client
            .send_from_peer(C, "peer follow-up", "other")
            .await
            .unwrap();
        let status = client.status().await.unwrap();
        assert_eq!(status.queued, 2, "messages must queue while compacting");
        assert_eq!(status.dispatching, 0);
        assert!(
            h.inputs.try_recv().is_err(),
            "compaction must not be steered"
        );
        h.events
            .send(if status_signal {
                Event::System {
                    subtype: Some("status".into()),
                    session_id: None,
                    status: None,
                }
            } else {
                compaction("compact_boundary")
            })
            .unwrap();
        assert_eq!(h.input().await.uuid, B);
        assert_eq!(h.input().await.uuid, C);
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn manual_compaction_blocks_messages_before_its_started_event() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "/compact").await.unwrap();
    h.input().await;
    client.send(B, "do not interrupt compaction").await.unwrap();
    assert_eq!(client.status().await.unwrap().queued, 1);
    assert!(h.inputs.try_recv().is_err());
    h.lifecycle(A, LifecycleState::Completed);
    h.result();
    let next = h.input().await;
    assert_eq!(next.uuid, B);
    assert_eq!(next.priority, None);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn ordinary_turn_start_does_not_block_steering() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events.send(compaction("turn_started")).unwrap();
    wait_compaction_record(&client, "turn_started").await;
    client.send(B, "steer normally").await.unwrap();
    assert_eq!(h.input().await.priority, Some(Priority::Now));
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn refusing_manual_compaction_releases_its_dispatch_guard() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "/compact").await.unwrap();
    h.input().await;
    client.send(B, "after compaction refusal").await.unwrap();
    for _ in 0..2 {
        h.lifecycle(A, LifecycleState::Refused);
        assert_eq!(h.input().await.uuid, A);
    }
    h.lifecycle(A, LifecycleState::Refused);
    assert_eq!(h.input().await.uuid, B);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn ready_compaction_marker_wins_over_new_input() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    for _ in 0..20 {
        let id = uuid::Uuid::new_v4().to_string();
        h.events.send(compaction("compact_started")).unwrap();
        // Both the event and this command can be ready for the actor's selection.
        client.send(&id, "do not race compaction").await.unwrap();
        assert_eq!(client.status().await.unwrap().queued, 1);
        assert!(h.inputs.try_recv().is_err());
        h.events.send(compaction("compact_boundary")).unwrap();
        assert_eq!(h.input().await.uuid, id);
        h.lifecycle(&id, LifecycleState::Started);
        wait_status(&client, |s| s.dispatching == 0).await;
    }
    runner.shutdown().await.unwrap();
}

/// Let the runner consume queued events before the clock moves, so a deadline cannot race them.
async fn settle(client: &boblm_runner::SessionHandle) {
    for _ in 0..8 {
        client.status().await.unwrap();
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn compaction_in_progress_extends_the_acknowledgment_deadline() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(50)).await;
    h.events.send(compaction("compact_started")).unwrap();
    settle(&client).await;
    // Well past the plain 60 second window, still inside the compaction window.
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    // A compaction that never ends is still caught.
    tokio::time::advance(boblm_tuning::ACK_DEADLINE_DURING_COMPACTION).await;
    wait_status(&client, |s| s.uncertain == 1).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn silence_after_compaction_ends_gets_the_ordinary_window_again() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    h.events.send(compaction("compact_started")).unwrap();
    settle(&client).await;
    tokio::time::advance(Duration::from_secs(100)).await;
    h.events.send(compaction("compact_boundary")).unwrap();
    settle(&client).await;
    tokio::time::advance(Duration::from_secs(59)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(Duration::from_millis(1001)).await;
    wait_status(&client, |s| s.uncertain == 1).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_started_turn_with_no_acknowledgment_yet_gets_the_extended_window() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    tokio::time::pause();
    h.events.send(compaction("turn_started")).unwrap();
    settle(&client).await;
    // Provider-side compaction can keep a started turn silent for minutes.
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(boblm_tuning::ACK_DEADLINE_DURING_COMPACTION).await;
    wait_status(&client, |s| s.uncertain == 1).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_started_turn_does_not_count_as_delivery() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "pending").await.unwrap();
    h.input().await;
    h.events.send(compaction("turn_started")).unwrap();
    settle(&client).await;
    let message = h.store.reader().unwrap().message(A).unwrap().unwrap();
    assert_eq!(message.delivery_state, "dispatching");
    runner.shutdown().await.unwrap();
}

fn tool_started(id: &str) -> Event {
    Event::Assistant {
        message: serde_json::json!({
            "id": format!("assistant-{id}"),
            "content": [{"type": "tool_use", "id": id, "name": "bash", "input": {}}]
        }),
    }
}

fn tool_finished(id: &str) -> Event {
    Event::User {
        message: serde_json::json!({
            "content": [{"type": "tool_result", "tool_use_id": id, "content": "done", "is_error": false}]
        }),
    }
}

#[tokio::test]
async fn pi_and_codex_steers_pause_the_ack_clock_during_a_tool_call() {
    for agent in ["pi", "codex"] {
        let mut h = Harness::new_with_agent(agent);
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "first").await.unwrap();
        assert_eq!(h.input().await.uuid, A);
        h.lifecycle(A, LifecycleState::Started);
        client.send(B, "steer").await.unwrap();
        let steer = h.input().await;
        assert_eq!(steer.uuid, B);
        assert_eq!(steer.priority, Some(Priority::Now));
        tokio::time::pause();
        h.events.send(tool_started("call-1")).unwrap();
        settle(&client).await;
        tokio::time::advance(boblm_tuning::ACK_DEADLINE + Duration::from_secs(120)).await;
        let status = client.status().await.unwrap();
        assert_eq!(status.uncertain, 0, "{agent}");
        assert!(status.running, "{agent}");
        assert_eq!(status.dispatching, 1, "{agent}");
        h.events.send(tool_finished("call-1")).unwrap();
        h.lifecycle(B, LifecycleState::Started);
        settle(&client).await;
        tokio::time::advance(Duration::from_secs(120)).await;
        assert_eq!(client.status().await.unwrap().uncertain, 0, "{agent}");
        tokio::time::resume();
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn pi_steer_during_model_streaming_keeps_the_ordinary_deadline() {
    let mut h = Harness::new_with_agent("pi");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    client.send(B, "steer").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    tokio::time::pause();
    settle(&client).await;
    tokio::time::advance(boblm_tuning::ACK_DEADLINE + Duration::from_secs(1)).await;
    wait_status(&client, |status| status.uncertain == 1 && !status.running).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn pi_and_codex_steers_at_turn_end_become_the_next_turns_input() {
    for agent in ["pi", "codex"] {
        let mut h = Harness::new_with_agent(agent);
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "first").await.unwrap();
        assert_eq!(h.input().await.uuid, A);
        h.lifecycle(A, LifecycleState::Started);
        client.send(B, "steer at the end").await.unwrap();
        assert_eq!(h.input().await.uuid, B);
        h.result();
        settle(&client).await;
        h.lifecycle(B, LifecycleState::Started);
        settle(&client).await;
        tokio::time::pause();
        tokio::time::advance(boblm_tuning::ACK_DEADLINE + Duration::from_secs(120)).await;
        let status = client.status().await.unwrap();
        assert_eq!(status.uncertain, 0, "{agent}");
        assert!(status.running, "{agent}");
        tokio::time::resume();
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn claude_steers_do_not_pause_for_a_tool_call() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    client.send(B, "steer").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    tokio::time::pause();
    h.events.send(tool_started("call-1")).unwrap();
    settle(&client).await;
    tokio::time::advance(boblm_tuning::ACK_DEADLINE + Duration::from_secs(1)).await;
    wait_status(&client, |status| status.uncertain == 1 && !status.running).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_queued_steer_restarts_its_deadline_when_the_last_tool_result_arrives() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    client.send(B, "steer").await.unwrap();
    h.input().await;
    h.lifecycle(B, LifecycleState::Queued);
    h.events.send(tool_started("call-1")).unwrap();
    h.events.send(tool_started("call-2")).unwrap();
    settle(&client).await;
    tokio::time::pause();
    tokio::time::advance(boblm_tuning::ACK_DEADLINE * 3).await;
    h.events.send(tool_finished("call-1")).unwrap();
    settle(&client).await;
    tokio::time::advance(boblm_tuning::ACK_DEADLINE * 3).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    h.events.send(tool_finished("call-2")).unwrap();
    settle(&client).await;
    tokio::time::advance(boblm_tuning::ACK_DEADLINE - Duration::from_secs(1)).await;
    assert_eq!(client.status().await.unwrap().uncertain, 0);
    tokio::time::advance(Duration::from_secs(2)).await;
    wait_status(&client, |s| s.uncertain == 1 && !s.running).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_queued_steer_at_turn_end_gets_a_bounded_consumption_window() {
    for consume in [false, true] {
        let mut h = Harness::new();
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "first").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        client.send(B, "steer").await.unwrap();
        h.input().await;
        h.lifecycle(B, LifecycleState::Queued);
        h.events.send(tool_started("call-1")).unwrap();
        settle(&client).await;
        tokio::time::pause();
        tokio::time::advance(boblm_tuning::ACK_DEADLINE * 3).await;
        // A turn can end without a separate tool-result event.
        h.result();
        settle(&client).await;
        tokio::time::advance(boblm_tuning::ACK_DEADLINE_AFTER_RESULT - Duration::from_secs(1))
            .await;
        assert_eq!(client.status().await.unwrap().uncertain, 0);
        if consume {
            h.lifecycle(B, LifecycleState::Completed);
            settle(&client).await;
            assert_eq!(delivery_state(&h, B), "started");
        } else {
            h.result();
            settle(&client).await;
        }
        tokio::time::advance(Duration::from_secs(2)).await;
        if consume {
            assert_eq!(client.status().await.unwrap().uncertain, 0);
        } else {
            wait_status(&client, |s| s.uncertain == 1 && !s.running).await;
        }
        tokio::time::resume();
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn background_jobs_are_durable_status_after_a_turn_and_clear_on_disconnect() {
    use boblm_protocol::jobs::{BackgroundJob, JobSnapshot};
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "run tests").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![BackgroundJob {
                    id: "job".into(),
                    command: "cargo test".into(),
                    agent_key: Some("worker".into()),
                }],
            },
        })
        .unwrap();
    h.result();
    wait_status(&client, |s| {
        !s.turn_active && s.dispatching == 0 && s.jobs.as_ref().is_some_and(|j| j.tasks.len() == 1)
    })
    .await;
    // The same snapshot used by browser session lists is persisted for bin/restart.
    let summaries = client
        .events_since(0, 1024)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "state_summary")
        .map(|event| serde_json::from_str::<serde_json::Value>(&event.payload).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        summaries.last().unwrap()["jobs"]["tasks"][0]["command"],
        "cargo test"
    );
    runner.shutdown().await.unwrap();
    let latest = h
        .store
        .reader()
        .unwrap()
        .events_since("s", 0, 1024)
        .unwrap()
        .into_iter()
        .rfind(|event| event.kind == "state_summary")
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&latest.payload).unwrap();
    assert!(value["jobs"].is_null());
}

#[tokio::test]
async fn background_job_completion_wakes_idle_parent_once() {
    use boblm_protocol::jobs::{BackgroundJob, JobSnapshot};
    let mut h = Harness::new_with_agent("codex");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "start a long test job").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![BackgroundJob {
                    id: "thread:123".into(),
                    command: "cargo test".into(),
                    agent_key: None,
                }],
            },
        })
        .unwrap();
    h.result();
    wait_status(&client, |s| {
        !s.turn_active && s.dispatching == 0 && s.jobs.as_ref().is_some_and(|j| j.tasks.len() == 1)
    })
    .await;
    let empty = Event::BackgroundJobs {
        snapshot: JobSnapshot {
            available: true,
            tasks: vec![],
        },
    };
    h.events.send(empty.clone()).unwrap();
    let wake = h.input().await;
    assert_eq!(wake.priority, None);
    assert!(wake.content.contains("cargo test"));
    assert!(wake.content.contains("no longer listed as running"));
    assert!(!wake.content.contains("succeeded"));
    h.lifecycle(&wake.uuid, LifecycleState::Started);
    h.result();
    h.events.send(empty).unwrap();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    assert!(h.inputs.try_recv().is_err());
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(&wake.uuid)
            .unwrap()
            .unwrap()
            .delivery_state,
        "started"
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn background_job_completion_steers_active_parent_after_inventory_recovers() {
    use boblm_protocol::jobs::{BackgroundJob, JobSnapshot};
    let mut h = Harness::new_with_agent("codex");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "work while the job runs").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    let jobs = vec![
        BackgroundJob {
            id: "own".into(),
            command: "tests".into(),
            agent_key: None,
        },
        BackgroundJob {
            id: "child".into(),
            command: "child tests".into(),
            agent_key: Some("worker".into()),
        },
    ];
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: jobs.clone(),
            },
        })
        .unwrap();
    wait_status(&client, |s| {
        s.dispatching == 0 && s.jobs.as_ref().is_some_and(|j| j.tasks.len() == 2)
    })
    .await;
    tokio::time::pause();
    // Malformed/failed scans must not manufacture completion notifications.
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: false,
                tasks: vec![],
            },
        })
        .unwrap();
    wait_status(&client, |s| {
        s.jobs
            .as_ref()
            .is_some_and(|j| !j.available && j.tasks.len() == 2)
    })
    .await;
    assert!(h.inputs.try_recv().is_err());
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![],
            },
        })
        .unwrap();
    let wake = h.input().await;
    assert_eq!(wake.priority, Some(Priority::Now));
    assert!(wake.content.contains("tests"));
    assert!(!wake.content.contains("child tests"));
    h.lifecycle(&wake.uuid, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn background_job_completion_respects_stop_until_owner_resumes() {
    use boblm_protocol::jobs::{BackgroundJob, JobSnapshot};
    let mut h = Harness::new_with_agent("codex");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "start tests").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![BackgroundJob {
                    id: "job".into(),
                    command: "tests".into(),
                    agent_key: None,
                }],
            },
        })
        .unwrap();
    h.result();
    wait_status(&client, |s| {
        !s.turn_active && s.dispatching == 0 && s.jobs.as_ref().is_some_and(|j| j.tasks.len() == 1)
    })
    .await;
    client.stop().await.unwrap();
    h.events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![],
            },
        })
        .unwrap();
    let status = wait_status(&client, |s| s.queued == 1).await;
    assert!(status.queue_held);
    assert!(!status.turn_active);
    assert!(h.inputs.try_recv().is_err());
    client.resume().await.unwrap();
    let wake = h.input().await;
    assert!(wake.content.contains("no longer listed as running"));
    h.lifecycle(&wake.uuid, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn persistent_runner_survives_client_shutdown_and_deduplicates_reconnection() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let owner = runner.handle();
    let socket = h._temp.path().join("runner.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let host = tokio::spawn(boblm_runner::transport::serve(listener, owner.clone()));
    let proxy = RunningSession::attach(h.store.clone(), "s", socket.clone(), None)
        .await
        .unwrap();
    let client = proxy.handle();
    assert!(client.send(A, "work through a deploy").await.unwrap());
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&owner, |s| s.turn_active && s.dispatching == 0).await;
    proxy.shutdown().await.unwrap();
    assert!(owner.status().await.unwrap().turn_active);
    h.result();
    wait_status(&owner, |s| !s.turn_active).await;
    let proxy = RunningSession::attach(h.store.clone(), "s", socket.clone(), None)
        .await
        .unwrap();
    let client = proxy.handle();
    assert!(!client.send(A, "work through a deploy").await.unwrap());
    assert!(h.inputs.try_recv().is_err());
    assert!(matches!(
        client.send(A, "conflicting retry").await,
        Err(boblm_runner::Error::Store(
            boblm_store::Error::MessageConflict
        ))
    ));
    client.send(B, "new work").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    h.lifecycle(B, LifecycleState::Started);
    wait_status(&owner, |s| s.dispatching == 0).await;
    client.stop().await.unwrap();
    proxy.shutdown().await.unwrap();
    let proxy = RunningSession::attach(h.store.clone(), "s", socket.clone(), None)
        .await
        .unwrap();
    let state = proxy.handle().status().await.unwrap();
    assert!(state.stopped && state.queue_held && !state.turn_active);
    assert!(
        RunningSession::attach(h.store.clone(), "another-chat", socket, None)
            .await
            .is_err()
    );
    proxy.shutdown().await.unwrap();
    runner.shutdown().await.unwrap();
    host.await.unwrap().unwrap();
}

#[tokio::test]
async fn persistent_runner_lost_reply_never_replays_an_accepted_command() {
    use tokio::io::AsyncWriteExt;
    let mut h = Harness::new();
    let runner = h.start().await;
    let owner = runner.handle();
    let socket = h._temp.path().join("runner.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let host = tokio::spawn(boblm_runner::transport::serve(listener, owner.clone()));
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version":boblm_tuning::RUNNER_PROTOCOL_VERSION,"session":"s",
        "action":{"Send":[A,"accepted before disconnect",[],null]}
    }))
    .unwrap();
    let mut connection = tokio::net::UnixStream::connect(&socket).await.unwrap();
    connection.write_u32(bytes.len() as u32).await.unwrap();
    connection.write_all(&bytes).await.unwrap();
    drop(connection);
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    h.result();
    wait_status(&owner, |s| !s.turn_active && s.dispatching == 0).await;
    let proxy = RunningSession::attach(h.store.clone(), "s", socket, None)
        .await
        .unwrap();
    assert!(
        !proxy
            .handle()
            .send(A, "accepted before disconnect")
            .await
            .unwrap()
    );
    assert!(h.inputs.try_recv().is_err());
    proxy.shutdown().await.unwrap();
    runner.shutdown().await.unwrap();
    host.await.unwrap().unwrap();
}

#[tokio::test]
async fn persistent_runner_recovery_keeps_an_explicit_stop_and_holds_schedules() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    client.stop().await.unwrap();
    runner.shutdown().await.unwrap();
    let recovered = h.start().await;
    let client = recovered.handle();
    assert!(client.status().await.unwrap().stopped);
    client
        .send(B, "Scheduled prompt: leave this stopped chat alone")
        .await
        .unwrap();
    let state = client.status().await.unwrap();
    assert!(state.stopped && state.queue_held);
    assert_eq!(state.queued, 1);
    assert!(!state.running);
    assert!(h.inputs.try_recv().is_err());
    recovered.shutdown().await.unwrap();
}

#[tokio::test]
async fn persistent_runner_unavailable_proxy_recovers_without_replaying_failed_input() {
    let mut h = Harness::new();
    let socket = h._temp.path().join("offline.sock");
    let proxy = RunningSession::attach_unavailable(h.store.clone(), "s", socket.clone(), None)
        .await
        .unwrap();
    assert!(
        !proxy
            .handle()
            .published_status()
            .unwrap()
            .jobs
            .unwrap()
            .available
    );
    assert!(
        proxy
            .handle()
            .send(A, "only on explicit retry")
            .await
            .is_err()
    );
    assert!(h.store.reader().unwrap().message(A).unwrap().is_none());
    let runner = h.start().await;
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let host = tokio::spawn(boblm_runner::transport::serve(listener, runner.handle()));
    assert!(!proxy.handle().status().await.unwrap().running);
    assert!(h.inputs.try_recv().is_err());
    assert!(
        proxy
            .handle()
            .send(A, "only on explicit retry")
            .await
            .unwrap()
    );
    assert_eq!(h.input().await.uuid, A);
    h.lifecycle(A, LifecycleState::Started);
    h.result();
    wait_status(&runner.handle(), |s| !s.turn_active && s.dispatching == 0).await;
    proxy.shutdown().await.unwrap();
    runner.shutdown().await.unwrap();
    host.await.unwrap().unwrap();
}

#[tokio::test]
async fn claude_steering_origin_is_durable_and_automated_input_uses_next() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "owner starts").await.unwrap();
    let first = h.input().await;
    assert!(first.human);
    assert_eq!(first.priority, None);
    h.lifecycle(A, LifecycleState::Started);
    client.send(B, "owner changes approach").await.unwrap();
    let owner = h.input().await;
    assert!(owner.human);
    assert_eq!(owner.priority, Some(Priority::Now));
    h.lifecycle(B, LifecycleState::Started);
    client
        .send_from_peer(C, "peer report", "other")
        .await
        .unwrap();
    let peer = h.input().await;
    assert!(!peer.human);
    assert_eq!(peer.priority, Some(Priority::Next));
    h.lifecycle(C, LifecycleState::Started);
    let notice = "00000000-0000-4000-8000-000000000004";
    client
        .send_automated(notice, "automated answer notice")
        .await
        .unwrap();
    let automatic = h.input().await;
    assert!(!automatic.human);
    assert_eq!(automatic.priority, Some(Priority::Next));
    h.lifecycle(notice, LifecycleState::Started);
    // A retry through a different ingress cannot rewrite the original provenance.
    assert!(
        !client
            .send_automated(B, "owner changes approach")
            .await
            .unwrap()
    );
    assert!(
        !client
            .send(notice, "automated answer notice")
            .await
            .unwrap()
    );
    let reader = h.store.reader().unwrap();
    assert!(reader.human_input(B).unwrap());
    assert!(!reader.human_input(notice).unwrap());
    assert!(!reader.human_input(C).unwrap());
    assert!(!reader.human_input("missing").unwrap());
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_steering_next_waits_for_tools_but_still_requires_consumption() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "long foreground work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    wait_status(&client, |s| s.dispatching == 0).await;
    client
        .send_automated(B, "wait for next boundary")
        .await
        .unwrap();
    assert_eq!(h.input().await.priority, Some(Priority::Next));
    h.lifecycle(B, LifecycleState::Queued);
    // Wait for the persisted queue acknowledgment before advancing virtual time.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            client.status().await.unwrap();
            if h.store
                .reader()
                .unwrap()
                .events_since("s", 0, 1000)
                .unwrap()
                .iter()
                .any(|e| {
                    let payload: serde_json::Value = serde_json::from_str(&e.payload).unwrap();
                    e.kind == "agent_event"
                        && payload["type"] == "command_lifecycle"
                        && payload["state"] == "queued"
                        && payload["command_uuid"] == B
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(boblm_tuning::ACK_DEADLINE * 3).await;
    let state = client.status().await.unwrap();
    assert!(state.running && state.turn_active);
    assert_eq!(state.dispatching, 1);
    h.result();
    wait_status(&client, |s| !s.turn_active).await;
    tokio::time::advance(boblm_tuning::ACK_DEADLINE_AFTER_RESULT - Duration::from_secs(1)).await;
    assert!(client.status().await.unwrap().running);
    tokio::time::advance(Duration::from_secs(2)).await;
    wait_status(&client, |s| s.uncertain == 1 && !s.running).await;
    runner.shutdown().await.unwrap();
}

struct CountingLauncher(Arc<std::sync::atomic::AtomicUsize>);
#[async_trait]
impl boblm_runner::transport::RunnerLauncher for CountingLauncher {
    async fn launch(&self, _: &str) -> std::io::Result<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn persistent_runner_proxy_restarts_a_missing_runner_once_per_interval() {
    let h = Harness::new();
    let socket = h._temp.path().join("gone.sock");
    let launches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let proxy = RunningSession::attach_unavailable(
        h.store.clone(),
        "s",
        socket,
        Some(Arc::new(CountingLauncher(launches.clone()))),
    )
    .await
    .unwrap();
    tokio::time::sleep(boblm_tuning::RUNNER_POLL_INTERVAL * 3).await;
    assert_eq!(launches.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(proxy);
}

#[tokio::test]
async fn fake_child_usage_updates_the_account_and_fans_out_to_an_idle_chat() {
    let mut h = Harness::new_with_account(Some("account-1"));
    h.store.create_session("idle", "/local/project", 1).unwrap();
    h.store
        .set_session_account("idle", Some("account-1"))
        .unwrap();
    h.store
        .publish_summary("idle", serde_json::json!({"turnActive":false}))
        .unwrap();
    let before = h
        .store
        .reader()
        .unwrap()
        .events_since("idle", 0, 1024)
        .unwrap()
        .len();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "collect usage").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(Event::Usage {
            five_hour: Some(boblm_protocol::usage::AllowanceUsage {
                used_percent: 80.0,
                resets_at: Some(2_000_000_000),
            }),
            weekly: Some(boblm_protocol::usage::AllowanceUsage {
                used_percent: 42.0,
                resets_at: Some(2_000_100_000),
            }),
            credits: None,
            context: None,
        })
        .unwrap();
    h.result();
    wait_status(&client, |s| s.account_usage.is_some()).await;
    let row = h
        .store
        .reader()
        .unwrap()
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap();
    assert_eq!(row.five_hour.as_ref().unwrap().used_percent, 80.0);
    assert_eq!(row.weekly.as_ref().unwrap().used_percent, 42.0);
    assert_eq!(
        client
            .status()
            .await
            .unwrap()
            .account_usage
            .unwrap()
            .weekly
            .unwrap()
            .used_percent,
        42.0
    );
    let events = h
        .store
        .reader()
        .unwrap()
        .events_since("idle", before as i64, 1024)
        .unwrap();
    let summary = events
        .iter()
        .find(|event| event.kind == "state_summary")
        .expect("the idle chat on the account must receive the change");
    let payload: serde_json::Value = serde_json::from_str(&summary.payload).unwrap();
    assert_eq!(payload["accountUsage"]["accountId"], "account-1");
    assert_eq!(payload["accountUsage"]["weekly"]["usedPercent"], 42.0);
    runner.shutdown().await.unwrap();
}

fn now_seconds() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

async fn role_marks(store: &Store) -> Vec<boblm_tuning::ExhaustedStep> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let marks = store.reader().unwrap().role_exhaustions().unwrap();
            if !marks.is_empty() {
                return marks;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn provider_error(text: &str) -> Event {
    Event::Assistant {
        message: serde_json::json!({
            "id": "provider-error",
            "content": [{"type": "text", "text": text}]
        }),
    }
}

#[tokio::test]
async fn pi_quota_rejection_marks_the_deepseek_step_until_the_retry_interval() {
    let mut h = Harness::new_role("pi", None, "implementer-deepseek");
    let runner = h.start().await;
    runner.handle().resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    let before = now_seconds();
    h.events
        .send(provider_error(
            "Pi stopped this turn with an error: 402 Payment Required: insufficient balance.",
        ))
        .unwrap();
    h.events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    let marks = role_marks(&h.store).await;
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].step, boblm_tuning::StepId::ImplementerDeepSeek);
    assert!(marks[0].until >= before + 3_500);
    assert!(marks[0].until <= before + 3_700);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn openai_billing_rejection_marks_the_reviewer_step_until_the_monthly_reset() {
    let mut h = Harness::new_role(
        "codex",
        Some(boblm_tuning::CODEX_ACCOUNT_KEY_API),
        "reviewer-openai-api",
    );
    let runner = h.start().await;
    runner.handle().resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    let before = now_seconds();
    h.events
        .send(provider_error(
            "Codex stopped this turn with an error: You exceeded your current quota, please check your plan and billing details.",
        ))
        .unwrap();
    h.events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    let marks = role_marks(&h.store).await;
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].step, boblm_tuning::StepId::ReviewerOpenAiApi);
    assert_eq!(marks[0].until, boblm_tuning::openai_api_reset_at(before));
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_api_billing_rejection_marks_the_reviewer_step_until_the_monthly_reset() {
    let mut h = Harness::new_role(
        "claude",
        Some(boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A),
        "reviewer-claude-api",
    );
    let runner = h.start().await;
    runner.handle().resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    let before = now_seconds();
    h.events
        .send(provider_error(
            "Claude reported an error: Credit balance is too low.",
        ))
        .unwrap();
    h.events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    let marks = role_marks(&h.store).await;
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].step, boblm_tuning::StepId::ReviewerClaudeApi);
    assert_eq!(marks[0].until, boblm_tuning::claude_api_reset_at(before));
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn openai_tpm_rate_limit_marks_a_short_cooldown_without_replaying_the_turn() {
    let mut h = Harness::new_role(
        "codex",
        Some(boblm_tuning::CODEX_ACCOUNT_KEY_API),
        "reviewer-openai-api",
    );
    let runner = h.start().await;
    runner.handle().resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    let before = now_seconds();
    h.events.send(provider_error(
        "Codex reported an error: Rate limit reached for gpt-6-luna in organization org-example on tokens per min (TPM): Limit 2000000, Used 1800000, Requested 240000. Please try again in 1.2s.",
    )).unwrap();
    // A retrying error is enough to hold new spawns off the API before the turn ends.
    let marks = role_marks(&h.store).await;
    let cooldown = i64::try_from(boblm_tuning::OPENAI_API_RATE_LIMIT_COOLDOWN.as_secs()).unwrap();
    assert_eq!(marks[0].step, boblm_tuning::StepId::ReviewerOpenAiApi);
    assert!((before + cooldown..=now_seconds() + cooldown).contains(&marks[0].until));
    h.events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    wait_status(&runner.handle(), |status| !status.turn_active).await;
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .messages("s", 0, i64::MAX)
            .unwrap()
            .len(),
        1
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_rate_limit_does_not_mark_a_role_step_exhausted() {
    let mut h = Harness::new_role("pi", None, "implementer-deepseek");
    let runner = h.start().await;
    runner.handle().resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    h.events
        .send(provider_error(
            "Pi stopped this turn with an error: 429 Too Many Requests: rate limit reached.",
        ))
        .unwrap();
    h.events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        h.store
            .reader()
            .unwrap()
            .role_exhaustions()
            .unwrap()
            .is_empty()
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_fake_child_without_an_account_id_records_no_account_usage() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "collect usage").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(Event::Usage {
            five_hour: None,
            weekly: Some(boblm_protocol::usage::AllowanceUsage {
                used_percent: 42.0,
                resets_at: Some(2_000_000_000),
            }),
            credits: None,
            context: None,
        })
        .unwrap();
    h.result();
    wait_status(&client, |s| !s.turn_active && s.dispatching == 0).await;
    let reader = h.store.reader().unwrap();
    assert!(reader.account_usage_for_session("s").unwrap().is_none());
    assert!(reader.account_usages(Some("claude")).unwrap().is_empty());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_new_runner_records_the_existing_chat_account_before_a_child_launches() {
    let h = Harness::new_with_account(Some("account-1"));
    let runner = h.start().await;
    let session = h
        .store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "s")
        .unwrap();
    assert_eq!(session.account_id.as_deref(), Some("account-1"));
    runner.shutdown().await.unwrap();
}

const HOUR_MS: i64 = 3_600_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn append_aged(h: &Harness, uuid: &str, age_hours: i64) {
    h.store
        .append(boblm_store::NewMessage {
            uuid,
            session: "s",
            role: "user",
            content: uuid,
            delivery_state: "queued",
            created_at: now_ms() - age_hours * HOUR_MS,
        })
        .unwrap();
}

fn delivery_state(h: &Harness, uuid: &str) -> String {
    h.store
        .reader()
        .unwrap()
        .message(uuid)
        .unwrap()
        .unwrap()
        .delivery_state
}

#[tokio::test]
async fn a_message_queued_over_six_hours_expires_unsent_and_a_fresher_one_is_delivered() {
    let mut h = Harness::new();
    append_aged(&h, A, 7);
    append_aged(&h, B, 5);
    let runner = h.start().await;
    let client = runner.handle();
    // The sidebar count drops the expired message at once.
    let status = client.status().await.unwrap();
    assert_eq!(status.queued, 1);
    assert!(
        status.queue_held,
        "a fresh message still waits for the owner"
    );
    assert_eq!(delivery_state(&h, A), "expired");
    assert_eq!(delivery_state(&h, B), "queued");
    assert!(h.inputs.try_recv().is_err());
    // Releasing the hold delivers the five-hour-old message and never the stale one.
    client.send(C, "owner is back").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    assert_eq!(h.input().await.uuid, C);
    assert!(h.inputs.try_recv().is_err());
    assert_eq!(delivery_state(&h, A), "expired");
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_message_that_ages_out_during_driver_startup_is_expired_and_never_sent() {
    let mut h = Harness::new();
    // Fresh at the first check, past the limit once the slow driver start finishes.
    h.store
        .append(boblm_store::NewMessage {
            uuid: A,
            session: "s",
            role: "user",
            content: A,
            delivery_state: "queued",
            created_at: now_ms() - 6 * HOUR_MS + 300,
        })
        .unwrap();
    *h.factory.spawn_delay.lock().unwrap() = Some(Duration::from_millis(700));
    let runner = h.start().await;
    let client = runner.handle();
    tokio::time::timeout(Duration::from_secs(10), async {
        while delivery_state(&h, A) != "expired" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the message should expire once startup finishes");
    assert!(
        h.inputs.try_recv().is_err(),
        "an expired message is never written"
    );
    // The queue is not stalled: a fresh message behind it is delivered.
    client.send(B, "fresh").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    assert!(h.inputs.try_recv().is_err());
    assert_eq!(delivery_state(&h, A), "expired");
    runner.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_steer_waiting_at_a_tool_boundary_is_never_expired_however_long_it_waits() {
    let mut h = Harness::new_with_agent("codex");
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "first turn").await.unwrap();
    assert_eq!(h.input().await.uuid, A);
    client.send(B, "steer").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    assert_eq!(delivery_state(&h, B), "dispatching");
    // No acknowledgment ever arrives; the sweep runs many times over the limit.
    tokio::time::advance(boblm_tuning::QUEUED_MESSAGE_MAX_AGE * 2).await;
    assert_ne!(delivery_state(&h, B), "expired");
    let _ = runner.handle().status().await;
    assert_ne!(delivery_state(&h, B), "expired");
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_old_unconfirmed_message_expires_without_being_resent_and_clears_the_hold() {
    let mut h = Harness::new();
    append_aged(&h, A, 7);
    h.store.transition(A, "queued", "dispatching").unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    let status = client.status().await.unwrap();
    assert_eq!((status.uncertain, status.queued), (0, 0));
    assert!(
        !status.queue_held,
        "nothing is left to release, so the hold clears"
    );
    assert_eq!(delivery_state(&h, A), "expired");
    let expired_from = h
        .store
        .reader()
        .unwrap()
        .message(A)
        .unwrap()
        .unwrap()
        .expired_from;
    assert_eq!(expired_from.as_deref(), Some("uncertain"));
    assert!(h.inputs.try_recv().is_err(), "expiry never resends");
    // The next message flows without an owner release.
    assert!(
        client
            .send_from_peer(B, "fresh peer message", "peer")
            .await
            .unwrap()
    );
    assert_eq!(h.input().await.uuid, B);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_hold_behind_a_fresh_message_survives_the_expiry_of_an_old_uncertain_one() {
    let mut h = Harness::new();
    append_aged(&h, A, 7);
    h.store.transition(A, "queued", "dispatching").unwrap();
    append_aged(&h, B, 1);
    let runner = h.start().await;
    let client = runner.handle();
    let status = client.status().await.unwrap();
    assert_eq!((status.uncertain, status.queued), (0, 1));
    assert!(status.queue_held);
    assert!(h.inputs.try_recv().is_err());
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_held_queue_reports_the_last_driver_error_until_it_is_released() {
    let h = Harness::new();
    let runner = RunningSession::start(h.store.clone(), "s", Arc::new(NeverStarts))
        .await
        .unwrap();
    let client = runner.handle();
    assert_eq!(client.status().await.unwrap().held_reason, None);
    client.send(A, "hello").await.unwrap();
    let status = wait_status(&client, |s| s.queue_held && s.held_reason.is_some()).await;
    let reason = status.held_reason.unwrap();
    assert!(reason.contains("could not be started"), "{reason}");
    assert!(reason.contains("the binary is missing"), "{reason}");
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn prompt_suggestion_is_current_only_and_clears_on_turn_start() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "hello").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.result();
    h.events
        .send(Event::PromptSuggestion {
            suggestion: serde_json::json!("Explain this"),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if client.status().await.unwrap().prompt_suggestion.as_deref() == Some("Explain this") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let page =
        boblm_runner::events::cursor_page(&client, "s", boblm_protocol::client::Sequence(0), 256)
            .await
            .unwrap();
    assert_eq!(
        page.status.prompt_suggestion.as_deref(),
        Some("Explain this")
    );
    let stored = client.events_since(0, 1024).await.unwrap();
    assert!(
        stored.iter().all(
            |e| !e.payload.contains("Explain this") && !e.payload.contains("prompt_suggestion")
        )
    );
    client.send(B, "next").await.unwrap();
    h.input().await;
    assert!(client.status().await.unwrap().prompt_suggestion.is_none());
    runner.shutdown().await.unwrap();
}

fn runaway_usage(tokens: u64) -> Event {
    Event::TokenUsage {
        model: "runaway-fixture".into(),
        input_tokens: tokens,
        cached_input_tokens: 0,
        cache_write_input_tokens: 0,
        output_tokens: 0,
    }
}

async fn runaway_barrier(h: &Harness, client: &boblm_runner::SessionHandle) {
    let mut cursor = client
        .events_since(0, 1024)
        .await
        .unwrap()
        .last()
        .map_or(0, |event| event.seq);
    let marker = uuid::Uuid::new_v4().to_string();
    h.events
        .send(Event::System {
            subtype: Some(marker.clone()),
            session_id: None,
            status: None,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = client.events_since(cursor, 1024).await.unwrap();
            if events.iter().any(|event| event.payload.contains(&marker)) {
                // Status is serialized after the batch's usage check and interrupt.
                client.status().await.unwrap();
                break;
            }
            if let Some(event) = events.last() {
                cursor = event.seq;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn runaway_reports(store: &Store) -> Vec<boblm_store::SubagentReport> {
    store
        .reader()
        .unwrap()
        .subagent_reports()
        .unwrap()
        .into_iter()
        .filter(|report| report.source_kind == "subagent_runaway_limit")
        .collect()
}

#[tokio::test]
async fn runaway_tokens_count_cache_and_output_once_and_hold_unconfirmed_input() {
    let mut h = Harness::new_role("pi", None, "implementer-deepseek");
    let runner = h.start().await;
    let client = runner.handle();
    client.resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    let quarter = boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS / 4;
    let call = Event::Assistant {
        message: serde_json::json!({
            "id": "one-call-many-blocks",
            "usage": {"input_tokens": quarter, "cache_read_input_tokens": quarter,
                "cache_creation_input_tokens": quarter, "output_tokens": quarter},
            "content": []
        }),
    };
    h.events.send(call.clone()).unwrap();
    h.events.send(call).unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 0);
    assert!(runaway_reports(&h.store).is_empty());
    // Another peer cannot reset the parent's round, and its unconfirmed input is held.
    client
        .send_from_peer(A, "peer steer", "other")
        .await
        .unwrap();
    assert_eq!(h.input().await.uuid, A);
    h.events.send(runaway_usage(1)).unwrap();
    wait_status(&client, |s| s.queue_held && s.uncertain == 1 && !s.stopping).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    let reports = runaway_reports(&h.store);
    assert_eq!(reports.len(), 1);
    let message = reports[0].error_message.as_deref().unwrap();
    assert!(message.contains(&format!(
        "{} tokens and 2 model calls",
        boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS + 1
    )));
    assert!(message.contains("was paused"));
    assert!(message.contains("ask the owner with the ask tool"));
    assert_eq!(reports[0].parent_session, "root");
    // The pause reason uses the existing error contract, so the chat's event stream
    // stays readable and the owner can see why this subagent was paused.
    let page =
        boblm_runner::events::cursor_page(&client, "s", boblm_protocol::client::Sequence(0), 256)
            .await
            .unwrap();
    assert!(page.events.iter().any(|event| matches!(
        event, boblm_protocol::client::StoredEvent::DriverError { payload, .. }
            if payload.message.contains("was paused")
    )));
    h.events.send(runaway_usage(1)).unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    assert_eq!(runaway_reports(&h.store).len(), 1);
    assert_eq!(
        h.store
            .reader()
            .unwrap()
            .message(A)
            .unwrap()
            .unwrap()
            .delivery_state,
        "uncertain"
    );
    assert!(h.inputs.try_recv().is_err());
    // The durable round claim survives opening a new store connection.
    let reopened = Store::open(h._temp.path().join("test.sqlite")).unwrap();
    assert!(
        !reopened
            .pause_runaway_subagent_round("s", now_ms())
            .unwrap()
    );
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn runaway_calls_crossing_interrupts_and_reports_once() {
    let mut h = Harness::new_role("pi", None, "implementer-deepseek");
    h.store
        .record_events(
            "s",
            "agent_event",
            (0..boblm_tuning::SUBAGENT_ROUND_MAX_CALLS)
                .map(|_| serde_json::to_value(runaway_usage(0)).unwrap())
                .collect(),
            now_ms(),
        )
        .unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    client.resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    // A non-usage event does not add a call. The exact ceiling is still allowed.
    assert!(!h.store.pause_runaway_subagent_round("s", now_ms()).unwrap());
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 0);
    assert!(runaway_reports(&h.store).is_empty());
    h.events.send(runaway_usage(0)).unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    wait_status(&client, |s| s.queue_held && !s.stopping).await;
    h.events.send(runaway_usage(0)).unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    let reports = runaway_reports(&h.store);
    assert_eq!(reports.len(), 1);
    let message = reports[0].error_message.as_deref().unwrap();
    assert!(message.contains(&format!(
        "0 tokens and {} model calls",
        boblm_tuning::SUBAGENT_ROUND_MAX_CALLS + 1
    )));
    assert!(message.contains(&format!(
        "limit of {} model calls",
        boblm_tuning::SUBAGENT_ROUND_MAX_CALLS
    )));
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn runaway_parent_ping_starts_a_fresh_round_and_retry_does_not() {
    let mut h = Harness::new_role("pi", None, "implementer-deepseek");
    let runner = h.start().await;
    let client = runner.handle();
    client.resume().await.unwrap();
    let work = h.input().await;
    h.lifecycle(&work.uuid, LifecycleState::Started);
    h.events
        .send(runaway_usage(
            boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS as u64 + 1,
        ))
        .unwrap();
    wait_status(&client, |s| s.queue_held && !s.stopping).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    client.send_from_peer(B, "continue", "root").await.unwrap();
    assert_eq!(h.input().await.uuid, B);
    h.lifecycle(B, LifecycleState::Started);
    h.events
        .send(runaway_usage(
            boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS as u64,
        ))
        .unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 1);
    assert_eq!(runaway_reports(&h.store).len(), 1);
    // The same accepted UUID is a retry, not a new round.
    client.send_from_peer(B, "continue", "root").await.unwrap();
    h.events.send(runaway_usage(1)).unwrap();
    wait_status(&client, |s| s.queue_held && !s.stopping).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 2);
    assert_eq!(runaway_reports(&h.store).len(), 2);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn runaway_root_chats_are_never_limited() {
    let mut h = Harness::new();
    h.store
        .record_events(
            "s",
            "agent_event",
            (0..=boblm_tuning::SUBAGENT_ROUND_MAX_CALLS)
                .map(|_| serde_json::to_value(runaway_usage(0)).unwrap())
                .collect(),
            now_ms(),
        )
        .unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "ordinary root work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    h.events
        .send(runaway_usage(
            boblm_tuning::SUBAGENT_ROUND_MAX_TOKENS as u64 + 1,
        ))
        .unwrap();
    runaway_barrier(&h, &client).await;
    assert_eq!(h.interrupts.load(Ordering::SeqCst), 0);
    assert!(runaway_reports(&h.store).is_empty());
    assert!(!client.status().await.unwrap().queue_held);
    runner.shutdown().await.unwrap();
}

async fn self_compact_context(h: &Harness, client: &boblm_runner::SessionHandle, used: u64) {
    h.events.send(context(used, 100_000)).unwrap();
    wait_status(client, |status| {
        status
            .usage
            .as_ref()
            .and_then(|usage| usage.context.as_ref())
            .is_some_and(|context| context.used_tokens == used)
    })
    .await;
}

#[tokio::test]
async fn self_compact_waits_for_turn_and_acknowledgments_and_deduplicates() {
    for agent in ["claude", "codex"] {
        let mut h = Harness::new_with_agent(agent);
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "phase one").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        client
            .send_from_peer(B, "follow-up", "other")
            .await
            .unwrap();
        assert_eq!(h.input().await.uuid, B);
        h.lifecycle(B, LifecycleState::Queued);
        self_compact_context(&h, &client, 65_000).await;
        let report = client
            .compact(Some("Keep phase two's requirements.\nKeep test results."))
            .await
            .unwrap();
        assert!(report.queued && !report.already_queued);
        assert_eq!(report.context.used, Some(65_000));
        assert_eq!(report.context.window, Some(100_000));
        assert_eq!(report.context.percent, Some(65.0));
        for _ in 0..3 {
            assert!(
                client
                    .compact(Some("ignored replacement"))
                    .await
                    .unwrap()
                    .already_queued
            );
        }
        assert_eq!(client.status().await.unwrap().queued, 1);
        assert!(h.inputs.try_recv().is_err());
        assert_eq!(h.interrupts.load(Ordering::SeqCst), 0);
        h.result();
        wait_status(&client, |status| {
            !status.turn_active && status.dispatching == 1
        })
        .await;
        assert!(
            h.inputs.try_recv().is_err(),
            "compaction must await follow-up acknowledgment"
        );
        assert!(client.compact(None).await.unwrap().already_queued);
        h.lifecycle(B, LifecycleState::Completed);
        let compact = h.input().await;
        assert_eq!(
            compact.content,
            "/compact Keep phase two's requirements.\nKeep test results."
        );
        assert_eq!(compact.priority, None, "compaction must never steer");
        assert!(!compact.human);
        h.lifecycle(&compact.uuid, LifecycleState::Started);
        wait_status(&client, |status| {
            status.turn_active && status.dispatching == 0
        })
        .await;
        assert!(client.compact(None).await.unwrap().already_queued);
        h.events.send(compaction("compact_boundary")).unwrap();
        h.result();
        let continuation = h.input().await;
        assert_eq!(continuation.content, "boblm: continue");
        assert_eq!(continuation.priority, None);
        assert!(!continuation.human);
        h.lifecycle(&continuation.uuid, LifecycleState::Started);
        h.result();
        wait_status(&client, |status| {
            !status.turn_active && status.dispatching == 0 && status.queued == 0
        })
        .await;
        assert!(h.inputs.try_recv().is_err());
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn self_compact_refuses_pi_low_and_unknown_context_without_dispatch() {
    for (agent, used, reason) in [
        ("pi", Some(65_000), "DeepSeek cache reads are nearly free"),
        ("claude", Some(49_999), "below the 50% minimum"),
        ("codex", None, "context usage is unknown"),
    ] {
        let mut h = Harness::new_with_agent(agent);
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "work").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        if let Some(used) = used {
            self_compact_context(&h, &client, used).await;
        }
        let report = client.compact(None).await.unwrap();
        assert!(!report.queued && !report.already_queued);
        assert!(report.message.contains(reason), "{}", report.message);
        assert_eq!(report.context.used, used);
        assert_eq!(report.context.window, used.map(|_| 100_000));
        assert_eq!(
            report.context.percent,
            used.map(|used| used as f64 / 1_000.0)
        );
        assert_eq!(client.status().await.unwrap().queued, 0);
        assert!(h.inputs.try_recv().is_err());
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn self_compact_preserves_stop_before_and_after_request() {
    for stop_first in [false, true] {
        let mut h = Harness::new();
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "work").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        self_compact_context(&h, &client, 65_000).await;
        if stop_first {
            client.stop().await.unwrap();
        }
        assert!(client.compact(None).await.unwrap().queued);
        if !stop_first {
            client.stop().await.unwrap();
        }
        let status = client.status().await.unwrap();
        assert!(status.stopped && status.queue_held && !status.stopping);
        assert_eq!(status.queued, 1);
        assert!(h.inputs.try_recv().is_err());
        client.resume().await.unwrap();
        assert_eq!(h.input().await.content, "/compact");
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn self_compact_keeps_recovered_queue_held_and_deduplicates_across_restart() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let client = runner.handle();
    client.send(A, "work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    self_compact_context(&h, &client, 65_000).await;
    runner.shutdown().await.unwrap();
    let recovered = h.start().await;
    let client = recovered.handle();
    assert!(client.status().await.unwrap().queue_held);
    assert!(
        client
            .compact(Some("first instructions"))
            .await
            .unwrap()
            .queued
    );
    assert!(client.status().await.unwrap().queue_held);
    assert_eq!(client.status().await.unwrap().queued, 1);
    recovered.shutdown().await.unwrap();
    let recovered = h.start().await;
    let client = recovered.handle();
    assert!(
        client
            .compact(Some("ignored"))
            .await
            .unwrap()
            .already_queued
    );
    let status = client.status().await.unwrap();
    assert!(status.queue_held && !status.running);
    assert_eq!(status.queued, 1);
    assert!(h.inputs.try_recv().is_err());
    recovered.shutdown().await.unwrap();
}

#[tokio::test]
async fn self_compact_crosses_persistent_runner_transport_without_steering() {
    let mut h = Harness::new();
    let runner = h.start().await;
    let owner = runner.handle();
    let socket = h._temp.path().join("compact.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let host = tokio::spawn(boblm_runner::transport::serve(listener, owner.clone()));
    let proxy = RunningSession::attach(h.store.clone(), "s", socket, None)
        .await
        .unwrap();
    let client = proxy.handle();
    client.send(A, "work").await.unwrap();
    h.input().await;
    h.lifecycle(A, LifecycleState::Started);
    self_compact_context(&h, &owner, 65_000).await;
    let report = client
        .compact(Some("Keep the requirements."))
        .await
        .unwrap();
    assert!(report.queued && !report.already_queued);
    assert_eq!(report.context.used, Some(65_000));
    assert_eq!(report.context.window, Some(100_000));
    assert_eq!(report.context.percent, Some(65.0));
    assert_eq!(client.published_status().unwrap().queued, 1);
    assert!(h.inputs.try_recv().is_err());
    h.result();
    let compact = h.input().await;
    assert_eq!(compact.content, "/compact Keep the requirements.");
    assert_eq!(compact.priority, None);
    proxy.shutdown().await.unwrap();
    runner.shutdown().await.unwrap();
    host.await.unwrap().unwrap();
}

#[tokio::test]
async fn self_compact_continuation_requires_success_and_preserves_stop() {
    for stopped in [false, true] {
        let mut h = Harness::new();
        let runner = h.start().await;
        let client = runner.handle();
        client.send(A, "work").await.unwrap();
        h.input().await;
        h.lifecycle(A, LifecycleState::Started);
        self_compact_context(&h, &client, 65_000).await;
        client.compact(None).await.unwrap();
        h.result();
        let compact = h.input().await;
        h.lifecycle(&compact.uuid, LifecycleState::Started);
        wait_status(&client, |status| {
            status.dispatching == 0 && status.turn_active
        })
        .await;
        if stopped {
            client.stop().await.unwrap();
            let status = client.status().await.unwrap();
            assert!(status.stopped && status.queue_held);
            assert_eq!(status.queued, 0, "Stop must cancel automatic continuation");
            assert!(h.inputs.try_recv().is_err());
        } else {
            h.events.send(result("error", true)).unwrap();
            wait_status(&client, |status| !status.turn_active && status.queued == 0).await;
            assert!(
                h.inputs.try_recv().is_err(),
                "failed compaction must not continue work"
            );
        }
        runner.shutdown().await.unwrap();
    }
}
