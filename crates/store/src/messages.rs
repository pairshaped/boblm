#[path = "messages.generated.rs"]
pub(crate) mod sql;

use crate::{Error, Reader, Result, Store};
pub use sql::{AppendParams as NewMessage, LookupRow, RangeRow};

impl Store {
    /// Returns false for an identical retry, without adding a message or event.
    pub fn append(&self, message: NewMessage<'_>) -> Result<bool> {
        self.append_with_attachments(message, &[])
    }

    /// Append a message and claim its uploads in one transaction. A retry is identical
    /// only when it names the same attachments in the same order.
    pub fn append_with_attachments(
        &self,
        message: NewMessage<'_>,
        attachments: &[String],
    ) -> Result<bool> {
        self.append_with_origin(message, attachments, None)
    }

    /// The sender is assigned by the authenticated peer boundary, never by message text.
    pub fn append_with_origin(
        &self,
        message: NewMessage<'_>,
        attachments: &[String],
        sender: Option<&str>,
    ) -> Result<bool> {
        self.append_message(message, attachments, sender, false, false)
    }

    /// Accept fresh owner input and reopen its chat atomically. Identical retries do neither.
    pub fn append_reopening(
        &self,
        message: NewMessage<'_>,
        attachments: &[String],
    ) -> Result<bool> {
        self.append_message(message, attachments, None, true, true)
    }

    /// Reopen for an automated answer or notice without claiming human origin.
    pub fn append_automated_reopening(
        &self,
        message: NewMessage<'_>,
        attachments: &[String],
    ) -> Result<bool> {
        self.append_message(message, attachments, None, true, false)
    }

    fn append_message(
        &self,
        message: NewMessage<'_>,
        attachments: &[String],
        sender: Option<&str>,
        reopen: bool,
        human: bool,
    ) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(existing) = sql::lookup(&tx, sql::LookupParams { uuid: message.uuid })?.pop() {
            let recorded_sender = sql::sender(
                &tx,
                sql::SenderParams {
                    session: message.session,
                    uuid: message.uuid,
                },
            )?
            .pop()
            .flatten();
            // Legacy messages have no sender metadata. Identical retries remain no-ops.
            if existing.session != message.session
                || existing.role != message.role
                || existing.content != message.content
                || crate::attachments::linked_ids(&tx, message.uuid)? != attachments
                || recorded_sender
                    .as_deref()
                    .is_some_and(|old| Some(old) != sender)
            {
                return Err(Error::MessageConflict);
            }
            return Ok(false);
        }
        let claimed = crate::attachments::claim(&tx, message.session, attachments)?;
        sql::append(&tx, NewMessage { ..message })?;
        if human {
            sql::mark_human_input(&tx, sql::MarkHumanInputParams { uuid: message.uuid })?;
        }
        crate::attachments::link(&tx, message.uuid, &claimed)?;
        let mut payload = serde_json::json!({"uuid": message.uuid, "role": message.role, "content": message.content, "delivery_state": message.delivery_state});
        if let Some(sender) = sender {
            payload["sender_session_id"] = sender.into();
        }
        if !claimed.is_empty() {
            payload["attachments"] = claimed
                .iter()
                .map(|a| serde_json::json!({"id": a.id, "name": a.name, "contentType": a.content_type, "size": a.size}))
                .collect();
        }
        crate::events::append(
            &tx,
            message.session,
            "message_appended",
            payload,
            message.created_at,
        )?;
        if reopen {
            crate::sessions::reopen(&tx, message.session)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Compare-and-set persistence primitive. The runner owns transition policy.
    pub fn transition(&self, uuid: &str, expected: &str, next: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let message = sql::lookup(&tx, sql::LookupParams { uuid })?
            .pop()
            .ok_or(Error::Transition)?;
        if sql::transition(
            &tx,
            sql::TransitionParams {
                uuid,
                expected,
                next,
            },
        )? != 1
        {
            return Err(Error::Transition);
        }
        crate::events::append(
            &tx,
            &message.session,
            "delivery_changed",
            serde_json::json!({"uuid": uuid, "delivery_state": next}),
            crate::events::now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    }
}
impl Reader {
    /// Missing and legacy provenance is conservatively treated as automated input.
    pub fn human_input(&self, uuid: &str) -> Result<bool> {
        Ok(
            sql::human_input(&self.conn, sql::HumanInputParams { uuid })?
                .pop()
                .unwrap_or(false),
        )
    }

    /// Half-open sequence range, ordered within one session.
    pub fn messages(&self, session: &str, start: i64, end: i64) -> Result<Vec<RangeRow>> {
        Ok(sql::range(
            &self.conn,
            sql::RangeParams {
                session,
                start,
                end,
            },
        )?)
    }
    /// The dispatcher's working set excludes inputs the agent already consumed.
    pub fn outstanding_messages(&self, session: &str) -> Result<Vec<RangeRow>> {
        Ok(
            sql::outstanding(&self.conn, sql::OutstandingParams { session })?
                .into_iter()
                .map(|row| RangeRow {
                    uuid: row.uuid,
                    session: row.session,
                    seq: row.seq,
                    role: row.role,
                    content: row.content,
                    delivery_state: row.delivery_state,
                    created_at: row.created_at,
                    retry_count: row.retry_count,
                    expired_from: row.expired_from,
                })
                .collect(),
        )
    }
    pub fn message(&self, uuid: &str) -> Result<Option<LookupRow>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { uuid })?.pop())
    }
}

