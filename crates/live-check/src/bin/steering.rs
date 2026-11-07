//! Bounded live verification in a caller-provided disposable work directory.
use boblm_claude_driver::ClaudeSession;
use boblm_protocol::{Event, LifecycleState, Priority};
use std::{io, path::Path, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("Provide a fresh disposable directory.")?;
    let mode = std::env::args().nth(2).unwrap_or_else(|| "bash".into());
    let dir = Path::new(&dir);
    std::fs::create_dir(dir)?;
    let mut session = ClaudeSession::spawn(dir).await?;
    let result =
        tokio::time::timeout(Duration::from_secs(120), check(&mut session, dir, &mode)).await;
    session.stop().await?;
    result??;
    Ok(())
}

async fn check(session: &mut ClaudeSession, dir: &Path, mode: &str) -> io::Result<()> {
    // The marker precedes a long foreground command, so steering happens while
    // real work is running. Completion is independently observable after steering.
    let command = "python3 -c \"from pathlib import Path; import time; Path('started').write_text('yes'); time.sleep(25); Path('finished').write_text('yes')\"";
    let prompt = if mode == "subagent" {
        format!(
            "For this controlled test, launch one subagent in the foreground. Tell it to run exactly `{command}` in the foreground in the current directory, then return DONE. Wait for it. Do nothing else."
        )
    } else {
        format!(
            "For this controlled test, run exactly `{command}` with Bash in the foreground, using timeout 60000 and no background flag. Wait for it. Do nothing else."
        )
    };
    session.send(&prompt).await?;
    let steering = uuid::Uuid::new_v4().to_string();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut sent = false;
    let mut consumed = false;
    let mut handled_before_completion = false;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if !sent && dir.join("started").exists() {
                    session.send_message_with_origin(&steering,
                        "Change of plan: leave the existing command or subagent running. Immediately run `printf yes > steered` while it continues. Do not kill or restart it. Then wait for its completion notification and report DONE.",
                        Some(Priority::Now), true).await?;
                    sent = true;
                    println!("sent human-origin now during {mode}");
                }
                if dir.join("steered").exists() && !dir.join("finished").exists() {
                    handled_before_completion = true;
                }
                if consumed && handled_before_completion && dir.join("finished").exists() {
                    println!("PASS: {mode} survived, steering consumed and handled before completion");
                    return Ok(());
                }
                if dir.join("finished").exists() && !handled_before_completion {
                    return Err(io::Error::other("Work finished before the steering marker appeared. Inspect the live transcript."));
                }
            }
            event = session.events.recv() => {
                match event {
                    Some(Event::CommandLifecycle { command_uuid, state }) if command_uuid == steering => {
                        println!("steering lifecycle {state:?}");
                        consumed |= matches!(state, LifecycleState::Started | LifecycleState::Completed);
                    }
                    Some(Event::Result { subtype, .. }) => println!("result {subtype:?}"),
                    Some(_) => {}
                    None => return Err(io::Error::other("Claude exited before verification completed.")),
                }
            }
        }
    }
}
