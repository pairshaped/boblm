//! Agent constraint migration over an existing database, including dependent history.
#[test]
fn pi_migration_preserves_existing_chats_and_their_history() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir_in(root.join("../../target")).unwrap();
    let before = temp.path().join("before");
    std::fs::create_dir(&before).unwrap();
    for entry in std::fs::read_dir(root.join("migrations")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "017" {
            std::fs::copy(entry.path(), before.join(entry.file_name())).unwrap();
        }
    }
    let db = temp.path().join("migration.sqlite");
    marmot::migrations::migrate_connection(
        &rusqlite::Connection::open(&db).unwrap(),
        &before,
        "schema_versions",
    )
    .unwrap();
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("INSERT INTO projects (id,path,name,created_at,position) VALUES ('p','/work','Work',1,9);
          INSERT INTO sessions (id,working_directory,created_at,project_id,agent,position) VALUES ('old','/work',1,'p','codex',17);
          INSERT INTO messages (uuid,session,seq,role,content,delivery_state,created_at) VALUES ('m','old',1,'user','keep me','uncertain',2);").unwrap();
    }
    let store = boblm_store::Store::open(&db).unwrap();
    let sessions = store.reader().unwrap().sessions().unwrap();
    assert_eq!(sessions[0].agent, "codex");
    let c = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row("SELECT content FROM messages WHERE uuid='m'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "keep me"
    );
    assert_eq!(
        c.query_row("SELECT position FROM sessions WHERE id='old'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert_eq!(
        c.query_row("SELECT position FROM projects WHERE id='p'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        9
    );
    c.execute("UPDATE sessions SET agent='pi' WHERE id='old'", [])
        .unwrap();
    assert!(
        c.execute("UPDATE sessions SET agent='other' WHERE id='old'", [])
            .is_err()
    );
    assert!(
        c.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}
