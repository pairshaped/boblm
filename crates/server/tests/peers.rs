use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use std::{io, sync::Arc};
use tower::ServiceExt;
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("peer discovery must not start Claude")
    }
}
#[tokio::test]
async fn mcp_discovery_requires_a_bound_caller_and_lists_other_sessions_without_starting_them() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("peers.sqlite")).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    store.create_session("peer", "/peer", 2).unwrap();
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let request = |token: &str| {
        Request::builder().method("POST").uri("/mcp/caller").header("Host","127.0.0.1").header("Authorization",format!("Bearer {token}")).header("Content-Type","application/json").header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_peers","arguments":{}}}"#)).unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(token))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let capability = boblm_server::mcp::session_token(token, "caller");
    let response = router.oneshot(request(&capability)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let text = value["result"]["content"][0]["text"].as_str().unwrap();
    let peers: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(peers["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(peers["sessions"][0]["sessionId"], "peer");
    assert_eq!(peers["sessions"][0]["status"]["running"], false);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_peers_labels_subagents_and_archiving_a_root_cascades() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("parents.sqlite")).unwrap();
    store.create_session("observer", "/observer", 1).unwrap();
    store.create_session("root", "/root", 2).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap();
    store
        .spawn_peer(boblm_store::PeerSpawn {
            request_id: "10000000-0000-4000-8000-000000000001".into(),
            caller: "root".into(),
            arguments: r#"{"message":"work"}"#.into(),
            project_id: project,
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            title: "Fixture chat".into(),
            content: "work".into(),
            created_at: 3,
            parent_session_id: Some("root".into()),
            root_session_id: Some("root".into()),
            role: Some("implementer".into()),
            resolved: None,
            working_directory: None,
            account_key: None,
        })
        .unwrap();
    let token = "peer-test-owner-token-only";
    let app = App::open(store.clone(), Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let listed = call(
        &router,
        token,
        "observer",
        "list_peers",
        serde_json::json!({}),
    )
    .await;
    let peers: serde_json::Value =
        serde_json::from_str(listed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let subagent = peers["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["sessionId"] == "10000000-0000-4000-8000-000000000001")
        .unwrap();
    assert_eq!(subagent["parentSessionId"], "root");
    assert_eq!(subagent["rootSessionId"], "root");
    assert_eq!(subagent["role"], "implementer");

    let archived = call(
        &router,
        token,
        "observer",
        "archive_peer",
        serde_json::json!({"sessionIdPrefix":"root"}),
    )
    .await;
    assert_eq!(archived["result"]["isError"], false);
    let sessions = store.reader().unwrap().sessions().unwrap();
    assert!(
        sessions
            .iter()
            .filter(|session| session.id != "observer")
            .all(|session| session.archived)
    );
    let deleted = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/deleteSession")
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"sessionId":"root"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    let reader = store.reader().unwrap();
    assert!(reader.session_deleted("root").unwrap());
    assert!(
        reader
            .session_deleted("10000000-0000-4000-8000-000000000001")
            .unwrap()
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn account_usage_tool_returns_every_account_and_the_calling_chat() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("accounts.sqlite")).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "caller")
        .and_then(|session| session.project_id)
        .unwrap();
    store
        .create_session_in(boblm_store::NewSession {
            id: "codex-peer",
            project_id: &project,
            name: "",
            agent: "codex",
            model: "gpt-6.1-sol",
            effort: None,
            created_at: 2,
        })
        .unwrap();
    store
        .set_session_account("caller", Some("claude-account"))
        .unwrap();
    store
        .set_session_account("codex-peer", Some("codex-account"))
        .unwrap();
    store
        .record_event(
            "caller",
            "agent_event",
            serde_json::json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}}),
            10_000,
        )
        .unwrap();
    store
        .record_event(
            "codex-peer",
            "agent_event",
            serde_json::json!({"type":"usage","five_hour":{"usedPercent":80,"resetsAt":2000000000},"credits":{"hasCredits":true,"unlimited":false,"balance":40000}}),
            10_000,
        )
        .unwrap();
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let reply = call(
        &router,
        token,
        "caller",
        "account_usage",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false);
    let report: serde_json::Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["caller"]["provider"], "claude");
    assert_eq!(report["caller"]["accountId"], "claude-account");
    let accounts = report["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    let claude = accounts
        .iter()
        .find(|account| account["provider"] == "claude")
        .unwrap();
    assert_eq!(claude["weekly"]["usedPercent"], 42.0);
    assert!(claude["weekly"]["windowElapsedPercent"].is_number());
    let codex = accounts
        .iter()
        .find(|account| account["provider"] == "codex")
        .unwrap();
    assert_eq!(codex["credits"]["balance"], 40000.0);
    let filtered = call(
        &router,
        token,
        "caller",
        "account_usage",
        serde_json::json!({"provider":"codex"}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(filtered["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["accounts"].as_array().unwrap().len(), 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn usage_report_groups_fixture_events_by_model_week_and_account_day() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("report.sqlite")).unwrap();
    store.create_session("claude-chat", "/local", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "claude-chat")
        .and_then(|session| session.project_id)
        .unwrap();
    for (id, agent, model) in [
        ("codex-chat", "codex", "gpt-6.1-sol"),
        ("pi-chat", "pi", "deepseek-ai/DeepSeek-V4.1-Flash-1017"),
    ] {
        store
            .create_session_in(boblm_store::NewSession {
                id,
                project_id: &project,
                name: "",
                agent,
                model,
                effort: None,
                created_at: 2,
            })
            .unwrap();
    }
    store
        .set_session_account("claude-chat", Some("claude-account"))
        .unwrap();
    store
        .set_session_account("codex-chat", Some("codex-account"))
        .unwrap();
    let day_one = 1_793_361_600_000_i64; // 2026-10-30 12:00 UTC
    let day_two = 1_793_448_000_000_i64; // 2026-10-31 12:00 UTC
    // Claude repeats the message usage for each content block; the final output wins.
    for (offset, output) in [(0, 100), (1, 200)] {
        store
            .record_event(
                "claude-chat",
                "agent_event",
                serde_json::json!({"type":"assistant","message":{"id":"c1","model":"sonnet","content":[],"usage":{"input_tokens":1000,"output_tokens":output,"cache_read_input_tokens":500,"cache_creation_input_tokens":100}}}),
                day_two + offset,
            )
            .unwrap();
    }
    store
        .record_event(
            "pi-chat",
            "agent_event",
            serde_json::json!({"type":"assistant","message":{"id":"p1","model":"deepseek-ai/DeepSeek-V4.1-Flash-1017","content":[],"usage":{"input_tokens":2000,"output_tokens":300,"cache_read_input_tokens":800,"cache_creation_input_tokens":0}}}),
            day_two,
        )
        .unwrap();
    store
        .record_event(
            "codex-chat",
            "agent_event",
            serde_json::json!({"type":"token_usage","model":"gpt-6.1-sol","input_tokens":4000,"cached_input_tokens":1000,"cache_write_input_tokens":50,"output_tokens":400}),
            day_two,
        )
        .unwrap();
    for (session, used, at) in [
        ("claude-chat", 40.0, day_one),
        ("claude-chat", 42.0, day_two),
        ("codex-chat", 90.0, day_two),
    ] {
        store
            .record_event(
                session,
                "agent_event",
                serde_json::json!({"type":"usage","weekly":{"usedPercent":used,"resetsAt":1793800800}}),
                at,
            )
            .unwrap();
    }
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let reply = call(
        &router,
        token,
        "claude-chat",
        "usage_report",
        serde_json::json!({"since":"2026-10-25","until":"2026-10-31"}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false);
    let report: serde_json::Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["since"], "2026-10-25");
    assert_eq!(report["until"], "2026-10-31");
    let models = report["models"].as_array().unwrap();
    assert_eq!(models.len(), 3);
    let claude = models
        .iter()
        .find(|model| model["provider"] == "claude")
        .unwrap();
    assert_eq!(claude["week"], "2026-10-26");
    assert_eq!(claude["inputTokens"], 1000);
    assert_eq!(claude["cacheReadTokens"], 500);
    assert_eq!(claude["cacheWriteTokens"], 100);
    assert_eq!(claude["outputTokens"], 200);
    assert_eq!(claude["calls"], 1);
    assert!(claude.get("estimatedCostUsd").is_none());
    let codex = models
        .iter()
        .find(|model| model["provider"] == "codex")
        .unwrap();
    assert_eq!(codex["inputTokens"], 4000);
    assert_eq!(codex["cacheReadTokens"], 1000);
    assert_eq!(codex["cacheWriteTokens"], 50);
    assert_eq!(codex["outputTokens"], 400);
    let pi = models
        .iter()
        .find(|model| model["provider"] == "pi")
        .unwrap();
    let cost = pi["estimatedCostUsd"].as_f64().unwrap();
    assert!((cost - 0.000585).abs() < 0.0000001, "{cost}");
    let allowance = report["allowanceDaily"].as_array().unwrap();
    let claude_day = allowance
        .iter()
        .find(|row| row["provider"] == "claude" && row["day"] == "2026-10-31")
        .unwrap();
    assert_eq!(claude_day["weeklyUsedPercent"], 42.0);
    assert_eq!(claude_day["previousDay"], "2026-10-30");
    assert_eq!(claude_day["dailyConsumedPercent"], 2.0);
    let codex_day = allowance
        .iter()
        .find(|row| row["provider"] == "codex" && row["day"] == "2026-10-31")
        .unwrap();
    assert!(codex_day["dailyConsumedPercent"].is_null());
    let filtered = call(
        &router,
        token,
        "claude-chat",
        "usage_report",
        serde_json::json!({"since":"2026-10-25","until":"2026-10-31","provider":"pi"}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(filtered["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["models"].as_array().unwrap().len(), 1);
    assert_eq!(report["allowanceDaily"].as_array().unwrap().len(), 0);
    let bad = call(
        &router,
        token,
        "claude-chat",
        "usage_report",
        serde_json::json!({"since":"yesterday"}),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true);
    app.shutdown().await.unwrap();
}

async fn call(
    router: &axum::Router,
    owner_token: &str,
    caller: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let capability = boblm_server::mcp::session_token(owner_token, caller);
    let response=router.clone().oneshot(Request::builder().method("POST").uri(format!("/mcp/{caller}")).header("Host","127.0.0.1").header("Authorization",format!("Bearer {capability}")).header("Content-Type","application/json").header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").body(Body::from(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn tool_names(router: &axum::Router, owner_token: &str, caller: &str) -> Vec<String> {
    let capability = boblm_server::mcp::session_token(owner_token, caller);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/mcp/{caller}"))
                .header("Host", "127.0.0.1")
                .header("Authorization", format!("Bearer {capability}"))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-11-25")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    value["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}
#[tokio::test]
async fn change_title_is_bound_to_caller_and_persists_across_app_restart() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = temp.path().join("title.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    store.create_session("peer", "/peer", 2).unwrap();
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let changed = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Caller task"}),
    )
    .await;
    assert_eq!(changed["result"]["isError"], false);
    let injected = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Other task","sessionId":"peer"}),
    )
    .await;
    assert!(injected.get("error").is_some() || injected["result"]["isError"] == true);
    app.shutdown().await.unwrap();
    drop(router);
    drop(app);
    let app = App::open(
        Store::open(path).unwrap(),
        Arc::new(NeverSpawn),
        token.into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let inspected = call(
        &router,
        token,
        "peer",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"call"}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["session"]["name"], "Caller task");
    assert_eq!(report["session"]["status"]["running"], false);
    app.shutdown().await.unwrap();
}

struct PingInput {
    uuid: String,
    content: String,
    priority: Option<boblm_protocol::Priority>,
}
struct Fake {
    hold_events_until_abort: bool,
    emit: tokio::sync::mpsc::UnboundedSender<boblm_protocol::Event>,
    events: tokio::sync::mpsc::UnboundedReceiver<boblm_protocol::Event>,
    inputs: tokio::sync::mpsc::UnboundedSender<PingInput>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(
        &mut self,
        uuid: &str,
        content: &str,
        priority: Option<boblm_protocol::Priority>,
    ) -> io::Result<()> {
        self.inputs
            .send(PingInput {
                uuid: uuid.into(),
                content: content.into(),
                priority,
            })
            .map_err(io::Error::other)
    }
    async fn next_event(&mut self) -> Option<boblm_protocol::Event> {
        if self.hold_events_until_abort {
            std::future::pending::<()>().await;
        }
        self.events.recv().await
    }
    async fn configure(&mut self, _: &str, _: Option<&str>) -> io::Result<()> {
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.hold_events_until_abort = false;
        self.emit
            .send(boblm_protocol::Event::Result {
                subtype: Some("interrupted".into()),
                is_error: Some(false),
            })
            .map_err(io::Error::other)
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        self.events.close();
        Ok(())
    }
}
struct Factory(tokio::sync::Mutex<Option<Fake>>);
#[async_trait]
impl DriverFactory for Factory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Ok(Box::new(self.0.lock().await.take().ok_or_else(|| {
            io::Error::other("unexpected second spawn")
        })?))
    }
}
struct SpawnFactory {
    inputs: tokio::sync::mpsc::UnboundedSender<PingInput>,
}
#[async_trait]
impl DriverFactory for SpawnFactory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        let (emit, events) = tokio::sync::mpsc::unbounded_channel();
        Ok(Box::new(Fake {
            hold_events_until_abort: false,
            emit,
            events,
            inputs: self.inputs.clone(),
        }))
    }
}
#[tokio::test]
async fn ping_is_idempotent_steers_active_peers_and_archive_preserves_inspectable_history() {
    use boblm_protocol::{Event, LifecycleState, Priority};
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("ping.sqlite")).unwrap();
    for id in ["caller", "peer-one", "peer-two"] {
        store.create_session(id, "/local", 1).unwrap();
    }
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(Factory(tokio::sync::Mutex::new(Some(Fake {
        // Queue the acknowledgment until abort so it is drained with the result.
        // Archive must publish that final delivery state before replying.
        hold_events_until_abort: true,
        emit: emit.clone(),
        events,
        inputs,
    }))));
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let ambiguous = call(
        &router,
        token,
        "caller",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"peer-"}),
    )
    .await;
    assert_eq!(ambiguous["result"]["isError"], true);
    let missing = call(
        &router,
        token,
        "caller",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"absent"}),
    )
    .await;
    assert_eq!(missing["result"]["isError"], true);
    let uuid = "00000000-0000-4000-8000-000000000001";
    let args = serde_json::json!({"sessionIdPrefix":"peer-one","messageId":uuid,"message":"first"});
    assert_eq!(
        call(&router, token, "caller", "ping_peer", args.clone()).await["result"]["isError"],
        false
    );
    let first = received.recv().await.unwrap();
    assert_eq!(first.priority, None);
    assert_eq!(first.uuid, uuid);
    assert!(first.content.contains("boblm session caller"));
    assert!(first.content.contains("reply_to_sender"));
    let recorded = app
        .handle_for_tests("peer-one")
        .await
        .events_since(0, 100)
        .await
        .unwrap();
    let message = recorded
        .iter()
        .find(|event| event.kind == "message_appended")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&message.payload).unwrap();
    assert_eq!(payload["sender_session_id"], "caller");
    emit.send(Event::CommandLifecycle {
        command_uuid: uuid.into(),
        state: LifecycleState::Started,
    })
    .unwrap();
    assert_eq!(
        call(&router, token, "caller", "ping_peer", args).await["result"]["isError"],
        false
    );
    assert!(received.try_recv().is_err());
    let conflict = call(
        &router,
        token,
        "caller",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":"peer-one","messageId":uuid,"message":"different"}),
    )
    .await;
    assert_eq!(conflict["result"]["isError"], true);
    assert!(received.try_recv().is_err());
    let next = "00000000-0000-4000-8000-000000000002";
    assert_eq!(
        call(
            &router,
            token,
            "caller",
            "ping_peer",
            serde_json::json!({"sessionIdPrefix":"peer-one","messageId":next,"message":"steer"})
        )
        .await["result"]["isError"],
        false
    );
    assert_eq!(
        received.recv().await.unwrap().priority,
        Some(Priority::Next)
    );
    assert_eq!(
        call(
            &router,
            token,
            "caller",
            "archive_peer",
            serde_json::json!({"sessionIdPrefix":"peer-one"})
        )
        .await["result"]["isError"],
        false
    );
    let inspected = call(
        &router,
        token,
        "caller",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"peer-one"}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["session"]["status"]["archived"], true);
    assert_eq!(report["session"]["status"]["uncertain"], 1);
    assert_eq!(report["messages"].as_array().unwrap().len(), 2);
    let inspected_messages = report["messages"].as_array().unwrap();
    assert_eq!(
        inspected_messages
            .iter()
            .find(|m| m["uuid"] == uuid)
            .unwrap()["delivery_state"],
        "started"
    );
    assert_eq!(
        inspected_messages
            .iter()
            .find(|m| m["uuid"] == next)
            .unwrap()["delivery_state"],
        "uncertain"
    );
    let retry = call(
        &router,
        token,
        "caller",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":"peer-one","messageId":next.to_uppercase(),"message":"steer"}),
    )
    .await;
    assert_eq!(
        retry["result"]["isError"], false,
        "identical retry after archive"
    );
    let retry = tool_value(&retry);
    assert_eq!(retry["alreadyAccepted"], true);
    assert_eq!(retry["messageId"], next);
    assert_eq!(
        call(
            &router,
            token,
            "caller",
            "ping_peer",
            serde_json::json!({"sessionIdPrefix":"peer-one","messageId":"00000000-0000-4000-8000-0000000000ff","message":"new input"})
        )
        .await["result"]["isError"],
        true
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_parent_reopens_its_own_archived_subagent_but_another_chat_cannot() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("reopen.sqlite")).unwrap();
    for id in ["root", "other"] {
        store.create_session(id, "/local", 1).unwrap();
    }
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap();
    let child = "10000000-0000-4000-8000-000000000777";
    store
        .spawn_peer(boblm_store::PeerSpawn {
            request_id: child.into(),
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
    store.transition(child, "queued", "started").unwrap();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let token = "peer-test-owner-token-only";
    let app = App::open(
        store.clone(),
        Arc::new(SpawnFactory { inputs }),
        token.into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let archived = call(
        &router,
        token,
        "root",
        "archive_peer",
        serde_json::json!({"sessionIdPrefix":child}),
    )
    .await;
    assert_eq!(archived["result"]["isError"], false);
    let ping = call(
        &router,
        token,
        "root",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":child,"messageId":"00000000-0000-4000-8000-000000000001","message":"one more thing"}),
    )
    .await;
    assert_eq!(ping["result"]["isError"], false, "{ping}");
    loop {
        let input = received.recv().await.unwrap();
        if input.content.contains("one more thing") {
            break;
        }
    }
    let reopened = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == child)
        .unwrap();
    assert!(!reopened.archived);

    let archived = call(
        &router,
        token,
        "root",
        "archive_peer",
        serde_json::json!({"sessionIdPrefix":child}),
    )
    .await;
    assert_eq!(archived["result"]["isError"], false);
    let refused = call(
        &router,
        token,
        "other",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":child,"messageId":"00000000-0000-4000-8000-000000000002","message":"not mine"}),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Only its parent can reopen")
    );
    let still_archived = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == child)
        .unwrap();
    assert!(still_archived.archived);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_root_reports_subagent_state_and_answers_its_questions() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("subagent-state.sqlite")).unwrap();
    store.create_session("root", "/local", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap();
    let child = "10000000-0000-4000-8000-000000000888";
    store
        .spawn_peer(boblm_store::PeerSpawn {
            request_id: child.into(),
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
    store.transition(child, "queued", "started").unwrap();
    let question = "20000000-0000-4000-8000-000000000888";
    store.ask(question, child, "Which way?", &[], 3).unwrap();
    store
        .record_event(
            child,
            "agent_event",
            serde_json::json!({"type":"result","is_error":true}),
            4,
        )
        .unwrap();
    store
        .record_event(
            child,
            "subagent_report_failed",
            serde_json::json!({"message":"fixture"}),
            5,
        )
        .unwrap();
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(Factory(tokio::sync::Mutex::new(Some(Fake {
        hold_events_until_abort: false,
        emit,
        events,
        inputs,
    }))));
    let token = "peer-test-owner-token-only";
    let app = App::open(store.clone(), factory, token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let post = |path: &str, body: serde_json::Value| {
        let router = router.clone();
        let token = token.to_owned();
        let path = path.to_owned();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Content-Type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        }
    };
    let sessions = post("/commands/listSessions", serde_json::json!({})).await;
    let root = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["sessionId"] == "root")
        .unwrap();
    assert_eq!(root["subagents"]["open"], 1);
    assert_eq!(root["subagents"]["questions"], 1);
    assert_eq!(root["subagents"]["failures"], 1);
    assert_eq!(root["subagents"]["reportFailures"], 1);

    let questions = post(
        "/commands/listQuestions",
        serde_json::json!({"sessionId":"root"}),
    )
    .await;
    assert_eq!(questions["pendingCount"], 1);
    assert_eq!(questions["questions"][0]["sessionId"], child);
    assert_eq!(questions["questions"][0]["id"], question);

    let answered = post(
        "/commands/answerQuestion",
        serde_json::json!({"sessionId":"root","questionId":question,"answer":"That way"}),
    )
    .await;
    assert_eq!(answered["pendingCount"], 0);
    let input = received
        .recv()
        .await
        .expect("the child receives its answer");
    assert!(input.content.contains("Owner's answer"));
    assert!(input.content.contains("That way"));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn spawn_requires_model_and_effort_and_inherits_directory_only() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("spawn.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    store
        .set_model_effort("caller", "sonnet", Some("low"))
        .unwrap();
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(Factory(tokio::sync::Mutex::new(Some(Fake {
        hold_events_until_abort: false,
        emit,
        events,
        inputs,
    }))));
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let id = "00000000-0000-4000-8000-000000000003";
    let args = serde_json::json!({"title":"Spawned chat","requestId":id,"message":"Do this independently","model":"opus","effort":"high"});
    let mut no_title = args.clone();
    no_title.as_object_mut().unwrap().remove("title");
    let invalid = call(&router, token, "caller", "spawn_peer", no_title).await;
    assert!(invalid.get("error").is_some() || invalid["result"]["isError"] == true);
    for field in ["model", "effort"] {
        let mut missing = args.clone();
        missing.as_object_mut().unwrap().remove(field);
        let invalid = call(&router, token, "caller", "spawn_peer", missing).await;
        assert!(invalid.get("error").is_some() || invalid["result"]["isError"] == true);
        let mut null = args.clone();
        null[field] = serde_json::Value::Null;
        let invalid = call(&router, token, "caller", "spawn_peer", null).await;
        assert!(invalid.get("error").is_some() || invalid["result"]["isError"] == true);
    }
    let (spawned, simultaneous) = tokio::join!(
        call(&router, token, "caller", "spawn_peer", args.clone()),
        call(&router, token, "caller", "spawn_peer", args.clone())
    );
    assert_eq!(simultaneous["result"]["isError"], false);
    assert_eq!(spawned["result"]["isError"], false);
    let report: serde_json::Value =
        serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["sessionId"], id);
    assert_eq!(
        report["session"]["workingDirectory"],
        temp.path().to_str().unwrap()
    );
    assert_eq!(report["session"]["name"], "Spawned chat");
    assert_eq!(report["session"]["status"]["model"], "opus");
    assert_eq!(report["session"]["status"]["effort"], "high");
    let first = received.recv().await.unwrap();
    assert_eq!(first.uuid, id);
    assert_eq!(first.priority, None);
    assert!(first.content.contains("Do this independently"));
    assert!(first.content.contains("reply_to_sender"));
    let duplicate = call(&router, token, "caller", "spawn_peer", args.clone()).await;
    assert_eq!(duplicate["result"]["isError"], false);
    assert!(received.try_recv().is_err());
    let conflict = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"message":"changed"}),
    )
    .await;
    assert!(conflict.get("error").is_some() || conflict["result"]["isError"] == true);
    let mut title_conflict = args.clone();
    title_conflict["title"] = serde_json::json!("Different title");
    let conflict = call(&router, token, "caller", "spawn_peer", title_conflict).await;
    assert_eq!(conflict["result"]["isError"], true);
    let mut conflict = args;
    conflict["effort"] = serde_json::json!("low");
    let conflict = call(&router, token, "caller", "spawn_peer", conflict).await;
    assert_eq!(conflict["result"]["isError"], true);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn role_spawn_surface_follows_the_cutover_gate() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("role-gate.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let names = tool_names(&router, token, "caller").await;
    assert!(
        names.iter().any(|name| name == "spawn_subagent"),
        "the cutover exposes role subagents by default: {names:?}"
    );
    app.set_role_subagents_enabled_for_tests(false);
    let names = tool_names(&router, token, "caller").await;
    assert!(
        !names.iter().any(|name| name == "spawn_subagent"),
        "{names:?}"
    );
    let refused = call(
        &router,
        token,
        "caller",
        "spawn_subagent",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000301","message":"hidden","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not enabled yet")
    );
    let role_peer = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000302","message":"role root","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(role_peer["result"]["isError"], true);
    assert!(
        role_peer["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Role routing is not enabled yet")
    );
    let plain = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000303","message":"plain","model":"sonnet","effort":"low"}),
    )
    .await;
    assert_eq!(plain["result"]["isError"], false);

    app.set_role_subagents_enabled_for_tests(true);
    let names = tool_names(&router, token, "caller").await;
    assert!(
        names.iter().any(|name| name == "spawn_subagent"),
        "{names:?}"
    );
    let allowed = call(
        &router,
        token,
        "caller",
        "spawn_subagent",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000304","message":"now allowed","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(allowed["result"]["isError"], false);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn role_subagents_resolve_once_refuse_nesting_and_enforce_the_limit() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("role-subagents.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    store
        .set_session_account(
            "caller",
            Some(
                &boblm_claude_driver::account::account_id()
                    .unwrap_or_else(|| "claude-account".into()),
            ),
        )
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store.clone(), factory, token.into())
        .await
        .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    let first_id = "00000000-0000-4000-8000-000000000101";
    let args = serde_json::json!({"title":"Spawned chat","requestId":first_id,"message":"implement this","role":"implementer","difficulty":"simple"});
    let first = tool_value(&call(&router, token, "caller", "spawn_subagent", args.clone()).await);
    assert_eq!(first["routing"]["model"], "gpt-6.1-sol");
    assert_eq!(first["routing"]["effort"], "medium");
    assert_eq!(first["routing"]["account"], "codex-plan");
    assert_eq!(first["routing"]["step"], "implementer-codex-plan");
    assert_eq!(first["session"]["parentSessionId"], "caller");
    assert_eq!(first["session"]["rootSessionId"], "caller");
    assert_eq!(first["session"]["role"], "implementer");
    assert_eq!(first["openSubagents"].as_array().unwrap().len(), 1);
    let stored = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == first_id)
        .unwrap();
    assert_eq!(stored.account_key.as_deref(), Some("codex-plan"));

    // A retry uses the stored route even after the allowances would choose DeepSeek.
    store
        .record_event(
            "caller",
            "agent_event",
            serde_json::json!({"type":"usage","weekly":{"usedPercent":100,"resetsAt":2_000_000_000}}),
            1_793_448_000_000_i64,
        )
        .unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "caller")
        .and_then(|session| session.project_id)
        .unwrap();
    store
        .create_session_in(boblm_store::NewSession {
            id: "codex-meter",
            project_id: &project,
            name: "",
            agent: "codex",
            model: "gpt-6.1-sol",
            effort: None,
            created_at: 2,
        })
        .unwrap();
    store
        .set_session_account("codex-meter", Some("codex-account"))
        .unwrap();
    store
        .record_event(
            "codex-meter",
            "agent_event",
            serde_json::json!({"type":"usage","weekly":{"usedPercent":100,"resetsAt":2_000_000_000},"credits":{"hasCredits":true,"unlimited":false,"balance":0}}),
            1_793_448_000_000_i64,
        )
        .unwrap();
    let repeated = tool_value(&call(&router, token, "caller", "spawn_subagent", args).await);
    assert_eq!(repeated["alreadyAccepted"], true);
    assert_eq!(repeated["routing"]["model"], "gpt-6.1-sol");
    assert_eq!(repeated["routing"]["step"], "implementer-codex-plan");

    let nested = call(
        &router,
        token,
        first_id,
        "spawn_subagent",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000102","message":"nested","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(nested["result"]["isError"], true);

    for (index, id) in [
        (2, "00000000-0000-4000-8000-000000000102"),
        (3, "00000000-0000-4000-8000-000000000103"),
    ] {
        let reply = tool_value(
            &call(
                &router,
                token,
                "caller",
                "spawn_subagent",
                serde_json::json!({"title":"Spawned chat","requestId":id,"message":format!("work {index}"),"role":"implementer","difficulty":"hard"}),
            )
            .await,
        );
        assert_eq!(
            reply["routing"]["model"],
            "deepseek-ai/DeepSeek-V4.1-Flash-1017"
        );
        assert_eq!(reply["routing"]["step"], "implementer-deepseek");
        assert_eq!(reply["openSubagents"].as_array().unwrap().len(), index);
    }
    let over = call(
        &router,
        token,
        "caller",
        "spawn_subagent",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000104","message":"too many","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(over["result"]["isError"], true);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn reply_to_sender_and_message_parent_resolve_targets_and_carry_trailers() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("reply-tools.sqlite")).unwrap();
    for id in ["parent", "child", "other"] {
        store
            .create_session(id, temp.path().to_str().unwrap(), 1)
            .unwrap();
    }
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store.clone(), factory, token.into())
        .await
        .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    let sub = "00000000-0000-4000-8000-000000000500";
    let spawned = call(
        &router,
        token,
        "parent",
        "spawn_subagent",
        serde_json::json!({"title":"Sub","requestId":sub,"message":"work","role":"implementer","difficulty":"simple"}),
    )
    .await;
    assert_eq!(spawned["result"]["isError"], false, "{spawned}");
    let subagent_message = store.reader().unwrap().message(sub).unwrap().unwrap();
    assert!(subagent_message.content.contains("message_parent"));
    assert!(
        subagent_message
            .content
            .contains("final reply reaches your parent")
    );
    let spawned_input = received.recv().await.unwrap();
    assert_eq!(spawned_input.uuid, sub);
    assert!(spawned_input.content.contains("message_parent"));

    let ping = call(
        &router,
        token,
        "parent",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":"child","messageId":"00000000-0000-4000-8000-000000000501","message":"hello child"}),
    )
    .await;
    assert_eq!(ping["result"]["isError"], false, "{ping}");
    let child = received.recv().await.unwrap();
    assert!(
        child.content.contains("reply_to_sender"),
        "{}",
        child.content
    );

    let reply_id = "00000000-0000-4000-8000-000000000502";
    let reply = call(
        &router,
        token,
        "child",
        "reply_to_sender",
        serde_json::json!({"messageId":reply_id,"message":"reply from child"}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let parent = received.recv().await.unwrap();
    assert_eq!(parent.uuid, reply_id);
    assert!(parent.content.contains("Message from boblm session child"));
    assert!(parent.content.contains("reply_to_sender"));
    let repeated = tool_value(
        &call(
            &router,
            token,
            "child",
            "reply_to_sender",
            serde_json::json!({"messageId":reply_id,"message":"reply from child"}),
        )
        .await,
    );
    assert_eq!(repeated["alreadyAccepted"], true);
    assert!(received.try_recv().is_err());

    let question_id = "00000000-0000-4000-8000-000000000503";
    let question = call(
        &router,
        token,
        sub,
        "message_parent",
        serde_json::json!({"messageId":question_id,"message":"question from sub"}),
    )
    .await;
    assert_eq!(question["result"]["isError"], false, "{question}");
    let parent_question = received.recv().await.unwrap();
    assert_eq!(parent_question.uuid, question_id);
    assert!(
        parent_question
            .content
            .contains(&format!("Message from boblm session {sub}"))
    );
    assert!(parent_question.content.contains("reply_to_sender"));

    let root_refused = call(
        &router,
        token,
        "parent",
        "message_parent",
        serde_json::json!({"messageId":"00000000-0000-4000-8000-000000000504","message":"root question"}),
    )
    .await;
    assert_eq!(root_refused["result"]["isError"], true);
    assert!(
        root_refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Only a subagent")
    );
    let lonely = call(
        &router,
        token,
        "other",
        "reply_to_sender",
        serde_json::json!({"messageId":"00000000-0000-4000-8000-000000000505","message":"nobody sent me anything"}),
    )
    .await;
    assert_eq!(lonely["result"]["isError"], true);
    assert!(
        lonely["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("has not received a peer message")
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn reply_to_sender_ignores_self_sent_messages_after_a_peer_message() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("reply-self.sqlite")).unwrap();
    for id in ["asker", "answerer"] {
        store
            .create_session(id, temp.path().to_str().unwrap(), 1)
            .unwrap();
    }
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store.clone(), factory, token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let ping = call(
        &router,
        token,
        "asker",
        "ping_peer",
        serde_json::json!({"sessionIdPrefix":"answerer","messageId":"00000000-0000-4000-8000-000000000601","message":"question"}),
    )
    .await;
    assert_eq!(ping["result"]["isError"], false, "{ping}");
    received.recv().await.unwrap();
    // A background notice the chat recorded with itself as sender.
    store
        .append_with_origin(
            boblm_store::NewMessage {
                uuid: "00000000-0000-4000-8000-000000000602",
                session: "answerer",
                role: "user",
                content: "Background command is no longer listed as running: sleep",
                delivery_state: "queued",
                created_at: 2,
            },
            &[],
            Some("answerer"),
        )
        .unwrap();
    let reply_id = "00000000-0000-4000-8000-000000000603";
    let reply = call(
        &router,
        token,
        "answerer",
        "reply_to_sender",
        serde_json::json!({"messageId":reply_id,"message":"answer"}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(tool_value(&reply)["sessionId"], "asker");
    let delivered = received.recv().await.unwrap();
    assert_eq!(delivered.uuid, reply_id);
    assert!(
        delivered
            .content
            .contains("Message from boblm session answerer")
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn role_resolver_skips_a_step_with_an_active_exhaustion_mark() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("role-exhaustion.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    store
        .mark_role_step_exhausted(
            boblm_tuning::StepId::ImplementerCodexPlan,
            4_000_000_000,
            1_900_000_000,
            "fixture",
        )
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    let reply = tool_value(
        &call(
            &router,
            token,
            "caller",
            "spawn_subagent",
            serde_json::json!({"title":"Marked route","requestId":"00000000-0000-4000-8000-000000000401","message":"work","role":"implementer","difficulty":"simple"}),
        )
        .await,
    );
    assert_eq!(reply["routing"]["step"], "implementer-claude");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn role_spawn_peer_creates_a_visible_root_with_the_resolved_choice() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("role-root.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(SpawnFactory { inputs });
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    let id = "00000000-0000-4000-8000-000000000201";
    let args = serde_json::json!({"title":"Spawned chat","requestId":id,"message":"implement this","role":"implementer","difficulty":"hard"});
    let spawned = tool_value(&call(&router, token, "caller", "spawn_peer", args.clone()).await);
    assert_eq!(spawned["routing"]["model"], "gpt-6.1-sol");
    assert_eq!(spawned["routing"]["effort"], "high");
    assert_eq!(spawned["routing"]["step"], "implementer-codex-plan");
    assert_eq!(spawned["session"]["name"], "Spawned chat");
    assert!(spawned["session"].get("parentSessionId").is_none());
    assert!(spawned["session"].get("rootSessionId").is_none());
    assert_eq!(spawned["session"]["role"], "implementer");
    let repeated = tool_value(&call(&router, token, "caller", "spawn_peer", args).await);
    assert_eq!(repeated["alreadyAccepted"], true);
    assert_eq!(repeated["routing"]["model"], "gpt-6.1-sol");

    let both = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000202","message":"both","role":"implementer","difficulty":"simple","model":"sonnet","effort":"low"}),
    )
    .await;
    assert_eq!(both["result"]["isError"], true);
    let reviewer_difficulty = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000203","message":"review","role":"reviewer","difficulty":"hard"}),
    )
    .await;
    assert_eq!(reviewer_difficulty["result"]["isError"], true);
    app.shutdown().await.unwrap();
}

struct FailingFactory(std::sync::atomic::AtomicUsize);
#[async_trait]
impl DriverFactory for FailingFactory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(io::Error::other("fixture startup failure"))
    }
}
fn tool_value(reply: &serde_json::Value) -> serde_json::Value {
    assert_eq!(reply["result"]["isError"], false);
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[tokio::test]
async fn spawn_failure_retains_first_prompt_and_explicit_directory_without_retrying_after_restart()
{
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = temp.path().join("failed-spawn.sqlite");
    let directory = temp.path().join("other-directory");
    std::fs::create_dir(&directory).unwrap();
    let store = Store::open(&path).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let other_project = "70000000-0000-4000-8000-000000000001";
    store
        .add_project(other_project, directory.to_str().unwrap(), "other", 1)
        .unwrap();
    let factory = Arc::new(FailingFactory(AtomicUsize::new(0)));
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory.clone(), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let id = "00000000-0000-4000-8000-000000000004";
    let args = serde_json::json!({"title":"Spawned chat","requestId":id,"message":"Keep this prompt","model":"opus","effort":"max","projectId":other_project});
    let reply = tool_value(&call(&router, token, "caller", "spawn_peer", args.clone()).await);
    assert_eq!(reply["dispatchFailed"], true);
    assert_eq!(
        reply["session"]["workingDirectory"],
        directory.to_str().unwrap()
    );
    assert_eq!(reply["session"]["status"]["queued"], 1);
    assert_eq!(reply["session"]["status"]["queueHeld"], true);
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    app.shutdown().await.unwrap();
    drop(router);
    drop(app);
    let app = App::open(Store::open(path).unwrap(), factory.clone(), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let repeated = tool_value(&call(&router, token, "caller", "spawn_peer", args).await);
    assert_eq!(repeated["alreadyAccepted"], true);
    assert_eq!(repeated["session"]["status"]["running"], false);
    assert_eq!(repeated["session"]["status"]["queued"], 1);
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    let inspected = tool_value(
        &call(
            &router,
            token,
            "caller",
            "inspect_peer",
            serde_json::json!({"sessionIdPrefix":id}),
        )
        .await,
    );
    assert_eq!(inspected["messages"].as_array().unwrap().len(), 1);
    assert!(
        inspected["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Keep this prompt")
    );
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn lost_spawn_reply_does_not_replay_uncertain_input_or_reset_later_choices() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = temp.path().join("uncertain-spawn.sqlite");
    let store = Store::open(&path).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(Factory(tokio::sync::Mutex::new(Some(Fake {
        hold_events_until_abort: false,
        emit,
        events,
        inputs,
    }))));
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory, token.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let id = "00000000-0000-4000-8000-000000000005";
    let args = serde_json::json!({"title":"Spawned chat","requestId":id,"message":"Exactly once","model":"opus","effort":"high"});
    let _ = call(&router, token, "caller", "spawn_peer", args.clone()).await;
    assert_eq!(received.recv().await.unwrap().uuid, id);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/setModel")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"sessionId":id,"model":"sonnet","effort":"low"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    app.shutdown().await.unwrap();
    drop(router);
    drop(app);
    let app = App::open(
        Store::open(path).unwrap(),
        Arc::new(NeverSpawn),
        token.into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let repeated = tool_value(&call(&router, token, "caller", "spawn_peer", args).await);
    assert_eq!(repeated["alreadyAccepted"], true);
    assert_eq!(repeated["session"]["status"]["uncertain"], 1);
    assert_eq!(repeated["session"]["status"]["running"], false);
    assert_eq!(repeated["session"]["status"]["model"], "sonnet");
    assert_eq!(repeated["session"]["status"]["effort"], "low");
    assert!(received.try_recv().is_err());
    app.shutdown().await.unwrap();
}

struct GatedFactory {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    driver: tokio::sync::Mutex<Option<Fake>>,
}
#[async_trait]
impl DriverFactory for GatedFactory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Box::new(
            self.driver
                .lock()
                .await
                .take()
                .ok_or_else(|| io::Error::other("second start"))?,
        ))
    }
}
#[tokio::test]
async fn disconnected_spawn_finishes_registration_and_shutdown_drains_its_work() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = temp.path().join("cancelled-spawn.sqlite");
    let store = Store::open(&path).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(GatedFactory {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        driver: tokio::sync::Mutex::new(Some(Fake {
            hold_events_until_abort: false,
            emit,
            events,
            inputs,
        })),
    });
    let token = "peer-test-owner-token-only";
    let app = App::open(store, factory.clone(), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let id = "00000000-0000-4000-8000-000000000007";
    let request = tokio::spawn(async move {
        call(&router,token,"caller","spawn_peer",serde_json::json!({"title":"Spawned chat","requestId":id,"message":"Finish accepting this","model":"sonnet","effort":"medium"})).await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        factory.entered.notified(),
    )
    .await
    .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let closing = tokio::spawn({
        let app = app.clone();
        async move { app.shutdown().await }
    });
    factory.release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), received.recv())
            .await
            .unwrap()
            .unwrap()
            .uuid,
        id
    );
    closing.await.unwrap().unwrap();
    drop(app);
    let app = App::open(
        Store::open(path).unwrap(),
        Arc::new(NeverSpawn),
        token.into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let inspected = tool_value(
        &call(
            &router,
            token,
            "caller",
            "inspect_peer",
            serde_json::json!({"sessionIdPrefix":id}),
        )
        .await,
    );
    assert_eq!(inspected["session"]["status"]["running"], false);
    assert_eq!(inspected["session"]["status"]["uncertain"], 1);
    assert_eq!(inspected["messages"].as_array().unwrap().len(), 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_bad_project_id_is_quoted_briefly_in_the_tool_error() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("path-error.sqlite")).unwrap();
    store.create_session("caller", "/local", 1).unwrap();
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let long = "x".repeat(500);
    let reply = call(
        &router,
        token,
        "caller",
        "spawn_peer",
        serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-000000000009","message":"hi","model":"sonnet","effort":"low","projectId":long}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], true);
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("projectId must be a project's UUID from list_projects"),
        "{text}"
    );
    assert!(
        text.len() < 300,
        "an oversized value is not echoed whole: {} bytes",
        text.len()
    );
    // A well-formed ID that names no project says so and points at the listing tool.
    let missing = call(&router, token, "caller", "spawn_peer", serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-00000000000a","message":"hi","model":"sonnet","effort":"low","projectId":"70000000-0000-4000-8000-0000000000ff"})).await;
    assert!(
        missing["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Call list_projects")
    );
    // The agent follows the model: a Codex model is accepted, an unknown one lists the catalog.
    let unknown = call(&router, token, "caller", "spawn_peer", serde_json::json!({"title":"Spawned chat","requestId":"00000000-0000-4000-8000-00000000000b","message":"hi","model":"gpt-9","effort":"low"})).await;
    assert!(
        unknown["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("is not a model this server offers")
    );
    // A catalog label passes the model check; this fixture then stops at its missing folder.
    let labelled = call(&router, token, "caller", "spawn_peer", serde_json::json!({"title":"Labelled chat","requestId":"00000000-0000-4000-8000-00000000000c","message":"hi","model":"terra","effort":"medium"})).await;
    let text = labelled["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("is not a model"), "{text}");
    assert!(text.contains("project folder"), "{text}");
    let listed = tool_value(
        &call(
            &router,
            token,
            "caller",
            "list_projects",
            serde_json::json!({}),
        )
        .await,
    );
    assert_eq!(listed["projects"][0]["path"], "/local");
    assert_eq!(listed["projects"][0]["chats"], 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn agents_manage_their_own_schedules_through_the_peer_tools() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("schedule-tools.sqlite")).unwrap();
    for id in ["caller", "other"] {
        store.create_session(id, "/local", 1).unwrap();
    }
    let token = "peer-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let id = "50000000-0000-4000-8000-000000000001";

    let created = call(
        &router,
        token,
        "caller",
        "schedule_prompt",
        serde_json::json!({"scheduleId":id,"prompt":"Check the build and carry on.","inMinutes":5,"everyMinutes":30,"maxRuns":4}),
    )
    .await;
    assert_eq!(created["result"]["isError"], false, "{created}");
    let created = tool_value(&created);
    assert_eq!(
        (
            created["kind"].as_str(),
            created["everyMinutes"].as_i64(),
            created["maxRuns"].as_i64()
        ),
        (Some("repeating"), Some(30), Some(4))
    );
    assert_eq!(created["state"], "active");
    assert!(created["nextRunAt"].as_str().unwrap().ends_with('Z'));

    let listed = tool_value(
        &call(
            &router,
            token,
            "caller",
            "list_schedules",
            serde_json::json!({}),
        )
        .await,
    );
    assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);
    assert_eq!(listed["schedules"][0]["scheduleId"], id);
    // Another session sees nothing of it and cannot cancel it.
    let theirs = tool_value(
        &call(
            &router,
            token,
            "other",
            "list_schedules",
            serde_json::json!({}),
        )
        .await,
    );
    assert!(theirs["schedules"].as_array().unwrap().is_empty());
    let refused = call(
        &router,
        token,
        "other",
        "cancel_schedule",
        serde_json::json!({"scheduleId":id}),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("has no schedule")
    );

    // A bad request explains itself to the agent.
    let bad = call(
        &router,
        token,
        "caller",
        "schedule_prompt",
        serde_json::json!({"scheduleId":"50000000-0000-4000-8000-000000000002","prompt":"x","everyMinutes":0}),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true);
    assert!(
        bad["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("everyMinutes must be at least 1"),
        "{bad}"
    );
    // The caller cannot be named in arguments.
    let named = call(
        &router,
        token,
        "caller",
        "schedule_prompt",
        serde_json::json!({"scheduleId":id,"prompt":"x","inMinutes":1,"sessionId":"other"}),
    )
    .await;
    assert_eq!(named["result"]["isError"], true);

    let cancelled = tool_value(
        &call(
            &router,
            token,
            "caller",
            "cancel_schedule",
            serde_json::json!({"scheduleId":id}),
        )
        .await,
    );
    assert_eq!(cancelled["cancelled"], true);
    let empty = tool_value(
        &call(
            &router,
            token,
            "caller",
            "list_schedules",
            serde_json::json!({}),
        )
        .await,
    );
    assert!(empty["schedules"].as_array().unwrap().is_empty());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn automatic_titles_name_only_unnamed_chats_and_preserve_existing_titles() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("automatic-title.sqlite")).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    let token = "automatic-title-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let first = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Fix Login", "onlyIfUnnamed":true}),
    );
    let second = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Login Repair", "onlyIfUnnamed":true}),
    );
    let (first, second) = tokio::join!(first, second);
    let reports: Vec<serde_json::Value> = [first, second]
        .iter()
        .map(|reply| {
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        reports
            .iter()
            .filter(|report| report["renamed"] == true)
            .count(),
        1
    );
    assert_eq!(
        reports
            .iter()
            .filter(|report| report["renamed"] == false)
            .count(),
        1
    );
    let explicit = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Owner Chosen Title"}),
    )
    .await;
    assert_eq!(explicit["result"]["isError"], false);
    let automatic = call(
        &router,
        token,
        "caller",
        "change_title",
        serde_json::json!({"title":"Another Task", "onlyIfUnnamed":true}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(automatic["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["renamed"], false);
    let inspected = call(
        &router,
        token,
        "caller",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"caller"}),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["session"]["name"], "Owner Chosen Title");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn subagent_capability_can_ping_its_own_boblm_parent_and_wake_it() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("self-ping.sqlite")).unwrap();
    store.create_session("parent", "/fixture", 1).unwrap();
    let (emit, events) = tokio::sync::mpsc::unbounded_channel();
    let (inputs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let factory = Arc::new(Factory(tokio::sync::Mutex::new(Some(Fake {
        hold_events_until_abort: false,
        emit: emit.clone(),
        events,
        inputs,
    }))));
    let token = "self-ping-test-owner-token";
    let app = App::open(store, factory, token.into()).await.unwrap();
    let router = app.router(None);
    // Codex children inherit this MCP connection's parent-bound capability.
    let response = call(
        &router,
        token,
        "parent",
        "ping_peer",
        serde_json::json!({
            "sessionIdPrefix":"parent",
            "messageId":"00000000-0000-4000-8000-000000000031",
            "message":"Subagent review completed: OK"
        }),
    )
    .await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    let input = received.recv().await.unwrap();
    assert_eq!(input.priority, None);
    assert!(input.content.contains("Subagent review completed: OK"));
    emit.send(boblm_protocol::Event::CommandLifecycle {
        command_uuid: input.uuid,
        state: boblm_protocol::LifecycleState::Started,
    })
    .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn concurrent_reviewer_spawns_reserve_the_last_api_slot_once() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("reviewer-headroom.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let app = App::open(
        store.clone(),
        Arc::new(SpawnFactory { inputs }),
        "reviewer-headroom-token".into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let args = |id: u64| {
        serde_json::json!({
            "title":"Review", "requestId":format!("00000000-0000-4000-8000-{id:012}"),
            "message":"review this", "role":"reviewer"
        })
    };
    for id in 1..boblm_tuning::OPENAI_API_MAX_ACTIVE_REVIEWERS as u64 {
        let spawned = tool_value(
            &call(
                &router,
                "reviewer-headroom-token",
                "caller",
                "spawn_peer",
                args(id),
            )
            .await,
        );
        assert_eq!(spawned["routing"]["step"], "reviewer-openai-api");
    }
    let cap = boblm_tuning::OPENAI_API_MAX_ACTIVE_REVIEWERS as u64;
    let (first, second) = tokio::join!(
        call(
            &router,
            "reviewer-headroom-token",
            "caller",
            "spawn_peer",
            args(cap)
        ),
        call(
            &router,
            "reviewer-headroom-token",
            "caller",
            "spawn_peer",
            args(cap + 1)
        )
    );
    let routes = [tool_value(&first), tool_value(&second)];
    assert_eq!(
        routes
            .iter()
            .filter(|route| route["routing"]["step"] == "reviewer-openai-api")
            .count(),
        1
    );
    let fallback = routes
        .iter()
        .find(|route| route["routing"]["step"] == "reviewer-claude-api")
        .unwrap();
    assert_eq!(fallback["routing"]["model"], "haiku");
    assert_eq!(fallback["routing"]["effort"], "high");
    let saved = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == fallback["sessionId"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        saved.account_key.as_deref(),
        Some(boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A)
    );
    assert!(
        fallback["routing"]["reason"]
            .as_str()
            .unwrap()
            .contains("cap of")
    );
    let repeated = tool_value(
        &call(
            &router,
            "reviewer-headroom-token",
            "caller",
            "spawn_peer",
            args(1),
        )
        .await,
    );
    assert_eq!(repeated["alreadyAccepted"], true);
    assert_eq!(repeated["routing"]["step"], "reviewer-openai-api");
    assert_eq!(
        store
            .reader()
            .unwrap()
            .api_reviewer_headroom(
                i64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis()
                )
                .unwrap()
            )
            .unwrap()
            .active_reviewers,
        boblm_tuning::OPENAI_API_MAX_ACTIVE_REVIEWERS
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn grunt_spawns_are_refused() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("grunt-routing.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let token = "grunt-route-fixture-token";
    let app = App::open(
        store.clone(),
        Arc::new(SpawnFactory { inputs }),
        token.into(),
    )
    .await
    .unwrap();
    let router = app.router(Some(temp.path().into()));
    let args = serde_json::json!({
        "title":"Screenshots", "requestId":"00000000-0000-4000-8000-000000000601",
        "message":"Take the requested screenshots", "role":"grunt"
    });
    for tool in ["spawn_subagent", "spawn_peer"] {
        for difficulty in [None, Some("simple")] {
            let mut invalid = args.clone();
            if let Some(difficulty) = difficulty {
                invalid["difficulty"] = serde_json::json!(difficulty);
            }
            let refused = call(&router, token, "caller", tool, invalid).await;
            assert_eq!(refused["result"]["isError"], true);
            assert!(refused["result"]["content"][0]["text"].as_str().unwrap().contains("The grunt role was retired. Use an implementer, or spawn_peer with an explicit model for mechanical work."));
        }
    }
    assert_eq!(store.reader().unwrap().sessions().unwrap().len(), 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn planner_spawn_routes_to_opus_without_difficulty() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("planner.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    store
        .set_session_account(
            "caller",
            Some(
                &boblm_claude_driver::account::account_id()
                    .unwrap_or_else(|| "claude-account".into()),
            ),
        )
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let app = App::open(
        store.clone(),
        Arc::new(SpawnFactory { inputs }),
        "planner-test-token".into(),
    )
    .await
    .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    for (tool, id) in [
        ("spawn_subagent", "00000000-0000-4000-8000-000000000801"),
        ("spawn_peer", "00000000-0000-4000-8000-000000000802"),
    ] {
        let result = call(&router, "planner-test-token", "caller", tool,
            serde_json::json!({"title":"Plan work","requestId":id,"message":"Plan this","role":"planner"})).await;
        assert_eq!(result["result"]["isError"], false, "{result}");
        let spawned = tool_value(&result);
        assert_eq!(spawned["routing"]["model"], boblm_tuning::MODEL_OPUS);
        assert_eq!(spawned["routing"]["effort"], "high");
        assert_eq!(spawned["routing"]["account"], "claude-plan-a");
        assert_eq!(spawned["routing"]["step"], "planner-claude");
        assert_eq!(spawned["session"]["role"], "planner");
    }
    store.record_event("caller", "agent_event",
        serde_json::json!({"type":"usage","weekly":{"usedPercent":100,"resetsAt":2_000_000_000}}),
        1_793_448_000_000_i64).unwrap();
    let refused = call(&router, "planner-test-token", "caller", "spawn_subagent",
        serde_json::json!({"title":"Unavailable planner","requestId":"00000000-0000-4000-8000-000000000803","message":"Plan this","role":"planner"})).await;
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let error = refused["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        error.contains(
            "No planner model is available until the Claude reset at 2033-05-18 03:33:20 UTC"
        ),
        "{error}"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn spawn_subagent_refuses_missing_folders_and_regular_files() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("spawn-folders.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let app = App::open(
        store.clone(),
        Arc::new(NeverSpawn),
        "folder-test-token".into(),
    )
    .await
    .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let file = root.join("file");
    std::fs::write(&file, "not a folder").unwrap();
    for path in [root.join("missing"), file] {
        let response = call(
            &router,
            "folder-test-token",
            "caller",
            "spawn_subagent",
            serde_json::json!({
                "title":"Worker", "requestId":uuid::Uuid::new_v4().to_string(), "message":"work",
                "role":"implementer", "difficulty":"simple", "workingDirectory":path
            }),
        )
        .await;
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(path.to_str().unwrap()), "{text}");
        assert!(text.contains("folder"), "{text}");
    }
    assert_eq!(store.reader().unwrap().sessions().unwrap().len(), 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn spawn_peer_refuses_a_missing_or_non_directory_project() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("peer-folders.sqlite")).unwrap();
    let project_path = temp.path().join("project");
    std::fs::create_dir(&project_path).unwrap();
    store
        .create_session("caller", project_path.to_str().unwrap(), 1)
        .unwrap();
    std::fs::remove_dir(&project_path).unwrap();
    let app = App::open(
        store.clone(),
        Arc::new(NeverSpawn),
        "folder-test-token".into(),
    )
    .await
    .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    for is_file in [false, true] {
        if is_file {
            std::fs::write(&project_path, "not a directory").unwrap();
        }
        for role in [false, true] {
            let mut args = serde_json::json!({"title":"Peer", "requestId":uuid::Uuid::new_v4().to_string(), "message":"work"});
            if role {
                args["role"] = "implementer".into();
                args["difficulty"] = "simple".into();
            } else {
                args["model"] = "sonnet".into();
                args["effort"] = "high".into();
            }
            let response = call(&router, "folder-test-token", "caller", "spawn_peer", args).await;
            assert_eq!(response["result"]["isError"], true);
            assert!(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains(project_path.to_str().unwrap())
            );
        }
    }
    assert_eq!(store.reader().unwrap().sessions().unwrap().len(), 1);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn compact_is_caller_bound_and_reports_context_on_acceptance_and_refusal() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("compact.sqlite")).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    store.create_session("peer", "/peer", 2).unwrap();
    store
        .record_event(
            "caller",
            "agent_event",
            serde_json::json!({
                "type":"usage", "context":{"usedTokens":65_000,"windowTokens":100_000}
            }),
            3,
        )
        .unwrap();
    let token = "compact-test-owner-token";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    assert!(
        tool_names(&router, token, "caller")
            .await
            .iter()
            .any(|name| name == "compact")
    );
    let response = call(
        &router,
        token,
        "caller",
        "compact",
        serde_json::json!({"instructions":"Keep the next phase."}),
    )
    .await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    let report: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["queued"], true);
    assert_eq!(
        report["context"],
        serde_json::json!({"used":65_000,"window":100_000,"percent":65.0})
    );
    let inspected = call(
        &router,
        token,
        "peer",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"caller"}),
    )
    .await;
    let inspected: serde_json::Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let status = &inspected["session"]["status"];
    assert_eq!(status["queueHeld"], true);
    assert_eq!(status["running"], false);
    assert_eq!(status["queued"], 1);
    let response = call(&router, token, "caller", "compact", serde_json::json!({})).await;
    let report: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["alreadyQueued"], true);
    let response = call(&router, token, "peer", "compact", serde_json::json!({})).await;
    assert_eq!(response["result"]["isError"], true);
    let report: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        report["context"],
        serde_json::json!({"used":null,"window":null,"percent":null})
    );
    assert!(
        report["message"]
            .as_str()
            .unwrap()
            .contains("context usage is unknown")
    );
    let injected = call(
        &router,
        token,
        "caller",
        "compact",
        serde_json::json!({"sessionIdPrefix":"peer"}),
    )
    .await;
    assert!(injected.get("error").is_some() || injected["result"]["isError"] == true);
    let inspected = call(
        &router,
        token,
        "caller",
        "inspect_peer",
        serde_json::json!({"sessionIdPrefix":"peer"}),
    )
    .await;
    let inspected: serde_json::Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(inspected["session"]["status"]["queued"], 0);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn retired_grunt_sessions_and_receipts_still_load_and_display() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("legacy.sqlite")).unwrap();
    store
        .create_session("root", temp.path().to_str().unwrap(), 1)
        .unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    let receipt = serde_json::json!({"role":"grunt","difficulty":"simple","model":"haiku","effort":"high","account":"claude-plan","step":"grunt-claude","reason":"Legacy route"});
    store
        .spawn_peer(boblm_store::PeerSpawn {
            request_id: "10000000-0000-4000-8000-000000000099".into(),
            caller: "root".into(),
            arguments: "{}".into(),
            project_id: project,
            agent: "claude".into(),
            model: "haiku".into(),
            effort: "high".into(),
            title: "Legacy grunt".into(),
            content: "old work".into(),
            created_at: 2,
            parent_session_id: Some("root".into()),
            root_session_id: Some("root".into()),
            role: Some("grunt".into()),
            resolved: Some(receipt.to_string()),
            working_directory: None,
            account_key: None,
        })
        .unwrap();
    let saved = store
        .reader()
        .unwrap()
        .peer_spawn_receipt("10000000-0000-4000-8000-000000000099")
        .unwrap()
        .unwrap();
    assert_eq!(
        boblm_server::role_spawn::route_from_json(
            &serde_json::from_str::<serde_json::Value>(&saved.resolved.unwrap()).unwrap()
        )
        .unwrap(),
        receipt
    );
    let app = App::open(store, Arc::new(NeverSpawn), "legacy-test-token".into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let listed = tool_value(
        &call(
            &router,
            "legacy-test-token",
            "root",
            "list_peers",
            serde_json::json!({}),
        )
        .await,
    );
    assert_eq!(listed["sessions"][0]["role"], "grunt");
    app.shutdown().await.unwrap();
}

