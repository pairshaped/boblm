//! The account usage modal's data: one labeled entry per enabled account (ADR 0049).
use crate::app::{ApiError, ApiResult, App};
use axum::{Json, body::Bytes, extract::State};
use boblm_protocol::client::{AccountUsageInfo, AccountUsageReply, ListAccountUsage};
use boblm_tuning::*;
use std::sync::Arc;

pub async fn list(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<AccountUsageReply> {
    let _: ListAccountUsage = super::app::body(&bytes)?;
    let store = app.store.clone();
    let enabled = app.claude_accounts();
    let accounts = tokio::task::spawn_blocking(move || {
        let reader = store.reader()?;
        let data_dir = store.path().parent().unwrap_or(std::path::Path::new("."));
        let sessions = reader.sessions()?;
        let usages = reader.account_usages(None)?;
        let usage_of = |provider: &str, id: Option<String>| {
            id.and_then(|id| {
                usages
                    .iter()
                    .find(|usage| usage.provider == provider && usage.account_id == id)
                    .cloned()
            })
        };
        let entry = |label: &str, kind: &str, provider: &str, usage: Option<_>| {
            let usage: Option<boblm_protocol::usage::AccountUsage> = usage;
            AccountUsageInfo {
                label: label.into(),
                kind: kind.into(),
                provider: provider.into(),
                five_hour: usage.as_ref().and_then(|usage| usage.five_hour.clone()),
                weekly: usage.as_ref().and_then(|usage| usage.weekly.clone()),
                credits: usage.and_then(|usage| usage.credits),
            }
        };
        let api = |label: &str, provider: &str| entry(label, "api", provider, None);
        // Codex plan chats have no key or the plan key; the API key never owns plan readings.
        let codex_id = sessions
            .iter()
            .find(|session| {
                session.agent == "codex"
                    && session
                        .account_key
                        .as_deref()
                        .is_none_or(|key| key == CODEX_ACCOUNT_KEY_PLAN)
                    && session.account_id.is_some()
            })
            .and_then(|session| session.account_id.clone());
        let mut accounts = vec![entry(
            "Codex plan",
            "plan",
            "codex",
            usage_of("codex", codex_id),
        )];
        for (key, label) in [
            (CLAUDE_ACCOUNT_KEY_PLAN_A, "Claude plan A"),
            (CLAUDE_ACCOUNT_KEY_PLAN_B, "Claude plan B"),
        ] {
            if enabled.plan.contains(&key) {
                let id = crate::claude_account::plan_account_id(&reader, data_dir, key);
                accounts.push(entry(label, "plan", "claude", usage_of("claude", id)));
            }
        }
        accounts.push(api("OpenAI API", "codex"));
        for (key, label) in [
            (CLAUDE_ACCOUNT_KEY_API_A, "Claude API A"),
            (CLAUDE_ACCOUNT_KEY_API_B, "Claude API B"),
        ] {
            if enabled.api.contains(&key) {
                accounts.push(api(label, "claude"));
            }
        }
        Ok::<_, boblm_store::Error>(accounts)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(AccountUsageReply { accounts }))
}
