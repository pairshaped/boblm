//! Explicit, bounded live probe through the server's one-key credential boundary.
//! Run only when authorized; uses isolated state under target and never prints raw events.
use async_trait::async_trait;
use boblm_pi_driver::{McpEndpoint, PiSession};
use boblm_protocol::Event;
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::{process::Command, time::timeout};
const MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";
struct NeverSpawn;
#[async_trait]
impl DriverFactory for NeverSpawn {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        Err(io::Error::other("The probe does not start peer agents."))
    }
}
async fn events(s: &mut PiSession, until_tool: bool) -> io::Result<()> {
    loop {
        let e = timeout(Duration::from_secs(45), s.events.recv())
            .await
            .map_err(|_| {
                io::Error::other("The Pi probe exceeded 45 seconds; no request was retried.")
            })?
            .ok_or_else(|| io::Error::other("Pi exited during the probe."))?;
        match e {
            Event::CommandLifecycle {
                command_uuid,
                state,
            } => {
                println!("delivery {command_uuid} {state:?}");
                if command_uuid == "probe-steer" && state == boblm_protocol::LifecycleState::Started
                {
                    s.request_interrupt().await?;
                }
            }
            Event::Assistant { message } => {
                if let Some(u) = message.get("pi_usage") {
                    println!("usage {}", u);
                }
                for b in message["content"].as_array().into_iter().flatten() {
                    if b["type"] == "text" {
                        println!("answer {}", b["text"].as_str().unwrap_or(""));
                    }
                    if b["type"] == "tool_use" {
                        println!("tool {}", b["name"]);
                        if until_tool {
                            return Ok(());
                        }
                    }
                }
            }
            Event::User { message } => {
                println!("tool_result error={}", message["content"][0]["is_error"])
            }
            Event::Result { subtype, is_error } => {
                println!("result {}", subtype.as_deref().unwrap_or("none"));
                if is_error == Some(true) {
                    return Err(io::Error::other(
                        "Pi reported a provider error; the probe will not retry.",
                    ));
                }
                return Ok(());
            }
            _ => {}
        }
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode=std::env::args().nth(1).ok_or("Choose remember, resume, tools, steer or interrupt; no mode runs more than one owner turn.")?;
    if !["remember", "resume", "tools", "steer", "interrupt"].contains(&mode.as_str()) {
        return Err("Choose a supported Pi probe mode.".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/pi-live")
        .canonicalize()?;
    let state = root.join("state");
    std::fs::create_dir_all(&state)?;
    let work = root.join("work");
    std::fs::create_dir_all(&work)?;
    let store = Store::open(root.join("probe.sqlite"))?;
    if store.reader()?.sessions()?.is_empty() {
        store.create_session(
            "caller",
            work.to_str().ok_or("The probe folder is not UTF-8.")?,
            1,
        )?;
    }
    let owner = "fictional-local-probe-owner";
    let app = App::open(store, Arc::new(NeverSpawn), owner.into()).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp/caller", listener.local_addr()?);
    let router = app.router(Some(root.clone()));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let token = boblm_server::mcp::session_token(owner, "caller");
    let resume = if mode == "resume" {
        Some(std::fs::read_to_string(root.join("resume-path"))?)
    } else {
        None
    };
    let mut command = Command::new(boblm_tuning::PI_BINARY);
    let limit = root.join("limit.mjs");
    std::fs::write(
        &limit,
        r#"export default pi => { let calls = 0; pi.on('before_provider_request', event => { if (++calls > 2) throw new Error('The probe permits at most two provider calls per process.'); return {...event.payload, max_tokens:128}; }); };"#,
    )?;
    command.arg("--extension").arg(limit);

    if mode == "steer" || mode == "interrupt" {
        command.args(["--tools", "bash"]);
    } else if mode == "tools" {
        command.arg("--no-builtin-tools");
    } else {
        command.arg("--no-tools");
    }
    let spawned = boblm_server::pi::spawn(
        command,
        &work,
        &state,
        resume.as_deref(),
        MODEL,
        Some(boblm_tuning::PI_DEFAULT_EFFORT),
        (mode == "tools").then_some(McpEndpoint {
            url: &url,
            token: &token,
        }),
    )
    .await;
    let outcome = match spawned {
        Err(e) => Err(e),
        Ok(mut s) => {
            let work = async {
                let prompt = match mode.as_str() {
                    "remember" => "Remember the codeword cobalt and reply OK on one line.",
                    "resume" => "Reply with only the codeword I asked you to remember.",
                    "tools" => "Call list_projects once and name its project on one line.",
                    "steer" => "Run sleep 2 once, then reply FIRST on one line.",
                    _ => "Run sleep 3 once, then reply DONE on one line.",
                };
                s.send_message("probe-first", prompt, None).await?;
                if mode == "steer" || mode == "interrupt" {
                    events(&mut s, true).await?;
                    if mode == "steer" {
                        s.send_message(
                            "probe-steer",
                            "Reply SECOND on one line instead.",
                            Some(boblm_protocol::Priority::Now),
                        )
                        .await?;
                    } else {
                        s.request_interrupt().await?;
                    }
                }
                events(&mut s, false).await?;
                if mode == "remember" {
                    std::fs::write(
                        root.join("resume-path"),
                        s.session_id()
                            .ok_or_else(|| io::Error::other("Pi reported no resume path."))?,
                    )?;
                }
                Ok(())
            }
            .await;
            let stopped = s.stop().await;
            work.and(stopped)
        }
    };
    app.shutdown().await?;
    server.abort();
    let _ = server.await;
    outcome?;
    Ok(())
}
