#[path = "deletions.generated.rs"]
pub(crate) mod sql;
use crate::{Error, Reader, Result, Store};
pub use sql::AgentReceiptRow as AgentDeletionReceipt;

impl Reader {
    pub fn session_deleted(&self, id: &str) -> Result<bool> {
        Ok(!sql::was_deleted(&self.conn, sql::WasDeletedParams { id })?.is_empty())
    }
    pub fn agent_deletion_receipt(&self, request_id: &str) -> Result<Option<AgentDeletionReceipt>> {
        Ok(sql::agent_receipt(&self.conn, sql::AgentReceiptParams { request_id })?.pop())
    }
    pub fn deletion_actor(&self, id: &str) -> Result<Option<String>> {
        Ok(
            sql::deletion_actor(&self.conn, sql::DeletionActorParams { id })?
                .pop()
                .flatten(),
        )
    }
    pub fn deleted_uploads(&self) -> Result<Vec<String>> {
        Ok(sql::pending_uploads(&self.conn)?)
    }
}
impl Store {
    /// Purge archived chat content and retain only its deletion receipt and upload cleanup paths.
    /// A root chat also deletes its subagents in the same transaction.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let subagents = sql::subagents(&tx, sql::SubagentsParams { id })?;
        delete_one(&tx, id, None)?;
        for subagent in subagents {
            delete_one(&tx, &subagent, None)?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Resolve within the caller's project and check the entire deletion set. The
    /// transaction that deletes the chats repeats these checks before committing.
    pub fn prepare_agent_deletion(
        &self,
        request_id: &str,
        caller: &str,
        prefix: &str,
    ) -> Result<AgentDeletionReceipt> {
        let conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        agent_deletion(&conn, request_id, caller, prefix)
    }
    pub fn delete_session_by_agent(
        &self,
        id: &str,
        request_id: &str,
        caller: &str,
        prefix: &str,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let receipt = agent_deletion(&tx, request_id, caller, prefix)?;
        if receipt.session_id != id {
            return Err(Error::ChatDeletion("The chat prefix now selects a different chat. Inspect it and use its full ID with a fresh requestId.".into()));
        }
        if !sql::agent_receipt(&tx, sql::AgentReceiptParams { request_id })?.is_empty() {
            return Ok(());
        }
        let subagents = sql::subagents(&tx, sql::SubagentsParams { id })?;
        delete_one(&tx, id, Some(caller))?;
        for subagent in subagents {
            delete_one(&tx, &subagent, Some(caller))?;
        }
        sql::save_agent_receipt(
            &tx,
            sql::SaveAgentReceiptParams {
                request_id,
                caller,
                session_id_prefix: prefix,
                session_id: id,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn forget_deleted_upload(&self, path: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::upload_removed(&mut *conn, sql::UploadRemovedParams { path })?;
        Ok(())
    }
}

fn delete_one(tx: &rusqlite::Transaction<'_>, id: &str, actor: Option<&str>) -> Result<()> {
    if !sql::was_deleted(tx, sql::WasDeletedParams { id })?.is_empty() {
        return Ok(());
    }
    if sql::archived(tx, sql::ArchivedParams { id })?.first() != Some(&true) {
        return Err(Error::ChatDeletion(
            "Archive the chat before deleting it. Reload the list if it is already gone.".into(),
        ));
    }
    if sql::saving_credentials(tx, sql::SavingCredentialsParams { id })?
        .first()
        .copied()
        .unwrap_or(0)
        != 0
    {
        return Err(Error::ChatDeletion("This chat is saving a credential. Wait for that operation to finish before deleting it.".into()));
    }
    // `subagent_grades` is deliberately not purged: a grade outlives its chat (ADR 0048).
    sql::queue_uploads(tx, sql::QueueUploadsParams { id })?;
    sql::attachments(tx, sql::AttachmentsParams { id })?;
    sql::messages(tx, sql::MessagesParams { id })?;
    sql::tools(tx, sql::ToolsParams { id })?;
    sql::events(tx, sql::EventsParams { id })?;
    sql::schedules(tx, sql::SchedulesParams { id })?;
    sql::secrets(tx, sql::SecretsParams { id })?;
    sql::spawns(tx, sql::SpawnsParams { id })?;
    sql::list_events(tx, sql::ListEventsParams { id })?;
    sql::receipt(
        tx,
        sql::ReceiptParams {
            id,
            deleted_by_session_id: actor,
        },
    )?;
    sql::session(tx, sql::SessionParams { id })?;
    crate::session_list::append(
        tx,
        id,
        "deleted",
        &serde_json::json!({}),
        crate::events::now_ms(),
    )?;
    Ok(())
}

fn agent_deletion(
    conn: &rusqlite::Connection,
    request_id: &str,
    caller: &str,
    prefix: &str,
) -> Result<AgentDeletionReceipt> {
    if let Some(receipt) = sql::agent_receipt(conn, sql::AgentReceiptParams { request_id })?.pop() {
        if receipt.caller == caller && receipt.session_id_prefix == prefix {
            return Ok(receipt);
        }
        return Err(Error::ChatDeletion("This requestId was already used by another caller or with a different chat prefix. Use a fresh UUID for a new deletion.".into()));
    }
    let targets = sql::project_targets(conn, sql::ProjectTargetsParams { caller })?;
    let exact = targets.iter().find(|target| target.id == prefix);
    let matches = targets
        .iter()
        .filter(|target| target.id.starts_with(prefix))
        .collect::<Vec<_>>();
    let target = match (exact, matches.as_slice()) {
        (Some(target), _) => target,
        (None, [target]) => *target,
        (None, []) => return Err(Error::ChatDeletion("No matching chat is available in your project. Use inspect_peer with a chat ID from your project.".into())),
        _ => return Err(Error::ChatDeletion("More than one chat in your project matches this prefix. Use the full chat ID.".into())),
    };
    let subagents = sql::subagents(conn, sql::SubagentsParams { id: &target.id })?;
    if subagents
        .iter()
        .any(|id| !targets.iter().any(|session| &session.id == id))
    {
        return Err(Error::ChatDeletion("No matching chat is available in your project. Use inspect_peer with a chat ID from your project.".into()));
    }
    if target.id == caller || subagents.iter().any(|id| id == caller) {
        return Err(Error::ChatDeletion("You cannot delete your own chat, including through its root's cascade. Ask another chat to delete it after your work is finished.".into()));
    }
    if !target.archived {
        return Err(Error::ChatDeletion(
            "This chat is not archived. Call archive_peer before delete_chat.".into(),
        ));
    }
    for id in std::iter::once(&target.id).chain(subagents.iter()) {
        if sql::archived(conn, sql::ArchivedParams { id })?.first() != Some(&true) {
            return Err(Error::ChatDeletion("This root chat has an unarchived subagent. Finish its work and call archive_peer on it before deleting the root.".into()));
        }
        if sql::pending_questions(conn, sql::PendingQuestionsParams { id })?
            .first()
            .copied()
            .unwrap_or(0)
            != 0
        {
            return Err(Error::ChatDeletion("This chat or one of its subagents has an unanswered owner question. Wait for an answer, or retire or defer the question, before deleting the chat.".into()));
        }
    }
    Ok(AgentDeletionReceipt {
        request_id: request_id.into(),
        caller: caller.into(),
        session_id_prefix: prefix.into(),
        session_id: target.id.clone(),
    })
}
