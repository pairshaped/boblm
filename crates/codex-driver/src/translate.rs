//! Turns Codex app-server notifications into the events the runner already understands
//! (the Claude-shaped `boblm_protocol::Event`), so storage, delivery tracking and the chat
//! need no Codex-specific code. Unknown notifications and fields are ignored.
use boblm_protocol::{Event, LifecycleState};
use serde_json::{Value, json};

/// Longest tool output kept in a stored event, in characters.
const MAX_OUTPUT_CHARS: usize = 20_000;

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    kept.push_str("\n[output cut; the full output is in Codex's own log]");
    kept
}

fn text_of(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

#[derive(Default)]
pub struct Translator {
    turn: Option<String>,
    /// A /compact command whose start is Codex announcing the compaction.
    compaction: Option<String>,
    /// The model Codex last reported for this thread, used to label token usage.
    model: Option<String>,
}

impl Translator {
    /// The turn Codex is running now, which a steer must name.
    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = Some(model.to_owned());
    }

    pub fn expect_compaction(&mut self, command_uuid: String) {
        self.compaction = Some(command_uuid);
    }

    pub fn notification(&mut self, method: &str, params: &Value) -> Vec<Event> {
        match method {
            "thread/tokenUsage/updated" => {
                let mut events = Vec::new();
                if let Some(context) = boblm_protocol::usage::codex_context(&params["tokenUsage"]) {
                    events.push(Event::Usage {
                        five_hour: None,
                        credits: None,
                        context: Some(context),
                        weekly: None,
                    });
                }
                let last = &params["tokenUsage"]["last"];
                if let (Some(input_tokens), Some(output_tokens)) =
                    (last["inputTokens"].as_u64(), last["outputTokens"].as_u64())
                {
                    let cached_input_tokens = last["cachedInputTokens"].as_u64().unwrap_or(0);
                    let cache_write_input_tokens =
                        last["cacheWriteInputTokens"].as_u64().unwrap_or(0);
                    events.push(Event::TokenUsage {
                        model: self.model.clone().unwrap_or_else(|| "unknown".into()),
                        // Codex includes cached input in inputTokens; the report stores
                        // uncached input separately, like Claude and Pi.
                        input_tokens: input_tokens
                            .saturating_sub(cached_input_tokens)
                            .saturating_sub(cache_write_input_tokens),
                        cached_input_tokens,
                        cache_write_input_tokens,
                        output_tokens,
                    });
                }
                events
            }
            "thread/settings/updated" => {
                if let Some(model) = params["threadSettings"]["model"].as_str() {
                    self.model = Some(model.to_owned());
                }
                Vec::new()
            }
            "account/rateLimits/updated" => boblm_protocol::usage::codex_allowances(params)
                .into_iter()
                .collect(),
            "turn/started" => {
                self.turn = params["turn"]["id"].as_str().map(str::to_owned);
                // Codex stays silent while the provider compacts a large thread, so this
                // is the only sign of life until the user message is acknowledged.
                vec![Event::System {
                    subtype: Some("turn_started".into()),
                    session_id: None,
                    status: None,
                }]
            }
            "turn/completed" => self.turn_completed(&params["turn"]),
            "error" => {
                let why = params["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex supplied no explanation");
                let next = if params["willRetry"].as_bool() == Some(true) {
                    "Codex is retrying; wait for its next response."
                } else {
                    "Check the server log before sending another message."
                };
                vec![assistant_text(
                    &format!("codex-error-{}", uuid::Uuid::new_v4()),
                    &format!("Codex reported an error: {}. {next}", clip(why)),
                )]
            }
            "item/started" => {
                let mut events = item_started(&params["item"]);
                if text_of(&params["item"]["type"]) == "contextCompaction"
                    && let Some(command_uuid) = self.compaction.take()
                {
                    events.push(Event::CommandLifecycle {
                        command_uuid,
                        state: LifecycleState::Started,
                    });
                }
                events
            }
            "item/completed" => item_completed(&params["item"]),
            "item/agentMessage/delta" => vec![Event::StreamEvent {
                event: json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": text_of(&params["delta"])}}),
            }],
            _ => Vec::new(),
        }
    }

    fn turn_completed(&mut self, turn: &Value) -> Vec<Event> {
        self.turn = None;
        let status = turn["status"].as_str().unwrap_or("completed");
        let mut events = Vec::new();
        if status == "failed" {
            let why = turn["error"]["message"]
                .as_str()
                .unwrap_or("Codex gave no reason");
            events.push(assistant_text(
                &format!("turn-failed-{}", turn["id"].as_str().unwrap_or("")),
                &format!("Codex stopped this turn with an error: {why}."),
            ));
        }
        events.push(Event::Result {
            subtype: Some(
                match status {
                    "interrupted" => "interrupted",
                    "failed" => "error_during_execution",
                    _ => "success",
                }
                .to_owned(),
            ),
            is_error: Some(status == "failed"),
        });
        events
    }
}

