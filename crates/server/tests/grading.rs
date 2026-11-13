use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{PeerSpawn, Session, Store};
use http_body_util::BodyExt;
use std::{io, sync::Arc};
use tower::ServiceExt;

struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("grading must not start a provider")
    }
}

const TOKEN: &str = "grading-test-owner-token";
const REVIEWER: &str = "20000000-0000-4000-8000-000000000001";
const IMPLEMENTER: &str = "20000000-0000-4000-8000-000000000002";
const RESOLVED: &str = r#"{"role":"reviewer","difficulty":"none","model":"gpt-6-luna","effort":"xhigh","account":"codex-api","step":"reviewer-openai-api","reason":"Luna on the OpenAI API"}"#;

async fn call(
    router: &axum::Router,
    caller: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let capability = boblm_server::mcp::session_token(TOKEN, caller);
    let response = router.clone().oneshot(Request::builder().method("POST").uri(format!("/mcp/{caller}")).header("Host","127.0.0.1").header("Authorization",format!("Bearer {capability}")).header("Content-Type","application/json").header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").body(Body::from(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn text(reply: &serde_json::Value) -> String {
    reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    app: Arc<App>,
    router: axum::Router,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("grading.sqlite")).unwrap();
    store.create_session("root", "/root", 1).unwrap();
    store.create_session("stranger", "/root", 2).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap();
    for (id, role, model, effort, account, resolved) in [
        (
            REVIEWER,
            "reviewer",
            "gpt-6-luna",
            "xhigh",
            Some("codex-api"),
            Some(RESOLVED),
        ),
        (IMPLEMENTER, "implementer", "sonnet", "medium", None, None),
    ] {
        store
            .spawn_peer(PeerSpawn {
                request_id: id.into(),
                caller: "root".into(),
                arguments: format!(r#"{{"message":"work","id":"{id}"}}"#),
                project_id: project.clone(),
                agent: if model == "sonnet" { "claude" } else { "codex" }.into(),
                model: model.into(),
                effort: effort.into(),
                title: format!("{role} chat"),
                content: "work".into(),
                created_at: 3,
                parent_session_id: Some("root".into()),
                root_session_id: Some("root".into()),
                role: Some(role.into()),
                resolved: resolved.map(str::to_owned),
                working_directory: None,
                account_key: account.map(str::to_owned),
            })
            .unwrap();
    }
    let app = App::open(store.clone(), Arc::new(NeverSpawn), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    Fixture {
        _temp: temp,
        store,
        app,
        router,
    }
}

#[tokio::test]
async fn grading_is_blind_overwrites_and_refuses_strangers() {
    let f = fixture().await;
    let refused = call(
        &f.router,
        "stranger",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":REVIEWER,"rating":"good"}),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(text(&refused).contains("Only the parent"));
    assert!(
        f.store
            .reader()
            .unwrap()
            .subagent_grade(REVIEWER)
            .unwrap()
            .is_none()
    );

    // A prefix is enough, and the result is tiny.
    let graded = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":"20000000-0000-4000-8000-000000000001","rating":"good","note":"caught the race"}),
    )
    .await;
    assert_eq!(graded["result"]["isError"], false);
    let body = text(&graded);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"sessionId":REVIEWER,"rating":"good","replaced":false})
    );
    let lower = body.to_lowercase();
    for hidden in [
        "gpt-6-luna",
        "xhigh",
        "reviewer-openai-api",
        "codex-api",
        "haiku",
        "luna",
        "openai",
        "claude",
        "codex",
    ] {
        assert!(!lower.contains(hidden), "{hidden} leaked in {body}");
    }
    let row = f
        .store
        .reader()
        .unwrap()
        .subagent_grade(REVIEWER)
        .unwrap()
        .unwrap();
    assert_eq!(row.model.as_deref(), Some("gpt-6-luna"));
    assert_eq!(row.effort.as_deref(), Some("xhigh"));
    assert_eq!(row.step.as_deref(), Some("reviewer-openai-api"));
    assert_eq!(row.account_key.as_deref(), Some("codex-api"));

    // A refusal for a reviewer names no routing either.
    let bad = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":REVIEWER,"rating":"superb"}),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true);
    let message = text(&bad);
    assert!(message.contains("good, ok or poor"));
    for hidden in ["luna", "openai", "codex", "xhigh"] {
        assert!(!message.to_lowercase().contains(hidden));
    }
    let long = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":REVIEWER,"rating":"ok","note":"y".repeat(501)}),
    )
    .await;
    assert!(text(&long).contains("limit is 500"));

    // Archived subagents can still be graded, and grading again replaces.
    f.store.set_archived(REVIEWER, true).unwrap();
    let again = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":REVIEWER,"rating":"poor"}),
    )
    .await;
    assert!(text(&again).contains("\"replaced\":true"));
    assert_eq!(
        f.store
            .reader()
            .unwrap()
            .subagent_grade(REVIEWER)
            .unwrap()
            .unwrap()
            .rating,
        "poor"
    );

    // Any role can be graded.
    let implementer = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":IMPLEMENTER,"rating":"ok"}),
    )
    .await;
    assert_eq!(implementer["result"]["isError"], false);
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_tool_refuses_while_grading_is_turned_off_and_keeps_stored_grades() {
    let f = fixture().await;
    f.store
        .grade_subagent("root", REVIEWER, "good", None, 5)
        .unwrap();
    f.app.set_subagent_grading_enabled_for_tests(false);
    let reply = call(
        &f.router,
        "root",
        "grade_subagent",
        serde_json::json!({"sessionIdPrefix":REVIEWER,"rating":"poor"}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(text(&reply), "Subagent grading is turned off.");
    assert_eq!(
        f.store
            .reader()
            .unwrap()
            .subagent_grade(REVIEWER)
            .unwrap()
            .unwrap()
            .rating,
        "good"
    );
    f.app.shutdown().await.unwrap();
}
