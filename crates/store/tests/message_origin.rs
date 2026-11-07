use boblm_store::{NewMessage, Store};

#[test]
fn peer_origin_survives_reload_and_is_part_of_retry_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("origin.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("chat", "/local", 1).unwrap();
    let message = || NewMessage {
        uuid: "message",
        session: "chat",
        role: "user",
        content: "Work to do",
        delivery_state: "queued",
        created_at: 1,
    };
    assert!(
        store
            .append_with_origin(message(), &[], Some("sender"))
            .unwrap()
    );
    assert!(
        !store
            .append_with_origin(message(), &[], Some("sender"))
            .unwrap()
    );
    assert!(
        store
            .append_with_origin(message(), &[], Some("other"))
            .is_err()
    );
    assert!(store.append(message()).is_err());
    let legacy = || NewMessage {
        uuid: "legacy",
        ..message()
    };
    assert!(store.append(legacy()).unwrap());
    assert!(
        !store
            .append_with_origin(legacy(), &[], Some("sender"))
            .unwrap()
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    let events = store
        .reader()
        .unwrap()
        .events_since("chat", 0, 100)
        .unwrap();
    let event = events
        .iter()
        .find(|event| event.kind == "message_appended")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
    assert_eq!(payload["sender_session_id"], "sender");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "message_appended")
            .count(),
        2
    );
}

#[test]
fn last_peer_sender_ignores_messages_the_session_sent_itself() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("sender.sqlite")).unwrap();
    store.create_session("chat", "/local", 1).unwrap();
    let message = |uuid| NewMessage {
        uuid,
        session: "chat",
        role: "user",
        content: "text",
        delivery_state: "queued",
        created_at: 1,
    };
    assert_eq!(
        store.reader().unwrap().last_peer_sender("chat").unwrap(),
        None
    );
    store
        .append_with_origin(message("peer"), &[], Some("peer-chat"))
        .unwrap();
    store
        .append_with_origin(message("self"), &[], Some("chat"))
        .unwrap();
    assert_eq!(
        store.reader().unwrap().last_peer_sender("chat").unwrap(),
        Some("peer-chat".to_owned())
    );
}