pub fn assistant_text(id: &str, text: &str) -> Event {
    Event::Assistant {
        message: json!({"id": id, "content": [{"type": "text", "text": text}]}),
    }
}

fn tool_use(item: &Value) -> Event {
    let kind = text_of(&item["type"]);
    let (name, input) = if kind == "commandExecution" {
        ("shell".to_owned(), json!({"command": item["command"]}))
    } else {
        let mut shown = item.clone();
        if let Some(map) = shown.as_object_mut() {
            map.remove("id");
        }
        let name = if matches!(kind, "mcpToolCall" | "dynamicToolCall") {
            item["tool"].as_str().unwrap_or(kind)
        } else {
            kind
        };
        (name.to_owned(), shown)
    };
    Event::Assistant {
        message: json!({"id": item["id"], "content": [{"type": "tool_use", "id": item["id"], "name": name, "input": input}]}),
    }
}

fn tool_result(item: &Value) -> Event {
    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"))
        || item["exitCode"].as_i64().is_some_and(|code| code != 0)
        || item["success"] == false
        || !item["error"].is_null();
    let output = if item["type"] == "commandExecution" {
        text_of(&item["aggregatedOutput"]).to_owned()
    } else {
        item.to_string()
    };
    Event::User {
        message: json!({"content": [{"type": "tool_result", "tool_use_id": item["id"], "content": clip(&output), "is_error": failed}]}),
    }
}

/// Item types that are not tool activity.
fn is_conversation(kind: &str) -> bool {
    matches!(
        kind,
        "userMessage" | "agentMessage" | "reasoning" | "contextCompaction"
    )
}

fn item_started(item: &Value) -> Vec<Event> {
    match text_of(&item["type"]) {
        "userMessage" => match item["clientId"].as_str() {
            // Codex announcing the message under our ID is the proof it was consumed.
            Some(id) => vec![Event::CommandLifecycle {
                command_uuid: id.to_owned(),
                state: LifecycleState::Started,
            }],
            None => Vec::new(),
        },
        "contextCompaction" => vec![Event::System {
            subtype: Some("compact_started".into()),
            session_id: None,
            status: None,
        }],
        "agentMessage" => vec![Event::StreamEvent {
            event: json!({"type": "message_start", "message": {"id": item["id"]}}),
        }],
        kind if is_conversation(kind) => Vec::new(),
        _ => vec![tool_use(item)],
    }
}

