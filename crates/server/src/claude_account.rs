//! Claude account credentials, routing and config isolation (ADR 0047).
use boblm_tuning::{claude_accounts::PlanAccount, *};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::process::Command;

/// Enabled Claude accounts for one server instance.
#[derive(Clone, Copy)]
pub struct EnabledAccounts {
    pub plan: &'static [&'static str],
    pub api: &'static [&'static str],
}

impl Default for EnabledAccounts {
    fn default() -> Self {
        Self {
            plan: CLAUDE_PLAN_ACCOUNTS,
            api: CLAUDE_API_ACCOUNTS,
        }
    }
}

pub fn is_api(key: &str) -> bool {
    matches!(key, CLAUDE_ACCOUNT_KEY_API_A | CLAUDE_ACCOUNT_KEY_API_B)
}

pub fn home(data_dir: &Path, key: &str) -> io::Result<PathBuf> {
    let (_, path) = CLAUDE_ACCOUNT_HOMES.iter().find(|(candidate, _)| *candidate == key)
        .ok_or_else(|| io::Error::other(format!("The Claude account key {key:?} has no configured home. Check CLAUDE_ACCOUNT_HOMES in crates/tuning.")))?;
    if path.is_empty() {
        return std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
            .ok_or_else(|| io::Error::other("Claude has no default config folder because HOME is missing. Set HOME and restart BobLM."));
    }
    Ok(data_dir.join(path))
}

pub fn account_id(data_dir: &Path, key: &str) -> Option<String> {
    if is_api(key) {
        return Some(key.into());
    }
    if key == CLAUDE_ACCOUNT_KEY_PLAN_A {
        return boblm_claude_driver::account::account_id();
    }
    boblm_claude_driver::account::account_id_in(&home(data_dir, key).ok()?.join(".claude.json"))
}

pub async fn api_key(account: &str) -> Result<String, String> {
    let (path, label, setting) = match account {
        CLAUDE_ACCOUNT_KEY_API_A => (
            CLAUDE_API_SOPS_KEY_A,
            "the Claude API key for account A",
            "CLAUDE_API_SOPS_KEY_A in crates/tuning",
        ),
        CLAUDE_ACCOUNT_KEY_API_B => (
            CLAUDE_API_SOPS_KEY_B,
            "the Claude API key for account B",
            "CLAUDE_API_SOPS_KEY_B in crates/tuning",
        ),
        _ => {
            return Err(
                "This Claude account is not an API account. Check the account key in tuning."
                    .into(),
            );
        }
    };
    let key = path.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
    crate::secrets::read_secret_labeled(CLAUDE_API_SOPS_FILE, &key, Path::new("/"), label, setting)
        .await
}

/// Attribute each sample to the UUID from that account's own config, never another chat.
pub fn plan_accounts(
    store: &boblm_store::Store,
    enabled: &[&'static str],
) -> Result<Vec<PlanAccount>, String> {
    let reader = store.reader().map_err(|error| error.to_string())?;
    let data_dir = store.path().parent().unwrap_or(Path::new("."));
    let marks = reader
        .claude_account_exhaustions()
        .map_err(|error| error.to_string())?;
    let mut accounts = Vec::new();
    for &key in enabled {
        let id = account_id(data_dir, key).or_else(|| {
            reader
                .sessions()
                .ok()?
                .into_iter()
                .find(|session| {
                    session.agent == "claude"
                        && session
                            .account_key
                            .as_deref()
                            .unwrap_or(CLAUDE_ACCOUNT_KEY_PLAN_A)
                            == key
                        && session.account_id.as_deref().is_some_and(|id| !is_api(id))
                })
                .and_then(|session| session.account_id)
        });
        let usage = id
            .map(|id| reader.account_usage("claude", &id))
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten();
        accounts.push(PlanAccount {
            key,
            used_percent: usage
                .as_ref()
                .and_then(|usage| usage.weekly.as_ref())
                .map(|reading| reading.used_percent),
            resets_at: usage
                .as_ref()
                .and_then(|usage| usage.weekly.as_ref())
                .and_then(|reading| reading.resets_at),
            exhausted_until: marks
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, until)| *until),
        });
    }
    Ok(accounts)
}

