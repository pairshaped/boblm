use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{io, sync::Arc};
use tower::ServiceExt;

const TOKEN: &str = "agent-deletion-test-owner-token";
const CHILD: &str = "10000000-0000-4000-8000-000000000001";
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("chat deletion must not start a provider")
    }
}
fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("deletion.sqlite")).unwrap();
    for id in ["caller", "target", "root"] {
        store.create_session(id, "/project", 1).unwrap();
    }
    store
        .create_session("foreign", "/other-project", 1)
        .unwrap();
    (temp, store)
}
fn child(store: &Store) {
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .unwrap()
        .project_id
        .unwrap();
    store
        .spawn_peer(boblm_store::PeerSpawn {
            request_id: CHILD.into(),
            caller: "root".into(),
            arguments: "{}".into(),
            project_id: project,
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            title: "Deletion fixture".into(),
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
}
async fn call(router: &axum::Router, caller: &str, prefix: &str, request: &str) -> Value {
    let capability = boblm_server::mcp::session_token(TOKEN, caller);
    let response = router.clone().oneshot(Request::builder().method("POST").uri(format!("/mcp/{caller}"))
        .header("Host", "127.0.0.1").header("Authorization", format!("Bearer {capability}"))
        .header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .body(Body::from(json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
            "name":"delete_chat", "arguments":{"sessionIdPrefix":prefix,"requestId":request}
        }}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn agent_deletion_refuses_other_projects_live_self_questions_and_open_subagents() {
    let (temp, store) = fixture();
    child(&store);
    for id in ["caller", "foreign", "root"] {
        store.set_archived(id, true).unwrap();
    }
    store.set_archived(CHILD, false).unwrap();
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    for (prefix, expected) in [
        ("foreign", "No matching chat is available in your project."),
        (
            "does-not-exist",
            "No matching chat is available in your project.",
        ),
        ("target", "Call archive_peer before delete_chat."),
        ("caller", "You cannot delete your own chat"),
        ("root", "This root chat has an unarchived subagent."),
    ] {
        let reply = call(&router, "caller", prefix, &uuid::Uuid::new_v4().to_string()).await;
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert!(text(&reply).contains(expected), "{reply}");
    }
    store.set_archived("target", true).unwrap();
    store
        .ask("question", "target", "Keep this?", &[], 3)
        .unwrap();
    let refused = call(
        &router,
        "caller",
        "target",
        &uuid::Uuid::new_v4().to_string(),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(text(&refused).contains("unanswered owner question"));
    assert!(!store.reader().unwrap().session_deleted("target").unwrap());
    assert!(!store.reader().unwrap().session_deleted(CHILD).unwrap());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_deletion_receipt_survives_retry_restart_and_conflicting_reuse() {
    let (temp, store) = fixture();
    store.set_archived("target", true).unwrap();
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let request = uuid::Uuid::new_v4().to_string();
    let (first, concurrent_retry) = tokio::join!(
        call(&router, "caller", "tar", &request),
        call(&router, "caller", "tar", &request),
    );
    assert_eq!(first, concurrent_retry);
    assert_eq!(first["result"]["isError"], false, "{first}");
    let receipt: Value = serde_json::from_str(text(&first)).unwrap();
    assert_eq!(
        receipt,
        json!({"requestId":request,"sessionId":"target","deleted":true,"deletedBySessionId":"caller"})
    );
    assert_eq!(call(&router, "caller", "tar", &request).await, first);
    assert!(store.reader().unwrap().session_deleted("target").unwrap());
    assert_eq!(
        store
            .reader()
            .unwrap()
            .deletion_actor("target")
            .unwrap()
            .as_deref(),
        Some("caller")
    );
    assert_eq!(
        store
            .reader()
            .unwrap()
            .agent_deletion_receipt(&request)
            .unwrap()
            .unwrap()
            .caller,
        "caller"
    );
    // Prefix identity remains the original request even if another chat later matches it.
    store.create_session("target-later", "/project", 5).unwrap();
    assert_eq!(call(&router, "caller", "tar", &request).await, first);
    for (caller, prefix) in [("caller", "root"), ("root", "tar")] {
        let conflict = call(&router, caller, prefix, &request).await;
        assert_eq!(conflict["result"]["isError"], true);
        assert!(text(&conflict).contains("fresh UUID"));
    }
    app.shutdown().await.unwrap();
    let store = Store::open(temp.path().join("deletion.sqlite")).unwrap();
    let app = App::open(store, Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    assert_eq!(
        call(
            &app.router(Some(temp.path().into())),
            "caller",
            "tar",
            &request
        )
        .await,
        first
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_deletion_cascades_archived_children_and_protects_their_questions() {
    let (temp, store) = fixture();
    child(&store);
    for id in ["root", CHILD] {
        store.set_archived(id, true).unwrap();
    }
    store
        .ask("question", CHILD, "Keep my work?", &[], 3)
        .unwrap();
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let self_cascade = call(&router, CHILD, "root", &uuid::Uuid::new_v4().to_string()).await;
    assert_eq!(self_cascade["result"]["isError"], true);
    assert!(text(&self_cascade).contains("your own chat"));
    let request = uuid::Uuid::new_v4().to_string();
    let refused = call(&router, "caller", "root", &request).await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(text(&refused).contains("unanswered owner question"));
    assert!(!store.reader().unwrap().session_deleted("root").unwrap());
    assert!(!store.reader().unwrap().session_deleted(CHILD).unwrap());
    // Deferring uses the idle nudge's definition of an answered or set-aside question.
    store.discuss_question_later("question", CHILD, 4).unwrap();
    let deleted = call(&router, "caller", "root", &request).await;
    assert_eq!(deleted["result"]["isError"], false, "{deleted}");
    for id in ["root", CHILD] {
        assert!(store.reader().unwrap().session_deleted(id).unwrap());
        assert_eq!(
            store
                .reader()
                .unwrap()
                .deletion_actor(id)
                .unwrap()
                .as_deref(),
            Some("caller")
        );
    }
    assert_eq!(call(&router, "caller", "root", &request).await, deleted);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_deletion_ignores_retired_and_answered_questions() {
    let (temp, store) = fixture();
    store.set_archived("target", true).unwrap();
    store.ask("retired", "target", "Old?", &[], 3).unwrap();
    store.delete_question("retired", "target", 4).unwrap();
    store
        .ask("answered", "target", "Finished?", &[], 3)
        .unwrap();
    store
        .answer_question("answered", "target", "Yes", 4)
        .unwrap();
    let app = App::open(store, Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let reply = call(
        &app.router(Some(temp.path().into())),
        "caller",
        "target",
        &uuid::Uuid::new_v4().to_string(),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn owner_deletion_keeps_its_existing_permissions_and_no_agent_attribution() {
    let (temp, store) = fixture();
    child(&store);
    for id in ["root", CHILD] {
        store.set_archived(id, true).unwrap();
    }
    store
        .ask("question", CHILD, "Keep my work?", &[], 3)
        .unwrap();
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/commands/deleteSession")
                    .header("Authorization", format!("Bearer {TOKEN}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(json!({"sessionId":"root"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    for id in ["root", CHILD] {
        assert!(store.reader().unwrap().session_deleted(id).unwrap());
        assert_eq!(store.reader().unwrap().deletion_actor(id).unwrap(), None);
    }
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_deletion_prepared_child_stays_quiet_through_reads_until_cancel_or_commit() {
    let (_temp, store) = fixture();
    child(&store);
    store.set_archived("root", true).unwrap();
    let mut child = boblm_runner::RunningSession::start(store.clone(), CHILD, Arc::new(NeverSpawn))
        .await
        .unwrap();
    let handle = child.handle();
    handle.prepare_delete().await.unwrap();
    handle.prepare_delete().await.unwrap();
    assert!(handle.status().await.unwrap().archived);
    assert!(
        handle
            .rename("Should wait")
            .await
            .unwrap_err()
            .to_string()
            .contains("being deleted")
    );
    handle.cancel_prepared_deletion().await.unwrap();
    handle.rename("Allowed after refusal").await.unwrap();
    handle.prepare_delete().await.unwrap();
    handle.archive(false).await.unwrap();
    assert!(!handle.status().await.unwrap().archived);
    store.set_archived("root", true).unwrap();
    handle.prepare_delete().await.unwrap();
    store
        .delete_session_by_agent("root", &uuid::Uuid::new_v4().to_string(), "caller", "root")
        .unwrap();
    // A read arriving after commit uses the pre-deletion snapshot, never removed rows.
    assert!(handle.status().await.unwrap().archived);
    handle.delete().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn agent_deletion_prepared_child_exits_after_commit_without_the_coordinator() {
    let (_temp, store) = fixture();
    child(&store);
    store.set_archived("root", true).unwrap();
    let mut child = boblm_runner::RunningSession::start(store.clone(), CHILD, Arc::new(NeverSpawn))
        .await
        .unwrap();
    child.handle().prepare_delete().await.unwrap();
    store
        .delete_session_by_agent("root", &uuid::Uuid::new_v4().to_string(), "caller", "root")
        .unwrap();
    tokio::time::advance(boblm_tuning::USAGE_POLL_INTERVAL).await;
    child.wait().await.unwrap();
}

#[tokio::test]
async fn agent_deletion_retry_uses_the_receipt_after_commit_with_stale_runner_handles() {
    let (temp, store) = fixture();
    child(&store);
    store.set_archived("root", true).unwrap();
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    for id in ["root", CHILD] {
        app.handle_for_tests(id)
            .await
            .prepare_delete()
            .await
            .unwrap();
    }
    let request = uuid::Uuid::new_v4().to_string();
    // Simulate a commit whose actor acknowledgment never reached the coordinator.
    store
        .delete_session_by_agent("root", &request, "caller", "root")
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let reply = call(&router, "caller", "root", &request).await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(call(&router, "caller", "root", &request).await, reply);
    app.shutdown().await.unwrap();
}
