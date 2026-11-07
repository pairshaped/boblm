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

fn summary_events(store: &Store, after: i64) -> Vec<String> {
    store
        .reader()
        .unwrap()
        .session_list_since(after, 256)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "subagents_changed")
        .map(|event| event.session)
        .collect()
}

#[test]
fn question_and_report_failure_changes_tell_the_root_list_once_each() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("changed.sqlite")).unwrap();
    let id = child(&store);
    let cursor = store
        .reader()
        .unwrap()
        .session_list_snapshot()
        .unwrap()
        .cursor;
    assert!(summary_events(&store, cursor).is_empty());
    store.ask("q", &id, "Which color?", &[], 3).unwrap();
    assert_eq!(summary_events(&store, cursor), ["parent"]);
    store.answer_question("q", &id, "Blue", 4).unwrap();
    assert_eq!(summary_events(&store, cursor), ["parent", "parent"]);
    store
        .record_event(&id, "subagent_report_failed", json!({"message": "gone"}), 5)
        .unwrap();
    assert_eq!(
        summary_events(&store, cursor),
        ["parent", "parent", "parent"]
    );
    // The root's own questions are not part of its subagent summary, so they stay quiet.
    store.ask("r", "parent", "Root question?", &[], 6).unwrap();
    assert_eq!(summary_events(&store, cursor).len(), 3);
}

#[test]
fn subagent_summary_failures_clear_on_archive_and_return_on_reopen() {
    for (kind, payload, failures, report_failures) in [
        (
            "agent_event",
            json!({"type": "result", "is_error": true, "subtype": "error_during_execution"}),
            1,
            0,
        ),
        ("driver_error", json!({"message": "fixture"}), 1, 0),
        (
            "subagent_report_failed",
            json!({"message": "fixture"}),
            0,
            1,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("summary.sqlite")).unwrap();
        let id = child(&store);
        store.record_event(&id, kind, payload, 3).unwrap();
        store.ask("q", &id, "Which color?", &[], 4).unwrap();
        for archived in [false, true, false] {
            store.set_archived(&id, archived).unwrap();
            let counts = store.reader().unwrap().subagent_summary("parent").unwrap();
            assert_eq!(counts.open, i64::from(!archived), "{kind}");
            assert_eq!(counts.questions, 1, "{kind}");
            assert_eq!(
                counts.failures,
                if archived { 0 } else { failures },
                "{kind}, archived={archived}"
            );
            assert_eq!(
                counts.report_failures,
                if archived { 0 } else { report_failures },
                "{kind}, archived={archived}"
            );
        }
    }
}
