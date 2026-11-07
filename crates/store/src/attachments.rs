#[path = "attachments.generated.rs"]
mod sql;

use crate::{Error, Reader, Result, Store};

/// An uploaded file. `message_uuid` is set once a sent message owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub session: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub path: String,
    pub created_at: i64,
    pub message_uuid: Option<String>,
    /// The file is on disk. A record is created first so a crash leaves something the
    /// sweep can find; only a stored upload can be attached to a message.
    pub stored: bool,
}
impl From<sql::LookupRow> for Attachment {
    fn from(row: sql::LookupRow) -> Self {
        Self {
            id: row.id,
            session: row.session,
            name: row.name,
            content_type: row.content_type,
            size: row.size,
            sha256: row.sha256,
            path: row.path,
            created_at: row.created_at,
            message_uuid: row.message_uuid,
            stored: row.stored != 0,
        }
    }
}
impl From<sql::ForMessageRow> for Attachment {
    fn from(row: sql::ForMessageRow) -> Self {
        Self {
            id: row.id,
            session: row.session,
            name: row.name,
            content_type: row.content_type,
            size: row.size,
            sha256: row.sha256,
            path: row.path,
            created_at: row.created_at,
            message_uuid: row.message_uuid,
            stored: row.stored != 0,
        }
    }
}

pub struct NewAttachment<'a> {
    pub id: &'a str,
    pub session: &'a str,
    pub name: &'a str,
    pub content_type: &'a str,
    pub size: i64,
    pub sha256: &'a str,
    pub path: &'a str,
    pub created_at: i64,
}

impl Store {
    /// Record an upload. A repeat of the same ID with the same file returns the stored
    /// record and `false`; the same ID with different content fails.
    pub fn create_attachment(&self, new: NewAttachment<'_>) -> Result<(Attachment, bool)> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(existing) = sql::lookup(&tx, sql::LookupParams { id: new.id })?.pop() {
            let existing = Attachment::from(existing);
            if existing.session != new.session
                || existing.name != new.name
                || existing.content_type != new.content_type
                || existing.size != new.size
                || existing.sha256 != new.sha256
            {
                return Err(Error::AttachmentConflict);
            }
            return Ok((existing, false));
        }
        sql::create(
            &tx,
            sql::CreateParams {
                id: new.id,
                session: new.session,
                name: new.name,
                content_type: new.content_type,
                size: new.size,
                sha256: new.sha256,
                path: new.path,
                created_at: new.created_at,
            },
        )?;
        tx.commit()?;
        let stored = Attachment {
            id: new.id.into(),
            session: new.session.into(),
            name: new.name.into(),
            content_type: new.content_type.into(),
            size: new.size,
            sha256: new.sha256.into(),
            path: new.path.into(),
            created_at: new.created_at,
            message_uuid: None,
            stored: false,
        };
        Ok((stored, true))
    }

    /// Drop an upload that was recorded but whose file could not be stored.
    pub fn forget_unsent_attachment(&self, id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::delete_orphan(&tx, sql::DeleteOrphanParams { id })?;
        tx.commit()?;
        Ok(())
    }

    /// Record that an upload's file is on disk, which lets a message claim it.
    pub fn mark_attachment_stored(&self, id: &str) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::mark_stored(&tx, sql::MarkStoredParams { id })? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        tx.commit()?;
        Ok(())
    }

    /// Uploads never attached to a message and created before `cutoff`, as (id, path).
    /// The caller deletes each file first and then calls `forget_unsent_attachment`,
    /// so an interrupted sweep leaves a record to retry rather than an unindexed file.
    pub fn orphan_attachments(&self, cutoff: i64) -> Result<Vec<(String, String)>> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let orphans = sql::orphans(&tx, sql::OrphansParams { cutoff })?;
        Ok(orphans.into_iter().map(|o| (o.id, o.path)).collect())
    }
}

impl Reader {
    pub fn attachment(&self, id: &str) -> Result<Option<Attachment>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { id })?
            .pop()
            .map(Attachment::from))
    }
    /// A message's attachments in the order the owner attached them.
    pub fn message_attachments(&self, message_uuid: &str) -> Result<Vec<Attachment>> {
        Ok(
            sql::for_message(&self.conn, sql::ForMessageParams { message_uuid })?
                .into_iter()
                .map(Attachment::from)
                .collect(),
        )
    }
}

/// Look up each attachment inside the message transaction and check it is free.
pub(crate) fn claim(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    ids: &[String],
) -> Result<Vec<Attachment>> {
    let mut claimed: Vec<Attachment> = Vec::with_capacity(ids.len());
    for id in ids {
        let row = sql::lookup(tx, sql::LookupParams { id })?
            .pop()
            .map(Attachment::from)
            .filter(|row| {
                row.session == session
                    && row.stored
                    && row.message_uuid.is_none()
                    && !claimed.iter().any(|seen| seen.id == row.id)
            })
            .ok_or(Error::AttachmentUnavailable)?;
        claimed.push(row);
    }
    Ok(claimed)
}

pub(crate) fn link(
    tx: &rusqlite::Transaction<'_>,
    message_uuid: &str,
    claimed: &[Attachment],
) -> Result<()> {
    for (position, attachment) in claimed.iter().enumerate() {
        let linked = sql::link(
            tx,
            sql::LinkParams {
                id: &attachment.id,
                message_uuid: Some(message_uuid),
                position: Some(i64::try_from(position).map_err(|_| Error::AttachmentUnavailable)?),
            },
        )?;
        if linked != 1 {
            return Err(Error::AttachmentUnavailable);
        }
    }
    Ok(())
}

pub(crate) fn linked_ids(
    tx: &rusqlite::Transaction<'_>,
    message_uuid: &str,
) -> Result<Vec<String>> {
    Ok(
        sql::for_message(tx, sql::ForMessageParams { message_uuid })?
            .into_iter()
            .map(|row| row.id)
            .collect(),
    )
}
