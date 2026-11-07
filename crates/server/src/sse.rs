use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use boblm_protocol::client::{MAX_BODY_BYTES, Sequence};
use boblm_runner::{SessionHandle, events::cursor_page};
use futures_core::Stream;
use std::{collections::BTreeMap, io, sync::Arc};

type Sessions = Arc<BTreeMap<String, SessionHandle>>;

/// Mount behind the application's bearer-token middleware before serving.
pub fn router(sessions: BTreeMap<String, SessionHandle>) -> Router {
    Router::new()
        .route("/sessions/{session_id}/events", get(session_events))
        .with_state(Arc::new(sessions))
}

async fn session_events(
    State(sessions): State<Sessions>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, StatusCode> {
    let handle = sessions
        .get(&session_id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    let after = last_event_id(&headers)?;
    Ok(
        Sse::new(event_stream(handle, session_id, after)).keep_alive(
            KeepAlive::new()
                .interval(boblm_tuning::SSE_KEEP_ALIVE)
                .text("keep-alive"),
        ),
    )
}

pub(crate) fn last_event_id(headers: &HeaderMap) -> Result<Sequence, StatusCode> {
    let mut values = headers.get_all("last-event-id").iter();
    let Some(value) = values.next() else {
        return Ok(Sequence(0));
    };
    if values.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Sequence::try_from(value.parse::<u64>().map_err(|_| StatusCode::BAD_REQUEST)?)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

/// Pull one event at a time from SQLite. Socket backpressure suspends this stream;
/// there is no producer task or unbounded subscriber queue to fall behind.
pub fn event_stream(
    handle: SessionHandle,
    session_id: String,
    mut after: Sequence,
) -> impl Stream<Item = Result<Event, io::Error>> {
    async_stream::try_stream! {
        loop {
            let page = cursor_page(&handle, &session_id, after, 1).await.map_err(io::Error::other)?;
            if let Some(event) = page.events.into_iter().next() {
                let json = serde_json::to_string(&event).map_err(io::Error::other)?;
                if json.len() > MAX_BODY_BYTES { Err(io::Error::other("event exceeds 1 MiB; cursor retained"))?; }
                let seq = event.seq();
                yield Event::default().id(seq.0.to_string()).event("sessionEvent").data(json);
                after = seq;
            } else {
                tokio::time::sleep(boblm_tuning::SSE_POLL_INTERVAL).await;
            }
        }
    }
}

pub fn list_router(store: boblm_store::Store) -> Router {
    Router::new()
        .route("/sessions/events", get(list_events))
        .with_state(store)
}
async fn list_events(
    State(store): State<boblm_store::Store>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, StatusCode> {
    let mut after = last_event_id(&headers)?;
    let stream = async_stream::try_stream! {
        loop {
            let page=boblm_runner::events::session_list_page(store.clone(),after,1).await.map_err(io::Error::other)?;
            if let Some(event)=page.into_iter().next() {
                let json=serde_json::to_string(&event).map_err(io::Error::other)?;
                if json.len()>MAX_BODY_BYTES {Err(io::Error::other("session list event exceeds 1 MiB"))?;}
                let seq=event.seq();
                yield Event::default().id(seq.0.to_string()).event("sessionListEvent").data(json);
                after=seq;
            } else {tokio::time::sleep(boblm_tuning::SSE_POLL_INTERVAL).await;}
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(boblm_tuning::SSE_KEEP_ALIVE)
            .text("keep-alive"),
    ))
}
