//! Pure role routing for BobLM subagents (ADR 0043). No I/O and no clock beyond the
//! caller's `now`, so every step boundary is table-testable.
use crate::*;

/// A role a parent can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Implementer,
    Reviewer,
    Planner,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "implementer" => Some(Self::Implementer),
            "reviewer" => Some(Self::Reviewer),
            "planner" => Some(Self::Planner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
            Self::Planner => "planner",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
            Self::Planner => "planner",
        }
    }
}

/// Implementer work is either simple or hard. Other roles take no difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Simple,
    Hard,
}

impl Difficulty {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "simple" => Some(Self::Simple),
            "hard" => Some(Self::Hard),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Hard => "hard",
        }
    }
}

/// The provider account a routing step needs. B and C map these to concrete homes and keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    CodexPlan,
    ClaudePlan,
    OpenAiApi,
    ClaudeApi,
    DeepSeek,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodexPlan => "codex-plan",
            Self::ClaudePlan => "claude-plan",
            Self::OpenAiApi => "openai-api",
            Self::ClaudeApi => "claude-api",
            Self::DeepSeek => "deepseek",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CodexPlan => "Codex plan",
            Self::ClaudePlan => "Claude plan",
            Self::OpenAiApi => "OpenAI API credit",
            Self::ClaudeApi => "Claude API credit",
            Self::DeepSeek => "DeepSeek",
        }
    }
}

/// One ordered routing step. The same step id appears in only one role chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    ImplementerCodexPlan,
    ImplementerClaude,
    ImplementerCodexCredits,
    ImplementerDeepSeek,
    ReviewerOpenAiApi,
    ReviewerCodexPlan,
    ReviewerClaudeApi,
    ReviewerDeepSeek,
    PlannerClaude,
}

impl StepId {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "implementer-codex-plan" => Some(Self::ImplementerCodexPlan),
            "implementer-claude" => Some(Self::ImplementerClaude),
            "implementer-codex-credits" => Some(Self::ImplementerCodexCredits),
            "implementer-deepseek" => Some(Self::ImplementerDeepSeek),
            "reviewer-claude-api" => Some(Self::ReviewerClaudeApi),
            "reviewer-openai-api" => Some(Self::ReviewerOpenAiApi),
            "reviewer-codex-plan" => Some(Self::ReviewerCodexPlan),
            "reviewer-deepseek" => Some(Self::ReviewerDeepSeek),
            "planner-claude" => Some(Self::PlannerClaude),
            _ => None,
        }
    }

    pub fn account(self) -> AccountKind {
        match self {
            Self::ImplementerCodexPlan
            | Self::ImplementerCodexCredits
            | Self::ReviewerCodexPlan => AccountKind::CodexPlan,
            Self::ReviewerClaudeApi => AccountKind::ClaudeApi,
            Self::ImplementerClaude | Self::PlannerClaude => AccountKind::ClaudePlan,
            Self::ImplementerDeepSeek | Self::ReviewerDeepSeek => AccountKind::DeepSeek,
            Self::ReviewerOpenAiApi => AccountKind::OpenAiApi,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImplementerCodexPlan => "implementer-codex-plan",
            Self::ImplementerClaude => "implementer-claude",
            Self::ImplementerCodexCredits => "implementer-codex-credits",
            Self::ImplementerDeepSeek => "implementer-deepseek",
            Self::ReviewerClaudeApi => "reviewer-claude-api",
            Self::ReviewerOpenAiApi => "reviewer-openai-api",
            Self::ReviewerCodexPlan => "reviewer-codex-plan",
            Self::ReviewerDeepSeek => "reviewer-deepseek",
            Self::PlannerClaude => "planner-claude",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ImplementerCodexPlan | Self::ReviewerCodexPlan => "Codex plan allowance",
            Self::ImplementerClaude => "Claude plan allowance",
            Self::ImplementerCodexCredits => "Codex plan credits",
            Self::ImplementerDeepSeek => "DeepSeek fallback",
            Self::ReviewerClaudeApi => "Claude API credit",
            Self::ReviewerOpenAiApi => "OpenAI API credit",
            Self::ReviewerDeepSeek => "DeepSeek fallback",
            Self::PlannerClaude => "Claude plan allowance",
        }
    }

    /// Whether the implementation for a routing step is available.
    fn configured(self) -> bool {
        matches!(
            self,
            Self::ImplementerCodexPlan
                | Self::ImplementerClaude
                | Self::ImplementerCodexCredits
                | Self::ImplementerDeepSeek
                | Self::ReviewerOpenAiApi
                | Self::ReviewerCodexPlan
                | Self::ReviewerClaudeApi
                | Self::ReviewerDeepSeek
                | Self::PlannerClaude
        )
    }
}

