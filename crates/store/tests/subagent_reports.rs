use boblm_store::{PeerSpawn, Store};
use serde_json::json;

fn child(store: &Store) -> String {
    store.create_session("parent", "/fake", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "parent")
        .and_then(|session| session.project_id)
        .unwrap();
    let id = "10000000-0000-4000-8000-000000000701";
    store
        .spawn_peer(PeerSpawn {
            request_id: id.into(),
            caller: "parent".into(),
            arguments: r#"{"title":"Worker","requestId":"10000000-0000-4000-8000-000000000701","role":"implementer","difficulty":"simple"}"#.into(),
            project_id: project,
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            title: "Worker".into(),
            content: "work".into(),
            created_at: 2,
            parent_session_id: Some("parent".into()),
            root_session_id: Some("parent".into()),
            role: Some("implementer".into()),
            resolved: None,
            working_directory: None,
            account_key: None,
        })
        .unwrap();
    id.into()
}

#[test]
fn result_events_queue_one_durable_report_and_old_sql_still_runs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reports.sqlite");
    let store = Store::open(&path).unwrap();
    let id = child(&store);
    for (created_at, text) in [(3, "first"), (4, "second")] {
        store
            .record_event(
                &id,
                "agent_event",
                json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}}),
                created_at,
            )
            .unwrap();
        store
            .record_event(
                &id,
                "agent_event",
                json!({"type":"result","subtype":"success","is_error":false}),
                created_at,
            )
            .unwrap();
    }
    let reports = store.reader().unwrap().pending_subagent_reports().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|report| report.parent_session == "parent")
    );
    assert_eq!(
        store
            .reader()
            .unwrap()
            .last_assistant_text(&id)
            .unwrap()
            .as_deref(),
        Some("second")
    );
    let first = &reports[0];
    store
        .mark_subagent_report_delivered(&first.child_session, first.result_seq, 5, None)
        .unwrap();
    let second = &reports[1];
    store
        .mark_subagent_report_failed(&second.child_session, second.result_seq, 6, "fixture")
        .unwrap();
    let reports = store.reader().unwrap().subagent_reports().unwrap();
    assert_eq!(reports[0].state, "delivered");
    assert_eq!(reports[1].state, "failed");
    assert_eq!(reports[1].error.as_deref(), Some("fixture"));
    drop(store);

    // The outbox survives reopening the database.
    let store = Store::open(&path).unwrap();
    assert_eq!(store.reader().unwrap().subagent_reports().unwrap().len(), 2);

    // The previous release's sessions SQL still runs after migration 038.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let fixture = include_str!("fixtures/sessions_a3fb372.sql");
    let sql = fixture
        .split("-- func: ")
        .find_map(|block| {
            let mut lines = block.lines();
            (lines.next()?.trim() == "list").then(|| {
                lines
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .trim_end_matches(';')
                    .to_owned()
            })
        })
        .unwrap();
    let mut statement = conn.prepare(&sql).unwrap();
    let mut rows = statement.query([]).unwrap();
    assert!(rows.next().unwrap().is_some());
}

#[test]
fn old_runner_event_sql_queues_errors_and_result_defaults_after_the_error_migration() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("old-runner.sqlite");
    let store = Store::open(&path).unwrap();
    let id = child(&store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let error = "Codex could not be started because its working folder is missing. Restore the folder and try again.";
    // A loaded runner still inserts the original five event columns.
    for seq in [100, 101] {
        conn.execute(
            "INSERT INTO events (session, seq, kind, payload, created_at) VALUES (?1, ?2, 'driver_error', ?3, 3)",
            rusqlite::params![id, seq, json!({"message":error}).to_string()],
        ).unwrap();
    }
    conn.execute(
        "INSERT INTO events (session, seq, kind, payload, created_at) VALUES ('parent', 100, 'driver_error', ?1, 3)",
        [json!({"message":error}).to_string()],
    ).unwrap();
    let report = store
        .reader()
        .unwrap()
        .pending_subagent_reports()
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(report.source_kind, "driver_error");
    assert_eq!(report.error_message.as_deref(), Some(error));
    store
        .mark_subagent_report_failed(&id, report.result_seq, 4, "The parent no longer exists.")
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    // Reopening and another explicit Resume cannot forget a reported error.
    conn.execute(
        "INSERT INTO events (session, seq, kind, payload, created_at) VALUES (?1, 102, 'driver_error', ?2, 5)",
        rusqlite::params![id, json!({"message":error}).to_string()],
    ).unwrap();
    assert_eq!(store.reader().unwrap().subagent_reports().unwrap().len(), 1);
    let report = store
        .reader()
        .unwrap()
        .subagent_reports()
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(report.error_message.as_deref(), Some(error));
    assert_eq!(
        report.error.as_deref(),
        Some("The parent no longer exists.")
    );
    // Different error text is actionable even without a successful start.
    conn.execute(
        "INSERT INTO events (session, seq, kind, payload, created_at) VALUES (?1, 103, 'driver_error', ?2, 6)",
        rusqlite::params![id, json!({"message":"Codex could not be started because the executable is missing. Restore Codex and try again."}).to_string()],
    ).unwrap();
    // The unchanged result trigger fills in the new kind's default.
    conn.execute(
        "INSERT INTO events (session, seq, kind, payload, created_at) VALUES (?1, 104, 'agent_event', ?2, 7)",
        rusqlite::params![id, json!({"type":"result","is_error":true}).to_string()],
    ).unwrap();
    let reports = store.reader().unwrap().subagent_reports().unwrap();
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[2].source_kind, "result");
    assert!(reports[2].error_message.is_none());
    // The loaded runner's original outbox update remains valid too.
    conn.execute(
        "UPDATE subagent_reports SET state = 'delivered', error = NULL, delivered_at = 8 WHERE child_session = ?1 AND result_seq = 104 AND state = 'pending'",
        [&id],
    ).unwrap();
}

#[test]
fn benign_restart_does_not_queue_reports_but_execution_errors_do() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("reports.sqlite")).unwrap();
    let id = child(&store);
    store
        .record_event(
            &id,
            "agent_event",
            json!({"type":"claude_turn_restart", "result":{
        "type":"result", "is_error":true, "subtype":"error_during_execution",
        "errors":["[ede_diagnostic] result_type=user"]}}),
            3,
        )
        .unwrap();
    assert!(
        store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .is_empty()
    );
    store
        .record_event(
            &id,
            "agent_event",
            json!({"type":"result", "is_error":true,
        "subtype":"error_during_execution"}),
            4,
        )
        .unwrap();
    assert_eq!(
        store
            .reader()
            .unwrap()
            .pending_subagent_reports()
            .unwrap()
            .len(),
        1
    );
}
