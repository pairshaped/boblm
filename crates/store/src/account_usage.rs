//! Allowance readings belong to a provider account, not to one chat (ADR 0037).
#[path = "account_usage.generated.rs"]
mod sql;

use crate::{Reader, Result};
use boblm_protocol::usage::{
    AccountUsage, AllowanceReading, AllowanceUsage, CreditsReading, CreditsUsage,
};
use rusqlite::Transaction;
use serde_json::Value;

/// Why a usage event did or did not change its account row.
pub(crate) fn observe(
    tx: &Transaction<'_>,
    session: &str,
    payload: &Value,
    observed_at: i64,
) -> Result<()> {
    if payload["type"] != "usage" {
        return Ok(());
    }
    let Some(account) = sql::session_account(tx, sql::SessionAccountParams { session })?.pop()
    else {
        return Ok(());
    };
    let Some(account_id) = account.account_id else {
        return Ok(());
    };
    let provider = account.provider;
    let incoming_five_hour = allowance(payload, "five_hour", observed_at);
    let incoming_weekly = allowance(payload, "weekly", observed_at);
    let incoming_credits = credits(payload, observed_at);
    if incoming_five_hour.is_none() && incoming_weekly.is_none() && incoming_credits.is_none() {
        return Ok(());
    }
    let stored = sql::get(
        tx,
        sql::GetParams {
            provider: &provider,
            account_id: &account_id,
        },
    )?
    .pop()
    .map(|row| stored(row.into()));
    let refresh =
        i64::try_from(boblm_tuning::ACCOUNT_USAGE_CONFIRM_REFRESH.as_millis()).unwrap_or(i64::MAX);
    let (five_hour, five_changed) = merge_allowance(
        stored.as_ref().and_then(|usage| usage.five_hour.as_ref()),
        incoming_five_hour,
        observed_at,
        refresh,
    );
    let (weekly, weekly_changed) = merge_allowance(
        stored.as_ref().and_then(|usage| usage.weekly.as_ref()),
        incoming_weekly,
        observed_at,
        refresh,
    );
    let (credits, credits_changed) = merge_credits(
        stored.as_ref().and_then(|usage| usage.credits.as_ref()),
        incoming_credits,
        observed_at,
        refresh,
    );
    if !(five_changed || weekly_changed || credits_changed) {
        return Ok(());
    }
    let usage = AccountUsage {
        provider: provider.clone(),
        account_id: account_id.clone(),
        five_hour,
        weekly,
        credits,
    };
    if stored.is_some() {
        sql::update(
            tx,
            sql::UpdateParams {
                provider: &provider,
                account_id: &account_id,
                five_hour_used: usage.five_hour.as_ref().map(|reading| reading.used_percent),
                five_hour_resets_at: usage
                    .five_hour
                    .as_ref()
                    .and_then(|reading| reading.resets_at),
                five_hour_observed_at: usage.five_hour.as_ref().map(|reading| reading.observed_at),
                weekly_used: usage.weekly.as_ref().map(|reading| reading.used_percent),
                weekly_resets_at: usage.weekly.as_ref().and_then(|reading| reading.resets_at),
                weekly_observed_at: usage.weekly.as_ref().map(|reading| reading.observed_at),
                credits_balance: usage.credits.as_ref().and_then(|reading| reading.balance),
                credits_observed_at: usage.credits.as_ref().map(|reading| reading.observed_at),
            },
        )?;
    } else {
        sql::insert(
            tx,
            sql::InsertParams {
                provider: &provider,
                account_id: &account_id,
                five_hour_used: usage.five_hour.as_ref().map(|reading| reading.used_percent),
                five_hour_resets_at: usage
                    .five_hour
                    .as_ref()
                    .and_then(|reading| reading.resets_at),
                five_hour_observed_at: usage.five_hour.as_ref().map(|reading| reading.observed_at),
                weekly_used: usage.weekly.as_ref().map(|reading| reading.used_percent),
                weekly_resets_at: usage.weekly.as_ref().and_then(|reading| reading.resets_at),
                weekly_observed_at: usage.weekly.as_ref().map(|reading| reading.observed_at),
                credits_balance: usage.credits.as_ref().and_then(|reading| reading.balance),
                credits_observed_at: usage.credits.as_ref().map(|reading| reading.observed_at),
            },
        )?;
    }
    publish(tx, &usage, observed_at)
}

