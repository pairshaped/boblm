//! Role routing inputs and spawn helpers for role-based subagents (ADR 0043).
pub use crate::claude_account::EnabledAccounts;
use boblm_protocol::client::SessionInfo;
use boblm_store::{Session, Store};
use boblm_tuning::{
    AccountKind, AccountReading, ChosenRoute, Difficulty, Role, RouteOutcome, StepId,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// How the receiver of a boblm-delivered message should answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyTrailer {
    Peer,
    Subagent,
    SubagentReport,
}

impl ReplyTrailer {
    pub fn text(self) -> &'static str {
        match self {
            Self::Peer => {
                "Reply to this message with reply_to_sender if a reply is needed. Text in your own chat is not seen by the sender."
            }
            Self::Subagent => {
                "Your final reply reaches your parent automatically when you finish, so do not send results back with message_parent. Use message_parent only for questions you need answered mid-task, and use ask for owner decisions."
            }
            Self::SubagentReport => {
                "No reply is needed to this report. Reuse the subagent with ping_peer if you need more work, or archive it when done."
            }
        }
    }
}

/// Append the stable reply instructions to a boblm-delivered message.
pub fn with_reply_trailer(content: &str, trailer: ReplyTrailer) -> String {
    format!("{}\n\n{}", content.trim_end(), trailer.text())
}

pub fn parse_role(role: &str, difficulty: Option<&str>) -> Result<(Role, Difficulty), String> {
    if role.trim() == "grunt" {
        return Err("The grunt role was retired. Use an implementer, or spawn_peer with an explicit model for mechanical work.".into());
    }
    let role = Role::parse(role.trim()).ok_or_else(|| {
        format!(
            "role must be implementer, reviewer or planner; {} is not a role.",
            boblm_protocol::client::shown(role)
        )
    })?;
    match (
        role,
        difficulty.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Role::Implementer, Some(value)) => {
            let difficulty = Difficulty::parse(value).ok_or_else(|| {
                format!(
                    "difficulty must be simple or hard for an implementer; {} is not a difficulty.",
                    boblm_protocol::client::shown(value)
                )
            })?;
            Ok((role, difficulty))
        }
        (Role::Implementer, None) => Err(
            "An implementer spawn needs a difficulty of simple or hard. Give one and try again."
                .into(),
        ),
        (Role::Reviewer | Role::Planner, Some(_)) => Err(format!(
            "A {} takes no difficulty. Omit difficulty; this role uses its fixed effort.",
            role.label()
        )),
        (Role::Reviewer | Role::Planner, None) => Ok((role, Difficulty::Simple)),
    }
}

