use boblm_store::{NewMessage, Store};

const HOUR: i64 = 3_600_000;

fn store_with(messages: &[(&str, &str, i64)]) -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("expiry.sqlite")).unwrap();
    store.create_session("s", "/fake", 1).unwrap();
    for (uuid, state, created_at) in messages {
        store
            .append(NewMessage {
                uuid,
                session: "s",
                role: "user",
                content: "text",
                delivery_state: "queued",
                created_at: *created_at,
            })
            .unwrap();
        match *state {
            "queued" => {}
            "uncertain" => {
                store.transition(uuid, "queued", "dispatching").unwrap();
                store.transition(uuid, "dispatching", "uncertain").unwrap();
            }
            "dispatching" => store.transition(uuid, "queued", "dispatching").unwrap(),
            "started" => {
                store.transition(uuid, "queued", "dispatching").unwrap();
                store.transition(uuid, "dispatching", "started").unwrap();
            }
            other => panic!("unexpected state {other}"),
        }
    }
    (temp, store)
}

#[test]
fn only_queued_and_uncertain_messages_past_the_cutoff_expire() {
    let now = 100 * HOUR;
    let cutoff = now - 6 * HOUR;
    let (_temp, store) = store_with(&[
        ("old-queued", "queued", cutoff - HOUR),
        ("old-uncertain", "uncertain", cutoff - HOUR),
        ("old-dispatching", "dispatching", cutoff - HOUR),
        ("old-started", "started", cutoff - HOUR),
        ("fresh-queued", "queued", now - 5 * HOUR),
        ("fresh-uncertain", "uncertain", now - 5 * HOUR),
    ]);
    let mut expired = store.expire_stale("s", now, cutoff).unwrap();
    expired.sort();
    assert_eq!(
        expired,
        [
            ("old-queued".to_owned(), "queued".to_owned()),
            ("old-uncertain".to_owned(), "uncertain".to_owned()),
        ]
    );
    let reader = store.reader().unwrap();
    let state = |uuid: &str| reader.message(uuid).unwrap().unwrap().delivery_state;
    assert_eq!(state("old-queued"), "expired");
    assert_eq!(state("old-uncertain"), "expired");
    // A message mid-dispatch, or already consumed, is never touched, however old.
    assert_eq!(state("old-dispatching"), "dispatching");
    assert_eq!(state("old-started"), "started");
    assert_eq!(state("fresh-queued"), "queued");
    assert_eq!(state("fresh-uncertain"), "uncertain");
    assert_eq!(
        reader
            .message("old-uncertain")
            .unwrap()
            .unwrap()
            .expired_from
            .as_deref(),
        Some("uncertain")
    );
    // Expired rows leave the dispatcher's working set and the delivery counts.
    let outstanding: Vec<_> = reader
        .outstanding_messages("s")
        .unwrap()
        .into_iter()
        .map(|m| m.uuid)
        .collect();
    assert_eq!(
        outstanding,
        ["old-dispatching", "fresh-queued", "fresh-uncertain"]
    );
    // The expiry is a durable event, and a second sweep has nothing left to do.
    let events = store.reader().unwrap().events_since("s", 0, 100).unwrap();
    let changes = events
        .iter()
        .filter(|e| {
            e.kind == "delivery_changed" && e.payload.contains("\"delivery_state\":\"expired\"")
        })
        .count();
    assert_eq!(changes, 2);
    assert!(store.expire_stale("s", now, cutoff).unwrap().is_empty());
}

#[test]
fn an_expired_message_cannot_be_dispatched_or_retried() {
    let now = 100 * HOUR;
    let (_temp, store) = store_with(&[("old", "queued", now - 7 * HOUR)]);
    store.expire_stale("s", now, now - 6 * HOUR).unwrap();
    assert!(store.transition("old", "queued", "dispatching").is_err());
    assert!(store.queue_retry("old", 0).is_err());
    assert_eq!(
        store
            .reader()
            .unwrap()
            .message("old")
            .unwrap()
            .unwrap()
            .delivery_state,
        "expired"
    );
}

#[test]
fn history_reports_the_expired_state_and_where_it_came_from() {
    let now = 100 * HOUR;
    let (_temp, store) = store_with(&[("old", "uncertain", now - 7 * HOUR)]);
    store.expire_stale("s", now, now - 6 * HOUR).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 50)
        .unwrap();
    let appended = page
        .events
        .iter()
        .find(|e| e.kind == "message_appended")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&appended.payload).unwrap();
    assert_eq!(payload["delivery_state"], "expired");
    assert_eq!(payload["expired_from"], "uncertain");
}
