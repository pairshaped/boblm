//! Isolated fake-driver server for browser checks. Never spawns Claude.
use async_trait::async_trait;
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_runner::{Driver, DriverFactory};
use boblm_server::app::App;
use boblm_store::{Session, Store};
use std::{io, sync::Arc};
use tokio::sync::mpsc;
struct Fake {
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}
#[async_trait]
impl Driver for Fake {
    async fn send(&mut self, uuid: &str, content: &str, _: Option<Priority>) -> io::Result<()> {
        self.tx
            .send(Event::CommandLifecycle {
                command_uuid: uuid.into(),
                state: LifecycleState::Started,
            })
            .await
            .map_err(io::Error::other)?;
        let tx = self.tx.clone();
        if content == "/compact" {
            self.tx
                .send(Event::System {
                    subtype: Some("compact_started".into()),
                    session_id: None,
                    status: None,
                })
                .await
                .map_err(io::Error::other)?;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let _ = tx
                    .send(Event::System {
                        subtype: Some("compact_boundary".into()),
                        session_id: None,
                        status: None,
                    })
                    .await;
                let _ = tx
                    .send(Event::Result {
                        subtype: Some("success".into()),
                        is_error: Some(false),
                    })
                    .await;
            });
            return Ok(());
        }
        tokio::spawn(async move {
            for i in 0..80 {
                if tx.send(Event::Assistant{message:serde_json::json!({"id":format!("fixture-{i}"),"content":[{"type":"text","text":format!("Streaming fixture line {i}. The composer must preserve every character and its selection while these events append.")}]})}).await.is_err(){return;}
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let _ = tx
                .send(Event::Result {
                    subtype: Some("success".into()),
                    is_error: Some(false),
                })
                .await;
        });
        Ok(())
    }
    async fn next_event(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
    async fn configure(&mut self, _: &str, _: Option<&str>) -> io::Result<()> {
        Ok(())
    }
    async fn abort(&mut self) -> io::Result<()> {
        self.tx
            .send(Event::Result {
                subtype: Some("interrupted".into()),
                is_error: Some(false),
            })
            .await
            .map_err(io::Error::other)
    }
    async fn shutdown(&mut self) -> io::Result<()> {
        self.rx.close();
        Ok(())
    }
}
struct Factory {
    startup_delay: std::time::Duration,
}
#[async_trait]
impl DriverFactory for Factory {
    async fn spawn(&self, _: &Session) -> io::Result<Box<dyn Driver>> {
        tokio::time::sleep(self.startup_delay).await;
        let (tx, rx) = mpsc::channel(128);
        Ok(Box::new(Fake { tx, rx }))
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A dev-instance config keeps benchmark data and credentials isolated.
    let args: Vec<String> = std::env::args().collect();
    let config = args
        .iter()
        .position(|arg| arg == "--config")
        .and_then(|index| args.get(index + 1))
        .map(std::fs::read_to_string)
        .transpose()?
        .map(|text| serde_json::from_str::<serde_json::Value>(&text))
        .transpose()?;
    let database = config
        .as_ref()
        .and_then(|value| value["database"].as_str())
        .or_else(|| args.get(1).map(String::as_str))
        .ok_or("database path required")?;
    let store = Store::open(database)?;
    if config.is_some() && store.reader()?.sessions()?.is_empty() {
        if config
            .as_ref()
            .is_some_and(|value| value["fixture"] == "subagents")
        {
            seed_subagents(&store)?;
        } else if config
            .as_ref()
            .is_some_and(|value| value["fixture"] == "usage")
        {
            seed_usage(&store)?;
        } else {
            seed_profile(&store)?;
        }
    }
    if config.is_none()
        && let Some(count) = args.get(2)
    {
        let count: usize = count.parse()?;
        store.create_session("history-fixture", "/tmp/example-project", 1)?;
        store.set_name("history-fixture", "Long Chat")?;
        let events = (0..count).map(|i| serde_json::json!({"type":"assistant","message":{"id":format!("history-{i}"),"content":[{"type":"text","text":format!("History message {i}. This is a longer fixture message for verifying the scroll anchor while older pages arrive and the composer stays in place.")}]}})).collect();
        store.record_events("history-fixture", "agent_event", events, 2)?;
        if std::env::args().nth(3).as_deref() == Some("tools") {
            let mut tools = Vec::new();
            for i in 0..8 {
                tools.push(serde_json::json!({"type":"assistant","message":{"id":format!("tool-{i}"),"content":[{"type":"tool_use","id":format!("call-{i}"),"name":"Read","input":{"path":format!("file-{i}.rs")}}]}}));
                tools.push(serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":format!("call-{i}"),"content":"Fixture output", "is_error":i == 7}]}}));
            }
            store.record_events("history-fixture", "agent_event", tools, 3)?;
        }
        if std::env::args().nth(3).as_deref() == Some("peer") {
            for (uuid, sender) in [
                ("owner-message", None),
                ("peer-message", Some("fixture-agent")),
            ] {
                store.append_with_origin(
                    boblm_store::NewMessage {
                        uuid,
                        session: "history-fixture",
                        role: "user",
                        content: "This is a message with a clear source.",
                        delivery_state: "started",
                        created_at: 3,
                    },
                    &[],
                    sender,
                )?;
            }
        }
    }
    let app = App::open(
        store,
        Arc::new(Factory {
            startup_delay: std::time::Duration::from_millis(
                config
                    .as_ref()
                    .and_then(|value| value["startupDelayMs"].as_u64())
                    .unwrap_or(0),
            ),
        }),
        config
            .as_ref()
            .and_then(|value| value["token"].as_str())
            .unwrap_or("browser-fixture-token-only")
            .into(),
    )
    .await?;
    let port = config
        .as_ref()
        .and_then(|value| value["port"].as_u64())
        .unwrap_or(39381);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    eprintln!("listening on {}", listener.local_addr()?);
    let router = app.router(Some("web/dist".into()));
    tokio::select! {r=axum::serve(listener,router)=>{r?;},_=tokio::signal::ctrl_c()=>{}}
    app.shutdown().await?;
    Ok(())
}

/// Deterministic mixed content for history-size and per-message density comparisons.
fn seed_profile(store: &Store) -> Result<(), Box<dyn std::error::Error>> {
    for (id, count, dense) in [
        ("profile-1000", 1_000, false),
        ("profile-10000", 10_000, false),
        ("profile-50000", 50_000, false),
        ("profile-dense", 1_000, true),
    ] {
        store.create_session(id, "/tmp/boblm-profile", 1)?;
        store.set_name(
            id,
            &format!("Profile {count}{}", if dense { " dense" } else { "" }),
        )?;
        for start in (0..count).step_by(500) {
            let events = (start..(start + 500).min(count)).map(|i| {
                let repeat = if dense { 30 } else { 1 };
                let text = match i % 6 {
                    0 => format!("Review {i}: We need to preserve input focus while the reply streams. Check the query, explain the result, and keep the scroll position stable.\n\n").repeat(repeat),
                    1 => "## Review findings\n\n- **Delivery:** wait for acknowledgment.\n- Keep `request_id` stable.\n- [Reference](https://example.com)\n\n> A write is not delivery.\n\n".repeat(repeat),
                    2 => "```rust\nfn delivery(id: &str) -> Result<(), Error> {\n    record(id)?;\n    Ok(())\n}\n```\n\n".repeat(repeat),
                    3 => format!("| Check | Result | Duration |\n| --- | --- | --- |\n{}", "| Render | Passed | 12 ms |\n".repeat(repeat * 10)),
                    4 => return serde_json::json!({"type":"assistant","message":{"id":format!("profile-{i}"),"content":[{"type":"tool_use","id":format!("call-{i}"),"name":"Read","input":{"path":"src/main.rs","output":"Deterministic fixture output.\n".repeat(repeat * 10)}}]}}),
                    _ => return serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":format!("call-{}", i - 1),"content":"Fixture query result.\n".repeat(repeat * 10),"is_error":false}]}}),
                };
                serde_json::json!({"type":"assistant","message":{"id":format!("profile-{i}"),"content":[{"type":"text","text":text}]}})
            }).collect();
            store.record_events(id, "agent_event", events, 2)?;
        }
    }
    Ok(())
}

/// A real transcript endpoint backed by isolated provider files, without launching a provider.
fn seed_subagents(store: &Store) -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(
        std::env::var_os("CLAUDE_CONFIG_DIR")
            .ok_or("Set CLAUDE_CONFIG_DIR to an isolated fixture directory")?,
    );
    let project = root.join("projects/-fixture");
    let children = project.join("fixture-parent/subagents");
    fs::create_dir_all(&children)?;
    store.create_session("subagent-parent", "/fixture", 1)?;
    store.set_name("subagent-parent", "Delivery review")?;
    store.set_agent_session("subagent-parent", "fixture-parent")?;
    let launch = serde_json::json!({"type":"assistant","message":{"id":"launch","content":[{"type":"tool_use","id":"fixture-task","name":"Agent","input":{"description":"Inspect delivery acknowledgments","prompt":"Check delivery acknowledgments."}}]}});
    store.record_event(
        "subagent-parent",
        "agent_event",
        launch.clone(),
        1_792_900_000_000,
    )?;
    fs::write(project.join("fixture-parent.jsonl"), format!("{launch}\n"))?;
    store.record_events("subagent-parent", "agent_event", (0..700).map(|i| serde_json::json!({"type":"assistant","message":{"id":format!("parent-progress-{i}"),"content":[{"type":"text","text":format!("Parent progress {i}. The earlier agent launch is outside this history page.")}]}})).collect(), 1_792_900_100_000)?;
    store.create_session("tool-heavy", "/fixture", 1)?;
    store.set_name("tool-heavy", "Tool-heavy history")?;
    let mut burst = vec![
        serde_json::json!({"type":"assistant","message":{"id":"before-tools","content":[{"type":"text","text":"Conversation before the long tool burst."}]}}),
    ];
    for i in 0..1200 {
        burst.push(serde_json::json!({"type":"assistant","message":{"id":format!("tool-message-{i}"),"content":[{"type":"tool_use","id":format!("read-{i}"),"name":"Read","input":{"path":"file.rs"}}]}}));
        burst.push(serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":format!("read-{i}"),"content":"Fixture result"}]}}));
    }
    store.record_events("tool-heavy", "agent_event", burst, 1_792_900_100_000)?;

    let mut lines = format!(
        "{}\n",
        serde_json::json!({"type":"user","uuid":"fixture-request","sessionId":"fixture-parent","timestamp":"2026-10-24T23:50:00Z","message":{"content":"Check delivery acknowledgments."}})
    );
    for i in 0..80 {
        let text = match i % 3 {
            0 => format!("## Check {i}\n\nThe request is recorded before delivery. I am tracing the acknowledgment path.\n\n- Preserve the request ID.\n- Wait for a lifecycle acknowledgment."),
            1 => "```rust\nif acknowledgment.started() {\n    mark_delivered(request_id);\n}\n```".into(),
            _ => "A successful write only proves that bytes reached the pipe. The child must acknowledge consumption.".into(),
        };
        lines.push_str(&format!("{}\n", serde_json::json!({"type":"assistant","uuid":format!("fixture-{i}"),"timestamp":"2026-10-24T23:51:00Z","message":{"id":format!("fixture-{i}"),"content":[{"type":"text","text":text}]}})));
    }
    fs::write(children.join("agent-fixture-child.jsonl"), lines)?;
    Ok(())
}

/// All meter values are synthetic; no provider or subscription API is contacted.
fn seed_usage(store: &Store) -> Result<(), Box<dyn std::error::Error>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    store.create_session("usage-claude", "/tmp/boblm-usage", now)?;
    store.set_name("usage-claude", "Claude meter review")?;
    let project = store
        .reader()?
        .sessions()?
        .into_iter()
        .find(|s| s.id == "usage-claude")
        .and_then(|s| s.project_id)
        .ok_or("fixture project missing")?;
    for (id, agent, model) in [
        ("usage-codex", "codex", "gpt-6.1-sol"),
        ("usage-pi", "pi", "deepseek-ai/DeepSeek-V4.1-Flash-1017"),
    ] {
        store.create_session_in(boblm_store::NewSession {
            id,
            project_id: &project,
            name: if agent == "codex" {
                "Codex meter review"
            } else {
                "Pi context review"
            },
            agent,
            model,
            effort: None,
            created_at: now,
        })?;
    }
    store.set_session_account("usage-claude", Some("fixture-claude-account"))?;
    store.set_session_account("usage-codex", Some("fixture-codex-account"))?;
    for id in ["usage-claude", "usage-codex", "usage-pi"] {
        let mut usage = serde_json::json!({"type":"usage","context":{"usedTokens":84000,"windowTokens":200000}});
        if id != "usage-pi" {
            usage["five_hour"] = serde_json::json!({"usedPercent":80,"resetsAt":now/1000+3600});
            usage["weekly"] = serde_json::json!({"usedPercent":if id == "usage-codex" {100} else {37},"resetsAt":now/1000+86400});
        }
        if id == "usage-codex" {
            usage["credits"] =
                serde_json::json!({"hasCredits":true,"unlimited":false,"balance":40000});
        }
        store.record_event(id, "agent_event", usage, now)?;
        store.record_event(id, "agent_event", serde_json::json!({"type":"assistant","message":{"id":"meter-review","content":[{"type":"text","text":"Context shows 42% used. Allowances show remaining capacity. Codex has an exhausted weekly allowance and 40,000 credits. These are synthetic values for checking the status bar layout."}]}}), now)?;
    }
    Ok(())
}
