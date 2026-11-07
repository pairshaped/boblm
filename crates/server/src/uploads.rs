//! Attachment upload and download (ADR 0017). Files live beside the database.
use crate::app::{ApiError, ApiResult, App};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use boblm_protocol::client::{AttachmentInfo, MAX_UPLOAD_BYTES};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

/// Uploads live beside the database, one directory per session.
pub fn uploads_dir(database: &FsPath) -> PathBuf {
    database
        .parent()
        .unwrap_or(FsPath::new("."))
        .join("uploads")
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// ASCII letters, digits, dot, dash and underscore only; never a leading dot.
pub(crate) fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim_start_matches('.');
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned.into()
    }
}

pub(crate) fn session_directory(root: &FsPath, session: &str) -> PathBuf {
    let digest = Sha256::digest(session.as_bytes());
    root.join(&format!("{digest:x}")[..32])
}

pub(crate) fn private_directory(path: &FsPath) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_owned)
}

fn info(attachment: &boblm_store::Attachment) -> Result<AttachmentInfo, ApiError> {
    Ok(AttachmentInfo {
        id: attachment.id.clone(),
        name: attachment.name.clone(),
        content_type: attachment.content_type.clone(),
        size: u64::try_from(attachment.size).map_err(ApiError::internal)?,
    })
}

pub(crate) async fn upload(
    State(app): State<Arc<App>>,
    Path((session, upload_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<AttachmentInfo> {
    let upload_id = boblm_runner::canonical_uuid(&upload_id)?;
    app.handle(&session).await?;
    let name = header_text(&headers, "x-filename")
        .and_then(|raw| {
            percent_encoding::percent_decode_str(&raw)
                .decode_utf8()
                .ok()
                .map(|n| n.trim().to_owned())
        })
        .filter(|n| !n.is_empty() && n.chars().count() <= 255 && !n.chars().any(char::is_control))
        .ok_or_else(|| ApiError::invalid("The X-Filename header must carry the file name, percent-encoded, 1 to 255 printable characters with no line breaks or control characters."))?;
    let content_type = header_text(&headers, "content-type")
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty() && t.len() <= 255 && t.is_ascii())
        .unwrap_or_else(|| "application/octet-stream".into());
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_UPLOAD_BYTES)
    {
        return Err(too_large());
    }

    let directory = session_directory(&uploads_dir(app.store.path()), &session);
    private_directory(&directory)
        .map_err(|e| ApiError::internal_while("create the folder for this chat's uploads", e))?;
    let (file, temp) = tempfile::NamedTempFile::new_in(&directory)
        .map_err(|e| ApiError::internal_while("create a temporary file for the upload", e))?
        .into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ApiError::invalid("The upload stopped before the whole file arrived (the connection closed or the request was cut off). Nothing was kept; try again."))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        size += data.len() as u64;
        if size > MAX_UPLOAD_BYTES {
            return Err(too_large());
        }
        hasher.update(&data);
        file.write_all(&data).await.map_err(|e| {
            ApiError::internal_while("write the upload to disk (is the disk full?)", e)
        })?;
    }
    file.sync_all()
        .await
        .map_err(|e| ApiError::internal_while("flush the upload to disk", e))?;
    drop(file);
    let sha256 = format!("{:x}", hasher.finalize());

    let path = directory.join(format!("{upload_id}-{}", safe_name(&name)));
    let path_text = path
        .to_str()
        .ok_or_else(|| ApiError::internal("non-UTF-8 upload path"))?
        .to_owned();
    let record = {
        let store = app.store.clone();
        let (session, id, name, content_type, sha256, path_text) = (
            session.clone(),
            upload_id.clone(),
            name.clone(),
            content_type.clone(),
            sha256.clone(),
            path_text.clone(),
        );
        tokio::task::spawn_blocking(move || {
            store.create_attachment(boblm_store::NewAttachment {
                id: &id,
                session: &session,
                name: &name,
                content_type: &content_type,
                size: i64::try_from(size).unwrap_or(i64::MAX),
                sha256: &sha256,
                path: &path_text,
                created_at: now(),
            })
        })
        .await
        .map_err(|e| ApiError::internal_while("record the upload in the database", e))?
    };
    let (attachment, created) = match record {
        Ok(record) => record,
        Err(boblm_store::Error::AttachmentConflict) => {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "upload_conflict",
                "That upload ID already belongs to a different file (other bytes, name or type). Use a new upload ID for a different file, or resend the original file unchanged.".into(),
            ));
        }
        Err(error) => return Err(ApiError::internal(error)),
    };
    // The record is written first so a crash leaves something the sweep can find, and
    // it only becomes claimable once the file is in place. An identical retry finishes
    // an upload an earlier request began, or restores a file that went missing.
    let file_present = attachment.stored
        && tokio::fs::try_exists(&attachment.path)
            .await
            .unwrap_or(false);
    if !file_present {
        if let Err(error) = temp.persist(&attachment.path) {
            if created {
                let store = app.store.clone();
                let id = upload_id.clone();
                let _ =
                    tokio::task::spawn_blocking(move || store.forget_unsent_attachment(&id)).await;
            }
            return Err(ApiError::internal_while(
                "move the finished upload into place",
                error,
            ));
        }
        let store = app.store.clone();
        let id = upload_id.clone();
        tokio::task::spawn_blocking(move || store.mark_attachment_stored(&id))
            .await
            .map_err(|e| ApiError::internal_while("mark the upload as stored", e))?
            .map_err(|e| ApiError::internal_while("mark the upload as stored", e))?;
    }
    // Otherwise the stored file is kept and the temporary copy drops here.
    Ok(Json(info(&attachment)?))
}

