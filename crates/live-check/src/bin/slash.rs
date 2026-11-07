//! Explicit live probe: does Claude emit a `result` for a slash command?
//! Prints every lifecycle and result event around a normal turn, `/cost` and `/compact`.
use boblm_claude_driver::ClaudeSession;
use boblm_protocol::Event;
use std::{io, time::Duration};
use tokio::{
    process::Command,
    time::{Instant, timeout},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::path::PathBuf::from(std::env::args().nth(1).ok_or("expected fresh workdir")?);
    std::fs::create_dir_all(&directory)?;
    let mut command = Command::new("claude");
    command.args([
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
    ]);
    let mut session = ClaudeSession::spawn_command(command, &directory, None).await?;
    let result = timeout(Duration::from_secs(240), probe(&mut session)).await;
    session.stop().await?;
    result??;
    Ok(())
}

/// Print events until nothing arrives for `quiet`, returning how many results were seen.
async fn observe(
    session: &mut ClaudeSession,
    t0: Instant,
    label: &str,
    quiet: Duration,
) -> io::Result<usize> {
    let mut results = 0;
    loop {
        match timeout(quiet, session.events.recv()).await {
            Err(_) => return Ok(results),
            Ok(None) => return Err(io::Error::other("Claude exited")),
            Ok(Some(event)) => match event {
                Event::CommandLifecycle {
                    command_uuid,
                    state,
                } => println!(
                    "{:6.1}s [{label}] lifecycle {} {state:?}",
                    t0.elapsed().as_secs_f32(),
                    command_uuid.chars().take(8).collect::<String>()
                ),
                Event::Result { subtype, is_error } => {
                    results += 1;
                    println!(
                        "{:6.1}s [{label}] result subtype={subtype:?} is_error={is_error:?}",
                        t0.elapsed().as_secs_f32()
                    );
                }
                Event::System { subtype, .. } => println!(
                    "{:6.1}s [{label}] system {subtype:?}",
                    t0.elapsed().as_secs_f32()
                ),
                _ => {}
            },
        }
    }
}

async fn probe(session: &mut ClaudeSession) -> io::Result<()> {
    let t0 = Instant::now();
    session.set_model(Some("haiku")).await?;
    session
        .send_message(
            "00000000-0000-4000-8000-0000000000a1",
            "Reply with the single word OK.",
            None,
        )
        .await?;
    let normal = observe(session, t0, "turn", Duration::from_secs(20)).await?;
    for (n, command) in [(2, "/cost"), (3, "/compact")] {
        session
            .send_message(
                &format!("00000000-0000-4000-8000-0000000000a{n}"),
                command,
                None,
            )
            .await?;
        let results = observe(session, t0, command, Duration::from_secs(25)).await?;
        println!("== {command}: {results} result event(s)");
    }
    println!("== normal turn: {normal} result event(s)");
    Ok(())
}
