use boblm_store::Store;
use serde_json::json;

#[test]
fn migrations_032_and_033_backfill_dedupe_and_keep_the_aggregate_current() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("report.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("claude-chat", "/fake", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "claude-chat")
        .and_then(|session| session.project_id)
        .unwrap();
    for (id, agent, model) in [
        ("codex-chat", "codex", "gpt-6.1-sol"),
        ("pi-chat", "pi", "deepseek-ai/DeepSeek-V4.1-Flash-1017"),
    ] {
        store
            .create_session_in(boblm_store::NewSession {
                id,
                project_id: &project,
                name: "",
                agent,
                model,
                effort: None,
                created_at: 2,
            })
            .unwrap();
    }
    store
        .set_session_account("claude-chat", Some("claude-account"))
        .unwrap();

    // Put the database back before 032 so the migrations have history to backfill.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS usage_allowance_observation_after_event;
         DROP TABLE IF EXISTS usage_allowance_observation;
         DROP TRIGGER IF EXISTS usage_token_after_event;
         DROP TRIGGER IF EXISTS usage_model_call_after_event;
         DROP TRIGGER IF EXISTS usage_model_call_after_insert;
         DROP TRIGGER IF EXISTS usage_model_call_after_update;
         DROP TRIGGER IF EXISTS usage_allowance_after_event;
         DROP TABLE IF EXISTS usage_model_call;
         DROP TABLE IF EXISTS usage_token_daily;
         DROP TABLE IF EXISTS usage_allowance_daily;",
    )
    .unwrap();
    let day_one = 1_793_361_600_000_i64;
    let day_two = 1_793_448_000_000_i64;
    // Claude repeats the same message usage on each content-block event. The final
    // event carries the complete output count and must replace the earlier counts.
    for (offset, output) in [(0, 50), (1, 120), (2, 200)] {
        store
            .record_event(
                "claude-chat",
                "agent_event",
                json!({"type":"assistant","message":{"id":"c1","model":"sonnet","content":[],"usage":{"input_tokens":1000,"output_tokens":output,"cache_read_input_tokens":500,"cache_creation_input_tokens":100}}}),
                day_two + offset,
            )
            .unwrap();
    }
    // Pi normally emits once, but the same dedupe applies if a message repeats.
    for (offset, output) in [(0, 100), (1, 300)] {
        store
            .record_event(
                "pi-chat",
                "agent_event",
                json!({"type":"assistant","message":{"id":"p1","model":"deepseek-ai/DeepSeek-V4.1-Flash-1017","content":[],"usage":{"input_tokens":2000,"output_tokens":output,"cache_read_input_tokens":800,"cache_creation_input_tokens":0}}}),
                day_two + offset,
            )
            .unwrap();
    }
    store
        .record_event(
            "codex-chat",
            "agent_event",
            json!({"type":"token_usage","model":"gpt-6.1-sol","input_tokens":4000,"cached_input_tokens":1000,"cache_write_input_tokens":50,"output_tokens":400}),
            day_two,
        )
        .unwrap();
    for (used, at) in [(40.0, day_one), (42.0, day_two)] {
        store
            .record_event(
                "claude-chat",
                "agent_event",
                json!({"type":"usage","weekly":{"usedPercent":used,"resetsAt":1793800800}}),
                at,
            )
            .unwrap();
    }

    conn.execute_batch(include_str!("../migrations/032_usage_report.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/033_usage_model_calls.sql"))
        .unwrap();
    // Deleted chats can leave aggregate rows after their source events are gone.
    conn.execute_batch("INSERT INTO usage_model_call(session, call_id, provider, model, day, last_seq) VALUES ('deleted-chat', 'orphan', 'claude', 'sonnet', '2026-01-01', 1);").unwrap();
    conn.execute_batch(include_str!("../migrations/044_usage_timestamps.sql"))
        .unwrap();
    let calls = store
        .reader()
        .unwrap()
        .usage_token_calls(day_one, day_two + 100)
        .unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|row| row.created_at >= day_two));
    assert_eq!(
        store
            .reader()
            .unwrap()
            .usage_allowance_observations(day_one, day_two + 100)
            .unwrap()
            .len(),
        2
    );
    let reader = store.reader().unwrap();
    let tokens = reader.usage_token_days("2026-10-25", "2026-10-31").unwrap();
    assert_eq!(tokens.len(), 3);
    let claude = tokens.iter().find(|row| row.provider == "claude").unwrap();
    assert_eq!(
        (
            claude.input_tokens,
            claude.cache_read_tokens,
            claude.cache_write_tokens,
            claude.output_tokens,
            claude.calls
        ),
        (1000, 500, 100, 200, 1)
    );
    let pi = tokens.iter().find(|row| row.provider == "pi").unwrap();
    assert_eq!(
        (
            pi.input_tokens,
            pi.cache_read_tokens,
            pi.cache_write_tokens,
            pi.output_tokens,
            pi.calls
        ),
        (2000, 800, 0, 300, 1)
    );
    let codex = tokens.iter().find(|row| row.provider == "codex").unwrap();
    assert_eq!(
        (
            codex.input_tokens,
            codex.cache_read_tokens,
            codex.cache_write_tokens,
            codex.output_tokens,
            codex.calls
        ),
        (4000, 1000, 50, 400, 1)
    );
    let allowance = reader
        .usage_allowance_days("2026-10-25", "2026-10-31")
        .unwrap();
    assert_eq!(allowance.len(), 2);
    assert_eq!(allowance[1].weekly_used, 42.0);
    assert_eq!(allowance[1].samples, 1);

    // The installed triggers keep counting after the one-time backfill.
    store
        .record_event(
            "codex-chat",
            "agent_event",
            json!({"type":"token_usage","model":"gpt-6.1-sol","input_tokens":10,"cached_input_tokens":2,"cache_write_input_tokens":1,"output_tokens":3}),
            day_two,
        )
        .unwrap();
    // A later block for the same Claude message replaces its earlier output count.
    store
        .record_event(
            "claude-chat",
            "agent_event",
            json!({"type":"assistant","message":{"id":"c1","model":"sonnet","content":[],"usage":{"input_tokens":1000,"output_tokens":250,"cache_read_input_tokens":500,"cache_creation_input_tokens":100}}}),
            day_two + 10,
        )
        .unwrap();
    let tokens = reader.usage_token_days("2026-10-25", "2026-10-31").unwrap();
    let codex = tokens.iter().find(|row| row.provider == "codex").unwrap();
    assert_eq!(codex.input_tokens, 4010);
    assert_eq!(codex.calls, 2);
    let claude = tokens.iter().find(|row| row.provider == "claude").unwrap();
    assert_eq!(
        (
            claude.input_tokens,
            claude.cache_read_tokens,
            claude.cache_write_tokens,
            claude.output_tokens,
            claude.calls
        ),
        (1000, 500, 100, 250, 1)
    );
    let calls = reader.usage_token_calls(day_one, day_two + 100).unwrap();
    assert_eq!(calls.len(), 4);
    let claude_call = calls.iter().find(|row| row.provider == "claude").unwrap();
    assert_eq!(claude_call.created_at, day_two + 10);
    assert_eq!(claude_call.output_tokens, 250);
}

