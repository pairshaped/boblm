//! Per-account Claude routing. Unknown and expired samples do not invent meter readings.
use crate::CLAUDE_ACCOUNT_CUTOFF_PERCENT;

#[derive(Clone, Debug)]
pub struct PlanAccount {
    pub key: &'static str,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub exhausted_until: Option<i64>,
}

impl PlanAccount {
    pub fn available(&self, now: i64) -> bool {
        !self.exhausted_until.is_some_and(|until| until > now)
            && (self.resets_at.is_some_and(|reset| reset <= now)
                || self.used_percent.is_none_or(|used| {
                    used.is_finite() && used < 100.0 - CLAUDE_ACCOUNT_CUTOFF_PERCENT
                }))
    }
}

/// Drain the account whose weekly reset is first. Stable ties keep configuration order.
pub fn choose(accounts: &[PlanAccount], now: i64) -> Option<&PlanAccount> {
    accounts
        .iter()
        .filter(|account| account.available(now))
        .min_by_key(|account| account.resets_at.unwrap_or(i64::MAX))
}

/// Owner requests always get an enabled account, even when every plan is at its
/// cutoff. A single enabled account keeps the existing explicit-chat behavior.
pub fn choose_for_owner(accounts: &[PlanAccount], now: i64) -> Option<&PlanAccount> {
    choose(accounts, now).or_else(|| {
        accounts
            .iter()
            .min_by_key(|account| account.resets_at.unwrap_or(i64::MAX))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_account_choice_drains_first_then_cutoff_and_reset() {
        let mut accounts = vec![
            PlanAccount {
                key: "a",
                used_percent: Some(97.9),
                resets_at: Some(200),
                exhausted_until: None,
            },
            PlanAccount {
                key: "b",
                used_percent: Some(1.0),
                resets_at: Some(300),
                exhausted_until: None,
            },
        ];
        assert_eq!(choose(&accounts, 100).unwrap().key, "a");
        accounts[0].used_percent = Some(98.0);
        assert_eq!(choose(&accounts, 100).unwrap().key, "b");
        assert_eq!(choose(&accounts, 200).unwrap().key, "a");
        accounts[0].exhausted_until = Some(250);
        assert_eq!(choose(&accounts, 200).unwrap().key, "b");
        accounts[1].used_percent = Some(100.0);
        assert!(choose(&accounts, 200).is_none());
        accounts[0].used_percent = None;
        assert_eq!(choose(&accounts[..1], 250).unwrap().key, "a");
    }
    #[test]
    fn claude_owner_creation_keeps_the_single_account_and_falls_back_when_all_are_full() {
        let accounts = vec![
            PlanAccount {
                key: "a",
                used_percent: Some(100.0),
                resets_at: Some(300),
                exhausted_until: Some(300),
            },
            PlanAccount {
                key: "b",
                used_percent: Some(99.0),
                resets_at: Some(200),
                exhausted_until: Some(200),
            },
        ];
        assert_eq!(choose_for_owner(&accounts[..1], 100).unwrap().key, "a");
        assert_eq!(choose_for_owner(&accounts, 100).unwrap().key, "b");
        assert!(
            choose(&accounts, 100).is_none(),
            "Role spawns still enforce the cutoff"
        );
        assert!(choose_for_owner(&[], 100).is_none());
    }
}
