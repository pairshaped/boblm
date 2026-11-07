//! Incremental token and weekly-allowance aggregates for the usage report (ADR 0042).
#[path = "usage_report.generated.rs"]
mod sql;

use crate::{Reader, Result};

#[derive(Debug, Clone)]
pub struct TokenUsageDay {
    pub provider: String,
    pub model: String,
    pub day: String,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub calls: i64,
}

#[derive(Debug, Clone)]
pub struct AllowanceUsageDay {
    pub provider: String,
    pub account_id: String,
    pub day: String,
    pub weekly_used: f64,
    pub weekly_resets_at: Option<i64>,
    pub observed_at: i64,
    pub samples: i64,
}

impl Reader {
    pub fn usage_token_days(&self, since: &str, until: &str) -> Result<Vec<TokenUsageDay>> {
        Ok(
            sql::token_daily(&self.conn, sql::TokenDailyParams { since, until })?
                .into_iter()
                .map(|row| TokenUsageDay {
                    provider: row.provider,
                    model: row.model,
                    day: row.day,
                    input_tokens: row.input_tokens,
                    cache_read_tokens: row.cache_read_tokens,
                    cache_write_tokens: row.cache_write_tokens,
                    output_tokens: row.output_tokens,
                    calls: row.calls,
                })
                .collect(),
        )
    }

    pub fn usage_allowance_days(&self, since: &str, until: &str) -> Result<Vec<AllowanceUsageDay>> {
        Ok(
            sql::allowance_daily(&self.conn, sql::AllowanceDailyParams { since, until })?
                .into_iter()
                .map(|row| AllowanceUsageDay {
                    provider: row.provider,
                    account_id: row.account_id,
                    day: row.day,
                    weekly_used: row.weekly_used,
                    weekly_resets_at: row.weekly_resets_at,
                    observed_at: row.observed_at,
                    samples: row.samples,
                })
                .collect(),
        )
    }
}

impl Reader {
    pub fn usage_token_calls(&self, since: i64, until: i64) -> Result<Vec<sql::TokenCallsRow>> {
        Ok(sql::token_calls(
            &self.conn,
            sql::TokenCallsParams { since, until },
        )?)
    }

    pub fn usage_allowance_observations(
        &self,
        since: i64,
        until: i64,
    ) -> Result<Vec<sql::AllowanceObservationsRow>> {
        Ok(sql::allowance_observations(
            &self.conn,
            sql::AllowanceObservationsParams { since, until },
        )?)
    }
}
