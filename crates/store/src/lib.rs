//! Synchronous SQLite boundary. Run database operations off Tokio worker threads.
mod account_usage;
mod api_headroom;
mod attachments;
mod claude_accounts;
pub use claude_accounts::ClaudeHandoff;
mod deletions;
mod events;
mod history;
mod idle_nudges;
pub use history::HistoryPage;
mod messages;
mod migrations;
mod notifications;
mod peer_spawns;
mod projects;
mod questions;
mod role_exhaustion;

mod schedules;
mod secrets;
mod session_list;
mod sessions;
mod subagent_reports;
mod subagent_rounds;
mod usage_report;

use rusqlite::{Connection, OpenFlags};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    ChatDeletion(String),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] marmot::migrations::MigrationError),
    #[error("the database writer was left locked by an operation that crashed; restart the server")]
    Poisoned,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(
        "the message was not in the delivery state this change expected, so another change got there first"
    )]
    Transition,
    #[error(
        "that message ID already belongs to a message with different text, attachments or chat"
    )]
    MessageConflict,
    #[error("that upload ID already belongs to a different file (other bytes, name or type)")]
    AttachmentConflict,
    #[error(
        "an attachment on this message cannot be used: it does not exist, belongs to another chat, has not finished uploading, or is already attached to a sent message; upload it again"
    )]
    AttachmentUnavailable,
    #[error(
        "that schedule ID already belongs to a different schedule (another chat or other settings)"
    )]
    ScheduleConflict,
    #[error("that project ID already belongs to a different project (another folder or name)")]
    ProjectConflict,
    #[error("the folder {0} is already a project; chats are created inside the existing one")]
    ProjectPathTaken(String),
    #[error("there is no project with that ID")]
    ProjectMissing,
    #[error(
        "this project still has {0} chat(s), archived ones included, and history belongs to its project; it cannot be removed"
    )]
    ProjectHasChats(i64),
    #[error("this chat already has {0} active schedules, the most it may have; cancel one first")]
    ScheduleLimit(usize),
    #[error("that request ID was already used by a different caller or with different arguments")]
    SpawnConflict,
    #[error("{0}")]
    Subagent(String),
    #[error("that credential request ID was already used for a different request")]
    SecretConflict,
    #[error(
        "this chat is already waiting on {0} credential requests, the most it may have; wait for the owner to answer or for one to expire"
    )]
    SecretLimit(usize),
    #[error(
        "That question ID was already used for different text, choices or chat. Use a fresh UUID for a new question."
    )]
    QuestionConflict,
    #[error(
        "This question does not belong to this chat or no longer exists. Reload the questions drawer."
    )]
    QuestionMissing,
    #[error(
        "This question was already answered or set aside for discussion. Reload the questions drawer to see its state."
    )]
    QuestionAnswered,
    #[error(
        "That notification ID was already used for different text or a different chat. Use a fresh UUID for a new notification."
    )]
    NotificationConflict,
    #[error("{0}")]
    Corrupt(String),
    #[error("{0}")]
    ReorderInvalid(String),
}
impl Error {
    /// A writer held the database past the busy timeout. The change did not happen
    /// and a later attempt can succeed, so this must not end a chat's runner.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Error::Sql(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }
}
/// Refuse an ordering that names the same item twice.
pub(crate) fn distinct(ids: &[String], what: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    match ids.iter().find(|id| !seen.insert(id.as_str())) {
        Some(twice) => Err(Error::ReorderInvalid(format!(
            "The new order names {what} {} twice. Send each {what} once.",
            shown(twice)
        ))),
        None => Ok(()),
    }
}
/// An ID as it appears in a message: short, quoted.
pub(crate) fn shown(value: &str) -> String {
    let short: String = value.chars().take(40).collect();
    format!("{short:?}")
}
pub type Result<T> = std::result::Result<T, Error>;

