//! One `codex app-server` process and its process group, spoken to over JSON-RPC on stdio
//! (docs/live-codex-protocol.md). Its output becomes the same events Claude's driver emits.
//! A write or an accepted request is never delivery: a message is consumed when Codex announces
//! it under the ID we gave it (see `translate`).
pub mod account;
mod background;
mod translate;

use boblm_protocol::{Event, LifecycleState, Priority};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use translate::{Translator, assistant_text};

/// What the chat starts Codex with. Model and effort apply from the next turn.
pub struct CodexOptions<'a> {
    pub cwd: &'a Path,
    /// The Codex thread to continue; None starts a new conversation.
    pub resume: Option<&'a str>,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    /// The boblm peer-tools endpoint and this session's capability (ADR 0015).
    pub mcp: Option<McpEndpoint<'a>>,
    /// Serialize startup while a boblm-owned home may still be initializing.
    pub home_lock: Option<&'a Path>,
}
pub struct McpEndpoint<'a> {
    pub url: &'a str,
    pub token: &'a str,
}

/// The environment variable that carries the capability, so it never appears in arguments.
const MCP_TOKEN_ENV: &str = "BOBLM_MCP_TOKEN";

type Reply = Result<Value, String>;
type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Reply>>>>;

struct Settings {
    model: String,
    effort: Option<String>,
    cwd: PathBuf,
    mcp_url: Option<String>,
}
struct Shared {
    pid: u32,
    capability: Option<String>,
    thread: Mutex<Option<String>>,
    turn: Mutex<Option<String>>,
    usage_gate: Mutex<Option<Arc<dyn Fn() -> bool + Send + Sync>>>,
    settings: Mutex<Settings>,
    /// The /compact command waiting for Codex to announce the compaction itself.
    compacting: Mutex<Option<String>>,
}

impl Shared {
    fn diagnostic(&self, message: &str) {
        let message = match &self.capability {
            Some(token) if !token.is_empty() => message.replace(token, "[REDACTED]"),
            _ => message.to_owned(),
        };
        let message: String = message
            .chars()
            .take(boblm_tuning::MAX_AGENT_DIAGNOSTIC_BYTES)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let thread = self.thread.lock().expect("thread lock");
        eprintln!(
            "codex pid={} thread={}: {message}",
            self.pid,
            thread.as_deref().unwrap_or("starting")
        );
    }
}

pub struct CodexSession {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    shared: Arc<Shared>,
    next_id: AtomicI64,
    background_worker: Option<JoinHandle<()>>,
    usage_enabled: Arc<AtomicBool>,
    usage_changed: Arc<tokio::sync::Notify>,
    worker: JoinHandle<io::Result<std::process::ExitStatus>>,
    process_group: Arc<AtomicI32>,
    /// Lets the session add events for outcomes Codex reports only as a reply. Weak, so the
    /// stream still ends when the process does.
    injector: mpsc::WeakSender<Event>,
    /// Events in the shape the runner already consumes.
    pub events: mpsc::Receiver<Event>,
}

impl CodexSession {
    /// Start `codex app-server` (or a stand-in) and open or resume the chat's thread.
    pub async fn spawn(options: CodexOptions<'_>) -> io::Result<Self> {
        Self::spawn_command(Command::new(boblm_tuning::CODEX_BINARY), options).await
    }

