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
    /// Exists only to read old receipts and stored sessions. Routing never produces it.
    ImplementerCodexCredits,
    /// Exists only to read old receipts and stored sessions. Routing never produces it.
    ImplementerDeepSeek,
    ReviewerOpenAiApi,
    /// Exists only to read old receipts and stored sessions. Routing never produces it.
    ReviewerCodexPlan,
    ReviewerClaudeApi,
    /// Exists only to read old receipts and stored sessions. Routing never produces it.
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
                | Self::ReviewerOpenAiApi
                | Self::ReviewerClaudeApi
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
        id: StepId::ImplementerClaude,
        model: MODEL_SONNET,
        simple_effort: ROLE_IMPLEMENTER_SIMPLE_EFFORT,
        hard_effort: ROLE_IMPLEMENTER_HARD_EFFORT,
        account: AccountKind::ClaudePlan,
    },
];

/// Ordered reviewer models and accounts. No reviewer fallback exists after API money is spent.
pub const REVIEWER_STEPS: &[RoleStep] = &[
    RoleStep {
        id: StepId::ReviewerOpenAiApi,
        model: MODEL_GPT_6_LUNA,
        simple_effort: ROLE_REVIEWER_LUNA_EFFORT,
        hard_effort: ROLE_REVIEWER_LUNA_EFFORT,
        account: AccountKind::OpenAiApi,
    },
    RoleStep {
        id: StepId::ReviewerClaudeApi,
        model: MODEL_HAIKU,
        simple_effort: ROLE_REVIEWER_HAIKU_EFFORT,
        hard_effort: ROLE_REVIEWER_HAIKU_EFFORT,
        account: AccountKind::ClaudeApi,
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
}

/// A provider billing account with its own open-chat and reviewer caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderGroup {
    CodexPlan,
    ClaudePlanA,
    ClaudePlanB,
    LunaApi,
    HaikuApiA,
    HaikuApiB,
    DsfApi,
}

impl ProviderGroup {
    pub const ALL: [Self; 7] = [
        Self::CodexPlan,
        Self::ClaudePlanA,
        Self::ClaudePlanB,
        Self::LunaApi,
        Self::HaikuApiA,
        Self::HaikuApiB,
        Self::DsfApi,
    ];

    pub fn account(self) -> AccountKind {
        match self {
            Self::CodexPlan => AccountKind::CodexPlan,
            Self::ClaudePlanA | Self::ClaudePlanB => AccountKind::ClaudePlan,
            Self::LunaApi => AccountKind::OpenAiApi,
            Self::HaikuApiA | Self::HaikuApiB => AccountKind::ClaudeApi,
            Self::DsfApi => AccountKind::DeepSeek,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CodexPlan => "The Codex plan group",
            Self::ClaudePlanA => "Claude plan account A",
            Self::ClaudePlanB => "Claude plan account B",
            Self::LunaApi => "The Luna API group",
            Self::HaikuApiA => "Haiku API account A",
            Self::HaikuApiB => "Haiku API account B",
            Self::DsfApi => "The DSF API group",
        }
    }

    pub fn cap(self) -> usize {
        match self {
            Self::CodexPlan => MAX_OPEN_CODEX_PLAN_AGENTS,
            Self::ClaudePlanA | Self::ClaudePlanB => MAX_OPEN_CLAUDE_PLAN_AGENTS,
            Self::LunaApi => OPENAI_API_MAX_ACTIVE_REVIEWERS,
            Self::HaikuApiA | Self::HaikuApiB => MAX_OPEN_HAIKU_API_AGENTS,
            Self::DsfApi => MAX_OPEN_DSF_API_AGENTS,
        }
    }
}

/// Open sessions by their current provider account. Enabled lists affect routing only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCounts {
    pub open: [usize; ProviderGroup::ALL.len()],
    pub reviewers: [usize; ProviderGroup::ALL.len()],
    pub enabled_plan: [bool; 2],
    pub enabled_api: [bool; 2],
    pub unavailable_plan: [bool; 2],
    pub unavailable_api: [bool; 2],
}

