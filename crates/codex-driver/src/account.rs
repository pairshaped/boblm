//! Read the provider's own account ID from Codex's local auth file. The file is
//! undocumented, so a missing field or an unreadable file means no account, never a guess.
use std::{
    io,
    path::{Path, PathBuf},
};

/// The account ID in the inherited Codex home.
pub fn account_id() -> Option<String> {
    account_id_in_home(&home_dir()?)
}

/// The account ID in the home mapped to a boblm account key.
pub fn account_id_for_key(key: &str) -> Option<String> {
    if key == boblm_tuning::CODEX_ACCOUNT_KEY_API {
        return Some(boblm_tuning::CODEX_API_ACCOUNT_ID.to_owned());
    }
    account_id_in_home(&home_for_key(key)?)
}

/// `$CODEX_HOME`, or `~/.codex` when it is unset.
pub fn home_dir() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(directory) => Some(PathBuf::from(directory)),
        None => Some(PathBuf::from(std::env::var_os("HOME")?).join(".codex")),
    }
}

/// The home mapped to an account key in the tuning table.
pub fn home_for_key(key: &str) -> Option<PathBuf> {
    let (_, home) = boblm_tuning::CODEX_ACCOUNT_HOMES
        .iter()
        .find(|(candidate, _)| *candidate == key)?;
    Some(expand_home(home))
}

pub fn account_id_in_home(directory: &Path) -> Option<String> {
    account_id_in(&directory.join("auth.json"))
}

/// An exclusive advisory lock on a Codex home. Dropping it releases the lock.
pub struct HomeLock {
    _lock: nix::fcntl::Flock<std::fs::File>,
}

/// Serialize home preparation and first Codex startup for a boblm-owned home.
pub fn lock_home(directory: &Path) -> io::Result<HomeLock> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let path = directory.join(".boblm-home.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|error| {
            io::Error::other(format!(
                "Could not open the Codex home lock {}: {error}. Check that the directory is writable, then reopen the chat.",
                path.display()
            ))
        })?;
    let lock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive).map_err(
        |(_file, error)| {
            io::Error::other(format!(
                "Could not lock the Codex home {}: {error}. Check that the directory is writable, then reopen the chat.",
                directory.display()
            ))
        },
    )?;
    Ok(HomeLock { _lock: lock })
}

/// Prepare the OpenAI API home: a private directory whose `config.toml` belongs to
/// the owner, like `~/.codex/config.toml`. BobLM never writes it; the provider comes
/// from [`api_provider_overrides`].
pub fn prepare_api_home(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
}

/// `-c` overrides that select the OpenAI API provider. The key itself is never
/// written; Codex reads the OPENAI_API_KEY environment variable named by env_key.
/// Overrides set only these keys, so servers and trust in `config.toml` survive.
pub fn api_provider_overrides() -> Vec<String> {
    let id = boblm_tuning::CODEX_API_PROVIDER_ID;
    vec![
        format!("model_provider=\"{id}\""),
        format!("model_providers.{id}.name=\"OpenAI API\""),
        format!(
            "model_providers.{id}.base_url=\"{}\"",
            boblm_tuning::CODEX_API_BASE_URL
        ),
        format!("model_providers.{id}.env_key=\"OPENAI_API_KEY\""),
        format!("model_providers.{id}.wire_api=\"responses\""),
    ]
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn account_id_in(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["tokens"]["account_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn reads_only_the_account_id_and_tolerates_missing_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        std::fs::write(
            &path,
            r#"{"tokens":{"account_id":"account-2","access_token":"must not be read"}}"#,
        )
        .unwrap();
        assert_eq!(account_id_in(&path).as_deref(), Some("account-2"));
        assert_eq!(
            account_id_in_home(directory.path()).as_deref(),
            Some("account-2")
        );
        std::fs::write(&path, r#"{"tokens":{}}"#).unwrap();
        assert!(account_id_in(&path).is_none());
        std::fs::write(&path, "not json").unwrap();
        assert!(account_id_in(&path).is_none());
        assert!(account_id_in(&directory.path().join("missing.json")).is_none());
    }

    #[test]
    fn account_keys_map_to_their_homes() {
        assert!(home_for_key(boblm_tuning::CODEX_ACCOUNT_KEY_PLAN).is_some());
        assert!(home_for_key(boblm_tuning::CODEX_ACCOUNT_KEY_API).is_some());
        assert!(home_for_key("unknown").is_none());
        assert_eq!(
            account_id_for_key(boblm_tuning::CODEX_ACCOUNT_KEY_API).as_deref(),
            Some(boblm_tuning::CODEX_API_ACCOUNT_ID)
        );
    }

    #[test]
    fn the_api_home_keeps_the_owners_config_and_selects_the_provider_by_override() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("codex-api");
        prepare_api_home(&home).unwrap();
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let owned = "[mcp_servers.refero]\nurl = \"https://refero.example/mcp\"\n";
        std::fs::write(home.join("config.toml"), owned).unwrap();
        prepare_api_home(&home).unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            owned
        );
        let overrides = api_provider_overrides().join("\n");
        assert!(overrides.contains(&format!(
            "model_provider=\"{}\"",
            boblm_tuning::CODEX_API_PROVIDER_ID
        )));
        assert!(overrides.contains("env_key=\"OPENAI_API_KEY\""));
        assert!(!overrides.contains("sk-"));
        assert!(!home.join("auth.json").exists());
    }
}
