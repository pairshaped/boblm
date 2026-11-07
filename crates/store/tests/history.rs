use boblm_store::Store;
use serde_json::json;

#[test]
fn history_pages_skip_transport_deltas_keep_streams_and_move_in_both_directions() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    let mut events = Vec::new();
    for i in 0..120 {
        events.push(json!({"type":"stream_event","event":{"type":"message_start","message":{"id":format!("a{i}")}}}));
        for _ in 0..100 {
            events.push(json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"text":"x"}}}));
        }
        events.push(json!({"type":"assistant","message":{"id":format!("a{i}"),"content":[{"type":"text","text":format!("message {i}")}]}}));
    }
    store.record_events("s", "agent_event", events, 1).unwrap();
    let reader = store.reader().unwrap();
    let latest = reader.history_page("s", None, false, 50).unwrap();
    assert_eq!(latest.events.len(), 50);
    assert!(latest.has_more);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&latest.events[0].payload).unwrap()["message"]["id"],
        "a70"
    );
    assert!(latest.events.iter().all(|e| {
        serde_json::from_str::<serde_json::Value>(&e.payload).unwrap()["type"] == "assistant"
    }));
    let older = reader
        .history_page("s", Some(latest.events[0].seq), false, 50)
        .unwrap();
    assert_eq!(older.events.len(), 50);
    assert!(older.events.last().unwrap().seq < latest.events[0].seq);
    let newer = reader
        .history_page("s", Some(older.events.last().unwrap().seq), true, 50)
        .unwrap();
    assert_eq!(
        newer.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        latest.events.iter().map(|e| e.seq).collect::<Vec<_>>()
    );
    assert!(!newer.has_more);
    // A partial reply is available immediately without replaying its deltas to the browser.
    store.record_events("s", "agent_event", vec![
        json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"partial"}}}),
        json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"text":"still "}}}),
        json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"text":"typing"}}}),
    ], 2).unwrap();
    let tail = reader.history_page("s", None, false, 1).unwrap();
    let partial: serde_json::Value = serde_json::from_str(&tail.events[0].payload).unwrap();
    assert_eq!(partial["history_streaming"], true);
    assert_eq!(partial["message"]["content"][0]["text"], "still typing");
    assert!(tail.cursor > latest.cursor);
}

#[test]
fn history_hydrates_delivery_and_tool_names_from_outside_the_page() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    store
        .append(boblm_store::NewMessage {
            uuid: "u",
            session: "s",
            role: "user",
            content: "hello",
            delivery_state: "queued",
            created_at: 1,
        })
        .unwrap();
    store.transition("u", "queued", "dispatching").unwrap();
    store.transition("u", "dispatching", "started").unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&page.events[0].payload).unwrap()["delivery_state"],
        "started"
    );
    store.record_events("s","agent_event",vec![
        json!({"type":"assistant","message":{"id":"call","content":[{"type":"text","text":"Reading"},{"type":"tool_use","id":"tool","name":"Read","input":{}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"output"}]}}),
    ],2).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&page.events[0].payload).unwrap()["message"]["content"]
            [0]["tool_name"],
        "Read"
    );
    // Unknown content shapes remain tolerable at the database trigger boundary.
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"assistant","message":{"content":"plain text"}}),
            3,
        )
        .unwrap();
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"assistant","message":{"content":["unknown",null,5]}}),
            3,
        )
        .unwrap();
}

#[test]
fn history_keeps_pending_credentials_outside_the_message_window() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    for id in ["pending", "closed"] {
        store
            .create_secret_request(
                boblm_store::NewSecretRequest {
                    id,
                    session: "s",
                    file: "/local/test-secrets.yaml",
                    key: &["key".into()],
                    expires_at: 100000,
                    created_at: 1,
                },
                5,
            )
            .unwrap();
    }
    store
        .finish_secret_request("closed", "canceled", 2)
        .unwrap();
    store.record_events("s","agent_event",(0..100).map(|i|json!({"type":"assistant","message":{"id":format!("a{i}"),"content":[{"type":"text","text":"response"}]}})).collect(),3).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.context.len(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&page.context[0].payload).unwrap()["request_id"],
        "pending"
    );
    assert!(page.context[0].seq < page.events[0].seq);
}

