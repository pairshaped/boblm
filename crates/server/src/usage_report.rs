//! Read-only token and allowance report from the incremental aggregates (ADR 0042).
use boblm_store::Store;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Optional filters. Dates are inclusive Toronto calendar dates in `YYYY-MM-DD` form.
pub struct Args {
    pub since: Option<String>,
    pub until: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
}

struct TokenBucket {
    week: NaiveDate,
    provider: String,
    model: String,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    calls: i64,
}

pub fn report(store: &Store, args: Args) -> Result<Value, String> {
    report_at(store, args, Utc::now())
}

fn report_at(store: &Store, args: Args, now: DateTime<Utc>) -> Result<Value, String> {
    let zone: chrono_tz::Tz = boblm_tuning::REPORT_TIME_ZONE.parse().map_err(|error| {
        format!("Could not use the report time zone: {error}. Correct REPORT_TIME_ZONE.")
    })?;
    let today = now.with_timezone(&zone).date_naive();
    let until = match args.until.as_deref() {
        Some(value) => parse_date("until", value)?,
        None => today,
    };
    let since = match args.since.as_deref() {
        Some(value) => parse_date("since", value)?,
        None => until - Duration::days(6),
    };
    if since > until {
        return Err(format!(
            "since ({since}) is after until ({until}); give a range that starts on or before it ends."
        ));
    }
    let provider = args
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let start = midnight(since, zone)?;
    let end_day = until
        .succ_opt()
        .ok_or("The until date is too late. Choose an earlier date.")?;
    let end = midnight(end_day, zone)?;
    let reader = store.reader().map_err(|error| {
        format!("Could not read the usage aggregate: {error}. Check the server database.")
    })?;
    let token_days = reader.usage_token_calls(start, end).map_err(|error| {
        format!("Could not read the token aggregate: {error}. Check the server database.")
    })?;
    let allowance_days = reader
        .usage_allowance_observations(start, end)
        .map_err(|error| {
            format!("Could not read the allowance aggregate: {error}. Check the server database.")
        })?;

    let mut buckets: BTreeMap<(NaiveDate, String, String), TokenBucket> = BTreeMap::new();
    for row in token_days {
        if provider.is_some_and(|wanted| wanted != row.provider)
            || model.is_some_and(|wanted| wanted != row.model)
        {
            continue;
        }
        let day = timestamp_day(row.created_at, zone)?;
        let week = day - Duration::days(i64::from(day.weekday().num_days_from_monday()));
        let bucket = buckets
            .entry((week, row.provider.clone(), row.model.clone()))
            .or_insert(TokenBucket {
                week,
                provider: row.provider,
                model: row.model,
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 0,
                calls: 0,
            });
        bucket.input_tokens += row.input_tokens;
        bucket.cache_read_tokens += row.cache_read_tokens;
        bucket.cache_write_tokens += row.cache_write_tokens;
        bucket.output_tokens += row.output_tokens;
        bucket.calls += 1;
    }
    let models: Vec<Value> = buckets
        .into_values()
        .map(|bucket| {
            let cached = bucket.cache_read_tokens;
            let input = bucket.input_tokens;
            let cache_write = bucket.cache_write_tokens;
            let denominator = input + cached + cache_write;
            let cache_hit_rate = (denominator > 0)
                .then(|| cached as f64 / denominator as f64)
                .map(round6);
            let mut value = json!({
                "week": bucket.week.to_string(),
                "provider": bucket.provider,
                "model": bucket.model,
                "modelLabel": boblm_tuning::model_display(&bucket.model),
                "inputTokens": input,
                "cacheReadTokens": cached,
                "cacheWriteTokens": cache_write,
                "outputTokens": bucket.output_tokens,
                "cacheHitRate": cache_hit_rate,
                "calls": bucket.calls,
            });
            if bucket.provider == "pi" {
                let cost = (input as f64 * boblm_tuning::PI_MODEL_COST_INPUT
                    + cached as f64 * boblm_tuning::PI_MODEL_COST_CACHE_READ
                    + bucket.output_tokens as f64 * boblm_tuning::PI_MODEL_COST_OUTPUT)
                    / 1_000_000.0;
                value["estimatedCostUsd"] = json!(round6(cost));
            }
            value
        })
        .collect();

    let mut daily: BTreeMap<(NaiveDate, String, String), boblm_store::AllowanceUsageDay> =
        BTreeMap::new();
    for row in allowance_days {
        let day = timestamp_day(row.observed_at, zone)?;
        let entry = daily
            .entry((day, row.provider.clone(), row.account_id.clone()))
            .or_insert(boblm_store::AllowanceUsageDay {
                provider: row.provider,
                account_id: row.account_id,
                day: day.to_string(),
                weekly_used: row.weekly_used,
                weekly_resets_at: row.weekly_resets_at,
                observed_at: row.observed_at,
                samples: 0,
            });
        entry.samples += 1;
        entry.weekly_used = row.weekly_used;
        entry.weekly_resets_at = row.weekly_resets_at;
        entry.observed_at = row.observed_at;
    }
    let mut previous: BTreeMap<(String, String), boblm_store::AllowanceUsageDay> = BTreeMap::new();
    let mut allowance = Vec::new();
    for row in daily.into_values() {
        if provider.is_some_and(|wanted| wanted != row.provider) {
            continue;
        }
        let day = parse_date("aggregate day", &row.day)?;
        let key = (row.provider.clone(), row.account_id.clone());
        let previous_day = previous.get(&key);
        let same_window = previous_day.is_some_and(|old| {
            old.weekly_resets_at.is_some()
                && old.weekly_resets_at == row.weekly_resets_at
                && old
                    .day
                    .parse::<NaiveDate>()
                    .ok()
                    .is_some_and(|old_day| old_day + Duration::days(1) == day)
        });
        let consumed = same_window.then(|| {
            let old = previous_day.expect("same_window checked the previous row");
            let delta = row.weekly_used - old.weekly_used;
            (delta >= 0.0).then_some(round6(delta))
        });
        allowance.push(json!({
            "provider": row.provider,
            "accountId": row.account_id,
            "day": row.day,
            "weeklyUsedPercent": round6(row.weekly_used),
            "weeklyResetsAt": row.weekly_resets_at,
            "observedAt": row.observed_at,
            "samples": row.samples,
            "previousDay": previous_day.map(|old| old.day.clone()),
            "dailyConsumedPercent": consumed.flatten(),
        }));
        previous.insert(key, row);
    }

    Ok(json!({
        "since": since.to_string(),
        "until": until.to_string(),
        "groupBy": "week",
        "models": models,
        "allowanceDaily": allowance,
        "notes": [
            "Claude and Codex are subscription plans, so their token groups have no dollar estimate.",
            "Pi cost uses PI_MODEL_COST_INPUT, PI_MODEL_COST_CACHE_READ and PI_MODEL_COST_OUTPUT; cache writes have no separate published price and contribute nothing.",
            "allowanceDaily is the last weekly reading per account per day. dailyConsumedPercent is null when the previous calendar day is missing, the reset window changed, or usage dropped inside the window.",
            "Codex token counts come from thread/tokenUsage/updated; events recorded before this report existed have no Codex token breakdown."
        ]
    }))
}

