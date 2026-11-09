#[path = "sessions.generated.rs"]
pub(crate) mod sql;

use crate::{Error, Reader, Result, Store};
pub use sql::ListRow as Session;
/// What a new chat starts with.
pub struct NewSession<'a> {
    pub name: &'a str,
    pub id: &'a str,
    pub project_id: &'a str,
    /// `claude` or `codex`.
    pub agent: &'a str,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub created_at: i64,
}

/// Counts shown on a root chat's sidebar entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubagentCounts {
    pub open: i64,
    pub questions: i64,
    pub failures: i64,
    pub report_failures: i64,
}

pub(crate) struct ParentLink<'a> {
    pub parent_session_id: &'a str,
    pub root_session_id: &'a str,
    pub role: Option<&'a str>,
}

/// Insert a chat into `project` with its creation events. The chosen model is already the
/// applied one: nothing has run yet, so there is no selection pending.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    project: &crate::Project,
    new: &NewSession<'_>,
    announce_choice: bool,
) -> Result<()> {
    insert_with_parent(tx, project, new, announce_choice, None, None, None)
}

pub(crate) fn insert_with_parent(
    tx: &rusqlite::Transaction<'_>,
    project: &crate::Project,
    new: &NewSession<'_>,
    announce_choice: bool,
    parent: Option<ParentLink<'_>>,
    role: Option<&str>,
    working_directory: Option<&str>,
) -> Result<()> {
    if !crate::deletions::sql::was_deleted(
        tx,
        crate::deletions::sql::WasDeletedParams { id: new.id },
    )?
    .is_empty()
    {
        return Err(Error::ChatDeletion("This chat was deleted. Start a new chat with a new ID instead of retrying its creation.".into()));
    }
    sql::create(
        tx,
        sql::CreateParams {
            id: new.id,
            name: new.name.trim(),
            working_directory: working_directory.unwrap_or(&project.path),
            project_id: &project.id,
            agent: new.agent,
            model: new.model,
            effort: new.effort,
            created_at: new.created_at,
        },
    )?;
    if let Some(parent) = &parent {
        sql::set_parent_link(
            tx,
            sql::SetParentLinkParams {
                id: new.id,
                parent_session_id: Some(parent.parent_session_id),
                root_session_id: Some(parent.root_session_id),
                role: parent.role,
            },
        )?;
    } else if role.is_some() {
        sql::set_role(tx, sql::SetRoleParams { id: new.id, role })?;
    }
    let created = serde_json::json!({
        "id": new.id,
        "working_directory": working_directory.unwrap_or(&project.path),
        "parent_session_id": parent.as_ref().map(|parent| parent.parent_session_id),
        "root_session_id": parent.as_ref().map(|parent| parent.root_session_id),
        "role": parent.as_ref().and_then(|parent| parent.role).or(role),
    });
    crate::events::append(
        tx,
        new.id,
        "session_created",
        created.clone(),
        new.created_at,
    )?;
    crate::session_list::append(tx, new.id, "created", &created, new.created_at)?;
    if announce_choice {
        crate::events::append(
            tx,
            new.id,
            "model_effort_changed",
            serde_json::json!({"model": new.model, "effort": new.effort}),
            new.created_at,
        )?;
    }
    Ok(())
}