pub fn handoff_target(
    store: &boblm_store::Store,
    session: &boblm_store::Session,
    enabled: &[&'static str],
    now: i64,
) -> Result<Option<boblm_runner::ClaudeHandoff>, String> {
    if enabled.len() < 2 {
        return Ok(None);
    }
    let key = session
        .account_key
        .as_deref()
        .unwrap_or(CLAUDE_ACCOUNT_KEY_PLAN_A);
    if is_api(key) {
        return Ok(None);
    }
    let mut accounts = plan_accounts(store, enabled)?;
    let Some(source) = accounts.iter().find(|account| account.key == key) else {
        return Ok(None);
    };
    if source.available(now) {
        return Ok(None);
    }
    let source_reset = source
        .resets_at
        .filter(|reset| *reset > now)
        .unwrap_or_else(|| {
            now.saturating_add(i64::try_from(WEEKLY_USAGE_WINDOW_MINUTES * 60).unwrap_or(i64::MAX))
        });
    let visits = store
        .reader()
        .map_err(|error| error.to_string())?
        .claude_account_handoffs(&session.id)
        .map_err(|error| error.to_string())?;
    accounts.retain(|account| {
        account.key != key
            && !visits
                .iter()
                .any(|(key, until)| key == account.key && *until > now)
    });
    let Some(target) = claude_accounts::choose(&accounts, now) else {
        return Ok(None);
    };
    let data = store.path().parent().unwrap_or(Path::new("."));
    Ok(Some(boblm_runner::ClaudeHandoff {
        source: key.into(),
        target: target.key.into(),
        account_id: account_id(data, target.key),
        source_reset,
    }))
}

pub fn choose_plan(
    store: &boblm_store::Store,
    enabled: &[&'static str],
    now: i64,
) -> Result<&'static str, String> {
    if let [key] = enabled {
        return Ok(key);
    }
    let accounts = plan_accounts(store, enabled)?;
    claude_accounts::choose_for_owner(&accounts, now).map(|account| account.key)
        .ok_or_else(|| "No Claude plan accounts are enabled. Add an account to CLAUDE_PLAN_ACCOUNTS in crates/tuning and rebuild BobLM.".into())
}

pub fn choose_api_from(
    store: &boblm_store::Store,
    enabled: &[&'static str],
    now: i64,
) -> Result<Option<&'static str>, String> {
    let reader = store.reader().map_err(|error| error.to_string())?;
    let marks = reader
        .claude_account_exhaustions()
        .map_err(|error| error.to_string())?;
    let legacy = reader
        .role_exhaustions()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|mark| mark.step == StepId::ReviewerClaudeApi && mark.until > now);
    for &key in enabled {
        if key == CLAUDE_ACCOUNT_KEY_API_A && legacy {
            continue;
        }
        if marks
            .iter()
            .any(|(candidate, until)| candidate == key && *until > now)
        {
            continue;
        }
        let usage = reader
            .account_usage("claude", key)
            .map_err(|error| error.to_string())?;
        if usage
            .as_ref()
            .and_then(|usage| usage.credits.as_ref())
            .is_some_and(|reading| reading.balance.is_some_and(|balance| balance <= 0.0))
        {
            continue;
        }
        return Ok(Some(key));
    }
    Ok(None)
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
    let key = account_key.unwrap_or(CLAUDE_ACCOUNT_KEY_PLAN_A);
    // Plan A preserves the existing default-config lookup, including ~/.claude.json.
    if key == CLAUDE_ACCOUNT_KEY_PLAN_A {
        return Ok(());
    }
    let home = home(data_dir, key)?;
    private_dir(&home)?;
    command.env("CLAUDE_CONFIG_DIR", home);
    if !is_api(key) {
        return Ok(());
    }
    let api_key = api_key.ok_or_else(|| io::Error::other(format!("The Claude API account {key} has no API key. Save its key in the configured SOPS file and try again.")))?;
    for name in [
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        command.env_remove(name);
    }
    command.env("ANTHROPIC_API_KEY", api_key);
    Ok(())
}

