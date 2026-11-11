#[path = "questions.generated.rs"]
mod sql;
use crate::{Error, Result, Store, events::append};
pub use sql::LookupRow as Question;

impl Store {
    /// Identical retries return the original question, including its answer.
    pub fn ask(
        &self,
        id: &str,
        session: &str,
        question: &str,
        choices: &[String],
        now: i64,
    ) -> Result<Question> {
        let choices = serde_json::to_string(choices).map_err(std::io::Error::other)?;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::edit_state(&tx, sql::EditStateParams { id })?
            .pop()
            .is_some_and(|q| q.retired)
        {
            return Err(Error::QuestionMissing);
        }
        if let Some(old) = sql::lookup(&tx, sql::LookupParams { id })?.pop() {
            if old.session != session || old.question != question || old.choices != choices {
                return Err(Error::QuestionConflict);
            }
            return Ok(old);
        }
        sql::create(
            &tx,
            sql::CreateParams {
                id,
                session,
                question,
                choices: &choices,
                created_at: now,
            },
        )?;
        append(
            &tx,
            session,
            "questions_changed",
            serde_json::json!({"question_id": id, "question": question}),
            now,
        )?;
        let question = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok(question)
    }
    /// Replace only a pending prompt, preserving its FIFO position. The replacement
    /// UUID is the retry receipt; retired IDs can never recreate an old question.
    pub fn update_question(
        &self,
        id: &str,
        replacement: &str,
        session: &str,
        question: &str,
        choices: &[String],
        now: i64,
    ) -> Result<()> {
        let choices = serde_json::to_string(choices).map_err(std::io::Error::other)?;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(old) = sql::edit_state(&tx, sql::EditStateParams { id: replacement })?.pop() {
            return if old.session == session
                && old.question == question
                && old.choices == choices
                && old.replacement_of.as_deref() == Some(id)
            {
                Ok(())
            } else {
                Err(Error::QuestionConflict)
            };
        }
        let old = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .filter(|q| q.session == session)
            .ok_or(Error::QuestionMissing)?;
        if old.answer.is_some() || old.discuss_later != 0 {
            return Err(Error::QuestionAnswered);
        }
        sql::replace(
            &tx,
            sql::ReplaceParams {
                id: replacement,
                previous: id,
                question,
                choices: &choices,
                created_at: now,
            },
        )?;
        sql::retire(&tx, sql::RetireParams { id })?;
        sql::suppress_retired_notices(&tx, sql::SuppressRetiredNoticesParams { id })?;
        append(
            &tx,
            session,
            "questions_changed",
            serde_json::json!({"question_id": replacement, "question": question}),
            now,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove a question from the drawer and outbox, keeping its UUID tombstone
    /// so an old create or update retry cannot restore it.
    pub fn delete_question(&self, id: &str, session: &str, now: i64) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let old = sql::edit_state(&tx, sql::EditStateParams { id })?
            .pop()
            .filter(|q| q.session == session)
            .ok_or(Error::QuestionMissing)?;
        if old.retired {
            return Ok(());
        }
        sql::retire(&tx, sql::RetireParams { id })?;
        sql::suppress_retired_notices(&tx, sql::SuppressRetiredNoticesParams { id })?;
        append(
            &tx,
            session,
            "questions_changed",
            serde_json::json!({}),
            now,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Persist the first answer and its delivery outbox together. Later answers cannot overwrite it.
    pub fn answer_question(
        &self,
        id: &str,
        session: &str,
        answer: &str,
        now: i64,
    ) -> Result<Question> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let old = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .filter(|q| q.session == session)
            .ok_or(Error::QuestionMissing)?;
        if old.discuss_later != 0 {
            return Err(Error::QuestionAnswered);
        }
        if let Some(previous) = &old.answer {
            return if previous == answer {
                Ok(old)
            } else {
                Err(Error::QuestionAnswered)
            };
        }
        sql::answer(
            &tx,
            sql::AnswerParams {
                id,
                answer: Some(answer),
                now: Some(now),
            },
        )?;
        append(
            &tx,
            session,
            "questions_changed",
            serde_json::json!({}),
            now,
        )?;
        let question = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok(question)
    }
    /// Close the structured prompt without inventing an answer. Its discussion request waits for the queue to empty.
    pub fn discuss_question_later(&self, id: &str, session: &str, now: i64) -> Result<Question> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let old = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .filter(|q| q.session == session)
            .ok_or(Error::QuestionMissing)?;
        if old.answer.is_some() {
            return Err(Error::QuestionAnswered);
        }
        if old.discuss_later != 0 {
            return Ok(old);
        }
        sql::defer(&tx, sql::DeferParams { id, now: Some(now) })?;
        append(
            &tx,
            session,
            "questions_changed",
            serde_json::json!({}),
            now,
        )?;
        let question = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok(question)
    }
    pub fn accept_question_delivery(&self, id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::accepted(&mut *conn, sql::AcceptedParams { id })?;
        Ok(())
    }
}
impl From<sql::ListAllRow> for Question {
    fn from(row: sql::ListAllRow) -> Self {
        Self {
            id: row.id,
            session: row.session,
            question: row.question,
            choices: row.choices,
            answer: row.answer,
            created_at: row.created_at,
            answered_at: row.answered_at,
            delivery_accepted: row.delivery_accepted,
            discuss_later: row.discuss_later,
            deferred_at: row.deferred_at,
        }
    }
}

impl crate::Reader {
    /// One live question by ID, including a subagent's question.
    pub fn question(&self, id: &str) -> Result<Option<Question>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { id })?.pop())
    }

    /// The chat that asked a question, including a retired question's original chat.
    pub fn question_owner_session(&self, id: &str) -> Result<Option<String>> {
        Ok(sql::owner_session(&self.conn, sql::OwnerSessionParams { id })?.pop())
    }

    /// Global question queue, including the oldest pending question.
    pub fn all_questions(&self) -> Result<Vec<Question>> {
        Ok(sql::list_all(
            &self.conn,
            sql::ListAllParams {
                limit: boblm_tuning::MAX_QUESTIONS_LIST,
            },
        )?
        .into_iter()
        .map(Question::from)
        .collect())
    }

    pub fn pending_question_count_all(&self) -> Result<i64> {
        Ok(sql::count_pending_all(&self.conn)?.pop().unwrap_or(0))
    }

    pub fn pending_question_count(&self, session: &str) -> Result<i64> {
        Ok(
            sql::count_pending(&self.conn, sql::CountPendingParams { session })?
                .pop()
                .unwrap_or(0),
        )
    }
    pub fn questions(&self, session: &str) -> Result<Vec<Question>> {
        sql::list(
            &self.conn,
            sql::ListParams {
                session,
                limit: boblm_tuning::MAX_QUESTIONS_LIST,
            },
        )?
        .into_iter()
        .map(|r| {
            Ok(Question {
                id: r.id,
                session: r.session,
                question: r.question,
                choices: r.choices,
                answer: r.answer,
                created_at: r.created_at,
                answered_at: r.answered_at,
                delivery_accepted: r.delivery_accepted,
                discuss_later: r.discuss_later,
                deferred_at: r.deferred_at,
            })
        })
        .collect()
    }
    pub fn pending_question_answers(&self) -> Result<Vec<Question>> {
        sql::pending_answers(&self.conn)?
            .into_iter()
            .map(|r| {
                Ok(Question {
                    id: r.id,
                    session: r.session,
                    question: r.question,
                    choices: r.choices,
                    answer: r.answer,
                    created_at: r.created_at,
                    answered_at: r.answered_at,
                    delivery_accepted: r.delivery_accepted,
                    discuss_later: r.discuss_later,
                    deferred_at: r.deferred_at,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, Store};
    #[test]
    fn questions_persist_deduplicate_and_keep_only_the_first_answer() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("questions.sqlite");
        let store = Store::open(&path).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        store.create_session("t", "/tmp", 1).unwrap();
        store
            .ask("q", "s", "Which color?", &["Blue".into()], 2)
            .unwrap();
        store
            .ask("q", "s", "Which color?", &["Blue".into()], 3)
            .unwrap();
        assert!(matches!(
            store.ask("q", "t", "Which color?", &["Blue".into()], 3),
            Err(Error::QuestionConflict)
        ));
        assert!(matches!(
            store.answer_question("q", "t", "Blue", 4),
            Err(Error::QuestionMissing)
        ));
        let q = store.answer_question("q", "s", "Green", 4).unwrap();
        assert_eq!(q.answer.as_deref(), Some("Green"));
        assert!(matches!(
            store.answer_question("q", "s", "Blue", 5),
            Err(Error::QuestionAnswered)
        ));
        assert_eq!(
            store
                .answer_question("q", "s", "Green", 5)
                .unwrap()
                .answered_at,
            Some(4)
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .reader()
                .unwrap()
                .pending_question_answers()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store.reader().unwrap().questions("s").unwrap()[0]
                .answer
                .as_deref(),
            Some("Green")
        );
        store.accept_question_delivery("q").unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_question_answers()
                .unwrap()
                .is_empty()
        );
        store.set_archived("s", true).unwrap();
        store.delete_session("s").unwrap();
        assert!(store.reader().unwrap().questions("s").unwrap().is_empty());
    }
}

#[cfg(test)]
mod queue_tests {
    use crate::Store;
    #[test]
    fn a_large_question_queue_is_fifo_and_discussion_waits_until_all_prompts_are_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.sqlite");
        let store = Store::open(&path).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        for i in 0..75 {
            store
                .ask(&format!("q{i:03}"), "s", &format!("Question {i}"), &[], 2)
                .unwrap();
        }
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            75
        );
        assert_eq!(
            store.reader().unwrap().questions("s").unwrap()[0].id,
            "q000"
        );
        let deferred = store.discuss_question_later("q000", "s", 3).unwrap();
        assert!(deferred.answer.is_none());
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            74
        );
        assert_eq!(
            store.reader().unwrap().questions("s").unwrap()[0].id,
            "q001"
        );
        assert!(
            store
                .reader()
                .unwrap()
                .pending_question_answers()
                .unwrap()
                .is_empty(),
            "discussion must not interrupt the remaining prompts"
        );
        assert!(store.answer_question("q000", "s", "invented", 4).is_err());
        for i in 1..75 {
            store
                .answer_question(&format!("q{i:03}"), "s", "Blue", 4)
                .unwrap();
        }
        let ready = store.reader().unwrap().pending_question_answers().unwrap();
        assert_eq!(ready.len(), 75);
        assert_eq!(
            ready.last().unwrap().id,
            "q000",
            "ordinary answers are handed off before discussions"
        );
        assert!(ready.last().unwrap().answer.is_none());
        drop(store);
        let store = Store::open(path).unwrap();
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            0
        );
        assert_eq!(
            store
                .reader()
                .unwrap()
                .pending_question_answers()
                .unwrap()
                .len(),
            75
        );
    }
}