pub struct RoleStep {
    pub id: StepId,
    pub model: &'static str,
    pub simple_effort: &'static str,
    pub hard_effort: &'static str,
    pub account: AccountKind,
}

/// Ordered implementer models and accounts.
pub const IMPLEMENTER_STEPS: &[RoleStep] = &[
    RoleStep {
        id: StepId::ImplementerCodexPlan,
        model: MODEL_GPT_6_1_SOL,
        simple_effort: ROLE_IMPLEMENTER_SIMPLE_EFFORT,
        hard_effort: ROLE_IMPLEMENTER_HARD_EFFORT,
        account: AccountKind::CodexPlan,
    },
    RoleStep {
        id: StepId::ImplementerCodexCredits,
        model: MODEL_GPT_6_1_SOL,
        simple_effort: ROLE_IMPLEMENTER_SIMPLE_EFFORT,
        hard_effort: ROLE_IMPLEMENTER_HARD_EFFORT,
        account: AccountKind::CodexPlan,
    },
    RoleStep {
        id: StepId::ImplementerClaude,
        model: MODEL_SONNET,
        simple_effort: ROLE_IMPLEMENTER_SIMPLE_EFFORT,
        hard_effort: ROLE_IMPLEMENTER_HARD_EFFORT,
        account: AccountKind::ClaudePlan,
    },
    RoleStep {
        id: StepId::ImplementerDeepSeek,
        model: MODEL_DEEPSEEK_V41_FLASH,
        simple_effort: ROLE_IMPLEMENTER_SIMPLE_EFFORT,
        hard_effort: ROLE_IMPLEMENTER_HARD_EFFORT,
        account: AccountKind::DeepSeek,
    },
];

/// Ordered reviewer models and accounts.
pub const REVIEWER_STEPS: &[RoleStep] = &[
    RoleStep {
        id: StepId::ReviewerOpenAiApi,
        model: MODEL_GPT_6_LUNA,
        simple_effort: ROLE_REVIEWER_LUNA_EFFORT,
        hard_effort: ROLE_REVIEWER_LUNA_EFFORT,
        account: AccountKind::OpenAiApi,
    },
    RoleStep {
        id: StepId::ReviewerCodexPlan,
        model: MODEL_GPT_6_LUNA,
        simple_effort: ROLE_REVIEWER_LUNA_EFFORT,
        hard_effort: ROLE_REVIEWER_LUNA_EFFORT,
        account: AccountKind::CodexPlan,
    },
    RoleStep {
        id: StepId::ReviewerClaudeApi,
        model: MODEL_HAIKU,
        simple_effort: ROLE_REVIEWER_HAIKU_EFFORT,
        hard_effort: ROLE_REVIEWER_HAIKU_EFFORT,
        account: AccountKind::ClaudeApi,
    },
    RoleStep {
        id: StepId::ReviewerDeepSeek,
        model: MODEL_DEEPSEEK_V41_FLASH,
        simple_effort: ROLE_REVIEWER_DEEPSEEK_EFFORT,
        hard_effort: ROLE_REVIEWER_DEEPSEEK_EFFORT,
        account: AccountKind::DeepSeek,
    },
];

/// Planning uses the Claude allowance with no fallback.
pub const PLANNER_STEPS: &[RoleStep] = &[RoleStep {
    id: StepId::PlannerClaude,
    model: MODEL_OPUS,
    simple_effort: ROLE_PLANNER_EFFORT,
    hard_effort: ROLE_PLANNER_EFFORT,
    account: AccountKind::ClaudePlan,
}];

pub fn steps(role: Role) -> &'static [RoleStep] {
    match role {
        Role::Implementer => IMPLEMENTER_STEPS,
        Role::Reviewer => REVIEWER_STEPS,
        Role::Planner => PLANNER_STEPS,
    }
}

/// One account's current allowance readings. Missing readings count as available for plan
/// accounts; a reading past its reset counts as available.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountReading {
    pub account: AccountKind,
    pub weekly_used_percent: Option<f64>,
    pub weekly_resets_at: Option<i64>,
    pub credits_balance: Option<f64>,
}

/// Measured API load, supplied by the caller to keep routing free of I/O.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApiHeadroom {
    pub recent_tokens: u64,
    pub projected_tokens: u64,
    pub active_reviewers: usize,
}