    pub async fn spawn_command(
        mut command: Command,
        options: CodexOptions<'_>,
    ) -> io::Result<Self> {
        let _home_lock = match options.home_lock {
            Some(directory) => {
                let directory = directory.to_path_buf();
                Some(
                    tokio::task::spawn_blocking(move || account::lock_home(&directory))
                        .await
                        .map_err(|error| {
                            io::Error::other(format!(
                                "The Codex home lock task failed: {error}. Reopen the chat."
                            ))
                        })??,
                )
            }
            None => None,
        };
        command
            .arg("app-server")
            .args(boblm_tuning::CODEX_EXTRA_ARGS);
        for feature in boblm_tuning::CODEX_MULTI_AGENT_FEATURES {
            command.arg("-c").arg(format!("features.{feature}=false"));
        }
        if let Some(mcp) = &options.mcp {
            command.env(MCP_TOKEN_ENV, mcp.token);
        }
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(not(unix))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process groups require Unix",
        ));
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned();
        let mut child = command
            .current_dir(options.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Could not start {program:?} in {}: {error}. Check that Codex is installed, that the server's PATH finds it, and that the chat's working directory still exists.",
                        options.cwd.display()
                    ),
                )
            })?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .ok_or_else(|| io::Error::other("Codex started but the operating system gave no process ID for it, so it cannot be supervised."))?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::other(
                "Codex started without an input pipe, so messages cannot be sent to it.",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::other("Codex started without an output pipe, so its replies cannot be read.")
        })?;
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let pending: Pending = Arc::default();
        let stderr = child.stderr.take().ok_or_else(|| {
            io::Error::other(
                "Codex started without a diagnostic pipe, so its errors cannot be read.",
            )
        })?;
        let shared = Arc::new(Shared {
            pid: group as u32,
            capability: options.mcp.as_ref().map(|m| m.token.to_owned()),
            thread: Mutex::new(None),
            turn: Mutex::new(None),
            usage_gate: Mutex::new(None),
            compacting: Mutex::new(None),
            settings: Mutex::new(Settings {
                model: options.model.to_owned(),
                effort: options.effort.map(str::to_owned),
                cwd: options.cwd.to_owned(),
                mcp_url: options.mcp.as_ref().map(|m| m.url.to_owned()),
            }),
        });
        let process_group = Arc::new(AtomicI32::new(group));
        let (tx, events) = mpsc::channel(boblm_tuning::EVENT_CHANNEL_CAPACITY);
        let injector = tx.downgrade();
        let usage_enabled = Arc::new(AtomicBool::new(false));
        let usage_changed = Arc::new(tokio::sync::Notify::new());
        let worker = tokio::spawn(read_loop(
            child,
            stdout,
            stderr,
            stdin.clone(),
            pending.clone(),
            shared.clone(),
            tx,
            process_group.clone(),
            usage_enabled.clone(),
            usage_changed.clone(),
        ));
        let session = Self {
            stdin,
            pending,
            shared,
            next_id: AtomicI64::new(0),
            background_worker: None,
            usage_enabled,
            usage_changed,
            worker,
            process_group,
            injector,
            events,
        };
        let startup_limit = startup_limit(options.resume.is_some());
        match timeout(startup_limit, session.handshake(options.resume)).await {
            Ok(Ok(())) => Ok(session),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "Codex did not finish starting within {} seconds, so it was stopped. Check that it is signed in (`codex login`) and not stuck waiting for input.",
                    startup_limit.as_secs()
                ),
            )),
        }
    }

    pub fn enable_background_reporting(&mut self) {
        if self.background_worker.is_none() {
            self.background_worker = Some(tokio::spawn(background::monitor(
                self.stdin.clone(),
                self.pending.clone(),
                self.shared.clone(),
                self.injector.clone(),
            )));
        }
    }

    pub fn enable_usage_reporting(&self) {
        self.usage_enabled.store(true, Ordering::Relaxed);
        self.usage_changed.notify_one();
    }

    /// Skip the periodic usage poll while the account row was confirmed recently.
    pub fn set_usage_poll_gate(&self, gate: Arc<dyn Fn() -> bool + Send + Sync>) {
        *self.shared.usage_gate.lock().expect("usage gate lock") = Some(gate);
    }

    async fn handshake(&self, resume: Option<&str>) -> io::Result<()> {
        self.rpc(
            "initialize",
            json!({"clientInfo": {"name": "boblm", "title": "boblm", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"experimentalApi": true}}),
        )
        .await?
        .map_err(|why| io::Error::other(format!("Codex refused to start: {why}.")))?;
        self.notify("initialized", None).await?;
        self.open_thread(resume, true).await
    }

    /// Open a new thread, or resume one. A failed resume is an error: it never silently
    /// starts a fresh conversation (ADR 0004).
    async fn open_thread(&self, resume: Option<&str>, announce: bool) -> io::Result<()> {
        let mut params = self.thread_params();
        let method = match resume {
            Some(id) => {
                params["threadId"] = json!(id);
                // The reply would otherwise carry every past turn on one protocol line; a long
                // thread exceeds the line limit, the reply is dropped and the resume never
                // completes. boblm reads history separately.
                params["excludeTurns"] = json!(true);
                "thread/resume"
            }
            None => "thread/start",
        };
        let reply = self.rpc(method, params).await?.map_err(|why| match resume {
            Some(id) => io::Error::other(format!(
                "Codex could not resume its conversation {id}: {why}. The chat's history is intact, but no new conversation was started in its place."
            )),
            None => io::Error::other(format!("Codex could not start a conversation: {why}.")),
        })?;
        let id = reply["thread"]["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("Codex answered thread start without a thread ID."))?
            .to_owned();
        *self.shared.thread.lock().expect("thread lock") = Some(id);
        if announce {
            self.emit_init().await;
        }
        Ok(())
    }

    async fn emit_init(&self) {
        // The runner records this ID to resume the conversation later.
        let thread = self.session_id();
        let _ = self
            .inject(Event::System {
                subtype: Some("init".into()),
                session_id: thread,
                status: None,
            })
            .await;
    }

    fn thread_params(&self) -> Value {
        let settings = self.shared.settings.lock().expect("settings lock");
        let mut config = json!({"features": {}});
        for feature in boblm_tuning::CODEX_MULTI_AGENT_FEATURES {
            config["features"][*feature] = json!(false);
        }
        if let Some(url) = &settings.mcp_url {
            config["mcp_servers"] =
                json!({"boblm": {"url": url, "bearer_token_env_var": MCP_TOKEN_ENV}});
        }
        json!({
            "cwd": settings.cwd,
            "approvalPolicy": "never",
            "sandbox": "danger-full-access",
            "model": settings.model,
            "developerInstructions": boblm_tuning::CODEX_DEVELOPER_INSTRUCTIONS,
            "config": config,
        })
    }

    /// The conversation ID the runner stores for resuming.
    pub fn session_id(&self) -> Option<String> {
        self.shared.thread.lock().expect("thread lock").clone()
    }

    /// Choose the model and effort for the next turns; nothing restarts.
    pub fn configure(&self, model: &str, effort: Option<&str>) {
        let mut settings = self.shared.settings.lock().expect("settings lock");
        settings.model = model.to_owned();
        settings.effort = effort.map(str::to_owned);
    }

    async fn write(&self, message: Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Codex's input pipe closed, so the message could not be written ({error}). Its process has probably exited."),
            )
        })?;
        stdin.flush().await
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> io::Result<()> {
        let mut message = json!({"method": method});
        if let Some(params) = params {
            message["params"] = params;
        }
        self.write(message).await
    }

    /// Send a request and wait for its reply: the outer error is a broken pipe or a timeout,
    /// the inner one is Codex answering with a JSON-RPC error.
    async fn rpc(&self, method: &str, params: Value) -> io::Result<Reply> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (reply, receive) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, reply);
        let sent = self
            .write(json!({"id": id, "method": method, "params": params}))
            .await;
        if let Err(error) = sent {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(error);
        }
        let limit = request_limit(method);
        match timeout(limit, receive).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("Codex exited before answering the {method} request."),
            )),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&id);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "Codex did not answer the {method} request within {} seconds.",
                        limit.as_secs()
                    ),
                ))
            }
        }
    }

    /// Put an event on the stream as if Codex had sent it. Used for outcomes the protocol
    /// reports only as a reply, not as a notification.
    async fn inject(&self, event: Event) -> io::Result<()> {
        let sender = self.injector.upgrade().ok_or_else(|| {
            io::Error::other("Codex has exited, so there is nothing to report to.")
        })?;
        sender
            .send(event)
            .await
            .map_err(|_| io::Error::other("The chat stopped listening to Codex."))
    }

    fn thread(&self) -> io::Result<String> {
        self.session_id()
            .ok_or_else(|| io::Error::other("Codex has no open conversation yet."))
    }

    /// Codex did not take this message. Say why in the chat, then reject it so the runner can
    /// retry it or hold it for the owner (ADR 0004).
    async fn refuse(&self, uuid: &str, why: &str) -> io::Result<()> {
        self.shared.diagnostic(&format!(
            "Codex refused message {uuid}: {why}. Inspect the chat before retrying."
        ));
        self.inject(assistant_text(
            &format!("refused-{uuid}"),
            &format!("Codex did not accept this message: {why}."),
        ))
        .await?;
        self.inject(Event::CommandLifecycle {
            command_uuid: uuid.to_owned(),
            state: LifecycleState::Refused,
        })
        .await
    }

    /// Hand a message to Codex. The reply to the request is not delivery; Codex announcing the
    /// message under `uuid` is, and arrives as an event. `/compact` and `/clear` map to Codex's
    /// own compaction and to a new conversation.
    pub async fn send_message(
        &self,
        uuid: &str,
        content: &str,
        _priority: Option<Priority>,
    ) -> io::Result<()> {
        let command = content.trim();
        if command == "/compact" || command.starts_with("/compact ") {
            return self.compact(uuid).await;
        }
        if command == "/clear" {
            return self.clear(uuid).await;
        }
        let thread = self.thread()?;
        let input = json!([{"type": "text", "text": content, "text_elements": []}]);
        // Steer a running turn, otherwise start one. The turn can end while we ask, so a steer
        // that finds nothing running becomes a new turn.
        let mut running = self.shared.turn.lock().expect("turn lock").clone();
        for attempt in 0..=boblm_tuning::CODEX_STEER_RETRIES {
            let reply = match &running {
                Some(turn) => {
                    self.rpc("turn/steer", json!({"threadId": thread, "expectedTurnId": turn, "input": input, "clientUserMessageId": uuid})).await?
                }
                None => {
                    let params = {
                        let settings = self.shared.settings.lock().expect("settings lock");
                        let mut params = json!({"threadId": thread, "input": input, "clientUserMessageId": uuid, "model": settings.model});
                        if let Some(effort) = &settings.effort {
                            params["effort"] = json!(effort);
                        }
                        params
                    };
                    self.rpc("turn/start", params).await?
                }
            };
            match reply {
                Ok(reply) => {
                    if running.is_none() {
                        *self.shared.turn.lock().expect("turn lock") =
                            reply["turn"]["id"].as_str().map(str::to_owned);
                    }
                    return Ok(());
                }
                Err(why)
                    if running.is_some()
                        && attempt < boblm_tuning::CODEX_STEER_RETRIES
                        && (why.contains("no active turn")
                            || why.starts_with("expected active turn id `")) =>
                {
                    // Only a definite refusal is safe to retry. Read the latest turn instead
                    // of extracting an ID from prose or trusting our notification cache.
                    let snapshot = match self
                        .rpc(
                            "thread/turns/list",
                            json!({
                                "threadId": thread, "sortDirection": "desc", "limit": 1,
                                "itemsView": "notLoaded"
                            }),
                        )
                        .await
                    {
                        Ok(Ok(snapshot)) => snapshot,
                        Ok(Err(error)) => {
                            return self
                                .refuse(
                                    uuid,
                                    &format!("{why}; the current turn could not be read: {error}"),
                                )
                                .await;
                        }
                        Err(error) => {
                            return self
                                .refuse(
                                    uuid,
                                    &format!("{why}; the current turn could not be read: {error}"),
                                )
                                .await;
                        }
                    };
                    let Some(turns) = snapshot["data"].as_array() else {
                        return self
                            .refuse(
                                uuid,
                                "Codex returned an invalid turn list; retry the message",
                            )
                            .await;
                    };
                    running = match turns.first() {
                        None => None,
                        Some(turn) => match turn["status"].as_str() {
                            Some("inProgress") => match turn["id"].as_str().filter(|id| !id.is_empty()) {
                                Some(id) => Some(id.to_owned()),
                                None => return self.refuse(uuid, "Codex returned a running turn without an ID; retry the message").await,
                            },
                            Some("completed" | "interrupted" | "failed") => None,
                            _ => return self.refuse(uuid, "Codex returned an unknown turn status; retry the message").await,
                        },
                    };
                    *self.shared.turn.lock().expect("turn lock") = running.clone();
                }
                Err(why) => return self.refuse(uuid, &why).await,
            }
        }
        self.refuse(uuid, "the running turn kept changing under the request")
            .await
    }

    async fn compact(&self, uuid: &str) -> io::Result<()> {
        let thread = self.thread()?;
        // An accepted request is not proof anything was compacted. The command starts when
        // Codex announces the compaction item (see the read loop), so uncertainty survives
        // a Codex that dies in between.
        *self.shared.compacting.lock().expect("compacting lock") = Some(uuid.to_owned());
        if let Err(why) = self
            .rpc("thread/compact/start", json!({"threadId": thread}))
            .await?
        {
            *self.shared.compacting.lock().expect("compacting lock") = None;
            return self.refuse(uuid, &why).await;
        }
        Ok(())
    }

    /// Codex has no clear; a new thread is the same thing. The chat keeps its history and
    /// Codex starts over, under a new conversation ID the runner will store.
    async fn clear(&self, uuid: &str) -> io::Result<()> {
        if let Err(error) = self.open_thread(None, false).await {
            return self.refuse(uuid, &error.to_string()).await;
        }
        self.inject(Event::CommandLifecycle {
            command_uuid: uuid.to_owned(),
            state: LifecycleState::Started,
        })
        .await?;
        self.emit_init().await;
        self.inject(Event::Result {
            subtype: Some("success".into()),
            is_error: Some(false),
        })
        .await?;
        self.inject(Event::CommandLifecycle {
            command_uuid: uuid.to_owned(),
            state: LifecycleState::Completed,
        })
        .await
    }

    /// Ask Codex to stop the running turn. The turn ends with its own result event, which is
    /// what the runner waits for (ADR 0004).
    pub async fn request_interrupt(&self) -> io::Result<()> {
        let Some(turn) = self.shared.turn.lock().expect("turn lock").clone() else {
            return Ok(());
        };
        let thread = self.thread()?;
        self.rpc(
            "turn/interrupt",
            json!({"threadId": thread, "turnId": turn}),
        )
        .await?
        .map(|_| ())
        .map_err(|why| io::Error::other(format!("Codex refused the stop request: {why}.")))
    }

    /// Stop the process group and wait for the reader to finish.
    pub async fn stop(mut self) -> io::Result<()> {
        self.events.close();
        kill_group(&self.process_group);
        (&mut self.worker).await.map_err(io::Error::other)??;
        Ok(())
    }
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        if let Some(worker) = self.background_worker.take() {
            worker.abort();
        }
        kill_group(&self.process_group);
        self.worker.abort();
        self.pending.lock().expect("pending lock").clear();
    }
}

