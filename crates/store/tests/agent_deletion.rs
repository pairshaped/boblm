use boblm_store::Store;

#[test]
fn agent_deletion_rechecks_questions_and_archive_inside_the_delete_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("guards.sqlite")).unwrap();
    store.create_session("caller", "/project", 1).unwrap();
    store.create_session("target", "/project", 1).unwrap();
    store.set_archived("target", true).unwrap();
    store
        .prepare_agent_deletion("request", "caller", "target")
        .unwrap();
    store.ask("q", "target", "Keep this?", &[], 2).unwrap();
    let refused = store
        .delete_session_by_agent("target", "request", "caller", "target")
        .unwrap_err();
    assert!(refused.to_string().contains("unanswered owner question"));
    assert!(!store.reader().unwrap().session_deleted("target").unwrap());
    assert!(
        store
            .reader()
            .unwrap()
            .agent_deletion_receipt("request")
            .unwrap()
            .is_none()
    );
    store.delete_question("q", "target", 3).unwrap();
    store.set_archived("target", false).unwrap();
    assert!(
        store
            .delete_session_by_agent("target", "request", "caller", "target")
            .unwrap_err()
            .to_string()
            .contains("archive_peer")
    );
    store.set_archived("target", true).unwrap();
    store
        .delete_session_by_agent("target", "request", "caller", "target")
        .unwrap();
    assert!(store.reader().unwrap().session_deleted("target").unwrap());
}
