//! Credential requests: a chat asks the owner for a secret, the owner answers in the browser,
//! and the value goes straight into a SOPS file (ADR 0025). The value lives only in the
//! request that carries it and the `sops` process that writes it; it is never stored, logged,
//! put in an event or sent to the chat.
use crate::{
    app::{ApiError, App},
    schedule::{derived_uuid, format_rfc3339},
};
use boblm_protocol::client::SecretReply;
use boblm_store::{NewSecretRequest, SecretClaim, SecretRequest};
use rmcp::schemars;
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{io::AsyncWriteExt, process::Command};

/// Written first to prove the key is encrypted before the real value goes in.
const CANARY: &str = "boblm-encryption-check-7f3a9c41";

/// What the agent supplies: where the value should go, never the value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SecretArgs {
    /// Fresh UUID for this request. Reuse it only to retry the identical call.
    pub request_id: String,
    /// An existing SOPS-encrypted YAML or JSON file: absolute, `~/` for the home folder, or
    /// relative to this chat's working folder.
    pub file: String,
    /// The path of object keys inside the file, for example ["db", "password"]. Missing keys
    /// are created.
    pub key: Vec<String>,
}

fn check_key(key: &[String]) -> Result<(), String> {
    if key.is_empty() || key.len() > 16 {
        return Err(
            "key must name a path of 1 to 16 object keys, for example [\"db\", \"password\"]."
                .into(),
        );
    }
    if key[0] == "sops" {
        return Err(
            "key cannot start with \"sops\": that section holds the file's own encryption data."
                .into(),
        );
    }
    if key
        .iter()
        .any(|k| k.is_empty() || k.chars().count() > 200 || k.chars().any(char::is_control))
    {
        return Err("each key must be 1 to 200 characters with no control characters.".into());
    }
    Ok(())
}

/// The key as SOPS spells a path: ["db"]["password"].
fn sops_path(key: &[String]) -> String {
    key.iter()
        .map(|k| format!("[{}]", serde_json::to_string(k).unwrap_or_default()))
        .collect()
}

/// Run `sops` with a bounded time. Its output is never inherited; on failure the first lines
/// of its error text are returned with `scrub` (the credential, if any) blanked out.
async fn sops(
    args: &[&str],
    stdin: Option<&str>,
    dir: &Path,
    scrub: &str,
) -> Result<String, String> {
    let mut command = Command::new(boblm_tuning::SOPS_BINARY);
    command
        .args(args)
        .current_dir(dir)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        format!(
            "Could not start {:?}: {e}. Install SOPS on the machine running BobLM and make sure the server's PATH finds it.",
            boblm_tuning::SOPS_BINARY
        )
    })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .await
            .map_err(|e| format!("Could not give SOPS its input: {e}."))?;
    }
    let output = tokio::time::timeout(boblm_tuning::SOPS_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "SOPS did not finish within {} seconds and was stopped. Check that it can reach its key (an age key file or a cloud key service).",
                boblm_tuning::SOPS_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("SOPS could not run: {e}."))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let mut why = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !scrub.is_empty() {
        why = why.replace(scrub, "***");
    }
    let why: String = why.chars().take(300).collect();
    Err(format!(
        "SOPS refused ({}): {}",
        output.status,
        if why.is_empty() {
            "it gave no reason"
        } else {
            &why
        }
    ))
}

