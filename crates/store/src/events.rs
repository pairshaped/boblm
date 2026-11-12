#[path = "events.generated.rs"]
mod sql;

pub use sql::RecentMessagesRow as RecentMessageEvent;
pub use sql::SinceRow as StoredEvent;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
pub(crate) fn append(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    kind: &str,
    mut payload: serde_json::Value,
    created_at: i64,
) -> crate::Result<()> {
    if kind == "agent_event" {
        if payload["type"] == "system"
            && payload["subtype"] == "status"
            && payload["status"] == "compacting"
        {
            payload["subtype"] = "compact_started".into();
        }
        let compact = payload["type"] == "system"
            && matches!(
                payload["subtype"].as_str(),
                Some("compact_started" | "compact_boundary" | "compact_interrupted")
            );
        let end = payload["type"] == "result"
            || (payload["type"] == "system" && payload["subtype"] == "init");
        if compact || end {
            let previous = sql::latest_compaction(tx, sql::LatestCompactionParams { session })?
                .pop()
                .map(|raw| serde_json::from_str::<serde_json::Value>(&raw))
                .transpose()
                .map_err(std::io::Error::other)?;
            let active = previous
                .as_ref()
                .filter(|p| p["subtype"] == "compact_started");
            if compact {
                payload["compaction_id"] = active
                    .and_then(|p| p["compaction_id"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                    .into();
            } else if let Some(active) = active {
                append(
                    tx,
                    session,
                    kind,
                    serde_json::json!({"type":"system", "subtype":"compact_interrupted", "compaction_id":active["compaction_id"]}),
                    created_at,
                )?;
            }
        }
    }
    sql::append(
        tx,
        sql::AppendParams {
            session,
            kind,
            payload: &payload.to_string(),
            created_at,
        },
    )?;
    if kind == "agent_event" {
        crate::account_usage::observe(tx, session, &payload, created_at)?;
    }
    if kind == "questions_changed" {
        crate::session_list::append(tx, session, kind, &serde_json::json!({}), created_at)?;
    }
    // A root's subagent summary counts open questions and failed report deliveries. Both are
    // recorded as these events on the subagent, so they are the one place to tell the list.
    if matches!(kind, "questions_changed" | "subagent_report_failed") {
        crate::session_list::subagents_changed(tx, session, created_at)?;
    }
    Ok(())
}
impl crate::Reader {
    pub fn recent_message_events(
        &self,
        session: &str,
        limit: i64,
    ) -> crate::Result<Vec<RecentMessageEvent>> {
        if !(1..=50).contains(&limit) {
            return Err(
                rusqlite::Error::InvalidParameterName("recent message limit".into()).into(),
            );
        }
        let mut rows =
            sql::recent_messages(&self.conn, sql::RecentMessagesParams { session, limit })?;
        // Append events are immutable history. Peer inspection needs the message's
        // current receipt, not the queued state captured when it was accepted.
        for row in &mut rows {
            if row.kind != "message_appended" {
                continue;
            }
            let mut payload: serde_json::Value =
                serde_json::from_str(&row.payload).map_err(std::io::Error::other)?;
            if let Some(uuid) = payload["uuid"].as_str()
                && let Some(message) = self.message(uuid)?
                && message.session == session
            {
                payload["delivery_state"] = message.delivery_state.into();
            }
            row.payload = payload.to_string();
        }
        rows.reverse();
        Ok(rows)
    }
    /// The sequence of the most recent assistant event that contains non-empty text.
    pub fn last_assistant_text_seq(&self, session: &str) -> crate::Result<Option<i64>> {
        Ok(
            sql::last_assistant_text_seq(&self.conn, sql::LastAssistantTextSeqParams { session })?
                .pop(),
        )
    }
    /// The most recent assistant event that contains non-empty text.
    pub fn last_assistant_text(&self, session: &str) -> crate::Result<Option<String>> {
        let Some(raw) =
            sql::last_assistant_text(&self.conn, sql::LastAssistantTextParams { session })?.pop()
        else {
            return Ok(None);
        };
        let payload: serde_json::Value =
            serde_json::from_str(&raw).map_err(std::io::Error::other)?;
        let text = payload["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Ok((!text.is_empty()).then_some(text))
    }
    pub fn events_since(
        &self,
        session: &str,
        after: i64,
        limit: i64,
    ) -> crate::Result<Vec<StoredEvent>> {
        if after < 0 || limit <= 0 {
            return Err(
                rusqlite::Error::InvalidParameterName("event cursor or limit".into()).into(),
            );
        }
        Ok(sql::since(
            &self.conn,
            sql::SinceParams {
                session,
                after,
                limit,
            },
        )?)
    }
}

impl crate::Store {
    /// Persist a Claude event before publishing it to connected devices.
    pub fn record_event(
        &self,
        session: &str,
        kind: &str,
        payload: serde_json::Value,
        created_at: i64,
    ) -> crate::Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        append(&tx, session, kind, payload, created_at)?;
        tx.commit()?;
        Ok(())
    }
}

impl crate::Store {
    /// Persist several events of one kind in a single transaction, in order. A burst of
    /// streamed output then costs one commit, not one per event.
    pub fn record_events(
        &self,
        session: &str,
        kind: &str,
        payloads: Vec<serde_json::Value>,
        created_at: i64,
    ) -> crate::Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        for payload in payloads {
            append(&tx, session, kind, payload, created_at)?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl crate::Reader {
    /// Newest conversation or turn-start event. Operational polls do not reset silence.
    pub fn runner_restart_activity(&self, session: &str) -> crate::Result<Option<(String, i64)>> {
        Ok(
            sql::runner_restart_activity(&self.conn, sql::RunnerRestartActivityParams { session })?
                .pop()
                .map(|row| (row.payload, row.created_at)),
        )
    }
}
