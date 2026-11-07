//! Pi startup is the credential boundary: extract one entry and inject only into the child.
use boblm_pi_driver::{McpEndpoint, PiOptions, PiSession};
use std::{io, path::Path};
use tokio::process::Command;

/// Also used by explicit live checks. `command` selects the executable, never a credential source.
pub async fn spawn(
    command: Command,
    cwd: &Path,
    state_dir: &Path,
    resume: Option<&str>,
    model: &str,
    effort: Option<&str>,
    mcp: Option<McpEndpoint<'_>>,
) -> io::Result<PiSession> {
    let key: Vec<String> = boblm_tuning::PI_SOPS_KEY
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let credential = crate::secrets::read_secret(boblm_tuning::PI_SOPS_FILE, &key, cwd)
        .await
        .map_err(io::Error::other)?;
    PiSession::spawn_command(
        command,
        PiOptions {
            cwd,
            state_dir,
            resume,
            model,
            effort,
            api_key: &credential,
            mcp,
        },
    )
    .await
}