#[test]
fn claude_api_usage_never_becomes_a_subscription_reading_without_a_keyed_chat() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("api-usage.sqlite")).unwrap();
    let plan_id =
        boblm_claude_driver::account::account_id().unwrap_or_else(|| "subscription".into());
    for (id, account, used) in [
        ("old-api", boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A, 0.0),
        ("plan", plan_id.as_str(), 100.0),
    ] {
        store.create_session(id, "/fake", 1).unwrap();
        store.set_session_account(id, Some(account)).unwrap();
        store.record_event(id, "agent_event", serde_json::json!({"type":"usage","weekly":{"usedPercent":used,"resetsAt":4000000000_i64}}), 2).unwrap();
    }
    let outcome = boblm_server::role_spawn::resolve_route(
        &store,
        boblm_tuning::Role::Planner,
        boblm_tuning::Difficulty::Simple,
    );
    assert!(
        outcome.is_err(),
        "API usage cannot make the exhausted Claude plan available"
    );
}

#[tokio::test]
async fn claude_spawns_choose_independently_of_a_parent_pinned_to_plan_b() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("independent.sqlite")).unwrap();
    store
        .create_session("caller", temp.path().to_str().unwrap(), 1)
        .unwrap();
    store
        .set_claude_account_key("caller", boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_B)
        .unwrap();
    let (inputs, _received) = tokio::sync::mpsc::unbounded_channel();
    let app = App::open(
        store.clone(),
        Arc::new(SpawnFactory { inputs }),
        "independent-test-token".into(),
    )
    .await
    .unwrap();
    app.set_role_subagents_enabled_for_tests(true);
    let router = app.router(Some(temp.path().into()));
    for (tool, id, extra) in [
        (
            "spawn_subagent",
            "d774da14-d160-4594-8615-6955d632fa4a",
            serde_json::json!({"role":"planner"}),
        ),
        (
            "spawn_peer",
            "ab0e38e9-c299-4eab-b8b3-e3d5fc767368",
            serde_json::json!({"model":boblm_tuning::MODEL_SONNET,"effort":"high"}),
        ),
    ] {
        let mut args = serde_json::json!({"title":"Independent account", "requestId":id,"message":"Test account selection"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let result = call(&router, "independent-test-token", "caller", tool, args).await;
        assert_eq!(result["result"]["isError"], false, "{result}");
        let child = store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.id == id)
            .unwrap();
        assert_eq!(
            child.account_key.as_deref(),
            Some(boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_A)
        );
    }
    store
        .exhaust_claude_account(
            "caller",
            boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_A,
            4_000_000_000,
            true,
        )
        .unwrap();
    let retry = call(&router, "independent-test-token", "caller", "spawn_peer", serde_json::json!({
        "title":"Independent account", "requestId":"ab0e38e9-c299-4eab-b8b3-e3d5fc767368", "message":"Test account selection", "model":boblm_tuning::MODEL_SONNET,"effort":"high"
    })).await;
    assert_eq!(
        retry["result"]["isError"], false,
        "Accepted spawns must survive account exhaustion: {retry}"
    );
    assert_eq!(tool_value(&retry)["alreadyAccepted"], true);
    let fresh = call(&router, "independent-test-token", "caller", "spawn_peer", serde_json::json!({
        "title":"Owner work at the cutoff", "requestId":"eb013c9a-2d76-4497-849f-557d74de0d29", "message":"Explicit work", "model":boblm_tuning::MODEL_SONNET,"effort":"high"
    })).await;
    assert_eq!(
        fresh["result"]["isError"], false,
        "Explicit Claude work stays available with one exhausted plan: {fresh}"
    );
    let role = call(&router, "independent-test-token", "caller", "spawn_subagent", serde_json::json!({
        "title":"Planner at the cutoff", "requestId":"9ec659d7-4af4-4fae-8bf1-9781e310565e", "message":"Plan work", "role":"planner"
    })).await;
    assert_eq!(
        role["result"]["isError"], true,
        "Role spawns still enforce the cutoff: {role}"
    );
    app.shutdown().await.unwrap();
}
