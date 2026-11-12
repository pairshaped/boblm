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
impl Store {
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
    fn notification_only_new_questions_and_credentials_enqueue_and_retries_deduplicate() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("notifications.sqlite")).unwrap();
        store.create_session("s", "/tmp", 1).unwrap();
        store.ask("q", "s", "Which way?", &[], 2).unwrap();
        store.ask("q", "s", "Which way?", &[], 3).unwrap();
        assert_eq!(
            store.reader().unwrap().pending_notifications().unwrap(),
            vec!["question:q"]
        );
        let notice = store
            .reader()
            .unwrap()
            .notification("question:q")
            .unwrap()
            .unwrap();
        assert_eq!(notice.question_id.as_deref(), Some("q"));
        assert_eq!(notice.message, "Which way?");
        store
            .update_question("q", "r", "s", "Revised?", &[], 4)
            .unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .is_empty()
        );
        store.answer_question("r", "s", "Yes", 5).unwrap();
        store.delete_question("r", "s", 6).unwrap();
        store.ask("deferred", "s", "Later?", &[], 7).unwrap();
        store
            .settle_notification("question:deferred", "sent", "")
            .unwrap();
        store.discuss_question_later("deferred", "s", 8).unwrap();
        for _ in 0..2 {
            store
                .create_secret_request(
                    crate::NewSecretRequest {
                        id: "credential",
                        session: "s",
                        file: "/private/file",
                        key: &["private-key".into()],
                        expires_at: 100,
                        created_at: 8,
                    },
                    3,
                )
                .unwrap();
        }
        assert_eq!(
            store.reader().unwrap().pending_notifications().unwrap(),
            vec!["secret:credential"]
        );
        let notice = store
            .reader()
            .unwrap()
            .notification("secret:credential")
            .unwrap()
            .unwrap();
        assert_eq!(notice.category, "questions");
        assert_eq!(notice.title, "A credential is requested");
        assert_eq!(notice.message, "Open this chat to submit it securely.");
        assert!(notice.question_id.is_none());
        store
            .settle_notification("secret:credential", "sent", "")
            .unwrap();
        for (kind, payload) in [
            (
                "questions_changed",
                json!({"question_id":"fake","question":"No push"}),
            ),
            ("secret_requested", json!({})),
            ("driver_error", json!({})),
            ("model_error", json!({})),
            ("delivery_timeout", json!({})),
            ("agent_event", json!({"type":"result","is_error":true})),
        ] {
            store.record_event("s", kind, payload, 9).unwrap();
        }
        store
            .publish_summary("s", json!({"running":true,"turnActive":true}))
            .unwrap();
        store
            .record_event(
                "s",
                "agent_event",
                json!({"type":"result","is_error":false}),
                10,
            )
            .unwrap();
        store.publish_summary("s", json!({"running":true,"turnActive":false,"stopping":false,"archived":false,"queued":0,"dispatching":0,"uncertain":0})).unwrap();
        assert!(
            store
                .reader()
                .unwrap()
                .pending_notifications()
                .unwrap()
                .is_empty()
        );
    }
}
