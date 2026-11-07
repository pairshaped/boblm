#[path = "peer_spawns.generated.rs"]
mod sql;

pub struct PeerSpawn {
    pub request_id: String,
    pub caller: String,
    pub arguments: String,
    pub project_id: String,
    /// `claude` or `codex`, decided by the model.
    pub agent: String,
    pub model: String,
    pub effort: String,
    /// The chat title supplied by the caller and stored in the spawn transaction.
    pub title: String,
    pub content: String,
    pub created_at: i64,
    /// Set for a role subagent. Roots leave all three empty.
    pub parent_session_id: Option<String>,
    pub root_session_id: Option<String>,
    pub role: Option<String>,
    /// Stable routing result stored on the receipt so retries never re-resolve.
    pub resolved: Option<String>,
    /// A working folder inside the project; the project root when omitted.
    pub working_directory: Option<String>,
    /// Codex account key chosen for this chat; null keeps the inherited home.
    pub account_key: Option<String>,
}
pub struct PeerSpawnReceipt {
    pub caller: String,
    pub arguments: String,
    pub resolved: Option<String>,
}

impl crate::Reader {
    pub fn peer_spawn_receipt(&self, request_id: &str) -> crate::Result<Option<PeerSpawnReceipt>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { request_id })?
            .pop()
            .map(|row| PeerSpawnReceipt {
                caller: row.caller,
                arguments: row.arguments,
                resolved: row.resolved,
            }))
    }
}

impl crate::Store {
    /// Create the child, first queued input and idempotency receipt atomically.
    /// An identical receipt never replays input or changes the child's settings.
    pub fn spawn_peer(&self, request: PeerSpawn) -> crate::Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(previous) = sql::lookup(
            &tx,
            sql::LookupParams {
                request_id: &request.request_id,
            },
        )?
        .pop()
        {
            if previous.caller != request.caller || previous.arguments != request.arguments {
                return Err(crate::Error::SpawnConflict);
            }
            return Ok(false);
        }
        let project = crate::projects::lookup(&tx, &request.project_id)?;
        let parent =
            request
                .parent_session_id
                .as_deref()
                .map(|parent| crate::sessions::ParentLink {
                    parent_session_id: parent,
                    root_session_id: request.root_session_id.as_deref().unwrap_or(parent),
                    role: request.role.as_deref(),
                });
        if let Some(parent) = &parent {
            let caller = crate::sessions::sql::link(
                &tx,
                crate::sessions::sql::LinkParams {
                    id: &request.caller,
                },
            )?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            if caller.parent_session_id.is_some() || caller.root_session_id.is_some() {
                return Err(crate::Error::Subagent(
                    "A subagent cannot spawn another subagent. Ask the root chat to start it."
                        .into(),
                ));
            }
            if parent.parent_session_id != request.caller {
                return Err(crate::Error::Subagent(
                    "A subagent's parent must be the chat that spawned it. Ask that chat to retry the spawn."
                        .into(),
                ));
            }
            let open = crate::sessions::sql::open_subagents(
                &tx,
                crate::sessions::sql::OpenSubagentsParams {
                    root_session_id: parent.root_session_id,
                },
            )?
            .pop()
            .unwrap_or(0);
            if open >= i64::try_from(boblm_tuning::MAX_OPEN_SUBAGENTS_PER_ROOT).unwrap_or(i64::MAX)
            {
                return Err(crate::Error::Subagent(format!(
                    "This root already has {} open subagents, the most it may have. Archive one before starting another.",
                    boblm_tuning::MAX_OPEN_SUBAGENTS_PER_ROOT
                )));
            }
        }
        crate::sessions::insert_with_parent(
            &tx,
            &project,
            &crate::sessions::NewSession {
                name: &request.title,
                id: &request.request_id,
                project_id: &project.id,
                agent: &request.agent,
                model: &request.model,
                effort: Some(&request.effort),
                created_at: request.created_at,
            },
            true,
            parent,
            request.role.as_deref(),
            request.working_directory.as_deref(),
        )?;
        if let Some(account_key) = request.account_key.as_deref() {
            crate::sessions::sql::set_account_key(
                &tx,
                crate::sessions::sql::SetAccountKeyParams {
                    id: &request.request_id,
                    account_key: Some(account_key),
                },
            )?;
        }
        crate::messages::sql::append(
            &tx,
            crate::NewMessage {
                uuid: &request.request_id,
                session: &request.request_id,
                role: "user",
                content: &request.content,
                delivery_state: "queued",
                created_at: request.created_at,
            },
        )?;
        let id = &request.request_id;
        let now = request.created_at;
        crate::events::append(
            &tx,
            id,
            "message_appended",
            serde_json::json!({"uuid":id,"role":"user","content":request.content,"delivery_state":"queued","sender_session_id":request.caller}),
            now,
        )?;
        sql::receipt(
            &tx,
            sql::ReceiptParams {
                request_id: id,
                caller: &request.caller,
                arguments: &request.arguments,
                resolved: request.resolved.as_deref(),
            },
        )?;
        tx.commit()?;
        Ok(true)
    }
}