fn too_large() -> ApiError {
    ApiError(
        StatusCode::PAYLOAD_TOO_LARGE,
        "upload_too_large",
        format!(
            "This file is larger than the {} MiB upload limit. Nothing was kept; split it or attach a smaller file.",
            MAX_UPLOAD_BYTES >> 20
        ),
    )
}

const INLINE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

pub(crate) async fn download(
    State(app): State<Arc<App>>,
    Path((session, upload_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let upload_id = boblm_runner::canonical_uuid(&upload_id)?;
    let store = app.store.clone();
    let id = upload_id.clone();
    let attachment = tokio::task::spawn_blocking(move || store.reader()?.attachment(&id))
        .await
        .map_err(|e| ApiError::internal_while("look up the upload", e))?
        .map_err(|e| ApiError::internal_while("look up the upload", e))?
        .filter(|a| a.session == session)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("Chat {session} has no upload with ID {upload_id}. It may belong to another chat or have been removed."),
            )
        })?;
    let file = tokio::fs::File::open(&attachment.path)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ApiError(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "The upload is recorded but its file is missing from disk (it may have been deleted outside BobLM). Upload it again.".into(),
                )
            } else {
                // Permissions or a disk fault: uploading again will not help.
                ApiError::internal_while("read the upload from disk", error)
            }
        })?;
    let inline = INLINE_TYPES.contains(&attachment.content_type.as_str());
    let mut response = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(file)));
    let headers = response.headers_mut();
    let content_type = if inline {
        attachment.content_type.as_str()
    } else {
        "application/octet-stream"
    };
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).map_err(ApiError::internal)?,
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from(attachment.size.unsigned_abs()),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if !inline {
        let encoded = percent_encoding::utf8_percent_encode(
            &attachment.name,
            percent_encoding::NON_ALPHANUMERIC,
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}"))
                .map_err(ApiError::internal)?,
        );
    }
    Ok(response.into_response())
}

impl App {
    /// Remove uploads never attached to a message that are older than `age`, with their
    /// files, and partial uploads a crash left behind. Run it before serving: a recorded
    /// upload's file and a new identical upload would otherwise race on one path.
    /// Returns how many records were removed.
    pub(crate) async fn remove_deleted_uploads(&self) -> Result<(), ApiError> {
        let store = self.store.clone();
        let paths = tokio::task::spawn_blocking(move || store.reader()?.deleted_uploads())
            .await
            .map_err(ApiError::internal)??;
        for path in paths {
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(ApiError::internal_while(
                    "remove an upload from a deleted chat; retry Delete or restart the server to retry cleanup",
                    error,
                ));
            }
            let store = self.store.clone();
            tokio::task::spawn_blocking(move || store.forget_deleted_upload(&path))
                .await
                .map_err(ApiError::internal)??;
        }
        Ok(())
    }
    pub async fn sweep_orphan_uploads(&self, age: Duration) -> Result<usize, ApiError> {
        self.remove_deleted_uploads().await?;
        let cutoff = now().saturating_sub(i64::try_from(age.as_millis()).unwrap_or(i64::MAX));
        let store = self.store.clone();
        let orphans = tokio::task::spawn_blocking(move || store.orphan_attachments(cutoff))
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?;
        // File first, record second: an interrupted sweep leaves a record to retry,
        // never a file nothing points at.
        for (id, path) in &orphans {
            if let Err(error) = tokio::fs::remove_file(path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("upload sweep: cannot remove {path}: {error}");
                continue;
            }
            let store = self.store.clone();
            let id = id.clone();
            tokio::task::spawn_blocking(move || store.forget_unsent_attachment(&id))
                .await
                .map_err(ApiError::internal)?
                .map_err(ApiError::internal)?;
        }
        remove_stale_partial_uploads(&uploads_dir(self.store.path()), age).await;
        Ok(orphans.len())
    }
}

/// Streaming uploads write `.tmp*` files; one older than `age` was abandoned by a crash.
async fn remove_stale_partial_uploads(root: &FsPath, age: Duration) {
    let Ok(mut sessions) = tokio::fs::read_dir(root).await else {
        return;
    };
    while let Ok(Some(session)) = sessions.next_entry().await {
        let Ok(mut entries) = tokio::fs::read_dir(session.path()).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let partial = entry.file_name().to_string_lossy().starts_with(".tmp");
            let stale = entry
                .metadata()
                .await
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|elapsed| elapsed >= age);
            if partial && stale {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}
