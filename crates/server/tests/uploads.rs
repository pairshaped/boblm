use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use boblm_protocol::{Event, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use std::{io, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use tower::ServiceExt;

const TOKEN: &str = "integration-test-token-only";
const U1: &str = "10000000-0000-4000-8000-000000000001";
const U2: &str = "10000000-0000-4000-8000-000000000002";
const M1: &str = "20000000-0000-4000-8000-000000000001";

struct Fake {
    events: mpsc::UnboundedReceiver<Event>,
    inputs: mpsc::UnboundedSender<String>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, _: &str, content: &str, _: Option<Priority>) -> io::Result<()> {
        self.inputs.send(content.into()).map_err(io::Error::other)
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

struct Fixture {
    store: Store,
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    app: Arc<App>,
    router: Router,
    inputs: mpsc::UnboundedReceiver<String>,
    _events: mpsc::UnboundedSender<Event>,
}
async fn fixture() -> Fixture {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("uploads.sqlite")).unwrap();
    store.create_session("s", "/local", 1).unwrap();
    store.create_session("other", "/local", 1).unwrap();
    let (events_tx, events) = mpsc::unbounded_channel();
    let (inputs_tx, inputs) = mpsc::unbounded_channel();
    let factory = Arc::new(Factory(Mutex::new(Some(Fake {
        events,
        inputs: inputs_tx,
    }))));
    let app = App::open(store.clone(), factory, TOKEN.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    Fixture {
        store,
        root: temp.path().join("uploads"),
        _temp: temp,
        app,
        router,
        inputs,
        _events: events_tx,
    }
}
async fn put(
    router: &Router,
    session: &str,
    id: &str,
    name: &str,
    content_type: &str,
    bytes: Vec<u8>,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/sessions/{session}/uploads/{id}"))
        .header("X-Filename", name)
        .header("Content-Type", content_type);
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
async fn get(router: &Router, session: &str, id: &str, token: bool) -> axum::response::Response {
    let mut request = Request::builder().uri(format!("/sessions/{session}/uploads/{id}"));
    if token {
        request = request.header("Authorization", format!("Bearer {TOKEN}"));
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
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
fn files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for directory in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for file in std::fs::read_dir(directory.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            out.push(file.path());
        }
    }
    out
}

#[tokio::test]
async fn upload_stores_a_private_file_and_is_idempotent() {
    let f = fixture().await;
    let bytes = b"\x89PNG not really".to_vec();
    let (status, info) = put(
        &f.router,
        "s",
        U1,
        "Screen shot ../1.png",
        "image/png",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        info,
        serde_json::json!({"id":U1,"name":"Screen shot ../1.png","contentType":"image/png","size":bytes.len()})
    );
    let stored = files(&f.root);
    assert_eq!(stored.len(), 1);
    let name = stored[0].file_name().unwrap().to_str().unwrap();
    assert_eq!(
        name,
        format!("{U1}-Screen_shot_.._1.png"),
        "unsafe characters become underscores"
    );
    assert_eq!(std::fs::read(&stored[0]).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&stored[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(stored[0].parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // The same upload again returns the record and stores nothing new.
    let (status, again) = put(
        &f.router,
        "s",
        U1.to_uppercase().as_str(),
        "Screen shot ../1.png",
        "image/png",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!((status, again), (StatusCode::OK, info));
    assert_eq!(files(&f.root).len(), 1, "no temporary file is left behind");
    // The same id with different bytes, or a different name, conflicts and changes nothing.
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "Screen shot ../1.png",
        "image/png",
        b"other".to_vec(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "renamed.png",
        "image/png",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(std::fs::read(&files(&f.root)[0]).unwrap(), bytes);
    assert_eq!(files(&f.root).len(), 1);
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn upload_requires_auth_a_live_session_and_valid_headers() {
    let f = fixture().await;
    let body = || b"x".to_vec();
    let (status, _) = put(&f.router, "s", U1, "a.txt", "text/plain", body(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        body(),
        Some("wrong-token-wrong-token"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = put(
        &f.router,
        "missing",
        U1,
        "a.txt",
        "text/plain",
        body(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = put(
        &f.router,
        "s",
        "not-a-uuid",
        "a.txt",
        "text/plain",
        body(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for name in ["", "%00x", "a%0Ab", &"n".repeat(256)] {
        let (status, _) = put(&f.router, "s", U1, name, "text/plain", body(), Some(TOKEN)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "name {name:?}");
    }
    assert!(files(&f.root).is_empty());
    command(
        &f.router,
        "archiveSession",
        serde_json::json!({"sessionId":"s"}),
    )
    .await;
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        body(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "archived sessions can prepare uploads without reopening"
    );
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn uploads_over_the_limit_are_refused_and_leave_nothing() {
    let f = fixture().await;
    let (status, reply) = put(
        &f.router,
        "s",
        U1,
        "big.bin",
        "application/octet-stream",
        vec![0; (50 << 20) + 1],
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{reply}");
    assert!(files(&f.root).is_empty(), "the partial file is removed");
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "ok.bin",
        "application/octet-stream",
        vec![0; 50 << 20],
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "exactly the limit is accepted");
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn downloads_are_authorized_scoped_and_cannot_run_as_scripts() {
    let f = fixture().await;
    put(
        &f.router,
        "s",
        U1,
        "pic.png",
        "image/png",
        b"png bytes".to_vec(),
        Some(TOKEN),
    )
    .await;
    put(
        &f.router,
        "s",
        U2,
        "evil%20%C3%A9.html",
        "text/html",
        b"<script>alert(1)</script>".to_vec(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        get(&f.router, "s", U1, false).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&f.router, "other", U1, true).await.status(),
        StatusCode::NOT_FOUND,
        "another session's upload"
    );
    assert_eq!(
        get(&f.router, "s", "30000000-0000-4000-8000-000000000003", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let image = get(&f.router, "s", U1, true).await;
    assert_eq!(image.status(), StatusCode::OK);
    assert_eq!(image.headers()["content-type"], "image/png");
    assert_eq!(image.headers()["x-content-type-options"], "nosniff");
    assert_eq!(image.headers()["content-security-policy"], "sandbox");
    assert!(image.headers().get("content-disposition").is_none());
    assert_eq!(
        image
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"png bytes"
    );

    let html = get(&f.router, "s", U2, true).await;
    assert_eq!(html.headers()["content-type"], "application/octet-stream");
    assert_eq!(html.headers()["x-content-type-options"], "nosniff");
    assert_eq!(html.headers()["content-security-policy"], "sandbox");
    let disposition = html.headers()["content-disposition"].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename*=UTF-8''evil%20%C3%A9%2Ehtml"),
        "{disposition}"
    );
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn sending_with_attachments_delivers_their_paths_and_rejects_bad_ones() {
    let mut f = fixture().await;
    put(
        &f.router,
        "s",
        U1,
        "notes.txt",
        "text/plain",
        b"hello".to_vec(),
        Some(TOKEN),
    )
    .await;
    put(
        &f.router,
        "other",
        U2,
        "foreign.txt",
        "text/plain",
        b"x".to_vec(),
        Some(TOKEN),
    )
    .await;
    let send = |content: &str, attachments: serde_json::Value| serde_json::json!({"sessionId":"s","messageId":M1,"content":content,"attachments":attachments});
    // Unknown, foreign and malformed attachment IDs are refused without ending the session.
    for bad in [
        serde_json::json!(["30000000-0000-4000-8000-000000000003"]),
        serde_json::json!([U2]),
        serde_json::json!(["nope"]),
        serde_json::json!(vec![U1; 11]),
    ] {
        let (status, _) = command(&f.router, "sendMessage", send("see", bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = command(&f.router, "sendMessage", send("", serde_json::json!([]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, reply) = command(
        &f.router,
        "sendMessage",
        send("see this", serde_json::json!([U1])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let delivered = tokio::time::timeout(Duration::from_secs(3), f.inputs.recv())
        .await
        .unwrap()
        .unwrap();
    let path = files(&f.root)
        .into_iter()
        .find(|p| p.to_str().unwrap().ends_with("notes.txt"))
        .unwrap();
    assert_eq!(
        delivered,
        format!(
            "see this\n\nAttached files:\n- {} (text/plain, 5 B, original name \"notes.txt\")",
            path.display()
        )
    );
    // An identical retry is the same message.
    let (status, _) = command(
        &f.router,
        "sendMessage",
        send("see this", serde_json::json!([U1])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), f.inputs.recv())
            .await
            .is_err()
    );
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoned_uploads_are_swept_with_their_files() {
    let f = fixture().await;
    put(
        &f.router,
        "s",
        U1,
        "old.txt",
        "text/plain",
        b"x".to_vec(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        f.app
            .sweep_orphan_uploads(Duration::from_secs(3600))
            .await
            .unwrap(),
        0,
        "too new"
    );
    assert_eq!(f.app.sweep_orphan_uploads(Duration::ZERO).await.unwrap(), 1);
    assert!(files(&f.root).is_empty());
    assert_eq!(
        get(&f.router, "s", U1, true).await.status(),
        StatusCode::NOT_FOUND
    );
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_identical_retry_restores_a_file_lost_after_the_record_was_written() {
    let f = fixture().await;
    let bytes = b"precious".to_vec();
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let path = files(&f.root).pop().unwrap();
    std::fs::remove_file(&path).unwrap();
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "the retry stores the file again"
    );
    assert_eq!(files(&f.root).len(), 1);
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_startup_sweep_removes_old_temporary_files_left_by_a_crash() {
    let f = fixture().await;
    put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        b"x".to_vec(),
        Some(TOKEN),
    )
    .await;
    let directory = files(&f.root).pop().unwrap().parent().unwrap().to_owned();
    let old = directory.join(".tmpOLD");
    let fresh = directory.join(".tmpFRESH");
    for path in [&old, &fresh] {
        std::fs::write(path, b"partial").unwrap();
    }
    let two_days = Duration::from_secs(2 * 24 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - two_days)
        .unwrap();
    f.app
        .sweep_orphan_uploads(Duration::from_secs(24 * 3600))
        .await
        .unwrap();
    assert!(!old.exists(), "an old partial upload is removed");
    assert!(fresh.exists(), "a recent one may still be streaming");
    assert_eq!(
        files(&f.root)
            .iter()
            .filter(|p| p.file_name().unwrap() == ".tmpOLD")
            .count(),
        0
    );
    assert!(
        files(&f.root)
            .iter()
            .any(|p| p.to_str().unwrap().ends_with("a.txt")),
        "recorded uploads stay"
    );
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_upload_cannot_be_sent_until_its_file_is_stored_and_a_retry_completes_it() {
    let mut f = fixture().await;
    // A crash between writing the record and storing the file leaves this state.
    let bytes = b"late bytes".to_vec();
    let sha256 = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&bytes))
    };
    let path = f.root.join("crashed-upload");
    f.store
        .create_attachment(boblm_store::NewAttachment {
            id: U1,
            session: "s",
            name: "late.txt",
            content_type: "text/plain",
            size: bytes.len() as i64,
            sha256: &sha256,
            path: path.to_str().unwrap(),
            created_at: 1,
        })
        .unwrap();
    let send =
        serde_json::json!({"sessionId":"s","messageId":M1,"content":"see","attachments":[U1]});
    let (status, _) = command(&f.router, "sendMessage", send.clone()).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unstored upload cannot be claimed"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), f.inputs.recv())
            .await
            .is_err()
    );
    // The identical PUT finishes the job.
    let (status, _) = put(
        &f.router,
        "s",
        U1,
        "late.txt",
        "text/plain",
        bytes.clone(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let (status, reply) = command(&f.router, "sendMessage", send).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    f.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unreadable_upload_is_reported_as_a_fault_not_as_missing() {
    let f = fixture().await;
    put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        b"x".to_vec(),
        Some(TOKEN),
    )
    .await;
    let path = files(&f.root).pop().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let response = get(&f.router, "s", U1, true).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        text.contains("read the upload from disk") && !text.contains("missing"),
        "{text}"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(&path).unwrap();
    let response = get(&f.router, "s", U1, true).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("missing from disk"));
    let (status, conflict) = put(
        &f.router,
        "s",
        U1,
        "a.txt",
        "text/plain",
        b"other".to_vec(),
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        conflict["message"]
            .as_str()
            .unwrap()
            .contains("Use a new upload ID"),
        "{conflict}"
    );
    f.app.shutdown().await.unwrap();
}

// ---- Images an agent shows in a chat (ADR 0032) ----

use boblm_server::agent_images::ImageArgs;

fn png() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend((0..200u8).map(|b| b.wrapping_mul(7)));
    bytes
}
fn args(path: &std::path::Path, caption: Option<&str>) -> ImageArgs {
    ImageArgs {
        path: path.to_str().unwrap().into(),
        caption: caption.map(str::to_owned),
    }
}
async fn history(f: &Fixture, session: &str) -> Vec<serde_json::Value> {
    let (status, page) = command(
        &f.router,
        "chatHistory",
        serde_json::json!({"sessionId": session}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["events"].as_array().unwrap().clone()
}

#[tokio::test]
async fn a_shown_image_is_kept_served_and_listed_in_history_even_after_the_agent_deletes_its_file()
{
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let source = work.path().join("shot.png");
    std::fs::write(&source, png()).unwrap();
    let reply = f
        .app
        .show_image("s", args(&source, Some("Home page")))
        .await
        .unwrap();
    assert_eq!(reply["shown"], true);
    assert_eq!(reply["contentType"], "image/png");
    let id = reply["attachmentId"].as_str().unwrap().to_owned();
    // It is in the chat's history as an assistant message that carries the image.
    let shown: Vec<_> = history(&f, "s")
        .await
        .into_iter()
        .filter(|e| e["kind"] == "message_appended")
        .collect();
    assert_eq!(shown.len(), 1);
    let payload = &shown[0]["payload"];
    assert_eq!(payload["role"], "assistant");
    assert_eq!(payload["content"], "Home page");
    assert_eq!(payload["delivery_state"], "started");
    assert_eq!(payload["attachments"][0]["id"], id.as_str());
    assert_eq!(payload["attachments"][0]["name"], "shot.png");
    assert_eq!(payload["attachments"][0]["contentType"], "image/png");
    // The owner's token fetches it; nobody else does, and no other chat does.
    let served = get(&f.router, "s", &id, true).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()["content-type"], "image/png");
    assert_eq!(served.headers()["x-content-type-options"], "nosniff");
    let body = served.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), png().as_slice());
    assert_eq!(
        get(&f.router, "s", &id, false).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&f.router, "other", &id, true).await.status(),
        StatusCode::NOT_FOUND
    );
    // boblm kept its own private copy, so the agent may delete or overwrite its file.
    std::fs::remove_file(&source).unwrap();
    assert_eq!(
        get(&f.router, "s", &id, true).await.status(),
        StatusCode::OK
    );
    let kept = files(&f.root);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        std::fs::metadata(&kept[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn an_image_without_a_caption_is_shown_under_its_file_name() {
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let source = work.path().join("final-result.png");
    std::fs::write(&source, png()).unwrap();
    f.app
        .show_image("s", args(&source, Some("   ")))
        .await
        .unwrap();
    let events = history(&f, "s").await;
    let payload = &events
        .iter()
        .find(|e| e["kind"] == "message_appended")
        .unwrap()["payload"];
    assert_eq!(payload["content"], "", "a blank caption is no caption");
    assert_eq!(payload["attachments"][0]["name"], "final-result.png");
}

#[tokio::test]
async fn the_kind_of_image_comes_from_its_contents_not_its_name() {
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let mut webp = b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec();
    webp.extend([0u8; 40]);
    let cases: [(&str, Vec<u8>, &str); 4] = [
        ("looks-like-text.txt", png(), "image/png"),
        (
            "photo",
            [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0u8; 64][..]].concat(),
            "image/jpeg",
        ),
        (
            "anim.bin",
            [&b"GIF89a"[..], &[0u8; 32][..]].concat(),
            "image/gif",
        ),
        ("modern.jpg", webp, "image/webp"),
    ];
    for (name, bytes, expected) in cases {
        let path = work.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let reply = f.app.show_image("s", args(&path, None)).await.unwrap();
        assert_eq!(reply["contentType"], expected, "{name}");
    }
}

#[tokio::test]
async fn files_that_are_not_safe_images_are_refused_with_a_reason_and_nothing_is_kept() {
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let text = work.path().join("notes.png");
    std::fs::write(&text, b"just some text pretending to be a picture").unwrap();
    let svg = work.path().join("logo.svg");
    std::fs::write(
        &svg,
        b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>",
    )
    .unwrap();
    let folder = work.path().join("a-folder");
    std::fs::create_dir(&folder).unwrap();
    let huge = work.path().join("huge.png");
    let file = std::fs::File::create(&huge).unwrap();
    file.set_len(boblm_tuning::MAX_AGENT_IMAGE_BYTES + 1)
        .unwrap();
    let fine = work.path().join("fine.png");
    std::fs::write(&fine, png()).unwrap();
    let long_caption = "x".repeat(boblm_tuning::MAX_IMAGE_CAPTION_CHARS + 1);
    let cases: Vec<(ImageArgs, &str)> = vec![
        (
            ImageArgs {
                path: "relative/shot.png".into(),
                caption: None,
            },
            "absolute path",
        ),
        (
            ImageArgs {
                path: String::new(),
                caption: None,
            },
            "absolute path",
        ),
        (
            args(&work.path().join("missing.png"), None),
            "Could not read",
        ),
        (args(&folder, None), "not a file"),
        (args(&text, None), "not a PNG, JPEG, GIF or WebP"),
        (args(&svg, None), "not a PNG, JPEG, GIF or WebP"),
        (args(&huge, None), "the most BobLM will show"),
        (args(&fine, Some(&long_caption)), "caption"),
    ];
    for (case, reason) in cases {
        let shown = case.path.clone();
        let error = f.app.show_image("s", case).await.unwrap_err();
        assert!(
            error.contains(reason),
            "{shown:?} gave {error:?}, expected {reason:?}"
        );
    }
    assert!(files(&f.root).is_empty(), "a refused image leaves no file");
    assert!(
        history(&f, "s")
            .await
            .iter()
            .all(|e| e["kind"] != "message_appended"),
        "and no message"
    );
}

#[tokio::test]
async fn shown_images_survive_the_orphan_sweep_and_go_when_their_chat_is_deleted() {
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let source = work.path().join("shot.png");
    std::fs::write(&source, png()).unwrap();
    let id = f.app.show_image("s", args(&source, None)).await.unwrap()["attachmentId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Uploads never attached to a message are swept; this one is attached to its message.
    assert_eq!(f.app.sweep_orphan_uploads(Duration::ZERO).await.unwrap(), 0);
    assert_eq!(
        get(&f.router, "s", &id, true).await.status(),
        StatusCode::OK
    );
    f.store.set_archived("s", true).unwrap();
    let (status, reply) = command(
        &f.router,
        "deleteSession",
        serde_json::json!({"sessionId": "s"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(
        files(&f.root).is_empty(),
        "the copy is removed with the chat"
    );
}

#[tokio::test]
async fn an_image_is_shown_only_in_the_chat_that_asked() {
    let f = fixture().await;
    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let source = work.path().join("shot.png");
    std::fs::write(&source, png()).unwrap();
    f.app
        .show_image("other", args(&source, Some("for other")))
        .await
        .unwrap();
    let in_s = history(&f, "s").await;
    assert!(in_s.iter().all(|e| e["kind"] != "message_appended"));
    let in_other = history(&f, "other").await;
    assert_eq!(
        in_other
            .iter()
            .filter(|e| e["kind"] == "message_appended")
            .count(),
        1
    );
    let error = f
        .app
        .show_image("no-such-chat", args(&source, None))
        .await
        .unwrap_err();
    assert!(error.contains("could not"), "{error}");
}

async fn mcp(
    f: &Fixture,
    session: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let capability = boblm_server::mcp::session_token(TOKEN, session);
    let response = f
        .router
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
                    serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn an_agent_shows_an_image_through_its_own_mcp_connection() {
    let f = fixture().await;
    let listed = mcp(&f, "s", "tools/list", serde_json::json!({})).await;
    assert!(
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != "notify"),
        "agents are not offered notification calls"
    );
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "show_image")
        .expect("agents are offered show_image");
    assert!(tool["description"].as_str().unwrap().contains("screenshot"));
    let properties = &tool["inputSchema"]["properties"];
    assert!(properties["path"].is_object() && properties["caption"].is_object());
    assert!(
        properties.get("sessionId").is_none(),
        "the chat is the caller's own and cannot be chosen"
    );

    let work = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let source = work.path().join("result.png");
    std::fs::write(&source, png()).unwrap();
    let ok = mcp(
        &f,
        "s",
        "tools/call",
        serde_json::json!({"name":"show_image","arguments":{"path":source,"caption":"After the change"}}),
    )
    .await;
    assert_ne!(ok["result"]["isError"], true, "{ok}");
    let text = ok["result"]["content"][0]["text"].as_str().unwrap();
    let reply: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(reply["shown"], true);
    let events = history(&f, "s").await;
    let message = events
        .iter()
        .find(|e| e["kind"] == "message_appended")
        .unwrap();
    assert_eq!(message["payload"]["content"], "After the change");
    assert_eq!(
        history(&f, "other")
            .await
            .iter()
            .filter(|e| e["kind"] == "message_appended")
            .count(),
        0
    );

    // A refusal comes back as a tool error the agent can read and act on, not as a failed call.
    let bad = mcp(
        &f,
        "s",
        "tools/call",
        serde_json::json!({"name":"show_image","arguments":{"path":"relative.png"}}),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true);
    assert!(
        bad["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("absolute path")
    );
}
