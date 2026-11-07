//! Per-parent-message runaway accounting from deduplicated model calls (ADR 0043).
#[path = "subagent_rounds.generated.rs"]
mod sql;

use crate::{Result, Store};
use boblm_tuning::{SUBAGENT_ROUND_MAX_CALLS, SUBAGENT_ROUND_MAX_TOKENS};

impl Store {
    /// Record one pause reason and parent report per round. The caller must own the
    /// session's delivery actor and interrupt it when this returns true.
    pub fn pause_runaway_subagent_round(&self, session: &str, now: i64) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        let Some(round) = sql::over_limit(&tx, sql::OverLimitParams { session })?.pop() else {
            return Ok(false);
        };
        if round.tokens <= SUBAGENT_ROUND_MAX_TOKENS && round.calls <= SUBAGENT_ROUND_MAX_CALLS {
            return Ok(false);
        }
        let parent_session = round.parent_session_id.as_deref().ok_or_else(|| {
            crate::Error::Corrupt("The runaway subagent has no parent for its report. Inspect its chat before resuming.".into())
        })?;
        let limit = match (
            round.tokens > SUBAGENT_ROUND_MAX_TOKENS,
            round.calls > SUBAGENT_ROUND_MAX_CALLS,
        ) {
            (true, true) => format!(
                "{SUBAGENT_ROUND_MAX_TOKENS} tokens and {SUBAGENT_ROUND_MAX_CALLS} model calls"
            ),
            (true, false) => format!("{SUBAGENT_ROUND_MAX_TOKENS} tokens"),
            _ => format!("{SUBAGENT_ROUND_MAX_CALLS} model calls"),
        };
        let title = if round.name.trim().is_empty() {
            session
        } else {
            round.name.trim()
        };
        let message = format!(
            "{title} was paused after using {} tokens and {} model calls in one round, over BobLM's runaway limit of {limit}. Decide whether it is still making real progress: inspect it with inspect_peer, then either ping it to continue (a new round starts with a fresh limit), or archive it. If you can't tell, ask the owner with the ask tool before continuing.",
            round.tokens, round.calls,
        );
        sql::pause_round(
            &tx,
            sql::PauseRoundParams {
                session,
                round_seq: round.round_seq,
                tokens: round.tokens,
                calls: round.calls,
                max_tokens: SUBAGENT_ROUND_MAX_TOKENS,
                max_calls: SUBAGENT_ROUND_MAX_CALLS,
                created_at: now,
            },
        )?;
        crate::events::append(
            &tx,
            session,
            "driver_error",
            serde_json::json!({"message": message}),
            now,
        )?;
        sql::queue_report(
            &tx,
            sql::QueueReportParams {
                session,
                parent_session,
            },
        )?;
        tx.commit()?;
        Ok(true)
    }
}
