use boblm_protocol::client::*;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

fn ts(schema: &Value) -> Result<String, Box<dyn std::error::Error>> {
    if schema == &Value::Bool(true) || schema.as_object().is_some_and(|object| object.is_empty()) {
        return Ok("JsonValue".into());
    }
    if schema == &Value::Bool(false) {
        return Ok("never".into());
    }
    let object = schema.as_object().ok_or("invalid schema")?;
    let supported = [
        "$ref",
        "$schema",
        "$defs",
        "title",
        "description",
        "type",
        "const",
        "enum",
        "oneOf",
        "anyOf",
        "allOf",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "pattern",
        "format",
        "default",
    ];
    for key in object.keys() {
        if !supported.contains(&key.as_str()) {
            return Err(format!("unsupported schema keyword {key}").into());
        }
    }
    if let Some(format) = schema["format"].as_str()
        && ![
            "uint8", "uint16", "uint32", "uint64", "int8", "int16", "int32", "int64", "float",
            "double",
        ]
        .contains(&format)
    {
        return Err(format!("unsupported schema format {format}").into());
    }
    if let Some(reference) = schema["$ref"].as_str() {
        return Ok(reference.rsplit('/').next().ok_or("invalid ref")?.into());
    }
    if let Some(value) = object.get("const") {
        return Ok(value.to_string());
    }
    if let Some(values) = schema["enum"].as_array() {
        return Ok(values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join(" | "));
    }
    for (key, join) in [("oneOf", " | "), ("anyOf", " | "), ("allOf", " & ")] {
        if let Some(values) = schema[key].as_array() {
            return Ok(format!(
                "({})",
                values
                    .iter()
                    .map(ts)
                    .collect::<Result<Vec<_>, _>>()?
                    .join(join)
            ));
        }
    }
    if let Some(types) = schema["type"].as_array() {
        return Ok(types
            .iter()
            .map(|kind| {
                let mut branch = schema.clone();
                branch["type"] = kind.clone();
                ts(&branch)
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(" | "));
    }
    Ok(match schema["type"].as_str() {
        Some("string") => "string".into(),
        Some("number" | "integer") => "number".into(),
        Some("boolean") => "boolean".into(),
        Some("null") => "null".into(),
        Some("array") => format!(
            "ReadonlyArray<{}>",
            ts(schema.get("items").unwrap_or(&Value::Bool(true)))?
        ),
        Some("object") => {
            let required: BTreeSet<&str> = schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let fields = schema["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(key, value)| {
                    Ok(format!(
                        "readonly {}{}: {};",
                        serde_json::to_string(key)?,
                        if required.contains(key.as_str()) {
                            ""
                        } else {
                            "?"
                        },
                        ts(value)?
                    ))
                })
                .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
            if fields.is_empty() && schema["additionalProperties"] == false {
                "Readonly<Record<string, never>>".into()
            } else {
                format!("{{ {} }}", fields.join(" "))
            }
        }
        _ => return Err(format!("unsupported schema {schema}").into()),
    })
}
fn publish(path: &Path, bytes: &str, check: bool) -> Result<(), Box<dyn std::error::Error>> {
    if check {
        if std::fs::read_to_string(path).ok().as_deref() != Some(bytes) {
            return Err(format!("stale generated contract: {}; run cargo run -j4 -p boblm-protocol --example protocol-generate", path.display()).into());
        }
    } else {
        std::fs::create_dir_all(path.parent().ok_or("missing output parent")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let check = std::env::args().any(|arg| arg == "--check");
    let mut schema = serde_json::to_value(schemars::schema_for!(ContractSchemas))?;
    // The browser decoder reads its size limit from the schema so it cannot disagree with the server.
    schema["x-maxBodyBytes"] = boblm_tuning::MAX_BODY_BYTES.into();
    let definitions = schema["$defs"]
        .as_object()
        .ok_or("missing schema definitions")?;
    let mut output = String::from(
        "// Generated from Rust serde types. Run the protocol-generate example to update.\nimport { decode } from '../decoder.ts';\nexport type JsonValue = null | boolean | number | string | ReadonlyArray<JsonValue> | { readonly [key: string]: JsonValue };\n",
    );
    for (name, definition) in definitions {
        output.push_str(&format!("export type {name} = {};\n", ts(definition)?));
    }
    output.push_str(&format!(
        "export const contractSchema = {} as const;\n",
        serde_json::to_string_pretty(&schema)?
    ));
    for name in [
        "Empty",
        "SessionRef",
        "CreateSession",
        "RenameSession",
        "SendMessage",
        "AttachmentInfo",
        "RetryMessage",
        "SetModel",
        "AddProject",
        "RenameProject",
        "ListDirectories",
        "DirectoryListing",
        "RemoveProject",
        "ReorderProjects",
        "ReorderChats",
        "PushConfigReply",
        "SubscribePush",
        "UnsubscribePush",
        "ChatPresence",
        "ChatPresenceReply",
        "QuestionInfo",
        "QuestionState",
        "DiscussQuestionLater",
        "QuestionsReply",
        "AnswerQuestion",
        "SubmitSecret",
        "SecretReply",
        "ProjectInfo",
        "ErrorInfo",
        "ListSessionsResponse",
        "SessionInfo",
        "SessionStatus",
        "SendMessageResponse",
        "CommandResponse",
        "ChatHistory",
        "ChatHistoryPage",
        "SubagentHistory",
        "SubagentHistoryPage",
        "EventPage",
        "DeliveryState",
        "StoredEvent",
        "Sequence",
        "SessionListEvent",
    ] {
        output.push_str(&format!("export function parse{name}(value: unknown): {name} {{ return decode(contractSchema, {name:?}, value) as {name}; }}\n"));
    }
    output.push_str(&format!(
        "export const limits = {{ maxChatNameChars: {}, maxAttachments: {}, maxBodyBytes: {}, maxUploadBytes: {}, maxMessageChars: {}, defaultModel: {:?}, defaultEffort: {:?}, historyPageSize: {}, historyMaxRows: {}, historyResumeLatestMs: {}, historyScrollIntentMs: {}, historyScrollThreshold: {}, longPressMs: {}, longPressMovePx: {}, maxProjectNameChars: {}, notificationHeartbeatMs: {}, browserConnectTimeoutMs: {}, subagentPollMs: {}, jobsVisibleLimit: {}, queuedMessageMaxAgeHours: {}, usageHighPercent: {} }} as const;\n",
        boblm_tuning::MAX_CHAT_NAME_CHARS,
        boblm_tuning::MAX_ATTACHMENTS,
        boblm_tuning::MAX_BODY_BYTES,
        boblm_tuning::MAX_UPLOAD_BYTES,
        boblm_tuning::MAX_MESSAGE_CHARS,
        boblm_tuning::DEFAULT_MODEL,
        boblm_tuning::DEFAULT_EFFORT,
        boblm_tuning::HISTORY_PAGE_SIZE,
        boblm_tuning::HISTORY_MAX_ROWS,
        boblm_tuning::HISTORY_RESUME_LATEST_MS,
        boblm_tuning::HISTORY_SCROLL_INTENT_MS,
        boblm_tuning::HISTORY_SCROLL_THRESHOLD,
        boblm_tuning::LONG_PRESS_MS,
        boblm_tuning::LONG_PRESS_MOVE_PX,
        boblm_tuning::MAX_PROJECT_NAME_CHARS,
        boblm_tuning::NOTIFICATION_HEARTBEAT_MS,
        boblm_tuning::BROWSER_CONNECT_TIMEOUT_MS,
        boblm_tuning::SUBAGENT_POLL_MS,
        boblm_tuning::JOBS_VISIBLE_LIMIT,
        boblm_tuning::QUEUED_MESSAGE_MAX_AGE.as_secs() / 3600,
        boblm_tuning::USAGE_HIGH_PERCENT
    ));
    publish(&root.join("web/src/generated/contract.ts"), &output, check)?;
    let status = SessionStatus {
        account_key: Some("claude-plan-a".into()),
        prompt_suggestion: None,
        jobs: None,
        usage: None,
        account_usage: Some(boblm_protocol::usage::AccountUsage {
            provider: "claude".into(),
            account_id: "account-1".into(),
            five_hour: Some(boblm_protocol::usage::AllowanceReading {
                used_percent: 80.0,
                resets_at: Some(2_000_000_000),
                observed_at: 1,
            }),
            weekly: None,
            credits: Some(boblm_protocol::usage::CreditsReading {
                balance: Some(40000.0),
                observed_at: 1,
            }),
        }),
        selection_pending: true,
        held_reason: Some("Claude could not be started, so the queued messages are held.".into()),
        queue_held: true,
        running: false,
        turn_active: false,
        stopping: false,
        queued: 2,
        dispatching: 0,
        uncertain: 1,
        rejected: 0,
        archived: false,
        model: "sonnet".into(),
        effort: Some("high".into()),
    };
    let session = SessionInfo {
        project_id: "20000000-0000-4000-8000-000000000001".into(),
        agent: Agent::Claude,
        name: "Fixture session".into(),
        session_id: "s".into(),
        working_directory: "/host/project".into(),
        agent_session_id: Some("claude-id".into()),
        parent_session_id: None,
        root_session_id: None,
        role: None,
        subagents: Some(SubagentSummary {
            open: 2,
            questions: 1,
            failures: 0,
            report_failures: 1,
        }),
        created_at: 1,
        status: status.clone(),
    };
    let mut fixtures = Vec::new();
    fixtures.push(json!({"type":"ChatPresenceReply","value":{"reloadId":"10000000-0000-4000-8000-000000000001"}}));

    for (name, value) in [
        ("Empty", json!({})),
        ("ListDirectories", json!({"path":null})),
        (
            "DirectoryListing",
            json!({"path":"/home/me", "parent":null,"directories":[{"name":"projects","path":"/home/me/projects"}]}),
        ),
        ("SessionRef", json!({"sessionId":"s"})),
        (
            "CreateSession",
            json!({"sessionId":"s","projectId":"20000000-0000-4000-8000-000000000001","model":"sonnet","effort":null}),
        ),
        (
            "SendMessage",
            json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001","content":"hello"}),
        ),
        (
            "SendMessage",
            json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000002","content":"","attachments":["10000000-0000-4000-8000-000000000001"]}),
        ),
        (
            "AttachmentInfo",
            json!({"id":"10000000-0000-4000-8000-000000000001","name":"Screenshot.png","contentType":"image/png","size":380928}),
        ),
        (
            "RetryMessage",
            json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001"}),
        ),
        (
            "SetModel",
            json!({"sessionId":"s","model":"opus","effort":"high"}),
        ),
        (
            "AddProject",
            json!({"projectId":"20000000-0000-4000-8000-000000000001","path":"/host/project","name":"project"}),
        ),
        (
            "AddProject",
            json!({"projectId":"20000000-0000-4000-8000-000000000002","path":"/host/other","name":null}),
        ),
        (
            "RemoveProject",
            json!({"projectId":"20000000-0000-4000-8000-000000000001"}),
        ),
        (
            "ReorderProjects",
            json!({"projectIds":["20000000-0000-4000-8000-000000000002","20000000-0000-4000-8000-000000000001"]}),
        ),
        (
            "ReorderChats",
            json!({"projectId":"20000000-0000-4000-8000-000000000001","sessionIds":["b","a"]}),
        ),
        (
            "SubmitSecret",
            json!({"requestId":"20000000-0000-4000-8000-000000000001","value":"example-only","cancel":null}),
        ),
        (
            "SubmitSecret",
            json!({"requestId":"20000000-0000-4000-8000-000000000001","value":null,"cancel":true}),
        ),
        ("SecretReply", json!({"status":"saved","message":"Saved."})),
        (
            "ProjectInfo",
            json!({"projectId":"20000000-0000-4000-8000-000000000001","path":"/host/project","name":"project","createdAt":1}),
        ),
        (
            "ErrorInfo",
            json!({"code":"invalid_request","message":"invalid UUID"}),
        ),
    ] {
        fixtures.push(json!({"type":name,"value":value}));
    }
    for (name, value) in [
        (
            "ListSessionsResponse",
            serde_json::to_value(ListSessionsResponse {
                cursor: Sequence(123),
                models: model_catalog(),
                projects: vec![ProjectInfo {
                    project_id: "20000000-0000-4000-8000-000000000001".into(),
                    path: "/host/project".into(),
                    name: "project".into(),
                    created_at: 1,
                }],
                sessions: vec![session.clone()],
            })?,
        ),
        ("SessionInfo", serde_json::to_value(session)?),
        (
            "CommandResponse",
            serde_json::to_value(CommandResponse {
                status: status.clone(),
            })?,
        ),
        (
            "SendMessageResponse",
            serde_json::to_value(SendMessageResponse {
                message_id: "00000000-0000-4000-8000-000000000001".into(),
                accepted: false,
                status: status.clone(),
            })?,
        ),
        (
            "ChatHistory",
            json!({"sessionId":"s","before":null,"after":null}),
        ),
        (
            "ChatHistoryPage",
            serde_json::to_value(ChatHistoryPage {
                session_id: "s".into(),
                partial_id: None,
                status: status.clone(),
                cursor: Sequence(0),
                events: vec![],
                context: vec![],
                has_more: false,
            })?,
        ),
        (
            "EventPage",
            serde_json::to_value(EventPage {
                session_id: "s".into(),
                cursor: Sequence::try_from(1)?,
                events: vec![serde_json::from_value(
                    json!({"sessionId":"s", "seq":1, "kind":"delivery_changed", "payload":{"uuid":"m", "delivery_state":"uncertain", "retry_count":null}, "createdAt":1}),
                )?],
                status: status.clone(),
                has_more: false,
            })?,
        ),
    ] {
        fixtures.push(json!({"type":name,"value":value}));
    }
    for (kind, payload) in [
        (
            "created",
            json!({"id":"s","working_directory":"/host/project","parent_session_id":null,"root_session_id":null,"role":null}),
        ),
        ("renamed", json!({"name":"Work"})),
        ("archived", json!({"archived":true})),
        ("state_summary", serde_json::to_value(&status)?),
        ("subagents_changed", json!({})),
        ("questions_changed", json!({})),
    ] {
        let event: SessionListEvent = serde_json::from_value(
            json!({"sessionId":"s","seq":1,"createdAt":1,"kind":kind,"payload":payload}),
        )?;
        fixtures.push(json!({"type":"SessionListEvent","value":event}));
    }
    fixtures.push(json!({"type":"StoredEvent","value":{"sessionId":"s","seq":1,"createdAt":1,"kind":"state_summary","payload":status}}));
    for state in [
        DeliveryState::Queued,
        DeliveryState::Dispatching,
        DeliveryState::Started,
        DeliveryState::Rejected,
        DeliveryState::Uncertain,
        DeliveryState::Expired,
    ] {
        fixtures.push(json!({"type":"DeliveryState", "value":state}));
    }
    for (kind, payload) in [
        ("session_renamed", json!({"name":"Fixture session"})),
        (
            "session_created",
            json!({"id":"s","working_directory":"/host/project","parent_session_id":null,"root_session_id":null,"role":null}),
        ),
        (
            "message_appended",
            json!({"uuid":"m","role":"user","content":"hello","delivery_state":"queued"}),
        ),
        (
            "message_appended",
            json!({"uuid":"m2","role":"user","content":"look","delivery_state":"queued","attachments":[{"id":"10000000-0000-4000-8000-000000000001","name":"Screenshot.png","contentType":"image/png","size":380928}]}),
        ),
        (
            "delivery_changed",
            json!({"uuid":"m","delivery_state":"started","retry_count":1}),
        ),
        (
            "delivery_changed",
            json!({"uuid":"m","delivery_state":"expired","expired_from":"uncertain","retry_count":null}),
        ),
        (
            "agent_session_changed",
            json!({"agent_session_id":"claude-id","previous":null}),
        ),
        (
            "agent_session_changed",
            json!({"agent_session_id":"claude-id-2","previous":"claude-id"}),
        ),
        ("archive_changed", json!({"archived":true})),
        (
            "model_effort_changed",
            json!({"model":"opus","effort":"high"}),
        ),
        (
            "delivery_rejected",
            json!({"uuid":"m","retry_available":true}),
        ),
        (
            "agent_event",
            json!({"type":"future_agent_event","extra":42}),
        ),
        (
            "secret_requested",
            json!({"request_id":"20000000-0000-4000-8000-000000000001","file":"/home/me/secrets.yaml","key":["db","password"],"expires_at":1}),
        ),
        (
            "secret_resolved",
            json!({"request_id":"20000000-0000-4000-8000-000000000001","status":"saved"}),
        ),
        ("driver_error", json!({"message":"process exited"})),
        ("model_error", json!({"message":"control refused"})),
        ("session_stopping", json!({})),
        ("session_stopped", json!({})),
        ("delivery_timeout", json!({})),
        ("stop_timeout", json!({})),
    ] {
        let event: StoredEvent = serde_json::from_value(
            json!({"sessionId":"s","seq":1,"kind":kind,"payload":payload,"createdAt":1}),
        )?;
        fixtures.push(json!({"type":"StoredEvent","value":event}));
    }
    publish(
        &root.join("web/src/generated/fixtures.json"),
        &(serde_json::to_string_pretty(&fixtures)? + "\n"),
        check,
    )?;
    Ok(())
}
