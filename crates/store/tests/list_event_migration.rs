#[test]
fn widening_the_list_kinds_keeps_rows_and_never_reissues_a_sequence_number() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let before = temp.path().join("before");
    std::fs::create_dir(&before).unwrap();
    for entry in std::fs::read_dir(root.join("migrations")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "040" {
            std::fs::copy(entry.path(), before.join(entry.file_name())).unwrap();
        }
    }
    let db = temp.path().join("old.sqlite");
    let connection = rusqlite::Connection::open(&db).unwrap();
    marmot::migrations::migrate_connection(&connection, &before, "schema_versions").unwrap();
    // Five events were issued, then the last three went with a deleted chat.
    for seq in 1..=5 {
        connection
            .execute(
                "INSERT INTO session_list_events(session,kind,payload,created_at) VALUES('s','renamed','{}',?1)",
                [seq],
            )
            .unwrap();
    }
    connection
        .execute("DELETE FROM session_list_events WHERE seq > 2", [])
        .unwrap();
    drop(connection);
    let store = boblm_store::Store::open(&db).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    // An old runner still inserts the kinds it knows, and the counter carries on past 5.
    connection
        .execute(
            "INSERT INTO session_list_events(session,kind,payload,created_at) VALUES('s','state_summary','{}',9)",
            [],
        )
        .unwrap();
    let seqs: Vec<i64> = store
        .reader()
        .unwrap()
        .session_list_since(0, 100)
        .unwrap()
        .into_iter()
        .map(|event| event.seq)
        .collect();
    assert_eq!(seqs, [1, 2, 6]);
}