#[cfg(test)]
pub(crate) fn sql_for_test(tx: &rusqlite::Transaction<'_>) {
    sql::append(
        tx,
        NewMessage {
            uuid: "b",
            session: "s",
            role: "user",
            content: "pending",
            delivery_state: "queued",
            created_at: 3,
        },
    )
    .unwrap();
}

impl Store {
    /// Expire this chat's queued and unconfirmed messages created before `cutoff`, in one
    /// transaction. Only rows still `queued` or `uncertain` change, so a message that is
    /// dispatching or already consumed is untouched, and an expired row is never sent
    /// (`transition` and `outstanding_messages` skip it). Returns each expired message's UUID
    /// with the state it expired from. The runner owns when to sweep.
    pub fn expire_stale(
        &self,
        session: &str,
        now: i64,
        cutoff: i64,
    ) -> Result<Vec<(String, String)>> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let expired = expire_stale_in(&tx, session, now, cutoff)?;
        tx.commit()?;
        Ok(expired)
    }

    /// Move a queued message to dispatching only while it is unexpired and newer than
    /// `cutoff`, so a message that aged out during dispatch setup is never sent. A message
    /// that is too old is expired in the same transaction and this returns `Ok(false)`;
    /// the caller moves on. Any other refusal is `Error::Transition`.
    pub fn dispatch(&self, uuid: &str, now: i64, cutoff: i64) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let message = sql::lookup(&tx, sql::LookupParams { uuid })?
            .pop()
            .ok_or(Error::Transition)?;
        if sql::dispatch(&tx, sql::DispatchParams { uuid, cutoff })? == 1 {
            crate::events::append(
                &tx,
                &message.session,
                "delivery_changed",
                serde_json::json!({"uuid": uuid, "delivery_state": "dispatching"}),
                crate::events::now_ms(),
            )?;
            tx.commit()?;
            return Ok(true);
        }
        expire_stale_in(&tx, &message.session, now, cutoff)?;
        let expired = sql::lookup(&tx, sql::LookupParams { uuid })?
            .pop()
            .is_some_and(|row| row.delivery_state == "expired");
        tx.commit()?;
        if expired {
            Ok(false)
        } else {
            Err(Error::Transition)
        }
    }

    /// Runner supplies the next automatic retry count, or zero for an explicit retry.
    pub fn queue_retry(&self, uuid: &str, retry_count: i64) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let message = sql::lookup(&tx, sql::LookupParams { uuid })?
            .pop()
            .ok_or(Error::Transition)?;
        if sql::queue_retry(&tx, sql::QueueRetryParams { uuid, retry_count })? != 1 {
            return Err(Error::Transition);
        }
        crate::events::append(
            &tx,
            &message.session,
            "delivery_changed",
            serde_json::json!({"uuid": uuid, "delivery_state": "queued", "retry_count": retry_count}),
            crate::events::now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    /// Move a dispatching message to rejected in one transaction. `Some(count)` requeues
    /// it for an automatic retry; `None` leaves it rejected and announces that it needs
    /// the owner. The runner owns the retry limit.
    pub fn reject(&self, uuid: &str, automatic_retry: Option<i64>) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let message = sql::lookup(&tx, sql::LookupParams { uuid })?
            .pop()
            .ok_or(Error::Transition)?;
        if sql::transition(
            &tx,
            sql::TransitionParams {
                uuid,
                expected: "dispatching",
                next: "rejected",
            },
        )? != 1
        {
            return Err(Error::Transition);
        }
        let now = crate::events::now_ms();
        crate::events::append(
            &tx,
            &message.session,
            "delivery_changed",
            serde_json::json!({"uuid": uuid, "delivery_state": "rejected"}),
            now,
        )?;
        match automatic_retry {
            Some(retry_count) => {
                if sql::queue_retry(&tx, sql::QueueRetryParams { uuid, retry_count })? != 1 {
                    return Err(Error::Transition);
                }
                crate::events::append(
                    &tx,
                    &message.session,
                    "delivery_changed",
                    serde_json::json!({"uuid": uuid, "delivery_state": "queued", "retry_count": retry_count}),
                    now,
                )?;
            }
            None => crate::events::append(
                &tx,
                &message.session,
                "delivery_rejected",
                serde_json::json!({"uuid": uuid, "retry_available": true}),
                now,
            )?,
        }
        tx.commit()?;
        Ok(())
    }
}

fn expire_stale_in(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    now: i64,
    cutoff: i64,
) -> Result<Vec<(String, String)>> {
    let rows = sql::expire_stale(
        tx,
        sql::ExpireStaleParams {
            now: Some(now),
            session,
            cutoff,
        },
    )?;
    let mut expired = Vec::with_capacity(rows.len());
    for row in rows {
        let from = row.expired_from.ok_or(Error::Transition)?;
        crate::events::append(
            tx,
            session,
            "delivery_changed",
            serde_json::json!({"uuid": row.uuid, "delivery_state": "expired", "expired_from": from}),
            now,
        )?;
        expired.push((row.uuid, from));
    }
    Ok(expired)
}
