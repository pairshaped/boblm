//! One supervised Pi RPC process. Only a matching user message proves consumption.
mod translate;
use boblm_protocol::{Event, LifecycleState, Priority};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io,
    path::Path,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use translate::{Translator, assistant_text};

pub struct PiOptions<'a> {
    pub cwd: &'a Path,
    /// Private, persistent Pi configuration and session directory owned by boblm.
    pub state_dir: &'a Path,
    /// Absolute session-file path previously announced by Pi.
    pub resume: Option<&'a str>,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    /// Extracted by the server, placed only in the child environment.
    pub api_key: &'a str,
    pub mcp: Option<McpEndpoint<'a>>,
}
pub struct McpEndpoint<'a> {
    pub url: &'a str,
    pub token: &'a str,
}
type Reply = Result<Value, String>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Reply>>>>;
pub struct PiSession {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    translator: Arc<Mutex<Translator>>,
    session_file: Mutex<Option<String>>,
    next_id: AtomicU64,
    usage_enabled: Arc<AtomicBool>,
    usage_changed: Arc<tokio::sync::Notify>,
    worker: JoinHandle<io::Result<std::process::ExitStatus>>,
    process_group: Arc<AtomicI32>,
    injector: mpsc::WeakSender<Event>,
    _extension: Option<tempfile::TempDir>,
    pub events: mpsc::Receiver<Event>,
}
impl PiSession {
    pub async fn spawn(options: PiOptions<'_>) -> io::Result<Self> {
        Self::spawn_command(Command::new(boblm_tuning::PI_BINARY), options).await
    }
    pub async fn spawn_command(mut command: Command, options: PiOptions<'_>) -> io::Result<Self> {
        if let Some(path) = options.resume {
            let meta = std::fs::metadata(path).map_err(|e| io::Error::other(format!(
                "Pi could not resume its conversation at {path}: {e}. Restore the session file; no new conversation was started in its place."
            )))?;
            if !Path::new(path).is_absolute() || !meta.is_file() {
                return Err(io::Error::other(format!(
                    "Pi cannot resume {path}: it must be an absolute path to a session file. Restore that file; no new conversation was started in its place."
                )));
            }
            command.arg("--session").arg(path);
        }
        check_project_settings(options.cwd)?;
        std::fs::create_dir_all(options.state_dir).map_err(|e| io::Error::other(format!("Pi could not create its state directory {}: {e}. Check the directory's permissions.",options.state_dir.display())))?;
        // Provider SDK retries are separate from Pi's automatic turn retries.
        std::fs::write(
            options.state_dir.join("settings.json"),
            serde_json::to_vec(&json!({
                "retry": {"enabled": false, "provider": {"maxRetries": boblm_tuning::PI_PROVIDER_MAX_RETRIES}},
                "compaction": {"enabled": false}
            }))?,
        )
        .map_err(|e| {
            io::Error::other(format!(
                "Pi could not write its settings in {}: {e}. Check the directory's permissions.",
                options.state_dir.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(options.state_dir, std::fs::Permissions::from_mode(0o700))?;
            command.process_group(0);
        }
        let system_prompt = format!(
            "{} {}",
            boblm_tuning::PI_SYSTEM_PROMPT_APPEND,
            boblm_tuning::PI_VISIBLE_PROGRESS_PROMPT
        );
        command
            .args([
                "--mode",
                "rpc",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--provider",
                boblm_tuning::PI_PROVIDER,
                "--model",
                options.model,
                "--append-system-prompt",
                &system_prompt,
            ])
            .arg("--session-dir")
            .arg(options.state_dir.join("sessions"))
            .env("PI_CODING_AGENT_DIR", options.state_dir)
            .env(boblm_tuning::PI_PROVIDER_API_KEY_ENV, options.api_key);
        // A private, credential-free catalog keeps Pi's bundled model list from selecting retired IDs.
        let models: Vec<Value> = boblm_tuning::MODELS.iter().filter(|m| m.agent == "pi").map(|m| json!({
            "id": m.id, "name": m.id, "reasoning": true, "input": ["text", "image"],
            "contextWindow": boblm_tuning::PI_MODEL_CONTEXT_WINDOW,
            "maxTokens": boblm_tuning::PI_MODEL_MAX_TOKENS,
            "cost": {"input": boblm_tuning::PI_MODEL_COST_INPUT, "output": boblm_tuning::PI_MODEL_COST_OUTPUT, "cacheRead": boblm_tuning::PI_MODEL_COST_CACHE_READ, "cacheWrite": 0}
        })).collect();
        std::fs::write(
            options.state_dir.join("models.json"),
            serde_json::to_vec(&json!({"providers": {
                boblm_tuning::PI_PROVIDER: {"api": boblm_tuning::PI_PROVIDER_API, "apiKey": boblm_tuning::PI_PROVIDER_API_KEY_ENV, "baseUrl": boblm_tuning::PI_PROVIDER_BASE_URL,
                    // DeepInfra rejects the developer role and max_completion_tokens that Pi
                    // otherwise sends to reasoning models on this protocol.
                    "compat": {"supportsDeveloperRole": false, "maxTokensField": "max_tokens", "thinkingFormat": boblm_tuning::PI_PROVIDER_COMPAT_THINKING_FORMAT},
                    "models": models}
            }}))?,
        )?;
        let extension = if let Some(mcp) = &options.mcp {
            let dir = tempfile::Builder::new()
                .prefix("boblm-tools-")
                .tempdir_in(options.state_dir)?;
            let path = dir.path().join("peer-tools.mjs");
            std::fs::write(&path, include_str!("peer-tools.mjs"))?;
            command
                .arg("--extension")
                .arg(path)
                .env("BOBLM_MCP_URL", mcp.url)
                .env("BOBLM_MCP_TOKEN", mcp.token)
                .env(
                    "BOBLM_MCP_TIMEOUT_MS",
                    boblm_tuning::PI_MCP_TIMEOUT.as_millis().to_string(),
                )
                .env(
                    "BOBLM_MCP_MAX_BYTES",
                    boblm_tuning::PI_MCP_MAX_BYTES.to_string(),
                );
            Some(dir)
        } else {
            None
        };
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned();
        let mut child = command.current_dir(options.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().map_err(|e| io::Error::new(e.kind(), format!("Could not start {program:?} in {}: {e}. Check that Pi is installed on the server's PATH and that the chat's directory exists.",options.cwd.display())))?;
        let group = child.id().and_then(|id| i32::try_from(id).ok()).ok_or_else(|| io::Error::other("Pi started without a process ID, so its process group cannot be supervised. Restart the chat."))?;
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take().ok_or_else(
            || io::Error::other("Pi started without an input pipe. Restart the chat."),
        )?));
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::other("Pi started without an output pipe. Restart the chat.")
        })?;
        let pending = Pending::default();
        let translator = Arc::new(Mutex::new(Translator::default()));
        let process_group = Arc::new(AtomicI32::new(group));
        let (tx, events) = mpsc::channel(boblm_tuning::EVENT_CHANNEL_CAPACITY);
        let injector = tx.downgrade();
        // These copies exist only for this process lifetime, to scrub accidental echoes from Pi.
        let mut scrub = vec![options.api_key.to_owned()];
        if let Some(mcp) = &options.mcp {
            scrub.push(mcp.token.to_owned());
        }
        let usage_enabled = Arc::new(AtomicBool::new(false));
        let usage_changed = Arc::new(tokio::sync::Notify::new());
        let worker = tokio::spawn(read_loop(
            child,
            stdout,
            stdin.clone(),
            pending.clone(),
            translator.clone(),
            tx,
            process_group.clone(),
            scrub,
            usage_enabled.clone(),
            usage_changed.clone(),
        ));
        let session = Self {
            stdin,
            pending,
            translator,
            session_file: Mutex::new(None),
            next_id: AtomicU64::new(0),
            usage_enabled,
            usage_changed,
            worker,
            process_group,
            injector,
            _extension: extension,
            events,
        };
        let startup = timeout(
            boblm_tuning::PI_STARTUP_TIMEOUT,
            session.handshake(&options),
        )
        .await;
        match startup {
            Ok(Ok(())) => Ok(session),
            outcome => {
                let error = match outcome {
                    Ok(Err(e)) => e,
                    _ => io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "Pi did not finish starting within {} seconds. Check Pi and the BobLM peer-tools endpoint, then reopen the chat.",
                            boblm_tuning::PI_STARTUP_TIMEOUT.as_secs()
                        ),
                    ),
                };
                let _ = session.stop().await;
                Err(error)
            }
        }
    }
    async fn handshake(&self, options: &PiOptions<'_>) -> io::Result<()> {
        let state = self.control("get_state", json!({})).await?;
        let file = session_path(&state)?;
        if let Some(expected) = options.resume
            && file != expected
        {
            return Err(io::Error::other(format!(
                "Pi could not resume its conversation at {expected}: it reported a different session file. No new conversation was started in its place; restore the original file."
            )));
        }
        if options.mcp.is_some() {
            let commands = self.control("get_commands", json!({})).await?;
            if !commands["commands"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|c| c["name"] == "boblm-peer-tools-ready"))
            {
                return Err(io::Error::other(
                    "Pi could not load BobLM's peer tools. Check that the BobLM HTTP MCP endpoint is reachable, then reopen the chat.",
                ));
            }
        }
        self.control("set_auto_retry", json!({"enabled":false}))
            .await?;
        self.control("set_auto_compaction", json!({"enabled":false}))
            .await?;
        self.control("set_steering_mode", json!({"mode":"all"}))
            .await?;
        self.configure(options.model, options.effort).await?;
        *self.session_file.lock().expect("session file lock") = Some(file);
        self.emit_init().await
    }
    pub fn session_id(&self) -> Option<String> {
        self.session_file.lock().expect("session file lock").clone()
    }
    async fn emit_init(&self) -> io::Result<()> {
        self.inject(Event::System {
            subtype: Some("init".into()),
            session_id: self.session_id(),
            status: None,
        })
        .await
    }
    pub fn enable_usage_reporting(&self) {
        self.usage_enabled.store(true, Ordering::Relaxed);
        self.usage_changed.notify_one();
    }

    pub async fn configure(&self, model: &str, effort: Option<&str>) -> io::Result<()> {
        let selected = self
            .control(
                "set_model",
                json!({"provider":boblm_tuning::PI_PROVIDER,"modelId":model}),
            )
            .await?;
        if selected["id"] != model || selected["provider"] != boblm_tuning::PI_PROVIDER {
            return Err(io::Error::other(
                "Pi selected a different model than requested. Check the Pi model catalog before sending another message.",
            ));
        }
        let effort = effort.unwrap_or(boblm_tuning::PI_DEFAULT_EFFORT);
        self.control("set_thinking_level", json!({"level":effort}))
            .await?;
        let state = self.control("get_state", json!({})).await?;
        if state["thinkingLevel"] != effort {
            return Err(io::Error::other(format!(
                "Pi did not select the requested thinking level {effort}. Choose a level supported by this model."
            )));
        }
        Ok(())
    }
    async fn write(&self, message: Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await.map_err(|e| io::Error::new(e.kind(),format!("Pi's input pipe closed while sending a request: {e}. Delivery is uncertain; inspect the chat before retrying.")))?;
        stdin.flush().await
    }
    async fn rpc(&self, kind: &str, mut fields: Value) -> io::Result<Reply> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst).to_string();
        fields["id"] = json!(id);
        fields["type"] = json!(kind);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock")
            .insert(id.clone(), tx);
        let outcome=timeout(boblm_tuning::PI_REQUEST_TIMEOUT,async { self.write(fields).await?; rx.await.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe,format!("Pi exited before answering the {kind} request. Its outcome is uncertain; inspect the chat before retrying."))) }).await;
        self.pending.lock().expect("pending lock").remove(&id);
        outcome.map_err(|_| io::Error::new(io::ErrorKind::TimedOut,format!("Pi did not answer the {kind} request within {} seconds. Its outcome is uncertain; inspect the chat before retrying.",boblm_tuning::PI_REQUEST_TIMEOUT.as_secs())))?
    }
    async fn control(&self, kind: &str, fields: Value) -> io::Result<Value> {
        self.rpc(kind, fields).await?.map_err(|why| {
            io::Error::other(format!(
                "Pi refused the {kind} request: {why}. Check the requested action before retrying."
            ))
        })
    }
    async fn inject(&self, event: Event) -> io::Result<()> {
        self.injector
            .upgrade()
            .ok_or_else(|| {
                io::Error::other("Pi has exited, so it cannot report this action. Reopen the chat.")
            })?
            .send(event)
            .await
            .map_err(|_| io::Error::other("The chat stopped listening to Pi. Reopen the chat."))
    }
    async fn refuse(&self, uuid: &str, why: &str) -> io::Result<()> {
        self.inject(assistant_text(
            &format!("refused-{uuid}"),
            &format!("Pi did not accept this message: {why}. Correct the request before retrying."),
        ))
        .await?;
        self.inject(Event::CommandLifecycle {
            command_uuid: uuid.into(),
            state: LifecycleState::Refused,
        })
        .await
    }
    pub async fn send_message(
        &self,
        uuid: &str,
        content: &str,
        _priority: Option<Priority>,
    ) -> io::Result<()> {
        let text = content.trim();
        if text == "/clear" {
            return self.clear(uuid).await;
        }
        if text == "/compact" || text.starts_with("/compact ") {
            self.translator.lock().expect("translator lock").compact = Some(uuid.into());
            // Compaction can outlive a control deadline. Read its events asynchronously so stop works.
            return self.write(json!({"type":"compact","id":format!("compact-{uuid}"),"customInstructions":text.strip_prefix("/compact").unwrap_or("").trim()})).await;
        }
        if text.starts_with('/') {
            return self
                .refuse(
                    uuid,
                    "only /compact and /clear are supported as Pi commands",
                )
                .await;
        }
        self.translator
            .lock()
            .expect("translator lock")
            .expect_message(uuid, content);
        if let Err(why) = self
            .rpc(
                "prompt",
                json!({"message":content,"streamingBehavior":"steer"}),
            )
            .await?
        {
            let unconsumed = self
                .translator
                .lock()
                .expect("translator lock")
                .reject(uuid);
            if unconsumed {
                self.refuse(uuid, &why).await?;
            }
        }
        Ok(())
    }
    async fn clear(&self, uuid: &str) -> io::Result<()> {
        let reply = match self.rpc("new_session", json!({})).await? {
            Ok(v) => v,
            Err(why) => return self.refuse(uuid, &why).await,
        };
        if reply["cancelled"] != false {
            return self.refuse(uuid, "the new conversation was canceled").await;
        }
        let state = self.control("get_state", json!({})).await?;
        let file = session_path(&state)?;
        if self.session_id().as_deref() == Some(&file) {
            return Err(io::Error::other(
                "Pi did not create a different conversation after /clear. The outcome is uncertain; inspect the chat before retrying.",
            ));
        }
        *self.session_file.lock().expect("session file lock") = Some(file);
        self.emit_init().await?;
        self.inject(Event::CommandLifecycle {
            command_uuid: uuid.into(),
            state: LifecycleState::Completed,
        })
        .await?;
        self.inject(Event::Result {
            subtype: Some("success".into()),
            is_error: Some(false),
        })
        .await
    }
    pub async fn request_interrupt(&self) -> io::Result<()> {
        self.control("abort", json!({})).await?;
        // Pi's abort leaves queued steering messages alive. Resume only persisted history.
        kill_group(&self.process_group);
        Ok(())
    }
    pub async fn stop(mut self) -> io::Result<()> {
        self.events.close();
        kill_group(&self.process_group);
        (&mut self.worker).await.map_err(io::Error::other)??;
        Ok(())
    }
}
impl Drop for PiSession {
    fn drop(&mut self) {
        kill_group(&self.process_group);
        self.worker.abort();
        self.pending.lock().expect("pending lock").clear();
    }
}
/// Project settings override private global settings, including delivery-sensitive policies.
fn check_project_settings(cwd: &Path) -> io::Result<()> {
    let path = cwd.join(".pi/settings.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(io::Error::other(format!(
                "Pi could not read {}: {e}. Check the project's Pi settings before reopening the chat.",
                path.display()
            )));
        }
    };
    let settings: Value = serde_json::from_slice(&bytes).map_err(|_| io::Error::other(format!("Pi cannot read {} because it is not valid JSON. Correct the project's Pi settings before reopening the chat.",path.display())))?;
    for (pointer, required) in [
        ("/retry/enabled", json!(false)),
        (
            "/retry/provider/maxRetries",
            json!(boblm_tuning::PI_PROVIDER_MAX_RETRIES),
        ),
        ("/compaction/enabled", json!(false)),
    ] {
        if let Some(value) = settings.pointer(pointer)
            && !value.is_null()
            && value != &required
        {
            return Err(io::Error::other(format!(
                "Pi cannot start because {} overrides {pointer}. Remove that override so BobLM can keep turn retries and automatic compaction disabled and use its chosen provider retry count.",
                path.display()
            )));
        }
    }
    Ok(())
}
fn session_path(state: &Value) -> io::Result<String> {
    state["sessionFile"].as_str().filter(|s|Path::new(s).is_absolute()).map(str::to_owned).ok_or_else(|| io::Error::other("Pi did not report an absolute session-file path. Check the Pi version before reopening this chat."))
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
struct LineReader {
    reader: BufReader<tokio::process::ChildStdout>,
    line: Vec<u8>,
    oversized: bool,
}
enum Line {
    Text(Vec<u8>),
    Oversized,
}
impl LineReader {
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

#[allow(clippy::too_many_arguments)]
async fn read_loop(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    translator: Arc<Mutex<Translator>>,
    tx: mpsc::Sender<Event>,
    group: Arc<AtomicI32>,
    scrub: Vec<String>,
    usage_enabled: Arc<AtomicBool>,
    usage_changed: Arc<tokio::sync::Notify>,
) -> io::Result<std::process::ExitStatus> {
    let mut lines = LineReader {
        reader: BufReader::new(stdout),
        line: Vec::new(),
        oversized: false,
    };
    let mut usage_tick = tokio::time::interval(boblm_tuning::USAGE_POLL_INTERVAL);
    usage_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut drain_until = None;
    let mut drain_limit = None;
    loop {
        if drain_limit.is_some_and(|limit| tokio::time::Instant::now() >= limit) {
            break;
        }
        let line = tokio::select! {
            biased;
            _=child.wait(), if drain_until.is_none()=>{
                kill_group(&group);
                let now=tokio::time::Instant::now();
                drain_until=Some(now+boblm_tuning::DRAIN_AFTER_EXIT);
                drain_limit=Some(now+boblm_tuning::DRAIN_LIMIT);continue;
            }
            _=usage_changed.notified()=>continue,
            _=usage_tick.tick(), if usage_enabled.load(Ordering::Relaxed) && drain_until.is_none()=>{
                try_usage_request(&stdin).await;
                continue;
            }
            line=lines.next()=>line,
            _=async {tokio::time::sleep_until(drain_until.unwrap_or_else(tokio::time::Instant::now)).await}, if drain_until.is_some()=>break,
        };
        let bytes = match line {
            Ok(Some(Line::Text(b))) => b,
            Ok(Some(Line::Oversized)) => continue,
            _ => break,
        };
        let Ok(mut text) = String::from_utf8(bytes) else {
            continue;
        };
        for secret in &scrub {
            if !secret.is_empty() {
                text = text.replace(secret, "[redacted]");
            }
        }
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if usage_enabled.load(Ordering::Relaxed) && v["type"] == "agent_end" {
            try_usage_request(&stdin).await;
        }
        if v["type"] == "response" {
            if v["id"] == "boblm-context-usage" {
                if v["success"] == true
                    && let Some(context) =
                        boblm_protocol::usage::pi_context(&v["data"]["contextUsage"])
                    && tx
                        .send(Event::Usage {
                            five_hour: None,
                            credits: None,
                            context: Some(context),
                            weekly: None,
                        })
                        .await
                        .is_err()
                {
                    break;
                }
                continue;
            }
            if let Some(waiter) = v["id"]
                .as_str()
                .and_then(|id| pending.lock().expect("pending lock").remove(id))
            {
                let reply = if v["success"] == true {
                    Ok(v.get("data").cloned().unwrap_or(Value::Null))
                } else {
                    Err(translate::error_reason(
                        v["error"].as_str().unwrap_or("Pi supplied no explanation"),
                    ))
                };
                let _ = waiter.send(reply);
            }
        } else if v["type"] == "extension_ui_request" && v["id"].is_string() {
            let reply = json!({"type":"extension_ui_response","id":v["id"],"cancelled":true});
            let mut data = serde_json::to_vec(&reply)?;
            data.push(b'\n');
            let sent = timeout(boblm_tuning::PI_REQUEST_TIMEOUT, async {
                let mut pipe = stdin.lock().await;
                pipe.write_all(&data).await?;
                pipe.flush().await
            })
            .await;
            if !matches!(sent, Ok(Ok(()))) {
                break;
            }
        }
        let events = translator.lock().expect("translator lock").notification(&v);
        for event in events {
            if tx.send(event).await.is_err() {
                kill_group(&group);
                pending.lock().expect("pending lock").clear();
                return child.wait().await;
            }
        }
    }
    kill_group(&group);
    pending.lock().expect("pending lock").clear();
    child.wait().await
}

/// Never wait for a metadata pipe write while the child may be working.
async fn try_usage_request(stdin: &tokio::sync::Mutex<ChildStdin>) {
    use std::{pin::Pin, task::Poll};
    use tokio::io::AsyncWrite;
    let line = b"{\"id\":\"boblm-context-usage\",\"type\":\"get_session_stats\"}\n";
    let Ok(mut input) = stdin.try_lock() else {
        return;
    };
    std::future::poll_fn(|cx| {
        let _ = Pin::new(&mut *input).poll_write(cx, line);
        Poll::Ready(())
    })
    .await;
}
