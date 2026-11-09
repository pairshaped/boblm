//! Durable account limits and per-chat handoff exclusions (ADR 0047).
#[path = "claude_accounts.generated.rs"]
mod sql;
use crate::{Error, Reader, Result, Store};

/// A selected destination and the source reset window that prevents returning early.
#[derive(Debug, Clone)]
pub struct ClaudeHandoff {
    pub source: String,
    pub target: String,
    pub account_id: Option<String>,
    pub source_reset: i64,
}

impl Reader {
    pub fn claude_account_exhaustions(&self) -> Result<Vec<(String, i64)>> {
        Ok(sql::exhaustions(&self.conn)?
            .into_iter()
            .map(|row| (row.account_key, row.until))
            .collect())
    }
    pub fn claude_account_handoffs(&self, session: &str) -> Result<Vec<(String, i64)>> {
        Ok(sql::handoffs(&self.conn, sql::HandoffsParams { session })?
            .into_iter()
            .map(|row| (row.account_key, row.until))
            .collect())
    }
    pub fn claude_account_needs_continuation(&self, session: &str) -> Result<bool> {
        Ok(
            !sql::needs_continuation(&self.conn, sql::NeedsContinuationParams { session })?
                .is_empty(),
        )
    }
}

impl Store {
    /// Pin a newly created chat, before its runner can start.
    pub fn set_claude_account_key(&self, session: &str, key: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        crate::sessions::sql::set_account_key(
            &tx,
            crate::sessions::sql::SetAccountKeyParams {
                id: session,
                account_key: Some(key),
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    /// A quota rejection marks only this account, including for owner-created chats.
    pub fn exhaust_claude_account(
        &self,
        session: &str,
        key: &str,
        until: i64,
        plan: bool,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::exhaust(
            &tx,
            sql::ExhaustParams {
                account_key: key,
                until,
            },
        )?;
        if plan {
            sql::limited(&tx, sql::LimitedParams { session })?;
        }
        tx.commit()?;
        Ok(())
    }
    /// A later successful turn on the same account no longer needs a cutoff note.
    pub fn clear_claude_account_continuation(&self, session: &str) -> Result<()> {
        if !self.reader()?.claude_account_needs_continuation(session)? {
            return Ok(());
        }
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::clear_continuation(&tx, sql::ClearContinuationParams { session })?;
        tx.commit()?;
        Ok(())
    }
    /// Called only after the idle child's transcript has been copied successfully.
    /// The key, provider identity, no-return marker and continuation commit together.
    pub fn handoff_claude_account(
        &self,
        session: &str,
        change: &ClaudeHandoff,
        now: i64,
    ) -> Result<()> {
        let ClaudeHandoff {
            source,
            target,
            account_id,
            source_reset: until,
        } = change;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        crate::sessions::sql::set_account_key(
            &tx,
            crate::sessions::sql::SetAccountKeyParams {
                id: session,
                account_key: Some(target),
            },
        )?;
        crate::sessions::sql::set_account(
            &tx,
            crate::sessions::sql::SetAccountParams {
                id: session,
                account_id: account_id.as_deref(),
            },
        )?;
        sql::record_handoff(
            &tx,
            sql::RecordHandoffParams {
                session,
                account_key: source,
                until: *until,
            },
        )?;
        if !sql::needs_continuation(&tx, sql::NeedsContinuationParams { session })?.is_empty() {
            let uuid = uuid::Uuid::new_v4().to_string();
            let content = "Your last turn was cut off by the Claude account limit. BobLM moved this conversation to another account. Continue from where you stopped.";
            crate::messages::sql::append(
                &tx,
                crate::NewMessage {
                    uuid: &uuid,
                    session,
                    role: "user",
                    content,
                    delivery_state: "queued",
                    created_at: now,
                },
            )?;
            crate::events::append(
                &tx,
                session,
                "message_appended",
                serde_json::json!({"uuid":uuid,"role":"user","content":content,"delivery_state":"queued"}),
                now,
            )?;
            sql::clear_continuation(&tx, sql::ClearContinuationParams { session })?;
        }
        crate::events::append(
            &tx,
            session,
            "agent_event",
            serde_json::json!({"type":"system","subtype":"claude_account_changed","previous":source,"account_key":target}),
            now,
        )?;
        tx.commit()?;
        Ok(())
    }
}
