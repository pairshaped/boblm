//! Recent API usage and reviewer reservations for spawn routing (ADR 0043).
#[path = "api_headroom.generated.rs"]
mod sql;

use crate::{Reader, Result};
use boblm_tuning::ApiHeadroom;

impl Reader {
    /// Measure one consistent snapshot. Token timestamps and `now` are Unix milliseconds.
    /// Cached tokens count toward TPM. Mid-turn reviewers reserve their last call size;
    /// recently active and newly spawned reviewers also occupy the concurrency cap.
    pub fn api_reviewer_headroom(&self, now: i64) -> Result<ApiHeadroom> {
        let window =
            i64::try_from(boblm_tuning::OPENAI_API_TPM_WINDOW.as_millis()).unwrap_or(i64::MAX);
        let row = sql::measure(
            &self.conn,
            sql::MeasureParams {
                account_key: boblm_tuning::CODEX_ACCOUNT_KEY_API,
                model: boblm_tuning::MODEL_GPT_6_LUNA,
                since: now.saturating_sub(window),
                now,
            },
        )?
        .pop()
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        Ok(ApiHeadroom {
            recent_tokens: u64::try_from(row.recent_tokens).unwrap_or_default(),
            projected_tokens: u64::try_from(row.projected_tokens).unwrap_or_default(),
            active_reviewers: usize::try_from(row.active_reviewers.unwrap_or_default())
                .unwrap_or_default(),
        })
    }
}
