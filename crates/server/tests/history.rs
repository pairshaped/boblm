use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use serde_json::json;
use std::{io, sync::Arc};
use tower::ServiceExt;
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("history reads must not start an agent")
    }
}
#[tokio::test]
async fn history_snapshot_cursor_connects_to_live_events_without_replaying_the_chat() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    store.record_events("s","agent_event",(0..300).map(|i|json!({"type":"assistant","message":{"id":format!("a{i}"),"content":[{"type":"text","text":format!("message {i}")}]}})).collect(),2).unwrap();
    let app = App::open(
        store.clone(),
        Arc::new(NeverSpawn),
        "history-fixture-token".into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let request = |body| {
        Request::builder()
            .method("POST")
            .uri("/commands/chatHistory")
            .header("Authorization", "Bearer history-fixture-token")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let response = router
        .clone()
        .oneshot(request(json!({"sessionId":"s"}).to_string()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: boblm_protocol::client::ChatHistoryPage =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page.events.len(), 50);
    assert!(page.has_more);
    assert!(page.cursor.0 >= page.events.last().unwrap().seq().0);
    store.record_event("s","agent_event",json!({"type":"assistant","message":{"id":"new","content":[{"type":"text","text":"arrived after snapshot"}]}}),3).unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/s/events")
                .header("Authorization", "Bearer history-fixture-token")
                .header("Last-Event-ID", page.cursor.0.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    let text = String::from_utf8(frame.to_vec()).unwrap();
    assert!(text.contains("arrived after snapshot"), "{text}");
    assert!(
        !text.contains("message 299"),
        "history must not replay: {text}"
    );
    let response = router
        .clone()
        .oneshot(request(
            json!({"sessionId":"s","before":3,"after":1}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/chatHistory")
                .body(Body::from("{\"sessionId\":\"s\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    app.shutdown().await.unwrap();
}

struct UnavailableRunner(std::path::PathBuf);
#[async_trait]
impl DriverFactory for UnavailableRunner {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("history must not start a provider")
    }
    async fn connect_runner(
        &self,
        store: &Store,
        session: &str,
    ) -> boblm_runner::Result<Option<boblm_runner::RunningSession>> {
        boblm_runner::RunningSession::attach_unavailable(
            store.clone(),
            session,
            self.0.clone(),
            None,
        )
        .await
        .map(Some)
    }
}

#[tokio::test]
async fn history_remains_readable_without_a_runner_socket() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    store.record_event("s", "agent_event", json!({"type":"assistant","message":{"id":"a","content":[{"type":"text","text":"retained history"}]}}), 2).unwrap();
    let app = App::open(
        store,
        Arc::new(UnavailableRunner(temp.path().join("missing.sock"))),
        "history-fixture-token".into(),
    )
    .await
    .unwrap();
    let response = app
        .router(None)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/chatHistory")
                .header("Authorization", "Bearer history-fixture-token")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"sessionId":"s"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let page: boblm_protocol::client::ChatHistoryPage = serde_json::from_slice(&bytes).unwrap();
    assert!(!page.status.running);
    assert!(!page.status.jobs.unwrap().available);
    assert!(page.events.iter().any(|e| {
        serde_json::to_string(e)
            .unwrap()
            .contains("retained history")
    }));
    app.shutdown().await.unwrap();
}