fn allowance(payload: &Value, name: &str, observed_at: i64) -> Option<AllowanceReading> {
    let value = payload.get(name)?;
    if value.is_null() {
        return None;
    }
    let reading: AllowanceUsage = serde_json::from_value(value.clone()).ok()?;
    Some(AllowanceReading {
        used_percent: reading.used_percent,
        resets_at: reading.resets_at,
        observed_at,
    })
}

fn credits(payload: &Value, observed_at: i64) -> Option<CreditsReading> {
    let value = payload.get("credits")?;
    if value.is_null() {
        return None;
    }
    let reading: CreditsUsage = serde_json::from_value(value.clone()).ok()?;
    Some(CreditsReading {
        balance: (reading.has_credits && !reading.unlimited)
            .then_some(reading.balance)
            .flatten(),
        observed_at,
    })
}

fn merge_allowance(
    stored: Option<&AllowanceReading>,
    incoming: Option<AllowanceReading>,
    now: i64,
    refresh: i64,
) -> (Option<AllowanceReading>, bool) {
    let Some(incoming) = incoming else {
        return (stored.cloned(), false);
    };
    match stored {
        Some(old) if incoming.observed_at <= old.observed_at => (Some(old.clone()), false),
        Some(old) => {
            let unchanged =
                old.used_percent == incoming.used_percent && old.resets_at == incoming.resets_at;
            if unchanged && old.observed_at > now - refresh {
                (Some(old.clone()), false)
            } else {
                (Some(incoming), true)
            }
        }
        None => (Some(incoming), true),
    }
}

fn merge_credits(
    stored: Option<&CreditsReading>,
    incoming: Option<CreditsReading>,
    now: i64,
    refresh: i64,
) -> (Option<CreditsReading>, bool) {
    let Some(incoming) = incoming else {
        return (stored.cloned(), false);
    };
    match stored {
        Some(old) if incoming.observed_at <= old.observed_at => (Some(old.clone()), false),
        Some(old) => {
            if old.balance == incoming.balance && old.observed_at > now - refresh {
                (Some(old.clone()), false)
            } else {
                (Some(incoming), true)
            }
        }
        None => (Some(incoming), true),
    }
}

struct StoredParts {
    provider: String,
    account_id: String,
    five_hour_used: Option<f64>,
    five_hour_resets_at: Option<i64>,
    five_hour_observed_at: Option<i64>,
    weekly_used: Option<f64>,
    weekly_resets_at: Option<i64>,
    weekly_observed_at: Option<i64>,
    credits_balance: Option<f64>,
    credits_observed_at: Option<i64>,
}

impl From<sql::GetRow> for StoredParts {
    fn from(row: sql::GetRow) -> Self {
        Self {
            provider: row.provider,
            account_id: row.account_id,
            five_hour_used: row.five_hour_used,
            five_hour_resets_at: row.five_hour_resets_at,
            five_hour_observed_at: row.five_hour_observed_at,
            weekly_used: row.weekly_used,
            weekly_resets_at: row.weekly_resets_at,
            weekly_observed_at: row.weekly_observed_at,
            credits_balance: row.credits_balance,
            credits_observed_at: row.credits_observed_at,
        }
    }
}

impl From<sql::AllRow> for StoredParts {
    fn from(row: sql::AllRow) -> Self {
        Self {
            provider: row.provider,
            account_id: row.account_id,
            five_hour_used: row.five_hour_used,
            five_hour_resets_at: row.five_hour_resets_at,
            five_hour_observed_at: row.five_hour_observed_at,
            weekly_used: row.weekly_used,
            weekly_resets_at: row.weekly_resets_at,
            weekly_observed_at: row.weekly_observed_at,
            credits_balance: row.credits_balance,
            credits_observed_at: row.credits_observed_at,
        }
    }
}

