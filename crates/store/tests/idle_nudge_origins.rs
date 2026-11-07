use boblm_store::{NewMessage, Store};
use serde_json::{Value, json};

#[test]
fn idle_nudge_output_before_origin_evidence_suppresses_except_proven_old_codex_results() {
    let result = |id: &str| json!({"type":"tool_result","tool_use_id":id,"content":"done"});
    let tool = |id: &str| json!({"type":"tool_use","id":id,"name":"exec_command","input":{}});
    let user = |content: Value| json!({"type":"user","message":{"content":content}});
    let cases = [
        ("old result", user(json!([result("old")])), false, true),
        (
            "multiple old results",
            user(json!([result("old"), result("older")])),
            false,
            true,
        ),
        (
            "unknown result",
            user(json!([result("unknown")])),
            false,
            false,
        ),
        (
            "old and unknown results",
            user(json!([result("old"), result("unknown")])),
            false,
            false,
        ),
        (
            "current call",
            user(json!([result("current")])),
            true,
            false,
        ),
        ("reused ID", user(json!([result("old")])), true, false),
        (
            "old result and text",
            user(json!([result("old"), {"type":"text","text":"work"}])),
            false,
            false,
        ),
        ("empty content", user(json!([])), false, false),
        ("plain content", user(json!("work")), false, false),
        ("missing content", json!({"type":"user"}), false, false),
        (
            "scalar block",
            user(json!([result("old"), "work"])),
            false,
            false,
        ),
        ("null block", user(json!([null])), false, false),
        (
            "missing ID",
            user(json!([{"type":"tool_result"}])),
            false,
            false,
        ),
        (
            "null ID",
            user(json!([result("old"), {"type":"tool_result","tool_use_id":null}])),
            false,
            false,
        ),
        ("empty ID", user(json!([result("")])), false, false),
        (
            "numeric ID",
            user(json!([{"type":"tool_result","tool_use_id":1}])),
            false,
            false,
        ),
        (
            "unknown block",
            user(json!([{"type":"future"}])),
            false,
            false,
        ),
        (
            "assistant",
            json!({"type":"assistant","message":{"content":[]}}),
            false,
            false,
        ),
        (
            "stream",
            json!({"type":"stream_event","event":{}}),
            false,
            false,
        ),
    ];
    for agent in ["codex", "claude"] {
        for (name, output, current_call, stale) in &cases {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("db.sqlite")).unwrap();
            store.create_session("chat", "/local", 1).unwrap();
            let conn = rusqlite::Connection::open(store.path()).unwrap();
            conn.execute("UPDATE sessions SET agent = ?1 WHERE id = 'chat'", [agent])
                .unwrap();
            let record = |payload| {
                store
                    .record_event("chat", "agent_event", payload, 2)
                    .unwrap();
                conn.query_row(
                    "SELECT MAX(seq) FROM events WHERE session = 'chat'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
            };
            record(json!({"type":"assistant","message":{"content":[tool("old"),tool("older")]}}));
            record(json!({"type":"result"}));
            let first = if *current_call {
                Some(record(
                    json!({"type":"assistant","message":{"content":[tool("current"),tool("old")]}}),
                ))
            } else {
                None
            };
            let output_seq = record(output.clone());
            record(json!({"type":"background_jobs","snapshot":{"available":true,"tasks":[]}}));
            store
                .append_with_origin(
                    NewMessage {
                        uuid: "notice",
                        session: "chat",
                        role: "user",
                        content: "Background command is no longer listed as running: check",
                        delivery_state: "started",
                        created_at: 3,
                    },
                    &[],
                    Some("chat"),
                )
                .unwrap();
            record(json!({"type":"system","subtype":"turn_started"}));
            let ack = record(
                json!({"type":"command_lifecycle","command_uuid":"notice","state":"started"}),
            );
            let end = record(json!({"type":"result"}));
            let input = store
                .reader()
                .unwrap()
                .idle_nudge_input("chat", end)
                .unwrap()
                .unwrap();
            let eligible = agent == "codex" && *stale;
            assert_eq!(
                input.seq,
                if eligible {
                    ack
                } else {
                    first.unwrap_or(output_seq)
                },
                "{agent}: {name}"
            );
            assert_eq!(
                input.sender.as_deref(),
                eligible.then_some("chat"),
                "{agent}: {name}"
            );
        }
    }
}
