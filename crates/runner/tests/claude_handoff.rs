use async_trait::async_trait;
use boblm_protocol::{
    Event, LifecycleState, Priority,
    jobs::{BackgroundJob, JobSnapshot},
};
use boblm_runner::{ClaudeHandoff, Driver, DriverFactory, RunningSession, SessionHandle};
use boblm_store::{Session, Store};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

const A: &str = boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_A;
const B: &str = boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_B;
const CONVERSATION: &str = "19ae6d66-c34f-432c-a001-6bf1847865e9";
struct Child {
    key: String,
    resume: Option<String>,
    events: mpsc::UnboundedSender<Event>,
}
struct Input {
    uuid: String,
    text: String,
    human: bool,
}
struct Factory {
    enabled: bool,
    at_cutoff: AtomicBool,
    copies: AtomicUsize,
    children: mpsc::UnboundedSender<Child>,
    inputs: mpsc::UnboundedSender<Input>,
}
struct Fake {
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<Input>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, uuid: &str, text: &str, _: Option<Priority>) -> io::Result<()> {
        self.inputs
            .send(Input {
                uuid: uuid.into(),
                text: text.into(),
                human: false,
            })
            .map_err(io::Error::other)
    }
    async fn send_with_origin(
        &mut self,
        uuid: &str,
        text: &str,
        _: Option<Priority>,
        human: bool,
    ) -> io::Result<()> {
        self.inputs
            .send(Input {
                uuid: uuid.into(),
                text: text.into(),
                human,
            })
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
        self.events.close();
        Ok(())
    }
    fn wakes_itself_on_background_job_completion(&self) -> bool {
        true
    }
}
#[async_trait]
impl DriverFactory for Factory {
    fn account_id(&self, session: &Session) -> Option<String> {
        Some(session.account_key.as_deref().unwrap_or(A).into())
    }
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.children
            .send(Child {
                key: session.account_key.as_deref().unwrap_or(A).into(),
                resume: session.agent_session_id.clone(),
                events: tx,
            })
            .map_err(io::Error::other)?;
        Ok(Box::new(Fake {
            events: rx,
            inputs: self.inputs.clone(),
        }))
    }
    async fn claude_handoff_target(
        &self,
        store: &Store,
        session: &Session,
    ) -> io::Result<Option<ClaudeHandoff>> {
        let marked = !store
            .reader()
            .map_err(io::Error::other)?
            .claude_account_exhaustions()
            .map_err(io::Error::other)?
            .is_empty();
        if !self.enabled
            || session.account_key.as_deref() == Some(B)
            || !(self.at_cutoff.load(Ordering::SeqCst) || marked)
        {
            return Ok(None);
        }
        Ok(Some(ClaudeHandoff {
            source: A.into(),
            target: B.into(),
            account_id: Some(B.into()),
            source_reset: 4_000_000_000,
        }))
    }
    async fn copy_claude_transcript(&self, session: &Session, target: &str) -> io::Result<()> {
        assert_eq!(session.agent_session_id.as_deref(), Some(CONVERSATION));
        assert_eq!(target, B);
        self.copies.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Harness {
    _dir: tempfile::TempDir,
    store: Store,
    factory: Arc<Factory>,
    children: mpsc::UnboundedReceiver<Child>,
    inputs: mpsc::UnboundedReceiver<Input>,
}
impl Harness {
    fn new(enabled: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("test.sqlite")).unwrap();
        store.create_session("s", "/fake", 1).unwrap();
        store.set_agent_session("s", CONVERSATION).unwrap();
        let (children, child_rx) = mpsc::unbounded_channel();
        let (inputs, input_rx) = mpsc::unbounded_channel();
        Self {
            _dir: dir,
            store,
            factory: Arc::new(Factory {
                enabled,
                at_cutoff: AtomicBool::new(false),
                copies: AtomicUsize::new(0),
                children,
                inputs,
            }),
            children: child_rx,
            inputs: input_rx,
        }
    }
    async fn start(&self) -> RunningSession {
        RunningSession::start(self.store.clone(), "s", self.factory.clone())
            .await
            .unwrap()
    }
    async fn child(&mut self) -> Child {
        tokio::time::timeout(Duration::from_secs(10), self.children.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn input(&mut self) -> Input {
        tokio::time::timeout(Duration::from_secs(10), self.inputs.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn copies(&self) -> usize {
        self.factory.copies.load(Ordering::SeqCst)
    }
    async fn moved(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.store.reader().unwrap().sessions().unwrap()[0]
                    .account_key
                    .as_deref()
                    == Some(B)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
fn usage(used: f64) -> Event {
    Event::Usage {
        five_hour: None,
        context: None,
        credits: None,
        weekly: Some(boblm_protocol::usage::AllowanceUsage {
            used_percent: used,
            resets_at: Some(4_000_000_000),
        }),
    }
}
fn ack(child: &Child, input: &Input) {
    child
        .events
        .send(Event::CommandLifecycle {
            command_uuid: input.uuid.clone(),
            state: LifecycleState::Started,
        })
        .unwrap();
}
fn result(child: &Child) {
    child
        .events
        .send(Event::Result {
            subtype: None,
            is_error: Some(false),
        })
        .unwrap();
}
async fn wait(client: &SessionHandle, predicate: impl Fn(&boblm_runner::Status) -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if predicate(&client.status().await.unwrap()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn claude_handoff_waits_for_turn_and_jobs_then_resumes_the_same_conversation() {
    let mut h = Harness::new(true);
    h.store.set_claude_account_key("s", A).unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    client
        .send("2b3cdc59-57ef-4d7b-8d51-a7cb01e3276f", "work")
        .await
        .unwrap();
    let first = h.child().await;
    let input = h.input().await;
    ack(&first, &input);
    h.factory.at_cutoff.store(true, Ordering::SeqCst);
    first.events.send(usage(98.0)).unwrap();
    first
        .events
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
    wait(&client, |s| {
        s.turn_active
            && s.dispatching == 0
            && s.jobs.as_ref().is_some_and(|jobs| jobs.tasks.len() == 1)
    })
    .await;
    assert_eq!(h.copies(), 0);
    result(&first);
    wait(&client, |s| {
        !s.turn_active && s.jobs.as_ref().is_some_and(|jobs| jobs.tasks.len() == 1)
    })
    .await;
    assert_eq!(h.copies(), 0);
    // Unknown inventory also cannot establish that the jobs ended.
    first
        .events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot::default(),
        })
        .unwrap();
    wait(&client, |s| {
        s.jobs.as_ref().is_some_and(|jobs| !jobs.available)
    })
    .await;
    assert_eq!(h.copies(), 0);
    first
        .events
        .send(Event::BackgroundJobs {
            snapshot: JobSnapshot {
                available: true,
                tasks: vec![],
            },
        })
        .unwrap();
    h.moved().await;
    assert_eq!(h.copies(), 1);
    client
        .send("e93bc7e3-5232-44d0-9d76-9a5efc78b7e5", "continue")
        .await
        .unwrap();
    let second = h.child().await;
    assert_eq!(first.key, A);
    assert_eq!(second.key, B);
    assert_eq!(second.resume.as_deref(), Some(CONVERSATION));
    let continued = h.input().await;
    ack(&second, &continued);
    second.events.send(usage(12.0)).unwrap();
    result(&second);
    wait(&client, |status| {
        !status.turn_active
            && status.dispatching == 0
            && status
                .account_usage
                .as_ref()
                .and_then(|usage| usage.weekly.as_ref())
                .is_some_and(|weekly| weekly.used_percent == 12.0)
    })
    .await;
    let reader = h.store.reader().unwrap();
    assert_eq!(
        reader
            .account_usage("claude", A)
            .unwrap()
            .unwrap()
            .weekly
            .unwrap()
            .used_percent,
        98.0
    );
    assert_eq!(
        reader
            .account_usage("claude", B)
            .unwrap()
            .unwrap()
            .weekly
            .unwrap()
            .used_percent,
        12.0
    );
    // Both snapshots and durable summaries expose the account change.
    let page =
        boblm_runner::events::cursor_page(&client, "s", boblm_protocol::client::Sequence(0), 256)
            .await
            .unwrap();
    assert_eq!(page.status.account_key.as_deref(), Some(B));
    let keys: Vec<_> = page
        .events
        .iter()
        .filter_map(|event| match event {
            boblm_protocol::client::StoredEvent::StateSummary { payload, .. } => {
                payload.account_key.as_deref()
            }
            _ => None,
        })
        .collect();
    let first_a = keys.iter().position(|key| *key == A).unwrap();
    let first_b = keys.iter().position(|key| *key == B).unwrap();
    assert!(first_a < first_b);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_handoff_waits_for_dispatch_acknowledgment_after_result() {
    let mut h = Harness::new(true);
    let runner = h.start().await;
    let client = runner.handle();
    client
        .send("11ba61cb-8507-46f8-a7f7-670b61ff3cba", "work")
        .await
        .unwrap();
    let child = h.child().await;
    let input = h.input().await;
    h.factory.at_cutoff.store(true, Ordering::SeqCst);
    result(&child);
    wait(&client, |s| !s.turn_active && s.dispatching == 1).await;
    assert_eq!(h.copies(), 0);
    child
        .events
        .send(Event::CommandLifecycle {
            command_uuid: input.uuid,
            state: LifecycleState::Completed,
        })
        .unwrap();
    h.moved().await;
    assert_eq!(h.copies(), 1);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_limit_handoff_queues_one_automated_note_and_single_account_still_waits() {
    for enabled in [false, true] {
        let mut h = Harness::new(enabled);
        let runner = h.start().await;
        let client = runner.handle();
        client
            .send("b8f320d1-0f60-4c72-9751-791cc248df0f", "work")
            .await
            .unwrap();
        let first = h.child().await;
        let input = h.input().await;
        ack(&first, &input);
        first.events.send(Event::Assistant { message: serde_json::json!({"content":[{"type":"text","text":"Claude reported an error: You've hit your limit. Usage resets tomorrow."}]}) }).unwrap();
        first
            .events
            .send(Event::Result {
                subtype: Some("error_during_execution".into()),
                is_error: Some(true),
            })
            .unwrap();
        if enabled {
            let second = h.child().await;
            let note = h.input().await;
            assert_eq!(second.key, B);
            assert_eq!(second.resume.as_deref(), Some(CONVERSATION));
            assert!(note.text.contains("last turn was cut off"));
            assert!(!note.human);
            assert_ne!(input.uuid, note.uuid);
            ack(&second, &note);
            result(&second);
            wait(&client, |s| !s.turn_active && s.dispatching == 0).await;
            assert_eq!(h.copies(), 1);
            assert!(h.inputs.try_recv().is_err());
            assert!(
                !h.store
                    .reader()
                    .unwrap()
                    .claude_account_needs_continuation("s")
                    .unwrap()
            );
        } else {
            wait(&client, |s| !s.turn_active && s.dispatching == 0).await;
            assert_eq!(h.copies(), 0);
            assert!(h.inputs.try_recv().is_err());
        }
        runner.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn claude_api_transient_rate_limit_does_not_exhaust_monthly_credit() {
    let mut h = Harness::new(false);
    h.store
        .set_claude_account_key("s", boblm_tuning::CLAUDE_ACCOUNT_KEY_API_B)
        .unwrap();
    let runner = h.start().await;
    let client = runner.handle();
    client
        .send("631957fc-99a7-4f09-81eb-ab7316a61482", "review")
        .await
        .unwrap();
    let child = h.child().await;
    let input = h.input().await;
    ack(&child, &input);
    child.events.send(Event::Assistant { message: serde_json::json!({"content":[{"type":"text","text":"Claude reported an error: rate_limit_error: Too many requests, please retry shortly."}]}) }).unwrap();
    child
        .events
        .send(Event::Result {
            subtype: Some("error_during_execution".into()),
            is_error: Some(true),
        })
        .unwrap();
    wait(&client, |status| {
        !status.turn_active && status.dispatching == 0
    })
    .await;
    assert!(
        h.store
            .reader()
            .unwrap()
            .claude_account_exhaustions()
            .unwrap()
            .is_empty()
    );
    assert_eq!(h.copies(), 0);
    runner.shutdown().await.unwrap();
}