fn parse_date(what: &str, value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
        format!(
            "{what} must be a Toronto date like 2026-10-31; {} could not be parsed.",
            boblm_protocol::client::shown(value)
        )
    })
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn midnight(day: NaiveDate, zone: chrono_tz::Tz) -> Result<i64, String> {
    day.and_hms_opt(0, 0, 0)
        .and_then(|time| zone.from_local_datetime(&time).single())
        .map(|time| time.timestamp_millis())
        .ok_or_else(|| {
            format!(
                "Could not resolve midnight on {day} in the report time zone. Choose another date."
            )
        })
}

fn timestamp_day(timestamp: i64, zone: chrono_tz::Tz) -> Result<NaiveDate, String> {
    DateTime::from_timestamp_millis(timestamp)
        .map(|time| time.with_timezone(&zone).date_naive())
        .ok_or_else(|| format!("The usage timestamp {timestamp} is outside the supported range. Check the recorded event."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn args(since: Option<&str>, until: Option<&str>) -> Args {
        Args {
            since: since.map(str::to_owned),
            until: until.map(str::to_owned),
            model: None,
            provider: None,
        }
    }

    #[test]
    fn toronto_usage_dates_and_allowance_deltas() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("usage.sqlite")).unwrap();
        store.create_session("chat", "/fake", 1).unwrap();
        store.set_session_account("chat", Some("account")).unwrap();
        // Sunday 23:30 Toronto is Monday in UTC, across a week boundary.
        for (id, time) in [
            ("late", "2026-11-02T04:30:00Z"),
            ("next", "2026-11-02T05:00:00Z"),
        ] {
            store.record_event("chat", "agent_event", json!({"type":"assistant","message":{"id":id,"model":"sonnet","usage":{"input_tokens":100}}}), at(time).timestamp_millis()).unwrap();
        }
        for (used, time) in [
            (10, "2026-11-01T03:30:00Z"),
            (15, "2026-11-01T04:00:00Z"),
            (20, "2026-11-01T05:30:00Z"),
            (25, "2026-11-01T06:30:00Z"),
            (30, "2026-11-02T04:30:00Z"),
        ] {
            store
                .record_event(
                    "chat",
                    "agent_event",
                    json!({"type":"usage","weekly":{"usedPercent":used,"resetsAt":1800000000}}),
                    at(time).timestamp_millis(),
                )
                .unwrap();
        }
        let result = report_at(
            &store,
            args(Some("2026-10-31"), Some("2026-11-01")),
            at("2026-11-02T04:30:00Z"),
        )
        .unwrap();
        assert_eq!(result["models"][0]["week"], "2026-10-26");
        assert_eq!(result["models"][0]["calls"], 1);
        let rows = result["allowanceDaily"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["day"], "2026-10-31");
        assert_eq!(rows[1]["day"], "2026-11-01");
        assert_eq!(rows[1]["samples"], 4);
        assert_eq!(rows[1]["weeklyUsedPercent"], 30.0);
        assert_eq!(rows[1]["dailyConsumedPercent"], 20.0);
        let default = report_at(&store, args(None, None), at("2026-11-02T04:30:00Z")).unwrap();
        assert_eq!(default["until"], "2026-11-01");
        assert_eq!(default["since"], "2026-10-26");
        let zone = boblm_tuning::REPORT_TIME_ZONE.parse().unwrap();
        assert_eq!(
            midnight(parse_date("day", "2026-11-02").unwrap(), zone).unwrap()
                - midnight(parse_date("day", "2026-11-01").unwrap(), zone).unwrap(),
            25 * 60 * 60 * 1000
        );
        assert_eq!(
            midnight(parse_date("day", "2026-03-09").unwrap(), zone).unwrap()
                - midnight(parse_date("day", "2026-03-08").unwrap(), zone).unwrap(),
            23 * 60 * 60 * 1000
        );
    }
}
