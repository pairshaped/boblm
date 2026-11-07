#[path = "schedules.generated.rs"]
mod sql;

use crate::{Error, Reader, Result, Store};

/// A prompt boblm sends to a chat later: once, or repeatedly at an interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub id: String,
    pub session: String,
    pub prompt: String,
    /// The tool call that created it, kept so an identical retry can be recognized.
    pub arguments: String,
    /// None for a one-shot.
    pub interval_ms: Option<i64>,
    pub next_run_at: i64,
    pub ends_at: Option<i64>,
    pub max_runs: Option<i64>,
    pub runs: i64,
    pub skipped: i64,
    pub last_run_at: Option<i64>,
    /// `active`, `done` or `missed`.
    pub state: String,
    pub created_at: i64,
}
macro_rules! from_row {
    ($row:ty) => {
        impl From<$row> for Schedule {
            fn from(r: $row) -> Self {
                Self {
                    id: r.id,
                    session: r.session,
                    prompt: r.prompt,
                    arguments: r.arguments,
                    interval_ms: r.interval_ms,
                    next_run_at: r.next_run_at,
                    ends_at: r.ends_at,
                    max_runs: r.max_runs,
                    runs: r.runs,
                    skipped: r.skipped,
                    last_run_at: r.last_run_at,
                    state: r.state,
                    created_at: r.created_at,
                }
            }
        }
    };
}
from_row!(sql::LookupRow);
from_row!(sql::ForSessionRow);
from_row!(sql::DueRow);

pub struct NewSchedule<'a> {
    pub id: &'a str,
    pub session: &'a str,
    pub prompt: &'a str,
    pub arguments: &'a str,
    pub interval_ms: Option<i64>,
    pub next_run_at: i64,
    pub ends_at: Option<i64>,
    pub max_runs: Option<i64>,
    pub created_at: i64,
}

/// What happened to one due occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The prompt was sent. `next` is the following run, or None when this was the last.
    Fired { next: Option<i64> },
    /// The occurrence was passed over (chat busy, or too late for a repeating schedule).
    Skipped { next: Option<i64> },
    /// A one-shot too far overdue to send.
    Missed,
}

impl Store {
    /// Create a schedule, or return the existing one for an identical retry. A different call
    /// under the same ID, or a chat already holding `max_active` active schedules, fails.
    pub fn create_schedule(
        &self,
        new: NewSchedule<'_>,
        max_active: usize,
    ) -> Result<(Schedule, bool)> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(existing) = sql::lookup(&tx, sql::LookupParams { id: new.id })?.pop() {
            let existing = Schedule::from(existing);
            if existing.session != new.session || existing.arguments != new.arguments {
                return Err(Error::ScheduleConflict);
            }
            return Ok((existing, false));
        }
        let active = sql::count_active(
            &tx,
            sql::CountActiveParams {
                session: new.session,
            },
        )?
        .pop()
        .unwrap_or(0);
        if usize::try_from(active).unwrap_or(usize::MAX) >= max_active {
            return Err(Error::ScheduleLimit(max_active));
        }
        sql::create(
            &tx,
            sql::CreateParams {
                id: new.id,
                session: new.session,
                prompt: new.prompt,
                arguments: new.arguments,
                interval_ms: new.interval_ms,
                next_run_at: new.next_run_at,
                ends_at: new.ends_at,
                max_runs: new.max_runs,
                created_at: new.created_at,
            },
        )?;
        let created = sql::lookup(&tx, sql::LookupParams { id: new.id })?
            .pop()
            .map(Schedule::from)
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok((created, true))
    }

    /// Record the outcome of the occurrence due at `occurrence`. Returns false if the schedule
    /// was cancelled or already moved on, so a stale caller changes nothing.
    pub fn advance_schedule(&self, id: &str, occurrence: i64, outcome: Outcome) -> Result<bool> {
        let (ran, skipped, next, state) = match outcome {
            Outcome::Fired { next } => (1, 0, next, if next.is_some() { "active" } else { "done" }),
            Outcome::Skipped { next } => {
                (0, 1, next, if next.is_some() { "active" } else { "done" })
            }
            Outcome::Missed => (0, 0, None, "missed"),
        };
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let changed = sql::advance(
            &tx,
            sql::AdvanceParams {
                id,
                occurrence: Some(occurrence),
                ran: Some(ran),
                skipped,
                next_run_at: next.unwrap_or(occurrence),
                state,
            },
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Cancel (delete) a chat's schedule. Returns false if it does not exist in that chat.
    pub fn delete_schedule(&self, id: &str, session: &str) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let removed = sql::delete(&tx, sql::DeleteParams { id, session })?;
        tx.commit()?;
        Ok(removed == 1)
    }
}

impl Reader {
    pub fn schedule(&self, id: &str) -> Result<Option<Schedule>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { id })?
            .pop()
            .map(Schedule::from))
    }
    pub fn schedules(&self, session: &str) -> Result<Vec<Schedule>> {
        Ok(
            sql::for_session(&self.conn, sql::ForSessionParams { session })?
                .into_iter()
                .map(Schedule::from)
                .collect(),
        )
    }
    /// Active schedules whose time has come, in chats that are not archived.
    pub fn due_schedules(&self, now: i64) -> Result<Vec<Schedule>> {
        Ok(sql::due(&self.conn, sql::DueParams { now })?
            .into_iter()
            .map(Schedule::from)
            .collect())
    }
}
