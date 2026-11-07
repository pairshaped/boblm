use boblm_claude_driver::{ClaudeSession, SteerOutcome};
use boblm_protocol::Event;
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("expected a fresh workdir")?);
    std::fs::create_dir_all(&dir)?;
    if dir.join("log.txt").exists() {
        return Err("workdir already has log.txt".into());
    }
    let mut session = ClaudeSession::spawn(&dir).await?;
    let result = tokio::time::timeout(Duration::from_secs(150), check(&mut session, &dir)).await;
    session.stop().await?;
    result??;
    Ok(())
}

async fn check(session: &mut ClaudeSession, dir: &std::path::Path) -> io::Result<()> {
    let t0 = Instant::now();
    session.send(
        "Run the Bash command `python3 -c \"import time; time.sleep(45)\" && echo BASHDONE` in the foreground. \
         After it finishes, run `echo WRONG >> log.txt`. Do nothing else.",
    ).await?;
    let steer_at = tokio::time::sleep(Duration::from_secs(20));
    tokio::pin!(steer_at);
    let mut steered = false;
    let mut results = 0;
    loop {
        tokio::select! {
            event = session.events.recv() => {
                let Some(event) = event else { return Err(io::Error::other("Claude event stream closed")); };
                match event {
                    Event::CommandLifecycle { command_uuid, state } =>
                        println!("{:6.1}s lifecycle {} {:?}", t0.elapsed().as_secs_f32(), command_uuid.chars().take(8).collect::<String>(), state),
                    Event::Result { .. } => { results += 1; println!("{:6.1}s result #{results}", t0.elapsed().as_secs_f32()); }
                    _ => {}
                }
                if steered && results >= 2 { break; }
            }
            _ = &mut steer_at, if !steered => {
                steered = true;
                let outcome = session.steer("Change of plan: do NOT write WRONG. Run `echo STEERED >> log.txt` and stop.", Duration::from_secs(60)).await;
                println!("{:6.1}s steer outcome: {:?}", t0.elapsed().as_secs_f32(), outcome);
                if outcome != SteerOutcome::Started { return Err(io::Error::other("steer was not acknowledged")); }
            }
        }
    }
    let log = std::fs::read_to_string(dir.join("log.txt"))?;
    println!("log.txt: {log}");
    if !log.lines().any(|line| line == "STEERED") || log.contains("WRONG") {
        return Err(io::Error::other("expected STEERED and no WRONG"));
    }
    Ok(())
}