/// Clones share the sole writer. Open once per database in the application.
#[derive(Clone)]
pub struct Store {
    writer: Arc<Mutex<Connection>>,
    path: std::path::PathBuf,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(boblm_tuning::SQLITE_BUSY_TIMEOUT)?;
        // A deferred transaction that reads and then writes fails at once with "database
        // is locked" when another process commits in between, ignoring the busy timeout.
        // Taking the write lock up front makes it wait its turn instead.
        conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrations::apply(&conn, &path)?;
        Ok(Self {
            writer: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// The database file; uploads live beside it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn reader(&self) -> Result<Reader> {
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(boblm_tuning::SQLITE_BUSY_TIMEOUT)?;
        Ok(Reader { conn })
    }
}

pub struct Reader {
    conn: Connection,
}

pub use attachments::{Attachment, NewAttachment};
pub use events::{RecentMessageEvent, StoredEvent};
pub use messages::{LookupRow, NewMessage, RangeRow};
pub use notifications::{NewNotification, Notification, PushSubscription, Subscription};
pub use peer_spawns::PeerSpawn;
pub use projects::{Project, default_name as default_project_name};
pub use questions::Question;

pub use idle_nudges::QuietCheck;
pub use schedules::{NewSchedule, Outcome as ScheduleOutcome, Schedule};
pub use secrets::{Claim as SecretClaim, NewSecretRequest, SecretRequest};
pub use session_list::{SessionListRecord, SessionListSnapshot};
pub use sessions::{NewSession, Session, SubagentCounts};
pub use subagent_reports::SubagentReport;
pub use usage_report::{AllowanceUsageDay, TokenUsageDay};

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
            .unwrap();
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        store.create_session("s", "/host/project", 1).unwrap();
        (temp, store)
    }
    #[test]
    fn peer_spawn_rolls_back_child_prompt_receipt_and_events_together() {
        let (_temp, store) = fixture();
        let request = || PeerSpawn {
            request_id: "00000000-0000-4000-8000-000000000006".into(),
            caller: "s".into(),
            arguments: r#"{"message":"keep"}"#.into(),
            project_id: store.reader().unwrap().sessions().unwrap()[0]
                .project_id
                .clone()
                .unwrap(),
            agent: "claude".into(),
            model: "opus".into(),
            effort: "high".into(),
            title: "Fixture chat".into(),
            content: "keep".into(),
            created_at: 2,
            parent_session_id: None,
            root_session_id: None,
            role: None,
            resolved: None,
            working_directory: None,
            account_key: None,
        };
        store.writer.lock().unwrap().execute_batch("CREATE TRIGGER fail_spawn BEFORE INSERT ON events WHEN NEW.kind = 'message_appended' BEGIN SELECT RAISE(ABORT, 'fixture spawn failure'); END;").unwrap();
        assert!(store.spawn_peer(request()).is_err());
        let reader = store.reader().unwrap();
        assert_eq!(reader.sessions().unwrap().len(), 1);
        assert!(reader.message(&request().request_id).unwrap().is_none());
        assert_eq!(reader.session_list_since(0, 100).unwrap().len(), 1);
        store
            .writer
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_spawn;")
            .unwrap();
        assert!(store.spawn_peer(request()).unwrap());
        assert!(!store.spawn_peer(request()).unwrap());
        assert_eq!(store.reader().unwrap().sessions().unwrap().len(), 2);
    }
    #[test]
    fn rejection_commits_state_retry_and_event_together() {
        let (_temp, store) = fixture();
        let dispatching = |uuid: &str| {
            store
                .append(NewMessage {
                    uuid,
                    session: "s",
                    role: "user",
                    content: uuid,
                    delivery_state: "queued",
                    created_at: 2,
                })
                .unwrap();
            store.transition(uuid, "queued", "dispatching").unwrap();
        };
        let (retried, held) = (
            "00000000-0000-4000-8000-0000000000a1",
            "00000000-0000-4000-8000-0000000000a2",
        );
        dispatching(retried);
        store.reject(retried, Some(1)).unwrap();
        let message = store.reader().unwrap().message(retried).unwrap().unwrap();
        assert_eq!(
            (message.delivery_state.as_str(), message.retry_count),
            ("queued", 1)
        );
        assert!(
            store.reject(retried, Some(2)).is_err(),
            "only dispatching input can be rejected"
        );

        dispatching(held);
        store.writer.lock().unwrap().execute_batch("CREATE TRIGGER fail_reject BEFORE INSERT ON events WHEN NEW.kind = 'delivery_rejected' BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(store.reject(held, None).is_err());
        let reader = store.reader().unwrap();
        assert_eq!(
            reader.message(held).unwrap().unwrap().delivery_state,
            "dispatching"
        );
        let before = reader.events_since("s", 0, 1000).unwrap().len();
        store
            .writer
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_reject;")
            .unwrap();
        store.reject(held, None).unwrap();
        let reader = store.reader().unwrap();
        assert_eq!(
            reader.message(held).unwrap().unwrap().delivery_state,
            "rejected"
        );
        let kinds: Vec<_> = reader
            .events_since("s", 0, 1000)
            .unwrap()
            .into_iter()
            .skip(before)
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, ["delivery_changed", "delivery_rejected"]);
    }
    #[test]
    fn event_batches_commit_in_order_or_not_at_all() {
        let (_temp, store) = fixture();
        let before = store
            .reader()
            .unwrap()
            .events_since("s", 0, 1000)
            .unwrap()
            .len();
        let batch =
            |range: std::ops::Range<i32>| range.map(|i| serde_json::json!({"i": i})).collect();
        store
            .record_events("s", "agent_event", batch(0..5), 2)
            .unwrap();
        let events = store.reader().unwrap().events_since("s", 0, 1000).unwrap();
        let recorded: Vec<_> = events.iter().skip(before).collect();
        assert_eq!(recorded.len(), 5);
        assert!(recorded.windows(2).all(|w| w[1].seq == w[0].seq + 1));
        assert_eq!(recorded[4].payload, r#"{"i":4}"#);
        store.writer.lock().unwrap().execute_batch("CREATE TRIGGER fail_third BEFORE INSERT ON events WHEN NEW.payload = '{\"i\":12}' BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(
            store
                .record_events("s", "agent_event", batch(10..14), 3)
                .is_err()
        );
        let after = store
            .reader()
            .unwrap()
            .events_since("s", 0, 1000)
            .unwrap()
            .len();
        assert_eq!(after, before + 5, "a failed batch leaves no partial events");
    }
    fn upload<'a>(id: &'a str, session: &'a str, created_at: i64) -> NewAttachment<'a> {
        NewAttachment {
            id,
            session,
            name: "Screenshot 1.png",
            content_type: "image/png",
            size: 3,
            sha256: "0000000000000000000000000000000000000000000000000000000000000001",
            path: "/uploads/x",
            created_at,
        }
    }
    fn message<'a>(uuid: &'a str, session: &'a str, content: &'a str) -> NewMessage<'a> {
        NewMessage {
            uuid,
            session,
            role: "user",
            content,
            delivery_state: "queued",
            created_at: 5,
        }
    }
    #[test]
    fn uploads_are_idempotent_and_conflict_on_different_content() {
        let (_temp, store) = fixture();
        let (first, created) = store.create_attachment(upload("u1", "s", 1)).unwrap();
        assert!(created);
        assert_eq!(first.message_uuid, None);
        let (again, created) = store.create_attachment(upload("u1", "s", 99)).unwrap();
        assert!(!created);
        assert_eq!(again.created_at, 1, "a retry keeps the original record");
        let mut changed = upload("u1", "s", 1);
        changed.size = 4;
        assert!(matches!(
            store.create_attachment(changed),
            Err(Error::AttachmentConflict)
        ));
        assert!(store.create_attachment(upload("u2", "absent", 1)).is_err());
    }

    #[test]
    fn a_message_claims_its_uploads_in_order_atomically() {
        let (_temp, store) = fixture();
        store.create_session("other", "/x", 1).unwrap();
        for id in ["u1", "u2"] {
            store.create_attachment(upload(id, "s", 1)).unwrap();
            store.mark_attachment_stored(id).unwrap();
        }
        store
            .create_attachment(upload("foreign", "other", 1))
            .unwrap();
        store.mark_attachment_stored("foreign").unwrap();
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let before = store
            .reader()
            .unwrap()
            .events_since("s", 0, 1000)
            .unwrap()
            .len();

        for bad in [
            ids(&["u1", "missing"]),
            ids(&["u1", "foreign"]),
            ids(&["u1", "u1"]),
        ] {
            assert!(matches!(
                store.append_with_attachments(message("m1", "s", "hi"), &bad),
                Err(Error::AttachmentUnavailable)
            ));
        }
        let reader = store.reader().unwrap();
        assert!(
            reader.message("m1").unwrap().is_none(),
            "a failed claim appends nothing"
        );
        assert_eq!(reader.attachment("u1").unwrap().unwrap().message_uuid, None);
        assert_eq!(reader.events_since("s", 0, 1000).unwrap().len(), before);

        assert!(
            store
                .append_with_attachments(message("m1", "s", "hi"), &ids(&["u2", "u1"]))
                .unwrap()
        );
        let reader = store.reader().unwrap();
        let linked = reader.message_attachments("m1").unwrap();
        assert_eq!(
            linked.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["u2", "u1"]
        );
        let event = reader.events_since("s", 0, 1000).unwrap().pop().unwrap();
        let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
        assert_eq!(payload["content"], "hi");
        assert_eq!(payload["attachments"][0]["id"], "u2");
        assert_eq!(payload["attachments"][0]["name"], "Screenshot 1.png");
        assert_eq!(payload["attachments"][0]["contentType"], "image/png");

        // An identical retry returns the message; any other list conflicts.
        assert!(
            !store
                .append_with_attachments(message("m1", "s", "hi"), &ids(&["u2", "u1"]))
                .unwrap()
        );
        assert!(matches!(
            store.append_with_attachments(message("m1", "s", "hi"), &ids(&["u1", "u2"])),
            Err(Error::MessageConflict)
        ));
        assert!(matches!(
            store.append_with_attachments(message("m1", "s", "hi"), &[]),
            Err(Error::MessageConflict)
        ));
        // A sent upload cannot be reused by another message.
        assert!(matches!(
            store.append_with_attachments(message("m2", "s", "again"), &ids(&["u1"])),
            Err(Error::AttachmentUnavailable)
        ));
    }

    #[test]
    fn only_stored_uploads_can_be_claimed() {
        let (_temp, store) = fixture();
        store.create_attachment(upload("u1", "s", 1)).unwrap();
        let ids = vec!["u1".to_string()];
        assert!(matches!(
            store.append_with_attachments(message("m1", "s", "x"), &ids),
            Err(Error::AttachmentUnavailable)
        ));
        assert!(
            store.reader().unwrap().message("m1").unwrap().is_none(),
            "a refused claim appends nothing"
        );
        store.mark_attachment_stored("u1").unwrap();
        assert!(
            store
                .append_with_attachments(message("m1", "s", "x"), &ids)
                .unwrap()
        );
    }

    #[test]
    fn only_old_unsent_uploads_are_listed_and_forgotten_after_their_files() {
        let (_temp, store) = fixture();
        for (id, created) in [("old", 10), ("new", 100), ("sent", 10)] {
            store.create_attachment(upload(id, "s", created)).unwrap();
            store.mark_attachment_stored(id).unwrap();
        }
        store
            .append_with_attachments(message("m1", "s", "x"), &["sent".to_string()])
            .unwrap();
        let orphans = store.orphan_attachments(50).unwrap();
        assert_eq!(orphans, [("old".to_string(), "/uploads/x".to_string())]);
        // Listing changes nothing, so a sweep interrupted after deleting files can retry.
        assert!(store.reader().unwrap().attachment("old").unwrap().is_some());
        store.forget_unsent_attachment("old").unwrap();
        let reader = store.reader().unwrap();
        assert!(reader.attachment("old").unwrap().is_none());
        assert!(reader.attachment("new").unwrap().is_some());
        assert!(reader.attachment("sent").unwrap().is_some());
        store.forget_unsent_attachment("sent").unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .attachment("sent")
                .unwrap()
                .is_some(),
            "a sent upload is never forgotten"
        );
    }
    #[test]
    fn a_changed_claude_session_records_its_predecessor_and_a_repeat_records_nothing() {
        let (_temp, store) = fixture();
        let changes = |store: &Store| -> Vec<serde_json::Value> {
            store
                .reader()
                .unwrap()
                .events_since("s", 0, 1000)
                .unwrap()
                .into_iter()
                .filter(|e| e.kind == "agent_session_changed")
                .map(|e| serde_json::from_str(&e.payload).unwrap())
                .collect()
        };
        store.set_agent_session("s", "first").unwrap();
        store.set_agent_session("s", "first").unwrap();
        assert_eq!(
            changes(&store),
            [serde_json::json!({"agent_session_id":"first","previous":null})],
            "the first ID has no predecessor and a repeat is not a change"
        );
        store.set_agent_session("s", "second").unwrap();
        store.set_agent_session("s", "second").unwrap();
        assert_eq!(
            changes(&store)[1],
            serde_json::json!({"agent_session_id":"second","previous":"first"})
        );
        assert_eq!(changes(&store).len(), 2);
        assert_eq!(
            store.reader().unwrap().sessions().unwrap()[0]
                .agent_session_id
                .as_deref(),
            Some("second")
        );
    }
    #[test]
    fn lowercasing_stored_message_ids_keeps_events_attached_and_never_merges_twins() {
        let (_temp, store) = fixture();
        let upper = "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
        let twin = "FFFFFFFF-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
        for (uuid, content) in [
            (upper, "only spelling"),
            (twin, "uppercase twin"),
            (&twin.to_lowercase(), "lowercase twin"),
        ] {
            append_content(&store, uuid, content);
        }
        store.transition(upper, "queued", "started").unwrap();
        store
            .writer
            .lock()
            .unwrap()
            .execute_batch(include_str!("../migrations/012_lowercase_message_ids.sql"))
            .unwrap();
        let reader = store.reader().unwrap();
        let renamed = reader.message(&upper.to_lowercase()).unwrap().unwrap();
        assert_eq!(renamed.content, "only spelling");
        assert_eq!(renamed.delivery_state, "started");
        assert!(reader.message(upper).unwrap().is_none());
        // Both twins survive untouched: nothing is merged and nothing is lost.
        assert_eq!(
            reader.message(twin).unwrap().unwrap().content,
            "uppercase twin"
        );
        assert_eq!(
            reader
                .message(&twin.to_lowercase())
                .unwrap()
                .unwrap()
                .content,
            "lowercase twin"
        );
        // Events that named the renamed message follow it; the twin's events stay put.
        let uuids: Vec<String> = reader
            .events_since("s", 0, 1000)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "message_appended" || e.kind == "delivery_changed")
            .map(|e| {
                serde_json::from_str::<serde_json::Value>(&e.payload).unwrap()["uuid"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert!(uuids.iter().all(|u| u != upper), "{uuids:?}");
        assert!(uuids.contains(&upper.to_lowercase()));
        assert!(uuids.contains(&twin.to_owned()));
    }
    #[test]
    fn lowercasing_message_ids_keeps_attachment_links_even_with_foreign_keys_enforced() {
        let (_temp, store) = fixture();
        let upper = "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE";
        store
            .create_attachment(NewAttachment {
                id: "up1",
                session: "s",
                name: "a.txt",
                content_type: "text/plain",
                size: 1,
                sha256: "0000000000000000000000000000000000000000000000000000000000000001",
                path: "/uploads/a",
                created_at: 1,
            })
            .unwrap();
        store.mark_attachment_stored("up1").unwrap();
        store
            .append_with_attachments(
                NewMessage {
                    uuid: upper,
                    session: "s",
                    role: "user",
                    content: "see",
                    delivery_state: "queued",
                    created_at: 2,
                },
                &["up1".to_string()],
            )
            .unwrap();
        for enforced in ["OFF", "ON"] {
            let conn = store.writer.lock().unwrap();
            conn.execute_batch(&format!("PRAGMA foreign_keys = {enforced};"))
                .unwrap();
            // Marmot applies each migration inside a transaction with foreign keys on; so do we.
            // Start each round from the old spelling.
            conn.execute_batch(
                "BEGIN; PRAGMA defer_foreign_keys = ON;
                 UPDATE messages SET uuid = upper(uuid);
                 UPDATE attachments SET message_uuid = upper(message_uuid); COMMIT;",
            )
            .unwrap();
            conn.execute_batch(&format!(
                "BEGIN; {} COMMIT;",
                include_str!("../migrations/012_lowercase_message_ids.sql")
            ))
            .unwrap_or_else(|e| panic!("foreign keys {enforced}: {e}"));
            drop(conn);
            let linked = store
                .reader()
                .unwrap()
                .message_attachments(&upper.to_lowercase())
                .unwrap();
            assert_eq!(
                linked.len(),
                1,
                "foreign keys {enforced}: the attachment follows its message"
            );
        }
    }
    fn schedule<'a>(id: &'a str, session: &'a str, next: i64) -> NewSchedule<'a> {
        NewSchedule {
            id,
            session,
            prompt: "keep going",
            arguments: r#"{"everyMinutes":30}"#,
            interval_ms: Some(1_800_000),
            next_run_at: next,
            ends_at: None,
            max_runs: None,
            created_at: 1,
        }
    }
    #[test]
    fn schedules_are_created_once_limited_and_isolated_by_chat() {
        let (_temp, store) = fixture();
        store.create_session("other", "/x", 1).unwrap();
        let (first, created) = store.create_schedule(schedule("a", "s", 100), 2).unwrap();
        assert!(created);
        assert_eq!(
            (first.state.as_str(), first.runs, first.skipped),
            ("active", 0, 0)
        );
        // An identical retry returns the stored schedule; any difference conflicts.
        let (again, created) = store.create_schedule(schedule("a", "s", 999), 2).unwrap();
        assert!(!created);
        assert_eq!(again.next_run_at, 100, "a retry keeps the original timing");
        let mut changed = schedule("a", "s", 100);
        changed.arguments = r#"{"everyMinutes":31}"#;
        assert!(matches!(
            store.create_schedule(changed, 2),
            Err(Error::ScheduleConflict)
        ));
        assert!(matches!(
            store.create_schedule(schedule("a", "other", 100), 2),
            Err(Error::ScheduleConflict)
        ));
        store.create_schedule(schedule("b", "s", 200), 2).unwrap();
        assert!(matches!(
            store.create_schedule(schedule("c", "s", 300), 2),
            Err(Error::ScheduleLimit(2))
        ));
        store
            .create_schedule(schedule("c", "other", 300), 2)
            .unwrap();
        // Another chat cannot cancel it; its own chat can.
        assert!(!store.delete_schedule("a", "other").unwrap());
        assert!(store.delete_schedule("a", "s").unwrap());
        assert!(!store.delete_schedule("a", "s").unwrap());
        store.create_schedule(schedule("d", "s", 400), 2).unwrap();
        let ids: Vec<_> = store
            .reader()
            .unwrap()
            .schedules("s")
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["b", "d"]);
    }

    #[test]
    fn due_schedules_skip_future_finished_and_archived_chats() {
        let (_temp, store) = fixture();
        store.create_session("sleeping", "/x", 1).unwrap();
        store
            .create_schedule(schedule("due", "s", 100), 20)
            .unwrap();
        store
            .create_schedule(schedule("later", "s", 500), 20)
            .unwrap();
        store
            .create_schedule(schedule("zzz", "sleeping", 100), 20)
            .unwrap();
        store.set_archived("sleeping", true).unwrap();
        let due = |store: &Store, now| -> Vec<String> {
            store
                .reader()
                .unwrap()
                .due_schedules(now)
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect()
        };
        assert_eq!(due(&store, 100), ["due"]);
        assert_eq!(due(&store, 99), Vec::<String>::new());
        assert_eq!(due(&store, 600), ["due", "later"]);
        store.set_archived("sleeping", false).unwrap();
        assert_eq!(due(&store, 600), ["due", "zzz", "later"]);
    }

    #[test]
    fn advancing_records_each_outcome_once_and_ignores_a_stale_caller() {
        let (_temp, store) = fixture();
        store.create_schedule(schedule("a", "s", 100), 20).unwrap();
        let get = |store: &Store| store.reader().unwrap().schedule("a").unwrap().unwrap();
        assert!(
            store
                .advance_schedule("a", 100, ScheduleOutcome::Fired { next: Some(200) })
                .unwrap()
        );
        let s = get(&store);
        assert_eq!(
            (
                s.runs,
                s.skipped,
                s.next_run_at,
                s.last_run_at,
                s.state.as_str()
            ),
            (1, 0, 200, Some(100), "active")
        );
        // The same occurrence cannot be recorded twice.
        assert!(
            !store
                .advance_schedule("a", 100, ScheduleOutcome::Fired { next: Some(300) })
                .unwrap()
        );
        assert_eq!(get(&store).runs, 1);
        assert!(
            store
                .advance_schedule("a", 200, ScheduleOutcome::Skipped { next: Some(300) })
                .unwrap()
        );
        let s = get(&store);
        assert_eq!(
            (s.runs, s.skipped, s.next_run_at, s.last_run_at),
            (1, 1, 300, Some(100))
        );
        assert!(
            store
                .advance_schedule("a", 300, ScheduleOutcome::Fired { next: None })
                .unwrap()
        );
        assert_eq!(get(&store).state, "done");
        assert!(
            store
                .reader()
                .unwrap()
                .due_schedules(10_000)
                .unwrap()
                .is_empty()
        );
        store
            .create_schedule(schedule("once", "s", 100), 20)
            .unwrap();
        assert!(
            store
                .advance_schedule("once", 100, ScheduleOutcome::Missed)
                .unwrap()
        );
        assert_eq!(
            store
                .reader()
                .unwrap()
                .schedule("once")
                .unwrap()
                .unwrap()
                .state,
            "missed"
        );
        // A cancelled schedule cannot be advanced by a caller that still holds it.
        store
            .create_schedule(schedule("gone", "s", 100), 20)
            .unwrap();
        store.delete_schedule("gone", "s").unwrap();
        assert!(
            !store
                .advance_schedule("gone", 100, ScheduleOutcome::Fired { next: Some(200) })
                .unwrap()
        );
    }
    #[test]
    fn projects_are_added_once_unique_by_folder_and_removed_only_when_empty() {
        let (_temp, store) = fixture();
        let (project, created) = store.add_project("p1", "/work/app", "app", 1).unwrap();
        assert!(created);
        assert_eq!(
            (project.path.as_str(), project.name.as_str()),
            ("/work/app", "app")
        );
        // An identical retry returns it; the same ID for another folder or name conflicts.
        let (again, created) = store.add_project("p1", "/work/app", "app", 99).unwrap();
        assert!(!created);
        assert_eq!(again.created_at, 1);
        assert!(matches!(
            store.add_project("p1", "/work/other", "app", 1),
            Err(Error::ProjectConflict)
        ));
        assert!(matches!(
            store.add_project("p1", "/work/app", "renamed", 1),
            Err(Error::ProjectConflict)
        ));
        // The same folder under a new ID is refused, naming the folder.
        let taken = store.add_project("p2", "/work/app", "app", 1).unwrap_err();
        assert!(
            taken.to_string().contains("/work/app is already a project"),
            "{taken}"
        );
        assert_eq!(default_project_name("/work/app/"), "app");
        assert_eq!(default_project_name("/"), "/");
        // A project with a chat cannot be removed; an empty one can, once.
        store
            .create_session_in(NewSession {
                name: "",
                id: "chat",
                project_id: "p1",
                agent: "codex",
                model: "gpt-6-luna",
                effort: Some("low"),
                created_at: 2,
            })
            .unwrap();
        let blocked = store.remove_project("p1").unwrap_err();
        assert!(matches!(blocked, Error::ProjectHasChats(1)), "{blocked}");
        store.add_project("p3", "/work/empty", "empty", 3).unwrap();
        assert!(store.remove_project("p3").unwrap());
        assert!(!store.remove_project("p3").unwrap());
        let names: Vec<_> = store
            .reader()
            .unwrap()
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(names.contains(&"p1".to_string()) && !names.contains(&"p3".to_string()));
    }

    #[test]
    fn a_chat_is_created_in_its_project_with_its_agent_and_nothing_pending() {
        let (_temp, store) = fixture();
        store.add_project("p1", "/work/app", "app", 1).unwrap();
        store
            .create_session_in(NewSession {
                name: "",
                id: "c1",
                project_id: "p1",
                agent: "codex",
                model: "gpt-6-luna",
                effort: Some("high"),
                created_at: 2,
            })
            .unwrap();
        let chat = store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == "c1")
            .unwrap();
        assert_eq!(chat.working_directory, "/work/app");
        assert_eq!(
            (chat.project_id.as_deref(), chat.agent.as_str()),
            (Some("p1"), "codex")
        );
        assert_eq!(
            (chat.model.as_str(), chat.effort.as_deref()),
            ("gpt-6-luna", Some("high"))
        );
        assert_eq!(
            (chat.applied_model.as_str(), chat.applied_effort.as_deref()),
            ("gpt-6-luna", Some("high")),
            "nothing pending on a fresh chat"
        );
        let kinds: Vec<_> = store
            .reader()
            .unwrap()
            .events_since("c1", 0, 100)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, ["session_created", "model_effort_changed"]);
        assert!(matches!(
            store.create_session_in(NewSession {
                name: "",
                id: "c2",
                project_id: "ghost",
                agent: "claude",
                model: "sonnet",
                effort: None,
                created_at: 3
            }),
            Err(Error::ProjectMissing)
        ));
        assert!(
            store
                .reader()
                .unwrap()
                .sessions()
                .unwrap()
                .iter()
                .all(|s| s.id != "c2"),
            "nothing is half created"
        );
    }

    #[test]
    fn creating_by_directory_reuses_the_project_for_that_folder() {
        let (_temp, store) = fixture();
        store.create_session("a", "/work/shared", 1).unwrap();
        store.create_session("b", "/work/shared", 2).unwrap();
        let reader = store.reader().unwrap();
        let sessions = reader.sessions().unwrap();
        let ids: Vec<_> = sessions
            .iter()
            .filter(|s| s.working_directory == "/work/shared")
            .map(|s| s.project_id.clone())
            .collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1]);
        assert_eq!(
            reader
                .projects()
                .unwrap()
                .iter()
                .filter(|p| p.path == "/work/shared")
                .count(),
            1
        );
    }

    fn secret(id: &str, at: i64) -> NewSecretRequest<'static> {
        NewSecretRequest {
            id: Box::leak(id.to_owned().into_boxed_str()),
            session: "s",
            file: "/home/me/secrets.yaml",
            key: Box::leak(vec!["db".to_owned(), "password".to_owned()].into_boxed_slice()),
            expires_at: at + 600_000,
            created_at: at,
        }
    }
    #[test]
    fn credential_requests_are_created_once_limited_and_never_hold_the_value() {
        let (_temp, store) = fixture();
        let (first, created) = store.create_secret_request(secret("r1", 1_000), 2).unwrap();
        assert!(created);
        assert_eq!(first.state, "pending");
        assert_eq!(first.key, ["db", "password"]);
        // An identical retry returns the same request; a different one under the ID fails.
        let (again, created) = store.create_secret_request(secret("r1", 1_500), 2).unwrap();
        assert!(!created);
        assert_eq!(again, first);
        let mut other = secret("r1", 1_600);
        other.file = "/home/me/other.yaml";
        assert!(matches!(
            store.create_secret_request(other, 2),
            Err(Error::SecretConflict)
        ));
        store.create_secret_request(secret("r2", 2_000), 2).unwrap();
        assert!(matches!(
            store.create_secret_request(secret("r3", 3_000), 2),
            Err(Error::SecretLimit(2))
        ));
        // Past their time, requests stop counting against the limit.
        store
            .create_secret_request(secret("r3", 700_000), 2)
            .unwrap();
        let events = store.reader().unwrap().events_since("s", 0, 100).unwrap();
        let asked: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "secret_requested")
            .collect();
        assert_eq!(asked.len(), 3);
        assert!(asked[0].payload.contains("\"request_id\":\"r1\""));
    }
    #[test]
    fn a_credential_request_can_be_claimed_by_one_caller_only() {
        let (_temp, store) = fixture();
        store.create_secret_request(secret("r1", 1_000), 5).unwrap();
        let claimed = store.claim_secret_request("r1", 2_000).unwrap();
        assert!(matches!(&claimed, SecretClaim::Claimed(r) if r.state == "saving"));
        assert_eq!(
            store.claim_secret_request("r1", 2_001).unwrap(),
            SecretClaim::Closed("saving".into()),
            "a second submission while one is saving is refused"
        );
        assert_eq!(
            store.claim_secret_request("nope", 2_000).unwrap(),
            SecretClaim::Unknown
        );
        let done = store.finish_secret_request("r1", "saved", 3_000).unwrap();
        assert_eq!(done.unwrap().state, "saved");
        assert!(
            store
                .finish_secret_request("r1", "failed", 3_001)
                .unwrap()
                .is_none(),
            "an ended request cannot end again"
        );
        assert_eq!(
            store.claim_secret_request("r1", 3_002).unwrap(),
            SecretClaim::Closed("saved".into())
        );
        let events = store.reader().unwrap().events_since("s", 0, 100).unwrap();
        let resolved: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "secret_resolved")
            .collect();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].payload.contains("\"status\":\"saved\""));
    }
    #[test]
    fn credential_requests_can_be_cancelled_expire_and_stuck_saves_fail() {
        let (_temp, store) = fixture();
        store.create_secret_request(secret("c", 1_000), 9).unwrap();
        assert!(
            store
                .finish_secret_request("c", "canceled", 2_000)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store.claim_secret_request("c", 2_001).unwrap(),
            SecretClaim::Closed("canceled".into())
        );
        // Claiming after the time has passed expires it instead.
        store
            .create_secret_request(secret("late", 1_000), 9)
            .unwrap();
        assert_eq!(
            store.claim_secret_request("late", 700_000).unwrap(),
            SecretClaim::Closed("expired".into())
        );
        // The sweep expires unanswered requests and fails saves that never finished.
        store
            .create_secret_request(secret("quiet", 1_000), 9)
            .unwrap();
        store
            .create_secret_request(secret("stuck", 1_000), 9)
            .unwrap();
        store.claim_secret_request("stuck", 2_000).unwrap();
        store
            .create_secret_request(secret("fresh", 690_000), 9)
            .unwrap();
        let swept = store.sweep_secret_requests(700_000, 60_000).unwrap();
        let mut states: Vec<_> = swept
            .iter()
            .map(|r| (r.id.as_str(), r.state.as_str()))
            .collect();
        states.sort();
        assert_eq!(states, [("quiet", "expired"), ("stuck", "failed")]);
        assert!(
            store
                .sweep_secret_requests(700_001, 60_000)
                .unwrap()
                .is_empty()
        );
    }
    fn project_ids(store: &Store) -> Vec<String> {
        store
            .reader()
            .unwrap()
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }
    fn chat_ids(store: &Store, project: &str) -> Vec<String> {
        store
            .reader()
            .unwrap()
            .sessions()
            .unwrap()
            .into_iter()
            .filter(|s| s.project_id.as_deref() == Some(project))
            .map(|s| s.id)
            .collect()
    }
    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }
    #[test]
    fn new_projects_and_chats_go_to_the_bottom_and_reordering_moves_only_the_named_items() {
        let (_temp, store) = fixture();
        // Created out of alphabetical order, so only position can explain the result.
        for (id, path) in [("pz", "/work/z"), ("pa", "/work/a"), ("pm", "/work/m")] {
            store.add_project(id, path, id, 5).unwrap();
        }
        let first = project_ids(&store);
        assert_eq!(
            first[first.len() - 3..],
            ids(&["pz", "pa", "pm"]),
            "creation order"
        );
        // Four chats in one project, one in another, all created at the same instant.
        let project = first[0].clone();
        for id in ["c4", "c1", "c3", "c2"] {
            store
                .create_session_in(NewSession {
                    name: "",
                    id,
                    project_id: &project,
                    agent: "claude",
                    model: "sonnet",
                    effort: None,
                    created_at: 9,
                })
                .unwrap();
        }
        store
            .create_session_in(NewSession {
                name: "",
                id: "other",
                project_id: "pa",
                agent: "claude",
                model: "sonnet",
                effort: None,
                created_at: 9,
            })
            .unwrap();
        let before = chat_ids(&store, &project);
        assert_eq!(before[before.len() - 4..], ids(&["c4", "c1", "c3", "c2"]));

        // Drag pm to the top of the three, and c2 up two places.
        store.reorder_projects(&ids(&["pm", "pz", "pa"])).unwrap();
        let after = project_ids(&store);
        assert_eq!(after[after.len() - 3..], ids(&["pm", "pz", "pa"]));
        assert_eq!(
            after[..after.len() - 3],
            first[..first.len() - 3],
            "projects not named keep their place"
        );
        store
            .reorder_sessions(&project, &ids(&["c4", "c2", "c1", "c3"]))
            .unwrap();
        let moved = chat_ids(&store, &project);
        assert_eq!(moved[moved.len() - 4..], ids(&["c4", "c2", "c1", "c3"]));
        assert_eq!(
            chat_ids(&store, "pa"),
            ids(&["other"]),
            "another project's chats are untouched"
        );

        // A project or chat created later still lands at the bottom.
        store.add_project("pn", "/work/n", "pn", 1).unwrap();
        assert_eq!(project_ids(&store).last().unwrap(), "pn");
        store
            .create_session_in(NewSession {
                name: "",
                id: "late",
                project_id: &project,
                agent: "claude",
                model: "sonnet",
                effort: None,
                created_at: 1,
            })
            .unwrap();
        assert_eq!(chat_ids(&store, &project).last().unwrap(), "late");
        // Naming only some items rearranges just those, in the slots they already held.
        store
            .reorder_sessions(&project, &ids(&["c3", "c4"]))
            .unwrap();
        let some = chat_ids(&store, &project);
        let slot = |id: &str| some.iter().position(|x| x == id).unwrap();
        assert!(slot("c3") < slot("c4"));
        assert_eq!(some.last().unwrap(), "late");
    }
    #[test]
    fn reordering_refuses_what_does_not_exist_or_does_not_belong() {
        let (_temp, store) = fixture();
        store.add_project("pa", "/work/a", "a", 1).unwrap();
        store.add_project("pb", "/work/b", "b", 1).unwrap();
        for (id, project) in [("a1", "pa"), ("b1", "pb")] {
            store
                .create_session_in(NewSession {
                    name: "",
                    id,
                    project_id: project,
                    agent: "claude",
                    model: "sonnet",
                    effort: None,
                    created_at: 2,
                })
                .unwrap();
        }
        let before = (project_ids(&store), chat_ids(&store, "pa"));
        assert!(matches!(
            store.reorder_projects(&ids(&["pa", "ghost"])),
            Err(Error::ProjectMissing)
        ));
        assert!(matches!(
            store.reorder_projects(&ids(&["pa", "pa"])),
            Err(Error::ReorderInvalid(_))
        ));
        // b1 is a chat, but not one of pa's.
        let foreign = store
            .reorder_sessions("pa", &ids(&["a1", "b1"]))
            .unwrap_err()
            .to_string();
        assert!(
            foreign.contains("b1") && foreign.contains("not in this project"),
            "{foreign}"
        );
        assert!(matches!(
            store.reorder_sessions("pa", &ids(&["a1", "a1"])),
            Err(Error::ReorderInvalid(_))
        ));
        assert_eq!(
            (project_ids(&store), chat_ids(&store, "pa")),
            before,
            "a refused reorder changes nothing"
        );
        // An empty or single-item list is fine and changes nothing.
        store.reorder_projects(&[]).unwrap();
        store.reorder_sessions("pa", &ids(&["a1"])).unwrap();
    }
    #[test]
    fn migration_016_numbers_existing_projects_and_chats_in_their_old_order() {
        let temp = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
            .unwrap();
        let path = temp.path().join("old.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            let mut names: Vec<_> = crate::migrations::MIGRATIONS.iter().collect();
            names.sort();
            for (name, sql) in names {
                if name.starts_with("016") {
                    continue;
                }
                conn.execute_batch(sql)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
            conn.execute_batch(
                "INSERT INTO projects (id, path, name, created_at) VALUES ('p2', '/b', 'b', 20), ('p1', '/a', 'a', 10), ('p3', '/c', 'c', 20);
                 INSERT INTO sessions (id, working_directory, project_id, created_at) VALUES
                   ('s3', '/a', 'p1', 30), ('s1', '/a', 'p1', 10), ('s2', '/a', 'p1', 10), ('t1', '/b', 'p2', 5);",
            )
            .unwrap();
            conn.execute_batch(include_str!("../migrations/016_positions.sql"))
                .unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        let order = |sql: &str| -> Vec<String> {
            let mut statement = conn.prepare(sql).unwrap();
            statement
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(
            order("SELECT id FROM projects ORDER BY position"),
            ["p1", "p2", "p3"]
        );
        assert_eq!(
            order("SELECT id FROM sessions WHERE project_id = 'p1' ORDER BY position"),
            ["s1", "s2", "s3"]
        );
        let lowest: i64 = conn
            .query_row(
                "SELECT MIN(position) FROM sessions WHERE project_id = 'p2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lowest, 1, "each list is numbered from 1");
    }
    #[test]
    fn migration_014_gives_existing_chats_projects_and_renames_the_old_event_kinds() {
        let temp = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
            .unwrap();
        let path = temp.path().join("old.sqlite");
        // A database as it was before projects: apply 001 to 013 by hand, add data, then open it.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            let mut names: Vec<_> = crate::migrations::MIGRATIONS.iter().collect();
            names.sort();
            for (name, sql) in names {
                // The database as it was before projects: everything up to 013.
                if *name >= "014" {
                    continue;
                }
                conn.execute_batch(sql)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
            conn.execute_batch(
                "INSERT INTO sessions (id, working_directory, claude_session_id, created_at) VALUES
                   ('a', '/work/app', 'resume-a', 5), ('b', '/work/app', NULL, 3), ('c', '/home/me/other', NULL, 9), ('r', '/', NULL, 1), ('d', '/work/app/', NULL, 8);
                 INSERT INTO events (session, seq, kind, payload, created_at) VALUES
                   ('a', 1, 'claude_event', '{\"type\":\"x\"}', 1),
                   ('a', 2, 'claude_session_changed', '{\"claude_session_id\":\"resume-a\",\"previous\":null}', 2);",
            )
            .unwrap();
        }
        // The version tracking the real runner uses is Marmot's; opening applies what is missing.
        drop(Connection::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/014_projects_and_agents.sql"))
            .unwrap();
        let project = |dir: &str| -> (String, String) {
            conn.query_row(
                "SELECT p.name, p.id FROM projects p WHERE p.path = ?1",
                [dir],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(project("/work/app").0, "app");
        assert_eq!(project("/home/me/other").0, "other");
        assert_eq!(project("/").0, "/", "a root folder is named by its path");
        assert_eq!(project("/work/app").1.len(), 36);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 3,
            "one project per distinct folder; a trailing slash is the same folder"
        );
        let slashed: String = conn
            .query_row("SELECT project_id FROM sessions WHERE id = 'd'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(slashed, project("/work/app").1);
        let (resume, linked, agent): (Option<String>, Option<String>, String) = conn
            .query_row(
                "SELECT agent_session_id, project_id, agent FROM sessions WHERE id = 'a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (resume.as_deref(), agent.as_str()),
            (Some("resume-a"), "claude")
        );
        assert_eq!(linked.unwrap(), project("/work/app").1);
        let refused = conn
            .execute(
                "INSERT INTO sessions (id, working_directory, created_at) VALUES ('z', '/x', 1)",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(refused.contains("must belong to a project"), "{refused}");
        let mut statement = conn
            .prepare("SELECT kind, payload FROM events ORDER BY seq")
            .unwrap();
        let kinds: Vec<(String, String)> = statement
            .query_map([], |r| -> rusqlite::Result<(String, String)> {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(kinds[0].0, "agent_event");
        assert_eq!(kinds[1].0, "agent_session_changed");
        assert!(
            kinds[1].1.contains("agent_session_id") && !kinds[1].1.contains("claude_session_id"),
            "{}",
            kinds[1].1
        );
    }
    fn append_content(store: &Store, uuid: &str, content: &str) {
        store
            .append(NewMessage {
                uuid,
                session: "s",
                role: "user",
                content,
                delivery_state: "queued",
                created_at: 2,
            })
            .unwrap();
    }
    fn append(store: &Store, uuid: &str, session: &str) -> Result<bool> {
        store.append(NewMessage {
            uuid,
            session,
            role: "user",
            content: "hello",
            delivery_state: "queued",
            created_at: 2,
        })
    }
    #[test]
    fn append_ranges_uuid_and_reopen() {
        let (temp, store) = fixture();
        store.create_session("other", "/host/other", 1).unwrap();
        append(&store, "a", "s").unwrap();
        append(&store, "b", "s").unwrap();
        append(&store, "c", "other").unwrap();
        assert!(!append(&store, "a", "s").unwrap());
        assert!(append(&store, "missing", "absent").is_err());
        store.set_agent_session("s", "claude-id").unwrap();
        drop(store);
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        let reader = store.reader().unwrap();
        let rows = reader.messages("s", 1, 3).unwrap();
        assert_eq!(
            rows.iter()
                .map(|m| (m.uuid.as_str(), m.seq))
                .collect::<Vec<_>>(),
            vec![("a", 1), ("b", 2)]
        );
        assert_eq!(reader.messages("s", 2, 3).unwrap().len(), 1);
        assert_eq!(reader.message("c").unwrap().unwrap().seq, 1);
        assert!(reader.message("absent").unwrap().is_none());
        assert_eq!(
            reader
                .sessions()
                .unwrap()
                .iter()
                .find(|s| s.id == "s")
                .unwrap()
                .agent_session_id
                .as_deref(),
            Some("claude-id")
        );
    }
    #[test]
    fn transitions_compare_and_set_and_persist() {
        let (_temp, store) = fixture();
        append(&store, "a", "s").unwrap();
        for (expected, next) in [
            ("queued", "dispatching"),
            ("dispatching", "rejected"),
            ("rejected", "queued"),
            ("queued", "dispatching"),
            ("dispatching", "uncertain"),
        ] {
            store.transition("a", expected, next).unwrap();
        }
        assert!(store.transition("a", "queued", "started").is_err());
        assert!(store.transition("a", "uncertain", "typo").is_err());
        assert_eq!(
            store
                .reader()
                .unwrap()
                .message("a")
                .unwrap()
                .unwrap()
                .delivery_state,
            "uncertain"
        );
        append(&store, "b", "s").unwrap();
        store.transition("b", "queued", "dispatching").unwrap();
        store.transition("b", "dispatching", "started").unwrap();
    }
    #[test]
    fn retries_events_and_resume_are_durable() {
        let (temp, store) = fixture();
        assert!(append(&store, "a", "s").unwrap());
        store.transition("a", "queued", "dispatching").unwrap();
        assert!(!append(&store, "a", "s").unwrap());
        assert!(
            store
                .append(NewMessage {
                    uuid: "a",
                    session: "s",
                    role: "user",
                    content: "different",
                    delivery_state: "queued",
                    created_at: 9
                })
                .is_err()
        );
        assert!(store.transition("a", "queued", "started").is_err());
        let events = store.reader().unwrap().events_since("s", 0, 100).unwrap();
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        store.create_session("other", "/host/other", 1).unwrap();
        assert_eq!(
            store
                .reader()
                .unwrap()
                .events_since("other", 0, 10)
                .unwrap()[0]
                .seq,
            1
        );
        drop(store);
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        store
            .record_event(
                "s",
                "agent_event",
                serde_json::json!({"type": "new_event"}),
                4,
            )
            .unwrap();
        let resumed = store.reader().unwrap().events_since("s", 2, 1).unwrap();
        assert_eq!(resumed[0].seq, 3);
        assert_eq!(
            store.reader().unwrap().events_since("s", 3, 100).unwrap()[0].seq,
            4
        );
        assert_eq!(
            store
                .reader()
                .unwrap()
                .message("a")
                .unwrap()
                .unwrap()
                .delivery_state,
            "dispatching"
        );
    }

    #[test]
    fn event_failure_rolls_back_message_and_transition() {
        let (_temp, store) = fixture();
        append(&store, "a", "s").unwrap();
        store.writer.lock().unwrap().execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'test event failure'); END;").unwrap();
        assert!(append(&store, "b", "s").is_err());
        assert!(store.transition("a", "queued", "dispatching").is_err());
        let reader = store.reader().unwrap();
        assert!(reader.message("b").unwrap().is_none());
        assert_eq!(
            reader.message("a").unwrap().unwrap().delivery_state,
            "queued"
        );
        assert_eq!(reader.events_since("s", 0, 100).unwrap().len(), 2);
    }

    #[test]
    fn cloned_writers_allocate_unique_sequences() {
        let (_temp, store) = fixture();
        let handles = (0..8)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || append(&store, &format!("message-{i}"), "s").unwrap())
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(handle.join().unwrap());
        }
        let reader = store.reader().unwrap();
        assert_eq!(
            reader
                .messages("s", 1, 100)
                .unwrap()
                .iter()
                .map(|m| m.seq)
                .collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>()
        );
        assert_eq!(
            reader
                .events_since("s", 0, 100)
                .unwrap()
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            (1..=9).collect::<Vec<_>>()
        );
    }

    #[test]
    fn archive_and_reopen_preserve_resume_id_history_and_events() {
        let (temp, store) = fixture();
        append(&store, "a", "s").unwrap();
        store.set_agent_session("s", "resume-me").unwrap();
        store.transition("a", "queued", "dispatching").unwrap();
        store.transition("a", "dispatching", "uncertain").unwrap();
        assert!(!store.reader().unwrap().sessions().unwrap()[0].archived);
        store.set_archived("s", true).unwrap();
        drop(store);
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        assert!(store.reader().unwrap().sessions().unwrap()[0].archived);
        store.set_archived("s", false).unwrap();
        let reader = store.reader().unwrap();
        let session = &reader.sessions().unwrap()[0];
        assert!(!session.archived);
        assert_eq!(session.agent_session_id.as_deref(), Some("resume-me"));
        assert_eq!(
            reader.message("a").unwrap().unwrap().delivery_state,
            "uncertain"
        );
        assert_eq!(reader.messages("s", 1, 100).unwrap().len(), 1);
        assert_eq!(
            reader
                .events_since("s", 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "archive_changed")
                .count(),
            2
        );
        assert!(store.set_archived("absent", true).is_err());
    }

    #[test]
    fn model_effort_survive_restart_and_archive() {
        let (temp, store) = fixture();
        let initial = &store.reader().unwrap().sessions().unwrap()[0];
        assert_eq!(initial.model, "sonnet");
        assert!(initial.effort.is_none());
        store.set_model_effort("s", "opus", Some("high")).unwrap();
        store.set_archived("s", true).unwrap();
        drop(store);
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        store.set_archived("s", false).unwrap();
        let reader = store.reader().unwrap();
        let session = &reader.sessions().unwrap()[0];
        assert_eq!(session.model, "opus");
        assert_eq!(session.effort.as_deref(), Some("high"));
        assert!(store.set_model_effort("s", "", Some("high")).is_err());
        assert_eq!(reader.sessions().unwrap()[0].model, "opus");
        assert_eq!(
            reader
                .events_since("s", 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "model_effort_changed")
                .count(),
            1
        );
    }

    #[test]
    fn concurrent_reader_sees_committed_snapshot_during_write() {
        let (_temp, store) = fixture();
        append(&store, "a", "s").unwrap();
        let reader = store.reader().unwrap();
        let mut writer = store.writer.lock().unwrap();
        let tx = writer.transaction().unwrap();
        messages::sql_for_test(&tx);
        std::thread::spawn(move || {
            assert_eq!(reader.messages("s", 1, 100).unwrap().len(), 1);
        })
        .join()
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            store.reader().unwrap().messages("s", 1, 100).unwrap().len(),
            2
        );
    }
    #[test]
    fn recent_message_events_report_current_delivery_without_rewriting_history() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("test.sqlite")).unwrap();
        store.create_session("s", "/local", 1).unwrap();
        store
            .append(NewMessage {
                uuid: "wake",
                session: "s",
                role: "user",
                content: "wake up",
                delivery_state: "queued",
                created_at: 2,
            })
            .unwrap();
        store.transition("wake", "queued", "dispatching").unwrap();
        store.transition("wake", "dispatching", "started").unwrap();
        let reader = store.reader().unwrap();
        let messages = reader.recent_message_events("s", 10).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&messages[0].payload).unwrap();
        assert_eq!(payload["delivery_state"], "started");
        assert_eq!(payload["content"], "wake up");
        let original = reader
            .events_since("s", 0, 100)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "message_appended")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&original.payload).unwrap();
        assert_eq!(payload["delivery_state"], "queued");
    }
}