fn private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// Copy just this conversation's transcript and its companion directory (subagents, etc.).
/// Preserve project folder names, including conversations that changed working directory.
pub fn copy_transcript(source: &Path, target: &Path, conversation: &str) -> io::Result<()> {
    let id = uuid::Uuid::parse_str(conversation).map_err(|error| io::Error::other(format!("The Claude conversation ID cannot be copied: {error}. Restore its valid session ID before resuming.")))?.to_string();
    if id != conversation {
        return Err(io::Error::other(
            "The Claude conversation ID is not canonical. Restore its original session ID before resuming.",
        ));
    }
    if source == target {
        return Err(io::Error::other(
            "Both Claude accounts use the same config folder. Give each account its own home before handing off.",
        ));
    }
    let mut copied = false;
    for entry in std::fs::read_dir(source.join("projects"))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let transcript = entry.path().join(format!("{id}.jsonl"));
        if !transcript.try_exists()? {
            continue;
        }
        let destination = target.join("projects").join(entry.file_name());
        private_dir(&destination)?;
        copy_file(&transcript, &destination.join(format!("{id}.jsonl")))?;
        let companion = entry.path().join(&id);
        if companion.try_exists()? {
            copy_directory(&companion, &destination.join(&id))?;
        }
        copied = true;
    }
    if !copied {
        return Err(io::Error::other(format!(
            "Claude's transcript for {id} was not found in {}. Restore the transcript before retrying the handoff.",
            source.display()
        )));
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if !source.symlink_metadata()?.is_file() {
        return Err(io::Error::other(
            "A Claude transcript contains a link or special file. Restore a regular transcript file before retrying the handoff.",
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(target.parent().ok_or_else(|| io::Error::other("The Claude transcript has no destination folder. Check the account home configuration."))?)?;
    std::io::copy(&mut std::fs::File::open(source)?, &mut temporary)?;
    temporary
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    Ok(())
}

fn copy_directory(source: &Path, target: &Path) -> io::Result<()> {
    if !source.symlink_metadata()?.is_dir() {
        return Err(io::Error::other(
            "A Claude companion folder is a symbolic link. Restore a regular folder before retrying the handoff.",
        ));
    }
    private_dir(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target.join(entry.file_name()))?;
        } else {
            copy_file(&entry.path(), &target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claude_api_key_and_config_home_reach_only_the_api_child() {
        let dir = tempfile::tempdir().unwrap();
        for (account, expected) in [
            (Some(CLAUDE_ACCOUNT_KEY_API_A), "claude-api-a"),
            (Some(CLAUDE_ACCOUNT_KEY_API_B), "claude-api-b"),
            (None, "subscription"),
        ] {
            let api = account.is_some();
            let mut command = Command::new("python3");
            command.env("ANTHROPIC_API_KEY", "inherited-key");
            command.env("ANTHROPIC_AUTH_TOKEN", "inherited-token");
            command.env("CLAUDE_CODE_OAUTH_TOKEN", "inherited-oauth");
            command.env("CLAUDE_CONFIG_DIR", dir.path().join("subscription"));
            configure(
                &mut command,
                dir.path(),
                account,
                api.then(|| "fictional-api-key".into()),
            )
            .unwrap();
            command.arg("-c").arg(if api {
                "import os,sys; assert os.environ['ANTHROPIC_API_KEY']=='fictional-api-key'; assert os.environ['CLAUDE_CONFIG_DIR']==sys.argv[1]; assert 'ANTHROPIC_AUTH_TOKEN' not in os.environ; assert 'CLAUDE_CODE_OAUTH_TOKEN' not in os.environ"
            } else {
                "import os,sys; assert 'ANTHROPIC_API_KEY' not in os.environ; assert os.environ['CLAUDE_CONFIG_DIR']==sys.argv[1]; assert 'ANTHROPIC_AUTH_TOKEN' not in os.environ; assert 'CLAUDE_CODE_OAUTH_TOKEN' not in os.environ"
            }).arg(dir.path().join(expected));
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

#[cfg(test)]
mod routing_tests {
    use super::*;
    use boblm_store::Store;

    fn setup() -> (tempfile::TempDir, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("accounts.sqlite")).unwrap();
        store.create_session("a", "/project", 1).unwrap();
        store.create_session("b", "/project", 1).unwrap();
        let a =
            account_id(dir.path(), CLAUDE_ACCOUNT_KEY_PLAN_A).unwrap_or_else(|| "plan-a-id".into());
        private_dir(&dir.path().join("claude-plan-b")).unwrap();
        std::fs::write(
            dir.path().join("claude-plan-b/.claude.json"),
            r#"{"oauthAccount":{"accountUuid":"plan-b-id"}}"#,
        )
        .unwrap();
        store.set_session_account("a", Some(&a)).unwrap();
        store.set_session_account("b", Some("plan-b-id")).unwrap();
        store
            .set_claude_account_key("b", CLAUDE_ACCOUNT_KEY_PLAN_B)
            .unwrap();
        (dir, store, a)
    }

    fn usage(store: &Store, session: &str, used: f64, reset: i64, observed: i64) {
        store
            .record_event(
                session,
                "agent_event",
                serde_json::json!({"type":"usage","weekly":{"usedPercent":used,"resetsAt":reset}}),
                observed,
            )
            .unwrap();
    }

    #[test]
    fn claude_accounts_read_their_own_usage_and_never_ping_pong_before_reset() {
        let (_dir, store, _) = setup();
        let enabled = &[CLAUDE_ACCOUNT_KEY_PLAN_A, CLAUDE_ACCOUNT_KEY_PLAN_B];
        usage(&store, "a", 98.0, 200, 1000);
        usage(&store, "b", 2.0, 300, 1000);
        let mut session = store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.id == "a")
            .unwrap();
        let accounts = plan_accounts(&store, enabled).unwrap();
        assert_eq!(accounts[0].used_percent, Some(98.0));
        assert_eq!(accounts[1].used_percent, Some(2.0));
        assert_eq!(
            claude_accounts::choose(&accounts, 100).unwrap().key,
            CLAUDE_ACCOUNT_KEY_PLAN_B
        );
        assert!(
            handoff_target(&store, &session, &[CLAUDE_ACCOUNT_KEY_PLAN_A], 100)
                .unwrap()
                .is_none()
        );
        let change = handoff_target(&store, &session, enabled, 100)
            .unwrap()
            .unwrap();
        assert_eq!(change.target, CLAUDE_ACCOUNT_KEY_PLAN_B);
        assert_eq!(change.account_id.as_deref(), Some("plan-b-id"));
        store.handoff_claude_account("a", &change, 1000).unwrap();
        session.account_key = Some(CLAUDE_ACCOUNT_KEY_PLAN_B.into());
        // A bonus lowers A's usage within its existing reset window. Returning still
        // pays a full context cache miss, so the durable exclusion wins.
        store.set_session_account("b", Some("plan-b-id")).unwrap();
        usage(&store, "b", 99.0, 300, 2000);
        store.create_session("a-reading", "/project", 1).unwrap();
        let id = account_id(_dir.path(), CLAUDE_ACCOUNT_KEY_PLAN_A)
            .unwrap_or_else(|| "plan-a-id".into());
        store.set_session_account("a-reading", Some(&id)).unwrap();
        usage(&store, "a-reading", 10.0, 200, 2000);
        assert!(
            handoff_target(&store, &session, enabled, 150)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            handoff_target(&store, &session, enabled, 201)
                .unwrap()
                .unwrap()
                .target,
            CLAUDE_ACCOUNT_KEY_PLAN_A
        );
    }

    #[test]
    fn claude_api_accounts_use_configuration_order_and_individual_exhaustion() {
        let (_dir, store, _) = setup();
        let enabled = &[CLAUDE_ACCOUNT_KEY_API_A, CLAUDE_ACCOUNT_KEY_API_B];
        assert_eq!(
            choose_api_from(&store, enabled, 100).unwrap(),
            Some(CLAUDE_ACCOUNT_KEY_API_A)
        );
        store
            .exhaust_claude_account("a", CLAUDE_ACCOUNT_KEY_API_A, 200, false)
            .unwrap();
        assert_eq!(
            choose_api_from(&store, enabled, 100).unwrap(),
            Some(CLAUDE_ACCOUNT_KEY_API_B)
        );
        store
            .exhaust_claude_account("a", CLAUDE_ACCOUNT_KEY_API_B, 300, false)
            .unwrap();
        assert!(choose_api_from(&store, enabled, 100).unwrap().is_none());
        assert_eq!(
            choose_api_from(&store, enabled, 200).unwrap(),
            Some(CLAUDE_ACCOUNT_KEY_API_A)
        );
    }

    #[test]
    fn claude_transcript_copy_keeps_project_paths_and_companions_only() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        let id = uuid::Uuid::new_v4().to_string();
        let project = source.join("projects/-work-dir");
        std::fs::create_dir_all(project.join(&id).join("subagents")).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), "transcript").unwrap();
        std::fs::write(
            project.join(&id).join("subagents/agent-a.jsonl"),
            "subagent",
        )
        .unwrap();
        std::fs::write(project.join("unrelated.jsonl"), "other chat").unwrap();
        std::fs::write(source.join(".credentials.json"), "never copy").unwrap();
        copy_transcript(&source, &target, &id).unwrap();
        assert_eq!(
            std::fs::read_to_string(
                target
                    .join("projects/-work-dir")
                    .join(format!("{id}.jsonl"))
            )
            .unwrap(),
            "transcript"
        );
        assert_eq!(
            std::fs::read_to_string(
                target
                    .join("projects/-work-dir")
                    .join(&id)
                    .join("subagents/agent-a.jsonl")
            )
            .unwrap(),
            "subagent"
        );
        assert!(!target.join(".credentials.json").exists());
        assert!(!target.join("projects/-work-dir/unrelated.jsonl").exists());
        assert!(copy_transcript(&source, &target, "../../secret").is_err());
        assert!(copy_transcript(&source, &target, &uuid::Uuid::new_v4().to_string()).is_err());
        std::fs::write(project.join(format!("{id}.jsonl")), "newer transcript").unwrap();
        copy_transcript(&source, &target, &id).unwrap();
        assert_eq!(
            std::fs::read_to_string(
                target
                    .join("projects/-work-dir")
                    .join(format!("{id}.jsonl"))
            )
            .unwrap(),
            "newer transcript"
        );
    }

    #[tokio::test]
    async fn claude_plan_b_config_preserves_session_id_and_strips_api_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("python3");
        command
            .env(SESSION_ID_ENV, "session-1")
            .env("ANTHROPIC_API_KEY", "inherited")
            .env("ANTHROPIC_AUTH_TOKEN", "inherited");
        configure(
            &mut command,
            dir.path(),
            Some(CLAUDE_ACCOUNT_KEY_PLAN_B),
            None,
        )
        .unwrap();
        command.arg("-c").arg("import os,sys; assert os.environ['CLAUDE_CONFIG_DIR']==sys.argv[1]; assert os.environ['BOBLM_SESSION_ID']=='session-1'; assert 'ANTHROPIC_API_KEY' not in os.environ; assert 'ANTHROPIC_AUTH_TOKEN' not in os.environ").arg(dir.path().join("claude-plan-b"));
        assert!(command.status().await.unwrap().success());
    }
}