/// Resolve and vet the file a credential would go into.
pub async fn destination(file: &str, cwd: &str) -> Result<PathBuf, String> {
    let typed = file.trim();
    let path = if let Some(rest) = typed.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .filter(|h| Path::new(h).is_absolute())
            .ok_or(
                "file starts with ~/ but HOME is not set for the server; give an absolute path.",
            )?;
        PathBuf::from(home).join(rest)
    } else if Path::new(typed).is_absolute() {
        PathBuf::from(typed)
    } else {
        Path::new(cwd).join(typed)
    };
    let resolved = tokio::fs::canonicalize(&path).await.map_err(|e| {
        format!(
            "{} is not an existing file on this machine ({e}). The file must already exist as a SOPS-encrypted YAML or JSON file.",
            boblm_protocol::client::shown(typed)
        )
    })?;
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    if !matches!(ext.as_deref(), Some("yaml" | "yml" | "json")) {
        return Err(format!(
            "{} is not a .yaml, .yml or .json file; credentials can only go into an existing SOPS YAML or JSON file.",
            resolved.display()
        ));
    }
    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|e| format!("Could not read {}: {e}.", resolved.display()))?;
    if !meta.is_file() || meta.len() > boblm_tuning::MAX_SOPS_FILE_BYTES {
        return Err(format!(
            "{} must be a regular file of at most {} bytes.",
            resolved.display(),
            boblm_tuning::MAX_SOPS_FILE_BYTES
        ));
    }
    let parent = resolved.parent().unwrap_or(Path::new("/"));
    let status = sops(
        &["filestatus", &resolved.to_string_lossy()],
        None,
        parent,
        "",
    )
    .await?;
    if !status.contains("\"encrypted\":true") {
        return Err(format!(
            "{} is not a SOPS-encrypted file, so BobLM will not write a credential into it.",
            resolved.display()
        ));
    }
    Ok(resolved)
}

/// Write `value` at `key` in `file` without ever exposing it. The change is made on a private
/// copy, checked to be encrypted, and moved over the original only if the original has not
/// changed meanwhile.
pub async fn write_secret(file: &Path, key: &[String], value: &str) -> Result<(), String> {
    let parent = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let original = tokio::fs::read(file)
        .await
        .map_err(|e| format!("Could not read {}: {e}.", file.display()))?;
    let before = tokio::fs::metadata(file)
        .await
        .map_err(|e| format!("Could not read {}: {e}.", file.display()))?;
    let scratch = tempfile::Builder::new()
        .prefix(".boblm-sops-")
        .tempdir_in(&parent)
        .map_err(|e| {
            format!(
                "Could not make a private working folder next to {} ({e}). The server user needs write access to that folder.",
                file.display()
            )
        })?;
    let staged = scratch.path().join(format!(
        "secret.{}",
        file.extension().and_then(|e| e.to_str()).unwrap_or("yaml")
    ));
    tokio::fs::write(&staged, &original)
        .await
        .map_err(|e| format!("Could not stage a copy of {}: {e}.", file.display()))?;
    tokio::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|e| format!("Could not protect the staged copy: {e}."))?;
    let staged_arg = staged.to_string_lossy().into_owned();
    let path = sops_path(key);
    let set = |text: &str| {
        serde_json::to_string(text).map_err(|e| format!("Could not encode the value: {e}."))
    };
    // A file's encryption rules can leave a key in plain text; prove this key is encrypted
    // before the real value is written.
    sops(
        &["set", "--value-stdin", &staged_arg, &path],
        Some(&set(CANARY)?),
        &parent,
        CANARY,
    )
    .await?;
    let probe = tokio::fs::read(&staged)
        .await
        .map_err(|e| format!("Could not check the staged copy: {e}."))?;
    if probe.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()) {
        return Err(format!(
            "SOPS would store that key in plain text (the file's encryption rules do not cover it), so nothing was written. Check the file's encrypted_regex or unencrypted settings for {}.",
            key.join(".")
        ));
    }
    sops(
        &["set", "--value-stdin", &staged_arg, &path],
        Some(&set(value)?),
        &parent,
        value,
    )
    .await?;
    let now = tokio::fs::symlink_metadata(file)
        .await
        .map_err(|e| format!("Could not recheck {}: {e}.", file.display()))?;
    let unchanged = now.is_file()
        && (now.dev(), now.ino()) == (before.dev(), before.ino())
        && tokio::fs::read(file)
            .await
            .map(|b| b == original)
            .unwrap_or(false);
    if !unchanged {
        return Err(format!(
            "{} changed while the credential was being saved, so nothing was written. Ask for the credential again.",
            file.display()
        ));
    }
    tokio::fs::set_permissions(
        &staged,
        std::fs::Permissions::from_mode(before.mode() & 0o777),
    )
    .await
    .map_err(|e| format!("Could not restore the file's permissions: {e}."))?;
    tokio::fs::rename(&staged, file)
        .await
        .map_err(|e| format!("Could not replace {}: {e}.", file.display()))?;
    Ok(())
}