/// Resolve a role from stored account readings, exhaustion marks and API headroom.
pub fn resolve_route(
    store: &Store,
    role: Role,
    difficulty: Difficulty,
    enabled: EnabledAccounts,
) -> Result<ChosenRoute, String> {
    let reader = store
        .reader()
        .map_err(|error| format!("Could not read account usage for role routing: {error}."))?;
    let usages = reader
        .account_usages(None)
        .map_err(|error| format!("Could not read account usage for role routing: {error}."))?;
    let exhausted = reader
        .role_exhaustions()
        .map_err(|error| format!("Could not read role exhaustion marks: {error}."))?;
    let sessions = reader
        .sessions()
        .map_err(|error| format!("Could not read account keys for role routing: {error}."))?;
    let keyed: Vec<(&str, &str)> = sessions
        .iter()
        .filter_map(|session| {
            Some((
                session.account_key.as_deref()?,
                session.account_id.as_deref()?,
            ))
        })
        .collect();
    let mut readings = Vec::new();
    for usage in usages {
        let account = match usage.provider.as_str() {
            "claude" => continue,
            "codex" => {
                if keyed.iter().any(|(key, id)| {
                    *key == boblm_tuning::CODEX_ACCOUNT_KEY_PLAN && *id == usage.account_id
                }) {
                    AccountKind::CodexPlan
                } else if keyed.iter().any(|(key, id)| {
                    *key == boblm_tuning::CODEX_ACCOUNT_KEY_API && *id == usage.account_id
                }) {
                    AccountKind::OpenAiApi
                } else {
                    AccountKind::CodexPlan
                }
            }
            _ => continue,
        };
        readings.push(AccountReading {
            account,
            weekly_used_percent: usage.weekly.as_ref().map(|weekly| weekly.used_percent),
            weekly_resets_at: usage.weekly.as_ref().and_then(|weekly| weekly.resets_at),
            credits_balance: usage.credits.as_ref().and_then(|credits| credits.balance),
        });
    }
    let now_ms = unix_millis();
    let mut counts = reader.open_provider_counts().map_err(|error| {
        format!("Could not count provider chats: {error}. Try again after storage is available.")
    })?;
    counts.enabled_plan = [
        enabled
            .plan
            .contains(&boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_A),
        enabled
            .plan
            .contains(&boblm_tuning::CLAUDE_ACCOUNT_KEY_PLAN_B),
    ];
    counts.enabled_api = [
        enabled
            .api
            .contains(&boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A),
        enabled
            .api
            .contains(&boblm_tuning::CLAUDE_ACCOUNT_KEY_API_B),
    ];
    let accounts = crate::claude_account::plan_accounts(store, enabled.plan)?;
    let candidates: Vec<_> = accounts
        .iter()
        .filter(|account| {
            boblm_tuning::provider_group("claude", boblm_tuning::MODEL_SONNET, Some(account.key))
                .is_none_or(|group| {
                    counts
                        .spawn_rejection(group, role == Role::Reviewer)
                        .is_none()
                })
        })
        .cloned()
        .collect();
    let plan = boblm_tuning::claude_accounts::choose(&candidates, now_ms / 1000);
    counts.unavailable_plan = [plan.is_none(); 2];
    readings.push(AccountReading {
        account: AccountKind::ClaudePlan,
        weekly_used_percent: plan
            .and_then(|account| account.used_percent)
            .or_else(|| plan.is_none().then_some(100.0)),
        weekly_resets_at: plan.and_then(|account| account.resets_at),
        credits_balance: None,
    });
    let api = crate::claude_account::choose_api_from(store, enabled.api, now_ms / 1000)?;
    counts.unavailable_api = [api.is_none(); 2];
    let mut exhausted = exhausted;
    // New quota failures mark individual Claude API accounts. Retain a legacy global
    // mark for A until its original expiry, without blocking a newly enabled B.
    if api.is_some_and(|key| key != boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A) {
        exhausted.retain(|mark| mark.step != StepId::ReviewerClaudeApi);
    }

    let headroom = reader.api_reviewer_headroom(now_ms)
        .map_err(|error| format!("Could not measure API reviewer headroom: {error}. Try spawning again after storage is available."))?;
    match boblm_tuning::resolve_with_groups(
        role,
        difficulty,
        &readings,
        &exhausted,
        headroom,
        now_ms / 1000,
        counts,
    ) {
        RouteOutcome::Chosen(mut route) => {
            route.account_key = match route.account {
                AccountKind::ClaudePlan => plan.map(|account| account.key),
                AccountKind::ClaudeApi => api,
                _ => None,
            };
            Ok(route)
        }
        RouteOutcome::Exhausted(rejections) => {
            let reasons = rejections
                .iter()
                .map(|rejection| rejection.reason.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if role == Role::Planner {
                let reset = accounts
                    .iter()
                    .filter_map(|account| account.resets_at.or(account.exhausted_until))
                    .filter(|reset| *reset > now_ms / 1000)
                    .chain(
                        exhausted
                            .iter()
                            .filter(|mark| mark.step == StepId::PlannerClaude)
                            .map(|mark| mark.until),
                    )
                    .min();
                let reset = reset
                    .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
                    .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "a time Claude has not reported".into());
                Err(format!(
                    "No planner model is available until the Claude reset at {reset}. {reasons}"
                ))
            } else {
                let advice = if rejections
                    .iter()
                    .any(|rejection| boblm_tuning::is_cap_reason(&rejection.reason))
                {
                    format!(" {}", boblm_tuning::CAP_ADVICE)
                } else {
                    String::new()
                };
                Err(format!(
                    "No {} step is available: {reasons}{advice}",
                    role.label()
                ))
            }
        }
    }
}

pub fn account_key_for_route(route: &ChosenRoute) -> Option<&'static str> {
    match route.account {
        AccountKind::CodexPlan => Some(boblm_tuning::CODEX_ACCOUNT_KEY_PLAN),
        AccountKind::OpenAiApi => Some(boblm_tuning::CODEX_ACCOUNT_KEY_API),
        AccountKind::ClaudeApi | AccountKind::ClaudePlan => route.account_key,
        AccountKind::DeepSeek => None,
    }
}

