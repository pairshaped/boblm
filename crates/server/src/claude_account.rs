//! Claude account credentials and child environment isolation (ADR 0025).
use std::{io, path::Path};
use tokio::process::Command;

pub async fn api_key() -> Result<String, String> {
    let key = boblm_tuning::CLAUDE_API_SOPS_KEY_A
        .iter()
        .map(|key| (*key).to_owned())
        .collect::<Vec<_>>();
    crate::secrets::read_secret_labeled(
        boblm_tuning::CLAUDE_API_SOPS_FILE,
        &key,
        Path::new("/"),
        "the Claude API key for account A",
        "CLAUDE_API_SOPS_KEY_A in crates/tuning",
    )
    .await
}

/// Only API children receive a key. Subscription children cannot inherit API authentication.
pub fn configure(
    command: &mut Command,
    data_dir: &Path,
    account_key: Option<&str>,
    api_key: Option<String>,
) -> io::Result<()> {
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        command.env_remove(name);
    }
    let Some(key) = account_key else {
        return Ok(());
    };
    let home = boblm_tuning::CLAUDE_ACCOUNT_HOMES.iter().find(|(candidate, _)| *candidate == key)
        .map(|(_, home)| data_dir.join(home)).ok_or_else(|| io::Error::other(format!(
            "The Claude account key {key:?} has no configured home. Check CLAUDE_ACCOUNT_HOMES in crates/tuning."
        )))?;
    std::fs::create_dir_all(&home).map_err(|error| io::Error::other(format!(
        "Could not create the Claude API config folder {}: {error}. Check the data directory permissions and try again.", home.display()
    )))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).map_err(|error| io::Error::other(format!(
            "Could not make the Claude API config folder {} private: {error}. Check its ownership and try again.", home.display()
        )))?;
    }
    let api_key = api_key.ok_or_else(|| io::Error::other(
        "The Claude API account has no API key. Save ANTHROPIC_API_KEY_A in the configured SOPS file and try again."
    ))?;
    // These inherited overrides could redirect the key or select subscription/cloud credentials.
    for name in [
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        command.env_remove(name);
    }
    command
        .env("CLAUDE_CONFIG_DIR", home)
        .env("ANTHROPIC_API_KEY", api_key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claude_api_key_and_config_home_reach_only_the_api_child() {
        let dir = tempfile::tempdir().unwrap();
        for api in [true, false] {
            let mut command = Command::new("python3");
            command.env("ANTHROPIC_API_KEY", "inherited-key");
            command.env("ANTHROPIC_AUTH_TOKEN", "inherited-token");
            command.env("CLAUDE_CODE_OAUTH_TOKEN", "inherited-oauth");
            command.env("CLAUDE_CONFIG_DIR", dir.path().join("subscription"));
            configure(
                &mut command,
                dir.path(),
                api.then_some(boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A),
                api.then(|| "fictional-api-key".into()),
            )
            .unwrap();
            command.arg("-c").arg(if api {
                "import os,sys; assert os.environ['ANTHROPIC_API_KEY']=='fictional-api-key'; assert os.environ['CLAUDE_CONFIG_DIR']==sys.argv[1]; assert 'ANTHROPIC_AUTH_TOKEN' not in os.environ; assert 'CLAUDE_CODE_OAUTH_TOKEN' not in os.environ"
            } else {
                "import os,sys; assert 'ANTHROPIC_API_KEY' not in os.environ; assert os.environ['CLAUDE_CONFIG_DIR']==sys.argv[1]; assert 'ANTHROPIC_AUTH_TOKEN' not in os.environ; assert 'CLAUDE_CODE_OAUTH_TOKEN' not in os.environ"
            }).arg(dir.path().join(if api { "claude-api-a" } else { "subscription" }));
            assert!(command.status().await.unwrap().success());
        }
        assert!(!dir.path().join("claude-api-a/.credentials.json").exists());
        assert!(configure(&mut Command::new("true"), dir.path(), Some("unknown"), None).is_err());
        assert!(
            configure(
                &mut Command::new("true"),
                dir.path(),
                Some(boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A),
                None
            )
            .is_err()
        );
    }
}
