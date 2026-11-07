#[test]
fn existing_subagent_lifecycle_records_are_indexed_during_upgrade() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let before = temp.path().join("before");
    std::fs::create_dir(&before).unwrap();
    for entry in std::fs::read_dir(root.join("migrations")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "024" {
            std::fs::copy(entry.path(), before.join(entry.file_name())).unwrap();
        }
    }
    let db = temp.path().join("old.sqlite");
    let connection = rusqlite::Connection::open(&db).unwrap();
    marmot::migrations::migrate_connection(&connection, &before, "schema_versions").unwrap();
    connection.execute_batch("INSERT INTO projects(id,path,name,created_at,position) VALUES('p','/project','Project',1,0); INSERT INTO sessions(id,working_directory,created_at,project_id,agent,position) VALUES('s','/project',1,'p','claude',0);").unwrap();
    let call = serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"agent-old","name":"Task","input":{"description":"Old task"}}]}});
    let result = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent-old","content":"done"}]}});
    for (seq, payload) in [(1, call), (2, result)] {
        connection.execute("INSERT INTO events(session,seq,kind,payload,created_at) VALUES('s',?1,'agent_event',?2,1)",rusqlite::params![seq,payload.to_string()]).unwrap();
    }
    drop(connection);
    let store = boblm_store::Store::open(&db).unwrap();
    let page = store
        .reader()
        .unwrap()
        .history_page("s", None, false, 1)
        .unwrap();
    assert_eq!(page.context.len(), 2);
    assert!(page.context[0].payload.contains("Old task"));
    store.record_event("s","agent_event",serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"new","name":"Agent","input":{"description":"New task"}}]}}),2).unwrap();
    assert_eq!(
        store
            .reader()
            .unwrap()
            .history_page("s", None, false, 1)
            .unwrap()
            .context
            .len(),
        3
    );
}