#[test]
fn latest_tool_bursts_include_readable_context_within_the_retained_budget() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    store.record_event("s","agent_event",json!({"type":"assistant","message":{"id":"explanation","content":[{"type":"text","text":"Here is the plan before I run the tools."}]}}),2).unwrap();
    let mut tools = Vec::new();
    for i in 0..140 {
        // Claude's stream starts are hydrated before deciding whether the window is tool-only.
        tools.push(json!({"type":"stream_event","event":{"type":"message_start","message":{"id":format!("tool-{i}")}}}));
        tools.push(json!({"type":"assistant","message":{"id":format!("tool-{i}"),"content":[{"type":"tool_use","id":format!("call-{i}"),"name":"Read","input":{"path":"file.rs"}}]}}));
    }
    store.record_events("s", "agent_event", tools, 3).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, i64::from(boblm_tuning::HISTORY_PAGE_SIZE))
        .unwrap();
    assert!(
        page.events
            .iter()
            .any(|event| event.payload.contains("Here is the plan"))
    );
    assert!(page.events.len() > usize::from(boblm_tuning::HISTORY_PAGE_SIZE));
    assert!(page.events.len() <= boblm_tuning::HISTORY_MAX_ROWS);
    assert!(page.cursor >= page.events.last().unwrap().seq);
    let more=(0..boblm_tuning::HISTORY_MAX_ROWS + 50).map(|i|json!({"type":"assistant","message":{"id":format!("extra-{i}"),"content":[{"type":"tool_use","id":format!("extra-call-{i}"),"name":"Read","input":{}}]}})).collect();
    store.record_events("s", "agent_event", more, 4).unwrap();
    let capped = store
        .reader()
        .unwrap()
        .history_page("s", None, false, i64::from(boblm_tuning::HISTORY_PAGE_SIZE))
        .unwrap();
    assert_eq!(capped.events.len(), boblm_tuning::HISTORY_MAX_ROWS);
    assert!(capped.has_more);
}

#[test]
fn subagent_context_survives_old_launches_and_omits_prompts_and_tool_output() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("history.sqlite")).unwrap();
    store.create_session("s", "/project", 1).unwrap();
    store.record_events("s","agent_event",vec![
      json!({"type":"assistant","message":{"id":"launch","content":[{"type":"tool_use","id":"agent-1","name":"Agent","input":{"description":"Check delivery","prompt":"PRIVATE LAUNCH PROMPT"}}]}}),
      json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent-1","content":"LARGE PRIVATE RESULT","is_error":false}]}}),
      json!({"type":"assistant","message":{"id":"activity","content":[{"type":"tool_use","id":"status","name":"subAgentActivity","input":{"agentPath":"/root/reviewer","agentThreadId":"child","kind":"started","extra":"PRIVATE EXTRA"}}]}}),
    ],2).unwrap();
    store.record_events("s","agent_event",(0..700).map(|i|json!({"type":"assistant","message":{"id":format!("message-{i}"),"content":[{"type":"text","text":"Later conversation"}]}})).collect(),3).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 50)
        .unwrap();
    assert!(
        !page
            .events
            .iter()
            .any(|event| event.payload.contains("Check delivery"))
    );
    assert_eq!(page.context.len(), 3);
    let context = page
        .context
        .iter()
        .map(|event| event.payload.as_str())
        .collect::<String>();
    assert!(context.contains("Check delivery"));
    assert!(context.contains("reviewer"));
    assert!(!context.contains("PRIVATE"));
    drop(store);
    let reopened = Store::open(temp.path().join("history.sqlite")).unwrap();
    assert_eq!(
        reopened
            .reader()
            .unwrap()
            .history_page("s", None, false, 50)
            .unwrap()
            .context
            .len(),
        3
    );
}
