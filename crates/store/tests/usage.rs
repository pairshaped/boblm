use boblm_store::Store;
use serde_json::json;

fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("usage.sqlite")).unwrap();
    for id in ["a", "b", "c"] {
        store.create_session(id, "/fake", 1).unwrap();
    }
    store.set_session_account("a", Some("account-1")).unwrap();
    store.set_session_account("b", Some("account-1")).unwrap();
    (temp, store)
}

#[test]
fn context_stays_per_chat_and_survives_history_eviction() {
    let (_temp, store) = setup();
    let sample = json!({"type":"usage","context":{"usedTokens":84000,"windowTokens":200000},"weekly":{"usedPercent":42,"resetsAt":2000000000}});
    store.record_event("a", "agent_event", sample, 2).unwrap();
    store
        .record_events(
            "a",
            "agent_event",
            (0..600)
                .map(|i| {
                    json!({"type":"assistant","message":{"id":format!("m{i}"),"content":[{"type":"text","text":"Later messages"}]}})
                })
                .collect(),
            3,
        )
        .unwrap();
    let reader = store.reader().unwrap();
    assert!(reader.usage_samples("a").unwrap().context.is_some());
    assert!(reader.usage_samples("b").unwrap().context.is_none());
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"system","subtype":"compact_boundary"}),
            4,
        )
        .unwrap();
    assert!(reader.usage_samples("a").unwrap().context.is_none());
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"usage","context":{"usedTokens":10,"windowTokens":100}}),
            5,
        )
        .unwrap();
    store.set_model_effort("a", "opus", None).unwrap();
    assert!(reader.usage_samples("a").unwrap().context.is_none());
}

#[test]
fn account_readings_are_latest_wins_and_keep_separate_observation_times() {
    let (_temp, store) = setup();
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"usage","five_hour":{"usedPercent":50,"resetsAt":2000000000}}),
            10_000,
        )
        .unwrap();
    let reader = store.reader().unwrap();
    let row = reader
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap();
    assert_eq!(row.five_hour.as_ref().unwrap().used_percent, 50.0);
    assert_eq!(row.five_hour.as_ref().unwrap().observed_at, 10_000);
    assert!(row.weekly.is_none());

    // An older observation never replaces a newer one.
    store
        .record_event(
            "b",
            "agent_event",
            json!({"type":"usage","five_hour":{"usedPercent":40,"resetsAt":2000000000}}),
            9_000,
        )
        .unwrap();
    let row = reader
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap();
    assert_eq!(row.five_hour.as_ref().unwrap().used_percent, 50.0);
    assert_eq!(row.five_hour.as_ref().unwrap().observed_at, 10_000);

    // A bonus reset can drop usage inside the same window; latest still wins.
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"usage","five_hour":{"usedPercent":30,"resetsAt":2000000000}}),
            20_000,
        )
        .unwrap();
    let row = reader
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap();
    assert_eq!(row.five_hour.as_ref().unwrap().used_percent, 30.0);
    assert_eq!(row.five_hour.as_ref().unwrap().observed_at, 20_000);
}

#[test]
fn an_expired_reset_stays_stored_until_the_provider_reports_a_new_window() {
    let (_temp, store) = setup();
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"usage","weekly":{"usedPercent":80,"resetsAt":1}}),
            10_000,
        )
        .unwrap();
    let row = store
        .reader()
        .unwrap()
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap();
    let weekly = row.weekly.unwrap();
    assert_eq!(weekly.used_percent, 80.0);
    assert_eq!(weekly.resets_at, Some(1));
    assert_eq!(weekly.observed_at, 10_000);
}

#[test]
fn an_unchanged_observation_is_skipped_until_the_confirmation_goes_stale() {
    let (_temp, store) = setup();
    let sample = json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}});
    store
        .record_event("a", "agent_event", sample.clone(), 1_000_000)
        .unwrap();
    let reader = store.reader().unwrap();
    let after_first = reader
        .account_usage("claude", "account-1")
        .unwrap()
        .unwrap()
        .weekly
        .unwrap()
        .observed_at;
    store
        .record_event("a", "agent_event", sample.clone(), 1_100_000)
        .unwrap();
    assert_eq!(
        reader
            .account_usage("claude", "account-1")
            .unwrap()
            .unwrap()
            .weekly
            .unwrap()
            .observed_at,
        after_first,
        "an unchanged write inside the refresh interval must not touch the row"
    );
    let refresh = i64::try_from(boblm_tuning::ACCOUNT_USAGE_CONFIRM_REFRESH.as_millis()).unwrap();
    store
        .record_event("a", "agent_event", sample, 1_000_000 + refresh + 1)
        .unwrap();
    assert_eq!(
        reader
            .account_usage("claude", "account-1")
            .unwrap()
            .unwrap()
            .weekly
            .unwrap()
            .observed_at,
        1_000_000 + refresh + 1,
        "an old confirmation is refreshed so staleness stays meaningful"
    );
}

#[test]
fn an_account_change_publishes_a_status_to_every_chat_on_the_account() {
    let (_temp, store) = setup();
    store
        .publish_summary("b", json!({"turnActive":false,"queued":0,"model":"sonnet"}))
        .unwrap();
    let before = store
        .reader()
        .unwrap()
        .events_since("b", 0, 100)
        .unwrap()
        .len();
    store
        .record_event(
            "a",
            "agent_event",
            json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}}),
            10_000,
        )
        .unwrap();
    let reader = store.reader().unwrap();
    let events = reader.events_since("b", before as i64, 100).unwrap();
    let summary = events
        .iter()
        .find(|event| event.kind == "state_summary")
        .expect("the idle chat on the same account must receive a status update");
    let payload: serde_json::Value = serde_json::from_str(&summary.payload).unwrap();
    assert_eq!(payload["accountUsage"]["accountId"], "account-1");
    assert_eq!(payload["accountUsage"]["weekly"]["usedPercent"], 42.0);
    assert_eq!(payload["turnActive"], false);
}

#[test]
fn a_chat_without_an_account_id_records_no_account_usage() {
    let (_temp, store) = setup();
    store
        .record_event(
            "c",
            "agent_event",
            json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}}),
            10_000,
        )
        .unwrap();
    let reader = store.reader().unwrap();
    assert!(reader.account_usage_for_session("c").unwrap().is_none());
    assert_eq!(reader.account_usages(Some("claude")).unwrap().len(), 0);
    assert!(reader.usage_samples("c").unwrap().context.is_none());
}

#[test]
fn per_chat_allowance_columns_and_triggers_are_gone_and_context_still_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("clean.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("a", "/fake", 1).unwrap();
    store
        .record_event(
            "a",
            "agent_event",
            json!({
                "type":"usage",
                "context":{"usedTokens":84000,"windowTokens":200000},
                "weekly":{"usedPercent":42,"resetsAt":2000001000}
            }),
            2,
        )
        .unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('session_usage') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns, ["session", "context", "context_at"]);
    let extra: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'remember_additional_allowances'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(extra, 0);
    let sample = store.reader().unwrap().usage_samples("a").unwrap();
    assert_eq!(sample.context.unwrap().1, 2);
}
