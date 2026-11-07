//! Provider telemetry is a snapshot, never a delivery acknowledgment.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used_tokens: u64,
    pub window_tokens: u64,
}
impl ContextUsage {
    pub fn new(used_tokens: u64, window_tokens: u64) -> Option<Self> {
        (window_tokens > 0).then_some(Self {
            used_tokens,
            window_tokens,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllowanceUsage {
    pub used_percent: f64,
    /// Unix seconds, when the provider reports a reset time.
    pub resets_at: Option<i64>,
}
impl AllowanceUsage {
    pub fn new(used_percent: f64, resets_at: Option<i64>) -> Option<Self> {
        (used_percent.is_finite() && used_percent >= 0.0).then_some(Self {
            used_percent,
            resets_at,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreditsUsage {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub context: Option<ContextUsage>,
    pub context_updated_at: Option<i64>,
}

/// One provider allowance reading, tied to the account that reported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllowanceReading {
    pub used_percent: f64,
    /// Unix seconds, when the provider reports a reset time.
    pub resets_at: Option<i64>,
    /// Unix milliseconds when this reading was observed.
    pub observed_at: i64,
}

/// A Codex credit balance reading. A missing balance means no numeric balance was reported,
/// including unlimited accounts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreditsReading {
    pub balance: Option<f64>,
    /// Unix milliseconds when this reading was observed.
    pub observed_at: i64,
}

/// Allowance readings for one provider account. Context stays on the chat snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub provider: String,
    pub account_id: String,
    pub five_hour: Option<AllowanceReading>,
    pub weekly: Option<AllowanceReading>,
    pub credits: Option<CreditsReading>,
}

impl AccountUsage {
    /// The newest confirmation across this row's readings.
    pub fn latest_observed_at(&self) -> Option<i64> {
        [
            self.five_hour.as_ref().map(|reading| reading.observed_at),
            self.weekly.as_ref().map(|reading| reading.observed_at),
            self.credits.as_ref().map(|reading| reading.observed_at),
        ]
        .into_iter()
        .flatten()
        .max()
    }
}

pub fn claude_context(value: &Value) -> Option<ContextUsage> {
    ContextUsage::new(value["totalTokens"].as_u64()?, value["maxTokens"].as_u64()?)
}
pub fn claude_weekly(value: &Value) -> Option<AllowanceUsage> {
    claude_allowance(value, "seven_day")
}
pub fn claude_five_hour(value: &Value) -> Option<AllowanceUsage> {
    claude_allowance(value, "five_hour")
}
fn claude_allowance(value: &Value, name: &str) -> Option<AllowanceUsage> {
    if value["rate_limits_available"].as_bool() != Some(true) {
        return None;
    }
    let weekly = &value["rate_limits"][name];
    let reset = weekly["resets_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp());
    AllowanceUsage::new(weekly["utilization"].as_f64()?, reset)
}
pub fn claude_passive_weekly(value: &Value) -> Option<AllowanceUsage> {
    claude_passive_allowance(value, "seven_day")
}
pub fn claude_passive_five_hour(value: &Value) -> Option<AllowanceUsage> {
    claude_passive_allowance(value, "five_hour")
}
fn claude_passive_allowance(value: &Value, name: &str) -> Option<AllowanceUsage> {
    let all = &value["unifiedWindows"][name];
    let window = if all.is_object() {
        all
    } else if value["rateLimitType"] == name {
        value
    } else {
        return None;
    };
    AllowanceUsage::new(
        window["utilization"].as_f64()? * 100.0,
        window["resetsAt"].as_i64(),
    )
}
pub fn codex_context(value: &Value) -> Option<ContextUsage> {
    ContextUsage::new(
        value["last"]["totalTokens"].as_u64()?,
        value["modelContextWindow"].as_u64()?,
    )
}
/// A secondary window is not necessarily weekly; never substitute another product bucket.
pub fn codex_weekly(value: &Value) -> Option<AllowanceUsage> {
    codex_allowance(value, boblm_tuning::WEEKLY_USAGE_WINDOW_MINUTES)
}
pub fn codex_five_hour(value: &Value) -> Option<AllowanceUsage> {
    codex_allowance(value, boblm_tuning::FIVE_HOUR_USAGE_WINDOW_MINUTES)
}
fn codex_limits(value: &Value) -> Option<&Value> {
    let limits = if value["rateLimitsByLimitId"].is_object() {
        value["rateLimitsByLimitId"].get("codex")?
    } else if value["rateLimits"].is_object() {
        &value["rateLimits"]
    } else {
        value
    };
    if limits["limitId"].as_str().is_some_and(|id| id != "codex") {
        return None;
    }
    Some(limits)
}
fn codex_allowance(value: &Value, minutes: u64) -> Option<AllowanceUsage> {
    let limits = codex_limits(value)?;
    ["primary", "secondary"].into_iter().find_map(|name| {
        let window = &limits[name];
        if window["windowDurationMins"].as_u64() != Some(minutes) {
            return None;
        }
        AllowanceUsage::new(window["usedPercent"].as_f64()?, window["resetsAt"].as_i64())
    })
}
pub fn codex_credits(value: &Value) -> Option<CreditsUsage> {
    let credits = &codex_limits(value)?["credits"];
    Some(CreditsUsage {
        has_credits: credits["hasCredits"].as_bool()?,
        unlimited: credits["unlimited"].as_bool()?,
        balance: credits["balance"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0),
    })
}
pub fn codex_allowances(value: &Value) -> Option<crate::Event> {
    let weekly = codex_weekly(value);
    let five_hour = codex_five_hour(value);
    let credits = codex_credits(value);
    (weekly.is_some() || five_hour.is_some() || credits.is_some()).then_some(crate::Event::Usage {
        context: None,
        weekly,
        five_hour,
        credits,
    })
}
pub fn pi_context(value: &Value) -> Option<ContextUsage> {
    ContextUsage::new(value["tokens"].as_u64()?, value["contextWindow"].as_u64()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn additional_allowances_use_reported_windows_and_credit_balances() {
        let claude = json!({"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":80,"resets_at":"2030-01-01T00:00:00Z"}}});
        assert_eq!(claude_five_hour(&claude).unwrap().used_percent, 80.0);
        assert!(claude_weekly(&claude).is_none());
        assert_eq!(
            claude_passive_five_hour(&json!({"rateLimitType":"five_hour","utilization":0.8}))
                .unwrap()
                .used_percent,
            80.0
        );
        let codex = json!({"rateLimits":{"limitId":"codex","secondary":{"usedPercent":80,"windowDurationMins":300},"credits":{"hasCredits":true,"unlimited":false,"balance":"40000.25"}}});
        assert_eq!(codex_five_hour(&codex).unwrap().used_percent, 80.0);
        assert!(codex_weekly(&codex).is_none());
        assert_eq!(codex_credits(&codex).unwrap().balance, Some(40000.25));
        assert!(
            codex_five_hour(&json!({"primary":{"usedPercent":80,"windowDurationMins":60}}))
                .is_none()
        );
        assert!(codex_five_hour(&json!({"rateLimitsByLimitId":{"reviews":{"primary":{"usedPercent":80,"windowDurationMins":300}}}})).is_none());
        for balance in ["-1", "NaN", "garbage"] {
            assert!(
                codex_credits(
                    &json!({"credits":{"hasCredits":true,"unlimited":false,"balance":balance}})
                )
                .unwrap()
                .balance
                .is_none()
            );
        }
        assert!(
            !codex_credits(
                &json!({"credits":{"hasCredits":false,"unlimited":false,"balance":"0"}})
            )
            .unwrap()
            .has_credits
        );
        assert!(
            serde_json::from_value::<UsageSnapshot>(
                json!({"context":{"usedTokens":10,"windowTokens":100}})
            )
            .is_ok()
        );
        assert_eq!(
            AccountUsage {
                provider: "claude".into(),
                account_id: "a".into(),
                five_hour: None,
                weekly: Some(AllowanceReading {
                    used_percent: 42.0,
                    resets_at: None,
                    observed_at: 7,
                }),
                credits: None,
            }
            .latest_observed_at(),
            Some(7)
        );
    }
    #[test]
    fn usage_keeps_provider_units_and_uses_only_the_weekly_window() {
        assert_eq!(
            claude_context(&json!({"totalTokens":84000,"maxTokens":200000}))
                .unwrap()
                .used_tokens,
            84000
        );
        assert_eq!(claude_weekly(&json!({"rate_limits_available":true,"rate_limits":{"seven_day":{"utilization":42,"resets_at":"2026-11-02T00:00:00Z"}}})).unwrap().used_percent,42.0);
        assert_eq!(
            claude_passive_weekly(
                &json!({"unifiedWindows":{"seven_day":{"utilization":0.42,"resetsAt":42}}})
            )
            .unwrap()
            .used_percent,
            42.0
        );
        let codex = json!({"rateLimits":{"limitId":"codex","primary":{"usedPercent":98,"windowDurationMins":300},"secondary":{"usedPercent":42,"windowDurationMins":10080}}});
        assert_eq!(codex_weekly(&codex).unwrap().used_percent, 42.0);
        assert_eq!(codex_context(&json!({"last":{"inputTokens":10,"totalTokens":25},"total":{"totalTokens":9000},"modelContextWindow":100})).unwrap().used_tokens,25);
        assert!(
            codex_weekly(&json!({"secondary":{"usedPercent":42,"windowDurationMins":300}}))
                .is_none()
        );
        assert!(codex_weekly(&json!({"rateLimitsByLimitId":{"reviews":{"secondary":{"usedPercent":42,"windowDurationMins":10080}}}})).is_none());
        assert!(
            claude_passive_weekly(&json!({"rateLimitType":"five_hour","utilization":0.5}))
                .is_none()
        );
        assert!(claude_weekly(&json!({"rate_limits_available":false})).is_none());
        assert!(ContextUsage::new(1, 0).is_none());
        assert!(AllowanceUsage::new(f64::NAN, None).is_none());
        assert!(pi_context(&json!({"tokens":null,"contextWindow":100})).is_none());
    }
}