impl From<sql::ByProviderRow> for StoredParts {
    fn from(row: sql::ByProviderRow) -> Self {
        Self {
            provider: row.provider,
            account_id: row.account_id,
            five_hour_used: row.five_hour_used,
            five_hour_resets_at: row.five_hour_resets_at,
            five_hour_observed_at: row.five_hour_observed_at,
            weekly_used: row.weekly_used,
            weekly_resets_at: row.weekly_resets_at,
            weekly_observed_at: row.weekly_observed_at,
            credits_balance: row.credits_balance,
            credits_observed_at: row.credits_observed_at,
        }
    }
}

fn stored(parts: StoredParts) -> AccountUsage {
    AccountUsage {
        provider: parts.provider,
        account_id: parts.account_id,
        five_hour: reading(
            parts.five_hour_used,
            parts.five_hour_resets_at,
            parts.five_hour_observed_at,
        ),
        weekly: reading(
            parts.weekly_used,
            parts.weekly_resets_at,
            parts.weekly_observed_at,
        ),
        credits: parts.credits_observed_at.map(|observed_at| CreditsReading {
            balance: parts.credits_balance,
            observed_at,
        }),
    }
}

fn reading(
    used_percent: Option<f64>,
    resets_at: Option<i64>,
    observed_at: Option<i64>,
) -> Option<AllowanceReading> {
    Some(AllowanceReading {
        used_percent: used_percent?,
        resets_at,
        observed_at: observed_at?,
    })
}

/// Every open chat on the account sees the new readings through its normal status stream.
fn publish(tx: &Transaction<'_>, usage: &AccountUsage, observed_at: i64) -> Result<()> {
    let sessions = sql::sessions_on_account(
        tx,
        sql::SessionsOnAccountParams {
            provider: &usage.provider,
            account_id: &usage.account_id,
        },
    )?;
    let account_usage = serde_json::to_value(usage).map_err(std::io::Error::other)?;
    for session in sessions {
        let Some(raw) =
            sql::latest_state_summary(tx, sql::LatestStateSummaryParams { session: &session })?
                .pop()
        else {
            continue;
        };
        let mut status: Value = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        status["accountUsage"] = account_usage.clone();
        crate::events::append(tx, &session, "state_summary", status.clone(), observed_at)?;
        crate::session_list::append(tx, &session, "state_summary", &status, observed_at)?;
    }
    Ok(())
}

impl Reader {
    /// Read one account row by its provider and provider account ID.
    pub fn account_usage(&self, provider: &str, account_id: &str) -> Result<Option<AccountUsage>> {
        Ok(sql::get(
            &self.conn,
            sql::GetParams {
                provider,
                account_id,
            },
        )?
        .pop()
        .map(|row| stored(row.into())))
    }

    /// Read every account row, optionally narrowed to one provider.
    pub fn account_usages(&self, provider: Option<&str>) -> Result<Vec<AccountUsage>> {
        let rows = match provider {
            Some(provider) => sql::by_provider(&self.conn, sql::ByProviderParams { provider })?
                .into_iter()
                .map(|row| stored(row.into()))
                .collect(),
            None => sql::all(&self.conn)?
                .into_iter()
                .map(|row| stored(row.into()))
                .collect(),
        };
        Ok(rows)
    }

    /// The account row for the provider account this chat last launched with.
    pub fn account_usage_for_session(&self, session: &str) -> Result<Option<AccountUsage>> {
        let Some(account) =
            sql::session_account(&self.conn, sql::SessionAccountParams { session })?.pop()
        else {
            return Ok(None);
        };
        let Some(account_id) = account.account_id else {
            return Ok(None);
        };
        self.account_usage(&account.provider, &account_id)
    }
}