impl ApiHeadroom {
    fn reason(self) -> Result<String, String> {
        let total = self.recent_tokens.saturating_add(self.projected_tokens);
        let budget = OPENAI_API_TPM_LIMIT as f64 * OPENAI_API_TPM_BUDGET;
        if total as f64 >= budget {
            return Err(format!(
                "OpenAI API used {} tokens in the last minute plus {} projected tokens, about {:.1}M of its {:.1}M-token spawn budget. Wait for API headroom or use the next reviewer step.",
                self.recent_tokens,
                self.projected_tokens,
                total as f64 / 1_000_000.0,
                budget / 1_000_000.0
            ));
        }
        if self.active_reviewers >= OPENAI_API_MAX_ACTIVE_REVIEWERS {
            return Err(format!(
                "OpenAI API has {} active reviewers, at or above its cap of {}. Wait for a reviewer to become idle or use the next reviewer step.",
                self.active_reviewers, OPENAI_API_MAX_ACTIVE_REVIEWERS
            ));
        }
        Ok(format!(
            "OpenAI API has {} recent tokens plus {} projected tokens below its {:.1}M-token spawn budget, and {} active reviewers below its cap of {}.",
            self.recent_tokens,
            self.projected_tokens,
            budget / 1_000_000.0,
            self.active_reviewers,
            OPENAI_API_MAX_ACTIVE_REVIEWERS
        ))
    }
}

/// Recognize a provider tokens-per-minute rejection, excluding request-rate limits.
pub fn is_api_tpm_rate_limit(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("rate limit") && text.contains("tokens per min") && text.contains("(tpm)")
}

/// Claude plan limits, as reported in provider error envelopes. Ordinary rate limiting
/// and assistant prose must never reach this classifier as a provider failure.
pub fn is_claude_usage_limit(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "you've hit your limit",
        "you have hit your limit",
        "usage limit",
        "weekly limit",
        "weekly allowance",
        "rate_limit_error",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// A step a provider quota or billing rejection marked exhausted until this Unix second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExhaustedStep {
    pub step: StepId,
    pub until: i64,
}

/// Whether a provider error is a quota or billing rejection rather than a rate limit.
pub fn is_provider_exhaustion(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let explicit = [
        "insufficient_quota",
        "insufficient quota",
        "exceeded your current quota",
        "quota exceeded",
        "quota has been exceeded",
        "out of quota",
        "monthly quota",
        "billing hard limit",
        "billing_hard_limit",
        "insufficient balance",
        "insufficient_balance",
        "insufficient credits",
        "not enough credits",
        "out of credits",
        "credit balance is too low",
        "credit limit reached",
        "payment required",
        "status 402",
        "http 402",
    ]
    .iter()
    .any(|needle| text.contains(needle));
    explicit || (text.contains("quota") && !text.contains("rate limit"))
}

/// When the OpenAI API credit next resets: the start of OPENAI_API_CREDIT_RESET_DAY (UTC),
/// or the month's last day when the month is shorter.
pub fn openai_api_reset_at(now: i64) -> i64 {
    credit_reset_at(now, OPENAI_API_CREDIT_RESET_DAY)
}

/// Next Claude Platform API credit renewal, in UTC.
pub fn claude_api_reset_at(now: i64) -> i64 {
    credit_reset_at(now, ANTHROPIC_API_CREDIT_RESET_DAY)
}

