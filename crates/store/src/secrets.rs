#[path = "secrets.generated.rs"]
mod sql;

use crate::{Error, Result, Store, events::append};
use serde_json::json;

/// Where a chat asked the owner to save a credential. The credential is never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRequest {
    pub id: String,
    pub session: String,
    /// The SOPS file, resolved to an absolute path when the request was made.
    pub file: String,
    /// The path of object keys inside the file.
    pub key: Vec<String>,
    /// `pending`, `saving`, `saved`, `canceled`, `expired` or `failed`.
    pub state: String,
    pub expires_at: i64,
    pub created_at: i64,
}
macro_rules! from_row {
    ($row:ty) => {
        impl TryFrom<$row> for SecretRequest {
            type Error = Error;
            fn try_from(r: $row) -> Result<Self> {
                let key = serde_json::from_str(&r.key).map_err(|e| {
                    Error::Corrupt(format!(
                        "credential request {} has a damaged key path: {e}",
                        r.id
                    ))
                })?;
                Ok(Self {
                    id: r.id,
                    session: r.session,
                    file: r.file,
                    key,
                    state: r.state,
                    expires_at: r.expires_at,
                    created_at: r.created_at,
                })
            }
        }
    };
}
from_row!(sql::LookupRow);
from_row!(sql::StalePendingRow);
from_row!(sql::StaleSavingRow);

pub struct NewSecretRequest<'a> {
    pub id: &'a str,
    pub session: &'a str,
    pub file: &'a str,
    pub key: &'a [String],
    pub expires_at: i64,
    pub created_at: i64,
}

/// What claiming a request to save it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The caller now holds the request and must finish it.
    Claimed(SecretRequest),
    /// No such request.
    Unknown,
    /// Not claimable: someone else is saving it, or it already ended in this state.
    Closed(String),
}

fn resolved(
    tx: &rusqlite::Transaction<'_>,
    request: &SecretRequest,
    status: &str,
    at: i64,
) -> Result<()> {
    append(
        tx,
        &request.session,
        "secret_resolved",
        json!({"request_id": request.id, "status": status}),
        at,
    )
}

impl Store {
    /// Record a request, or return the existing one for an identical retry. A different call
    /// under the same ID, or a chat already waiting on `max_open` requests, fails.
    pub fn create_secret_request(
        &self,
        new: NewSecretRequest<'_>,
        max_open: usize,
    ) -> Result<(SecretRequest, bool)> {
        let key = serde_json::to_string(new.key).map_err(|e| Error::Corrupt(e.to_string()))?;
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        if let Some(existing) = sql::lookup(&tx, sql::LookupParams { id: new.id })?.pop() {
            let existing = SecretRequest::try_from(existing)?;
            if existing.session != new.session
                || existing.file != new.file
                || existing.key != new.key
            {
                return Err(Error::SecretConflict);
            }
            return Ok((existing, false));
        }
        let open = sql::count_open(
            &tx,
            sql::CountOpenParams {
                session: new.session,
                now: new.created_at,
            },
        )?
        .pop()
        .unwrap_or(0);
        if usize::try_from(open).unwrap_or(usize::MAX) >= max_open {
            return Err(Error::SecretLimit(max_open));
        }
        sql::create(
            &tx,
            sql::CreateParams {
                id: new.id,
                session: new.session,
                file: new.file,
                key: &key,
                expires_at: new.expires_at,
                created_at: new.created_at,
            },
        )?;
        append(
            &tx,
            new.session,
            "secret_requested",
            json!({"request_id": new.id, "file": new.file, "key": new.key, "expires_at": new.expires_at}),
            new.created_at,
        )?;
        let created = sql::lookup(&tx, sql::LookupParams { id: new.id })?
            .pop()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok((SecretRequest::try_from(created)?, true))
    }

    /// Take a pending request to save it. Only one caller can: a second finds it closed.
    /// A request found past its time is expired here and now.
    pub fn claim_secret_request(&self, id: &str, now: i64) -> Result<Claim> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let Some(row) = sql::lookup(&tx, sql::LookupParams { id })?.pop() else {
            return Ok(Claim::Unknown);
        };
        let request = SecretRequest::try_from(row)?;
        if sql::claim(&tx, sql::ClaimParams { id, now })? == 1 {
            tx.commit()?;
            return Ok(Claim::Claimed(SecretRequest {
                state: "saving".into(),
                ..request
            }));
        }
        if request.state == "pending" {
            sql::settle(
                &tx,
                sql::SettleParams {
                    id,
                    state: "expired",
                    from_state: "pending",
                },
            )?;
            resolved(&tx, &request, "expired", now)?;
            tx.commit()?;
            return Ok(Claim::Closed("expired".into()));
        }
        Ok(Claim::Closed(request.state))
    }

    /// End a request: `saved` or `failed` after a claim, or `canceled` straight from pending.
    /// Returns the request when this call ended it, None if it had already ended.
    pub fn finish_secret_request(
        &self,
        id: &str,
        status: &str,
        now: i64,
    ) -> Result<Option<SecretRequest>> {
        let from = if status == "canceled" {
            "pending"
        } else {
            "saving"
        };
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let Some(row) = sql::lookup(&tx, sql::LookupParams { id })?.pop() else {
            return Ok(None);
        };
        let request = SecretRequest::try_from(row)?;
        if sql::settle(
            &tx,
            sql::SettleParams {
                id,
                state: status,
                from_state: from,
            },
        )? != 1
        {
            return Ok(None);
        }
        resolved(&tx, &request, status, now)?;
        tx.commit()?;
        Ok(Some(SecretRequest {
            state: status.into(),
            ..request
        }))
    }

    /// Close requests nobody answered in time, and saves that never finished (the server
    /// stopped mid-write). Returns each one closed, so its chat can be told.
    pub fn sweep_secret_requests(&self, now: i64, saving_grace: i64) -> Result<Vec<SecretRequest>> {
        let mut conn = self.writer.lock().map_err(|_| Error::Poisoned)?;
        let tx = conn.transaction()?;
        let mut closed = Vec::new();
        for (rows, from, to) in [
            (
                sql::stale_pending(&tx, sql::StalePendingParams { now })?
                    .into_iter()
                    .map(SecretRequest::try_from)
                    .collect::<Result<Vec<_>>>()?,
                "pending",
                "expired",
            ),
            (
                sql::stale_saving(
                    &tx,
                    sql::StaleSavingParams {
                        cutoff: now - saving_grace,
                    },
                )?
                .into_iter()
                .map(SecretRequest::try_from)
                .collect::<Result<Vec<_>>>()?,
                "saving",
                "failed",
            ),
        ] {
            for request in rows {
                if sql::settle(
                    &tx,
                    sql::SettleParams {
                        id: &request.id,
                        state: to,
                        from_state: from,
                    },
                )? == 1
                {
                    resolved(&tx, &request, to, now)?;
                    closed.push(SecretRequest {
                        state: to.into(),
                        ..request
                    });
                }
            }
        }
        tx.commit()?;
        Ok(closed)
    }
}

impl crate::Reader {
    pub fn secret_request(&self, id: &str) -> Result<Option<SecretRequest>> {
        sql::lookup(&self.conn, sql::LookupParams { id })?
            .pop()
            .map(SecretRequest::try_from)
            .transpose()
    }
}
