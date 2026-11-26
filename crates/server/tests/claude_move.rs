//! The owner's manual move of an idle Claude plan chat to another plan account (ADR 0047).
//! Fake drivers and a temporary database stand in for Claude; nothing touches a real chat.
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use boblm_protocol::{Event, LifecycleState};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tower::ServiceExt;

const TOKEN: &str = "claude-move-test-token-only";
const A: &str = "claude-plan-a";
const B: &str = "claude-plan-b";
const API: &str = "claude-api-a";
const CONVERSATION: &str = "8c2f436d-4403-4100-b1e9-155f997e4715";

struct Child {
    key: Option<String>,
    resume: Option<String>,
    events: mpsc::UnboundedSender<Event>,
}
struct Fake {
    emit: mpsc::UnboundedSender<Event>,
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<String>,
    shutdowns: Arc<AtomicUsize>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(
        &mut self,
        uuid: &str,
        _: &str,
        _: Option<boblm_protocol::Priority>,
    ) -> io::Result<()> {
        self.inputs.send(uuid.into()).map_err(io::Error::other)
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }
    async fn configure(&mut self, _: &str, _: Option<&str>, _: Option<&str>) -> io::Result<()> {
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.emit
            .send(Event::Result {
                subtype: Some("interrupted".into()),
                is_error: Some(false),
            })
            .map_err(io::Error::other)
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Factory {
    children: Mutex<Vec<Child>>,
    copies: Mutex<Vec<(Option<String>, String)>>,
    fail_copy: AtomicBool,
    /// Account keys with no signed-in Claude account.
    logged_out: Mutex<Vec<String>>,
    inputs: mpsc::UnboundedSender<String>,
    shutdowns: Arc<AtomicUsize>,
}
#[async_trait]
impl DriverFactory for Factory {
    async fn spawn(&self, session: &Session) -> io::Result<Box<dyn Driver>> {
        let (emit, events) = mpsc::unbounded_channel();
        self.children.lock().unwrap().push(Child {
            key: session.account_key.clone(),
            resume: session.agent_session_id.clone(),
            events: emit.clone(),
        });
        Ok(Box::new(Fake {
            emit,
            events,
            inputs: self.inputs.clone(),
            shutdowns: self.shutdowns.clone(),
        }))
    }
    async fn copy_claude_transcript(&self, session: &Session, target: &str) -> io::Result<()> {
        assert_eq!(session.agent_session_id.as_deref(), Some(CONVERSATION));
        if self.fail_copy.load(Ordering::SeqCst) {
            return Err(io::Error::other(
                "Claude's transcript was not found. Restore it before retrying.",
            ));
        }
        self.copies
            .lock()
            .unwrap()
            .push((session.account_key.clone(), target.into()));
        Ok(())
    }
    fn account_id(&self, session: &Session) -> Option<String> {
        let key = session.account_key.as_deref().unwrap_or(A);
        (!self.logged_out.lock().unwrap().iter().any(|k| k == key)).then(|| format!("id-{key}"))
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    store: Store,
    app: Arc<App>,
    router: Router,
    factory: Arc<Factory>,
    inputs: mpsc::UnboundedReceiver<String>,
    shutdowns: Arc<AtomicUsize>,
}
impl Harness {
    async fn new() -> Self {
        Self::with(|_, _| {}).await
    }
    /// `prepare` runs before the server attaches its runners to the stored chats.
    async fn with(prepare: impl FnOnce(&Store, &str)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("move.sqlite")).unwrap();
        let project = dir.path().to_str().unwrap().to_owned();
        store.create_session("s", &project, 1).unwrap();
        store.set_claude_account_key("s", B).unwrap();
        store.set_agent_session("s", CONVERSATION).unwrap();
        prepare(&store, &project);
        let (inputs, input_rx) = mpsc::unbounded_channel();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let factory = Arc::new(Factory {
            children: Mutex::new(Vec::new()),
            copies: Mutex::new(Vec::new()),
            fail_copy: AtomicBool::new(false),
            logged_out: Mutex::new(Vec::new()),
            inputs,
            shutdowns: shutdowns.clone(),
        });
        let app = App::open(store.clone(), factory.clone(), TOKEN.into())
            .await
            .unwrap();
        app.set_claude_accounts_for_tests(&[A, B], &[API]);
        let router = app.router(Some(dir.path().into()));
        Self {
            _dir: dir,
            store,
            app,
            router,
            factory,
            inputs: input_rx,
            shutdowns,
        }
    }
    async fn command(&self, name: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/commands/{name}"))
                    .header("Authorization", format!("Bearer {TOKEN}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    async fn move_to(&self, session: &str, key: &str) -> (StatusCode, Value) {
        self.command(
            "moveClaudeAccount",
            json!({"sessionId":session,"accountKey":key}),
        )
        .await
    }
    /// One turn that ends normally, leaving a live idle child.
    async fn run_a_turn(&mut self, id: &str) {
        let (status, reply) = self
            .command(
                "sendMessage",
                json!({"sessionId":"s","messageId":id,"content":"hello"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(self.inputs.recv().await.unwrap(), id);
        let events = self.children().last().unwrap().events.clone();
        events
            .send(Event::CommandLifecycle {
                command_uuid: id.into(),
                state: LifecycleState::Started,
            })
            .unwrap();
        events
            .send(Event::Result {
                subtype: None,
                is_error: Some(false),
            })
            .unwrap();
        self.wait(|s| !s.turn_active && s.running).await;
    }
    fn children(&self) -> Vec<Child> {
        self.factory
            .children
            .lock()
            .unwrap()
            .iter()
            .map(|c| Child {
                key: c.key.clone(),
                resume: c.resume.clone(),
                events: c.events.clone(),
            })
            .collect()
    }
    async fn wait(&self, predicate: impl Fn(&boblm_runner::Status) -> bool) {
        let handle = self.app.handle_for_tests("s").await;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if predicate(&handle.status().await.unwrap()) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    /// A turn result old enough that no late acknowledgment can still arrive.
    fn settle(&self, id: &str) {
        self.store
            .record_event(
                id,
                "agent_event",
                json!({"type":"result"}),
                chrono::Utc::now().timestamp_millis() - 60_000,
            )
            .unwrap();
    }
    fn session(&self, id: &str) -> Session {
        self.store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
    }
    fn assert_unmoved(&self, id: &str, key: &str) {
        let session = self.session(id);
        assert_eq!(session.account_key.as_deref(), Some(key));
        assert!(
            !session.archived,
            "a refused or failed move must reopen the chat"
        );
        assert!(
            self.store
                .reader()
                .unwrap()
                .claude_account_handoffs(id)
                .unwrap()
                .is_empty(),
            "a refused or failed move must leave no exclusion"
        );
    }
}
fn message(reply: &Value) -> String {
    reply["message"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn an_idle_chat_moves_with_its_conversation_and_resumes_on_the_new_account() {
    let mut h = Harness::new().await;
    h.run_a_turn("00000000-0000-4000-8000-000000000001").await;
    h.settle("s");
    assert_eq!(h.children().len(), 1);
    assert_eq!(h.children()[0].key.as_deref(), Some(B));

    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(
        reply,
        json!({"sessionId":"s","from":"Claude plan B","to":"Claude plan A","status":"moved"})
    );
    // The idle child was stopped through the archive path and the chat is open again.
    assert_eq!(h.shutdowns.load(Ordering::SeqCst), 1);
    let handle = h.app.handle_for_tests("s").await;
    let live = handle.status().await.unwrap();
    assert!(!live.running && !live.archived && !live.stopped);
    // The transcript was copied from the source account to the destination.
    assert_eq!(
        *h.factory.copies.lock().unwrap(),
        vec![(Some(B.to_owned()), A.to_owned())]
    );
    // Key and provider identity switched together; the conversation ID did not change.
    let session = h.session("s");
    assert_eq!(session.account_key.as_deref(), Some(A));
    assert_eq!(session.account_id.as_deref(), Some("id-claude-plan-a"));
    assert_eq!(session.agent_session_id.as_deref(), Some(CONVERSATION));
    // The source is excluded until its weekly reset, so automatic handoff cannot bounce back.
    let exclusions = h
        .store
        .reader()
        .unwrap()
        .claude_account_handoffs("s")
        .unwrap();
    assert_eq!(exclusions.len(), 1);
    assert_eq!(exclusions[0].0, B);
    assert!(exclusions[0].1 > chrono::Utc::now().timestamp());
    // The next message starts a fresh child on the new account that resumes the same conversation.
    let id = "00000000-0000-4000-8000-000000000002";
    let (status, _) = h
        .command(
            "sendMessage",
            json!({"sessionId":"s","messageId":id,"content":"again"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.inputs.recv().await.unwrap(), id);
    let children = h.children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[1].key.as_deref(), Some(A));
    assert_eq!(children[1].resume.as_deref(), Some(CONVERSATION));

    // A retry of the finished move changes nothing and says so.
    let copies = h.factory.copies.lock().unwrap().len();
    let (status, retry) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["status"], "already_moved");
    assert_eq!(retry["from"], "Claude plan B");
    assert_eq!(retry["to"], "Claude plan A");
    assert_eq!(h.factory.copies.lock().unwrap().len(), copies);
    assert_eq!(h.children().len(), 2);
}

#[tokio::test]
async fn a_chat_without_a_running_child_moves_without_stopping_anything() {
    let h = Harness::new().await;
    let (status, reply) = h.move_to("s", A).await;
    // Its conversation exists but no activity proves a finished turn, so it is not idle yet.
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert_eq!(reply["code"], "settling");
    h.settle("s");
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(h.shutdowns.load(Ordering::SeqCst), 0);
    assert_eq!(h.session("s").account_key.as_deref(), Some(A));
}

#[tokio::test]
async fn a_busy_chat_is_refused_with_what_is_running_and_stays_where_it_is() {
    let mut h = Harness::new().await;
    // A turn in flight.
    let id = "00000000-0000-4000-8000-000000000001";
    let (status, _) = h
        .command(
            "sendMessage",
            json!({"sessionId":"s","messageId":id,"content":"work"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.inputs.recv().await.unwrap(), id);
    let events = h.children()[0].events.clone();
    events
        .send(Event::CommandLifecycle {
            command_uuid: id.into(),
            state: LifecycleState::Started,
        })
        .unwrap();
    h.wait(|s| s.turn_active).await;
    h.settle("s");
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert_eq!(reply["code"], "not_idle");
    let text = message(&reply);
    assert!(text.contains("a turn is running"), "{text}");
    assert!(text.contains("stays on Claude plan B"), "{text}");
    assert!(text.contains("Try again when it is idle."), "{text}");
    h.assert_unmoved("s", B);
    assert!(h.factory.copies.lock().unwrap().is_empty());
    assert_eq!(h.shutdowns.load(Ordering::SeqCst), 0);

    // The same chat, finished a moment ago: the quiet period has not passed.
    events
        .send(Event::Result {
            subtype: None,
            is_error: Some(false),
        })
        .unwrap();
    h.wait(|s| !s.turn_active).await;
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert_eq!(reply["code"], "settling");
    assert!(message(&reply).contains("less than 20 seconds"), "{reply}");
    h.assert_unmoved("s", B);
}

#[tokio::test]
async fn open_subagents_and_a_stopped_chat_refuse_the_move() {
    let h = Harness::with(|store, project| {
        store.create_session("kid", project, 2).unwrap();
    })
    .await;
    rusqlite::Connection::open(h.store.path())
        .unwrap()
        .execute(
            "UPDATE sessions SET parent_session_id = 's', root_session_id = 's' WHERE id = 'kid'",
            [],
        )
        .unwrap();
    h.settle("s");
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert!(
        message(&reply).contains("1 subagents are still open"),
        "{reply}"
    );
    h.assert_unmoved("s", B);
    // A subagent never moves on its own.
    let (status, reply) = h.move_to("kid", A).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(message(&reply).contains("is a subagent"), "{reply}");
    // Once it is archived the root can move.
    let (status, _) = h
        .command("archiveSession", json!({"sessionId":"kid"}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    // A stopped chat is the owner's own decision and must be resumed first.
    let h = Harness::new().await;
    h.settle("s");
    let (status, _) = h.command("stopSession", json!({"sessionId":"s"})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::CONFLICT, "{reply}");
    assert!(message(&reply).contains("the owner stopped it"), "{reply}");
    h.assert_unmoved("s", B);
}

#[tokio::test]
async fn only_enabled_claude_plan_destinations_for_claude_plan_chats_are_accepted() {
    let h = Harness::with(|store, project| {
        store.create_session("api", project, 2).unwrap();
        store.set_claude_account_key("api", API).unwrap();
        store
            .create_session_in(boblm_store::NewSession {
                id: "codex",
                project_id: &store.reader().unwrap().projects().unwrap()[0].id,
                name: "",
                agent: "codex",
                model: "gpt-6.1-sol",
                effort: None,
                speed: None,
                created_at: 3,
            })
            .unwrap();
    })
    .await;
    h.settle("s");
    for (session, target, needle) in [
        ("api", A, "Claude API account"),
        ("codex", A, "runs Codex"),
        ("s", B, "already runs on Claude plan B"),
        ("s", API, "is not a Claude plan account"),
        ("s", "claude-plan-c", "is not a Claude plan account"),
    ] {
        let (status, reply) = h.move_to(session, target).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{session} {target}: {reply}"
        );
        assert!(message(&reply).contains(needle), "{reply}");
        assert!(message(&reply).contains("Nothing was changed"), "{reply}");
    }
    // A target that is not enabled on this server.
    h.app.set_claude_accounts_for_tests(&[A], &[API]);
    let (status, reply) = h.move_to("s", B).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        message(&reply).contains("Claude plan B is not enabled"),
        "{reply}"
    );
    // A target with no signed-in account cannot resume a conversation.
    h.app.set_claude_accounts_for_tests(&[A, B], &[API]);
    h.factory.logged_out.lock().unwrap().push(A.into());
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        message(&reply).contains("no signed-in Claude account"),
        "{reply}"
    );
    h.assert_unmoved("s", B);
    assert_eq!(h.session("api").account_key.as_deref(), Some(API));
    assert!(h.factory.copies.lock().unwrap().is_empty());
    // Malformed requests are rejected by the contract, not the move.
    let (status, _) = h
        .command(
            "moveClaudeAccount",
            json!({"sessionId":"s","accountKey":A,"extra":1}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = h.move_to("missing", A).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_failed_copy_keeps_the_source_account_and_leaves_the_chat_usable() {
    let mut h = Harness::new().await;
    h.run_a_turn("00000000-0000-4000-8000-000000000001").await;
    h.settle("s");
    h.factory.fail_copy.store(true, Ordering::SeqCst);
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{reply}");
    assert_eq!(reply["code"], "move_failed");
    let text = message(&reply);
    assert!(
        text.contains("could not be copied to Claude plan A"),
        "{text}"
    );
    assert!(text.contains("stays on Claude plan B"), "{text}");
    h.assert_unmoved("s", B);
    // The chat still works on its source account and resumes the same conversation there.
    let id = "00000000-0000-4000-8000-000000000002";
    let (status, _) = h
        .command(
            "sendMessage",
            json!({"sessionId":"s","messageId":id,"content":"still here"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.inputs.recv().await.unwrap(), id);
    let children = h.children();
    assert_eq!(children.last().unwrap().key.as_deref(), Some(B));
    assert_eq!(
        children.last().unwrap().resume.as_deref(),
        Some(CONVERSATION)
    );
    // A later attempt, once the transcript is restored, succeeds from the same state.
    h.factory.fail_copy.store(false, Ordering::SeqCst);
    let events = children.last().unwrap().events.clone();
    events
        .send(Event::CommandLifecycle {
            command_uuid: id.into(),
            state: LifecycleState::Started,
        })
        .unwrap();
    events
        .send(Event::Result {
            subtype: None,
            is_error: Some(false),
        })
        .unwrap();
    h.wait(|s| !s.turn_active).await;
    h.settle("s");
    let (status, reply) = h.move_to("s", A).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
}

#[test]
fn the_script_mirrors_the_compaction_threshold_in_tuning() {
    let script = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bin/move-chat-account"
    ))
    .unwrap();
    assert!(
        script.contains(&format!(
            "COMPACT_PERCENT = {}",
            boblm_tuning::CLAUDE_MOVE_COMPACT_PERCENT
        )),
        "bin/move-chat-account must mirror CLAUDE_MOVE_COMPACT_PERCENT"
    );
}

/// Read one HTTP request, headers and the whole body, since they can arrive separately.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let length = socket.read(&mut chunk).await.unwrap();
        bytes.extend_from_slice(&chunk[..length]);
        let text = String::from_utf8_lossy(&bytes).to_string();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let wanted = head
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if body.len() >= wanted {
                return text;
            }
        }
        if length == 0 {
            return text;
        }
    }
}

/// A stand-in server that answers listSessions and records every command it receives.
async fn mock_server(chat: Value) -> (u16, Arc<Mutex<Vec<String>>>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            record.lock().unwrap().push(path);
            let body = json!({"cursor":0,"models":[],"projects":[],"sessions":[chat]}).to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    (port, seen)
}
fn chat(turn_active: bool) -> Value {
    json!({
        "sessionId":"abcdef12-0000-4000-8000-000000000001","name":"Big chat","agent":"claude",
        "status":{"accountKey":"claude-plan-b","turnActive":turn_active,"stopping":false,
            "queued":0,"dispatching":0,"uncertain":0,"archived":false,"running":true,
            "jobs":{"tasks":[],"available":true},
            "usage":{"context":{"usedTokens":940,"windowTokens":1000},"contextUpdatedAt":1}},
        "subagents":{"open":0}
    })
}
async fn script(port: u16, args: &[&str]) -> (i32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("boblm.json");
    std::fs::write(&config, json!({"port":port,"token":TOKEN}).to_string()).unwrap();
    let output = tokio::process::Command::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bin/move-chat-account"
    ))
    .args(args)
    .env("BOBLM_CONFIG", &config)
    .output()
    .await
    .unwrap();
    let (out, err) = (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    );
    assert!(
        !out.contains(TOKEN) && !err.contains(TOKEN),
        "the token must never be printed"
    );
    (output.status.code().unwrap(), out, err)
}

#[tokio::test]
async fn the_script_plans_and_dry_runs_without_changing_anything() {
    let (port, seen) = mock_server(chat(false)).await;
    let (code, out, _) = script(port, &["abcd", "a", "--dry-run"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("from Claude plan B to Claude plan A"), "{out}");
    assert!(
        out.contains("94%") && out.contains("compact first"),
        "{out}"
    );
    assert!(out.contains("Nothing was changed"), "{out}");
    let (code, out, _) = script(port, &["abcd", "a", "--dry-run", "--no-compact"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no compaction"), "{out}");
    // Only the read-only chat list was ever requested.
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|p| p == "/commands/listSessions"),
        "{:?}",
        seen.lock().unwrap()
    );

    // A busy chat is refused at once without --wait, and a real run sends nothing either.
    let (port, seen) = mock_server(chat(true)).await;
    let (code, out, err) = script(port, &["abcd", "a"]).await;
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        err.contains("a turn is running") && err.contains("Try again when it is idle."),
        "{err}"
    );
    let (code, _, err) = script(port, &["abcd", "a", "--dry-run"]).await;
    assert_eq!(code, 1, "{err}");
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|p| p == "/commands/listSessions"),
        "{:?}",
        seen.lock().unwrap()
    );
}

#[tokio::test]
async fn the_script_rejects_bad_arguments_before_contacting_the_server() {
    for args in [
        vec![],
        vec!["abcd"],
        vec!["abcd", "c"],
        vec!["abcd", "a", "--wait", "soon"],
        vec!["abcd", "a", "--bogus"],
    ] {
        let (code, _, err) = script(1, &args).await;
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains("Usage: bin/move-chat-account"), "{err}");
    }
    let (port, _) = mock_server(chat(false)).await;
    let (code, _, err) = script(port, &["zzzz", "a", "--dry-run"]).await;
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("No chat has an ID starting with zzzz"),
        "{err}"
    );
}

/// A stand-in that plays a chat at 94% context: /compact makes it busy for a few polls and
/// leaves it at 12%, and the first move request is refused as not yet settled.
#[tokio::test]
async fn the_script_compacts_a_full_chat_waits_for_it_to_settle_and_then_moves() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        let (mut phase, mut settling) = (0, true);
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            let content = request.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
            record.lock().unwrap().push(format!("{path} {content}"));
            let (code, body) = match path.as_str() {
                "/commands/sendMessage" => {
                    phase = 1;
                    (200, json!({"messageId":"x","accepted":true}))
                }
                "/commands/moveClaudeAccount" if settling => {
                    settling = false;
                    (
                        409,
                        json!({"code":"settling","message":"The chat is not settled yet."}),
                    )
                }
                "/commands/moveClaudeAccount" => (
                    200,
                    json!({"sessionId":"abcdef12-0000-4000-8000-000000000001","from":"Claude plan B","to":"Claude plan A","status":"moved"}),
                ),
                _ => {
                    let mut value = chat(false);
                    if (1..4).contains(&phase) {
                        value["status"]["turnActive"] = json!(true);
                    }
                    if phase >= 4 {
                        value["status"]["usage"] = json!({"context":{"usedTokens":120,"windowTokens":1000},"contextUpdatedAt":2});
                    }
                    if phase >= 1 {
                        phase += 1;
                    }
                    (
                        200,
                        json!({"cursor":0,"models":[],"projects":[],"sessions":[value]}),
                    )
                }
            };
            let body = body.to_string();
            let reply = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("boblm.json");
    std::fs::write(&config, json!({"port":port,"token":TOKEN}).to_string()).unwrap();
    let output = tokio::process::Command::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bin/move-chat-account"
    ))
    .args(["abcd", "a"])
    .env("BOBLM_CONFIG", &config)
    .env("BOBLM_MOVE_POLL_SECONDS", "0.05")
    .output()
    .await
    .unwrap();
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{out}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.contains("Sent /compact"), "{out}");
    assert!(out.contains("Moved to Claude plan A"), "{out}");
    assert!(out.contains("Context is 12%"), "{out}");
    let commands: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|line| !line.starts_with("/commands/listSessions"))
        .cloned()
        .collect();
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(commands[0].starts_with("/commands/sendMessage") && commands[0].contains("/compact"));
    assert!(
        commands[1].starts_with("/commands/moveClaudeAccount")
            && commands[1].contains("claude-plan-a")
    );
    assert!(
        commands[2].starts_with("/commands/moveClaudeAccount"),
        "the settling refusal is retried"
    );
}

