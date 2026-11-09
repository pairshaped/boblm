//! Read the provider's own account ID from Claude Code's local config. The file is
//! undocumented, so a missing field or an unreadable file means no account, never a guess.
use std::path::{Path, PathBuf};

/// The account UUID in the config directory's `.claude.json`, or `~/.claude.json`.
pub fn account_id() -> Option<String> {
    let path = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(directory) => PathBuf::from(directory).join(".claude.json"),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".claude.json"),
    };
    account_id_in(&path)
}

pub fn account_id_in(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["oauthAccount"]["accountUuid"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_the_account_uuid_and_tolerates_missing_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"oauthAccount":{"accountUuid":"account-1"},"token":"must not be read"}"#,
        )
        .unwrap();
        assert_eq!(account_id_in(&path).as_deref(), Some("account-1"));
        std::fs::write(&path, r#"{"oauthAccount":{}}"#).unwrap();
        assert!(account_id_in(&path).is_none());
        std::fs::write(&path, "not json").unwrap();
        assert!(account_id_in(&path).is_none());
        assert!(account_id_in(&directory.path().join("missing.json")).is_none());
    }
}