fn item_completed(item: &Value) -> Vec<Event> {
    match text_of(&item["type"]) {
        "agentMessage" => vec![assistant_text(text_of(&item["id"]), text_of(&item["text"]))],
        "contextCompaction" => vec![Event::System {
            subtype: Some("compact_boundary".into()),
            session_id: None,
            status: None,
        }],
        kind if is_conversation(kind) => Vec::new(),
        _ => vec![tool_result(item)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(translator: &mut Translator, method: &str, params: Value) -> Vec<Event> {
        translator.notification(method, &params)
    }

    #[test]
    fn our_message_id_coming_back_is_the_start_of_that_message() {
        let mut t = Translator::default();
        let events = run(
            &mut t,
            "item/started",
            json!({"item": {"type": "userMessage", "id": "i1", "clientId": "uuid-1"}}),
        );
        assert!(
            matches!(&events[..], [Event::CommandLifecycle { command_uuid, state: LifecycleState::Started }] if command_uuid == "uuid-1")
        );
        assert!(
            run(
                &mut t,
                "item/started",
                json!({"item": {"type": "userMessage", "id": "i2"}})
            )
            .is_empty(),
            "a message we did not send proves nothing"
        );
        assert!(
            run(
                &mut t,
                "item/completed",
                json!({"item": {"type": "userMessage", "id": "i1", "clientId": "uuid-1"}})
            )
            .is_empty()
        );
    }

    #[test]
    fn an_answer_streams_then_settles_into_one_message() {
        let mut t = Translator::default();
        let started = run(
            &mut t,
            "item/started",
            json!({"item": {"type": "agentMessage", "id": "m1", "text": ""}}),
        );
        assert!(
            matches!(&started[..], [Event::StreamEvent { event }] if event["type"] == "message_start" && event["message"]["id"] == "m1")
        );
        let delta = run(
            &mut t,
            "item/agentMessage/delta",
            json!({"itemId": "m1", "delta": "Hel"}),
        );
        assert!(
            matches!(&delta[..], [Event::StreamEvent { event }] if event["delta"]["text"] == "Hel")
        );
        let done = run(
            &mut t,
            "item/completed",
            json!({"item": {"type": "agentMessage", "id": "m1", "text": "Hello"}}),
        );
        assert!(
            matches!(&done[..], [Event::Assistant { message }] if message["id"] == "m1" && message["content"][0]["text"] == "Hello")
        );
    }

    #[test]
    fn a_shell_command_becomes_a_tool_call_and_its_result() {
        let mut t = Translator::default();
        let call = run(
            &mut t,
            "item/started",
            json!({"item": {"type": "commandExecution", "id": "e1", "command": "ls"}}),
        );
        assert!(
            matches!(&call[..], [Event::Assistant { message }] if message["content"][0]["name"] == "shell" && message["content"][0]["input"]["command"] == "ls")
        );
        let result = run(
            &mut t,
            "item/completed",
            json!({"item": {"type": "commandExecution", "id": "e1", "aggregatedOutput": "a\nb\n"}}),
        );
        assert!(
            matches!(&result[..], [Event::User { message }] if message["content"][0]["content"] == "a\nb\n" && message["content"][0]["tool_use_id"] == "e1")
        );
        let long = "x".repeat(MAX_OUTPUT_CHARS + 50);
        let big = run(
            &mut t,
            "item/completed",
            json!({"item": {"type": "commandExecution", "id": "e2", "aggregatedOutput": long}}),
        );
        let Event::User { message } = &big[0] else {
            panic!("{big:?}")
        };
        assert!(
            message["content"][0]["content"]
                .as_str()
                .unwrap()
                .contains("output cut")
        );
        // Other tool kinds are shown generically; compaction becomes the chat's marker.
        let other = run(
            &mut t,
            "item/started",
            json!({"item": {"type": "fileChange", "id": "f1", "changes": []}}),
        );
        assert!(
            matches!(&other[..], [Event::Assistant { message }] if message["content"][0]["name"] == "fileChange")
        );
        let marker = run(
            &mut t,
            "item/completed",
            json!({"item": {"type": "contextCompaction", "id": "c1"}}),
        );
        assert!(
            matches!(&marker[..], [Event::System { subtype: Some(s), .. }] if s == "compact_boundary")
        );
    }

    #[test]
    fn failed_tool_results_keep_their_error_state() {
        for item in [
            json!({"type":"commandExecution", "id":"x", "exitCode":2}),
            json!({"type":"fileChange", "id":"x", "status":"declined"}),
            json!({"type":"mcpToolCall", "id":"x", "status":"failed"}),
            json!({"type":"dynamicToolCall", "id":"x", "success":false}),
        ] {
            let Event::User { message } = tool_result(&item) else {
                panic!("Expected a tool result")
            };
            assert_eq!(message["content"][0]["is_error"], true);
        }
        let Event::User { message } =
            tool_result(&json!({"type":"commandExecution", "exitCode":0}))
        else {
            panic!("Expected a tool result")
        };
        assert_eq!(message["content"][0]["is_error"], false);
    }

    #[test]
    fn a_started_turn_is_reported_as_progress_but_never_as_delivery() {
        let mut t = Translator::default();
        let events = run(&mut t, "turn/started", json!({"turn": {"id": "t1"}}));
        assert!(
            matches!(&events[..], [Event::System { subtype: Some(s), .. }] if s == "turn_started"),
            "turn/started tells the runner the agent is working: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::CommandLifecycle { .. })),
            "only an acknowledged user message proves consumption"
        );
    }

    #[test]
    fn turns_track_the_running_turn_and_end_with_a_result() {
        let mut t = Translator::default();
        assert_eq!(t.turn(), None);
        run(&mut t, "turn/started", json!({"turn": {"id": "t1"}}));
        assert_eq!(t.turn(), Some("t1"));
        let done = run(
            &mut t,
            "turn/completed",
            json!({"turn": {"id": "t1", "status": "completed"}}),
        );
        assert!(
            matches!(&done[..], [Event::Result { subtype: Some(s), is_error: Some(false) }] if s == "success")
        );
        assert_eq!(t.turn(), None);
        let stopped = run(
            &mut t,
            "turn/completed",
            json!({"turn": {"id": "t2", "status": "interrupted"}}),
        );
        assert!(
            matches!(&stopped[..], [Event::Result { subtype: Some(s), is_error: Some(false) }] if s == "interrupted")
        );
        let failed = run(
            &mut t,
            "turn/completed",
            json!({"turn": {"id": "t3", "status": "failed", "error": {"message": "rate limit reached"}}}),
        );
        assert_eq!(failed.len(), 2, "the reason is shown before the result");
        assert!(
            matches!(&failed[0], Event::Assistant { message } if message["content"][0]["text"].as_str().unwrap().contains("rate limit reached"))
        );
        assert!(
            matches!(&failed[1], Event::Result { subtype: Some(s), is_error: Some(true) } if s == "error_during_execution")
        );
    }

    #[test]
    fn token_usage_is_labelled_with_the_thread_model() {
        let mut t = Translator::default();
        assert!(
            run(
                &mut t,
                "thread/settings/updated",
                json!({"threadSettings": {"model": "gpt-6.1-sol"}})
            )
            .is_empty()
        );
        let events = run(
            &mut t,
            "thread/tokenUsage/updated",
            json!({"tokenUsage": {
                "last": {"inputTokens": 100, "cachedInputTokens": 20, "cacheWriteInputTokens": 5, "outputTokens": 30, "reasoningOutputTokens": 10, "totalTokens": 130},
                "modelContextWindow": 1000
            }}),
        );
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(
            &events[0],
            Event::Usage {
                context: Some(_),
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            Event::TokenUsage {
                model,
                input_tokens: 75,
                cached_input_tokens: 20,
                cache_write_input_tokens: 5,
                output_tokens: 30,
            } if model == "gpt-6.1-sol"
        ));
    }

    #[test]
    fn unknown_notifications_are_ignored() {
        let mut t = Translator::default();
        for method in [
            "account/rateLimits/updated",
            "thread/tokenUsage/updated",
            "something/new",
            "item/reasoning/textDelta",
        ] {
            assert!(
                run(&mut t, method, json!({"anything": 1})).is_empty(),
                "{method}"
            );
        }
    }
}
