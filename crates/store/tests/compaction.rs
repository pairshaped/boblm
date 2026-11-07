use boblm_store::Store;
use serde_json::{Value, json};

#[test]
fn compaction_history_replaces_start_across_pages_and_reloads() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("compact.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("s", "/local", 1).unwrap();
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"system","subtype":"status","status":"compacting"}),
            2,
        )
        .unwrap();
    let started = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    let first: Value = serde_json::from_str(&started.events[0].payload).unwrap();
    assert_eq!(first["subtype"], "compact_started");
    let id = first["compaction_id"].clone();
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"assistant","message":{"id":"a","content":[]}}),
            3,
        )
        .unwrap();
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"system","subtype":"compact_boundary"}),
            4,
        )
        .unwrap();
    let store = Store::open(path).unwrap();
    let latest = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    assert_eq!(latest.events.len(), 1);
    let older = store
        .reader()
        .unwrap()
        .history_page("s", Some(latest.events[0].seq), false, 1)
        .unwrap();
    let finished: Value = serde_json::from_str(&older.events[0].payload).unwrap();
    assert_eq!(finished["subtype"], "compact_boundary");
    assert_eq!(finished["compaction_id"], id);
    assert_eq!(older.events[0].seq, started.events[0].seq);
    let forwards = store
        .reader()
        .unwrap()
        .history_page("s", Some(0), true, 50)
        .unwrap();
    assert_eq!(forwards.events.len(), 2);
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"system","subtype":"compact_started"}),
            5,
        )
        .unwrap();
    store
        .record_event(
            "s",
            "agent_event",
            json!({"type":"result","is_error":true}),
            6,
        )
        .unwrap();
    let failed = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    let failed: Value = serde_json::from_str(&failed.events[0].payload).unwrap();
    assert_eq!(failed["subtype"], "compact_interrupted");
    assert_ne!(failed["compaction_id"], id);
}