/// A wait can last hours and BobLM is redeployed meanwhile, so failed list calls must not end it.
#[tokio::test]
async fn the_script_keeps_waiting_through_failed_list_calls() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut lists = 0;
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            let (code, body) = if path == "/commands/moveClaudeAccount" {
                (
                    200,
                    json!({"sessionId":"abcdef12-0000-4000-8000-000000000001","from":"Claude plan B","to":"Claude plan A","status":"moved"}),
                )
            } else {
                lists += 1;
                match lists {
                    // The first call finds the chat busy, two calls then fail as they did
                    // during a deploy, and the chat goes idle afterwards.
                    1 | 4 => (
                        200,
                        json!({"cursor":0,"models":[],"projects":[],"sessions":[chat(true)]}),
                    ),
                    2 | 3 => (
                        404,
                        json!({"code":"not_found","message":"There is no chat with ID \"gone\" on this server."}),
                    ),
                    _ => (
                        200,
                        json!({"cursor":0,"models":[],"projects":[],"sessions":[chat(false)]}),
                    ),
                }
            };
            let body = body.to_string();
            let reply = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("boblm.json");
    std::fs::write(&config, json!({"port":port,"token":TOKEN}).to_string()).unwrap();
    let output = tokio::process::Command::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bin/move-chat-account"
    ))
    .args(["abcd", "a", "--no-compact", "--wait", "30"])
    .env("BOBLM_CONFIG", &config)
    .env("BOBLM_MOVE_POLL_SECONDS", "0.05")
    .output()
    .await
    .unwrap();
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{out}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.contains("Could not list the chats"), "{out}");
    assert!(out.contains("Moved to Claude plan A"), "{out}");
}