impl Store {
    /// Put the named chats of a project in the given order, using the positions they already
    /// hold between them, so chats not named (one created on another device a moment ago)
    /// keep their places. A chat that is not in this project refuses the whole change.
    pub fn reorder_sessions(&self, project_id: &str, ids: &[String]) -> Result<()> {
        crate::distinct(ids, "chat")?;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let mut slots = Vec::with_capacity(ids.len());
        for id in ids {
            let found =
                sql::position_in_project(&tx, sql::PositionInProjectParams { id, project_id })?;
            slots.push(found.first().copied().ok_or_else(|| {
                Error::ReorderInvalid(format!(
                    "Chat {} is not in this project, so the chats cannot be reordered. Reload the list and try again.",
                    crate::shown(id)
                ))
            })?);
        }
        slots.sort_unstable();
        for (id, position) in ids.iter().zip(slots) {
            sql::set_position(&tx, sql::SetPositionParams { id, position })?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Create a chat inside a project, with its model and effort, in one transaction.
    pub fn create_session_in(&self, new: NewSession<'_>) -> Result<()> {
        self.create_session_on_account(new, None)
    }

    /// Create and pin a chat in one transaction before its runner can observe it.
    pub fn create_session_on_account(
        &self,
        new: NewSession<'_>,
        account_key: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let project = crate::projects::lookup(&tx, new.project_id)?;
        insert(&tx, &project, &new, true)?;
        if let Some(key) = account_key {
            sql::set_account_key(
                &tx,
                sql::SetAccountKeyParams {
                    id: new.id,
                    account_key: Some(key),
                },
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Create a Claude chat in the project for `working_directory`, adding the project if the
    /// folder is new. The production path is `create_session_in`; this serves tests and setup.
    pub fn create_session(&self, id: &str, working_directory: &str, created_at: i64) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let project = crate::projects::find_or_create(&tx, working_directory, created_at)?;
        let new = NewSession {
            name: "",
            id,
            project_id: &project.id,
            agent: "claude",
            model: "sonnet",
            effort: None,
            created_at,
        };
        insert(&tx, &project, &new, false)?;
        tx.commit()?;
        Ok(())
    }
    /// Remember which provider account this chat's child last launched with. A missing
    /// account ID clears the mapping instead of guessing.
    pub fn set_session_account(&self, id: &str, account_id: Option<&str>) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::set_account(&tx, sql::SetAccountParams { id, account_id })? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        tx.commit()?;
        Ok(())
    }

    /// Remember the agent's conversation ID. Claude reports it on every turn, so a repeat
    /// records nothing; a different ID (after `/clear`) records the one it replaced.
    pub fn set_agent_session(&self, id: &str, agent_session_id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let previous = sql::agent_session(&tx, sql::AgentSessionParams { id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if previous.as_deref() == Some(agent_session_id) {
            return Ok(());
        }
        if sql::set_agent_session(
            &tx,
            sql::SetAgentSessionParams {
                id,
                agent_session_id: Some(agent_session_id),
            },
        )? != 1
        {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        crate::events::append(
            &tx,
            id,
            "agent_session_changed",
            serde_json::json!({"agent_session_id": agent_session_id, "previous": previous}),
            crate::events::now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    }
}
impl Reader {
    pub fn sessions(&self) -> Result<Vec<Session>> {
        Ok(sql::list(&self.conn)?)
    }

    /// Immediate or root-linked subagents of this root chat.
    pub fn subagents_of_root(&self, root_session_id: &str) -> Result<Vec<String>> {
        Ok(sql::subagents(
            &self.conn,
            sql::SubagentsParams { root_session_id },
        )?)
    }

    /// Counts for one root's sidebar entry.
    pub fn subagent_summary(&self, root_session_id: &str) -> Result<SubagentCounts> {
        let row =
            sql::subagent_summary(&self.conn, sql::SubagentSummaryParams { root_session_id })?
                .pop()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        Ok(SubagentCounts {
            open: row.open.unwrap_or(0),
            questions: row.questions.unwrap_or(0),
            failures: row.failures.unwrap_or(0),
            report_failures: row.report_failures.unwrap_or(0),
        })
    }

    /// Unarchived subagents whose latest event is at or before `cutoff`.
    pub fn idle_subagents(&self, cutoff: i64) -> Result<Vec<(String, i64)>> {
        Ok(
            sql::idle_subagents(&self.conn, sql::IdleSubagentsParams { cutoff })?
                .into_iter()
                .map(|row| (row.id, row.last_activity))
                .collect(),
        )
    }

    /// Archived subagents whose archive is old enough to delete, without a pending report.
    pub fn prunable_subagents(&self, cutoff: i64) -> Result<Vec<String>> {
        Ok(sql::prunable_subagents(
            &self.conn,
            sql::PrunableSubagentsParams { cutoff },
        )?)
    }

    /// Whether a subagent still owes its parent a report.
    pub fn has_pending_subagent_report(&self, child_session: &str) -> Result<bool> {
        Ok(sql::has_pending_subagent_report(
            &self.conn,
            sql::HasPendingSubagentReportParams { child_session },
        )?
        .pop()
        .unwrap_or(0)
            != 0)
    }

    /// Unix milliseconds of the latest stored event, or the chat's creation time.
    pub fn last_activity(&self, id: &str) -> Result<i64> {
        Ok(
            sql::last_activity(&self.conn, sql::LastActivityParams { id })?
                .pop()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?,
        )
    }
}

impl Store {
    pub fn set_archived(&self, id: &str, archived: bool) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let subagents = if archived {
            sql::subagents(
                &tx,
                sql::SubagentsParams {
                    root_session_id: id,
                },
            )?
        } else {
            Vec::new()
        };
        set_one_archived(&tx, id, archived)?;
        for subagent in subagents {
            set_one_archived(&tx, &subagent, archived)?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn set_one_archived(tx: &rusqlite::Transaction<'_>, id: &str, archived: bool) -> Result<()> {
    let current =
        crate::deletions::sql::archived(tx, crate::deletions::sql::ArchivedParams { id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    if current == archived {
        return Ok(());
    }
    if sql::set_archived(tx, sql::SetArchivedParams { id, archived })? != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    archive_events(tx, id, archived)?;
    Ok(())
}

impl Store {
    /// Persist the next-turn choice. Model/effort catalog validation belongs to commands.
    pub fn set_model_effort(&self, id: &str, model: &str, effort: Option<&str>) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::set_model_effort(&tx, sql::SetModelEffortParams { id, model, effort })? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        crate::events::append(
            &tx,
            id,
            "model_effort_changed",
            serde_json::json!({"model": model, "effort": effort}),
            crate::events::now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    /// Record the model and effort a running Claude process has actually applied.
    /// A session whose stored choice differs from this has a pending selection.
    pub fn set_applied_choice(&self, id: &str, model: &str, effort: Option<&str>) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::set_applied_choice(&tx, sql::SetAppliedChoiceParams { id, model, effort })? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    pub fn set_name(&self, id: &str, name: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::set_name(&tx, sql::SetNameParams { id, name })? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        let payload = serde_json::json!({"name":name});
        let now = crate::events::now_ms();
        crate::events::append(&tx, id, "session_renamed", payload.clone(), now)?;
        crate::session_list::append(&tx, id, "renamed", &payload, now)?;
        tx.commit()?;
        Ok(())
    }
}

/// Reopening shares the new-message transaction, so failed input leaves the archive intact.
pub(crate) fn reopen(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<()> {
    if sql::reopen(tx, sql::ReopenParams { id })? == 1 {
        archive_events(tx, id, false)?;
    }
    Ok(())
}

fn archive_events(tx: &rusqlite::Transaction<'_>, id: &str, archived: bool) -> Result<()> {
    crate::events::append(
        tx,
        id,
        "archive_changed",
        serde_json::json!({"archived": archived}),
        crate::events::now_ms(),
    )?;
    crate::session_list::append(
        tx,
        id,
        "archived",
        &serde_json::json!({"archived":archived}),
        crate::events::now_ms(),
    )?;
    Ok(())
}

impl Reader {
    /// Provider thread identity for the owning runner's passive observers.
    pub fn agent_session_id(&self, id: &str) -> Result<Option<String>> {
        Ok(
            sql::agent_session(&self.conn, sql::AgentSessionParams { id })?
                .pop()
                .flatten(),
        )
    }

    /// The parent of a subagent, or None for a root chat.
    pub fn parent_session_id(&self, id: &str) -> Result<Option<String>> {
        Ok(
            sql::parent_session_id(&self.conn, sql::ParentSessionIdParams { id })?
                .pop()
                .flatten()
                .filter(|id| !id.is_empty()),
        )
    }

    /// The session that sent this chat its most recent peer message.
    pub fn last_peer_sender(&self, session: &str) -> Result<Option<String>> {
        Ok(
            sql::last_peer_sender(&self.conn, sql::LastPeerSenderParams { session })?
                .pop()
                .flatten()
                .filter(|id| !id.is_empty()),
        )
    }
}

impl Store {
    /// A clean-stop marker belongs to one replacement actor. Clear it before recovery
    /// so a later crash cannot reuse an earlier runner's clean stop.
    pub fn take_runner_quiet_stop(&self, session: &str) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let quiet = sql::runner_quiet_stop(&tx, sql::RunnerQuietStopParams { session })?
            .pop()
            .unwrap_or(false);
        sql::set_runner_quiet_stop(
            &tx,
            sql::SetRunnerQuietStopParams {
                session,
                quiet_stop: false,
            },
        )?;
        tx.commit()?;
        Ok(quiet)
    }

    /// Recheck durable input under the write lock: input accepted outside the actor
    /// during provider shutdown must prevent a quiet-stop marker too.
    pub fn set_runner_quiet_stop(&self, session: &str, quiet_stop: bool) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let outstanding = crate::messages::sql::outstanding(
            &tx,
            crate::messages::sql::OutstandingParams { session },
        )?;
        let quiet_stop = quiet_stop
            && !outstanding.iter().any(|m| {
                matches!(
                    m.delivery_state.as_str(),
                    "queued" | "dispatching" | "uncertain"
                )
            });
        sql::set_runner_quiet_stop(
            &tx,
            sql::SetRunnerQuietStopParams {
                session,
                quiet_stop,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_runner_stopped(&self, session: &str, stopped: bool) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::set_runner_stopped(&mut *conn, sql::SetRunnerStoppedParams { session, stopped })?;
        Ok(())
    }
}
impl Reader {
    pub fn runner_stopped(&self, session: &str) -> Result<bool> {
        Ok(
            sql::runner_stopped(&self.conn, sql::RunnerStoppedParams { session })?
                .pop()
                .unwrap_or(false),
        )
    }
}