fn where_text(request: &SecretRequest) -> String {
    format!("{} in {}", request.key.join("."), request.file)
}

/// What the chat is told once the owner has answered (or the time ran out).
fn outcome_text(request: &SecretRequest, status: &str, why: &str) -> String {
    let id = &request.id;
    match status {
        "saved" => format!(
            "Credential request {id}: the owner saved the value at {}. It is not shown to you; do not read it back or print it. Continue with your task.",
            where_text(request)
        ),
        "canceled" => format!(
            "Credential request {id}: the owner declined to provide the value for {}. Ask them what to do instead; do not ask for it again unprompted.",
            where_text(request)
        ),
        "expired" => format!(
            "Credential request {id}: the owner did not answer in time, so the request for {} expired and nothing was saved. Ask again only if you still need it.",
            where_text(request)
        ),
        _ => format!(
            "Credential request {id}: the value for {} could not be saved and nothing was changed. {why}",
            where_text(request)
        ),
    }
}

impl App {
    /// Tell the chat how a request ended. Best effort: the answer is already recorded, and the
    /// message ID is derived from the request, so a repeat is harmless.
    async fn tell_chat(&self, request: &SecretRequest, status: &str, why: &str) {
        let id = derived_uuid(&format!("boblm secret\0{}\0{status}", request.id));
        match self.handle(&request.session).await {
            Ok(handle) => {
                if let Err(error) = handle
                    .send_automated(&id, &outcome_text(request, status, why))
                    .await
                {
                    eprintln!(
                        "server: could not tell chat {} about credential request {}: {error}",
                        request.session, request.id
                    );
                }
            }
            Err(error) => eprintln!(
                "server: chat {} is not running to hear about credential request {}: {}",
                request.session, request.id, error.2
            ),
        }
    }

