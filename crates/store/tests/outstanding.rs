use boblm_store::Store;
use std::time::Instant;
#[test]
fn outstanding_inputs_exclude_consumed_history_and_follow_delivery_transitions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("inputs.sqlite");
    let store = Store::open(&path).unwrap();
    store.create_session("s", "/fake", 1).unwrap();
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    let tx = connection.transaction().unwrap();
    for i in 1..=2000 {
        tx.execute("INSERT INTO messages(uuid,session,seq,role,content,delivery_state,created_at) VALUES(?1,'s',?2,'user',?3,'started',1)",rusqlite::params![format!("history-{i}"),i,"historical input ".repeat(32)]).unwrap();
    }
    for (i, state) in ["queued", "dispatching", "uncertain", "rejected"]
        .into_iter()
        .enumerate()
    {
        tx.execute("INSERT INTO messages(uuid,session,seq,role,content,delivery_state,created_at) VALUES(?1,'s',?2,'user','pending',?3,1)",rusqlite::params![format!("pending-{i}"),2001+i as i64,state]).unwrap();
    }
    tx.commit().unwrap();
    let reader = store.reader().unwrap();
    let start = Instant::now();
    let all = reader.messages("s", 1, i64::MAX).unwrap();
    let all_us = start.elapsed().as_micros();
    let start = Instant::now();
    let outstanding = reader.outstanding_messages("s").unwrap();
    let outstanding_us = start.elapsed().as_micros();
    assert_eq!(all.len(), 2004);
    assert_eq!(outstanding.len(), 4);
    assert_eq!(
        outstanding
            .iter()
            .map(|m| m.delivery_state.as_str())
            .collect::<Vec<_>>(),
        ["queued", "dispatching", "uncertain", "rejected"]
    );
    store
        .transition("pending-1", "dispatching", "started")
        .unwrap();
    store
        .transition("pending-0", "queued", "dispatching")
        .unwrap();
    assert_eq!(reader.outstanding_messages("s").unwrap().len(), 3);
    assert_eq!(
        reader.message("history-1").unwrap().unwrap().delivery_state,
        "started"
    );
    eprintln!(
        "history_rows={} outstanding_rows={} all_read_us={} outstanding_read_us={}",
        all.len(),
        outstanding.len(),
        all_us,
        outstanding_us
    );
}
