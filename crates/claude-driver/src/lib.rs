//! One Claude process and its process group. Writes never prove delivery.
pub mod account;
mod background;
mod usage;
use boblm_protocol::{Event, LifecycleState, Priority, UserMessage};
use std::{
    collections::HashMap,
    io,
    path::Path,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::timeout,
};

enum Line {
    Text(Vec<u8>),
    Oversized,
}

/// Newline framing over raw bytes: invalid UTF-8 stays a bad line instead of a
/// read error, and a line over the cap is skipped without being buffered.
struct LineReader<R> {
    reader: BufReader<R>,
    line: Vec<u8>,
    oversized: bool,
}
impl<R: tokio::io::AsyncRead + Unpin> LineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            line: Vec::new(),
            oversized: false,
        }
    }
    /// Cancel safe: all progress is kept in `self` between awaits.
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
                if self.line.len() > boblm_tuning::MAX_CLAUDE_LINE_BYTES {
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

#[derive(Debug, PartialEq, Eq)]
pub enum SteerOutcome {
    Started,
    Rejected,
    Uncertain,
}
type Pending<T = LifecycleState> = Arc<Mutex<HashMap<String, oneshot::Sender<T>>>>;
pub type UsagePollGate = Arc<dyn Fn() -> bool + Send + Sync>;
type UsageGateSlot = Arc<std::sync::Mutex<Option<UsagePollGate>>>;

// Removal on cancellation prevents abandoned waiters accumulating.
struct Registration<T> {
    pending: Pending<T>,
    uuid: String,
}
impl<T> Drop for Registration<T> {
    fn drop(&mut self) {
        self.pending
            .lock()
            .expect("pending lock")
            .remove(&self.uuid);
    }
}

// A cancelled partial JSON write must not corrupt the next frame.
struct IncompleteWrite<'a> {
    group: &'a AtomicI32,
    complete: bool,
}
impl Drop for IncompleteWrite<'_> {
    fn drop(&mut self) {
        if !self.complete {
            kill_group(self.group);
        }
    }
}

/// A mode 0600 file in a mode 0700 directory, removed on drop.
struct PrivateFile(std::path::PathBuf);
impl PrivateFile {
    fn create(directory: &Path, contents: &[u8]) -> io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        // An existing directory keeps its mode, so refuse one others can use.
        if std::fs::metadata(directory)?.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} must be accessible only by its owner",
                    directory.display()
                ),
            ));
        }
        let path = directory.join(format!("{}.mcp.json", uuid::Uuid::new_v4()));
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let file = Self(path);
        io::Write::write_all(&mut handle, contents)?;
        Ok(file)
    }
}
impl Drop for PrivateFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct ClaudeSession {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    usage_enabled: Arc<AtomicBool>,
    usage_changed: Arc<tokio::sync::Notify>,
    usage_gate: UsageGateSlot,
    pending: Pending,
    controls: Pending<io::Result<serde_json::Value>>,
    session_id: watch::Receiver<Option<String>>,
    worker: JoinHandle<io::Result<std::process::ExitStatus>>,
    process_group: Arc<AtomicI32>,
    mcp_config: Option<PrivateFile>,
    pub events: mpsc::Receiver<Event>,
}

impl ClaudeSession {
    pub async fn spawn(cwd: &Path) -> io::Result<Self> {
        Self::resume(cwd, None).await
    }

    pub async fn resume(cwd: &Path, session_id: Option<&str>) -> io::Result<Self> {
        Self::spawn_command(Command::new(boblm_tuning::CLAUDE_BINARY), cwd, session_id).await
    }

