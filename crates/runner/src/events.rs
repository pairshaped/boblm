//! Transport-independent durable cursor pages.
use crate::{Error, Result, SessionHandle, Status};
use boblm_protocol::client::{EventPage, Sequence, SessionStatus, StoredEvent};

pub fn wire_status(status: Status) -> Result<SessionStatus> {
    let count = |n: usize| {
        u32::try_from(n).map_err(|_| Error::Invalid("delivery count exceeds wire range".into()))
    };
    Ok(SessionStatus {
        account_key: status.account_key,
        prompt_suggestion: status.prompt_suggestion,
        jobs: status.jobs,
        usage: status.usage,
        account_usage: status.account_usage.map(|usage| *usage),
        selection_pending: status.selection_pending,
        held_reason: status.held_reason,
        queue_held: status.queue_held,
        running: status.running,
        turn_active: status.turn_active,
        stopping: status.stopping,
        queued: count(status.queued)?,
        dispatching: count(status.dispatching)?,
        uncertain: count(status.uncertain)?,
        rejected: count(status.rejected)?,
        archived: status.archived,
        model: status.model,
        effort: status.effort,
    })
}
fn sequence(value: i64) -> Result<Sequence> {
    Sequence::try_from(
        u64::try_from(value).map_err(|_| Error::Invalid("negative event sequence".into()))?,
    )
    .map_err(|error| Error::Invalid(error.into()))
}
/// Read one bounded durable page. Reading never advances server state.
pub async fn cursor_page(
    handle: &SessionHandle,
    session_id: &str,
    after: Sequence,
    limit: u16,
) -> Result<EventPage> {
    if !(1..=256).contains(&limit) {
        return Err(Error::Invalid("invalid page limit".into()));
    }
    let mut records = handle
        .events_since(
            i64::try_from(after.0).map_err(|_| Error::Invalid("invalid cursor".into()))?,
            i64::from(limit) + 1,
        )
        .await?;
    let has_more = records.len() > usize::from(limit);
    records.truncate(usize::from(limit));
    let current = handle.published_status()?;
    let events = records.into_iter().map(|mut event| {
        if event.kind == "state_summary" {
            let mut payload: serde_json::Value = serde_json::from_str(&event.payload).map_err(std::io::Error::other)?;
            if let Some(suggestion) = &current.prompt_suggestion {
                payload["promptSuggestion"] = serde_json::json!(suggestion);
            }
            event.payload = payload.to_string();
        }
        serde_json::from_value(serde_json::json!({"sessionId":event.session,"seq":sequence(event.seq)?,"kind":event.kind,"payload":serde_json::from_str::<serde_json::Value>(&event.payload).map_err(std::io::Error::other)?,"createdAt":event.created_at})).map_err(|error| std::io::Error::other(error).into())
    }).collect::<Result<Vec<StoredEvent>>>()?;
    Ok(EventPage {
        session_id: session_id.into(),
        cursor: events.last().map_or(after, StoredEvent::seq),
        events,
        status: wire_status(handle.published_status()?)?,
        has_more,
    })
}

/// The list has its own global cursor; it never consumes a session's sequence.
pub async fn session_list_page(
    store: boblm_store::Store,
    after: Sequence,
    limit: u16,
) -> Result<Vec<boblm_protocol::client::SessionListEvent>> {
    let records = tokio::task::spawn_blocking(move || {
        store.reader()?.session_list_since(
            i64::try_from(after.0).map_err(|_| std::io::Error::other("invalid cursor"))?,
            i64::from(limit),
        )
    })
    .await??;
    records.into_iter().map(|e| serde_json::from_value(serde_json::json!({"sessionId":e.session,"seq":sequence(e.seq)?,"kind":e.kind,"payload":serde_json::from_str::<serde_json::Value>(&e.payload).map_err(std::io::Error::other)?,"createdAt":e.created_at})).map_err(|e|std::io::Error::other(e).into())).collect()
}
