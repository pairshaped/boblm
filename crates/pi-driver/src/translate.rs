//! Pi notifications mapped to the runner's Claude-shaped events.
use boblm_protocol::{Event, LifecycleState};
use serde_json::{Value, json};
use std::collections::VecDeque;
#[derive(Default)]
pub struct Translator {
    messages: VecDeque<(String, String)>,
    pub compact: Option<String>,
    compact_started: bool,
    assistant: String,
    outcome: Option<String>,
}
pub fn assistant_text(id: &str, text: &str) -> Event {
    Event::Assistant {
        message: json!({"id":id,"content":[{"type":"text","text":text}]}),
    }
}
impl Translator {
    pub fn expect_message(&mut self, id: &str, text: &str) {
        self.messages.push_back((id.into(), text.into()));
    }
    pub fn reject(&mut self, id: &str) -> bool {
        let before = self.messages.len();
        self.messages.retain(|(uuid, _)| uuid != id);
        before != self.messages.len()
    }
    pub fn notification(&mut self, v: &Value) -> Vec<Event> {
        match v["type"].as_str().unwrap_or("") {
            "message_start" if v["message"]["role"] == "user" => {
                let content = &v["message"]["content"];
                let text = content.as_str().map(str::to_owned).or_else(|| {
                    content.as_array().and_then(|a| {
                        if a.len() == 1 && a[0]["type"] == "text" {
                            a[0]["text"].as_str().map(str::to_owned)
                        } else {
                            None
                        }
                    })
                });
                if self
                    .messages
                    .front()
                    .is_some_and(|(_, expected)| Some(expected) == text.as_ref())
                {
                    let (id, _) = self.messages.pop_front().expect("checked head");
                    vec![Event::CommandLifecycle {
                        command_uuid: id,
                        state: LifecycleState::Started,
                    }]
                } else {
                    Vec::new()
                }
            }
            "agent_start" => {
                self.outcome = None;
                Vec::new()
            }
            "message_start" if v["message"]["role"] == "assistant" => {
                self.assistant = uuid::Uuid::new_v4().to_string();
                vec![Event::StreamEvent {
                    event: json!({"type":"message_start","message":{"id":self.assistant}}),
                }]
            }
            "message_update" => {
                let e = &v["assistantMessageEvent"];
                let (kind, field) = match e["type"].as_str() {
                    Some("text_delta") => ("text_delta", "text"),
                    Some("thinking_delta") => ("thinking_delta", "thinking"),
                    _ => return Vec::new(),
                };
                let Some(delta) = e["delta"].as_str() else {
                    return Vec::new();
                };
                let mut value = json!({"type":kind});
                value[field] = json!(delta);
                vec![Event::StreamEvent {
                    event: json!({"type":"content_block_delta","index":e["contentIndex"],"delta":value}),
                }]
            }
            "message_end" if v["message"]["role"] == "assistant" => {
                let m = &v["message"];
                if self.assistant.is_empty() {
                    self.assistant = uuid::Uuid::new_v4().to_string();
                }
                let content:Vec<Value>=m["content"].as_array().into_iter().flatten().filter_map(|b| match b["type"].as_str() {
                    Some("text")=>Some(json!({"type":"text","text":b["text"]})),
                    Some("thinking")=>Some(json!({"type":"thinking","thinking":b["thinking"]})),
                    Some("toolCall")=>Some(json!({"type":"tool_use","id":b["id"],"name":b["name"],"input":b["arguments"]})),
                    _=>None,
                }).collect();
                let u = &m["usage"];
                let mut events = vec![Event::Assistant {
                    message: json!({"id":self.assistant,"model":m["model"],"content":content,"usage":{"input_tokens":u["input"],"output_tokens":u["output"],"cache_read_input_tokens":u["cacheRead"],"cache_creation_input_tokens":u["cacheWrite"]},"pi_usage":u}),
                }];
                match m["stopReason"].as_str() {
                    Some("error") => {
                        let why = error_reason(
                            m["errorMessage"]
                                .as_str()
                                .unwrap_or("the provider supplied no explanation"),
                        );
                        self.outcome = Some(if is_context_overflow(&why) {
                            "context_overflow".into()
                        } else {
                            "error_during_execution".into()
                        });
                        events.push(assistant_text(&format!("error-{}",self.assistant), &format!("Pi stopped this turn with an error: {why}. Check the provider and model before retrying.")));
                    }
                    Some("aborted") => self.outcome = Some("interrupted".into()),
                    _ => {}
                }
                self.assistant.clear();
                events
            }
            "tool_execution_end" => vec![Event::User {
                message: json!({"content":[{"type":"tool_result","tool_use_id":v["toolCallId"],"content":v["result"]["content"],"is_error":v["isError"]}]}),
            }],
            "agent_end" => vec![self.result()],
            "compaction_start" => {
                self.compact_started = true;
                let mut events = vec![Event::System {
                    subtype: Some("compact_started".into()),
                    session_id: None,
                    status: None,
                }];
                if let Some(id) = &self.compact {
                    events.push(Event::CommandLifecycle {
                        command_uuid: id.clone(),
                        state: LifecycleState::Started,
                    });
                }
                events
            }
            "compaction_end" => {
                let id = self.compact.take();
                let started = std::mem::take(&mut self.compact_started);
                if !started {
                    return Vec::new();
                };
                let Some(id) = id else {
                    return vec![Event::System {
                        subtype: Some("status".into()),
                        session_id: None,
                        status: None,
                    }];
                };
                let success = v["result"].is_object()
                    && v["aborted"] == false
                    && v.get("errorMessage").is_none();
                let mut events = Vec::new();
                if success {
                    events.push(Event::System {
                        subtype: Some("compact_boundary".into()),
                        session_id: None,
                        status: None,
                    });
                } else {
                    events.push(assistant_text(&format!("compact-{id}"),&format!("Pi could not compact this conversation: {}. Retry /compact after checking the provider.",error_reason(v["errorMessage"].as_str().unwrap_or("compaction was interrupted")))));
                }
                events.push(Event::CommandLifecycle {
                    command_uuid: id,
                    state: LifecycleState::Completed,
                });
                events.push(Event::Result {
                    subtype: Some(
                        if success {
                            "success"
                        } else {
                            "error_during_execution"
                        }
                        .into(),
                    ),
                    is_error: Some(!success),
                });
                events
            }
            "response" if v["command"] == "compact" && v["success"] == false => {
                let Some(id) = self.compact.take() else {
                    return Vec::new();
                };
                self.compact_started = false;
                vec![
                    assistant_text(
                        &format!("compact-{id}"),
                        &format!(
                            "Pi refused compaction: {}. Retry /compact after checking the conversation.",
                            error_reason(
                                v["error"].as_str().unwrap_or("Pi supplied no explanation")
                            )
                        ),
                    ),
                    Event::CommandLifecycle {
                        command_uuid: id,
                        state: LifecycleState::Refused,
                    },
                ]
            }
            _ => Vec::new(),
        }
    }
    fn result(&mut self) -> Event {
        let outcome = self.outcome.take().unwrap_or_else(|| "success".into());
        Event::Result {
            is_error: Some(matches!(
                outcome.as_str(),
                "error_during_execution" | "context_overflow"
            )),
            subtype: Some(outcome),
        }
    }
}

