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
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use tower::ServiceExt;
const TOKEN: &str = "integration-test-token-only";
struct Input {
    uuid: String,
    priority: Option<Priority>,
}
struct Fake {
    emit: mpsc::UnboundedSender<Event>,
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<Input>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, uuid: &str, _: &str, priority: Option<Priority>) -> io::Result<()> {
        self.inputs
            .send(Input {
                uuid: uuid.into(),
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
        self.emit
            .send(Event::Result {
                subtype: Some("interrupted".into()),
                is_error: Some(false),
            })
            .map_err(io::Error::other)
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
async fn command(
    router: &Router,
    name: &str,
    value: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
/// Add the folder as a project and return its ID.
async fn project(router: &Router, path: &std::path::Path) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let (status, reply) = command(
        router,
        "addProject",
        serde_json::json!({"projectId":id,"path":path}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    id
}
#[tokio::test]
async fn authenticated_commands_create_steer_stop_idempotently_and_recover_without_execution() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let database = temp.path().join("commands.sqlite");
    let store = Store::open(&database).unwrap();
    let (emit, events) = mpsc::unbounded_channel();
    let (inputs, mut received) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        emit: emit.clone(),
        events,
        inputs,
    }))));
    let app = App::open(store.clone(), factory.clone(), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    for path in [
        "/commands/listSessions",
        "/sessions/events",
        "/sessions/s/events",
    ] {
        let status = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(if path.starts_with("/commands") {
                        "POST"
                    } else {
                        "GET"
                    })
                    .uri(path)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let project_id = project(&router, temp.path()).await;
    let (status, _) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":project_id,"model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let uuid = "00000000-0000-4000-8000-000000000001";
    let second = "00000000-0000-4000-8000-000000000002";
    let send = serde_json::json!({"sessionId":"s","messageId":uuid,"content":"one"});
    assert_eq!(
        command(&router, "sendMessage", send.clone()).await.0,
        StatusCode::OK
    );
    let first = received.recv().await.unwrap();
    assert_eq!(first.uuid, uuid);
    assert_eq!(first.priority, None);
    emit.send(Event::CommandLifecycle {
        command_uuid: uuid.into(),
        state: LifecycleState::Started,
    })
    .unwrap();
    assert_eq!(
        command(&router, "sendMessage", send.clone()).await.0,
        StatusCode::OK
    );
    assert!(received.try_recv().is_err());
    assert_eq!(
        command(
            &router,
            "sendMessage",
            serde_json::json!({"sessionId":"s","messageId":uuid,"content":"conflict"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        command(
            &router,
            "sendMessage",
            serde_json::json!({"sessionId":"s","messageId":second,"content":"steer"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(received.recv().await.unwrap().priority, Some(Priority::Now));
    let (status, result) =
        command(&router, "stopSession", serde_json::json!({"sessionId":"s"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["status"]["uncertain"], 1);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/s/events")
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("Last-Event-ID", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let data = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert!(std::str::from_utf8(&data).unwrap().contains("id: 2\n"));
    drop(body);
    for operation in ["archiveSession", "reopenSession"] {
        assert_eq!(
            command(&router, operation, serde_json::json!({"sessionId":"s"}))
                .await
                .0,
            StatusCode::OK
        );
    }
    app.shutdown().await.unwrap();
    drop(router);
    drop(app);
    drop(store);
    let app = App::open(Store::open(&database).unwrap(), factory, TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let (_, list) = command(&router, "listSessions", serde_json::json!({})).await;
    assert_eq!(list["sessions"][0]["status"]["running"], false);
    assert_eq!(list["sessions"][0]["status"]["uncertain"], 1);
    assert!(!matches!(
        tokio::time::timeout(Duration::from_millis(20), received.recv()).await,
        Ok(Some(_))
    ));
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn message_ids_are_canonical_in_replies_and_deduped_across_case() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("canonical.sqlite")).unwrap();
    let (emit, events) = mpsc::unbounded_channel();
    let (inputs, mut received) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        emit,
        events,
        inputs,
    }))));
    let app = App::open(store, factory, TOKEN.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let project_id = project(&router, temp.path()).await;
    let (status, _) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":project_id,"model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let upper = "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
    let lower = upper.to_lowercase();
    let (status, reply) = command(
        &router,
        "sendMessage",
        serde_json::json!({"sessionId":"s","messageId":upper,"content":"once"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["messageId"], lower);
    let (status, reply) = command(
        &router,
        "sendMessage",
        serde_json::json!({"sessionId":"s","messageId":lower,"content":"once"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["messageId"], lower);
    assert_eq!(received.recv().await.unwrap().uuid, lower);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        received.try_recv().is_err(),
        "the retry must not dispatch again"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn api_errors_say_what_is_wrong_and_what_to_do() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("errors.sqlite")).unwrap();
    let (emit, events) = mpsc::unbounded_channel();
    let (inputs, _received) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        emit,
        events,
        inputs,
    }))));
    let app = App::open(store, factory, TOKEN.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    let project_id = project(&router, temp.path()).await;
    let (status, _) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":project_id,"model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let message = |reply: &serde_json::Value| reply["message"].as_str().unwrap().to_owned();

    let (status, reply) = command(
        &router,
        "sendMessage",
        serde_json::json!({"sessionId":"s","messageId":"nope","content":"x"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&reply).contains("messageId must be a UUID like"),
        "{reply}"
    );

    let (status, reply) = command(&router, "sendMessage", serde_json::json!({"sessionId":"ghost","messageId":"00000000-0000-4000-8000-000000000001","content":"x"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        message(&reply).contains("There is no chat with ID \"ghost\"")
            && message(&reply).contains("reload"),
        "{reply}"
    );

    let (status, reply) = command(
        &router,
        "retryMessage",
        serde_json::json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-0000000000aa"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&reply).contains("no message with ID")
            && message(&reply).contains("nothing to retry"),
        "{reply}"
    );

    let (status, reply) = command(
        &router,
        "setModel",
        serde_json::json!({"sessionId":"s","model":"opus","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let id = "00000000-0000-4000-8000-000000000002";
    command(
        &router,
        "sendMessage",
        serde_json::json!({"sessionId":"s","messageId":id,"content":"first"}),
    )
    .await;
    let (status, reply) = command(
        &router,
        "sendMessage",
        serde_json::json!({"sessionId":"s","messageId":id,"content":"different"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message(&reply).contains("resend it unchanged with the same ID"),
        "{reply}"
    );

    command(
        &router,
        "archiveSession",
        serde_json::json!({"sessionId":"s"}),
    )
    .await;
    let (status, reply) = command(&router, "sendMessage", serde_json::json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000003","content":"x"})).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["status"]["archived"], false);

    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/listSessions")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = unauthorized.into_body().collect().await.unwrap().to_bytes();
    let reply: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(message(&reply).contains("Authorization: Bearer"), "{reply}");
    app.shutdown().await.unwrap();
}

async fn app() -> (tempfile::TempDir, Arc<App>, Router) {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("projects.sqlite")).unwrap();
    let (emit, events) = mpsc::unbounded_channel();
    let (inputs, _received) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        emit,
        events,
        inputs,
    }))));
    let app = App::open(store, factory, TOKEN.into()).await.unwrap();
    let router = app.router(Some(temp.path().into()));
    (temp, app, router)
}
const P1: &str = "20000000-0000-4000-8000-000000000001";
const P2: &str = "20000000-0000-4000-8000-000000000002";
fn text(reply: &serde_json::Value) -> String {
    reply["message"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn a_chat_cannot_be_created_until_a_project_exists() {
    let (temp, app, router) = app().await;
    let (status, list) = command(&router, "listSessions", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        list["projects"].as_array().unwrap().is_empty()
            && list["sessions"].as_array().unwrap().is_empty()
    );
    let (status, reply) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":P1,"model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{reply}");
    assert!(
        text(&reply).contains("no project with that ID") && text(&reply).contains("add its folder"),
        "{reply}"
    );
    let (status, reply) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":"","model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text(&reply).contains("Add a project"), "{reply}");
    let (status, _) = command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P1,"path":temp.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, reply) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":P1,"model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn projects_are_validated_normalized_unique_and_removable_only_when_empty() {
    let (temp, app, router) = app().await;
    let folder = temp.path().join("work");
    std::fs::create_dir(&folder).unwrap();
    // Stored paths are the resolved ones, whatever way the folder was reached.
    let resolved = folder.canonicalize().unwrap();
    for (path, expect) in [
        ("relative/dir", "absolute path"),
        ("/definitely/not/a/folder", "not an existing folder"),
    ] {
        let (status, reply) = command(
            &router,
            "addProject",
            serde_json::json!({"projectId":P1,"path":path}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert!(text(&reply).contains(expect), "{reply}");
    }
    // A trailing slash is the same folder; the name defaults to the folder's name.
    let with_slash = format!("{}/", folder.display());
    let (status, project) = command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P1,"path":with_slash}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{project}");
    assert_eq!(
        (
            project["path"].as_str().unwrap(),
            project["name"].as_str().unwrap()
        ),
        (resolved.to_str().unwrap(), "work")
    );
    // Other spellings of the same folder are the same folder too.
    for spelling in [
        format!("{}/./", folder.display()),
        format!("{}/../work", folder.display()),
        format!("{}//", folder.display()),
    ] {
        let (status, other) = command(
            &router,
            "addProject",
            serde_json::json!({"projectId":P2,"path":spelling}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{spelling}: {other}");
    }
    // Retrying the same project is fine; the same folder under another ID is not.
    let (status, again) = command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P1.to_uppercase(),"path":folder}),
    )
    .await;
    assert_eq!(
        (status, again["createdAt"].clone()),
        (StatusCode::OK, project["createdAt"].clone())
    );
    let (status, taken) = command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P2,"path":folder}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(text(&taken).contains("is already a project"), "{taken}");
    // A project with a chat cannot be removed; an empty one can, and only once.
    command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"s","projectId":P1,"model":"sonnet","effort":null}),
    )
    .await;
    let (status, busy) = command(
        &router,
        "removeProject",
        serde_json::json!({"projectId":P1}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(text(&busy).contains("still has 1 active chat"), "{busy}");
    let (status, _) = command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P2,"path":temp.path(),"name":"scratch"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        command(
            &router,
            "removeProject",
            serde_json::json!({"projectId":P2})
        )
        .await
        .0,
        StatusCode::OK
    );
    // A retry after a lost reply finds the work done, which is what the caller wanted.
    let (status, gone) = command(
        &router,
        "removeProject",
        serde_json::json!({"projectId":P2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gone}");
    let (_, list) = command(&router, "listSessions", serde_json::json!({})).await;
    assert_eq!(list["projects"].as_array().unwrap().len(), 1);
    assert_eq!(list["sessions"][0]["projectId"], P1);
    app.shutdown().await.unwrap();
}

/// A raw reply: status, headers and the bytes as they were sent.
async fn raw_command(
    router: &Router,
    name: &str,
    value: serde_json::Value,
    accept_encoding: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/commands/{name}"))
        .header("Authorization", format!("Bearer {TOKEN}"))
        .header("Content-Type", "application/json");
    if let Some(ask) = accept_encoding {
        request = request.header("Accept-Encoding", ask);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, bytes.to_vec())
}

#[tokio::test]
async fn large_json_replies_are_gzipped_for_browsers_that_ask_and_small_ones_are_not() {
    use std::io::Read;
    let (temp, app, router) = app().await;
    let project = project(&router, temp.path()).await;
    for n in 0..12 {
        command(
            &router,
            "createSession",
            serde_json::json!({"sessionId":format!("chat-{n}"),"projectId":project,"model":"sonnet","effort":null,"name":format!("A chat with a reasonably long name, number {n}")}),
        )
        .await;
    }
    let (_, plain_headers, plain) =
        raw_command(&router, "listSessions", serde_json::json!({}), None).await;
    assert!(
        plain.len() > 1024,
        "the reply is big enough to be worth compressing: {}",
        plain.len()
    );
    assert!(plain_headers.get("content-encoding").is_none());
    let (status, headers, packed) = raw_command(
        &router,
        "listSessions",
        serde_json::json!({}),
        Some("gzip, deflate, br"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-encoding"], "gzip");
    assert!(
        packed.len() * 3 < plain.len(),
        "{} against {}",
        packed.len(),
        plain.len()
    );
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(packed.as_slice())
        .read_to_end(&mut unpacked)
        .unwrap();
    assert_eq!(unpacked, plain, "the same JSON comes out");
    // A tiny reply costs more to compress than it saves.
    let (_, small, _) = raw_command(
        &router,
        "removeProject",
        serde_json::json!({"projectId": uuid::Uuid::new_v4().to_string()}),
        Some("gzip"),
    )
    .await;
    assert!(small.get("content-encoding").is_none());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn event_streams_are_never_compressed() {
    let (temp, app, router) = app().await;
    let project = project(&router, temp.path()).await;
    command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"live","projectId":project,"model":"sonnet","effort":null}),
    )
    .await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/live/events")
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("Accept-Encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.headers().get("content-encoding").is_none(),
        "a stream must reach the browser as it is produced, not buffered by a compressor"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_project_with_only_archived_chats_is_removed_along_with_them() {
    let (temp, app, router) = app().await;
    command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P1,"path":temp.path()}),
    )
    .await;
    for id in ["old", "newer"] {
        command(
            &router,
            "createSession",
            serde_json::json!({"sessionId":id,"projectId":P1,"model":"sonnet","effort":null}),
        )
        .await;
    }
    command(
        &router,
        "archiveSession",
        serde_json::json!({"sessionId":"old"}),
    )
    .await;
    // One active chat blocks the removal, and nothing is deleted on the way.
    let (status, busy) = command(
        &router,
        "removeProject",
        serde_json::json!({"projectId":P1}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{busy}");
    assert!(text(&busy).contains("still has 1 active chat"), "{busy}");
    assert!(text(&busy).contains("Archive"), "{busy}");
    let (_, list) = command(&router, "listSessions", serde_json::json!({})).await;
    assert_eq!(
        list["sessions"].as_array().unwrap().len(),
        2,
        "no chat was deleted"
    );
    // With every chat archived, the project and its chats go together.
    command(
        &router,
        "archiveSession",
        serde_json::json!({"sessionId":"newer"}),
    )
    .await;
    let (status, done) = command(
        &router,
        "removeProject",
        serde_json::json!({"projectId":P1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let (_, list) = command(&router, "listSessions", serde_json::json!({})).await;
    assert!(list["sessions"].as_array().unwrap().is_empty());
    assert!(list["projects"].as_array().unwrap().is_empty());
    // A retry after a lost reply finds the work done.
    let (status, again) = command(
        &router,
        "removeProject",
        serde_json::json!({"projectId":P1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_model_chosen_for_a_chat_decides_its_agent_and_it_cannot_change() {
    let (temp, app, router) = app().await;
    command(
        &router,
        "addProject",
        serde_json::json!({"projectId":P1,"path":temp.path()}),
    )
    .await;
    let (status, codex) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"c","projectId":P1,"model":"gpt-6-luna","effort":"low"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{codex}");
    assert_eq!(codex["agent"], "codex");
    assert_eq!(
        (
            codex["status"]["model"].as_str(),
            codex["status"]["effort"].as_str()
        ),
        (Some("gpt-6-luna"), Some("low"))
    );
    assert_eq!(
        codex["status"]["selectionPending"], false,
        "a fresh chat has nothing pending"
    );
    let (_, claude) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"k","projectId":P1,"model":"opus","effort":null}),
    )
    .await;
    assert_eq!(claude["agent"], "claude");
    // The catalog is enforced with a message that lists what is allowed.
    let (status, bad) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"x","projectId":P1,"model":"gpt-6-luna","effort":"ultra"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        text(&bad).contains("does not accept")
            && text(&bad).contains("low, medium, high, xhigh, max"),
        "{bad}"
    );
    // Switching model works inside an agent, never across.
    let (status, _) = command(
        &router,
        "setModel",
        serde_json::json!({"sessionId":"c","model":"gpt-6.1-sol","effort":"max"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, across) = command(
        &router,
        "setModel",
        serde_json::json!({"sessionId":"c","model":"sonnet","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        text(&across).contains(
            "This is a Codex chat, so it cannot switch to sonnet, a Claude model. Start a new chat"
        ),
        "{across}"
    );
    // The same ID cannot be reused for a different kind of chat.
    let (status, reused) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"c","projectId":P1,"model":"opus","effort":null}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        text(&reused).contains("already belongs to a different chat"),
        "{reused}"
    );
    // An identical retry returns the chat that exists.
    let (status, same) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"k","projectId":P1,"model":"opus","effort":null}),
    )
    .await;
    assert_eq!(
        (status, same["sessionId"].as_str()),
        (StatusCode::OK, Some("k"))
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn dragging_reorders_projects_and_each_projects_chats_separately() {
    let (temp, app, router) = app().await;
    let mut projects = Vec::new();
    for name in ["one", "two", "three"] {
        let folder = temp.path().join(name);
        std::fs::create_dir(&folder).unwrap();
        projects.push(project(&router, &folder).await);
    }
    for (id, project) in [("a1", 0), ("a2", 0), ("a3", 0), ("b1", 1)] {
        let (status, reply) = command(
            &router,
            "createSession",
            serde_json::json!({"sessionId":id,"projectId":projects[project],"model":"sonnet","effort":null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reply}");
    }
    let listed = || async {
        let (_, list) = command(&router, "listSessions", serde_json::json!({})).await;
        let ids = |key: &str, field: &str| -> Vec<String> {
            list[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v[field].as_str().unwrap().to_owned())
                .collect()
        };
        (ids("projects", "projectId"), ids("sessions", "sessionId"))
    };
    // New items are at the bottom, in the order they were made.
    let (order, chats) = listed().await;
    assert_eq!(order, projects);
    // Order is per project: the list interleaves projects, so compare one project at a time.
    let of = |chats: &[String], prefix: char| -> Vec<String> {
        chats
            .iter()
            .filter(|c| c.starts_with(prefix))
            .cloned()
            .collect()
    };
    assert_eq!(of(&chats, 'a'), ["a1", "a2", "a3"]);
    assert_eq!(of(&chats, 'b'), ["b1"]);

    // Drag the third project to the top, and a3 above a1.
    let moved = [
        projects[2].clone(),
        projects[0].clone(),
        projects[1].clone(),
    ];
    let (status, _) = command(
        &router,
        "reorderProjects",
        serde_json::json!({"projectIds": moved}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = command(
        &router,
        "reorderChats",
        serde_json::json!({"projectId":projects[0],"sessionIds":["a3","a1","a2"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (order, chats) = listed().await;
    assert_eq!(order, moved);
    assert_eq!(of(&chats, 'a'), ["a3", "a1", "a2"]);
    assert_eq!(
        of(&chats, 'b'),
        ["b1"],
        "another project's chat is untouched"
    );

    // A new chat lands at the bottom of its own project.
    command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"a4","projectId":projects[0],"model":"sonnet","effort":null}),
    )
    .await;
    let (_, chats) = listed().await;
    let top: Vec<_> = chats
        .iter()
        .filter(|c| c.starts_with('a'))
        .cloned()
        .collect();
    assert_eq!(top, ["a3", "a1", "a2", "a4"]);

    // Refusals say why and change nothing.
    for (name, body, expect) in [
        (
            "reorderProjects",
            serde_json::json!({"projectIds":["nope"]}),
            "UUID",
        ),
        (
            "reorderProjects",
            serde_json::json!({"projectIds":[projects[0], projects[0]]}),
            "twice",
        ),
        (
            "reorderChats",
            serde_json::json!({"projectId":projects[0],"sessionIds":["a1","b1"]}),
            "not in this project",
        ),
    ] {
        let (status, reply) = command(&router, name, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {reply}");
        assert!(text(&reply).contains(expect), "{body}: {reply}");
    }
    assert_eq!(listed().await.0, moved);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn directory_picker_lists_only_folders_inside_home() {
    use serde_json::json;
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let app = App::open(
        Store::open(temp.path().join("browse.sqlite")).unwrap(),
        Arc::new(Factory(Mutex::new(None))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let home = std::fs::canonicalize(std::env::var_os("HOME").unwrap()).unwrap();
    let root = tempfile::tempdir_in(&home).unwrap();
    std::fs::create_dir(root.path().join("Zulu")).unwrap();
    std::fs::create_dir(root.path().join("alpha")).unwrap();
    std::fs::create_dir(root.path().join(".hidden")).unwrap();
    std::fs::write(root.path().join("file.txt"), "not a folder").unwrap();
    std::os::unix::fs::symlink("/", root.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(root.path().join("alpha"), root.path().join("alias")).unwrap();
    let (status, reply) = command(&router, "listDirectories", json!({"path":root.path()})).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["path"], json!(root.path()));
    assert_eq!(reply["parent"], json!(home));
    assert_eq!(
        reply["directories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["alias", "alpha", "Zulu"]
    );
    let (status, reply) = command(&router, "listDirectories", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["path"], json!(home));
    assert!(reply["parent"].is_null());
    for path in [
        root.path().join("outside"),
        root.path().join("../../.."),
        root.path().join("missing"),
        root.path().join("file.txt"),
    ] {
        let (status, reply) = command(&router, "listDirectories", json!({"path":path})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
        assert!(
            reply["message"].as_str().unwrap().contains("folder"),
            "{reply}"
        );
    }
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands/listDirectories")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn directory_picker_rejects_invalid_paths_and_bounds_large_folders() {
    use serde_json::json;
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let app = App::open(
        Store::open(temp.path().join("browse.sqlite")).unwrap(),
        Arc::new(Factory(Mutex::new(None))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    for path in [
        "relative".to_owned(),
        " ".to_owned(),
        "/nul\0path".to_owned(),
        format!(
            "/{}",
            "x".repeat(boblm_tuning::MAX_WORKING_DIRECTORY_CHARS as usize)
        ),
    ] {
        let (status, reply) = command(&router, "listDirectories", json!({"path":path})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
        assert!(reply["message"].as_str().unwrap().contains("folder"));
    }
    let home = std::env::var_os("HOME").unwrap();
    let folder = tempfile::tempdir_in(home).unwrap();
    for n in 0..=boblm_tuning::MAX_DIRECTORY_ENTRIES {
        std::fs::write(folder.path().join(n.to_string()), "").unwrap();
    }
    let (status, reply) = command(&router, "listDirectories", json!({"path":folder.path()})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(
        reply["message"]
            .as_str()
            .unwrap()
            .contains("reduce the number of entries"),
        "{reply}"
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn chat_names_survive_creation_rename_and_creation_retry() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("names.sqlite")).unwrap();
    let app = App::open(store, Arc::new(Factory(Mutex::new(None))), TOKEN.into())
        .await
        .unwrap();
    let router = app.router(None);
    let p = project(&router, temp.path()).await;
    let create = serde_json::json!({"sessionId":"named", "projectId":p, "model":"sonnet", "name":"  First name  "});
    let (code, reply) = command(&router, "createSession", create.clone()).await;
    assert_eq!(code, StatusCode::OK, "{reply}");
    assert_eq!(reply["name"], "First name");
    let (code, reply) = command(
        &router,
        "renameSession",
        serde_json::json!({"sessionId":"named", "name":"Second name"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{reply}");
    let (_, reply) = command(&router, "createSession", create).await;
    assert_eq!(reply["name"], "Second name");
    let (code, _) = command(
        &router,
        "renameSession",
        serde_json::json!({"sessionId":"named", "name":"   "}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, reply) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"unnamed", "projectId":p, "model":"sonnet", "name":"   "}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{reply}");
    assert_eq!(reply["name"], "");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn deleting_requires_archive_removes_history_and_cannot_recreate_the_id() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("delete.sqlite")).unwrap();
    let app = App::open(
        store.clone(),
        Arc::new(Factory(Mutex::new(None))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let p = project(&router, temp.path()).await;
    let create = serde_json::json!({"sessionId":"remove-me", "projectId":p, "model":"sonnet"});
    assert_eq!(
        command(&router, "createSession", create.clone()).await.0,
        StatusCode::OK
    );
    let uploaded = temp.path().join("upload.txt");
    std::fs::write(&uploaded, "private upload").unwrap();
    store
        .create_attachment(boblm_store::NewAttachment {
            id: "upload",
            session: "remove-me",
            name: "upload.txt",
            content_type: "text/plain",
            size: 14,
            sha256: &"a".repeat(64),
            path: uploaded.to_str().unwrap(),
            created_at: 1,
        })
        .unwrap();
    store.mark_attachment_stored("upload").unwrap();
    store
        .append_with_attachments(
            boblm_store::NewMessage {
                uuid: "message",
                session: "remove-me",
                role: "user",
                content: "private message",
                delivery_state: "queued",
                created_at: 2,
            },
            &["upload".into()],
        )
        .unwrap();
    store
        .create_schedule(
            boblm_store::NewSchedule {
                id: "schedule",
                session: "remove-me",
                prompt: "private prompt",
                arguments: "{}",
                interval_ms: None,
                next_run_at: 999,
                ends_at: None,
                max_runs: None,
                created_at: 1,
            },
            20,
        )
        .unwrap();
    let body = serde_json::json!({"sessionId":"remove-me"});
    assert_eq!(
        command(&router, "deleteSession", body.clone()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        command(&router, "archiveSession", body.clone()).await.0,
        StatusCode::OK
    );
    for _ in 0..2 {
        let (code, reply) = command(&router, "deleteSession", body.clone()).await;
        assert_eq!(code, StatusCode::OK, "{reply}");
    }
    assert!(!uploaded.exists());
    assert!(
        store
            .reader()
            .unwrap()
            .deleted_uploads()
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .reader()
            .unwrap()
            .message("message")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .reader()
            .unwrap()
            .attachment("upload")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .reader()
            .unwrap()
            .schedules("remove-me")
            .unwrap()
            .is_empty()
    );
    assert!(store.reader().unwrap().sessions().unwrap().is_empty());
    assert!(
        store
            .reader()
            .unwrap()
            .events_since("remove-me", 0, 100)
            .unwrap()
            .is_empty()
    );
    let list = store.reader().unwrap().session_list_since(0, 100).unwrap();
    assert_eq!(list.last().unwrap().kind, "deleted");
    assert_eq!(
        command(&router, "createSession", create).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        command(&router, "reopenSession", body).await.0,
        StatusCode::NOT_FOUND
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn project_rename_changes_only_the_label_and_survives_reload() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let database = temp.path().join("project-name.sqlite");
    let store = Store::open(&database).unwrap();
    let app = App::open(
        store.clone(),
        Arc::new(Factory(Mutex::new(None))),
        TOKEN.into(),
    )
    .await
    .unwrap();
    let router = app.router(None);
    let id = project(&router, temp.path()).await;
    let (code, _) = command(
        &router,
        "createSession",
        serde_json::json!({"sessionId":"kept", "projectId":id, "model":"sonnet"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, before) = command(&router, "listSessions", serde_json::json!({})).await;
    let (code, renamed) = command(
        &router,
        "renameProject",
        serde_json::json!({"projectId":id,"name":"  Friendly name  "}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Friendly name");
    assert_eq!(renamed["path"], before["projects"][0]["path"]);
    let (_, after) = command(&router, "listSessions", serde_json::json!({})).await;
    assert_eq!(before["sessions"], after["sessions"]);
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .reader()
            .unwrap()
            .projects()
            .unwrap()[0]
            .name,
        "Friendly name"
    );
    for name in [
        " ".to_owned(),
        "x".repeat(boblm_tuning::MAX_PROJECT_NAME_CHARS + 1),
    ] {
        let (code, _) = command(
            &router,
            "renameProject",
            serde_json::json!({"projectId":id,"name":name}),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }
    let (code, _) = command(
        &router,
        "renameProject",
        serde_json::json!({"projectId":uuid::Uuid::new_v4().to_string(),"name":"Gone"}),
    )
    .await;
    assert_ne!(code, StatusCode::OK);
}
