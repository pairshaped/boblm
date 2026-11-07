#[path = "notifications.generated.rs"]
mod sql;
use crate::{Error, Result, Store};
pub use sql::{
    InsertNoticeParams as NewNotification, NoticeRow as Notification,
    SubscribeParams as PushSubscription, SubscriptionsRow as Subscription,
};

pub(crate) fn enqueue(
    tx: &rusqlite::Transaction<'_>,
    new: NewNotification<'_>,
) -> Result<Notification> {
    if let Some(old) = sql::notice(tx, sql::NoticeParams { id: new.id })?.pop() {
        if old.session != new.session
            || old.category != new.category
            || old.title != new.title
            || old.message != new.message
            || old.question_id.as_deref() != new.question_id
        {
            return Err(Error::NotificationConflict);
        }
        return Ok(old);
    }
    let id = new.id;
    sql::insert_notice(tx, new)?;
    sql::notice(tx, sql::NoticeParams { id })?
        .pop()
        .ok_or(rusqlite::Error::QueryReturnedNoRows.into())
}
/// Application events use the same durable path as explicit agent notices.
pub(crate) fn automatic(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    kind: &str,
    payload: &serde_json::Value,
    now: i64,
) -> Result<()> {
    let notice = match kind {
        "questions_changed" if payload["question_id"].is_string() => Some((
            "questions",
            "A question needs your answer",
            payload["question"]
                .as_str()
                .unwrap_or("Open the Questions drawer to answer."),
            payload["question_id"].as_str(),
        )),
        "secret_requested" => Some((
            "questions",
            "A credential is requested",
            "Open this chat to submit it securely.",
            None,
        )),
        "driver_error" | "model_error" | "delivery_timeout" => Some((
            "failures",
            "A chat needs attention",
            "Open the chat to see what failed and what to do next.",
            None,
        )),
        "agent_event" if payload["type"] == "result" && payload["is_error"] == true => Some((
            "failures",
            "An agent turn failed",
            "Open the chat to see the failure.",
            None,
        )),
        "state_summary"
            if payload["running"] == true
                && payload["turnActive"] == false
                && payload["stopping"] == false
                && payload["archived"] == false
                && payload["queued"] == 0
                && payload["dispatching"] == 0
                && payload["uncertain"] == 0 =>
        {
            let previous = sql::previous_status(tx, sql::PreviousStatusParams { session })?
                .pop()
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()
                .map_err(std::io::Error::other)?;
            let end = sql::turn_end(tx, sql::TurnEndParams { session })?.pop();
            let ended_successfully = end
                .filter(|e| e.kind == "agent_event")
                .map(|e| serde_json::from_str::<serde_json::Value>(&e.payload))
                .transpose()
                .map_err(std::io::Error::other)?
                .is_some_and(|e| e["is_error"] != true);
            if previous.is_some_and(|p| p["turnActive"] == true) && ended_successfully {
                Some((
                    "completion",
                    "An agent turn finished",
                    "Open the chat to read the reply.",
                    None,
                ))
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some((category, title, message, question_id)) = notice {
        enqueue(
            tx,
            NewNotification {
                id: &format!("event:{}", uuid::Uuid::new_v4()),
                session,
                category,
                title,
                message: &message
                    .chars()
                    .take(boblm_tuning::MAX_NOTIFICATION_MESSAGE_CHARS)
                    .collect::<String>(),
                question_id,
                state: "pending",
                reason: "",
                created_at: now,
            },
        )?;
    }
    Ok(())
}
impl Store {
    pub fn notify(&self, new: NewNotification<'_>) -> Result<Notification> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let notice = enqueue(&tx, new)?;
        tx.commit()?;
        Ok(notice)
    }
    pub fn settle_notification(&self, id: &str, state: &str, reason: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::settle(&mut *conn, sql::SettleParams { id, state, reason })?;
        Ok(())
    }
    pub fn subscribe_push(&self, new: PushSubscription<'_>) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::subscribe(&mut *conn, new)?;
        Ok(())
    }
    pub fn unsubscribe_push(&self, id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::unsubscribe(&mut *conn, sql::UnsubscribeParams { id })?;
        Ok(())
    }
    /// A claim is persisted before network I/O. Uncertain sends are held and never replayed.
    pub fn claim_push_delivery(&self, notification: &str, subscription: &str) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        Ok(sql::claim_delivery(
            &mut *conn,
            sql::ClaimDeliveryParams {
                notification,
                subscription,
            },
        )? == 1)
    }
    pub fn finish_push_delivery(
        &self,
        notification: &str,
        subscription: &str,
        state: &str,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::finish_delivery(
            &mut *conn,
            sql::FinishDeliveryParams {
                notification,
                subscription,
                state,
            },
        )?;
        Ok(())
    }
}
impl crate::Reader {
    pub fn notification(&self, id: &str) -> Result<Option<Notification>> {
        Ok(sql::notice(&self.conn, sql::NoticeParams { id })?.pop())
    }
    pub fn pending_notifications(&self) -> Result<Vec<String>> {
        Ok(sql::pending(
            &self.conn,
            sql::PendingParams {
                limit: boblm_tuning::NOTIFICATION_BATCH_SIZE,
            },
        )?)
    }
    pub fn push_subscriptions(&self) -> Result<Vec<Subscription>> {
        Ok(sql::subscriptions(&self.conn)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn completion_requires_an_idle_acknowledged_turn_and_stops_do_not_notify() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("auto.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        let active = json!({"running":true,"turnActive":true,"stopping":false,"archived":false,"queued":0,"dispatching":0,"uncertain":0});
        let mut idle = active.clone();
        idle["turnActive"] = false.into();
        store.publish_summary("s", active.clone()).unwrap();
        store
            .record_event(
                "s",
                "agent_event",
                json!({"type":"result","subtype":"success","is_error":false}),
                2,
            )
            .unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .is_empty()
        );
        let mut waiting = idle.clone();
        waiting["dispatching"] = 1.into();
        store.publish_summary("s", waiting).unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .is_empty(),
            "a steer is still waiting on acknowledgment"
        );
        store.publish_summary("s", active.clone()).unwrap();
        store.publish_summary("s", idle.clone()).unwrap();
        let pending = store.reader().unwrap().pending_notifications().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            store
                .reader()
                .unwrap()
                .notification(&pending[0])
                .unwrap()
                .unwrap()
                .category,
            "completion"
        );
        store.publish_summary("s", active.clone()).unwrap();
        store
            .record_event("s", "session_stopped", json!({}), 3)
            .unwrap();
        let mut stopped = idle.clone();
        stopped["running"] = false.into();
        store.publish_summary("s", stopped).unwrap();
        assert_eq!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .len(),
            1
        );
        store.publish_summary("s", active).unwrap();
        store
            .record_event(
                "s",
                "agent_event",
                json!({"type":"result","is_error":true}),
                4,
            )
            .unwrap();
        store.publish_summary("s", idle).unwrap();
        assert_eq!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .len(),
            2,
            "a failure does not also get a completion notice"
        );
        store
            .record_event(
                "s",
                "secret_requested",
                json!({"file":"private-file","key":["private-key"]}),
                5,
            )
            .unwrap();
        let records = store
            .reader()
            .unwrap()
            .pending_notifications()
            .unwrap()
            .into_iter()
            .map(|id| store.reader().unwrap().notification(&id).unwrap().unwrap())
            .collect::<Vec<_>>();
        assert!(records.iter().all(|n| !n.message.contains("private-")));
    }
}
