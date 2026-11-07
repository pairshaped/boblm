use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use std::{io, sync::Arc};
use tower::ServiceExt;
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("Viewing a subagent must never launch or resume an agent")
    }
}
#[tokio::test]
async fn subagent_reads_require_authentication_and_reject_arbitrary_paths_without_spawning() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("fixture.sqlite")).unwrap();
    store.create_session("owner", "/fixture", 1).unwrap();
    let app = App::open(
        store,
        Arc::new(NeverSpawn),
        "fixture-token-for-subagent-test".into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    for (token, key, expected) in [
        ("wrong", "agent", StatusCode::UNAUTHORIZED),
        (
            "fixture-token-for-subagent-test",
            "../../other",
            StatusCode::BAD_REQUEST,
        ),
        (
            "fixture-token-for-subagent-test",
            "agent",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/commands/subagentHistory")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"sessionId":"owner","agentKey":key,"before":null})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    app.shutdown().await.unwrap();
}