/// A provider refusal that means the conversation no longer fits its context window.
pub fn is_context_overflow(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "context window",
        "context length",
        "maximum context",
        "context_length_exceeded",
        "context overflow",
        "too many tokens",
        "prompt is too long",
        "input is too long",
        "exceeds the maximum",
        "token limit",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Keep the provider's status and explanation, excluding JSON envelopes and stack frames.
pub fn error_reason(text: &str) -> String {
    let first = text
        .lines()
        .next()
        .unwrap_or("Pi supplied no explanation")
        .trim();
    let detail = if let Some(start) = first.find('{') {
        let parsed = serde_json::from_str::<Value>(&first[start..]).ok();
        let message = parsed
            .as_ref()
            .and_then(|v| v.pointer("/error/message").or_else(|| v.get("message")))
            .and_then(Value::as_str);
        let prefix = first[..start].trim();
        match message {
            Some(message) if prefix.is_empty() => message.to_owned(),
            Some(message) => format!("{prefix}: {message}"),
            None => format!("{prefix}: the provider returned an unreadable error body"),
        }
    } else {
        first.to_owned()
    };
    detail
        .lines()
        .next()
        .unwrap_or("Pi supplied no explanation")
        .chars()
        .take(boblm_tuning::PI_ERROR_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_overflow_text_is_recognized_without_matching_other_errors() {
        assert!(is_context_overflow(
            "This model's maximum context length is 1048576 tokens"
        ));
        assert!(is_context_overflow("context_length_exceeded"));
        assert!(!is_context_overflow("429: rate limit reached"));
    }

    #[test]
    fn automatic_compaction_reports_both_transitions_without_ending_the_turn() {
        let mut translator = Translator::default();
        let start = translator.notification(&json!({"type":"compaction_start","reason":"auto"}));
        assert!(
            matches!(&start[..], [Event::System { subtype: Some(s), .. }] if s == "compact_started")
        );
        let end = translator.notification(
            &json!({"type":"compaction_end","reason":"auto","result":{},"aborted":false}),
        );
        assert!(
            matches!(&end[..], [Event::System { subtype: Some(s), status: None, .. }] if s == "status")
        );
    }
}
