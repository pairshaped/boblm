//! Durable result and error reports from a subagent to its parent (ADR 0043).
#[path = "subagent_reports.generated.rs"]
mod sql;

use crate::{Reader, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReport {
    pub child_session: String,
    pub result_seq: i64,
    pub parent_session: String,
    pub state: String,
    pub source_kind: String,
    pub error_message: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub delivered_at: Option<i64>,
}

impl From<sql::AllRow> for SubagentReport {
    fn from(row: sql::AllRow) -> Self {
        Self {
            child_session: row.child_session,
            result_seq: row.result_seq,
            parent_session: row.parent_session,
            state: row.state,
            source_kind: row.source_kind,
            error_message: row.error_message,
            error: row.error,
            created_at: row.created_at,
            delivered_at: row.delivered_at,
        }
    }
}

impl From<sql::PendingRow> for SubagentReport {
    fn from(row: sql::PendingRow) -> Self {
        Self {
            child_session: row.child_session,
            result_seq: row.result_seq,
            parent_session: row.parent_session,
            state: row.state,
            source_kind: row.source_kind,
            error_message: row.error_message,
            error: row.error,
            created_at: row.created_at,
            delivered_at: row.delivered_at,
        }
    }
}

impl Reader {
    /// Assistant and result events belonging to this reported turn only.
    pub fn subagent_turn_events(
        &self,
        child_session: &str,
        result_seq: i64,
    ) -> Result<Vec<String>> {
        Ok(sql::turn_events(
            &self.conn,
            sql::TurnEventsParams {
                child_session,
                result_seq,
            },
        )?)
    }

    pub fn subagent_reports(&self) -> Result<Vec<SubagentReport>> {
        Ok(sql::all(&self.conn)?
            .into_iter()
            .map(SubagentReport::from)
            .collect())
    }

    pub fn pending_subagent_reports(&self) -> Result<Vec<SubagentReport>> {
        Ok(sql::pending(&self.conn)?
            .into_iter()
            .map(SubagentReport::from)
            .collect())
    }
}

impl Store {
    pub fn mark_subagent_report_delivered(
        &self,
        child_session: &str,
        result_seq: i64,
        delivered_at: i64,
        text_seq: Option<i64>,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::mark_delivered(
            &tx,
            sql::MarkDeliveredParams {
                child_session,
                result_seq,
                delivered_at: Some(delivered_at),
                delivered_text_seq: text_seq,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// A newer result queued behind older pending ones carries the child's final text, so the
    /// older reports are superseded and send nothing. Returns how many were superseded.
    pub fn supersede_older_subagent_reports(
        &self,
        child_session: &str,
        newest_seq: i64,
        superseded_at: i64,
    ) -> Result<usize> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        let count = sql::supersede_older(
            &tx,
            sql::SupersedeOlderParams {
                child_session,
                newest_seq,
                superseded_at: Some(superseded_at),
            },
        )?;
        tx.commit()?;
        Ok(count)
    }

    /// A pending result with no assistant text newer than the last delivered report's text
    /// would repeat that report, so it sends nothing. Returns how many were superseded.
    pub fn supersede_repeated_subagent_reports(&self, superseded_at: i64) -> Result<usize> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        let count = sql::supersede_repeats(
            &tx,
            sql::SupersedeRepeatsParams {
                superseded_at: Some(superseded_at),
            },
        )?;
        tx.commit()?;
        Ok(count)
    }

    pub fn mark_subagent_report_failed(
        &self,
        child_session: &str,
        result_seq: i64,
        failed_at: i64,
        error: &str,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::mark_failed(
            &tx,
            sql::MarkFailedParams {
                child_session,
                result_seq,
                failed_at: Some(failed_at),
                error: Some(error),
            },
        )?;
        tx.commit()?;
        Ok(())
    }
}
