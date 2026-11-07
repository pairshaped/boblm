#[path = "deletions.generated.rs"]
pub(crate) mod sql;
use crate::{Error, Reader, Result, Store};

impl Reader {
    pub fn session_deleted(&self, id: &str) -> Result<bool> {
        Ok(!sql::was_deleted(&self.conn, sql::WasDeletedParams { id })?.is_empty())
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
        delete_one(&tx, id)?;
        for subagent in subagents {
            delete_one(&tx, &subagent)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn forget_deleted_upload(&self, path: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        sql::upload_removed(&mut *conn, sql::UploadRemovedParams { path })?;
        Ok(())
    }
}

fn delete_one(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<()> {
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
    sql::queue_uploads(tx, sql::QueueUploadsParams { id })?;
    sql::attachments(tx, sql::AttachmentsParams { id })?;
    sql::messages(tx, sql::MessagesParams { id })?;
    sql::tools(tx, sql::ToolsParams { id })?;
    sql::events(tx, sql::EventsParams { id })?;
    sql::schedules(tx, sql::SchedulesParams { id })?;
    sql::secrets(tx, sql::SecretsParams { id })?;
    sql::spawns(tx, sql::SpawnsParams { id })?;
    sql::list_events(tx, sql::ListEventsParams { id })?;
    sql::receipt(tx, sql::ReceiptParams { id })?;
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
