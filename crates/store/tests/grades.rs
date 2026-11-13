use boblm_store::{PeerSpawn, Store};

fn store_with_subagent(role: &str, resolved: Option<&str>) -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("grades.sqlite")).unwrap();
    store.create_session("root", "/project", 1).unwrap();
    store.create_session("stranger", "/project", 1).unwrap();
    let project = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap();
    store
        .spawn_peer(PeerSpawn {
            request_id: "child".into(),
            caller: "root".into(),
            arguments: r#"{"message":"work"}"#.into(),
            project_id: project,
            agent: "codex".into(),
            model: "gpt-6-luna".into(),
            effort: "xhigh".into(),
            title: "Check the diff".into(),
            content: "work".into(),
            created_at: 2,
            parent_session_id: Some("root".into()),
            root_session_id: Some("root".into()),
            role: Some(role.into()),
            resolved: resolved.map(str::to_owned),
            working_directory: None,
            account_key: Some("codex-api".into()),
        })
        .unwrap();
    (temp, store)
}

const RESOLVED: &str = r#"{"role":"reviewer","model":"gpt-6-luna","effort":"xhigh","account":"codex-api","step":"reviewer-openai-api","reason":"r","difficulty":"none"}"#;

#[test]
fn a_grade_copies_the_routing_and_grading_again_overwrites_it() {
    let (_temp, store) = store_with_subagent("reviewer", Some(RESOLVED));
    let first = store
        .grade_subagent("root", "child", "good", Some("  found the bug  "), 10)
        .unwrap();
    assert!(!first.replaced);
    let row = store
        .reader()
        .unwrap()
        .subagent_grade("child")
        .unwrap()
        .unwrap();
    assert_eq!(row.rating, "good");
    assert_eq!(row.note.as_deref(), Some("found the bug"));
    assert_eq!(row.grader, "root");
    assert_eq!(row.role.as_deref(), Some("reviewer"));
    assert_eq!(row.agent.as_deref(), Some("codex"));
    assert_eq!(row.model.as_deref(), Some("gpt-6-luna"));
    assert_eq!(row.effort.as_deref(), Some("xhigh"));
    assert_eq!(row.step.as_deref(), Some("reviewer-openai-api"));
    assert_eq!(row.account_key.as_deref(), Some("codex-api"));
    assert_eq!(row.title.as_deref(), Some("Check the diff"));
    assert_eq!((row.created_at, row.updated_at), (10, 10));

    let second = store
        .grade_subagent("root", "child", "poor", None, 20)
        .unwrap();
    assert!(second.replaced);
    let row = store
        .reader()
        .unwrap()
        .subagent_grade("child")
        .unwrap()
        .unwrap();
    assert_eq!(row.rating, "poor");
    assert_eq!(row.note, None);
    assert_eq!((row.created_at, row.updated_at), (10, 20));
}

#[test]
fn a_missing_receipt_leaves_the_step_empty() {
    let (_temp, store) = store_with_subagent("implementer", None);
    store
        .grade_subagent("root", "child", "ok", None, 10)
        .unwrap();
    let row = store
        .reader()
        .unwrap()
        .subagent_grade("child")
        .unwrap()
        .unwrap();
    assert_eq!(row.step, None);
    assert_eq!(row.role.as_deref(), Some("implementer"));
}

