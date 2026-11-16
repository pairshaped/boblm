//! The manual move's transaction checks that the chat is still archived, still on the
//! source account and without accepted input, so a message accepted mid-move wins.
use boblm_store::{ClaudeHandoff, NewMessage, Store};

const A: &str = "claude-plan-a";
const B: &str = "claude-plan-b";

fn change() -> ClaudeHandoff {
    ClaudeHandoff {
        source: B.into(),
        target: A.into(),
        account_id: Some("account-a".into()),
        source_reset: 4_000_000_000,
    }
}
fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("move.sqlite")).unwrap();
    store.create_session("s", "/fake", 1).unwrap();
    store.set_claude_account_key("s", B).unwrap();
    (dir, store)
}
fn key(store: &Store) -> Option<String> {
    store.reader().unwrap().sessions().unwrap()[0]
        .account_key
        .clone()
}

#[test]
fn the_move_switches_account_identity_and_exclusion_together_only_when_archived_and_quiet() {
    let (_dir, store) = store();
    // An open chat is never moved: an owner message may have reopened it.
    assert!(!store.move_claude_account("s", &change(), 10).unwrap());
    assert_eq!(key(&store).as_deref(), Some(B));
    store.set_archived("s", true).unwrap();
    // Accepted input that has not been consumed blocks the move.
    store
        .append(NewMessage {
            uuid: "00000000-0000-4000-8000-000000000001",
            session: "s",
            role: "user",
            content: "hello",
            delivery_state: "queued",
            created_at: 5,
        })
        .unwrap();
    assert!(!store.move_claude_account("s", &change(), 10).unwrap());
    assert_eq!(key(&store).as_deref(), Some(B));
    assert!(
        store
            .reader()
            .unwrap()
            .claude_account_handoffs("s")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_quiet_archived_chat_on_the_source_account_moves_once() {
    let (_dir, store) = store();
    store.set_archived("s", true).unwrap();
    assert!(store.move_claude_account("s", &change(), 10).unwrap());
    let session = &store.reader().unwrap().sessions().unwrap()[0];
    assert_eq!(session.account_key.as_deref(), Some(A));
    assert_eq!(session.account_id.as_deref(), Some("account-a"));
    assert_eq!(
        store
            .reader()
            .unwrap()
            .claude_account_handoffs("s")
            .unwrap(),
        vec![(B.to_owned(), 4_000_000_000)]
    );
    // No cutoff note: the owner asked for this move, nothing was cut off.
    assert!(
        store
            .reader()
            .unwrap()
            .outstanding_messages("s")
            .unwrap()
            .is_empty()
    );
    // A stale second request names a source the chat no longer uses.
    assert!(!store.move_claude_account("s", &change(), 11).unwrap());
    assert_eq!(key(&store).as_deref(), Some(A));
}
