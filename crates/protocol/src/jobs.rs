//! Current background work, independent of the rendered conversation window.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundJob {
    pub id: String,
    pub command: String,
    /// None identifies the parent chat; otherwise the provider's subagent key.
    pub agent_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobSnapshot {
    pub tasks: Vec<BackgroundJob>,
    /// False retains the last known jobs but cannot establish that restart is safe.
    pub available: bool,
}
