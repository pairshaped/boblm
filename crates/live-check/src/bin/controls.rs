//! Explicit live check for the Model/effort/stop Bean.
use boblm_claude_driver::ClaudeSession;
use boblm_protocol::Event;
use std::{io, path::Path, time::Duration};
use tokio::{
    process::Command,
    time::{Instant, timeout},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::path::PathBuf::from(std::env::args().nth(1).ok_or("expected fresh workdir")?);
    std::fs::create_dir_all(&directory)?;
    if directory.join("ready.txt").exists() {
        return Err("expected fresh workdir".into());
    }
    let mut command = Command::new("claude");
    command.args([
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
    ]);
    let mut session = ClaudeSession::spawn_command(command, &directory, None).await?;
    let result = timeout(Duration::from_secs(180), check(&mut session, &directory)).await;
    session.stop().await?;
    result??;
    Ok(())
}
async fn turn(session: &mut ClaudeSession, prompt: &str) -> io::Result<String> {
    session.send(prompt).await?;
    let mut model = String::new();
    loop {
        match session
            .events
            .recv()
            .await
            .ok_or_else(|| io::Error::other("Claude exited"))?
        {
            Event::Assistant { message } => {
                if let Some(value) = message.get("model").and_then(serde_json::Value::as_str) {
                    model = value.into();
                }
            }
            Event::Result { is_error, .. } => {
                if is_error == Some(true) {
                    return Err(io::Error::other("Claude turn failed"));
                }
                return Ok(model);
            }
            _ => {}
        }
    }
}
async fn check(session: &mut ClaudeSession, directory: &Path) -> io::Result<()> {
    // No initialize handshake is required to change controls before the first turn.
    session.set_model(Some("sonnet")).await?;
    session.set_effort(Some("low")).await?;
    let first = turn(session, "Reply only FIRST_OK.").await?;
    let id = session
        .session_id()
        .ok_or_else(|| io::Error::other("missing init ID"))?;
    println!("first model: {first}; session: {id}");
    session.set_model(Some("haiku")).await?;
    session.set_effort(Some("high")).await?;
    let settings = session
        .control(
            serde_json::json!({"subtype":"get_settings"}),
            Duration::from_secs(5),
        )
        .await?;
    if settings["effective"]["effortLevel"] != "high" {
        return Err(io::Error::other("effort was not applied"));
    }
    println!("apply_flag_settings verified effortLevel=high");
    let second = turn(session, "Reply only SECOND_OK.").await?;
    if !first.contains("sonnet")
        || !second.contains("haiku")
        || session.session_id().as_deref() != Some(&id)
    {
        return Err(io::Error::other(
            "model switch or session continuity failed",
        ));
    }
    println!("second model: {second}; same session confirmed");
    session.send("Use Bash to run exactly `echo READY > ready.txt; sleep 30; echo WRONG >> stop.log` in the foreground. Do nothing else.").await?;
    let started = Instant::now();
    loop {
        tokio::select! {
            event = session.events.recv() => {
                if event.is_none() { return Err(io::Error::other("Claude exited before Bash")); }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if directory.join("ready.txt").exists() { break; }
                if started.elapsed() > Duration::from_secs(60) { return Err(io::Error::other("Bash did not start")); }
            }
        }
    }
    let interrupted = Instant::now();
    session.request_interrupt().await?;
    loop {
        match session
            .events
            .recv()
            .await
            .ok_or_else(|| io::Error::other("Claude exited during stop"))?
        {
            Event::Result { .. } => break,
            Event::ControlResponse { .. } => {
                println!("interrupt control reply observed; still waiting for result")
            }
            _ => {}
        }
    }
    if interrupted.elapsed() >= Duration::from_secs(25) {
        return Err(io::Error::other("interrupt did not stop Bash promptly"));
    }
    let log = std::fs::read_to_string(directory.join("stop.log")).unwrap_or_default();
    if log.contains("WRONG") {
        return Err(io::Error::other("Bash completed after stop"));
    }
    println!(
        "Bash interrupted in {:.2}s; no WRONG",
        interrupted.elapsed().as_secs_f32()
    );
    let third = turn(session, "Reply only AFTER_STOP_OK.").await?;
    if !third.contains("haiku") || session.session_id().as_deref() != Some(&id) {
        return Err(io::Error::other("process did not remain usable after stop"));
    }
    println!("follow-up completed on {third}; no restart");
    Ok(())
}