impl Default for ProviderCounts {
    fn default() -> Self {
        Self {
            open: [0; ProviderGroup::ALL.len()],
            reviewers: [0; ProviderGroup::ALL.len()],
            enabled_plan: [true, false],
            enabled_api: [true, false],
            unavailable_plan: [false; 2],
            unavailable_api: [false; 2],
        }
    }
}

/// Added once to a refusal caused by a full provider cap, so the agent knows how to free a slot.
pub const CAP_ADVICE: &str = "To free a slot, use list_peers and inspect_peer to find chats that have sat idle for a long time, and subagents that have stopped responding and may have fallen over (a failed or stuck turn, no recent events). If one's work is finished, can be resumed later, or is lost to a failure, save any progress worth keeping to durable storage first (a file in the repository, a note or a tracker item), then archive it with archive_peer. Archiving stops a chat, so never archive one that is still working normally, and use ask to check with the owner when you are not sure.";

/// Whether a step rejection came from a full provider cap rather than credit or allowance.
pub fn is_cap_reason(reason: &str) -> bool {
    reason.contains(" open across ")
}

impl ProviderCounts {
    pub fn add(&mut self, group: ProviderGroup) {
        self.open[group as usize] += 1;
    }
    pub fn add_reviewer(&mut self, group: ProviderGroup) {
        self.reviewers[group as usize] += 1;
    }
    pub fn count(self, group: ProviderGroup) -> usize {
        self.open[group as usize]
    }

    pub fn reviewer_rejection(self, group: ProviderGroup) -> Option<String> {
        let count = self.reviewers[group as usize];
        (count >= MAX_OPEN_REVIEWERS_PER_PROVIDER).then(|| {
            format!(
                "{} already has {count} of {} reviewers open across every chat.",
                group.label(),
                MAX_OPEN_REVIEWERS_PER_PROVIDER
            )
        })
    }

    pub fn rejection(self, group: ProviderGroup) -> Option<String> {
        let count = self.count(group);
        let cap = group.cap();
        (count >= cap).then(|| {
            format!(
                "{} has {count} of {cap} chats open across all chats, counting owner chats.",
                group.label()
            )
        })
    }

    pub fn spawn_rejection(self, group: ProviderGroup, reviewer: bool) -> Option<String> {
        self.rejection(group)
            .or_else(|| reviewer.then(|| self.reviewer_rejection(group)).flatten())
    }

