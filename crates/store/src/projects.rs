#[path = "projects.generated.rs"]
mod sql;

use crate::{Error, Reader, Result, Store};

/// A base directory chats work in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub path: String,
    pub name: String,
    pub created_at: i64,
}
macro_rules! from_row {
    ($row:ty) => {
        impl From<$row> for Project {
            fn from(r: $row) -> Self {
                Self {
                    id: r.id,
                    path: r.path,
                    name: r.name,
                    created_at: r.created_at,
                }
            }
        }
    };
}
from_row!(sql::LookupRow);
from_row!(sql::ByPathRow);
from_row!(sql::ListRow);

/// The last part of a path, which names a project when the owner gives no name.
pub fn default_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or("");
    if base.is_empty() {
        path.to_owned()
    } else {
        base.to_owned()
    }
}

/// Find the project for a path, creating it if there is none.
pub(crate) fn find_or_create(
    tx: &rusqlite::Transaction<'_>,
    path: &str,
    now: i64,
) -> Result<Project> {
    if let Some(found) = sql::by_path(tx, sql::ByPathParams { path })?.pop() {
        return Ok(found.into());
    }
    let id = uuid::Uuid::new_v4().hyphenated().to_string();
    let name = default_name(path);
    sql::create(
        tx,
        sql::CreateParams {
            id: &id,
            path,
            name: &name,
            created_at: now,
        },
    )?;
    Ok(Project {
        id,
        path: path.to_owned(),
        name,
        created_at: now,
    })
}

pub(crate) fn lookup(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<Project> {
    sql::lookup(tx, sql::LookupParams { id })?
        .pop()
        .map(Project::from)
        .ok_or(Error::ProjectMissing)
}

impl Store {
    /// Put the named projects in the given order, using the positions they already hold
    /// between them, so projects not named keep their places.
    pub fn reorder_projects(&self, ids: &[String]) -> Result<()> {
        crate::distinct(ids, "project")?;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let mut slots = Vec::with_capacity(ids.len());
        for id in ids {
            let found = sql::position_of(&tx, sql::PositionOfParams { id })?;
            slots.push(found.first().copied().ok_or(Error::ProjectMissing)?);
        }
        slots.sort_unstable();
        for (id, position) in ids.iter().zip(slots) {
            sql::set_position(&tx, sql::SetPositionParams { id, position })?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Add a project, or return the existing one for an identical retry. The same folder under
    /// another ID, or the same ID for another folder, fails.
    pub fn add_project(
        &self,
        id: &str,
        path: &str,
        name: &str,
        created_at: i64,
    ) -> Result<(Project, bool)> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(existing) = sql::lookup(&tx, sql::LookupParams { id })?.pop() {
            let existing = Project::from(existing);
            if existing.path != path || existing.name != name {
                return Err(Error::ProjectConflict);
            }
            return Ok((existing, false));
        }
        if sql::by_path(&tx, sql::ByPathParams { path })?
            .pop()
            .is_some()
        {
            return Err(Error::ProjectPathTaken(path.to_owned()));
        }
        sql::create(
            &tx,
            sql::CreateParams {
                id,
                path,
                name,
                created_at,
            },
        )?;
        tx.commit()?;
        Ok((
            Project {
                id: id.into(),
                path: path.into(),
                name: name.into(),
                created_at,
            },
            true,
        ))
    }

    /// Rename only the display label, preserving the project path and chat directories.
    pub fn rename_project(&self, id: &str, name: &str) -> Result<Project> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let row = sql::lookup(&tx, sql::LookupParams { id })?
            .pop()
            .ok_or(Error::ProjectMissing)?;
        sql::rename(&tx, sql::RenameParams { id, name })?;
        let mut project = Project::from(row);
        project.name = name.to_owned();
        tx.commit()?;
        Ok(project)
    }

    /// Remove a project that has no chats. Returns false if it does not exist.
    pub fn remove_project(&self, id: &str) -> Result<bool> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if sql::lookup(&tx, sql::LookupParams { id })?.is_empty() {
            return Ok(false);
        }
        let chats = sql::chat_count(&tx, sql::ChatCountParams { project_id: id })?
            .pop()
            .unwrap_or(0);
        if chats > 0 {
            return Err(Error::ProjectHasChats(chats));
        }
        sql::delete(&tx, sql::DeleteParams { id })?;
        tx.commit()?;
        Ok(true)
    }
}

impl Reader {
    pub fn projects(&self) -> Result<Vec<Project>> {
        Ok(sql::list(&self.conn)?
            .into_iter()
            .map(Project::from)
            .collect())
    }
    pub fn project(&self, id: &str) -> Result<Option<Project>> {
        Ok(sql::lookup(&self.conn, sql::LookupParams { id })?
            .pop()
            .map(Project::from))
    }
}