#[test]
fn migration_045_keeps_deleted_chat_calls_in_the_report() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("deleted-usage.sqlite");
    let store = Store::open(&path).unwrap();
    let noon = 1_793_448_000_000_i64;
    for session in ["deleted-chat", "retained-chat"] {
        store.create_session(session, "/fake", 1).unwrap();
        store
            .record_event(
                session,
                "agent_event",
                json!({
                    "type": "assistant", "message": {
                        "id": "call", "model": "sonnet", "usage": {"output_tokens": 123}
                    }
                }),
                noon + 120_000,
            )
            .unwrap();
    }
    store.set_archived("deleted-chat", true).unwrap();
    store.delete_session("deleted-chat").unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    // Reproduce 044's backfill after deletion has removed the source event.
    conn.execute(
        "UPDATE usage_model_call SET created_at = NULL WHERE session = 'deleted-chat'",
        [],
    )
    .unwrap();
    let missing: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM usage_model_call WHERE created_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(missing, 1);
    assert_eq!(
        store
            .reader()
            .unwrap()
            .usage_token_calls(noon - 3_600_000, noon + 3_600_000)
            .unwrap()
            .len(),
        1
    );

    conn.execute_batch(include_str!(
        "../migrations/045_usage_timestamp_fallback.sql"
    ))
    .unwrap();
    let calls = store
        .reader()
        .unwrap()
        .usage_token_calls(noon - 3_600_000, noon + 3_600_000)
        .unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].created_at, noon);
    assert_eq!(calls[0].output_tokens, 123);
    assert_eq!(calls[1].created_at, noon + 120_000);
    assert_eq!(calls[1].output_tokens, 123);
    let daily = store
        .reader()
        .unwrap()
        .usage_token_days("2026-10-31", "2026-10-31")
        .unwrap();
    assert_eq!(daily[0].calls, 2);
    assert_eq!(daily[0].output_tokens, 246);
}