pub fn route_json(route: &ChosenRoute) -> Value {
    json!({
        "role": route.role.as_str(),
        "difficulty": route.difficulty.as_str(),
        "model": route.model,
        "modelLabel": boblm_tuning::model_display(route.model),
        "effort": route.effort,
        "account": route.account_key.unwrap_or(route.account.as_str()),
        "accountKind": route.account.as_str(),
        "step": route.step.as_str(),
        "reason": route.reason,
    })
}

pub fn route_from_json(value: &Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("The stored role routing receipt is not an object. Start a new request.")?;
    for field in [
        "role",
        "difficulty",
        "model",
        "effort",
        "account",
        "step",
        "reason",
    ] {
        if object.get(field).and_then(Value::as_str).is_none() {
            return Err(format!(
                "The stored role routing receipt has no {field}. Start a new request."
            ));
        }
    }
    Ok(value.clone())
}

pub fn route_session_model(route: &ChosenRoute) -> (&'static str, &'static str) {
    (route.model, route.effort)
}

pub fn step_label(step: StepId) -> &'static str {
    step.label()
}

/// Resolve the requested folder inside the caller's project. Relative paths are joined
/// to the project root; symlinks and `..` cannot escape it after canonicalization.
pub fn working_directory(
    store: &Store,
    caller: &Session,
    requested: Option<&str>,
) -> Result<String, String> {
    let project_id = caller
        .project_id
        .as_deref()
        .ok_or("Your chat has no project, so a subagent cannot inherit a working folder.")?;
    let project = store
        .reader()
        .map_err(|error| format!("Could not read your project: {error}."))?
        .project(project_id)
        .map_err(|error| format!("Could not read your project: {error}."))?
        .ok_or("Your project no longer exists. Reload the chat list and try again.")?;
    let root = std::fs::canonicalize(&project.path).map_err(|error| {
        format!(
            "The project folder {} cannot be used: {error}. Restore the folder and try again.",
            project.path
        )
    })?;
    let requested = requested.unwrap_or(&caller.working_directory);
    let path = Path::new(requested);
    let candidate: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let candidate = std::fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "The subagent working folder {} cannot be used: {error}. Give a folder inside your project.",
            candidate.display()
        )
    })?;
    if !candidate.is_dir() {
        return Err(format!(
            "The subagent working folder {} is not a directory. Give an existing folder inside your project.",
            candidate.display()
        ));
    }
    if !candidate.starts_with(&root) {
        return Err(format!(
            "The subagent working folder {} is outside your project {}. Give a folder inside it.",
            candidate.display(),
            root.display()
        ));
    }
    Ok(candidate.to_string_lossy().into_owned())
}

/// Open subagents of one root, newest last, with idle time from their last stored event.
pub fn open_subagents(store: &Store, root_session_id: &str) -> Result<Vec<Value>, String> {
    let reader = store
        .reader()
        .map_err(|error| format!("Could not read open subagents: {error}."))?;
    let now = unix_millis();
    let mut open = Vec::new();
    for session in reader
        .sessions()
        .map_err(|error| format!("Could not read open subagents: {error}."))?
    {
        if session.root_session_id.as_deref() != Some(root_session_id) || session.archived {
            continue;
        }
        let last = reader
            .last_activity(&session.id)
            .map_err(|error| format!("Could not read a subagent's idle time: {error}."))?;
        open.push(json!({
            "sessionId": session.id,
            "name": session.name,
            "role": session.role,
            "model": session.model,
            "modelLabel": boblm_tuning::model_display(&session.model),
            "effort": session.effort,
            "idleSeconds": (now.saturating_sub(last) / 1000).max(0),
        }));
    }
    Ok(open)
}

pub fn is_subagent(session: &Session) -> bool {
    session.parent_session_id.is_some() || session.root_session_id.is_some()
}

pub fn role_spawn_result(
    session: SessionInfo,
    already_accepted: bool,
    dispatch_failed: bool,
    route: Value,
    open: Vec<Value>,
) -> Value {
    json!({
        "sessionId": session.session_id,
        "messageId": session.session_id,
        "accepted": true,
        "alreadyAccepted": already_accepted,
        "dispatchFailed": dispatch_failed,
        "session": session,
        "routing": route,
        "openSubagents": open,
    })
}

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
