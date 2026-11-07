use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use tower::ServiceExt;
const TOKEN: &str = "question-test-token-only";
const ID: &str = "30000000-0000-4000-8000-000000000001";
struct Input {
    uuid: String,
    content: String,
    priority: Option<Priority>,
}
struct Fake {
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<Input>,
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
            .send(Input {
                uuid: uuid.into(),
                content: content.into(),
                priority,
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
        Ok(())
    }
}
struct Factory(Mutex<Option<Fake>>);
#[async_trait]
impl DriverFactory for Factory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Ok(Box::new(
            self.0
                .lock()
                .await
                .take()
                .ok_or_else(|| io::Error::other("second spawn"))?,
        ))
    }
}

async fn command(router: &Router, name: &str, value: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/commands/{name}"))
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("Content-Type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap(),
    )
}
#[tokio::test]
async fn questions_return_immediately_and_answers_require_a_lifecycle_ack() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("questions.sqlite")).unwrap();
    store.create_session("s", "/tmp", 1).unwrap();
    store.create_session("t", "/tmp", 1).unwrap();
    let (emit, events) = mpsc::unbounded_channel();
    let (inputs, mut received) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake { events, inputs }))));
    let app = App::open(store.clone(), factory, TOKEN.into())
        .await
        .unwrap();
    let router = app.router(None);
    let capability = boblm_server::mcp::session_token(TOKEN, "s");
    let response = router.clone().oneshot(Request::builder().method("POST").uri("/mcp/s").header("Host","127.0.0.1").header("Authorization",format!("Bearer {capability}")).header("Content-Type","application/json").header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-11-25").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ask","arguments":{"questionId":ID,"question":"Which color?","choices":["Blue","Red"]}}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["result"]["isError"], false, "{body}");
    assert!(
        received.try_recv().is_err(),
        "asking does not start an agent"
    );
    let (_, listed) = command(&router, "listQuestions", json!({"sessionId":"s"})).await;
    assert_eq!(listed["questions"][0]["question"], "Which color?");
    assert_eq!(
        command(
            &router,
            "answerQuestion",
            json!({"sessionId":"t","questionId":ID,"answer":"Red"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // Save the answer without handing it to the runner, as if the HTTP connection ended here.
    store.answer_question(ID, "s", "Green", 2).unwrap();
    app.deliver_question_answers().await;
    let sent = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(sent.content.contains("Which color?"));
    assert!(sent.content.contains("Owner's answer:\nGreen"));
    assert_eq!(sent.priority, None);
    assert_eq!(
        store
            .reader()
            .unwrap()
            .message(&sent.uuid)
            .unwrap()
            .unwrap()
            .delivery_state,
        "dispatching"
    );
    let reply = command(
        &router,
        "answerQuestion",
        json!({"sessionId":"s","questionId":ID,"answer":"Green"}),
    )
    .await;
    assert_eq!(reply.0, StatusCode::OK);
    assert_eq!(
        command(
            &router,
            "answerQuestion",
            json!({"sessionId":"s","questionId":ID,"answer":"Blue"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        received.try_recv().is_err(),
        "an identical retry never sends twice"
    );
    emit.send(Event::CommandLifecycle {
        command_uuid: sent.uuid.clone(),
        state: LifecycleState::Started,
    })
    .unwrap();
    for _ in 0..100 {
        if store
            .reader()
            .unwrap()
            .message(&sent.uuid)
            .unwrap()
            .unwrap()
            .delivery_state
            == "started"
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        store
            .reader()
            .unwrap()
            .message(&sent.uuid)
            .unwrap()
            .unwrap()
            .delivery_state,
        "started"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn discussion_waits_for_the_question_queue_and_is_delivered_without_an_invented_answer() {
    const SECOND: &str = "30000000-0000-4000-8000-000000000002";
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("discuss.sqlite")).unwrap();
    store.create_session("s", "/tmp", 1).unwrap();
    let (_emit, events) = mpsc::unbounded_channel();
    let (inputs, mut received) = mpsc::unbounded_channel();
    let app = App::open(
        store.clone(),
        Arc::new(Factory(Mutex::new(Some(Fake { events, inputs })))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    store.ask(ID, "s", "Which framework?", &[], 2).unwrap();
    store.ask(SECOND, "s", "Which color?", &[], 2).unwrap();
    let (status, deferred) = command(
        &router,
        "discussQuestionLater",
        json!({"sessionId":"s","questionId":ID}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deferred["pendingCount"], 1);
    assert_eq!(deferred["questions"][0]["state"], "discuss_later");
    assert!(deferred["questions"][0]["answer"].is_null());
    assert!(received.try_recv().is_err());
    let (status, _) = command(
        &router,
        "answerQuestion",
        json!({"sessionId":"s","questionId":SECOND,"answer":"Blue"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(3), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(first.content.contains("Owner's answer:\nBlue"));
    assert!(
        next.content
            .contains("I'm not sure. Let's discuss once there are no more open question prompts.")
    );
    assert!(next.content.contains("Which framework?"));
    assert!(!next.content.contains("Owner's answer:"));
    assert_eq!(
        store
            .reader()
            .unwrap()
            .message(&next.uuid)
            .unwrap()
            .unwrap()
            .delivery_state,
        "dispatching",
        "a write still does not prove consumption"
    );
    assert_eq!(
        command(
            &router,
            "discussQuestionLater",
            json!({"sessionId":"s","questionId":ID})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        received.try_recv().is_err(),
        "retrying a deferred decision cannot send it twice"
    );
    app.shutdown().await.unwrap();
}

async fn question_tool(router: &Router, session: &str, name: &str, args: Value) -> Value {
    let capability = boblm_server::mcp::session_token(TOKEN, session);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/mcp/{session}"))
                .header("Host", "127.0.0.1")
                .header("Authorization", format!("Bearer {capability}"))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-11-25")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":name,"arguments":args}})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn question_tools_revise_delete_validate_and_reject_other_chats() {
    const REVISED: &str = "30000000-0000-4000-8000-000000000003";
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("question-tools.sqlite")).unwrap();
    store.create_session("s", "/tmp", 1).unwrap();
    store.create_session("t", "/tmp", 1).unwrap();
    let (_emit, events) = mpsc::unbounded_channel();
    let (inputs, mut received) = mpsc::unbounded_channel();
    let app = App::open(
        store.clone(),
        Arc::new(Factory(Mutex::new(Some(Fake { events, inputs })))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let ask = json!({"questionId":ID,"question":"Original?","choices":["Yes"]});
    assert_eq!(
        question_tool(&router, "s", "ask", ask.clone()).await["result"]["isError"],
        false
    );
    let update =
        json!({"questionId":ID,"replacementId":REVISED,"question":"Revised?","choices":["No"]});
    for (name, args) in [
        ("update_question", update.clone()),
        ("delete_question", json!({"questionId":ID})),
    ] {
        assert_eq!(
            question_tool(&router, "t", name, args).await["result"]["isError"],
            true
        );
    }
    let mut invalid = update.clone();
    invalid["question"] = json!(" ");
    assert_eq!(
        question_tool(&router, "s", "update_question", invalid).await["result"]["isError"],
        true
    );
    let mut invalid = update.clone();
    invalid["choices"] = json!([""]);
    assert_eq!(
        question_tool(&router, "s", "update_question", invalid).await["result"]["isError"],
        true
    );
    for _ in 0..2 {
        let updated = question_tool(&router, "s", "update_question", update.clone()).await;
        assert_eq!(updated["result"]["isError"], false, "{updated}");
        let receipt: Value =
            serde_json::from_str(updated["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["questionId"], REVISED);
    }
    let (_, listed) = command(&router, "listQuestions", json!({"sessionId":"s"})).await;
    assert_eq!(listed["pendingCount"], 1);
    assert_eq!(listed["questions"][0]["question"], "Revised?");
    assert_eq!(listed["questions"][0]["choices"], json!(["No"]));
    assert_eq!(
        command(
            &router,
            "answerQuestion",
            json!({"sessionId":"s","questionId":ID,"answer":"stale"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for _ in 0..2 {
        assert_eq!(
            question_tool(
                &router,
                "s",
                "delete_question",
                json!({"questionId":REVISED})
            )
            .await["result"]["isError"],
            false
        );
    }
    assert_eq!(
        question_tool(&router, "s", "ask", ask).await["result"]["isError"],
        true
    );
    assert_eq!(
        question_tool(&router, "s", "update_question", update).await["result"]["isError"],
        false
    );
    let (_, listed) = command(&router, "listQuestions", json!({"sessionId":"s"})).await;
    assert_eq!(listed["pendingCount"], 0);
    assert_eq!(listed["questions"], json!([]));
    app.deliver_question_answers().await;
    assert!(
        received.try_recv().is_err(),
        "question tools must not start an agent"
    );
    app.shutdown().await.unwrap();
}
