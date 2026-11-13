//! Temporary subagent grades kept as tuning data (ADR 0048).
#[path = "grades.generated.rs"]
mod sql;

use crate::{Error, Reader, Result, Store};
pub use sql::OneRow as SubagentGrade;

/// What `grade_subagent` reports back. It never carries routing facts: reviews are graded blind.
#[derive(Debug)]
pub struct GradeOutcome {
    pub session_id: String,
    pub replaced: bool,
}

impl Reader {
    pub fn subagent_grade(&self, session: &str) -> Result<Option<SubagentGrade>> {
        Ok(sql::one(&self.conn, sql::OneParams { session })?.pop())
    }
}

impl Store {
    /// Record or overwrite the grade for one chat. The caller must be the chat's parent or root,
    /// or the chat that spawned it. The model, effort, step and account are copied into the row.
    pub fn grade_subagent(
        &self,
        caller: &str,
        session: &str,
        rating: &str,
        note: Option<&str>,
        now: i64,
    ) -> Result<GradeOutcome> {
        if !["good", "ok", "poor"].contains(&rating) {
            return Err(Error::Grade(format!(
                "The rating {rating:?} is not one BobLM accepts. Use exactly good, ok or poor."
            )));
        }
        let note = note.map(str::trim).filter(|note| !note.is_empty());
        if let Some(note) = note
            && note.chars().count() > boblm_tuning::MAX_GRADE_NOTE_CHARS
        {
            return Err(Error::Grade(format!(
                "The note is {} characters long and the limit is {}. Shorten it or leave it out.",
                note.chars().count(),
                boblm_tuning::MAX_GRADE_NOTE_CHARS
            )));
        }
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let subject = sql::subject(&tx, sql::SubjectParams { id: session })?
            .pop()
            .ok_or_else(|| {
                Error::Grade(
                    "That chat no longer exists, so it cannot be graded. Grade a subagent before you delete it."
                        .into(),
                )
            })?;
        let receipt = sql::receipt(
            &tx,
            sql::ReceiptParams {
                request_id: session,
            },
        )?
        .pop();
        let allowed = subject.parent_session_id.as_deref() == Some(caller)
            || subject.root_session_id.as_deref() == Some(caller)
            || receipt
                .as_ref()
                .is_some_and(|receipt| receipt.caller == caller);
        if !allowed {
            return Err(Error::Grade(
                "Only the parent of a subagent, or the chat that spawned it, can grade it, and you are neither. Grade only subagents you started."
                    .into(),
            ));
        }
        let step = receipt
            .and_then(|receipt| receipt.resolved)
            .and_then(|resolved| serde_json::from_str::<serde_json::Value>(&resolved).ok())
            .and_then(|resolved| resolved["step"].as_str().map(str::to_owned));
        let replaced = !sql::existing(&tx, sql::ExistingParams { session })?.is_empty();
        sql::save(
            &tx,
            sql::SaveParams {
                session,
                grader: caller,
                rating,
                note,
                role: subject.role.as_deref(),
                agent: Some(&subject.agent),
                model: Some(&subject.model),
                effort: subject.effort.as_deref(),
                step: step.as_deref(),
                account_key: subject.account_key.as_deref(),
                title: Some(&subject.name),
                now,
            },
        )?;
        tx.commit()?;
        Ok(GradeOutcome {
            session_id: session.to_owned(),
            replaced,
        })
    }
}