    /// A role step is full only when every enabled account for that step is full.
    fn route_rejection(self, account: AccountKind, reviewer: bool) -> Option<String> {
        let mut reasons = Vec::new();
        for group in ProviderGroup::ALL {
            let enabled = match group {
                ProviderGroup::ClaudePlanA => self.enabled_plan[0],
                ProviderGroup::ClaudePlanB => self.enabled_plan[1],
                ProviderGroup::HaikuApiA => self.enabled_api[0],
                ProviderGroup::HaikuApiB => self.enabled_api[1],
                _ => true,
            };
            if group.account() != account || !enabled {
                continue;
            }
            if let Some(reason) = self.spawn_rejection(group, reviewer) {
                reasons.push(reason);
                continue;
            }
            let unavailable = match group {
                ProviderGroup::ClaudePlanA => self.unavailable_plan[0],
                ProviderGroup::ClaudePlanB => self.unavailable_plan[1],
                ProviderGroup::HaikuApiA => self.unavailable_api[0],
                ProviderGroup::HaikuApiB => self.unavailable_api[1],
                _ => false,
            };
            if unavailable {
                let resource = if account == AccountKind::ClaudePlan {
                    "plan allowance"
                } else {
                    "API credit"
                };
                reasons.push(format!("{} has no available {resource}. Wait for its quota reset or use another account.", group.label()));
            } else {
                return None;
            }
        }
        if reasons.is_empty() && matches!(account, AccountKind::ClaudePlan | AccountKind::ClaudeApi)
        {
            return Some(format!(
                "No {} accounts are enabled. Enable an account before starting another chat.",
                account.label()
            ));
        }
        (!reasons.is_empty()).then(|| reasons.join(" "))
    }
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
                "OpenAI API used {} tokens in the last minute plus {} projected tokens, about {:.1}M of its {:.1}M-token spawn budget. Wait for API headroom or let the reviewer fall back to Haiku on the Claude API.",
                self.recent_tokens,
                self.projected_tokens,
                total as f64 / 1_000_000.0,
                budget / 1_000_000.0
            ));
        }
        if self.active_reviewers >= OPENAI_API_MAX_ACTIVE_REVIEWERS {
            return Err(format!(
                "OpenAI API has {} active reviewers, at or above its cap of {}. Wait for a reviewer to become idle or let the reviewer fall back to Haiku on the Claude API.",
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

/// Next Claude Platform API credit grant, in UTC.
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
    resolve_with_groups(
        role,
        difficulty,
        readings,
        exhausted,
        headroom,
        now,
        ProviderCounts::default(),
    )
}

#[cfg(test)]
fn resolve_with(
    role: Role,
    difficulty: Difficulty,
    readings: &[AccountReading],
    exhausted: &[ExhaustedStep],
    headroom: ApiHeadroom,
    now: i64,
    (configured, codex_enabled, openai_enabled): (impl Fn(StepId) -> bool, bool, bool),
) -> RouteOutcome {
    resolve_core(
        role,
        difficulty,
        readings,
        exhausted,
        headroom,
        now,
        (
            configured,
            codex_enabled,
            openai_enabled,
            ProviderCounts::default(),
        ),
    )
}

/// Resolve with global provider reservations in addition to allowance and API headroom.
pub fn resolve_with_groups(
    role: Role,
    difficulty: Difficulty,
    readings: &[AccountReading],
    exhausted: &[ExhaustedStep],
    headroom: ApiHeadroom,
    now: i64,
    counts: ProviderCounts,
) -> RouteOutcome {
    resolve_core(
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
            counts,
        ),
    )
}

fn resolve_core(
    role: Role,
    difficulty: Difficulty,
    readings: &[AccountReading],
    exhausted: &[ExhaustedStep],
    headroom: ApiHeadroom,
    now: i64,
    (configured, codex_enabled, openai_enabled, counts): (
        impl Fn(StepId) -> bool,
        bool,
        bool,
        ProviderCounts,
    ),
) -> RouteOutcome {
    let mut rejections = Vec::new();
    for step in steps(role) {
        let disabled = match step.id {
            StepId::ImplementerCodexPlan if !codex_enabled => Some("CODEX_PLAN_ROUTING_ENABLED"),
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
        if let Some(reason) = counts.route_rejection(step.account, role == Role::Reviewer) {
            rejections.push(StepRejection {
                step: step.id,
                model: step.model,
                reason,
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
            StepId::ImplementerCodexPlan | StepId::ImplementerClaude | StepId::PlannerClaude => {
                weekly_plan_reason(step.account, reading, now)
            }
            // Kept only to read old receipts; no role chain contains them.
            StepId::ImplementerCodexCredits
            | StepId::ReviewerCodexPlan
            | StepId::ImplementerDeepSeek
            | StepId::ReviewerDeepSeek => continue,
            StepId::ReviewerOpenAiApi => headroom.reason(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn all_configured(_: StepId) -> bool {
        true
    }

    fn reading(account: AccountKind, used: Option<f64>, reset: Option<i64>) -> AccountReading {
        AccountReading {
            account,
            weekly_used_percent: used,
            weekly_resets_at: reset,
        }
    }

    #[test]
    fn routing_chains_walk_every_available_exhausted_and_disabled_step() {
        let now = 1000;
        let readings = [reading(AccountKind::CodexPlan, Some(0.0), Some(2000))];
        for (role, expected) in [
            (
                Role::Implementer,
                vec![StepId::ImplementerCodexPlan, StepId::ImplementerClaude],
            ),
            (
                Role::Reviewer,
                vec![StepId::ReviewerOpenAiApi, StepId::ReviewerClaudeApi],
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
                        StepId::ImplementerCodexPlan => codex,
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
                                1
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
                &[reading(AccountKind::ClaudePlan, Some(used), Some(reset))],
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
        let configured = |step| matches!(step, StepId::ImplementerClaude);
        // No reserve: days to reset never matter, only the 2% cutoff.
        for (used, days, expected) in [
            (10.0, 6.0, Some(StepId::ImplementerClaude)),
            (97.9, 6.0, Some(StepId::ImplementerClaude)),
            (97.9, 0.1, Some(StepId::ImplementerClaude)),
            (98.5, 6.0, None),
        ] {
            let readings = [reading(
                AccountKind::ClaudePlan,
                Some(used),
                Some(now + (days * 86_400.0) as i64),
            )];
            let outcome = resolve_with(
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
            );
            let chosen = match outcome {
                RouteOutcome::Chosen(chosen) => Some(chosen.step),
                RouteOutcome::Exhausted(_) => None,
            };
            assert_eq!(chosen, expected, "{used}% used, {days} days left");
        }
    }

    #[test]
    fn implementers_use_the_codex_plan_then_claude_and_never_codex_credits() {
        let now = 1_000_000;
        let reset = now + 86_400;
        let routed = |readings: &[AccountReading]| {
            let RouteOutcome::Chosen(chosen) = resolve_with(
                Role::Implementer,
                Difficulty::Hard,
                readings,
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
            chosen.step
        };
        assert_eq!(
            routed(&[reading(AccountKind::CodexPlan, Some(99.0), Some(reset))]),
            StepId::ImplementerCodexPlan
        );
        // With the Codex plan allowance exhausted, an implementer goes to Sonnet on Claude.
        let sonnet = routed(&[
            reading(AccountKind::CodexPlan, Some(100.0), Some(reset)),
            reading(AccountKind::ClaudePlan, Some(10.0), Some(reset)),
        ]);
        assert_eq!(sonnet, StepId::ImplementerClaude);
        let RouteOutcome::Exhausted(rejections) = resolve_with(
            Role::Implementer,
            Difficulty::Hard,
            &[
                reading(AccountKind::CodexPlan, Some(100.0), Some(reset)),
                reading(AccountKind::ClaudePlan, Some(100.0), Some(reset)),
            ],
            &[],
            ApiHeadroom::default(),
            now,
            (all_configured, true, true),
        ) else {
            panic!()
        };
        assert_eq!(
            rejections.iter().map(|r| r.step).collect::<Vec<_>>(),
            vec![StepId::ImplementerCodexPlan, StepId::ImplementerClaude]
        );
        assert!(IMPLEMENTER_STEPS.iter().all(|step| {
            step.id != StepId::ImplementerCodexCredits && step.id != StepId::ImplementerDeepSeek
        }));
        assert_eq!(
            StepId::parse("implementer-codex-credits"),
            Some(StepId::ImplementerCodexCredits)
        );
    }

    #[test]
    fn reviewers_never_route_to_a_plan_or_deepseek_under_any_readings() {
        let now = 1000;
        let api = [StepId::ReviewerOpenAiApi, StepId::ReviewerClaudeApi];
        let marks: Vec<_> = api
            .iter()
            .map(|&step| ExhaustedStep {
                step,
                until: now + 100,
            })
            .collect();
        for used in [None, Some(0.0), Some(100.0)] {
            let readings = [
                reading(AccountKind::CodexPlan, used, Some(now + 100)),
                reading(AccountKind::ClaudePlan, used, Some(now + 100)),
            ];
            for n in 0..=api.len() {
                for (codex, openai) in [(true, true), (false, true), (true, false), (false, false)]
                {
                    let outcome = resolve_with(
                        Role::Reviewer,
                        Difficulty::Simple,
                        &readings,
                        &marks[..n],
                        ApiHeadroom::default(),
                        now,
                        (StepId::configured, codex, openai),
                    );
                    match outcome {
                        RouteOutcome::Chosen(route) => {
                            assert!(api.contains(&route.step));
                            assert_ne!(route.account, AccountKind::CodexPlan);
                            assert_ne!(route.account, AccountKind::ClaudePlan);
                            assert_ne!(route.account, AccountKind::DeepSeek);
                            assert!(!route.reason.contains("Codex plan"));
                        }
                        RouteOutcome::Exhausted(rejections) => {
                            assert_eq!(n, api.len());
                            assert_eq!(rejections.iter().map(|r| r.step).collect::<Vec<_>>(), api);
                        }
                    }
                }
            }
        }
        assert!(REVIEWER_STEPS.iter().all(|step| {
            step.id != StepId::ReviewerCodexPlan && step.id != StepId::ReviewerDeepSeek
        }));
        assert_eq!(
            StepId::parse("reviewer-codex-plan"),
            Some(StepId::ReviewerCodexPlan)
        );
        assert_eq!(StepId::ReviewerCodexPlan.as_str(), "reviewer-codex-plan");
        assert!(!StepId::ReviewerCodexPlan.configured());
    }

    #[test]
    fn haiku_reviewer_effort_is_xhigh() {
        assert_eq!(ROLE_REVIEWER_HAIKU_EFFORT, "xhigh");
    }

    #[test]
    fn reviewers_start_at_luna_on_the_api() {
        let now = 1000;
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
        let configured = |step| matches!(step, StepId::ImplementerClaude);
        let marks = [ExhaustedStep {
            step: StepId::ImplementerClaude,
            until: now + 10,
        }];
        let RouteOutcome::Exhausted(rejections) = resolve_with(
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
        assert!(rejections.iter().any(|rejection| {
            rejection.step == StepId::ImplementerClaude
                && rejection.reason.contains("marked exhausted")
        }));

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
                step: StepId::ImplementerClaude,
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
            rejection.step == StepId::ImplementerClaude
                && rejection.reason.contains("marked exhausted")
        }));
    }

    #[test]
    fn reviewer_headroom_falls_back_to_haiku() {
        let budget = (OPENAI_API_TPM_LIMIT as f64 * OPENAI_API_TPM_BUDGET) as u64;
        for (tokens, projected, active, expected) in [
            (budget - 1, 0, 0, StepId::ReviewerOpenAiApi),
            (budget, 0, 0, StepId::ReviewerClaudeApi),
            (budget - 1, 1, 0, StepId::ReviewerClaudeApi),
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
                StepId::ReviewerClaudeApi,
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
            if expected == StepId::ReviewerClaudeApi {
                assert_eq!(route.model, MODEL_HAIKU);
                assert_eq!(route.effort, ROLE_REVIEWER_HAIKU_EFFORT);
                assert_eq!(route.account, AccountKind::ClaudeApi);
                assert!(route.reason.contains("fall back to Haiku"));
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
    fn api_reset_uses_each_providers_configured_utc_credit_day() {
        assert_eq!(crate::OPENAI_API_CREDIT_RESET_DAY, 1);
        assert_eq!(crate::ANTHROPIC_API_CREDIT_RESET_DAY, 27);
        let claude_day = crate::ANTHROPIC_API_CREDIT_RESET_DAY;
        assert_eq!(
            claude_api_reset_at(days_from_civil(2026, 12, 1) * 86_400),
            days_from_civil(2026, 12, claude_day) * 86_400
        );
        assert_eq!(
            claude_api_reset_at(days_from_civil(2026, 12, claude_day) * 86_400),
            days_from_civil(2027, 1, claude_day) * 86_400
        );
        assert_eq!(
            claude_api_reset_at(days_from_civil(2026, 12, 31) * 86_400),
            days_from_civil(2027, 1, claude_day) * 86_400
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

#[cfg(test)]
mod provider_cap_tests {
    use super::*;

    #[test]
    fn provider_caps_mapping_matches_every_route_and_distinguishes_accounts() {
        for role in [Role::Implementer, Role::Reviewer, Role::Planner] {
            for step in steps(role) {
                let (agent, key) = match step.account {
                    AccountKind::CodexPlan => ("codex", Some(CODEX_ACCOUNT_KEY_PLAN)),
                    AccountKind::OpenAiApi => ("codex", Some(CODEX_ACCOUNT_KEY_API)),
                    AccountKind::ClaudePlan => ("claude", Some(CLAUDE_ACCOUNT_KEY_PLAN_B)),
                    AccountKind::ClaudeApi => ("claude", Some(CLAUDE_ACCOUNT_KEY_API_B)),
                    AccountKind::DeepSeek => ("pi", None),
                };
                assert_eq!(
                    provider_group(agent, step.model, key).unwrap().account(),
                    step.account
                );
            }
        }
        for (agent, model, key, expected) in [
            (
                "codex",
                MODEL_GPT_6_1_SOL,
                None,
                Some(ProviderGroup::CodexPlan),
            ),
            ("claude", MODEL_OPUS, None, Some(ProviderGroup::ClaudePlanA)),
            (
                "claude",
                MODEL_OPUS,
                Some(CLAUDE_ACCOUNT_KEY_PLAN_B),
                Some(ProviderGroup::ClaudePlanB),
            ),
            (
                "claude",
                MODEL_HAIKU,
                Some(CLAUDE_ACCOUNT_KEY_API_A),
                Some(ProviderGroup::HaikuApiA),
            ),
            (
                "claude",
                MODEL_HAIKU,
                Some(CLAUDE_ACCOUNT_KEY_API_B),
                Some(ProviderGroup::HaikuApiB),
            ),
            ("pi", "another-model", None, None),
            (
                "codex",
                MODEL_GPT_6_1_SOL,
                Some(CODEX_ACCOUNT_KEY_API),
                None,
            ),
            ("claude", MODEL_OPUS, Some(CLAUDE_ACCOUNT_KEY_API_A), None),
        ] {
            assert_eq!(provider_group(agent, model, key), expected);
        }
    }

    #[test]
    fn provider_caps_fall_through_and_report_every_full_account() {
        let mut counts = ProviderCounts {
            enabled_plan: [true; 2],
            enabled_api: [true; 2],
            ..Default::default()
        };
        for (group, role, expected) in [
            (
                ProviderGroup::CodexPlan,
                Role::Implementer,
                Some(StepId::ImplementerClaude),
            ),
            (
                ProviderGroup::ClaudePlanA,
                Role::Implementer,
                Some(StepId::ImplementerClaude),
            ),
            (ProviderGroup::ClaudePlanB, Role::Implementer, None),
            (
                ProviderGroup::LunaApi,
                Role::Reviewer,
                Some(StepId::ReviewerClaudeApi),
            ),
            (
                ProviderGroup::HaikuApiA,
                Role::Reviewer,
                Some(StepId::ReviewerClaudeApi),
            ),
            (ProviderGroup::HaikuApiB, Role::Reviewer, None),
        ] {
            counts.open[group as usize] = group.cap();
            let outcome = resolve_with_groups(
                role,
                Difficulty::Simple,
                &[],
                &[],
                ApiHeadroom::default(),
                1,
                counts,
            );
            match (outcome, expected) {
                (RouteOutcome::Chosen(route), Some(step)) => assert_eq!(route.step, step),
                (RouteOutcome::Exhausted(rejections), None) => {
                    if group == ProviderGroup::HaikuApiB {
                        let reasons = rejections
                            .iter()
                            .map(|r| r.reason.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        assert!(reasons.contains("Haiku API account A"));
                        assert!(reasons.contains("Haiku API account B"));
                        assert!(!reasons.contains("marked exhausted"));
                    }
                }
                other => panic!("{group:?}: {other:?}"),
            }
        }
        for role in [Role::Implementer, Role::Reviewer, Role::Planner] {
            let RouteOutcome::Exhausted(reasons) = resolve_with_groups(
                role,
                Difficulty::Simple,
                &[],
                &[],
                ApiHeadroom::default(),
                1,
                counts,
            ) else {
                panic!()
            };
            assert_eq!(reasons.len(), steps(role).len());
            assert!(reasons.iter().all(|reason| is_cap_reason(&reason.reason)));
            assert!(
                reasons
                    .iter()
                    .all(|reason| !reason.reason.contains(CAP_ADVICE)),
                "the advice is added once to the final error, not to each step"
            );
        }
    }

    #[test]
    fn luna_api_allows_four_reviewers_and_haiku_api_stays_at_three() {
        assert_eq!(MAX_OPEN_REVIEWERS_PER_PROVIDER, 4);
        assert_eq!(ProviderGroup::LunaApi.cap(), 4);
        let luna = ProviderGroup::LunaApi as usize;
        let mut counts = ProviderCounts::default();
        for open in 0..4 {
            counts.open[luna] = open;
            counts.reviewers[luna] = open;
            assert!(counts.rejection(ProviderGroup::LunaApi).is_none());
            assert!(counts.reviewer_rejection(ProviderGroup::LunaApi).is_none());
        }
        counts.open[luna] = 4;
        counts.reviewers[luna] = 4;
        assert!(counts.rejection(ProviderGroup::LunaApi).is_some());
        for group in [ProviderGroup::HaikuApiA, ProviderGroup::HaikuApiB] {
            let index = group as usize;
            counts.open[index] = 2;
            counts.reviewers[index] = 2;
            assert!(counts.rejection(group).is_none());
            counts.open[index] = 3;
            counts.reviewers[index] = 3;
            assert!(
                counts.rejection(group).is_some(),
                "a third Haiku reviewer fills the account, so a fourth cannot start"
            );
        }
    }

    #[test]
    fn a_fifth_luna_reviewer_falls_through_to_haiku() {
        let luna = ProviderGroup::LunaApi as usize;
        for (open, expected) in [
            (3, StepId::ReviewerOpenAiApi),
            (4, StepId::ReviewerClaudeApi),
        ] {
            let mut counts = ProviderCounts {
                enabled_plan: [true; 2],
                enabled_api: [true; 2],
                ..Default::default()
            };
            counts.open[luna] = open;
            counts.reviewers[luna] = open;
            let headroom = ApiHeadroom {
                active_reviewers: open,
                ..Default::default()
            };
            let RouteOutcome::Chosen(route) = resolve_with_groups(
                Role::Reviewer,
                Difficulty::Simple,
                &[],
                &[],
                headroom,
                1,
                counts,
            ) else {
                panic!()
            };
            assert_eq!(route.step, expected);
        }
    }

    #[test]
    fn provider_caps_reviewers_are_separate_per_account_and_do_not_limit_implementers() {
        let mut counts = ProviderCounts {
            enabled_plan: [true; 2],
            enabled_api: [true; 2],
            ..Default::default()
        };
        for group in ProviderGroup::ALL {
            counts.reviewers[group as usize] = MAX_OPEN_REVIEWERS_PER_PROVIDER;
            assert!(
                counts
                    .reviewer_rejection(group)
                    .unwrap()
                    .contains("reviewers open")
            );
        }
        assert!(matches!(
            resolve_with_groups(
                Role::Reviewer,
                Difficulty::Simple,
                &[],
                &[],
                ApiHeadroom::default(),
                1,
                counts
            ),
            RouteOutcome::Exhausted(_)
        ));
        let RouteOutcome::Chosen(route) = resolve_with_groups(
            Role::Implementer,
            Difficulty::Simple,
            &[],
            &[],
            ApiHeadroom::default(),
            1,
            counts,
        ) else {
            panic!()
        };
        assert_eq!(route.step, StepId::ImplementerCodexPlan);
        counts.reviewers[ProviderGroup::HaikuApiB as usize] = 0;
        let RouteOutcome::Chosen(route) = resolve_with_groups(
            Role::Reviewer,
            Difficulty::Simple,
            &[],
            &[],
            ApiHeadroom::default(),
            1,
            counts,
        ) else {
            panic!()
        };
        assert_eq!(route.step, StepId::ReviewerClaudeApi);
    }
}
