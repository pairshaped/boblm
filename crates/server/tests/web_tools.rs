//! Fake local endpoints exercise both web tools end to end. No real provider or page is
//! contacted; the credential test runs the real `sops` and `age-keygen` with a throwaway key.
use axum::{
    Router,
    body::Bytes,
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::{
    app::App,
    web::{self, AddressPolicy},
};
use boblm_store::{Session, Store};
use http_body_util::BodyExt;
use serde_json::json;
use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};
use tower::ServiceExt;

/// Start a fake server on a loopback port and return its base URL. The task is aborted by
/// the caller at the end of the test.
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{address}"), handle)
}

/// A fake Responses API reply that echoes the model and tool it was sent, so the test can
/// confirm the request shape, plus two citations and one duplicate.
async fn responses_ok(body: Bytes) -> Response {
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let model = request["model"].as_str().unwrap_or("missing").to_owned();
    let tool = request["tools"][0]["type"]
        .as_str()
        .unwrap_or("missing")
        .to_owned();
    let choice = request["tool_choice"]
        .as_str()
        .unwrap_or("missing")
        .to_owned();
    let store = request["store"].as_bool();
    let reply = json!({
        "output": [
            {"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"x"}},
            {"type":"message","id":"m1","status":"completed","role":"assistant","content":[
                {"type":"output_text",
                 "text":format!("model={model} tool={tool} choice={choice} store={store:?} answer"),
                 "annotations":[
                    {"type":"url_citation","start_index":0,"end_index":1,"url":"https://a.test/","title":"A"},
                    {"type":"url_citation","start_index":0,"end_index":1,"url":"https://b.test/","title":"B"},
                    {"type":"url_citation","start_index":0,"end_index":1,"url":"https://a.test/","title":"Duplicate"}
                 ]}
            ]}
        ]
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        reply.to_string(),
    )
        .into_response()
}

async fn responses_provider_error() -> Response {
    let body =
        json!({"error":{"message":"You exceeded your current quota","type":"insufficient_quota"}});
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn slow() -> Response {
    tokio::time::sleep(Duration::from_secs(3)).await;
    "late".into_response()
}

#[tokio::test]
async fn web_search_returns_the_answer_and_cited_sources() {
    let (base, server) = serve(Router::new().route("/v1/responses", post(responses_ok))).await;
    let value = web::search(
        "rust language",
        "test-key",
        &format!("{base}/v1/responses"),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(value["query"], "rust language");
    let answer = value["answer"].as_str().unwrap();
    assert!(answer.contains(&format!("model={}", boblm_tuning::OPENAI_WEB_SEARCH_MODEL)));
    assert!(answer.contains("tool=web_search"));
    assert!(answer.contains("choice=required"));
    assert!(answer.contains("store=Some(false)"));
    assert!(answer.contains("answer"));
    assert_eq!(value["searchCalls"], 1);
    let sources = value["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2, "the duplicate citation must be dropped");
    assert_eq!(sources[0]["url"], "https://a.test/");
    assert_eq!(sources[0]["title"], "A");
    assert_eq!(sources[1]["url"], "https://b.test/");
    server.abort();
}

#[tokio::test]
async fn web_search_reports_a_provider_error_with_its_reason() {
    let (base, server) =
        serve(Router::new().route("/v1/responses", post(responses_provider_error))).await;
    let error = web::search(
        "anything",
        "test-key",
        &format!("{base}/v1/responses"),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert!(error.contains("OpenAI refused"));
    assert!(error.contains("429"));
    assert!(error.contains("quota"));
    server.abort();
}

#[tokio::test]
async fn web_search_stops_at_the_timeout() {
    let (base, server) = serve(Router::new().route("/v1/responses", post(slow))).await;
    let error = web::search(
        "anything",
        "test-key",
        &format!("{base}/v1/responses"),
        Duration::from_millis(150),
    )
    .await
    .unwrap_err();
    assert!(error.contains("did not answer"), "{error}");
    server.abort();
}

#[tokio::test]
async fn web_search_rejects_an_empty_query_without_calling_the_provider() {
    let error = web::search(
        "   ",
        "test-key",
        "http://127.0.0.1:1/v1/responses",
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.contains("non-empty query"), "{error}");
}

async fn page_html() -> Response {
    let body = "<html><head><title>Example &amp; Co</title></head><body><h1>Hello</h1>\
        <p>World <a href=\"https://x.test/\">link</a></p><script>ignore()</script></body></html>";
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

async fn page_text() -> Response {
    ([(header::CONTENT_TYPE, "text/plain")], "plain body").into_response()
}

async fn page_big() -> Response {
    let body = "a".repeat(boblm_tuning::WEB_FETCH_MAX_BYTES + 1024);
    ([(header::CONTENT_TYPE, "text/plain")], body).into_response()
}

async fn page_image() -> Response {
    ([(header::CONTENT_TYPE, "image/png")], vec![0u8, 1, 2, 3]).into_response()
}

async fn redirect_to_private() -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, "http://10.9.8.7/secret")],
    )
        .into_response()
}

async fn fetch_from_fake(url: &str, timeout: Duration) -> Result<serde_json::Value, String> {
    web::fetch_with(url, timeout, AddressPolicy::allow_loopback_for_tests()).await
}

#[tokio::test]
async fn web_fetch_converts_html_to_readable_text() {
    let (base, server) = serve(Router::new().route("/page", get(page_html))).await;
    let value = fetch_from_fake(&format!("{base}/page"), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(value["title"], "Example & Co");
    assert_eq!(value["contentType"], "text/html");
    assert_eq!(value["truncated"], false);
    let content = value["content"].as_str().unwrap();
    assert!(content.contains("Hello"));
    assert!(content.contains("[link](https://x.test/)"));
    assert!(!content.contains("ignore()"));
    server.abort();
}

#[tokio::test]
async fn web_fetch_returns_plain_text_unchanged() {
    let (base, server) = serve(Router::new().route("/text", get(page_text))).await;
    let value = fetch_from_fake(&format!("{base}/text"), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(value["content"], "plain body");
    assert_eq!(value["contentType"], "text/plain");
    server.abort();
}

#[tokio::test]
async fn web_fetch_refuses_an_oversized_page() {
    let (base, server) = serve(Router::new().route("/big", get(page_big))).await;
    let error = fetch_from_fake(&format!("{base}/big"), Duration::from_secs(30))
        .await
        .unwrap_err();
    assert!(error.contains("larger than"), "{error}");
    assert!(error.contains("byte limit"), "{error}");
    server.abort();
}

#[tokio::test]
async fn web_fetch_refuses_non_text_content() {
    let (base, server) = serve(Router::new().route("/image", get(page_image))).await;
    let error = fetch_from_fake(&format!("{base}/image"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(error.contains("image/png"), "{error}");
    assert!(error.contains("cannot read as text"), "{error}");
    server.abort();
}

#[tokio::test]
async fn web_fetch_refuses_a_bad_url_scheme() {
    let error = web::fetch("ftp://example.com/file", Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(error.contains("http and https"), "{error}");
    assert!(error.contains("ftp"), "{error}");
}

#[tokio::test]
async fn web_fetch_stops_at_the_timeout() {
    let (base, server) = serve(Router::new().route("/slow", get(slow))).await;
    let error = fetch_from_fake(&format!("{base}/slow"), Duration::from_millis(150))
        .await
        .unwrap_err();
    assert!(error.contains("did not answer"), "{error}");
    server.abort();
}

#[tokio::test]
async fn web_fetch_refuses_a_loopback_url() {
    let error = web::fetch("http://127.0.0.1:1/", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(error.contains("local or private"), "{error}");
    assert!(error.contains("127.0.0.1"), "{error}");
    assert!(error.contains("loopback"), "{error}");
}

#[tokio::test]
async fn web_fetch_refuses_a_hostname_that_resolves_to_loopback() {
    let policy = AddressPolicy::PUBLIC_ONLY
        .resolve_for_tests("public-looking.example", IpAddr::V4(Ipv4Addr::LOCALHOST));
    let error = web::fetch_with(
        "http://public-looking.example/page",
        Duration::from_secs(1),
        policy,
    )
    .await
    .unwrap_err();
    assert!(error.contains("local or private"), "{error}");
    assert!(error.contains("public-looking.example"), "{error}");
    assert!(error.contains("resolves to 127.0.0.1"), "{error}");
    assert!(error.contains("loopback"), "{error}");
}

#[tokio::test]
async fn web_fetch_refuses_a_redirect_to_a_private_address() {
    let (base, server) = serve(Router::new().route("/go", get(redirect_to_private))).await;
    let error = fetch_from_fake(&format!("{base}/go"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(error.contains("local or private"), "{error}");
    assert!(error.contains("10.9.8.7"), "{error}");
    assert!(error.contains("private"), "{error}");
    server.abort();
}

struct NeverSpawn;
#[async_trait::async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        panic!("web tools must not start an agent")
    }
}

#[tokio::test]
async fn mcp_lists_both_web_tools_for_every_caller() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("web.sqlite")).unwrap();
    store.create_session("caller", "/caller", 1).unwrap();
    let token = "web-test-owner-token-only";
    let app = App::open(store, Arc::new(NeverSpawn), token.into())
        .await
        .unwrap();
    let router = app.router(Some(temp.path().into()));
    let capability = boblm_server::mcp::session_token(token, "caller");
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/caller")
                .header("Host", "127.0.0.1")
                .header("Authorization", format!("Bearer {capability}"))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-11-25")
                .body(axum::body::Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let names: Vec<&str> = value["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"web_search"), "{names:?}");
    assert!(names.contains(&"web_fetch"), "{names:?}");
    app.shutdown().await.unwrap();
}

/// A missing OpenAI key must produce a plain-English error that names the key and file and
/// never echoes a value. Runs the real sops and age-keygen with a throwaway identity.
#[tokio::test]
async fn web_search_reports_a_missing_openai_key_without_echoing_secrets() {
    use std::{io::Write, process::Stdio};
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let identity = dir.path().join("age.txt");
    let keygen = std::process::Command::new("age-keygen")
        .arg("-o")
        .arg(&identity)
        .output()
        .unwrap();
    assert!(keygen.status.success());
    let public = String::from_utf8(keygen.stderr)
        .unwrap()
        .split_whitespace()
        .find(|s| s.starts_with("age1"))
        .unwrap()
        .to_owned();
    // SAFETY: this is the only test in the binary that starts a SOPS command.
    unsafe {
        std::env::set_var("SOPS_AGE_KEY_FILE", &identity);
    }
    let file = dir.path().join("keys.json");
    let mut encrypt = std::process::Command::new("sops")
        .args([
            "encrypt",
            "--input-type",
            "json",
            "--output-type",
            "json",
            "--age",
            &public,
            "/dev/stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    encrypt
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"DEEPINFRA_API_KEY":"fictional-provider-value"}"#)
        .unwrap();
    let encrypted = encrypt.wait_with_output().unwrap();
    assert!(encrypted.status.success());
    std::fs::write(&file, encrypted.stdout).unwrap();
    let key: Vec<String> = vec!["OPENAI_API_KEY".into()];
    let error = boblm_server::secrets::read_secret_labeled(
        file.to_str().unwrap(),
        &key,
        dir.path(),
        "the OpenAI API key for web_search",
        "OPENAI_SOPS_KEY in crates/tuning",
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("OPENAI_API_KEY"), "{error}");
    assert!(error.contains("keys.json"), "{error}");
    assert!(!error.contains("fictional-provider-value"), "{error}");
}
