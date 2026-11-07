#[path = "session_list.generated.rs"]
mod sql;
pub use sql::SinceRow as SessionListRecord;

pub(crate) fn append(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    kind: &str,
    payload: &serde_json::Value,
    created_at: i64,
) -> crate::Result<()> {
    sql::append(
        tx,
        sql::AppendParams {
            session,
            kind,
            payload: &payload.to_string(),
            created_at,
        },
    )?;
    Ok(())
}
/// Tell the list a root's subagent summary changed. Only a subagent has a root to tell.
pub(crate) fn subagents_changed(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    created_at: i64,
) -> crate::Result<()> {
    if let Some(Some(root)) = sql::root_of(tx, sql::RootOfParams { session })?.pop() {
        append(
            tx,
            &root,
            "subagents_changed",
            &serde_json::json!({}),
            created_at,
        )?;
    }
    Ok(())
}
impl crate::Store {
    /// Publish one state summary to both durable streams in the same transaction.
    pub fn publish_summary(&self, session: &str, status: serde_json::Value) -> crate::Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        let now = crate::events::now_ms();
        crate::events::append(&tx, session, "state_summary", status.clone(), now)?;
        append(&tx, session, "state_summary", &status, now)?;
        tx.commit()?;
        Ok(())
    }
}
impl crate::Reader {
    pub fn session_list_since(
        &self,
        after: i64,
        limit: i64,
    ) -> crate::Result<Vec<SessionListRecord>> {
        if after < 0 || !(1..=256).contains(&limit) {
            return Err(rusqlite::Error::InvalidParameterName(
                "session list cursor or limit".into(),
            )
            .into());
        }
        Ok(sql::since(&self.conn, sql::SinceParams { after, limit })?)
    }
}

/// One SQLite read transaction establishes the baseline before live list events.
pub struct SessionListSnapshot {
    pub cursor: i64,
    pub sessions: Vec<crate::Session>,
    pub projects: Vec<crate::Project>,
}
impl crate::Reader {
    pub fn session_list_snapshot(&self) -> crate::Result<SessionListSnapshot> {
        let tx = self.conn.unchecked_transaction()?;
        let cursor = sql::latest_cursor(&tx)?.pop().unwrap_or(0);
        let sessions = self.sessions()?;
        let projects = self.projects()?;
        tx.commit()?;
        Ok(SessionListSnapshot {
            cursor,
            sessions,
            projects,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;
    #[test]
    fn list_snapshot_resumes_after_history_without_losing_new_events() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("snapshot.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        for i in 0..20 {
            store
                .publish_summary("s", serde_json::json!({"turnActive": i % 2 == 0}))
                .unwrap();
        }
        let reader = store.reader().unwrap();
        let snapshot = reader.session_list_snapshot().unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot.cursor >= 20);
        assert!(
            reader
                .session_list_since(snapshot.cursor, 256)
                .unwrap()
                .is_empty()
        );
        store
            .publish_summary("s", serde_json::json!({"turnActive": true}))
            .unwrap();
        let changes = reader.session_list_since(snapshot.cursor, 256).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].seq > snapshot.cursor);
        assert_eq!(changes[0].kind, "state_summary");
    }
}
