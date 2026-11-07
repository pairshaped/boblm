//! Explicit live probe: how `/compact` and `/clear` behave over stream-json.
//! Prints lifecycle, system events, session IDs and assistant text for each step,
//! then resumes the final session ID in a new process to see which conversation it holds.
use boblm_claude_driver::ClaudeSession;
use boblm_protocol::Event;
use std::{io, path::Path, time::Duration};
use tokio::{
    process::Command,
    time::{Instant, timeout},
};

fn claude() -> Command {
    let mut command = Command::new("claude");
    command.args([
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
    ]);
    command
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::path::PathBuf::from(std::env::args().nth(1).ok_or("expected fresh workdir")?);
    std::fs::create_dir_all(&directory)?;
    let result = timeout(Duration::from_secs(300), probe(&directory)).await;
    result??;
    Ok(())
}

/// Send one message and print events until the turn's result and a quiet period.
async fn step(session: &mut ClaudeSession, t0: Instant, label: &str, text: &str) -> io::Result<()> {
    println!("--- {label}: {text}");
    session
        .send_message(&uuid::Uuid::new_v4().to_string(), text, None)
        .await?;
    let mut saw_result = false;
    loop {
        let quiet = if saw_result {
            Duration::from_secs(3)
        } else {
            Duration::from_secs(90)
        };
        match timeout(quiet, session.events.recv()).await {
            Err(_) if saw_result => return Ok(()),
            Err(_) => return Err(io::Error::other("no result")),
            Ok(None) => return Err(io::Error::other("Claude exited")),
            Ok(Some(event)) => {
                let at = t0.elapsed().as_secs_f32();
                match event {
                    Event::CommandLifecycle { state, .. } => {
                        println!("{at:6.1}s lifecycle {state:?}")
                    }
                    Event::System {
                        subtype,
                        session_id,
                        ..
                    } => {
                        println!("{at:6.1}s system {subtype:?} session_id={session_id:?}")
                    }
                    Event::Result { subtype, is_error } => {
                        saw_result = true;
                        println!("{at:6.1}s result {subtype:?} is_error={is_error:?}")
                    }
                    Event::Assistant { message } => {
                        for block in message["content"].as_array().into_iter().flatten() {
                            if let Some(text) = block["text"].as_str() {
                                println!("{at:6.1}s assistant: {text}");
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn probe(directory: &Path) -> io::Result<()> {
    let t0 = Instant::now();
    let mut session = ClaudeSession::spawn_command(claude(), directory, None).await?;
    session.set_model(Some("haiku")).await?;
    step(
        &mut session,
        t0,
        "remember",
        "Remember the secret word BANANA-42. Reply with just OK.",
    )
    .await?;
    println!("session id now: {:?}", session.session_id());
    step(&mut session, t0, "compact", "/compact").await?;
    println!("session id now: {:?}", session.session_id());
    step(
        &mut session,
        t0,
        "recall after compact",
        "What is the secret word? Reply with just the word, or UNKNOWN if you do not know it.",
    )
    .await?;
    step(&mut session, t0, "clear", "/clear").await?;
    println!("session id now: {:?}", session.session_id());
    step(
        &mut session,
        t0,
        "recall after clear",
        "What is the secret word? Reply with just the word, or UNKNOWN if you do not know it.",
    )
    .await?;
    let last = session.session_id();
    session.stop().await?;
    println!("=== resume {last:?} in a new process");
    let mut resumed = ClaudeSession::spawn_command(claude(), directory, last.as_deref()).await?;
    resumed.set_model(Some("haiku")).await?;
    step(
        &mut resumed,
        t0,
        "recall after resume",
        "What is the secret word? Reply with just the word, or UNKNOWN if you do not know it.",
    )
    .await?;
    println!("session id after resume: {:?}", resumed.session_id());
    resumed.stop().await?;
    Ok(())
}
