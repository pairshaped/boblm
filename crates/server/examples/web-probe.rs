//! Explicit, bounded live check of the web tools through the server's MCP boundary.
//! Run only when authorized: it makes one paid OpenAI web search and one public page
//! fetch, using the real SOPS key. It never prints the key and uses isolated state
//! under target. The live boblm server is never touched.
use async_trait::async_trait;
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use std::{io, sync::Arc, time::Duration};

struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Err(io::Error::other(
            "The web probe does not start peer agents.",
        ))
    }
}

async fn call(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    name: &str,
    args: serde_json::Value,
) -> Result<(bool, String), Box<dyn std::error::Error>> {
    let response = client
        .post(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .body(
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})
                .to_string(),
        )
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    if !status.is_success() {
        return Err(format!("{name} answered HTTP {status}: {body}").into());
    }
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    let text = body["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok((is_error, text))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).unwrap_or_else(|| {
        "What is the latest stable version of the Rust programming language and when was it released?"
            .to_owned()
    });
    let page = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "https://example.com/".to_owned());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/web-live");
    std::fs::create_dir_all(&root)?;
    let store = Store::open(root.join("probe.sqlite"))?;
    if store.reader()?.sessions()?.is_empty() {
        store.create_session(
            "probe",
            root.to_str().ok_or("The probe folder is not UTF-8.")?,
            1,
        )?;
    }
    let owner = "fictional-local-web-probe-owner";
    let app = App::open(store, Arc::new(NeverSpawn), owner.into()).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp/probe", listener.local_addr()?);
    let router = app.router(Some(root.clone()));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let token = boblm_server::mcp::session_token(owner, "probe");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let started = std::time::Instant::now();
    let (is_error, text) = call(
        &client,
        &url,
        &token,
        "web_search",
        serde_json::json!({"query": query}),
    )
    .await?;
    println!(
        "web_search error={is_error} elapsed={:?}",
        started.elapsed()
    );
    println!("web_search reply: {}", truncate(&text, 2000));

    let started = std::time::Instant::now();
    let (is_error, text) = call(
        &client,
        &url,
        &token,
        "web_fetch",
        serde_json::json!({"url": page}),
    )
    .await?;
    println!("web_fetch error={is_error} elapsed={:?}", started.elapsed());
    println!("web_fetch reply: {}", truncate(&text, 2000));

    app.shutdown().await?;
    server.abort();
    let _ = server.await;
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("...");
    out
}
