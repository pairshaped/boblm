use boblm_store::{PeerSpawn, Store};

fn root(store: &Store) -> String {
    store.create_session("root", "/fake", 1).unwrap();
    store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == "root")
        .and_then(|session| session.project_id)
        .unwrap()
}

fn spawn_subagent(store: &Store, project_id: &str, id: &str) {
    store
        .spawn_peer(PeerSpawn {
            request_id: id.into(),
            caller: "root".into(),
            arguments: format!(r#"{{"message":"work {id}"}}"#),
            project_id: project_id.into(),
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            title: "Fixture chat".into(),
            content: "work".into(),
            created_at: 2,
            parent_session_id: Some("root".into()),
            root_session_id: Some("root".into()),
            role: Some("implementer".into()),
            resolved: None,
            working_directory: None,
            account_key: None,
        })
        .unwrap();
}

#[test]
fn spawn_records_the_parent_root_and_role_and_archives_and_deletes_cascade() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("parents.sqlite")).unwrap();
    let project = root(&store);
    spawn_subagent(&store, &project, "10000000-0000-4000-8000-000000000001");
    spawn_subagent(&store, &project, "10000000-0000-4000-8000-000000000002");

    let sessions = store.reader().unwrap().sessions().unwrap();
    let subagent = sessions
        .iter()
        .find(|session| session.id == "10000000-0000-4000-8000-000000000001")
        .unwrap();
    assert_eq!(subagent.parent_session_id.as_deref(), Some("root"));
    assert_eq!(subagent.root_session_id.as_deref(), Some("root"));
    assert_eq!(subagent.role.as_deref(), Some("implementer"));
    let root_session = sessions
        .iter()
        .find(|session| session.id == "root")
        .unwrap();
    assert!(root_session.parent_session_id.is_none());
    assert!(root_session.root_session_id.is_none());
    assert!(root_session.role.is_none());

    let created = store
        .reader()
        .unwrap()
        .events_since("10000000-0000-4000-8000-000000000001", 0, 100)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "session_created")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&created.payload).unwrap();
    assert_eq!(payload["parent_session_id"], "root");
    assert_eq!(payload["root_session_id"], "root");
    assert_eq!(payload["role"], "implementer");

    store.set_archived("root", true).unwrap();
    let sessions = store.reader().unwrap().sessions().unwrap();
    assert!(sessions.iter().all(|session| session.archived));

    // Reopening the root does not reopen its subagents.
    store.set_archived("root", false).unwrap();
    let sessions = store.reader().unwrap().sessions().unwrap();
    assert!(!sessions.iter().find(|s| s.id == "root").unwrap().archived);
    assert!(
        sessions
            .iter()
            .filter(|session| session.id != "root")
            .all(|session| session.archived)
    );

    store.set_archived("root", true).unwrap();
    store.delete_session("root").unwrap();
    let reader = store.reader().unwrap();
    assert!(reader.session_deleted("root").unwrap());
    for id in [
        "10000000-0000-4000-8000-000000000001",
        "10000000-0000-4000-8000-000000000002",
    ] {
        assert!(reader.session_deleted(id).unwrap());
    }
    assert!(reader.sessions().unwrap().is_empty());
}

#[test]
fn the_previous_release_sessions_list_still_runs_after_migration_034() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("compatible.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("root", "/fake", 1).unwrap();
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
    let row = rows
        .next()
        .unwrap()
        .expect("the old list query must return a row");
    let id: String = row.get(0).unwrap();
    assert_eq!(id, "root");
}

#[test]
fn role_spawn_receipt_keeps_the_resolved_choice_and_old_sql_still_runs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("role-spawn.sqlite");
    let store = Store::open(&path).unwrap();
    let project = root(&store);
    let id = "10000000-0000-4000-8000-000000000003";
    store
        .spawn_peer(PeerSpawn {
            request_id: id.into(),
            caller: "root".into(),
            arguments: r#"{"role":"implementer","difficulty":"simple"}"#.into(),
            project_id: project,
            agent: "claude".into(),
            model: "sonnet".into(),
            effort: "medium".into(),
            title: "Fixture chat".into(),
            content: "work".into(),
            created_at: 2,
            parent_session_id: Some("root".into()),
            root_session_id: Some("root".into()),
            role: Some("implementer".into()),
            resolved: Some(
                r#"{"role":"implementer","difficulty":"simple","model":"sonnet","effort":"medium","account":"claude-plan","step":"implementer-claude","reason":"Claude has room."}"#
                    .into(),
            ),
            working_directory: None,
            account_key: Some("codex-plan".into()),
        })
        .unwrap();
    let session = store
        .reader()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.id == id)
        .unwrap();
    assert_eq!(session.account_key.as_deref(), Some("codex-plan"));
    let receipt = store
        .reader()
        .unwrap()
        .peer_spawn_receipt(id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.caller, "root");
    assert!(receipt.resolved.unwrap().contains(r#""model":"sonnet""#));

    // The previous release's receipt SQL still runs after the new column is added.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let fixture = include_str!("fixtures/peer_spawns_a3fb372.sql");
    let lookup = fixture
        .split("-- func: ")
        .find_map(|block| {
            let mut lines = block.lines();
            (lines.next()?.trim() == "lookup").then(|| {
                lines
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .trim_end_matches(';')
                    .to_owned()
            })
        })
        .unwrap();
    let caller: String = conn
        .query_row(&lookup, rusqlite::params![id], |row| row.get(0))
        .unwrap();
    assert_eq!(caller, "root");
    let old_receipt = fixture
        .split("-- func: ")
        .find_map(|block| {
            let mut lines = block.lines();
            (lines.next()?.trim() == "receipt").then(|| {
                lines
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .trim_end_matches(';')
                    .to_owned()
            })
        })
        .unwrap();
    store.create_session("old-request", "/fake", 3).unwrap();
    conn.execute(
        &old_receipt,
        rusqlite::params!["old-request", "root", r#"{"message":"old"}"#],
    )
    .unwrap();
    let resolved: Option<String> = conn
        .query_row(
            "SELECT resolved FROM peer_spawn_requests WHERE request_id = 'old-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(resolved.is_none());
}