#[test]
fn only_the_parent_root_or_spawner_may_grade() {
    let (_temp, store) = store_with_subagent("planner", Some(RESOLVED));
    let refused = store
        .grade_subagent("stranger", "child", "good", None, 10)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("neither"), "{refused}");
    assert!(
        store
            .reader()
            .unwrap()
            .subagent_grade("child")
            .unwrap()
            .is_none()
    );
    store
        .grade_subagent("root", "child", "good", None, 10)
        .unwrap();

    // The spawner is allowed even when the child carries no parent link (a role-routed root).
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("spawner.sqlite")).unwrap();
    store.create_session("spawner", "/project", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    store
        .spawn_peer(PeerSpawn {
            request_id: "visible".into(),
            caller: "spawner".into(),
            arguments: r#"{"message":"work"}"#.into(),
            project_id: project,
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "medium".into(),
            title: "Visible".into(),
            content: "work".into(),
            created_at: 2,
            parent_session_id: None,
            root_session_id: None,
            role: Some("implementer".into()),
            resolved: None,
            working_directory: None,
            account_key: None,
        })
        .unwrap();
    store.create_session("other", "/project", 1).unwrap();
    assert!(
        store
            .grade_subagent("other", "visible", "ok", None, 5)
            .is_err()
    );
    store
        .grade_subagent("spawner", "visible", "ok", None, 5)
        .unwrap();
}

#[test]
fn the_root_of_a_subagent_may_grade_it() {
    let (_temp, store) = store_with_subagent("reviewer", None);
    // Re-link the child so only its root, not its parent, is the caller.
    store.create_session("middle", "/project", 1).unwrap();
    let project = store.reader().unwrap().sessions().unwrap()[0]
        .project_id
        .clone()
        .unwrap();
    store
        .spawn_peer(PeerSpawn {
            request_id: "grandchild".into(),
            caller: "middle".into(),
            arguments: r#"{"message":"work"}"#.into(),
            project_id: project,
            agent: "codex".into(),
            model: "gpt-6-luna".into(),
            effort: "xhigh".into(),
            title: "Grandchild".into(),
            content: "work".into(),
            created_at: 3,
            parent_session_id: Some("middle".into()),
            root_session_id: Some("root".into()),
            role: Some("reviewer".into()),
            resolved: None,
            working_directory: None,
            account_key: None,
        })
        .unwrap();
    // The spawn receipt names "middle" as the caller; "root" qualifies only through root_session_id.
    store
        .grade_subagent("root", "grandchild", "good", None, 4)
        .unwrap();
}

#[test]
fn invalid_ratings_and_long_notes_are_refused_in_plain_english() {
    let (_temp, store) = store_with_subagent("reviewer", None);
    let rating = store
        .grade_subagent("root", "child", "great", None, 1)
        .unwrap_err()
        .to_string();
    assert!(rating.contains("good, ok or poor"), "{rating}");
    let long = "x".repeat(boblm_tuning::MAX_GRADE_NOTE_CHARS + 1);
    let note = store
        .grade_subagent("root", "child", "good", Some(&long), 1)
        .unwrap_err()
        .to_string();
    assert!(note.contains("limit is 500"), "{note}");
    // Multi-byte characters count as characters, not bytes.
    let exact = "é".repeat(boblm_tuning::MAX_GRADE_NOTE_CHARS);
    store
        .grade_subagent("root", "child", "good", Some(&exact), 1)
        .unwrap();
    assert!(
        store
            .grade_subagent("root", "ghost", "good", None, 1)
            .is_err()
    );
}

#[test]
fn an_archived_subagent_can_be_graded_and_the_grade_survives_deleting_the_chat() {
    let (_temp, store) = store_with_subagent("reviewer", Some(RESOLVED));
    store.set_archived("child", true).unwrap();
    store
        .grade_subagent("root", "child", "ok", Some("fine"), 10)
        .unwrap();
    store.set_archived("root", true).unwrap();
    store.delete_session("root").unwrap();
    let reader = store.reader().unwrap();
    assert!(reader.session_deleted("child").unwrap());
    let row = reader.subagent_grade("child").unwrap().unwrap();
    assert_eq!(row.rating, "ok");
    assert_eq!(row.model.as_deref(), Some("gpt-6-luna"));
    let gone = store
        .grade_subagent("root", "child", "good", None, 11)
        .unwrap_err()
        .to_string();
    assert!(gone.contains("no longer exists"), "{gone}");
}