    /// A chat asks the owner for a credential. Returns at once; the owner answers from any device.
    pub async fn request_secret(
        &self,
        caller: &str,
        args: SecretArgs,
        now: i64,
    ) -> Result<serde_json::Value, String> {
        let id = boblm_runner::canonical_uuid(&args.request_id).map_err(|_| {
            format!(
                "requestId must be a fresh UUID like 123e4567-e89b-12d3-a456-426614174000; got {}.",
                boblm_protocol::client::shown(&args.request_id)
            )
        })?;
        check_key(&args.key)?;
        let store = self.store.clone();
        let who = caller.to_owned();
        let cwd = tokio::task::spawn_blocking(move || store.reader()?.sessions())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|s| s.id == who)
            .map(|s| s.working_directory)
            .ok_or("Your session is not known to this server.")?;
        let file = destination(&args.file, &cwd).await?;
        let expires_at =
            now + i64::try_from(boblm_tuning::SECRET_REQUEST_TTL.as_millis()).unwrap_or(i64::MAX);
        let (store, session, file_text, key) = (
            self.store.clone(),
            caller.to_owned(),
            file.to_string_lossy().into_owned(),
            args.key.clone(),
        );
        let created_id = id.clone();
        let (request, created) = tokio::task::spawn_blocking(move || {
            store.create_secret_request(
                NewSecretRequest {
                    id: &created_id,
                    session: &session,
                    file: &file_text,
                    key: &key,
                    expires_at,
                    created_at: now,
                },
                boblm_tuning::MAX_PENDING_SECRET_REQUESTS,
            )
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not record the credential request: {e}."))?;
        Ok(serde_json::json!({
            "requestId": request.id,
            "state": request.state,
            "file": request.file,
            "key": request.key,
            "expiresAt": format_rfc3339(request.expires_at),
            "alreadyExisted": !created,
            "next": "A masked input is now showing in this chat for the owner. Tell them it is ready and end your turn. BobLM sends you a message when they answer; you never receive the value."
        }))
    }

    /// The owner's answer: save the value, or decline.
    pub async fn submit_secret(
        &self,
        request_id: &str,
        value: Option<String>,
        now: i64,
    ) -> Result<SecretReply, ApiError> {
        let id = boblm_runner::canonical_uuid(request_id)?;
        let reply = |status: &str, message: String| SecretReply {
            status: status.into(),
            message,
        };
        let Some(value) = value else {
            let store = self.store.clone();
            let cancel_id = id.clone();
            let ended = tokio::task::spawn_blocking(move || {
                store.finish_secret_request(&cancel_id, "canceled", now)
            })
            .await
            .map_err(ApiError::internal)??;
            return Ok(match ended {
                Some(request) => {
                    self.tell_chat(&request, "canceled", "").await;
                    reply("canceled", "Declined. The chat has been told.".into())
                }
                None => reply("closed", self.closed_message(&id).await?),
            });
        };
        let store = self.store.clone();
        let claim_id = id.clone();
        let claim = tokio::task::spawn_blocking(move || store.claim_secret_request(&claim_id, now))
            .await
            .map_err(ApiError::internal)??;
        let request = match claim {
            SecretClaim::Claimed(request) => request,
            SecretClaim::Unknown => {
                return Ok(reply("closed", "There is no such credential request. It may belong to an older run of the server; ask the chat to request it again.".into()));
            }
            SecretClaim::Closed(state) => {
                if let Some(request) = self.find_request(&id).await?
                    && state == "expired"
                {
                    self.tell_chat(&request, "expired", "").await;
                }
                return Ok(reply(
                    if state == "expired" {
                        "expired"
                    } else {
                        "closed"
                    },
                    self.closed_message(&id).await?,
                ));
            }
        };
        let written = write_secret(Path::new(&request.file), &request.key, &value).await;
        drop(value);
        let (status, why) = match &written {
            Ok(()) => ("saved", String::new()),
            Err(why) => ("failed", why.clone()),
        };
        let store = self.store.clone();
        let finish_id = id.clone();
        tokio::task::spawn_blocking(move || store.finish_secret_request(&finish_id, status, now))
            .await
            .map_err(ApiError::internal)??;
        self.tell_chat(&request, status, &why).await;
        Ok(match written {
            Ok(()) => reply(
                "saved",
                format!(
                    "Saved at {}. The chat has been told; it never sees the value.",
                    where_text(&request)
                ),
            ),
            Err(why) => reply("failed", format!("{why} Nothing was changed.")),
        })
    }

    async fn find_request(&self, id: &str) -> Result<Option<SecretRequest>, ApiError> {
        let (store, id) = (self.store.clone(), id.to_owned());
        tokio::task::spawn_blocking(move || store.reader()?.secret_request(&id))
            .await
            .map_err(ApiError::internal)?
            .map_err(Into::into)
    }

    async fn closed_message(&self, id: &str) -> Result<String, ApiError> {
        Ok(match self.find_request(id).await? {
            Some(r) if r.state == "expired" => "This request expired before it was answered, so nothing was saved. Ask the chat to request it again.".into(),
            Some(r) if r.state == "saving" => "Another answer for this request is being saved right now.".into(),
            Some(r) => format!("This request was already closed ({}).", r.state),
            None => "There is no such credential request.".into(),
        })
    }

    /// Close requests the owner never answered, and saves that never finished, and tell the chats.
    pub async fn sweep_secrets(&self, now: i64) {
        let store = self.store.clone();
        let grace = i64::try_from((boblm_tuning::SOPS_TIMEOUT * 3).as_millis()).unwrap_or(i64::MAX);
        let swept =
            tokio::task::spawn_blocking(move || store.sweep_secret_requests(now, grace)).await;
        match swept {
            Ok(Ok(closed)) => {
                for request in closed {
                    let why = "The save did not finish, most likely because the server stopped while writing; the file was not changed.";
                    let status = request.state.clone();
                    self.tell_chat(&request, &status, why).await;
                }
            }
            Ok(Err(error)) => eprintln!("server: could not sweep credential requests: {error}"),
            Err(error) => eprintln!("server: could not sweep credential requests: {error}"),
        }
    }
}

/// Decrypt exactly one configured entry for a Pi child's environment. Neither stdout
/// nor stderr from a failed decryption is ever returned in an error.
pub async fn read_secret(file: &str, key: &[String], cwd: &Path) -> Result<String, String> {
    read_secret_labeled(
        file,
        key,
        cwd,
        "the Pi provider key",
        "PI_SOPS_KEY in crates/tuning",
    )
    .await
}

/// Decrypt exactly one configured entry of `file`, for direct use by the server (a child
/// environment or an outbound API call). `what` names the credential and `hint` names the
/// tunable that points at it, so a failure says which key to fix. Neither stdout nor stderr
/// from a failed decryption is ever returned in an error.
pub async fn read_secret_labeled(
    file: &str,
    key: &[String],
    cwd: &Path,
    what: &str,
    hint: &str,
) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let path_text = sops_path(key);
    let context = format!("{what} {path_text} in {file}");
    check_key(key).map_err(|why| format!("Could not load {context}: {why} Fix {hint}."))?;
    let resolved = destination(file, &cwd.to_string_lossy())
        .await
        .map_err(|why| {
            format!("Could not load {context}: {why} Fix this file and key before trying again.")
        })?;
    let mut child = Command::new(boblm_tuning::SOPS_BINARY)
        .args(["decrypt", "--extract", &path_text])
        .arg(&resolved)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true).spawn()
        .map_err(|e| format!("Could not load {context}: SOPS could not start ({e}). Check SOPS on the server's PATH."))?;
    let pipe = child.stdout.take().ok_or_else(|| {
        format!("Could not load {context}: SOPS supplied no output pipe. Restart the chat.")
    })?;
    let mut output = Vec::new();
    let operation = async {
        pipe.take(boblm_tuning::MAX_SECRET_BYTES as u64 + 1).read_to_end(&mut output).await
            .map_err(|e|format!("Could not load {context}: reading SOPS output failed ({e}). Check the encrypted file and age identity."))?;
        if output.len() > boblm_tuning::MAX_SECRET_BYTES {
            return Err(format!(
                "Could not load {context}: the extracted value exceeds {} bytes. Save a single provider API key at that path.",
                boblm_tuning::MAX_SECRET_BYTES
            ));
        }
        let status = child.wait().await.map_err(|e|format!("Could not load {context}: waiting for SOPS failed ({e}). Check the encrypted file and age identity."))?;
        if !status.success() {
            return Err(format!(
                "Could not load {context}: SOPS exited with {status}. Check the file, key path and the server's age identity, then try again."
            ));
        }
        Ok(())
    };
    let outcome = tokio::time::timeout(boblm_tuning::SOPS_TIMEOUT, operation).await;
    match outcome {
        Ok(Ok(())) => {}
        result => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(match result {
                Ok(Err(why)) => why,
                _ => format!(
                    "Could not load {context}: SOPS exceeded {} seconds and was stopped. Check the file and the server's age identity, then try again.",
                    boblm_tuning::SOPS_TIMEOUT.as_secs()
                ),
            });
        }
    }
    let text=String::from_utf8(output).map_err(|_|format!("Could not load {context}: the extracted value is not UTF-8 text. Save a single provider API key at that path."))?;
    // SOPS prints one trailing newline for a scalar; embedded whitespace is not an API key.
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty()
        || text.chars().any(char::is_whitespace)
        || text.chars().any(char::is_control)
    {
        return Err(format!(
            "Could not load {context}: the extracted value is empty or contains whitespace or control characters. Save a single provider API key at that path."
        ));
    }
    Ok(text.to_owned())
}
