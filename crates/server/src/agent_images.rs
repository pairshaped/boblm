//! Agent images (ADR 0032): an agent shows the owner an image file, and boblm keeps its own copy as
//! an attachment of an assistant-side message, so it is served, remembered and deleted like an upload.
use std::{io::Write, path::Path};

use rmcp::schemars;
use sha2::{Digest, Sha256};

use crate::{
    app::App,
    uploads::{now, private_directory, safe_name, session_directory, uploads_dir},
};

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImageArgs {
    /// Absolute path of a PNG, JPEG, GIF or WebP file on this machine, for example /tmp/screenshot.png.
    pub path: String,
    /// A short label shown above the image. Defaults to the file name.
    pub caption: Option<String>,
}

/// The content type of the image formats boblm will show, from the file's first bytes and not its name.
/// SVG is left out on purpose: it can carry script (ADR 0017).
pub(crate) fn image_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}

impl App {
    /// Copy an image into the calling chat and show it there. The caller is the chat the agent runs in.
    pub async fn show_image(
        &self,
        caller: &str,
        args: ImageArgs,
    ) -> Result<serde_json::Value, String> {
        let shown = args.path.trim();
        let path = Path::new(shown);
        if !path.is_absolute() {
            return Err(format!(
                "show_image needs the absolute path of an image file, such as /tmp/screenshot.png, but got {shown:?}. Give the full path and call it again."
            ));
        }
        let caption = args
            .caption
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty());
        if let Some(caption) = &caption
            && caption.chars().count() > boblm_tuning::MAX_IMAGE_CAPTION_CHARS
        {
            return Err(format!(
                "The caption is {} characters, and the most allowed is {}. Shorten it and call show_image again.",
                caption.chars().count(),
                boblm_tuning::MAX_IMAGE_CAPTION_CHARS
            ));
        }
        let limit = boblm_tuning::MAX_AGENT_IMAGE_BYTES;
        let meta = tokio::fs::metadata(path).await.map_err(|e| {
            format!(
                "Could not read {shown}: {e}. Check that the file exists and that you can read it, then call show_image again."
            )
        })?;
        if !meta.is_file() {
            return Err(format!(
                "{shown} is not a file. Give the path of an image file, not a folder."
            ));
        }
        if meta.len() > limit {
            return Err(format!(
                "{shown} is {} MiB, and the most BobLM will show is {} MiB. Save a smaller copy (scale it down or crop it) and call show_image again.",
                meta.len().div_ceil(1_048_576),
                limit / 1_048_576
            ));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("Could not read {shown}: {e}."))?;
        if bytes.len() as u64 > limit {
            return Err(format!(
                "{shown} grew past the {} MiB limit while it was being read. Save a finished, smaller copy and call show_image again.",
                limit / 1_048_576
            ));
        }
        let Some(content_type) = image_type(&bytes) else {
            return Err(format!(
                "{shown} is not a PNG, JPEG, GIF or WebP image (BobLM checks the file's contents, not its name), so it cannot be shown. SVG and other formats are not supported; convert it to PNG first."
            ));
        };
        let name: String = path
            .file_name()
            .map(|n| n.to_string_lossy().chars().take(255).collect())
            .filter(|n: &String| !n.is_empty() && !n.chars().any(char::is_control))
            .unwrap_or_else(|| "image".into());
        let id = uuid::Uuid::new_v4().to_string();
        let directory = session_directory(&uploads_dir(self.store.path()), caller);
        let kept = directory.join(format!("{id}-{}", safe_name(&name)));
        let kept_text = kept
            .to_str()
            .ok_or("The folder for this chat's images has a path BobLM cannot store.")?
            .to_owned();
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        let store = self.store.clone();
        let (session, caption_text, name_text) = (caller.to_owned(), caption.clone(), name.clone());
        let attachment = id.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            private_directory(&directory).map_err(|e| {
                format!("BobLM could not create the folder for this chat's images: {e}.")
            })?;
            let mut file = tempfile::NamedTempFile::new_in(&directory)
                .map_err(|e| format!("BobLM could not create a file for the image: {e}."))?;
            file.write_all(&bytes)
                .and_then(|()| file.as_file().sync_all())
                .map_err(|e| {
                    format!("BobLM could not write the image to disk (is the disk full?): {e}.")
                })?;
            let created = now();
            store
                .create_attachment(boblm_store::NewAttachment {
                    id: &attachment,
                    session: &session,
                    name: &name_text,
                    content_type,
                    size,
                    sha256: &sha256,
                    path: &kept_text,
                    created_at: created,
                })
                .map_err(|e| format!("BobLM could not record the image: {e}."))?;
            if let Err(e) = file.persist(&kept_text) {
                let _ = store.forget_unsent_attachment(&attachment);
                return Err(format!("BobLM could not put the image in place: {e}."));
            }
            store
                .mark_attachment_stored(&attachment)
                .map_err(|e| format!("BobLM could not finish recording the image: {e}."))?;
            // An assistant-side message that is already past delivery; the attachment is what it shows.
            store
                .append_with_attachments(
                    boblm_store::NewMessage {
                        uuid: &uuid::Uuid::new_v4().to_string(),
                        session: &session,
                        role: "assistant",
                        content: caption_text.as_deref().unwrap_or(""),
                        delivery_state: "started",
                        created_at: created,
                    },
                    std::slice::from_ref(&attachment),
                )
                .map_err(|e| format!("BobLM could not add the image to the chat: {e}."))?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(serde_json::json!({
            "shown": true,
            "attachmentId": id,
            "name": name,
            "contentType": content_type,
            "size": size,
        }))
    }
}
