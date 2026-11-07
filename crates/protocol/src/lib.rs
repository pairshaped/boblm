//! Claude Code stream-json wire types. Unknown event types and unknown fields
//! are tolerated on purpose: Claude Code adds both between releases.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Now,
    Next,
    Later,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MessageOrigin {
    Human,
}

#[derive(Debug, Serialize)]
pub struct UserMessage<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub uuid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<MessageOrigin>,
    pub message: UserBody<'a>,
}

#[derive(Debug, Serialize)]
pub struct UserBody<'a> {
    pub role: &'static str,
    pub content: &'a str,
}

impl<'a> UserMessage<'a> {
    pub fn new(uuid: &'a str, content: &'a str, priority: Option<Priority>) -> Self {
        Self {
            kind: "user",
            uuid,
            priority,
            origin: None,
            message: UserBody {
                role: "user",
                content,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Queued,
    Started,
    Completed,
    Cancelled,
    Discarded,
    Refused,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    PromptSuggestion {
        #[serde(default)]
        suggestion: serde_json::Value,
    },
    CommandLifecycle {
        command_uuid: String,
        state: LifecycleState,
    },
    Result {
        subtype: Option<String>,
        is_error: Option<bool>,
    },
    BackgroundJobs {
        snapshot: jobs::JobSnapshot,
    },
    Usage {
        #[serde(default)]
        five_hour: Option<usage::AllowanceUsage>,
        #[serde(default)]
        credits: Option<usage::CreditsUsage>,
        #[serde(default)]
        context: Option<usage::ContextUsage>,
        #[serde(default)]
        weekly: Option<usage::AllowanceUsage>,
    },
    TokenUsage {
        model: String,
        input_tokens: u64,
        cached_input_tokens: u64,
        cache_write_input_tokens: u64,
        output_tokens: u64,
    },
    RateLimitEvent {
        #[serde(default)]
        rate_limit_info: serde_json::Value,
    },
    Assistant {
        message: serde_json::Value,
    },
    User {
        message: serde_json::Value,
    },
    System {
        subtype: Option<String>,
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    StreamEvent {
        event: serde_json::Value,
    },
    ControlResponse {
        response: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_claude_compaction_status() {
        let event: Event = serde_json::from_str(
            r#"{"type":"system","subtype":"status","status":"compacting","new_field":true}"#,
        )
        .unwrap();
        assert!(
            matches!(event, Event::System { status: Some(status), .. } if status == "compacting")
        );
    }

    #[test]
    fn parses_lifecycle_and_tolerates_new_fields_and_states() {
        let e: Event = serde_json::from_str(
            r#"{"type":"command_lifecycle","command_uuid":"a","state":"started","brand_new":1}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            Event::CommandLifecycle {
                state: LifecycleState::Started,
                ..
            }
        ));
        let e: Event = serde_json::from_str(
            r#"{"type":"command_lifecycle","command_uuid":"a","state":"frobnicated"}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            Event::CommandLifecycle {
                state: LifecycleState::Unknown,
                ..
            }
        ));
        let e: Event =
            serde_json::from_str(r#"{"type":"rate_limit_event","rate_limit_info":{}}"#).unwrap();
        assert!(matches!(e, Event::RateLimitEvent { .. }));
    }

    #[test]
    fn now_priority_serializes_and_default_omits() {
        let now = serde_json::to_string(&UserMessage::new("u", "hi", Some(Priority::Now))).unwrap();
        assert!(now.contains(r#""priority":"now""#));
        let plain = serde_json::to_string(&UserMessage::new("u", "hi", None)).unwrap();
        assert!(!plain.contains("priority"));
    }
}

pub mod client;
pub mod jobs;

pub mod usage;
