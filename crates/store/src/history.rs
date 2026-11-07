//! Indexed history reads, separate from the durable live-event cursor.
#[path = "history.generated.rs"]
mod sql;

use crate::{Reader, Result, StoredEvent};
use serde_json::{Value, json};

pub struct HistoryPage {
    pub cursor: i64,
    pub partial_id: Option<String>,
    pub events: Vec<StoredEvent>,
    pub context: Vec<StoredEvent>,
    pub has_more: bool,
}
impl Reader {
    /// A read transaction binds every page item and its context to one event cursor.
    pub fn history_page(
        &self,
        session: &str,
        position: Option<i64>,
        newer: bool,
        limit: i64,
    ) -> Result<HistoryPage> {
        if position.is_some_and(|p| p < 0)
            || limit <= 0
            || limit > i64::from(boblm_tuning::HISTORY_PAGE_SIZE)
        {
            return Err(crate::Error::Corrupt(
                "The history cursor or page size is invalid. Reload the chat and try again.".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let cursor = sql::cursor(&tx, sql::CursorParams { session })?
            .into_iter()
            .next()
            .unwrap_or(0);
        let (mut events, mut has_more) =
            history_records(&tx, session, position, newer, cursor, limit)?;
        // Dense tool runs are grouped by the browser. Read enough context to avoid tiny pages,
        // while keeping one base batch from the previous window for scroll anchoring.
        let budget = if position.is_some() {
            boblm_tuning::HISTORY_MAX_ROWS
                .saturating_sub(boblm_tuning::HISTORY_PAGE_SIZE as usize)
                .max(limit as usize)
        } else {
            boblm_tuning::HISTORY_MAX_ROWS
        };
        while limit == i64::from(boblm_tuning::HISTORY_PAGE_SIZE)
            && has_more
            && events.len() < budget
            && events.iter().all(tool_only)
        {
            let boundary = if newer { events.last() } else { events.first() };
            let Some(boundary) = boundary else {
                break;
            };
            let count = (budget - events.len()).min(limit as usize);
            let (mut adjacent, more) = history_records(
                &tx,
                session,
                Some(boundary.seq),
                newer,
                cursor,
                count as i64,
            )?;
            has_more = more;
            if adjacent.is_empty() {
                break;
            }
            if newer {
                events.append(&mut adjacent);
            } else {
                adjacent.append(&mut events);
                events = adjacent;
            }
        }
        let mut context: Vec<StoredEvent> = sql::pending_secrets(
            &tx,
            sql::PendingSecretsParams {
                session,
                snapshot: cursor,
            },
        )?
        .into_iter()
        .map(|r| StoredEvent {
            session: r.session,
            seq: r.seq,
            kind: r.kind,
            payload: r.payload,
            created_at: r.created_at,
        })
        .collect();
        // Only lean lifecycle metadata crosses this boundary, never prompts or tool output.
        for row in sql::subagent_events(
            &tx,
            sql::SubagentEventsParams {
                session,
                snapshot: cursor,
            },
        )? {
            let payload: Value =
                serde_json::from_str(&row.payload).map_err(std::io::Error::other)?;
            let mut blocks = Vec::new();
            for block in payload["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if block["type"] == "tool_use" {
                    let name = block["name"].as_str().unwrap_or("");
                    if !matches!(
                        name.to_ascii_lowercase().as_str(),
                        "agent" | "task" | "subagentactivity"
                    ) {
                        continue;
                    }
                    let input = if name.eq_ignore_ascii_case("subagentactivity") {
                        json!({"agentPath":block["input"]["agentPath"],"agentThreadId":block["input"]["agentThreadId"],"kind":block["input"]["kind"]})
                    } else {
                        json!({"description":block["input"]["description"],"subagent_type":block["input"]["subagent_type"]})
                    };
                    blocks.push(
                        json!({"type":"tool_use","id":block["id"],"name":name,"input":input}),
                    );
                } else if block["type"] == "tool_result"
                    && let Some(id) = block["tool_use_id"].as_str()
                    && let Some(name) =
                        sql::tool_name(&tx, sql::ToolNameParams { session, id })?.pop()
                    && matches!(
                        name.to_ascii_lowercase().as_str(),
                        "agent" | "task" | "subagentactivity"
                    )
                {
                    blocks.push(json!({"type":"tool_result","tool_use_id":id,"tool_name":name,"is_error":block["is_error"].as_bool().unwrap_or(false),"content":""}));
                }
            }
            if !blocks.is_empty() {
                context.push(StoredEvent { session: row.session, seq: row.seq, kind: row.kind, created_at: row.created_at,
                    payload: json!({"type":payload["type"],"message":{"id":format!("subagent-context-{}",row.seq),"content":blocks}}).to_string() });
            }
        }
        context.sort_by_key(|event| event.seq);
        let partial_id = if let Some(raw) = sql::current_stream(
            &tx,
            sql::CurrentStreamParams {
                session,
                snapshot: cursor,
            },
        )?
        .pop()
        {
            let payload: Value = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
            if let Some(id) = payload.pointer("/event/message/id").and_then(Value::as_str) {
                if sql::completed(
                    &tx,
                    sql::CompletedParams {
                        session,
                        id,
                        snapshot: cursor,
                    },
                )?
                .is_empty()
                {
                    Some(id.to_owned())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        tx.commit()?;
        Ok(HistoryPage {
            cursor,
            partial_id,
            events,
            context,
            has_more,
        })
    }
}

/// Resolve partial text, current delivery and tool names against the same read snapshot.
fn hydrate_events(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    cursor: i64,
    events: &mut [StoredEvent],
) -> Result<()> {
    for e in events {
        let mut payload: Value = serde_json::from_str(&e.payload).map_err(std::io::Error::other)?;
        if e.kind == "message_appended" {
            if let Some(uuid) = payload["uuid"].as_str()
                && let Some(row) = sql::delivery(tx, sql::DeliveryParams { session, uuid })?.pop()
            {
                payload["delivery_state"] = row.delivery_state.into();
                if let Some(from) = row.expired_from {
                    payload["expired_from"] = from.into();
                }
            }
        } else if e.kind == "agent_event" {
            if let Some(id) = payload["compaction_id"].as_str()
                && let Some(latest) = sql::compaction_state(
                    tx,
                    sql::CompactionStateParams {
                        session,
                        id,
                        snapshot: cursor,
                    },
                )?
                .pop()
            {
                payload = serde_json::from_str(&latest).map_err(std::io::Error::other)?;
            }
            if payload["type"] == "stream_event" {
                let id = payload
                    .pointer("/event/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("partial:{}", e.seq));
                if let Some(completed) = sql::completed(
                    tx,
                    sql::CompletedParams {
                        session,
                        id: &id,
                        snapshot: cursor,
                    },
                )?
                .pop()
                {
                    payload = serde_json::from_str(&completed).map_err(std::io::Error::other)?;
                } else {
                    let text = sql::stream_text(
                        tx,
                        sql::StreamTextParams {
                            session,
                            start: e.seq,
                            snapshot: cursor,
                        },
                    )?
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                    payload = json!({"type":"assistant","history_streaming":true,"message":{"id":id,"content":[{"type":"text","text":text}]}});
                }
            }
            if let Some(blocks) = payload
                .pointer_mut("/message/content")
                .and_then(Value::as_array_mut)
            {
                for block in blocks {
                    if block["type"] == "tool_result"
                        && let Some(id) = block["tool_use_id"].as_str()
                        && let Some(row) =
                            sql::tool_name(tx, sql::ToolNameParams { session, id })?.pop()
                    {
                        block["tool_name"] = row.into();
                    }
                }
            }
        }
        e.payload = payload.to_string();
    }
    Ok(())
}
fn tool_only(event: &StoredEvent) -> bool {
    if event.kind != "agent_event" {
        return false;
    }
    let Ok(payload) = serde_json::from_str::<Value>(&event.payload) else {
        return false;
    };
    if !matches!(payload["type"].as_str(), Some("assistant" | "user")) {
        return false;
    }
    payload["message"]["content"]
        .as_array()
        .is_some_and(|blocks| {
            blocks.iter().all(|block| {
                matches!(
                    block["type"].as_str(),
                    Some("tool_use" | "tool_result" | "thinking" | "redacted_thinking")
                ) || (block["type"] == "text"
                    && block["text"]
                        .as_str()
                        .is_some_and(|text| text.trim().is_empty()))
            })
        })
}

fn history_records(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    position: Option<i64>,
    newer: bool,
    cursor: i64,
    limit: i64,
) -> Result<(Vec<StoredEvent>, bool)> {
    let mut events: Vec<StoredEvent> = if newer {
        sql::after(
            tx,
            sql::AfterParams {
                session,
                position: position.unwrap_or(0),
                snapshot: cursor,
                limit: limit + 1,
            },
        )?
        .into_iter()
        .map(|r| StoredEvent {
            session: r.session,
            seq: r.seq,
            kind: r.kind,
            payload: r.payload,
            created_at: r.created_at,
        })
        .collect()
    } else {
        sql::before(
            tx,
            sql::BeforeParams {
                session,
                position: position.unwrap_or(cursor + 1),
                snapshot: cursor,
                limit: limit + 1,
            },
        )?
        .into_iter()
        .map(|r| StoredEvent {
            session: r.session,
            seq: r.seq,
            kind: r.kind,
            payload: r.payload,
            created_at: r.created_at,
        })
        .collect()
    };
    let has_more = events.len() > limit as usize;
    events.truncate(limit as usize);
    if !newer {
        events.reverse();
    }
    hydrate_events(tx, session, cursor, &mut events)?;
    Ok((events, has_more))
}