    /// Connect directly to the hub's HTTP MCP endpoint; no stdio bridge. The
    /// capability goes in a private file under `runtime_dir`, never on the command
    /// line where `ps` and `/proc` would show it. The file is removed with the session.
    pub async fn spawn_command_with_mcp(
        mut command: Command,
        cwd: &Path,
        resume: Option<&str>,
        url: &str,
        bearer_token: &str,
        runtime_dir: &Path,
    ) -> io::Result<Self> {
        let config = serde_json::json!({"mcpServers":{"boblm":{"type":"http","url":url,"headers":{"Authorization":format!("Bearer {bearer_token}")}}}});
        let file = PrivateFile::create(runtime_dir, config.to_string().as_bytes())?;
        command.args(["--strict-mcp-config", "--mcp-config"]);
        command.arg(&file.0);
        let mut session = Self::spawn_command(command, cwd, resume).await?;
        session.mcp_config = Some(file);
        Ok(session)
    }

    /// The command may name a fake executable; Claude's normal arguments are appended.
    pub async fn spawn_command(
        mut command: Command,
        cwd: &Path,
        resume: Option<&str>,
    ) -> io::Result<Self> {
        command.args([
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--include-partial-messages",
            "--permission-prompts",
            "none",
            "--disallowedTools",
            boblm_tuning::CLAUDE_DISALLOWED_TOOLS,
            "--append-system-prompt",
            boblm_tuning::CLAUDE_SYSTEM_PROMPT_APPEND,
            "--permission-mode",
            boblm_tuning::CLAUDE_PERMISSION_MODE,
            "--model",
            boblm_tuning::CLAUDE_LAUNCH_MODEL,
        ]);
        let executable = std::env::current_exe()?;
        let quoted = format!(
            "'{}'",
            executable.to_string_lossy().replace('\'', "'\"'\"'")
        );
        let settings = serde_json::json!({
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{
                "type": "command",
                "command": format!("{quoted} {}", boblm_tuning::CLAUDE_HOOK_BACKGROUND_BASH)
            }]}]}
        });
        command.args(["--settings", &settings.to_string()]);
        command.args(boblm_tuning::CLAUDE_EXTRA_ARGS);
        if let Some(id) = resume {
            command.args(["--resume", id]);
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
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "Could not start {program:?} in {}: {error}. Check that Claude Code is installed, that the server's PATH finds it, and that the chat's working directory still exists.",
                        cwd.display()
                    ),
                )
            })?;
        let process_group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .ok_or_else(|| io::Error::other("Claude started but the operating system gave no process ID for it, so it cannot be supervised."))?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::other(
                "Claude started without an input pipe, so messages cannot be sent to it.",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::other(
                "Claude started without an output pipe, so its replies cannot be read.",
            )
        })?;
        let pending: Pending = Arc::default();
        let reader_pending = pending.clone();
        let controls: Pending<io::Result<serde_json::Value>> = Arc::default();
        let reader_controls = controls.clone();
        let (tx, events) = mpsc::channel(boblm_tuning::EVENT_CHANNEL_CAPACITY);
        let (session_tx, session_id) = watch::channel(None);
        let process_group = Arc::new(AtomicI32::new(process_group));
        let worker_group = process_group.clone();
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let usage_stdin = stdin.clone();
        let usage_enabled = Arc::new(AtomicBool::new(false));
        let usage_flag = usage_enabled.clone();
        let usage_changed = Arc::new(tokio::sync::Notify::new());
        let usage_wake = usage_changed.clone();
        let usage_gate: UsageGateSlot = Arc::default();
        let worker_gate = usage_gate.clone();
        let worker = tokio::spawn(async move {
            let mut lines = LineReader::new(stdout);
            let mut probes = usage::Probes::default();
            let mut background_tasks = background::BackgroundTasks::default();
            let mut usage_ready = false;
            let mut usage_tick = tokio::time::interval(boblm_tuning::USAGE_POLL_INTERVAL);
            usage_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut drain_until = None;
            let mut drain_limit = None;
            'read: loop {
                if drain_limit.is_some_and(|limit| tokio::time::Instant::now() >= limit) {
                    break;
                }
                // Buffered output wins over the idle drain deadline, so nothing Claude
                // wrote before exiting is dropped. Only the absolute limit above
                // can cut off a writer that never pauses.
                let line = tokio::select! {
                    biased;
                    // Noticing the exit only starts the drain; it never discards lines.
                    _ = child.wait(), if drain_until.is_none() => {
                        kill_group(&worker_group);
                        let now = tokio::time::Instant::now();
                        drain_until = Some(now + boblm_tuning::DRAIN_AFTER_EXIT);
                        drain_limit = Some(now + boblm_tuning::DRAIN_LIMIT);
                        continue;
                    }
                    _ = usage_wake.notified() => continue,
                    _ = usage_tick.tick(), if usage_ready && usage_flag.load(Ordering::Relaxed) => {
                        let fresh = worker_gate
                            .lock()
                            .expect("usage gate lock")
                            .as_ref()
                            .is_some_and(|gate| gate());
                        if !fresh {
                            probes.poll(&usage_stdin, false).await;
                            probes.poll(&usage_stdin, true).await;
                        }
                        continue;
                    }
                    line = lines.next() => line,
                    _ = async { tokio::time::sleep_until(drain_until.unwrap_or_else(tokio::time::Instant::now)).await }, if drain_until.is_some() => break,
                };
                let mut event = match line {
                    Ok(Some(Line::Text(bytes))) => {
                        match serde_json::from_slice::<serde_json::Value>(&bytes) {
                            Ok(value) => match background_tasks
                                .event(&value)
                                .or_else(|| serde_json::from_value::<Event>(value).ok())
                            {
                                Some(event) => event,
                                None => continue,
                            },
                            Err(_) => continue,
                        }
                    }
                    Ok(Some(Line::Oversized)) => Event::Other,
                    Ok(None) | Err(_) => break,
                };
                if matches!(&event, Event::System { subtype: Some(kind), .. } if kind == "init") {
                    usage_ready = true;
                }
                if let Event::ControlResponse { response } = &event {
                    if let Some(sample) = probes.response(response) {
                        event = sample;
                    }
                } else if let Event::RateLimitEvent { rate_limit_info } = &event {
                    let weekly = boblm_protocol::usage::claude_passive_weekly(rate_limit_info);
                    let five_hour =
                        boblm_protocol::usage::claude_passive_five_hour(rate_limit_info);
                    if weekly.is_some() || five_hour.is_some() {
                        event = Event::Usage {
                            context: None,
                            weekly,
                            five_hour,
                            credits: None,
                        };
                    }
                }
                if usage_flag.load(Ordering::Relaxed) && matches!(&event, Event::Result { .. }) {
                    probes.poll(&usage_stdin, false).await;
                }
                if let Event::System {
                    subtype: Some(subtype),
                    session_id: Some(id),
                    ..
                } = &event
                    && subtype == "init"
                {
                    session_tx.send_replace(Some(id.clone()));
                }
                if let Event::CommandLifecycle {
                    command_uuid,
                    state,
                } = &event
                    && matches!(
                        state,
                        LifecycleState::Started
                            | LifecycleState::Completed
                            | LifecycleState::Cancelled
                            | LifecycleState::Discarded
                            | LifecycleState::Refused
                    )
                    && let Some(waiter) = reader_pending
                        .lock()
                        .expect("pending lock")
                        .remove(command_uuid)
                {
                    let _ = waiter.send(*state);
                }
                if let Event::ControlResponse { response } = &event
                    && let Some(id) = response
                        .get("request_id")
                        .and_then(serde_json::Value::as_str)
                    && let Some(waiter) = reader_controls.lock().expect("control lock").remove(id)
                {
                    let result = if response.get("subtype").and_then(serde_json::Value::as_str)
                        == Some("success")
                    {
                        Ok(response
                            .get("response")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(io::Error::other(
                            response
                                .get("error")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("control request rejected")
                                .to_owned(),
                        ))
                    };
                    let _ = waiter.send(result);
                }
                // Backpressure preserves events. Stopping or dropping the session
                // closes the receiver, which ends this wait.
                let permit = loop {
                    tokio::select! {
                        biased;
                        permit = tx.reserve() => break permit,
                        _ = child.wait(), if drain_until.is_none() => {
                            kill_group(&worker_group);
                            let now = tokio::time::Instant::now();
                        drain_until = Some(now + boblm_tuning::DRAIN_AFTER_EXIT);
                        drain_limit = Some(now + boblm_tuning::DRAIN_LIMIT);
                        }
                    }
                };
                match permit {
                    Ok(permit) => permit.send(event),
                    Err(_) => break 'read,
                }
            }
            kill_group(&worker_group);
            reader_pending.lock().expect("pending lock").clear();
            reader_controls.lock().expect("control lock").clear();
            child.wait().await
        });
        Ok(Self {
            stdin,
            usage_enabled,
            usage_changed,
            usage_gate,
            pending,
            controls,
            session_id,
            worker,
            process_group,
            mcp_config: None,
            events,
        })
    }

    /// Enable telemetry only for an already-launched child, never as a reason to start one.
    pub fn enable_usage_reporting(&self) {
        self.usage_enabled.store(true, Ordering::Relaxed);
        self.usage_changed.notify_one();
    }

    /// Skip the periodic usage poll while the account row was confirmed recently.
    pub fn set_usage_poll_gate(&self, gate: UsagePollGate) {
        *self.usage_gate.lock().expect("usage gate lock") = Some(gate);
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.borrow().clone()
    }

    async fn write_json(&self, value: &impl serde::Serialize) -> io::Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        let mut guard = IncompleteWrite {
            group: &self.process_group,
            complete: false,
        };
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        guard.complete = true;
        Ok(())
    }

    /// Use the persisted client UUID when dispatching from Runner.
    pub async fn send_message(
        &self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
    ) -> io::Result<()> {
        self.send_message_with_origin(uuid, content, priority, false)
            .await
    }

    /// Only authenticated owner input may claim human-origin Send now behavior.
    pub async fn send_message_with_origin(
        &self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        human: bool,
    ) -> io::Result<()> {
        if self.worker.is_finished() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Claude's process has already exited, so it cannot take this message.",
            ));
        }
        let mut message = UserMessage::new(uuid, content, priority);
        message.origin = human.then_some(boblm_protocol::MessageOrigin::Human);
        self.write_json(&message).await
    }

    pub async fn send(&self, content: &str) -> io::Result<String> {
        let uuid = uuid::Uuid::new_v4().to_string();
        self.send_message(&uuid, content, None).await?;
        Ok(uuid)
    }

    pub async fn steer(&self, content: &str, deadline: Duration) -> SteerOutcome {
        self.dispatch_with_origin(
            &uuid::Uuid::new_v4().to_string(),
            content,
            Some(Priority::Now),
            deadline,
            true,
        )
        .await
    }

    /// Register before writing, so an immediate acknowledgment cannot be missed.
    pub async fn dispatch(
        &self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        deadline: Duration,
    ) -> SteerOutcome {
        self.dispatch_with_origin(uuid, content, priority, deadline, false)
            .await
    }

    pub async fn dispatch_with_origin(
        &self,
        uuid: &str,
        content: &str,
        priority: Option<Priority>,
        deadline: Duration,
        human: bool,
    ) -> SteerOutcome {
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("pending lock");
            if pending.contains_key(uuid) {
                return SteerOutcome::Uncertain;
            }
            pending.insert(uuid.to_owned(), tx);
        }
        let _registration = Registration {
            pending: self.pending.clone(),
            uuid: uuid.to_owned(),
        };
        match timeout(deadline, async {
            self.send_message_with_origin(uuid, content, priority, human)
                .await
                .ok()?;
            rx.await.ok()
        })
        .await
        {
            Ok(Some(LifecycleState::Started | LifecycleState::Completed)) => SteerOutcome::Started,
            Ok(Some(
                LifecycleState::Cancelled | LifecycleState::Discarded | LifecycleState::Refused,
            )) => SteerOutcome::Rejected,
            _ => SteerOutcome::Uncertain,
        }
    }

    /// Write an interrupt; the consumer must wait for the turn result or process exit.
    pub async fn request_interrupt(&self) -> io::Result<String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let result = self.write_json(&serde_json::json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}})).await;
        self.pending.lock().expect("pending lock").clear();
        result?;
        Ok(request_id)
    }

    /// Interrupt the running turn, awaiting its control response for up to five seconds.
    pub async fn abort(&self) -> io::Result<String> {
        self.abort_with_deadline(boblm_tuning::CONTROL_REQUEST_TIMEOUT)
            .await
    }

    pub async fn abort_with_deadline(&self, deadline: Duration) -> io::Result<String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let result = self
            .control_request(
                &request_id,
                serde_json::json!({"subtype":"interrupt"}),
                deadline,
            )
            .await;
        self.pending.lock().expect("pending lock").clear();
        result?;
        Ok(request_id)
    }

    pub async fn set_model(&self, model: Option<&str>) -> io::Result<()> {
        self.control(
            serde_json::json!({"subtype":"set_model", "model":model}),
            boblm_tuning::CONTROL_REQUEST_TIMEOUT,
        )
        .await
        .map(|_| ())
    }
    pub async fn set_effort(&self, effort: Option<&str>) -> io::Result<()> {
        self.control(
            serde_json::json!({"subtype":"apply_flag_settings", "settings":{"effortLevel":effort}}),
            boblm_tuning::CONTROL_REQUEST_TIMEOUT,
        )
        .await
        .map(|_| ())
    }
    /// Correlated control replies carry data for capability and settings checks.
    pub async fn control(
        &self,
        request: serde_json::Value,
        deadline: Duration,
    ) -> io::Result<serde_json::Value> {
        self.control_request(&uuid::Uuid::new_v4().to_string(), request, deadline)
            .await
    }
    async fn control_request(
        &self,
        request_id: &str,
        request: serde_json::Value,
        deadline: Duration,
    ) -> io::Result<serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        self.controls
            .lock()
            .expect("control lock")
            .insert(request_id.to_owned(), tx);
        let _registration = Registration {
            pending: self.controls.clone(),
            uuid: request_id.to_owned(),
        };
        let what = request
            .get("subtype")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("control")
            .to_owned();
        let outcome = timeout(deadline, async {
            self.write_json(&serde_json::json!({"type":"control_request", "request_id":request_id, "request":request})).await?;
            rx.await.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, format!("Claude exited before answering the {what} request.")))?
        }).await;
        match outcome {
            Ok(result) => result,
            Err(_) => {
                kill_group(&self.process_group);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "Claude did not answer the {what} request within {}, so its process was stopped to avoid running with a setting that may not have applied.",
                        duration_text(deadline)
                    ),
                ))
            }
        }
    }

    pub async fn stop(mut self) -> io::Result<()> {
        // Unread events must not keep the reader parked on a full channel.
        self.events.close();
        kill_group(&self.process_group);
        (&mut self.worker).await.map_err(io::Error::other)??;
        Ok(())
    }
}

/// "5 seconds", "1.5 seconds" or "300 milliseconds", never a floored "0 seconds".
fn duration_text(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{} seconds", duration.as_secs())
    } else if duration < Duration::from_secs(1) {
        format!("{} milliseconds", duration.as_millis())
    } else {
        format!("{:.1} seconds", duration.as_secs_f64())
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

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        kill_group(&self.process_group);
        self.worker.abort();
        self.pending.lock().expect("pending lock").clear();
        self.controls.lock().expect("control lock").clear();
    }
}
