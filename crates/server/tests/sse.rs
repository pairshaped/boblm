use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory, RunningSession};
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
use tower::ServiceExt;
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("reading events must not start Claude")
    }
}
fn request(cursor: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri("/sessions/s/events");
    if let Some(cursor) = cursor {
        builder = builder.header("Last-Event-ID", cursor);
    }
    builder.body(Body::empty()).unwrap()
}
async fn event(body: &mut Body) -> (u64, serde_json::Value) {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let text = std::str::from_utf8(&data).unwrap();
        let id = text.lines().find_map(|line| line.strip_prefix("id: "));
        if let Some(id) = id {
            let json = text
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap();
            return (id.parse().unwrap(), serde_json::from_str(json).unwrap());
        }
    }
}
#[tokio::test]
async fn slow_reader_and_last_event_id_reconnect_receive_every_durable_event() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("s.sqlite")).unwrap();
    store.create_session("s", "/local", 1).unwrap();
    let runner = RunningSession::start(store.clone(), "s", Arc::new(NeverSpawn))
        .await
        .unwrap();
    let app = boblm_server::sse::router(BTreeMap::from([("s".into(), runner.handle())]));
    let response = app.clone().oneshot(request(None)).await.unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let mut received = vec![event(&mut body).await.0];
    // Leave the HTTP body unpolled while the durable history grows.
    for i in 0..40 {
        store
            .record_event("s", "agent_event", serde_json::json!({"index":i}), 2)
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(60)).await;
    for _ in 0..5 {
        received.push(event(&mut body).await.0);
    }
    let cursor = *received.last().unwrap();
    drop(body);
    for i in 40..60 {
        store
            .record_event("s", "agent_event", serde_json::json!({"index":i}), 2)
            .unwrap();
    }
    let mut body = app
        .clone()
        .oneshot(request(Some(&cursor.to_string())))
        .await
        .unwrap()
        .into_body();
    for _ in 0..56 {
        let (id, value) = event(&mut body).await;
        assert_eq!(value["seq"], id);
        received.push(id);
    }
    assert_eq!(received, (1..=62).collect::<Vec<_>>());
    drop(body);
    assert!(!runner.handle().status().await.unwrap().running);
    runner.shutdown().await.unwrap();
}
#[tokio::test]
async fn rejects_malformed_resume_ids_and_sends_keep_alive_comments() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("s.sqlite")).unwrap();
    store.create_session("s", "/local", 1).unwrap();
    let runner = RunningSession::start(store, "s", Arc::new(NeverSpawn))
        .await
        .unwrap();
    let app = boblm_server::sse::router(BTreeMap::from([("s".into(), runner.handle())]));
    for id in ["-1", "1.5", "x", "9007199254740992", ""] {
        assert_eq!(
            app.clone()
                .oneshot(request(Some(id)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut body = app.oneshot(request(Some("2"))).await.unwrap().into_body();
    tokio::time::pause();
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(std::str::from_utf8(&frame).unwrap(), ": keep-alive\n\n");
    drop(body);
    runner.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_list_global_cursor_resumes_across_sessions_and_database_reopen() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let path = temp.path().join("list.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("a", "/a", 1).unwrap();
    store.create_session("b", "/b", 2).unwrap();
    let app = boblm_server::sse::list_router(store.clone());
    let mut body = app
        .oneshot(
            Request::builder()
                .uri("/sessions/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .into_body();
    let (seq, first) = event(&mut body).await;
    assert_eq!((seq, first["sessionId"].as_str()), (1, Some("a")));
    drop(body);
    drop(store);
    let store = Store::open(&path).unwrap();
    store.set_archived("a", true).unwrap();
    store.create_session("c", "/c", 3).unwrap();
    let app = boblm_server::sse::list_router(store.clone());
    let mut body = app
        .oneshot(
            Request::builder()
                .uri("/sessions/events")
                .header("Last-Event-ID", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .into_body();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (seq, value) = event(&mut body).await;
        ids.push((
            seq,
            value["sessionId"].as_str().unwrap().to_owned(),
            value["kind"].as_str().unwrap().to_owned(),
        ));
    }
    assert_eq!(
        ids,
        vec![
            (2, "b".into(), "created".into()),
            (3, "a".into(), "archived".into()),
            (4, "c".into(), "created".into())
        ]
    );
    assert_eq!(
        store
            .reader()
            .unwrap()
            .events_since("a", 0, 100)
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[tokio::test]
async fn list_snapshot_cursor_skips_old_statuses_but_keeps_new_changes() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("list-snapshot.sqlite")).unwrap();
    store.create_session("s", "/tmp", 1).unwrap();
    for i in 0..20 {
        store
            .publish_summary("s", serde_json::json!({"turnActive": i % 2 == 0}))
            .unwrap();
    }
    let app = boblm_server::app::App::open(
        store.clone(),
        Arc::new(NeverSpawn),
        "snapshot-test-token-only".into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/listSessions")
                .header("Authorization", "Bearer snapshot-test-token-only")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let snapshot: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let cursor = snapshot["cursor"].as_i64().unwrap();
    assert!(cursor >= 20);
    assert_eq!(snapshot["sessions"][0]["status"]["turnActive"], false);
    assert!(
        store
            .reader()
            .unwrap()
            .session_list_since(cursor, 256)
            .unwrap()
            .is_empty()
    );
    store
        .publish_summary("s", serde_json::json!({"turnActive": true}))
        .unwrap();
    assert_eq!(
        store
            .reader()
            .unwrap()
            .session_list_since(cursor, 256)
            .unwrap()
            .len(),
        1
    );
    app.shutdown().await.unwrap();
}
