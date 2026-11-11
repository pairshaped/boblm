//! Local, versioned control transport. Losing a client never stops its runner.
use super::*;
use std::{io, path::Path, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    session: String,
    /// None reads the published status without waiting behind provider work.
    action: Option<Action>,
}
#[derive(Serialize, Deserialize)]
struct Response {
    version: u32,
    result: std::result::Result<Reply, Failure>,
}
#[derive(Serialize, Deserialize)]
struct Failure {
    kind: String,
    message: String,
}
impl Failure {
    fn from_error(error: Error) -> Self {
        let kind = match &error {
            Error::Store(boblm_store::Error::MessageConflict) => "message_conflict",
            Error::Store(boblm_store::Error::AttachmentUnavailable) => "attachment_unavailable",
            Error::Store(boblm_store::Error::AttachmentConflict) => "attachment_conflict",
            Error::Store(boblm_store::Error::ChatDeletion(_)) => "chat_deletion",
            Error::Closed => "closed",
            Error::Invalid(_) => "invalid",
            _ => "io",
        };
        Self {
            kind: kind.into(),
            message: error.to_string(),
        }
    }
    fn into_error(self) -> Error {
        match self.kind.as_str() {
            "message_conflict" => Error::Store(boblm_store::Error::MessageConflict),
            "attachment_unavailable" => Error::Store(boblm_store::Error::AttachmentUnavailable),
            "attachment_conflict" => Error::Store(boblm_store::Error::AttachmentConflict),
            "chat_deletion" => Error::Store(boblm_store::Error::ChatDeletion(self.message)),
            "closed" => Error::Closed,
            "invalid" => Error::Invalid(self.message),
            _ => Error::Io(io::Error::other(self.message)),
        }
    }
}
async fn read<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let size = stream.read_u32().await? as usize;
    if size > boblm_tuning::RUNNER_FRAME_BYTES {
        return Err(io::Error::other(
            "The runner message exceeds its size limit. Shorten the message and try again.",
        ));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
async fn write(stream: &mut UnixStream, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > boblm_tuning::RUNNER_FRAME_BYTES {
        return Err(io::Error::other(
            "The runner reply exceeds its size limit. Inspect the chat before retrying.",
        ));
    }
    stream
        .write_u32(u32::try_from(bytes.len()).map_err(io::Error::other)?)
        .await?;
    stream.write_all(&bytes).await
}
async fn request(path: &Path, session: &str, action: Option<Action>) -> Result<Reply> {
    let operation = async {
        let mut socket = UnixStream::connect(path).await.map_err(|cause| io::Error::other(format!(
            "This chat's runner could not be reached ({cause}). Check its service before retrying the same message ID."
        )))?;
        write(
            &mut socket,
            &Request {
                version: boblm_tuning::RUNNER_PROTOCOL_VERSION,
                session: session.into(),
                action,
            },
        )
        .await?;
        let response: Response = read(&mut socket).await.map_err(|cause| io::Error::other(format!(
            "The runner reply could not be read ({cause}). The outcome is uncertain; inspect the chat before retrying the same message ID."
        )))?;
        if response.version != boblm_tuning::RUNNER_PROTOCOL_VERSION {
            return Err(Error::Invalid("This runner uses a different control protocol. Let its work finish before upgrading that chat's runner.".into()));
        }
        response.result.map_err(Failure::into_error)
    };
    tokio::time::timeout(boblm_tuning::RUNNER_REQUEST_TIMEOUT, operation).await
        .map_err(|_| Error::Io(io::Error::other("The runner did not confirm the command in time. Its outcome is uncertain; inspect the chat before retrying.")))?
}
/// Read-only handshake, also used to distinguish a live runner from a stale socket.
pub async fn status(path: &Path, session: &str) -> Result<Status> {
    match request(path, session, None).await? {
        Reply::Status(status) => Ok(status),
        _ => Err(Error::Invalid(
            "The runner returned an unexpected handshake. Inspect its version before reconnecting."
                .into(),
        )),
    }
}
/// Starts a chat's runner again after it exits. Starting is idempotent: a runner
/// that is already live keeps its lock and the new launch changes nothing.
#[async_trait::async_trait]
pub trait RunnerLauncher: Send + Sync {
    async fn launch(&self, session: &str) -> io::Result<()>;
}
impl RunningSession {
    /// A web-server-side proxy. Dropping or shutting down it only detaches the client.
    /// With a launcher, the proxy restarts a runner that has gone away.
    pub async fn attach(
        store: Store,
        session: &str,
        path: std::path::PathBuf,
        launcher: Option<Arc<dyn RunnerLauncher>>,
    ) -> Result<Self> {
        let initial = status(&path, session).await?;
        Ok(Self::proxy(store, session, path, initial, launcher))
    }
    /// Keep one unavailable chat from preventing the rest of the web app from
    /// starting. This only polls for a connection; it never replays queued input.
    pub async fn attach_unavailable(
        store: Store,
        session: &str,
        path: std::path::PathBuf,
        launcher: Option<Arc<dyn RunnerLauncher>>,
    ) -> Result<Self> {
        let source = store.clone();
        let id = session.to_owned();
        let initial = tokio::task::spawn_blocking(move || {
            let reader = source.reader()?;
            let row = reader
                .sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| {
                    Error::Invalid("This chat no longer exists. Reload the chat list.".into())
                })?;
            let account_usage = reader.account_usage_for_session(&id)?;
            let messages = reader.outstanding_messages(&id)?;
            let count = |state| {
                messages
                    .iter()
                    .filter(|m| m.delivery_state == state)
                    .count()
            };
            Ok::<_, Error>(Status {
                account_key: row.account_key,
                prompt_suggestion: None,
                jobs: Some(Default::default()),
                usage: None,
                account_usage: account_usage.map(Box::new),
                stopped: reader.runner_stopped(&id)?,
                selection_pending: row.applied_model != row.model
                    || row.applied_effort != row.effort,
                held_reason: None,
                queue_held: true,
                running: false,
                turn_active: false,
                stopping: false,
                queued: count("queued"),
                dispatching: count("dispatching"),
                uncertain: count("uncertain"),
                rejected: count("rejected"),
                archived: row.archived,
                model: row.model,
                effort: row.effort,
            })
        })
        .await??;
        Ok(Self::proxy(store, session, path, initial, launcher))
    }
    fn proxy(
        store: Store,
        session: &str,
        path: std::path::PathBuf,
        initial: Status,
        launcher: Option<Arc<dyn RunnerLauncher>>,
    ) -> Self {
        let (updates, published_status) = watch::channel(initial);
        let (tx, mut commands) = mpsc::channel::<Command>(boblm_tuning::COMMAND_QUEUE_CAPACITY);
        let id = session.to_owned();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(boblm_tuning::RUNNER_POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_launch: Option<tokio::time::Instant> = None;
            loop {
                tokio::select! {
                    command = commands.recv() => {
                        let Some(command) = command else { break };
                        // Never retry a mutation automatically, including after a lost reply.
                        let result = request(&path, &id, Some(command.action)).await;
                        if let Ok(refreshed) = status(&path, &id).await { updates.send_replace(refreshed); }
                        let _ = command.reply.send(result);
                    }
                    _ = tick.tick() => {
                        match status(&path, &id).await {
                            Ok(status) => { updates.send_replace(status); }
                            Err(error) => {
                                // A runner that cannot be reached is gone or never started.
                                // A protocol mismatch means a live runner, which must not be replaced.
                                let gone = !matches!(error, Error::Invalid(_));
                                let due = last_launch.is_none_or(|at| at.elapsed() >= boblm_tuning::RUNNER_RELAUNCH_INTERVAL);
                                if let (true, true, Some(launcher)) = (gone, due, launcher.as_ref()) {
                                    last_launch = Some(tokio::time::Instant::now());
                                    if let Err(cause) = launcher.launch(&id).await {
                                        eprintln!("Session {id} runner could not be restarted: {cause}. The server will try again shortly.");
                                    }
                                }
                                // Unknown job state must prevent a deployment from assuming idle.
                                let mut unavailable = updates.borrow().clone();
                                unavailable.jobs.get_or_insert_with(Default::default).available = false;
                                updates.send_replace(unavailable);
                            }
                        }
                    }
                }
            }
            Ok(())
        });
        Self {
            handle: SessionHandle {
                tx,
                store,
                id: session.into(),
                published_status,
            },
            task: Some(task),
            remote: true,
        }
    }
    /// The owner process observes actor termination without stopping it on client disconnect.
    pub async fn wait(&mut self) -> Result<()> {
        if let Some(task) = self.task.as_mut() {
            task.await??;
        }
        self.task = None;
        Ok(())
    }
}
/// Serve only inside a private owner-only directory. Each connection handles one command.
/// Actor ownership belongs to the caller, not these disposable client connections.
pub async fn serve(listener: UnixListener, handle: SessionHandle) -> io::Result<()> {
    let mut clients = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = handle.tx.closed() => {
                while clients.join_next().await.is_some() {}
                return Ok(());
            }
            connection = listener.accept() => {
                let (mut socket, _) = connection?;
                if clients.len() >= boblm_tuning::COMMAND_QUEUE_CAPACITY { continue; }
                let handle = handle.clone();
                clients.spawn(async move {
                    let operation = async {
                        let request: Request = read(&mut socket).await?;
                        let result = if request.version != boblm_tuning::RUNNER_PROTOCOL_VERSION || request.session != handle.id {
                            Err(Error::Invalid("This runner does not match the requested chat or protocol. Reconnect to the correct runner.".into()))
                        } else if matches!(request.action, Some(Action::Shutdown)) {
                            Err(Error::Invalid("Stop the runner through its supervisor. Disconnecting the web server must leave agents running.".into()))
                        } else {
                            match request.action {
                                Some(action) => handle.call(action).await,
                                None => handle.published_status().map(Reply::Status),
                            }
                        };
                        write(&mut socket, &Response { version: boblm_tuning::RUNNER_PROTOCOL_VERSION,
                            result: result.map_err(Failure::from_error) }).await
                    };
                    // Bound abandoned clients and partial frames. A disconnected caller
                    // cannot cancel an accepted actor command or cause automatic replay.
                    let _ = tokio::time::timeout(boblm_tuning::RUNNER_REQUEST_TIMEOUT, operation).await;
                });
            }
            Some(_) = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}
