use boblm_store::{NewMessage, Store};

#[test]
fn fresh_input_reopens_atomically_but_retries_and_invalid_uploads_do_not() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("reopen.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("chat", "/local", 1).unwrap();
    store.set_archived("chat", true).unwrap();
    let message = || NewMessage {
        uuid: "message",
        session: "chat",
        role: "user",
        content: "Hello",
        delivery_state: "queued",
        created_at: 2,
    };
    assert!(
        store
            .append_reopening(message(), &["missing".into()])
            .is_err()
    );
    assert!(store.reader().unwrap().sessions().unwrap()[0].archived);
    assert!(
        store
            .reader()
            .unwrap()
            .message("message")
            .unwrap()
            .is_none()
    );
    assert!(store.append_reopening(message(), &[]).unwrap());
    assert!(!store.reader().unwrap().sessions().unwrap()[0].archived);
    store.set_archived("chat", true).unwrap();
    assert!(!store.append_reopening(message(), &[]).unwrap());
    assert!(
        store
            .append_reopening(
                NewMessage {
                    content: "Changed",
                    ..message()
                },
                &[]
            )
            .is_err()
    );
    assert!(store.reader().unwrap().sessions().unwrap()[0].archived);
    assert!(
        store
            .append_reopening(
                NewMessage {
                    uuid: "second",
                    ..message()
                },
                &[]
            )
            .unwrap()
    );
    let reopened = Store::open(path).unwrap();
    assert!(!reopened.reader().unwrap().sessions().unwrap()[0].archived);
    assert!(
        reopened
            .reader()
            .unwrap()
            .message("second")
            .unwrap()
            .is_some()
    );
}