fn credit_reset_at(now: i64, reset_day: u32) -> i64 {
    let days = now.div_euclid(86_400);
    let (mut year, mut month, _) = civil_from_days(days);
    let day = reset_day.min(days_in_month(year, month));
    let mut reset = days_from_civil(year, month, day) * 86_400;
    if reset <= now {
        month += 1;
        if month == 13 {
            month = 1;
            year += 1;
        }
        reset = days_from_civil(year, month, reset_day.min(days_in_month(year, month))) * 86_400;
    }
    reset
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Days since 1970-01-01 from a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * month_index + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of `days_from_civil`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChosenRoute {
    pub account_key: Option<&'static str>,
    pub role: Role,
    pub difficulty: Difficulty,
    pub step: StepId,
    pub model: &'static str,
    pub effort: &'static str,
    pub account: AccountKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepRejection {
    pub step: StepId,
    pub model: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteOutcome {
    Chosen(ChosenRoute),
    Exhausted(Vec<StepRejection>),
}

/// Resolve a role to one model, effort and account, or explain why every step is unavailable.
pub fn resolve(
    role: Role,
    difficulty: Difficulty,
    readings: &[AccountReading],
    exhausted: &[ExhaustedStep],
    headroom: ApiHeadroom,
    now: i64,
) -> RouteOutcome {
    resolve_with(
        role,
        difficulty,
        readings,
        exhausted,
        headroom,
        now,
        (
            StepId::configured,
            CODEX_PLAN_ROUTING_ENABLED,
            OPENAI_API_ROUTING_ENABLED,
        ),
    )
}

fn resolve_with(
    role: Role,
    difficulty: Difficulty,
    readings: &[AccountReading],
    exhausted: &[ExhaustedStep],
    headroom: ApiHeadroom,
    now: i64,
    (configured, codex_enabled, openai_enabled): (impl Fn(StepId) -> bool, bool, bool),
) -> RouteOutcome {
    let mut rejections = Vec::new();
    for step in steps(role) {
        let disabled = match step.id {
            StepId::ImplementerCodexPlan
            | StepId::ImplementerCodexCredits
            | StepId::ReviewerCodexPlan
                if !codex_enabled =>
            {
                Some("CODEX_PLAN_ROUTING_ENABLED")
            }
            StepId::ReviewerOpenAiApi if !openai_enabled => Some("OPENAI_API_ROUTING_ENABLED"),
            _ => None,
        };
        if let Some(flag) = disabled {
            rejections.push(StepRejection {
                step: step.id,
                model: step.model,
                reason: format!("The {} step is disabled by {flag}.", step.id.label()),
            });
            continue;
        }
        let effort = if role != Role::Implementer {
            step.hard_effort
        } else {
            match difficulty {
                Difficulty::Simple => step.simple_effort,
                Difficulty::Hard => step.hard_effort,
            }
        };
        if !configured(step.id) {
            rejections.push(StepRejection {
                step: step.id,
                model: step.model,
                reason: format!("The {} step is not configured yet.", step.id.label()),
            });
            continue;
        }
        if let Some(mark) = exhausted
            .iter()
            .find(|mark| mark.step == step.id && mark.until > now)
        {
            rejections.push(StepRejection {
                step: step.id,
                model: step.model,
                reason: format!(
                    "The {} step is marked exhausted until Unix time {}.",
                    step.id.label(),
                    mark.until
                ),
            });
            continue;
        }
        let reading = readings
            .iter()
            .find(|reading| reading.account == step.account);
        let reason = match step.id {
            StepId::ImplementerCodexPlan
            | StepId::ReviewerCodexPlan
            | StepId::ImplementerClaude
            | StepId::PlannerClaude => weekly_plan_reason(step.account, reading, now),
            StepId::ImplementerCodexCredits => codex_credits_reason(reading),
            StepId::ReviewerOpenAiApi => headroom.reason(),
            StepId::ImplementerDeepSeek | StepId::ReviewerDeepSeek => Ok(format!(
                "{} is the last step and has no allowance window.",
                step.id.label()
            )),
            StepId::ReviewerClaudeApi => {
                Ok("Claude API credit has no active quota or billing exhaustion mark.".into())
            }
        };
        match reason {
            Ok(reason) => {
                let reason = rejections
                    .iter()
                    .map(|rejection| rejection.reason.as_str())
                    .chain(std::iter::once(reason.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ");
                return RouteOutcome::Chosen(ChosenRoute {
                    account_key: None,
                    role,
                    difficulty,
                    step: step.id,
                    model: step.model,
                    effort,
                    account: step.account,
                    reason,
                });
            }
            Err(reason) => rejections.push(StepRejection {
                step: step.id,
                model: step.model,
                reason,
            }),
        }
    }
    RouteOutcome::Exhausted(rejections)
}

fn weekly_plan_reason(
    account: AccountKind,
    reading: Option<&AccountReading>,
    now: i64,
) -> Result<String, String> {
    let label = account.label();
    let Some(reading) = reading else {
        return Ok(format!(
            "The {label} has no reading, which counts as available."
        ));
    };
    if reading.weekly_resets_at.is_some_and(|reset| reset <= now) {
        return Ok(format!(
            "The {label} reading is past its reset, which counts as available."
        ));
    }
    // A Claude account keeps its last few percent for the chats already running on it.
    let limit = if account == AccountKind::ClaudePlan {
        100.0 - CLAUDE_ACCOUNT_CUTOFF_PERCENT
    } else {
        100.0
    };
    match reading.weekly_used_percent {
        None => Ok(format!(
            "The {label} has no weekly reading, which counts as available."
        )),
        Some(used) if used < limit => {
            Ok(format!("The {label} weekly allowance is {used:.1}% used."))
        }
        Some(used) if used < 100.0 => Err(format!(
            "The {label} weekly allowance is {used:.1}% used, past the {CLAUDE_ACCOUNT_CUTOFF_PERCENT}% kept for chats already running on it."
        )),
        Some(used) => Err(format!(
            "The {label} weekly allowance is {used:.1}% used and has not reset."
        )),
    }
}

fn codex_credits_reason(reading: Option<&AccountReading>) -> Result<String, String> {
    match reading.and_then(|reading| reading.credits_balance) {
        Some(balance) if balance > 0.0 => {
            Ok(format!("The Codex plan credit balance is {balance:.2}."))
        }
        Some(balance) => Err(format!(
            "The Codex plan credit balance is {balance:.2}, which is not positive."
        )),
        None => Err("The Codex plan credit balance has not been reported.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_configured(_: StepId) -> bool {
        true
    }

    fn reading(
        account: AccountKind,
        used: Option<f64>,
        reset: Option<i64>,
        credits: Option<f64>,
    ) -> AccountReading {
        AccountReading {
            account,
            weekly_used_percent: used,
            weekly_resets_at: reset,
            credits_balance: credits,
        }
    }

    #[test]
    fn routing_chains_walk_every_available_exhausted_and_disabled_step() {
        let now = 1000;
        let readings = [reading(
            AccountKind::CodexPlan,
            Some(0.0),
            Some(2000),
            Some(10.0),
        )];
        for (role, expected) in [
            (
                Role::Implementer,
                vec![
                    StepId::ImplementerCodexPlan,
                    StepId::ImplementerCodexCredits,
                    StepId::ImplementerClaude,
                    StepId::ImplementerDeepSeek,
                ],
            ),
            (
                Role::Reviewer,
                vec![
                    StepId::ReviewerOpenAiApi,
                    StepId::ReviewerCodexPlan,
                    StepId::ReviewerClaudeApi,
                    StepId::ReviewerDeepSeek,
                ],
            ),
        ] {
            assert_eq!(
                steps(role).iter().map(|step| step.id).collect::<Vec<_>>(),
                expected
            );
            for (codex, openai) in [(true, true), (false, true), (true, false), (false, false)] {
                let enabled = expected
                    .iter()
                    .copied()
                    .filter(|step| match step {
                        StepId::ImplementerCodexPlan
                        | StepId::ImplementerCodexCredits
                        | StepId::ReviewerCodexPlan => codex,
                        StepId::ReviewerOpenAiApi => openai,
                        _ => true,
                    })
                    .collect::<Vec<_>>();
                let mut marks = Vec::new();
                for step in enabled {
                    for difficulty in [Difficulty::Simple, Difficulty::Hard] {
                        let RouteOutcome::Chosen(route) = resolve_with(
                            role,
                            difficulty,
                            &readings,
                            &marks,
                            ApiHeadroom::default(),
                            now,
                            (all_configured, codex, openai),
                        ) else {
                            panic!()
                        };
                        assert_eq!(route.step, step);
                        assert_eq!(route.account, step.account());
                        if role == Role::Implementer {
                            assert_eq!(
                                route.effort,
                                if difficulty == Difficulty::Simple {
                                    ROLE_IMPLEMENTER_SIMPLE_EFFORT
                                } else {
                                    ROLE_IMPLEMENTER_HARD_EFFORT
                                }
                            );
                        }
                        if !codex && role == Role::Implementer {
                            assert_eq!(
                                route
                                    .reason
                                    .matches("disabled by CODEX_PLAN_ROUTING_ENABLED")
                                    .count(),
                                2
                            );
                        }
                        if !openai && role == Role::Reviewer {
                            assert!(
                                route
                                    .reason
                                    .contains("disabled by OPENAI_API_ROUTING_ENABLED")
                            );
                        }
                    }
                    marks.push(ExhaustedStep {
                        step,
                        until: now + 10,
                    });
                }
                let RouteOutcome::Exhausted(reasons) = resolve_with(
                    role,
                    Difficulty::Simple,
                    &readings,
                    &marks,
                    ApiHeadroom::default(),
                    now,
                    (all_configured, codex, openai),
                ) else {
                    panic!()
                };
                assert_eq!(reasons.len(), expected.len());
            }
        }
        assert_eq!(Role::parse("grunt"), None);
    }

    #[test]
    fn planner_allowance_table() {
        for (used, reset, available) in [
            (0.0, 2000, true),
            (97.9, 2000, true),
            (98.0, 2000, false),
            (98.5, 2000, false),
            (100.0, 2000, false),
            (100.0, 1000, true),
        ] {
            let result = resolve(
                Role::Planner,
                Difficulty::Simple,
                &[reading(
                    AccountKind::ClaudePlan,
                    Some(used),
                    Some(reset),
                    None,
                )],
                &[],
                ApiHeadroom::default(),
                1000,
            );
            match result {
                RouteOutcome::Chosen(route) => {
                    assert!(available);
                    assert_eq!(route.model, MODEL_OPUS);
                    assert_eq!(route.effort, ROLE_PLANNER_EFFORT);
                    assert_eq!(route.account, AccountKind::ClaudePlan);
                    assert_eq!(route.step, StepId::PlannerClaude);
                }
                RouteOutcome::Exhausted(rejections) => {
                    assert!(!available);
                    assert_eq!(rejections.len(), 1);
                }
            }
        }
    }

    #[test]
    fn every_role_model_and_effort_exists_in_the_catalog() {
        for step in IMPLEMENTER_STEPS
            .iter()
            .chain(REVIEWER_STEPS)
            .chain(PLANNER_STEPS)
        {
            let model = MODELS
                .iter()
                .find(|model| model.id == step.model)
                .unwrap_or_else(|| panic!("{} is not in MODELS", step.model));
            assert!(
                model.efforts.contains(&step.simple_effort),
                "{} does not accept {}",
                step.model,
                step.simple_effort
            );
            assert!(
                model.efforts.contains(&step.hard_effort),
                "{} does not accept {}",
                step.model,
                step.hard_effort
            );
        }
    }

    #[test]
    fn every_configured_chain_starts_at_its_first_step() {
        let chosen = resolve(
            Role::Implementer,
            Difficulty::Simple,
            &[],
            &[],
            ApiHeadroom::default(),
            1_000,
        );
        let RouteOutcome::Chosen(chosen) = chosen else {
            panic!("{chosen:?}")
        };
        assert_eq!(chosen.step, StepId::ImplementerCodexPlan);
        assert_eq!(chosen.model, MODEL_GPT_6_1_SOL);
        assert_eq!(chosen.effort, ROLE_IMPLEMENTER_SIMPLE_EFFORT);

        let chosen = resolve(
            Role::Implementer,
            Difficulty::Hard,
            &[],
            &[],
            ApiHeadroom::default(),
            1_000,
        );
        let RouteOutcome::Chosen(chosen) = chosen else {
            panic!("{chosen:?}")
        };
        assert_eq!(chosen.effort, ROLE_IMPLEMENTER_HARD_EFFORT);

        let chosen = resolve(
            Role::Reviewer,
            Difficulty::Simple,
            &[],
            &[],
            ApiHeadroom::default(),
            1_000,
        );
        let RouteOutcome::Chosen(chosen) = chosen else {
            panic!("{chosen:?}")
        };
        assert_eq!(chosen.step, StepId::ReviewerOpenAiApi);
        assert_eq!(chosen.model, MODEL_GPT_6_LUNA);
        assert_eq!(chosen.effort, ROLE_REVIEWER_LUNA_EFFORT);
    }

    #[test]
    fn sonnet_implementers_drain_claude_to_the_cutoff() {
        let now = 1_000_000;
        let configured = |step| {
            matches!(
                step,
                StepId::ImplementerClaude | StepId::ImplementerDeepSeek
            )
        };
        // No reserve: days to reset never matter, only the 2% cutoff.
        for (used, days, expected) in [
            (10.0, 6.0, StepId::ImplementerClaude),
            (97.9, 6.0, StepId::ImplementerClaude),
            (97.9, 0.1, StepId::ImplementerClaude),
            (98.5, 6.0, StepId::ImplementerDeepSeek),
        ] {
            let readings = [reading(
                AccountKind::ClaudePlan,
                Some(used),
                Some(now + (days * 86_400.0) as i64),
                None,
            )];
            let RouteOutcome::Chosen(chosen) = resolve_with(
                Role::Implementer,
                Difficulty::Simple,
                &readings,
                &[],
                ApiHeadroom::default(),
                now,
                (
                    configured,
                    CODEX_PLAN_ROUTING_ENABLED,
                    OPENAI_API_ROUTING_ENABLED,
                ),
            ) else {
                panic!()
            };
            assert_eq!(chosen.step, expected, "{used}% used, {days} days left");
        }
    }

    #[test]
    fn codex_plan_credits_and_api_steps_are_ordered() {
        let now = 1_000_000;
        let reset = now + 86_400;
        let readings = [reading(
            AccountKind::CodexPlan,
            Some(99.0),
            Some(reset),
            Some(10.0),
        )];
        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Implementer,
            Difficulty::Hard,
            &readings,
            &[],
            ApiHeadroom::default(),
            now,
            (
                all_configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ImplementerCodexPlan);

        let readings = [
            reading(AccountKind::CodexPlan, Some(100.0), Some(reset), Some(10.0)),
            reading(AccountKind::ClaudePlan, Some(100.0), Some(reset), None),
        ];
        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Implementer,
            Difficulty::Hard,
            &readings,
            &[],
            ApiHeadroom::default(),
            now,
            (
                all_configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ImplementerCodexCredits);

        let readings = [
            reading(AccountKind::CodexPlan, Some(100.0), Some(reset), Some(0.0)),
            reading(AccountKind::ClaudePlan, Some(100.0), Some(reset), None),
        ];
        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Implementer,
            Difficulty::Hard,
            &readings,
            &[],
            ApiHeadroom::default(),
            now,
            (
                all_configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ImplementerDeepSeek);

        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Reviewer,
            Difficulty::Simple,
            &[],
            &[],
            ApiHeadroom::default(),
            now,
            (
                all_configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ReviewerOpenAiApi);
        assert_eq!(chosen.model, MODEL_GPT_6_LUNA);
        assert_eq!(chosen.effort, ROLE_REVIEWER_LUNA_EFFORT);
    }

    #[test]
    fn exhausted_marks_skip_until_their_reset_and_past_marks_are_ignored() {
        let now = 1_000_000;
        let configured = |step| {
            matches!(
                step,
                StepId::ImplementerClaude | StepId::ImplementerDeepSeek
            )
        };
        let marks = [ExhaustedStep {
            step: StepId::ImplementerClaude,
            until: now + 10,
        }];
        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Implementer,
            Difficulty::Simple,
            &[],
            &marks,
            ApiHeadroom::default(),
            now,
            (
                configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ImplementerDeepSeek);

        let marks = [ExhaustedStep {
            step: StepId::ImplementerClaude,
            until: now - 1,
        }];
        let RouteOutcome::Chosen(chosen) = resolve_with(
            Role::Implementer,
            Difficulty::Simple,
            &[],
            &marks,
            ApiHeadroom::default(),
            now,
            (
                configured,
                CODEX_PLAN_ROUTING_ENABLED,
                OPENAI_API_ROUTING_ENABLED,
            ),
        ) else {
            panic!()
        };
        assert_eq!(chosen.step, StepId::ImplementerClaude);

        let marks = [
            ExhaustedStep {
                step: StepId::ImplementerCodexPlan,
                until: now + 10,
            },
            ExhaustedStep {
                step: StepId::ImplementerCodexCredits,
                until: now + 10,
            },
            ExhaustedStep {
                step: StepId::ImplementerClaude,
                until: now + 10,
            },
            ExhaustedStep {
                step: StepId::ImplementerDeepSeek,
                until: now + 10,
            },
        ];
        let RouteOutcome::Exhausted(rejections) = resolve(
            Role::Implementer,
            Difficulty::Simple,
            &[],
            &marks,
            ApiHeadroom::default(),
            now,
        ) else {
            panic!()
        };
        assert!(rejections.iter().any(|rejection| {
            rejection.step == StepId::ImplementerDeepSeek
                && rejection.reason.contains("marked exhausted")
        }));
        assert!(rejections.iter().any(|rejection| {
            rejection.step == StepId::ImplementerClaude
                && rejection.reason.contains("marked exhausted")
        }));
    }

    #[test]
    fn reviewer_headroom_falls_back_to_codex_plan() {
        let budget = (OPENAI_API_TPM_LIMIT as f64 * OPENAI_API_TPM_BUDGET) as u64;
        for (tokens, projected, active, expected) in [
            (budget - 1, 0, 0, StepId::ReviewerOpenAiApi),
            (budget, 0, 0, StepId::ReviewerCodexPlan),
            (budget - 1, 1, 0, StepId::ReviewerCodexPlan),
            (
                0,
                0,
                OPENAI_API_MAX_ACTIVE_REVIEWERS - 1,
                StepId::ReviewerOpenAiApi,
            ),
            (
                0,
                0,
                OPENAI_API_MAX_ACTIVE_REVIEWERS,
                StepId::ReviewerCodexPlan,
            ),
        ] {
            let RouteOutcome::Chosen(route) = resolve(
                Role::Reviewer,
                Difficulty::Simple,
                &[],
                &[],
                ApiHeadroom {
                    recent_tokens: tokens,
                    projected_tokens: projected,
                    active_reviewers: active,
                },
                1000,
            ) else {
                panic!()
            };
            assert_eq!(route.step, expected);
            if expected == StepId::ReviewerCodexPlan {
                assert_eq!(route.model, MODEL_GPT_6_LUNA);
                assert_eq!(route.effort, ROLE_REVIEWER_LUNA_EFFORT);
                assert_eq!(route.account, AccountKind::CodexPlan);
            }
        }
    }

    #[test]
    fn reviewer_plan_fallback_obeys_allowance_marks_and_flags() {
        let now = 1000;
        for (codex, openai, used, marked, haiku_marked, expected) in [
            (true, false, 99.9, false, false, StepId::ReviewerCodexPlan),
            (true, true, 99.9, false, false, StepId::ReviewerCodexPlan),
            (true, true, 100.0, false, false, StepId::ReviewerClaudeApi),
            (false, true, 0.0, false, false, StepId::ReviewerClaudeApi),
            (true, true, 0.0, true, false, StepId::ReviewerClaudeApi),
            (true, true, 100.0, false, true, StepId::ReviewerDeepSeek),
        ] {
            let mut marks = vec![ExhaustedStep {
                step: StepId::ReviewerOpenAiApi,
                until: now + 120,
            }];
            if marked {
                marks.push(ExhaustedStep {
                    step: StepId::ReviewerCodexPlan,
                    until: now + 100,
                });
            }
            if haiku_marked {
                marks.push(ExhaustedStep {
                    step: StepId::ReviewerClaudeApi,
                    until: now + 100,
                });
            }
            let RouteOutcome::Chosen(route) = resolve_with(
                Role::Reviewer,
                Difficulty::Simple,
                &[reading(
                    AccountKind::CodexPlan,
                    Some(used),
                    Some(now + 100),
                    None,
                )],
                &marks,
                ApiHeadroom::default(),
                now,
                (StepId::configured, codex, openai),
            ) else {
                panic!()
            };
            assert_eq!(route.step, expected);
            if !codex {
                assert!(
                    route
                        .reason
                        .contains("disabled by CODEX_PLAN_ROUTING_ENABLED")
                );
            }
        }
    }

    #[test]
    fn api_tpm_classifier_accepts_the_provider_error_and_excludes_other_limits() {
        assert!(is_api_tpm_rate_limit(
            "Rate limit reached for gpt-6-luna in organization org-example on tokens per min (TPM): Limit 2000000, Used 1800000, Requested 240000. Please try again in 1.2s."
        ));
        for text in [
            "Rate limit reached on requests per min (RPM)",
            "429 Too Many Requests",
            "insufficient_quota",
            "Review the tokens per min (TPM) configuration",
        ] {
            assert!(!is_api_tpm_rate_limit(text), "{text}");
        }
    }

    #[test]
    fn provider_exhaustion_does_not_treat_a_rate_limit_as_a_quota_rejection() {
        assert!(is_provider_exhaustion(
            "You exceeded your current quota, please check your plan and billing details"
        ));
        assert!(is_provider_exhaustion("insufficient_quota"));
        assert!(is_provider_exhaustion("402 Payment Required"));
        assert!(is_provider_exhaustion("The credit balance is too low"));
        assert!(!is_provider_exhaustion("429 Too Many Requests"));
        assert!(!is_provider_exhaustion("rate limit reached for requests"));
    }

    #[test]
    fn api_reset_uses_the_configured_day_of_the_next_month() {
        assert_eq!(crate::OPENAI_API_CREDIT_RESET_DAY, 1);
        assert_eq!(crate::ANTHROPIC_API_CREDIT_RESET_DAY, 1);
        assert_eq!(
            claude_api_reset_at(days_from_civil(2026, 12, 1) * 86_400),
            days_from_civil(2027, 1, 1) * 86_400
        );
        let noon = 12 * 3_600;
        let late_november = days_from_civil(2026, 11, 29) * 86_400 + noon;
        assert_eq!(
            openai_api_reset_at(late_november),
            days_from_civil(2026, 12, 1) * 86_400
        );
        let on_reset_day = days_from_civil(2026, 12, 1) * 86_400;
        assert_eq!(
            openai_api_reset_at(on_reset_day),
            days_from_civil(2027, 1, 1) * 86_400
        );
        let year_end = days_from_civil(2026, 12, 31) * 86_400 + noon;
        assert_eq!(
            openai_api_reset_at(year_end),
            days_from_civil(2027, 1, 1) * 86_400
        );
    }
}

#[cfg(test)]
mod claude_limit_tests {
    use super::*;
    #[test]
    fn claude_usage_limit_errors_are_recognized() {
        assert!(is_claude_usage_limit(
            "Claude reported an error: You've hit your limit. Resets Nov 10."
        ));
        assert!(!is_claude_usage_limit(
            "Claude reported an error: server overloaded"
        ));
        assert!(!is_claude_usage_limit(
            "Claude reported an error: context limit reached"
        ));
        assert!(!is_claude_usage_limit(
            "Claude reported an error: rate limit reached for requests"
        ));
    }
}