mod usage;
pub use usage::UsageSamples;

#[cfg(test)]
mod busy_tests {
    use super::Error;
    #[test]
    fn only_busy_and_locked_sql_failures_count_as_busy() {
        let failure = |code| {
            Error::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        assert!(failure(rusqlite::ffi::SQLITE_BUSY).is_busy());
        assert!(failure(rusqlite::ffi::SQLITE_LOCKED).is_busy());
        assert!(!failure(rusqlite::ffi::SQLITE_CONSTRAINT).is_busy());
        assert!(!Error::Transition.is_busy());
    }
}

#[cfg(test)]
mod write_lock_tests {
    use super::*;

    /// A deferred transaction would let another process commit between its read and its
    /// write, and then fail at once. Holding the lock from the start makes others wait.
    #[test]
    fn a_store_transaction_holds_the_write_lock_before_its_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.sqlite");
        let store = Store::open(&path).unwrap();
        let other = Connection::open(&path).unwrap();
        let mut conn = store.writer.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let blocked = other.execute_batch("BEGIN IMMEDIATE").unwrap_err();
        assert!(
            matches!(blocked, rusqlite::Error::SqliteFailure(ref f, _) if f.code == rusqlite::ErrorCode::DatabaseBusy),
            "{blocked}"
        );
        tx.commit().unwrap();
    }
}