fn kill_group(group: &AtomicI32) {
    let group = group.swap(0, Ordering::SeqCst);
    if group == 0 {
        return;
    }
    #[cfg(unix)]
    {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(group),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

/// Newline framing over raw bytes with a size cap, as for Claude's output: a bad or oversized
/// line is skipped instead of ending the session.
struct LineReader<R> {
    reader: BufReader<R>,
    limit: usize,
    line: Vec<u8>,
    oversized: bool,
}
enum Line {
    Text(Vec<u8>),
    Oversized,
}
impl<R: AsyncRead + Unpin> LineReader<R> {
    async fn next(&mut self) -> io::Result<Option<Line>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if std::mem::take(&mut self.oversized) {
                    return Ok(Some(Line::Oversized));
                }
                return Ok(
                    (!self.line.is_empty()).then(|| Line::Text(std::mem::take(&mut self.line)))
                );
            }
            let newline = available.iter().position(|b| *b == b'\n');
            let consumed = newline.map_or(available.len(), |i| i + 1);
            if !self.oversized {
                self.line
                    .extend_from_slice(&available[..newline.unwrap_or(available.len())]);
                if self.line.len() > self.limit {
                    self.oversized = true;
                    self.line = Vec::new();
                }
            }
            self.reader.consume(consumed);
            if newline.is_some() {
                if std::mem::take(&mut self.oversized) {
                    return Ok(Some(Line::Oversized));
                }
                return Ok(Some(Line::Text(std::mem::take(&mut self.line))));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_loop(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    shared: Arc<Shared>,
    tx: mpsc::Sender<Event>,
    group: Arc<AtomicI32>,
    usage_enabled: Arc<AtomicBool>,
    usage_changed: Arc<tokio::sync::Notify>,
) -> io::Result<std::process::ExitStatus> {
    let mut lines = LineReader {
        reader: BufReader::new(stdout),
        limit: boblm_tuning::MAX_CLAUDE_LINE_BYTES,
        line: Vec::new(),
        oversized: false,
    };
    let mut diagnostics = LineReader {
        reader: BufReader::new(stderr),
        limit: boblm_tuning::MAX_AGENT_DIAGNOSTIC_BYTES,
        line: Vec::new(),
        oversized: false,
    };
    let mut stderr_open = true;
    let mut stdout_open = true;
    let mut translator = Translator::default();
    translator.set_model(&shared.settings.lock().expect("settings lock").model);
    let mut usage_tick = tokio::time::interval(boblm_tuning::USAGE_POLL_INTERVAL);
    usage_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut drain_until: Option<tokio::time::Instant> = None;
    let mut drain_limit: Option<tokio::time::Instant> = None;
    loop {
        if drain_limit.is_some_and(|limit| tokio::time::Instant::now() >= limit) {
            break;
        }
        let line = tokio::select! {
            status = child.wait(), if drain_until.is_none() => {
                match status {
                    Ok(status) => shared.diagnostic(&format!("Codex exited with {status}. Unconfirmed messages are held; inspect the chat before resuming.")),
                    Err(error) => shared.diagnostic(&format!("Codex's exit status could not be read: {error}. Check the server process.")),
                }
                kill_group(&group);
                let now = tokio::time::Instant::now();
                drain_until = Some(now + boblm_tuning::DRAIN_AFTER_EXIT);
                drain_limit = Some(now + boblm_tuning::DRAIN_LIMIT);
                continue;
            }
            line = diagnostics.next(), if stderr_open => {
                match line {
                    Ok(Some(Line::Text(bytes))) => shared.diagnostic(&format!("stderr: {}", String::from_utf8_lossy(&bytes))),
                    Ok(Some(Line::Oversized)) => shared.diagnostic("A Codex stderr line exceeded the diagnostic limit and was omitted."),
                    Ok(None) => stderr_open = false,
                    Err(error) => {
                        shared.diagnostic(&format!("Codex's diagnostic pipe could not be read: {error}. Check the server process."));
                        stderr_open = false;
                    }
                }
                continue;
            }
            _ = usage_changed.notified() => continue,
            _ = usage_tick.tick(), if usage_enabled.load(Ordering::Relaxed) && shared.thread.lock().expect("thread lock").is_some() && drain_until.is_none() => {
                let fresh = shared
                    .usage_gate
                    .lock()
                    .expect("usage gate lock")
                    .as_ref()
                    .is_some_and(|gate| gate());
                if !fresh {
                    try_usage_request(&stdin).await;
                }
                continue;
            }
            line = lines.next(), if stdout_open => line,
            _ = async { tokio::time::sleep_until(drain_until.unwrap_or_else(tokio::time::Instant::now)).await }, if drain_until.is_some() => break,
        };
        let bytes = match line {
            Ok(Some(Line::Text(bytes))) => bytes,
            Ok(Some(Line::Oversized)) => {
                shared.diagnostic("A Codex protocol line exceeded the output limit and was skipped. Its acknowledgment may be missing; inspect the chat before resuming.");
                continue;
            }
            Ok(None) => {
                if drain_until.is_none() && group.load(Ordering::SeqCst) != 0 {
                    shared.diagnostic("Codex closed its output pipe. Its process will be stopped; inspect the chat before resuming.");
                    kill_group(&group);
                }
                stdout_open = false;
                if !stderr_open && drain_until.is_some() {
                    break;
                }
                continue;
            }
            Err(error) => {
                shared.diagnostic(&format!("Codex's output pipe could not be read: {error}. Its process will be stopped; inspect the chat before resuming."));
                kill_group(&group);
                stdout_open = false;
                continue;
            }
        };
        let message = match serde_json::from_slice::<Value>(&bytes) {
            Ok(message) => message,
            Err(error) => {
                shared.diagnostic(&format!("Codex sent invalid JSON at line {}, column {}. The event was skipped; inspect the chat if delivery stalls.", error.line(), error.column()));
                continue;
            }
        };
        match (
            message.get("id"),
            message.get("method").and_then(Value::as_str),
        ) {
            (Some(id), None) => {
                // Negative IDs are reserved for optional metadata, outside the command queue.
                if id.as_i64() == Some(-1) {
                    if let Some(usage) = boblm_protocol::usage::codex_allowances(&message["result"])
                        && tx.send(usage).await.is_err()
                    {
                        break;
                    }
                    continue;
                }
                // A reply to one of our requests.
                if let Some(waiter) = id
                    .as_i64()
                    .and_then(|id| pending.lock().expect("pending lock").remove(&id))
                {
                    let reply = match message.get("error") {
                        Some(error) => Err(error["message"]
                            .as_str()
                            .unwrap_or("Codex gave no reason")
                            .to_owned()),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = waiter.send(reply);
                }
            }
            (Some(id), Some(_)) => {
                // Codex asking us something (an approval, a question for the owner). Sessions
                // never wait on the owner (ADR 0010), so decline instead of leaving it hanging.
                let refusal = json!({"id": id, "error": {"code": -32601, "message": "BobLM cannot answer this: the owner is never waited on, so say what you need in plain text and end your turn"}});
                let mut line = serde_json::to_vec(&refusal).unwrap_or_default();
                line.push(b'\n');
                let mut stdin = stdin.lock().await;
                let _ = stdin.write_all(&line).await;
                let _ = stdin.flush().await;
            }
            (None, Some(method)) => {
                let params = &message["params"];
                if params["threadId"].as_str().is_some_and(|thread| {
                    Some(thread) != shared.thread.lock().expect("thread lock").as_deref()
                }) {
                    continue;
                }
                if method == "error" {
                    let why = params["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex supplied no explanation");
                    shared.diagnostic(&format!(
                        "Codex reported an error: {why}. willRetry={}",
                        params["willRetry"]
                    ));
                } else if method == "turn/completed" && params["turn"]["status"] == "failed" {
                    let why = params["turn"]["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex supplied no explanation");
                    shared.diagnostic(&format!("Codex stopped its turn with an error: {why}. Inspect the chat before resuming."));
                }
                if let Some(uuid) = shared.compacting.lock().expect("compacting lock").take() {
                    translator.expect_compaction(uuid);
                }
                let events = translator.notification(method, &message["params"]);
                if matches!(method, "turn/started" | "turn/completed") {
                    *shared.turn.lock().expect("turn lock") = translator.turn().map(str::to_owned);
                }
                for event in events {
                    if tx.send(event).await.is_err() {
                        kill_group(&group);
                        pending.lock().expect("pending lock").clear();
                        return child.wait().await;
                    }
                }
            }
            (None, None) => {}
        }
    }
    kill_group(&group);
    pending.lock().expect("pending lock").clear();
    child.wait().await
}

/// A tiny read-only frame is atomic on the nonblocking child pipe; skip a busy pipe.
async fn try_usage_request(stdin: &tokio::sync::Mutex<ChildStdin>) {
    use std::{pin::Pin, task::Poll};
    use tokio::io::AsyncWrite;
    let line = b"{\"id\":-1,\"method\":\"account/rateLimits/read\",\"params\":{}}\n";
    let Ok(mut input) = stdin.try_lock() else {
        return;
    };
    std::future::poll_fn(|cx| {
        let _ = Pin::new(&mut *input).poll_write(cx, line);
        Poll::Ready(())
    })
    .await;
}

/// How long one request may take. Resuming a thread loads its whole rollout, so it gets
/// the resume allowance; everything else gets the ordinary request allowance.
fn request_limit(method: &str) -> Duration {
    if method == "thread/resume" {
        boblm_tuning::CODEX_RESUME_TIMEOUT
    } else {
        boblm_tuning::CODEX_REQUEST_TIMEOUT.max(Duration::from_secs(1))
    }
}

/// The whole startup budget: launch and handshake, plus the resume allowance when resuming.
fn startup_limit(resuming: bool) -> Duration {
    boblm_tuning::CODEX_STARTUP_TIMEOUT
        + if resuming {
            boblm_tuning::CODEX_RESUME_TIMEOUT
        } else {
            Duration::ZERO
        }
}

#[cfg(test)]
mod limits {
    use super::*;

    #[test]
    fn resume_gets_its_own_allowance_inside_a_larger_startup_budget() {
        assert_eq!(
            request_limit("thread/resume"),
            boblm_tuning::CODEX_RESUME_TIMEOUT
        );
        assert_eq!(
            request_limit("turn/start"),
            boblm_tuning::CODEX_REQUEST_TIMEOUT
        );
        assert!(boblm_tuning::CODEX_RESUME_TIMEOUT > boblm_tuning::CODEX_REQUEST_TIMEOUT);
        assert!(startup_limit(true) > boblm_tuning::CODEX_RESUME_TIMEOUT);
        assert_eq!(startup_limit(false), boblm_tuning::CODEX_STARTUP_TIMEOUT);
    }
}
