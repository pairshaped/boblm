//! One serial command queue per session. SQLite work runs on blocking workers.
pub mod driver;
pub mod events;
pub mod transport;
use boblm_protocol::{Event, LifecycleState, Priority};
use boblm_store::{NewMessage, RangeRow, Session, Store, StoredEvent};
pub use driver::{ClaudeDriver, ClaudeFactory, CodexDriver, Driver, DriverFactory, PiDriver};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] boblm_store::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("this chat's task is not running, usually because the server is shutting down")]
    Closed,
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
}
pub type Result<T> = std::result::Result<T, Error>;

/// A scheduled prompt starts with this (ADR 0022). Like a slash command it never steers a
/// running turn, and unlike an owner message it does not release a held queue.
pub const SCHEDULED_PREFIX: &str = "Scheduled prompt";

/// Messages that wait for the agent to be idle instead of steering it.
fn waits_for_idle(content: &str) -> bool {
    let content = content.trim_start();
    content.starts_with('/') || content.starts_with(SCHEDULED_PREFIX)
}

/// The one spelling of a message UUID used for dedupe and storage: lowercase, hyphenated.
/// Every entry point canonicalizes before comparing, so case or punctuation cannot
/// turn one client message into two.
pub fn canonical_uuid(value: &str) -> Result<String> {
    uuid::Uuid::parse_str(value)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|_| {
            Error::Invalid(format!(
                "{} is not a message UUID; use one like 123e4567-e89b-12d3-a456-426614174000.",
                boblm_protocol::client::shown(value)
            ))
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    /// Latest provider prediction; transient and absent when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_suggestion: Option<String>,
    pub jobs: Option<boblm_protocol::jobs::JobSnapshot>,
    pub usage: Option<Box<boblm_protocol::usage::UsageSnapshot>>,
    #[serde(default)]
    pub account_usage: Option<Box<boblm_protocol::usage::AccountUsage>>,
    /// The owner stopped the chat and has not spoken since; scheduled prompts leave it alone.
    pub stopped: bool,
    pub selection_pending: bool,
    /// The last driver error, while it is why the queue is held.
    #[serde(default)]
    pub held_reason: Option<String>,
    pub queue_held: bool,
    pub running: bool,
    pub turn_active: bool,
    pub stopping: bool,
    pub queued: usize,
    pub dispatching: usize,
    pub uncertain: usize,
    pub rejected: usize,
    pub archived: bool,
    pub model: String,
    pub effort: Option<String>,
}
/// Outcome of a self-compaction request, including refusals and the observed context size.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactResult {
    pub queued: bool,
    pub already_queued: bool,
    pub message: String,
    pub context: CompactContext,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactContext {
    pub used: Option<u64>,
    pub window: Option<u64>,
    pub percent: Option<f64>,
}
#[derive(Clone)]
pub struct SessionHandle {
    tx: mpsc::Sender<Command>,
    store: Store,
    id: String,
    published_status: watch::Receiver<Status>,
}
pub struct RunningSession {
    handle: SessionHandle,
    task: Option<JoinHandle<Result<()>>>,
    remote: bool,
}
#[derive(Serialize, Deserialize)]
enum Action {
    Delete,
    Compact(Option<String>),
    Send(String, String, Vec<String>, Option<String>),
    SendAutomated(String, String),
    SubagentResult(String, String),
    Status,
    Resume,
    Stop,
    Archive(bool),
    Retry(String),
    SetModel(String, Option<String>),
    Rename { name: String, only_if_unnamed: bool },
    Shutdown,
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Compact(CompactResult),
    Accepted(bool),
    Status(Status),
    Done,
}
struct Command {
    action: Action,
    reply: oneshot::Sender<Result<Reply>>,
}
impl SessionHandle {
    /// Queue an early compaction without steering, releasing a hold or clearing Stop.
    pub async fn compact(&self, instructions: Option<&str>) -> Result<CompactResult> {
        match self
            .call(Action::Compact(instructions.map(str::to_owned)))
            .await?
        {
            Reply::Compact(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    pub async fn rename(&self, name: &str) -> Result<()> {
        self.call(Action::Rename {
            name: name.into(),
            only_if_unnamed: false,
        })
        .await
        .map(|_| ())
    }
    /// Names a chat only when it has no title, serialized with explicit renames.
    pub async fn name_if_unnamed(&self, name: &str) -> Result<bool> {
        match self
            .call(Action::Rename {
                name: name.into(),
                only_if_unnamed: true,
            })
            .await?
        {
            Reply::Accepted(changed) => Ok(changed),
            _ => Err(Error::Closed),
        }
    }
    pub async fn set_model(&self, model: &str, effort: Option<&str>) -> Result<()> {
        self.call(Action::SetModel(
            model.to_owned(),
            effort.map(str::to_owned),
        ))
        .await
        .map(|_| ())
    }
    async fn call(&self, action: Action) -> Result<Reply> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command { action, reply })
            .await
            .map_err(|_| Error::Closed)?;
        rx.await.map_err(|_| Error::Closed)?
    }
    pub async fn send(&self, uuid: &str, content: &str) -> Result<bool> {
        self.send_with_attachments(uuid, content, &[]).await
    }
    /// Automated notices retain owner queue behavior without impersonating human input.
    pub async fn send_automated(&self, uuid: &str, content: &str) -> Result<bool> {
        match self
            .call(Action::SendAutomated(uuid.into(), content.into()))
            .await?
        {
            Reply::Accepted(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    /// Attachments are upload IDs in the owner's order. Returns false for an identical retry.
    pub async fn send_with_attachments(
        &self,
        uuid: &str,
        content: &str,
        attachments: &[String],
    ) -> Result<bool> {
        let action = Action::Send(uuid.into(), content.into(), attachments.to_vec(), None);
        match self.call(action).await? {
            Reply::Accepted(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    /// Read-side status never waits behind provider startup or a write to its input pipe.
    pub fn published_status(&self) -> Result<Status> {
        if self.tx.is_closed() {
            return Err(Error::Closed);
        }
        Ok(self.published_status.borrow().clone())
    }
    /// Watch published status changes without queueing Status commands behind the actor.
    pub fn status_updates(&self) -> watch::Receiver<Status> {
        self.published_status.clone()
    }
    pub async fn status(&self) -> Result<Status> {
        match self.call(Action::Status).await? {
            Reply::Status(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    pub async fn send_from_peer(&self, uuid: &str, content: &str, sender: &str) -> Result<bool> {
        match self
            .call(Action::Send(
                uuid.into(),
                content.into(),
                Vec::new(),
                Some(sender.into()),
            ))
            .await?
        {
            Reply::Accepted(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    /// Accept a completion only between turns. False means the observer must recheck
    /// native delivery after the current turn; an accepted retry returns true too.
    pub async fn send_subagent_result(&self, uuid: &str, content: &str) -> Result<bool> {
        match self
            .call(Action::SubagentResult(uuid.into(), content.into()))
            .await?
        {
            Reply::Accepted(value) => Ok(value),
            _ => Err(Error::Closed),
        }
    }
    pub async fn delete(&self) -> Result<()> {
        self.call(Action::Delete).await.map(|_| ())
    }
    pub async fn resume(&self) -> Result<()> {
        self.call(Action::Resume).await.map(|_| ())
    }
    pub async fn stop(&self) -> Result<()> {
        self.call(Action::Stop).await.map(|_| ())
    }
    pub async fn archive(&self, archived: bool) -> Result<()> {
        self.call(Action::Archive(archived)).await.map(|_| ())
    }
    pub async fn retry(&self, uuid: &str) -> Result<()> {
        self.call(Action::Retry(uuid.into())).await.map(|_| ())
    }
    /// Durable pages avoid a replay/live race and let slow consumers catch up.
    pub async fn events_since(&self, after: i64, limit: i64) -> Result<Vec<StoredEvent>> {
        if self.tx.is_closed() {
            return Err(Error::Closed);
        }
        if limit > boblm_tuning::EVENT_PAGE_LIMIT_MAX {
            return Err(Error::Invalid(format!(
                "Asked for {limit} events at once; the limit is {}. Request a smaller page.",
                boblm_tuning::EVENT_PAGE_LIMIT_MAX
            )));
        }
        let store = self.store.clone();
        let id = self.id.clone();
        Ok(
            tokio::task::spawn_blocking(move || store.reader()?.events_since(&id, after, limit))
                .await??,
        )
    }
}
/// The agent's name for messages the owner reads, so a Codex chat never blames Claude.
fn agent_label(agent: &str) -> &'static str {
    boblm_protocol::client::Agent::parse(agent).map_or("The agent", |a| a.label())
}
impl RunningSession {
    pub async fn start(
        store: Store,
        session: &str,
        factory: Arc<dyn DriverFactory>,
    ) -> Result<Self> {
        if let Some(runner) = factory.connect_runner(&store, session).await? {
            return Ok(runner);
        }
        let source = store.clone();
        let id = session.to_owned();
        let stopped =
            tokio::task::spawn_blocking(move || source.reader()?.runner_stopped(&id)).await??;
        let source = store.clone();
        let id = session.to_owned();
        let quiet_stop =
            tokio::task::spawn_blocking(move || source.take_runner_quiet_stop(&id)).await??;
        let mut actor = Actor {
            prompt_suggestion: None,
            store,
            id: session.into(),
            factory,
            driver: None,
            jobs: None,
            messages: BTreeMap::new(),
            pending: HashMap::new(),
            active: false,
            consumed_active: false,
            compacting: false,
            held: true,
            hold_reason: None,
            stopped,
            stop_deadline: None,
            stop_replies: Vec::new(),
            archive_after_stop: false,
            applied_choice: None,
            auto_task: None,
            overflow_resume_used: false,
            outstanding_tools: HashSet::new(),
            provider_rejection: false,
            published_status: None,
            status_updates: None,
        };
        actor.refresh().await?;
        actor.session().await?;
        // A new runner can restore an existing chat's account mapping before a child
        // launches, using the provider file in its own environment. Missing means leave it.
        actor.remember_account().await?;
        for message in actor.messages.values().cloned().collect::<Vec<_>>() {
            if message.delivery_state == "dispatching" {
                actor
                    .transition(&message.uuid, "dispatching", "uncertain")
                    .await?;
            }
        }
        if quiet_stop
            && !actor.stopped
            && !actor
                .messages
                .values()
                .any(|m| matches!(m.delivery_state.as_str(), "dispatching" | "uncertain"))
        {
            actor.release_hold();
        }
        // Stale input expires before anything can dispatch it, including after a long outage.
        actor.expire_stale().await?;
        actor.refresh().await?;
        actor.publish_status().await?;
        let (status_tx, published_status) = watch::channel(actor.status().await?);
        actor.status_updates = Some(status_tx);
        let read_store = actor.store.clone();
        let read_id = actor.id.clone();
        let (tx, rx) = mpsc::channel(boblm_tuning::COMMAND_QUEUE_CAPACITY);
        let task_id = actor.id.clone();
        let task = tokio::spawn(async move {
            let result = actor.run(rx).await;
            if let Err(error) = &result {
                eprintln!(
                    "session={task_id}: The chat task stopped with an error: {error}. Check the server before resuming the chat."
                );
            }
            result
        });
        Ok(Self {
            handle: SessionHandle {
                tx,
                store: read_store,
                id: read_id,
                published_status,
            },
            task: Some(task),
            remote: false,
        })
    }
    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }
    pub async fn shutdown(mut self) -> Result<()> {
        if self.task.is_none() {
            return Ok(());
        }
        if self.remote {
            if let Some(task) = self.task.take() {
                task.abort();
            }
            return Ok(());
        }
        let result = self.handle.call(Action::Shutdown).await;
        if let Some(task) = self.task.take() {
            task.await??;
        }
        result.map(|_| ())
    }
}
impl Drop for RunningSession {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
use boblm_protocol::client::MAX_ATTACHMENTS;
struct Pending {
    deferred_to_boundary: bool,
    awaiting_tool_boundary: bool,
    queued: bool,
    deadline: Instant,
    result_seen: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum AutoKind {
    Compact,
    Resume,
}
struct AutoTask {
    uuid: String,
    kind: AutoKind,
    resume_after: bool,
    dispatched: bool,
}
struct Actor {
    prompt_suggestion: Option<String>,
    jobs: Option<boblm_protocol::jobs::JobSnapshot>,
    store: Store,
    id: String,
    factory: Arc<dyn DriverFactory>,
    driver: Option<Box<dyn Driver>>,
    messages: BTreeMap<i64, RangeRow>,
    pending: HashMap<String, Pending>,
    active: bool,
    consumed_active: bool,
    compacting: bool,
    held: bool,
    hold_reason: Option<String>,
    stopped: bool,
    stop_deadline: Option<Instant>,
    stop_replies: Vec<oneshot::Sender<Result<Reply>>>,
    archive_after_stop: bool,
    applied_choice: Option<(String, Option<String>)>,
    auto_task: Option<AutoTask>,
    overflow_resume_used: bool,
    outstanding_tools: HashSet<String>,
    provider_rejection: bool,
    published_status: Option<serde_json::Value>,
    status_updates: Option<watch::Sender<Status>>,
}
impl Actor {
    async fn db<T: Send + 'static>(
        &self,
        f: impl FnOnce(Store) -> boblm_store::Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.store.clone();
        Ok(tokio::task::spawn_blocking(move || f(store)).await??)
    }
    async fn session(&self) -> Result<Session> {
        let id = self.id.clone();
        self.db(move |store| store.reader()?.sessions())
            .await?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "No chat with ID {} exists on this server.",
                    boblm_protocol::client::shown(&self.id)
                ))
            })
    }
    async fn remember_account(&self) -> Result<()> {
        let session = self.session().await?;
        let Some(account_id) = self.factory.account_id(&session) else {
            return Ok(());
        };
        let id = self.id.clone();
        self.db(move |store| store.set_session_account(&id, Some(&account_id)))
            .await?;
        Ok(())
    }
    async fn refresh(&mut self) -> Result<()> {
        let id = self.id.clone();
        self.messages = self
            .db(move |store| store.reader()?.outstanding_messages(&id))
            .await?
            .into_iter()
            .map(|m| (m.seq, m))
            .collect();
        Ok(())
    }
    async fn transition(&self, uuid: &str, from: &str, to: &str) -> Result<()> {
        let (uuid, from, to) = (uuid.to_owned(), from.to_owned(), to.to_owned());
        self.db(move |store| store.transition(&uuid, &from, &to))
            .await
    }
    async fn record(&mut self, kind: &str, payload: serde_json::Value) -> Result<()> {
        if matches!(kind, "driver_error" | "model_error") {
            eprintln!(
                "session={} {kind}: {}",
                self.id,
                payload["message"]
                    .as_str()
                    .unwrap_or("No diagnostic message was provided.")
            );
        }
        if matches!(kind, "driver_error" | "model_error")
            && let Some(message) = payload["message"].as_str()
        {
            self.hold_reason = Some(
                message
                    .chars()
                    .take(boblm_tuning::MAX_HELD_REASON_CHARS)
                    .collect(),
            );
        }
        let (id, kind) = (self.id.clone(), kind.to_owned());
        self.db(move |store| store.record_event(&id, &kind, payload, now()))
            .await
    }
    /// The owner or an explicit retry lifted the hold; the error that caused it is spent.
    fn release_hold(&mut self) {
        self.held = false;
        self.hold_reason = None;
    }
    /// Expire queued, held and unconfirmed messages older than QUEUED_MESSAGE_MAX_AGE. Expiry
    /// never sends anything, so the never-replay rule holds (ADR 0004). The store conditionally
    /// updates only rows still queued or uncertain, in one transaction, so a message already
    /// dispatching is untouched. When nothing is left to release, the hold clears too.
    async fn expire_stale(&mut self) -> Result<()> {
        let cutoff = stale_cutoff();
        let id = self.id.clone();
        let expired = self
            .db(move |store| store.expire_stale(&id, now(), cutoff))
            .await?;
        if expired.is_empty() {
            return Ok(());
        }
        self.after_expiry().await
    }
    /// Reload the queue after rows expired and lift the hold when nothing is left to release.
    async fn after_expiry(&mut self) -> Result<()> {
        self.refresh().await?;
        if self.auto_task.as_ref().is_some_and(|task| {
            !task.dispatched
                && !self
                    .messages
                    .values()
                    .any(|message| message.uuid == task.uuid)
        }) {
            self.auto_task = None;
        }
        let nothing_left = !self
            .messages
            .values()
            .any(|m| matches!(m.delivery_state.as_str(), "queued" | "uncertain"));
        if nothing_left && !self.stopped {
            self.release_hold();
        }
        Ok(())
    }
    /// When the oldest message that can still expire is due, never more than
    /// QUEUED_MESSAGE_SWEEP_INTERVAL away, so a suspended clock cannot delay expiry for long.
    fn expiry_deadline(&self) -> Option<Instant> {
        let oldest = self
            .messages
            .values()
            .filter(|m| matches!(m.delivery_state.as_str(), "queued" | "uncertain"))
            .map(|m| m.created_at)
            .min()?;
        let max_age = boblm_tuning::QUEUED_MESSAGE_MAX_AGE.as_millis() as i64;
        let due_ms = u64::try_from(oldest.saturating_add(max_age) - now()).unwrap_or(0);
        let wait = std::time::Duration::from_millis(due_ms)
            .min(boblm_tuning::QUEUED_MESSAGE_SWEEP_INTERVAL);
        Some(Instant::now() + wait)
    }
    async fn status(&self) -> Result<Status> {
        let session = self.session().await?;
        let id = self.id.clone();
        let samples = self
            .db(move |store| store.reader()?.usage_samples(&id))
            .await?;
        let context = samples
            .context
            .as_ref()
            .and_then(|(raw, _)| {
                serde_json::from_str::<boblm_protocol::usage::ContextUsage>(raw).ok()
            })
            .filter(|value| value.window_tokens > 0);
        let usage = context.as_ref().map(|context| {
            Box::new(boblm_protocol::usage::UsageSnapshot {
                context_updated_at: samples.context.map(|(_, at)| at),
                context: Some(context.clone()),
            })
        });
        let account_id = self.id.clone();
        let account_usage = self
            .db(move |store| store.reader()?.account_usage_for_session(&account_id))
            .await?
            .map(Box::new);
        let count = |state| {
            self.messages
                .values()
                .filter(|m| m.delivery_state == state)
                .count()
        };
        // Durable, so a disconnect or restart cannot hide a stored but unapplied choice.
        let selection_pending =
            session.applied_model != session.model || session.applied_effort != session.effort;
        Ok(Status {
            prompt_suggestion: self.prompt_suggestion.clone(),
            jobs: self.jobs.clone(),
            usage,
            account_usage,
            stopped: self.stopped,
            selection_pending,
            held_reason: if self.held {
                self.hold_reason.clone()
            } else {
                None
            },
            queue_held: self.held,
            running: self.driver.is_some(),
            turn_active: self.active,
            stopping: self.stop_deadline.is_some(),
            queued: count("queued"),
            dispatching: count("dispatching"),
            uncertain: count("uncertain"),
            rejected: count("rejected"),
            archived: session.archived,
            model: session.model,
            effort: session.effort,
        })
    }
    async fn publish_status(&mut self) -> Result<()> {
        let current = self.status().await?;
        let status = serde_json::to_value(events::wire_status(current.clone())?)
            .map_err(std::io::Error::other)?;
        if self.published_status.as_ref() != Some(&status) {
            let (id, mut payload) = (self.id.clone(), status.clone());
            if let Some(object) = payload.as_object_mut() {
                object.remove("promptSuggestion");
            }
            self.db(move |store| store.publish_summary(&id, payload))
                .await?;
            self.published_status = Some(status);
        }
        if let Some(updates) = &self.status_updates {
            updates.send_replace(current);
        }
        Ok(())
    }
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) -> Result<()> {
        self.pump().await?;
        self.publish_status().await?;
        let mut clean_shutdown = false;
        loop {
            let pause_tool_boundary = !self.outstanding_tools.is_empty();
            let deadline = self
                .pending
                .values()
                .filter(|pending| {
                    !(pause_tool_boundary && pending.awaiting_tool_boundary)
                        && !(pending.deferred_to_boundary
                            && pending.queued
                            && !pending.result_seen
                            && self.consumed_active)
                })
                .map(|p| p.deadline)
                .chain(self.stop_deadline)
                .min();
            enum Wake {
                Command(Option<Command>),
                Event(Option<Event>),
                Deadline,
                Expiry,
            }
            let expiry = self.expiry_deadline();
            let wake = tokio::select! {
                command = commands.recv() => Wake::Command(command),
                event = async { match &mut self.driver { Some(driver) => driver.next_event().await, None => std::future::pending().await } } => Wake::Event(event),
                _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => Wake::Deadline,
                _ = async { match expiry { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Wake::Expiry,
            };
            match wake {
                Wake::Command(Some(command)) => {
                    let shutdown = matches!(command.action, Action::Shutdown);
                    let deleting = matches!(command.action, Action::Delete);
                    let defer_stop = matches!(command.action, Action::Stop | Action::Archive(true));
                    let result = self.execute(command.action).await;
                    let succeeded = result.is_ok();
                    let deleted = deleting && succeeded;
                    let fatal = matches!(&result, Err(Error::Store(error)) if !error.is_busy() && !matches!(error, boblm_store::Error::MessageConflict | boblm_store::Error::ChatDeletion(_)))
                        || matches!(&result, Err(Error::Task(_)));
                    if defer_stop && result.is_ok() && self.stop_deadline.is_some() {
                        self.stop_replies.push(command.reply);
                    } else {
                        let _ = command.reply.send(result);
                    }
                    if deleted {
                        return Ok(());
                    }
                    if shutdown || fatal {
                        clean_shutdown = shutdown && succeeded;
                        break;
                    }
                }
                Wake::Command(None) => break,
                Wake::Event(event) => {
                    self.consume_event_batch(event).await?;
                }
                Wake::Expiry => self.expire_stale().await?,
                Wake::Deadline => {
                    if self.stop_deadline.is_some() {
                        eprintln!(
                            "session={}: The agent did not finish stopping within {} seconds. Its process will be stopped; unconfirmed messages remain held.",
                            self.id,
                            boblm_tuning::STOP_RESULT_DEADLINE.as_secs()
                        );
                        self.record("stop_timeout", serde_json::json!({})).await?;
                        self.finish_stop(true).await?;
                    } else {
                        let session = self.session().await?;
                        let mut pending: Vec<_> = self.pending.keys().cloned().collect();
                        pending.sort();
                        eprintln!(
                            "session={} agent={} thread={}: The consumption acknowledgment deadline expired for messages {}. The agent will be stopped; inspect the chat before resuming. Initial window: {} seconds; after a result: at most {} seconds.",
                            self.id,
                            session.agent,
                            session.agent_session_id.as_deref().unwrap_or("unknown"),
                            pending.join(", "),
                            boblm_tuning::ACK_DEADLINE.as_secs(),
                            boblm_tuning::ACK_DEADLINE_AFTER_RESULT.as_secs()
                        );
                        self.record("delivery_timeout", serde_json::json!({}))
                            .await?;
                        self.disconnect().await?;
                    }
                }
            }
            self.pump().await?;
            self.publish_status().await?;
        }
        // Capture activity before disconnect clears it. Unknown job inventory cannot
        // prove quiet while a provider is running. Owner Stop keeps its separate hold.
        let quiet = clean_shutdown
            && !self.stopped
            && !self.active
            && !self.compacting
            && self.pending.is_empty()
            && self.stop_deadline.is_none()
            && self.outstanding_tools.is_empty()
            && !self.messages.values().any(|m| {
                matches!(
                    m.delivery_state.as_str(),
                    "queued" | "dispatching" | "uncertain"
                )
            })
            && self.jobs.as_ref().map_or(self.driver.is_none(), |jobs| {
                jobs.available && jobs.tasks.is_empty()
            });
        self.finish_stop(true).await?;
        self.disconnect().await?;
        self.publish_status().await?;
        let id = self.id.clone();
        self.db(move |store| store.set_runner_quiet_stop(&id, quiet))
            .await
    }
    async fn execute(&mut self, action: Action) -> Result<Reply> {
        let completion = matches!(&action, Action::SubagentResult(..));
        let automated = matches!(&action, Action::SendAutomated(..));
        let action = match action {
            Action::SendAutomated(uuid, content) => Action::Send(uuid, content, Vec::new(), None),
            Action::SubagentResult(uuid, content) => {
                if self.active
                    || !self.pending.is_empty()
                    || self.compacting
                    || self.held
                    || self.stopped
                    || self.stop_deadline.is_some()
                    || self.driver.is_none()
                    || self.session().await?.archived
                {
                    return Ok(Reply::Accepted(false));
                }
                // Use the ordinary peer queue and its durable UUID receipt.
                Action::Send(uuid, content, Vec::new(), Some(self.id.clone()))
            }
            action => action,
        };
        match action {
            Action::SendAutomated(..) | Action::SubagentResult(..) => {
                unreachable!("completion was handled above")
            }
            Action::Delete => {
                if !self.session().await?.archived {
                    return Err(Error::Invalid(
                        "Archive the chat before deleting it.".into(),
                    ));
                }
                self.finish_stop(true).await?;
                self.disconnect().await?;
                let id = self.id.clone();
                self.db(move |store| store.delete_session(&id)).await?;
                Ok(Reply::Done)
            }
            Action::Send(uuid, content, attachments, sender) => {
                let uuid = canonical_uuid(&uuid)?;
                let attachments = attachments
                    .iter()
                    .map(|id| canonical_uuid(id))
                    .collect::<Result<Vec<_>>>()?;
                if attachments.len() > MAX_ATTACHMENTS {
                    return Err(Error::Invalid(format!(
                        "a message carries at most {MAX_ATTACHMENTS} attachments"
                    )));
                }
                if (content.trim().is_empty() && attachments.is_empty())
                    || content.len() > boblm_tuning::MAX_MESSAGE_BYTES
                {
                    return Err(Error::Invalid(
                        "A message needs some text or an attachment, and at most 1 MiB of text."
                            .into(),
                    ));
                }
                // Only fresh owner input reopens a chat. Peer and scheduled input must
                // respect its archive state; identical retries remain no-ops.
                let scheduled = content.trim_start().starts_with(SCHEDULED_PREFIX);
                let owner = sender.is_none() && !scheduled;
                let known = {
                    let uuid = uuid.clone();
                    self.db(move |store| store.reader()?.message(&uuid))
                        .await?
                        .is_some()
                };
                if !known && !owner && self.session().await?.archived {
                    return Err(Error::Invalid(
                        "This chat is archived, so it cannot take new messages. Reopen it first; messages already sent can still be resent unchanged.".into(),
                    ));
                }
                // A scheduled prompt lifts the hold only when it is the sole queued input and
                // the owner has not stopped the chat; otherwise it queues behind the hold.
                let alone = !self.messages.values().any(|m| m.delivery_state == "queued");
                let releases_held = !scheduled || (alone && !self.stopped);
                let id = self.id.clone();
                let accepted = self
                    .db(move |store| {
                        let message = NewMessage {
                            uuid: &uuid,
                            session: &id,
                            role: "user",
                            content: &content,
                            delivery_state: "queued",
                            created_at: now(),
                        };
                        if owner && automated {
                            store.append_automated_reopening(message, &attachments)
                        } else if owner {
                            store.append_reopening(message, &attachments)
                        } else {
                            store.append_with_origin(message, &attachments, sender.as_deref())
                        }
                    })
                    .await
                    .map_err(|error| match error {
                        // The owner's mistake, not a broken store: keep the session running.
                        Error::Store(
                            error @ (boblm_store::Error::AttachmentUnavailable
                            | boblm_store::Error::AttachmentConflict),
                        ) => Error::Invalid(error.to_string()),
                        error => error,
                    })?;
                if accepted && owner {
                    self.archive_after_stop = false;
                }
                if accepted && releases_held {
                    self.release_hold();
                }
                if accepted && !scheduled {
                    self.set_stopped(false).await?;
                }
                self.refresh().await?;
                // Publish durable queued state before the receipt. The outer loop dispatches
                // afterward, so provider startup cannot delay reads of already-saved input.
                self.publish_status().await?;
                Ok(Reply::Accepted(accepted || completion))
            }
            Action::Compact(instructions) => {
                let report = self.self_compact(instructions).await?;
                if report.queued {
                    self.publish_status().await?;
                }
                Ok(Reply::Compact(report))
            }
            Action::Status => Ok(Reply::Status(self.status().await?)),
            Action::Resume => {
                if self.session().await?.archived {
                    return Err(Error::Invalid(
                        "This chat is archived, so its queued messages cannot be released. Reopen it first.".into(),
                    ));
                }
                self.release_hold();
                self.set_stopped(false).await?;
                self.pump().await?;
                Ok(Reply::Done)
            }
            Action::Stop => {
                self.interrupt().await?;
                Ok(Reply::Done)
            }
            Action::Archive(archived) => {
                let id = self.id.clone();
                self.db(move |store| store.set_archived(&id, archived))
                    .await?;
                self.archive_after_stop = archived;
                if !archived {
                    // Reopening is the owner's act: schedules resume, as ADR 0022 says.
                    self.set_stopped(false).await?;
                }
                self.held = true;
                if archived {
                    self.interrupt().await?;
                    if self.stop_deadline.is_none() {
                        self.disconnect().await?;
                    }
                }
                Ok(Reply::Done)
            }
            Action::Retry(uuid) => {
                // A message stored before IDs were lowercased may share its canonical spelling
                // with a twin; the spelling the browser names wins.
                let requested = uuid.clone();
                let uuid = canonical_uuid(&uuid)?;
                let id = self.id.clone();
                let canonical = uuid.clone();
                let message = self
                    .db(move |store| {
                        let reader = store.reader()?;
                        if let Some(message) = reader
                            .message(&requested)?
                            .filter(|message| message.session == id)
                        {
                            return Ok(Some(message));
                        }
                        Ok(reader
                            .message(&canonical)?
                            .filter(|message| message.session == id))
                    })
                    .await?
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "This chat has no message with ID {uuid}, so there is nothing to retry."
                        ))
                    })?;
                if message.delivery_state != "rejected" {
                    let why = match message.delivery_state.as_str() {
                        "uncertain" => {
                            "An unconfirmed message is never resent on its own because the agent may already have received it; send it again as a new message if you want it repeated."
                        }
                        "started" => "The agent has already taken this message.",
                        _ => "It is still waiting for its turn or being delivered.",
                    };
                    return Err(Error::Invalid(format!(
                        "Message {uuid} is {}, and only rejected messages can be retried. {why}",
                        message.delivery_state
                    )));
                }
                if self.session().await?.archived {
                    return Err(Error::Invalid(
                        "This chat is archived, so it cannot retry. Reopen it first.".into(),
                    ));
                }
                let stored = message.uuid.clone();
                self.db(move |store| store.queue_retry(&stored, 0)).await?;
                self.release_hold();
                self.set_stopped(false).await?;
                self.refresh().await?;
                self.pump().await?;
                Ok(Reply::Done)
            }
            Action::Rename {
                name,
                only_if_unnamed,
            } => {
                let name = name.trim().to_owned();
                boblm_protocol::client::check_chat_name(&name, false).map_err(Error::Invalid)?;
                if only_if_unnamed && !self.session().await?.name.trim().is_empty() {
                    return Ok(Reply::Accepted(false));
                }
                let id = self.id.clone();
                self.db(move |store| store.set_name(&id, &name)).await?;
                Ok(Reply::Accepted(true))
            }
            Action::SetModel(model, effort) => {
                let agent = boblm_protocol::client::check_choice(&model, effort.as_deref())
                    .map_err(Error::Invalid)?;
                let current = self.session().await?;
                if current.agent != agent.as_str() {
                    let own = boblm_protocol::client::Agent::parse(&current.agent)
                        .map_or("this", |a| a.label());
                    return Err(Error::Invalid(format!(
                        "This is a {own} chat, so it cannot switch to {model}, a {} model. Start a new chat in the project to use it.",
                        agent.label()
                    )));
                }
                let id = self.id.clone();
                self.db(move |store| store.set_model_effort(&id, &model, effort.as_deref()))
                    .await?;
                Ok(Reply::Done)
            }
            Action::Shutdown => {
                self.consume_ready_events().await?;
                self.refresh().await?;
                Ok(Reply::Done)
            }
        }
    }
    async fn context_usage(&self) -> Result<Option<boblm_protocol::usage::ContextUsage>> {
        let id = self.id.clone();
        let samples = self
            .db(move |store| store.reader()?.usage_samples(&id))
            .await?;
        let context = samples
            .context
            .and_then(|(raw, _)| {
                serde_json::from_str::<boblm_protocol::usage::ContextUsage>(&raw).ok()
            })
            .filter(|context| context.window_tokens > 0);
        Ok(context)
    }
    async fn context_percent(&self) -> Result<Option<f64>> {
        Ok(self
            .context_usage()
            .await?
            .map(|context| 100.0 * context.used_tokens as f64 / context.window_tokens as f64))
    }
    async fn self_compact(&mut self, instructions: Option<String>) -> Result<CompactResult> {
        let usage = self.context_usage().await?;
        let context = CompactContext {
            used: usage.as_ref().map(|value| value.used_tokens),
            window: usage.as_ref().map(|value| value.window_tokens),
            percent: usage
                .as_ref()
                .map(|value| 100.0 * value.used_tokens as f64 / value.window_tokens as f64),
        };
        let session = self.session().await?;
        let refusal = if session.agent == "pi" {
            Some("DeepSeek cache reads are nearly free, so compacting only loses detail. BobLM already auto-compacts Pi when it nears its window. Continue working instead.".to_owned())
        } else if context.percent.is_none() {
            Some("BobLM cannot compact this chat because its context usage is unknown. Continue working until the provider reports its context size.".to_owned())
        } else if context
            .percent
            .is_some_and(|percent| percent < f64::from(boblm_tuning::SELF_COMPACT_MIN_PERCENT))
        {
            Some(format!(
                "BobLM cannot compact this chat because its context is below the {}% minimum. Continue working and finish instead if little work remains.",
                boblm_tuning::SELF_COMPACT_MIN_PERCENT
            ))
        } else if session.archived {
            Some("BobLM cannot compact this chat because it is archived. Reopen it before requesting compaction.".to_owned())
        } else {
            None
        };
        if let Some(message) = refusal {
            return Ok(CompactResult {
                queued: false,
                already_queued: false,
                message,
                context,
            });
        }
        // Include recovered and owner-queued compactions, so a reconnect cannot duplicate one.
        let already_queued = self
            .auto_task
            .as_ref()
            .is_some_and(|task| task.kind == AutoKind::Compact)
            || self.compacting
            || self.messages.values().any(|message| {
                matches!(message.delivery_state.as_str(), "queued" | "dispatching")
                    && message.content.split_whitespace().next() == Some("/compact")
            });
        if !already_queued {
            let content = match instructions {
                Some(instructions) if !instructions.is_empty() => {
                    format!("/compact {instructions}")
                }
                _ => "/compact".to_owned(),
            };
            if content.len() > boblm_protocol::client::MAX_BODY_BYTES {
                return Ok(CompactResult {
                    queued: false, already_queued: false,
                    message: "BobLM cannot queue compaction because its instructions exceed 1 MiB. Shorten the instructions and try again.".to_owned(),
                    context,
                });
            }
            self.enqueue_auto(&content, AutoKind::Compact, true).await?;
        }
        Ok(CompactResult {
            queued: true,
            already_queued,
            message: if already_queued {
                "Compaction is already queued or running; the first instructions are retained. End your turn at the phase boundary. Stop and held queues still prevent dispatch."
            } else {
                "Compaction is queued for after this turn and all outstanding acknowledgments settle. End your turn at the phase boundary. Stop and held queues still prevent dispatch."
            }.to_owned(),
            context,
        })
    }
    async fn enqueue_auto(
        &mut self,
        content: &str,
        kind: AutoKind,
        resume_after: bool,
    ) -> Result<()> {
        if self.auto_task.is_some() {
            return Ok(());
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let stored_uuid = uuid.clone();
        let id = self.id.clone();
        let content = content.to_owned();
        let created_at = now();
        self.db(move |store| {
            store.append_with_origin(
                NewMessage {
                    uuid: &stored_uuid,
                    session: &id,
                    role: "user",
                    content: &content,
                    delivery_state: "queued",
                    created_at,
                },
                &[],
                None,
            )
        })
        .await?;
        self.auto_task = Some(AutoTask {
            uuid,
            kind,
            resume_after,
            dispatched: false,
        });
        self.refresh().await
    }
    async fn mark_provider_exhaustion(&self, tpm: bool) -> Result<()> {
        let session = self.session().await?;
        let id = self.id.clone();
        let step = self
            .db(move |store| store.reader()?.peer_spawn_receipt(&id))
            .await?
            .and_then(|receipt| receipt.resolved)
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| value["step"].as_str().and_then(boblm_tuning::StepId::parse));
        let Some(step) = step else {
            return Ok(());
        };
        let account = step.account();
        let matches = match account {
            boblm_tuning::AccountKind::OpenAiApi => {
                session.agent == "codex"
                    && session.account_key.as_deref() == Some(boblm_tuning::CODEX_ACCOUNT_KEY_API)
            }
            boblm_tuning::AccountKind::ClaudeApi => {
                session.agent == "claude"
                    && session.account_key.as_deref()
                        == Some(boblm_tuning::CLAUDE_ACCOUNT_KEY_API_A)
            }
            boblm_tuning::AccountKind::DeepSeek => session.agent == "pi",
            _ => false,
        };
        if !matches {
            return Ok(());
        }
        let now = now() / 1_000;
        if tpm && step != boblm_tuning::StepId::ReviewerOpenAiApi {
            return Ok(());
        }
        let until = match account {
            boblm_tuning::AccountKind::OpenAiApi if tpm => now.saturating_add(
                i64::try_from(boblm_tuning::OPENAI_API_RATE_LIMIT_COOLDOWN.as_secs())
                    .unwrap_or(i64::MAX),
            ),
            boblm_tuning::AccountKind::OpenAiApi => boblm_tuning::openai_api_reset_at(now),
            boblm_tuning::AccountKind::ClaudeApi => boblm_tuning::claude_api_reset_at(now),
            boblm_tuning::AccountKind::DeepSeek => now.saturating_add(
                i64::try_from(boblm_tuning::ROLE_EXHAUSTION_RETRY.as_secs()).unwrap_or(i64::MAX),
            ),
            _ => return Ok(()),
        };
        let reason = if tpm {
            "OpenAI API reached its tokens-per-minute rate limit. New API reviewers will wait for the cooldown to end.".to_owned()
        } else {
            format!(
                "The provider rejected the {} step for quota or billing.",
                step.label()
            )
        };
        self.db(move |store| store.mark_role_step_exhausted(step, until, now, &reason))
            .await?;
        Ok(())
    }
    async fn maybe_auto_compact(&mut self, overflow: bool, succeeded: bool) -> Result<()> {
        if self.auto_task.is_some() || self.session().await?.agent != "pi" {
            return Ok(());
        }
        if overflow {
            if !self.overflow_resume_used {
                self.overflow_resume_used = true;
                self.enqueue_auto("/compact", AutoKind::Compact, true)
                    .await?;
            }
            return Ok(());
        }
        if succeeded {
            self.overflow_resume_used = false;
        }
        if self
            .context_percent()
            .await?
            .is_some_and(|percent| percent >= f64::from(boblm_tuning::PI_AUTO_COMPACT_PERCENT))
        {
            self.enqueue_auto("/compact", AutoKind::Compact, false)
                .await?;
        }
        Ok(())
    }
    /// Drain provider activity before making a dispatch or quiet-stop decision.
    async fn consume_ready_events(&mut self) -> Result<()> {
        loop {
            let ready = self.driver.as_mut().and_then(|driver| {
                use std::task::{Context, Poll, Waker};
                let mut cx = Context::from_waker(Waker::noop());
                match driver.next_event().as_mut().poll(&mut cx) {
                    Poll::Ready(event) => Some(event),
                    Poll::Pending => None,
                }
            });
            let Some(event) = ready else {
                return Ok(());
            };
            if self.consume_event_batch(event).await? < boblm_tuning::MAX_EVENT_BATCH {
                return Ok(());
            }
        }
    }
    async fn pump(&mut self) -> Result<()> {
        loop {
            // A queued compaction marker wins over concurrently accepted input.
            self.consume_ready_events().await?;
            if self.compacting
                || self.held
                || self.stop_deadline.is_some()
                || self.session().await?.archived
            {
                break;
            }
            let idle = !self.active && self.pending.is_empty();
            let next = if let Some(auto) = &self.auto_task {
                idle.then(|| {
                    self.messages
                        .values()
                        .find(|m| m.uuid == auto.uuid && m.delivery_state == "queued")
                        .cloned()
                })
                .flatten()
            } else {
                self.messages
                    .values()
                    .find(|m| {
                        m.delivery_state == "queued"
                            && (idle || (m.retry_count == 0 && !waits_for_idle(&m.content)))
                    })
                    .cloned()
            };
            let Some(message) = next else { break };
            // A stale message must never reach the agent, even if its expiry wake has not run.
            if message.created_at <= stale_cutoff() {
                self.expire_stale().await?;
                continue;
            }
            if self.driver.is_none() {
                let session = self.session().await?;
                let account_id = self.factory.account_id(&session);
                let session_id = self.id.clone();
                let stored_account = account_id.clone();
                self.db(move |store| {
                    store.set_session_account(&session_id, stored_account.as_deref())
                })
                .await?;
                match self.factory.spawn(&session).await {
                    Ok(mut driver) => {
                        driver.set_usage_poll_gate(usage_poll_gate(
                            self.store.clone(),
                            session.agent.clone(),
                            account_id,
                        ));
                        self.jobs = match session.agent.as_str() {
                            "codex" => Some(boblm_protocol::jobs::JobSnapshot::default()),
                            // Pi has no background job tool, so it never has tracked jobs.
                            "claude" | "pi" => Some(boblm_protocol::jobs::JobSnapshot {
                                tasks: Vec::new(),
                                available: true,
                            }),
                            _ => None,
                        };
                        self.driver = Some(driver);
                        self.applied_choice = None;
                    }
                    Err(error) => {
                        let agent = agent_label(&session.agent);
                        self.held = true;
                        self.record(
                            "driver_error",
                            serde_json::json!({"message": format!("{agent} could not be started, so the queued messages are held (cause: {error}). Send a message or press Resume to try again.")}),
                        )
                        .await?;
                        return Ok(());
                    }
                }
            }
            if idle {
                let session = self.session().await?;
                let (model, effort) = (session.model.clone(), session.effort.clone());
                if self.applied_choice.as_ref() != Some(&(model.clone(), effort.clone())) {
                    if let Err(error) = self
                        .driver
                        .as_mut()
                        .ok_or(Error::Closed)?
                        .configure(&model, effort.as_deref())
                        .await
                    {
                        self.record(
                            "model_error",
                            serde_json::json!({"message": format!("The chosen model or effort could not be applied, so nothing was sent and the queued messages are held. {error}")}),
                        )
                        .await?;
                        self.disconnect().await?;
                        return Ok(());
                    }
                    let (id, model_name, effort_name) =
                        (self.id.clone(), model.clone(), effort.clone());
                    self.db(move |store| {
                        store.set_applied_choice(&id, &model_name, effort_name.as_deref())
                    })
                    .await?;
                    self.applied_choice = Some((model, effort));
                }
            }
            let uuid = message.uuid.clone();
            let human = self
                .db(move |store| store.reader()?.human_input(&uuid))
                .await?;
            let priority = if idle {
                None
            } else if !human && self.session().await?.agent == "claude" {
                Some(Priority::Next)
            } else {
                Some(Priority::Now)
            };
            let awaiting_tool_boundary = !idle
                && self
                    .driver
                    .as_ref()
                    .is_some_and(|driver| driver.steers_wait_for_tool_boundary());
            let (uuid, cutoff) = (message.uuid.clone(), stale_cutoff());
            if !self
                .db(move |store| store.dispatch(&uuid, now(), cutoff))
                .await?
            {
                // It aged past the limit while the driver started; it is expired, never sent.
                self.after_expiry().await?;
                continue;
            }
            if let Some(task) = &mut self.auto_task
                && task.uuid == message.uuid
            {
                task.dispatched = true;
            }
            self.pending.insert(
                message.uuid.clone(),
                Pending {
                    deferred_to_boundary: priority == Some(Priority::Next),
                    awaiting_tool_boundary,
                    queued: false,
                    deadline: Instant::now() + boblm_tuning::ACK_DEADLINE,
                    result_seen: false,
                },
            );
            self.prompt_suggestion = None;
            self.active = true;
            if message.content.split_whitespace().next() == Some("/compact") {
                // Protect the command before its asynchronous started notification.
                self.compacting = true;
            }
            let uuid = message.uuid.clone();
            let attachments = self
                .db(move |store| store.reader()?.message_attachments(&uuid))
                .await?;
            let text = dispatch_text(&message.content, &attachments);
            // Readers can observe dispatching while the child's input pipe is blocked.
            self.refresh().await?;
            self.publish_status().await?;
            let outcome = tokio::time::timeout(
                boblm_tuning::SEND_WRITE_TIMEOUT,
                self.driver.as_mut().ok_or(Error::Closed)?.send_with_origin(
                    &message.uuid,
                    &text,
                    priority,
                    human,
                ),
            )
            .await;
            let failure = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some(format!(
                    "the send did not finish within {} seconds",
                    boblm_tuning::SEND_WRITE_TIMEOUT.as_secs()
                )),
            };
            if let Some(cause) = failure {
                self.record("driver_error", serde_json::json!({"message": format!("Message {} could not be confirmed because {cause}. The agent was stopped and unconfirmed messages are held; inspect the chat before resuming.", message.uuid)})).await?;
                self.disconnect().await?;
                return Ok(());
            }
        }
        Ok(())
    }
    async fn consume_event_batch(&mut self, first: Option<Event>) -> Result<usize> {
        let Some(first) = first else {
            self.finish_stop(true).await?;
            self.disconnect().await?;
            return Ok(0);
        };
        let (batch, closed) = self.drain_ready_events(first);
        let count = batch.len();
        let records_usage = batch.iter().any(|event| match event {
            Event::TokenUsage { .. } => true,
            Event::Assistant { message } => message.get("usage").is_some_and(|u| u.is_object()),
            _ => false,
        });
        self.record_agent_events(&batch).await?;
        for event in batch {
            let stopped = self.stop_deadline.is_some() && matches!(event, Event::Result { .. });
            self.apply(event).await?;
            if stopped {
                self.finish_stop(false).await?;
            }
        }
        if records_usage {
            // The actor serializes this with parent input. A round cannot change between
            // the durable claim and interrupt, and the ordinary stop path holds input.
            let id = self.id.clone();
            if self
                .db(move |store| store.pause_runaway_subagent_round(&id, now()))
                .await?
            {
                self.interrupt().await?;
            }
        }
        if closed {
            self.finish_stop(true).await?;
            self.disconnect().await?;
        }
        Ok(count)
    }
    /// Take every event the driver already has queued, up to a bound, so a burst of
    /// streamed output is stored in one transaction. The flag reports a closed stream.
    fn drain_ready_events(&mut self, first: Event) -> (Vec<Event>, bool) {
        use std::task::{Context, Poll, Waker};
        let mut batch = vec![first];
        let Some(driver) = self.driver.as_mut() else {
            return (batch, false);
        };
        let mut cx = Context::from_waker(Waker::noop());
        while batch.len() < boblm_tuning::MAX_EVENT_BATCH {
            let mut next = driver.next_event();
            match next.as_mut().poll(&mut cx) {
                Poll::Ready(Some(event)) => batch.push(event),
                Poll::Ready(None) => return (batch, true),
                Poll::Pending => break,
            }
        }
        (batch, false)
    }
    async fn record_agent_events(&self, events: &[Event]) -> Result<()> {
        let payloads = events
            .iter()
            .filter(|event| !matches!(event, Event::PromptSuggestion { .. }))
            .map(|event| serde_json::to_value(event).map_err(std::io::Error::other))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let id = self.id.clone();
        self.db(move |store| store.record_events(&id, "agent_event", payloads, now()))
            .await
    }
    /// Apply one already recorded event to delivery state.
    async fn apply(&mut self, event: Event) -> Result<()> {
        // Only lifecycle events change message rows; reloading history for every
        // streamed delta would slow the consumer as a session grows.
        let changes_messages = matches!(event, Event::CommandLifecycle { .. });
        let tools_changed = self.track_tool_events(&event);
        if let Event::Assistant { message } = &event
            && let Some(text) = driver_error_text(message)
        {
            self.provider_rejection |= boblm_tuning::is_provider_exhaustion(&text);
            if boblm_tuning::is_api_tpm_rate_limit(&text) {
                // Record cooldown immediately, even if Codex is about to retry.
                self.mark_provider_exhaustion(true).await?;
            }
        }
        match event {
            Event::PromptSuggestion { suggestion } => {
                if !self.active {
                    self.prompt_suggestion = suggestion
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_owned);
                }
            }
            Event::BackgroundJobs { mut snapshot } => {
                if !snapshot.available {
                    // An incomplete scan cannot establish that any job ended.
                    snapshot.tasks = self
                        .jobs
                        .as_ref()
                        .map(|j| j.tasks.clone())
                        .unwrap_or_default();
                } else if let Some(previous) = &self.jobs
                    && !self
                        .driver
                        .as_ref()
                        .is_some_and(|driver| driver.wakes_itself_on_background_job_completion())
                {
                    let finished: Vec<_> = previous
                        .tasks
                        .iter()
                        .filter(|job| {
                            job.agent_key.is_none()
                                && !snapshot.tasks.iter().any(|current| current.id == job.id)
                        })
                        .cloned()
                        .collect();
                    for job in finished {
                        // Inventory removal proves only that the provider no longer
                        // lists the command. Its exit status and output need checking.
                        let mut content = format!(
                            "Background command is no longer listed as running: {}\nJob: {}\nCheck its output and exit status, then continue your work.",
                            job.command, job.id
                        );
                        if content.len() > boblm_tuning::MAX_MESSAGE_BYTES {
                            let mut end = boblm_tuning::MAX_MESSAGE_BYTES;
                            while !content.is_char_boundary(end) {
                                end -= 1;
                            }
                            content.truncate(end);
                        }
                        let id = self.id.clone();
                        let uuid = uuid::Uuid::new_v4().to_string();
                        self.db(move |store| {
                            store.append_with_origin(
                                NewMessage {
                                    uuid: &uuid,
                                    session: &id,
                                    role: "user",
                                    content: &content,
                                    delivery_state: "queued",
                                    created_at: now(),
                                },
                                &[],
                                Some(&id),
                            )
                        })
                        .await?;
                    }
                    // Keep Stop, archive and uncertainty holds intact. The ordinary
                    // pump starts an idle turn or steers at the next tool boundary.
                    self.refresh().await?;
                }
                self.jobs = Some(snapshot);
            }
            Event::System {
                subtype: Some(subtype),
                session_id: Some(id),
                ..
            } if subtype == "init" => {
                let session = self.id.clone();
                self.db(move |store| store.set_agent_session(&session, &id))
                    .await?;
            }
            Event::System {
                subtype, status, ..
            } if matches!(subtype.as_deref(), Some("compact_started" | "turn_started"))
                || status.as_deref() == Some("compacting") =>
            {
                if subtype.as_deref() == Some("compact_started")
                    || status.as_deref() == Some("compacting")
                {
                    self.compacting = true;
                }
                // A started turn or a compaction can keep the agent silent for minutes (Codex
                // compacts a large thread on the provider side without saying so).
                for pending in self.pending.values_mut() {
                    pending.deadline = pending
                        .deadline
                        .max(Instant::now() + boblm_tuning::ACK_DEADLINE_DURING_COMPACTION);
                }
            }
            Event::System {
                subtype: Some(subtype),
                status,
                ..
            } if subtype == "compact_boundary"
                || (subtype == "status" && status.is_none() && self.compacting) =>
            {
                self.compacting = false;
                // Compaction ended: further silence is suspicious again.
                for pending in self.pending.values_mut() {
                    pending.deadline = Instant::now()
                        + if pending.result_seen {
                            boblm_tuning::ACK_DEADLINE_AFTER_RESULT
                        } else {
                            boblm_tuning::ACK_DEADLINE
                        };
                }
            }
            Event::CommandLifecycle {
                command_uuid,
                state,
            } => {
                let message = self
                    .messages
                    .values()
                    .find(|m| m.uuid == command_uuid)
                    .cloned();
                if let Some(message) = message {
                    match state {
                        LifecycleState::Queued => {
                            if let Some(pending) = self.pending.get_mut(&command_uuid) {
                                pending.queued = true;
                                // The provider accepted the input but will consume it
                                // only at a later boundary, including for Claude now.
                                pending.awaiting_tool_boundary = true;
                            }
                        }
                        LifecycleState::Started | LifecycleState::Completed => {
                            let pending = self.pending.remove(&command_uuid);
                            if matches!(
                                message.delivery_state.as_str(),
                                "dispatching" | "uncertain"
                            ) {
                                self.transition(&command_uuid, &message.delivery_state, "started")
                                    .await?;
                            }
                            if let Some(pending) = pending {
                                if state == LifecycleState::Started || !pending.result_seen {
                                    self.prompt_suggestion = None;
                                    self.active = true;
                                    self.consumed_active = true;
                                } else if self.pending.is_empty() && !self.consumed_active {
                                    self.active = false;
                                }
                            } else if state == LifecycleState::Started
                                && message.delivery_state == "uncertain"
                            {
                                // A late acknowledgment proves Claude is running this turn.
                                self.prompt_suggestion = None;
                                self.active = true;
                                self.consumed_active = true;
                            }
                        }
                        LifecycleState::Cancelled
                        | LifecycleState::Discarded
                        | LifecycleState::Refused
                            if message.delivery_state == "dispatching" =>
                        {
                            let automatic = self
                                .auto_task
                                .as_ref()
                                .is_some_and(|task| task.uuid == command_uuid);
                            if automatic {
                                self.auto_task = None;
                            }
                            self.pending.remove(&command_uuid);
                            if message.content.split_whitespace().next() == Some("/compact") {
                                self.compacting = false;
                            }
                            let uuid = command_uuid.clone();
                            let retry = (!automatic && message.retry_count < 2)
                                .then_some(message.retry_count + 1);
                            self.db(move |store| store.reject(&uuid, retry)).await?;
                            if self.pending.is_empty() && !self.consumed_active {
                                self.active = false;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::Result { subtype, is_error } => {
                self.outstanding_tools.clear();
                if is_error == Some(true) && self.provider_rejection {
                    self.mark_provider_exhaustion(false).await?;
                }
                self.provider_rejection = false;
                let succeeded =
                    is_error != Some(true) && subtype.as_deref() != Some("context_overflow");
                // A self-compaction can be queued during the turn producing this result.
                // Retain it until a result follows dispatch of the automated command itself.
                let auto_dispatched = self.auto_task.as_ref().is_some_and(|task| task.dispatched);
                if let Some(task) = auto_dispatched.then(|| self.auto_task.take()).flatten() {
                    match task.kind {
                        AutoKind::Compact if task.resume_after && succeeded && !self.stopped => {
                            self.enqueue_auto("boblm: continue", AutoKind::Resume, false)
                                .await?;
                        }
                        AutoKind::Resume if succeeded => {
                            self.overflow_resume_used = false;
                        }
                        _ => {}
                    }
                } else {
                    self.maybe_auto_compact(
                        subtype.as_deref() == Some("context_overflow"),
                        succeeded,
                    )
                    .await?;
                }
                self.compacting = false;
                self.active = false;
                self.consumed_active = false;
                for pending in self.pending.values_mut() {
                    let after_result = Instant::now() + boblm_tuning::ACK_DEADLINE_AFTER_RESULT;
                    pending.deadline = if pending.queued && !pending.result_seen {
                        after_result
                    } else {
                        pending.deadline.min(after_result)
                    };
                    pending.result_seen = true;
                }
            }
            _ => {}
        }
        if tools_changed && self.outstanding_tools.is_empty() {
            // The tool boundary is the first chance these steers can be consumed.
            for pending in self.pending.values_mut() {
                if pending.awaiting_tool_boundary {
                    pending.deadline = Instant::now() + boblm_tuning::ACK_DEADLINE;
                }
            }
        }
        if changes_messages {
            self.refresh().await?;
        }
        Ok(())
    }
    fn track_tool_events(&mut self, event: &Event) -> bool {
        let mut changed = false;
        match event {
            Event::Assistant { message } => {
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use"
                        && let Some(id) = block["id"].as_str()
                    {
                        changed |= self.outstanding_tools.insert(id.to_owned());
                    }
                }
            }
            Event::User { message } => {
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result"
                        && let Some(id) = block["tool_use_id"].as_str()
                    {
                        changed |= self.outstanding_tools.remove(id);
                    }
                }
            }
            _ => {}
        }
        changed
    }
    async fn hold_pending(&mut self) -> Result<()> {
        for uuid in self.pending.keys().cloned().collect::<Vec<_>>() {
            self.transition(&uuid, "dispatching", "uncertain").await?;
        }
        self.pending.clear();
        self.active = false;
        self.consumed_active = false;
        self.held = true;
        self.refresh().await
    }
    async fn set_stopped(&mut self, stopped: bool) -> Result<()> {
        if self.stopped != stopped {
            let id = self.id.clone();
            self.db(move |store| store.set_runner_stopped(&id, stopped))
                .await?;
            self.stopped = stopped;
        }
        Ok(())
    }
    async fn interrupt(&mut self) -> Result<()> {
        self.set_stopped(true).await?;
        if self.stop_deadline.is_some() {
            self.held = true;
            return Ok(());
        }
        let active = self.active || !self.pending.is_empty();
        self.record("session_stopping", serde_json::json!({}))
            .await?;
        self.hold_pending().await?;
        if let Some(driver) = &mut self.driver {
            let written =
                tokio::time::timeout(boblm_tuning::INTERRUPT_WRITE_TIMEOUT, driver.abort()).await;
            if !matches!(written, Ok(Ok(()))) {
                let agent = agent_label(&self.session().await?.agent);
                self.record(
                    "driver_error",
                    serde_json::json!({"message": format!("{agent} did not accept the stop request within {} seconds, or its input pipe had closed, so its process was shut down instead. Unconfirmed messages are held, not resent.", boblm_tuning::INTERRUPT_WRITE_TIMEOUT.as_secs())}),
                )
                .await?;
                return self.disconnect().await;
            }
        }
        if active && self.driver.is_some() {
            self.stop_deadline = Some(Instant::now() + boblm_tuning::STOP_RESULT_DEADLINE);
        } else {
            self.record("session_stopped", serde_json::json!({}))
                .await?;
        }
        Ok(())
    }
    async fn finish_stop(&mut self, kill: bool) -> Result<()> {
        if self.stop_deadline.take().is_none() {
            return Ok(());
        }
        if kill || self.archive_after_stop {
            self.disconnect().await?;
        }
        self.record("session_stopped", serde_json::json!({}))
            .await?;
        // Stop/archive callers can inspect the published snapshot as soon as they
        // receive their reply, including acknowledgments drained with the result.
        self.publish_status().await?;
        for reply in self.stop_replies.drain(..) {
            let _ = reply.send(Ok(Reply::Done));
        }
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.prompt_suggestion = None;
        self.compacting = false;
        self.auto_task = None;
        self.outstanding_tools.clear();
        self.provider_rejection = false;
        self.jobs = None;
        self.hold_pending().await?;
        self.applied_choice = None;
        if let Some(mut driver) = self.driver.take()
            && let Err(error) = driver.shutdown().await
        {
            eprintln!(
                "session={}: The agent could not shut down cleanly: {error}. Check the server process.",
                self.id
            );
        }
        Ok(())
    }
}
/// Only the provider's own error sentence, not an agent answer that happens to mention quota.
fn driver_error_text(message: &serde_json::Value) -> Option<String> {
    let mut text = String::new();
    for block in message["content"].as_array().into_iter().flatten() {
        if block["type"] == "text"
            && let Some(part) = block["text"].as_str()
        {
            text.push_str(part);
            text.push('\n');
        }
    }
    let lower = text.trim_start().to_ascii_lowercase();
    (lower.starts_with("pi stopped this turn with an error:")
        || lower.starts_with("codex stopped this turn with an error:")
        || lower.starts_with("codex reported an error:")
        || lower.starts_with("claude reported an error:"))
    .then_some(text)
}

/// What Claude receives: the owner's text, then where each attached file lives.
fn dispatch_text(content: &str, attachments: &[boblm_store::Attachment]) -> String {
    if attachments.is_empty() {
        return content.to_owned();
    }
    let mut text = String::new();
    if !content.trim().is_empty() {
        text.push_str(content);
        text.push_str("\n\n");
    }
    text.push_str("Attached files:");
    for a in attachments {
        let size = match a.size {
            n if n >= 1 << 20 => format!("{} MiB", n >> 20),
            n if n >= 1 << 10 => format!("{} KiB", n >> 10),
            n => format!("{n} B"),
        };
        text.push_str(&format!(
            "\n- {} ({}, {size}, original name {:?})",
            a.path, a.content_type, a.name
        ));
    }
    text
}
/// The child skips its periodic poll while any chat on the account confirmed the row recently.
fn usage_poll_gate(
    store: Store,
    provider: String,
    account_id: Option<String>,
) -> Arc<dyn Fn() -> bool + Send + Sync> {
    Arc::new(move || {
        let Some(account_id) = account_id.as_deref() else {
            return false;
        };
        let Ok(reader) = store.reader() else {
            return false;
        };
        let Ok(Some(usage)) = reader.account_usage(&provider, account_id) else {
            return false;
        };
        let window =
            i64::try_from(boblm_tuning::USAGE_POLL_INTERVAL.as_millis()).unwrap_or(i64::MAX);
        usage
            .latest_observed_at()
            .is_some_and(|at| at >= now() - window)
    })
}

/// Messages created at or before this Unix millisecond time are stale.
fn stale_cutoff() -> i64 {
    now() - boblm_tuning::QUEUED_MESSAGE_MAX_AGE.as_millis() as i64
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_poll_gate_skips_only_a_fresh_account_confirmation() {
        let temp =
            tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")).unwrap();
        let store = Store::open(temp.path().join("gate.sqlite")).unwrap();
        store.create_session("fresh", "/local", 1).unwrap();
        store
            .set_session_account("fresh", Some("account-fresh"))
            .unwrap();
        store.create_session("old", "/local", 1).unwrap();
        store
            .set_session_account("old", Some("account-old"))
            .unwrap();
        let fresh = usage_poll_gate(store.clone(), "claude".into(), Some("account-fresh".into()));
        assert!(!fresh(), "an account with no row still polls");
        store
            .record_event(
                "fresh",
                "agent_event",
                serde_json::json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}}),
                now(),
            )
            .unwrap();
        assert!(fresh());
        let old = usage_poll_gate(store.clone(), "claude".into(), Some("account-old".into()));
        let stale =
            now() - i64::try_from(boblm_tuning::USAGE_POLL_INTERVAL.as_millis()).unwrap() - 1;
        store
            .record_event(
                "old",
                "agent_event",
                serde_json::json!({"type":"usage","weekly":{"usedPercent":42,"resetsAt":2000000000}}),
                stale,
            )
            .unwrap();
        assert!(!old(), "an old confirmation polls again");
    }
}