#[cfg(test)]
mod edit_tests {
    use crate::{Error, Store};
    #[test]
    fn question_edits_preserve_fifo_and_retries_cannot_restore_old_prompts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("edits.sqlite");
        let store = Store::open(&path).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        store.create_session("t", "/tmp", 1).unwrap();
        store.ask("first", "s", "First?", &[], 2).unwrap();
        store.ask("second", "s", "Second?", &[], 3).unwrap();
        assert!(matches!(
            store.update_question("first", "revision", "t", "Revised?", &[], 4),
            Err(Error::QuestionMissing)
        ));
        store
            .update_question("first", "revision", "s", "Revised?", &["Yes".into()], 4)
            .unwrap();
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            2
        );
        let current = store.reader().unwrap().questions("s").unwrap();
        assert_eq!(current[0].id, "revision");
        assert_eq!(current[0].question, "Revised?");
        assert_eq!(current[0].choices, "[\"Yes\"]");
        assert!(matches!(
            store.answer_question("first", "s", "stale answer", 5),
            Err(Error::QuestionMissing)
        ));
        assert!(store.ask("first", "s", "First?", &[], 5).is_err());
        assert!(
            store
                .update_question("first", "revision", "s", "Different?", &[], 5)
                .is_err()
        );
        let before = store
            .reader()
            .unwrap()
            .events_since("s", 0, 100)
            .unwrap()
            .len();
        store
            .update_question("first", "revision", "s", "Revised?", &["Yes".into()], 5)
            .unwrap();
        assert_eq!(
            store
                .reader()
                .unwrap()
                .events_since("s", 0, 100)
                .unwrap()
                .len(),
            before
        );
        assert!(store.delete_question("revision", "t", 6).is_err());
        store.delete_question("revision", "s", 6).unwrap();
        store.delete_question("revision", "s", 7).unwrap();
        // Retrying an applied update after deletion must not resurrect its prompt.
        store
            .update_question("first", "revision", "s", "Revised?", &["Yes".into()], 8)
            .unwrap();
        assert_eq!(
            store.reader().unwrap().questions("s").unwrap()[0].id,
            "second"
        );
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            1
        );
        drop(store);
        let store = Store::open(path).unwrap();
        assert!(
            store
                .ask("revision", "s", "Revised?", &["Yes".into()], 9)
                .is_err()
        );
        store.delete_question("second", "s", 10).unwrap();
        assert!(store.reader().unwrap().questions("s").unwrap().is_empty());
        assert_eq!(
            store.reader().unwrap().pending_question_count("s").unwrap(),
            0
        );
        assert!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn question_deletion_suppresses_answer_and_discussion_outboxes() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("outbox.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        store.ask("answered", "s", "Color?", &[], 2).unwrap();
        store.answer_question("answered", "s", "Blue", 3).unwrap();
        assert!(matches!(
            store.update_question("answered", "new", "s", "Other?", &[], 4),
            Err(Error::QuestionAnswered)
        ));
        store.delete_question("answered", "s", 4).unwrap();
        store.ask("deferred", "s", "Why?", &[], 5).unwrap();
        store.discuss_question_later("deferred", "s", 6).unwrap();
        assert!(matches!(
            store.update_question("deferred", "new", "s", "Other?", &[], 7),
            Err(Error::QuestionAnswered)
        ));
        store.delete_question("deferred", "s", 7).unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_question_answers()
                .unwrap()
                .is_empty()
        );
        assert!(store.reader().unwrap().questions("s").unwrap().is_empty());
        assert!(
            store
                .answer_question("deferred", "s", "Old answer", 8)
                .is_err()
        );
    }
}

#[cfg(test)]
mod global_queue_tests {
    #[test]
    fn global_questions_order_by_creation_then_id_and_publish_list_events() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::Store::open(temp.path().join("global.sqlite")).unwrap();
        store.create_session("a", "/tmp", 1).unwrap();
        store.create_session("b", "/tmp", 1).unwrap();
        store.ask("z", "a", "Later ID?", &[], 10).unwrap();
        store.ask("a", "b", "Earlier ID?", &[], 10).unwrap();
        let reader = store.reader().unwrap();
        assert_eq!(reader.all_questions().unwrap()[0].id, "a");
        assert_eq!(reader.pending_question_count_all().unwrap(), 2);
        let events = reader.session_list_since(0, 256).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "questions_changed")
                .count(),
            2
        );
        store.answer_question("a", "b", "Yes", 11).unwrap();
        assert_eq!(reader.all_questions().unwrap()[0].id, "z");
        store.delete_question("z", "a", 12).unwrap();
        assert_eq!(reader.pending_question_count_all().unwrap(), 0);
    }
}
