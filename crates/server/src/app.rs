use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use boblm_protocol::client::*;
use boblm_runner::{DriverFactory, RunningSession, SessionHandle, events::wire_status};
use boblm_store::{Session, Store};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tower_http::{
    compression::{
        CompressionLayer,
        predicate::{Predicate, SizeAbove},
    },
    services::ServeDir,
};

pub type ApiResult<T> = Result<Json<T>, ApiError>;
#[derive(Debug)]
pub struct ApiError(
    pub(crate) StatusCode,
    pub(crate) &'static str,
    pub(crate) String,
);
impl ApiError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request", message.into())
    }
    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        Self::internal_while("finish that request", error)
    }
    /// A fault on the server. Says what it was doing and why it failed; the owner is the
    /// only reader, so the cause is included rather than hidden.
    pub(crate) fn internal_while(doing: &str, error: impl std::fmt::Display) -> Self {
        eprintln!("server: could not {doing}: {error}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            format!(
                "The server could not {doing}: {error}. This is a fault on the server, not a problem with your input; the server log has the details."
            ),
        )
    }
}
impl std::fmt::Display for ApiError {
    /// The sentence the owner reads; never a debug dump of the error.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.2)
    }
}
impl From<boblm_runner::Error> for ApiError {
    fn from(error: boblm_runner::Error) -> Self {
        match error {
            boblm_runner::Error::Invalid(message) => Self::invalid(message),
            boblm_runner::Error::Store(boblm_store::Error::MessageConflict) => Self(
                StatusCode::CONFLICT,
                "message_conflict",
                "That message ID was already used for different text, attachments or a different chat. To retry a message, resend it unchanged with the same ID; for a new message, use a new ID.".into(),
            ),
            boblm_runner::Error::Store(error) => Self::from(error),
            boblm_runner::Error::Closed => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "This chat's task is not running right now, which usually means the server is shutting down. Try again in a moment.".into(),
            ),
            error => Self::internal(error),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(ErrorInfo {
                code: self.1.into(),
                message: self.2,
            }),
        )
            .into_response()
    }
}

#[derive(Default)]
struct PeerOperations {
    closing: bool,
    tasks: tokio::task::JoinSet<()>,
}
/// Compress only JSON replies. Event streams must reach the browser as they are produced, and files
/// are already in whatever form they were uploaded in.
#[derive(Clone, Copy)]
struct JsonOnly;
impl Predicate for JsonOnly {
    fn should_compress<B>(&self, response: &axum::http::Response<B>) -> bool
    where
        B: axum::body::HttpBody,
    {
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("application/json"))
    }
}
pub struct App {
    pub(crate) role_routing: tokio::sync::Mutex<()>,
    pub(crate) question_delivery: tokio::sync::Mutex<()>,
    pub(crate) store: Store,
    factory: Arc<dyn DriverFactory>,
    pub(crate) external_runners: bool,
    sessions: Mutex<BTreeMap<String, RunningSession>>,
    token: String,
    peer_operations: Mutex<PeerOperations>,
    scheduler: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) notification_task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) notifications: std::sync::Mutex<crate::notifications::Notifications>,
    pub(crate) browser_reloads: std::sync::Mutex<crate::browser_reload::BrowserReloads>,
    /// Held while a due occurrence is settled and while a schedule is cancelled.
    pub(crate) schedule_claim: tokio::sync::Mutex<()>,
    role_subagents_enabled: std::sync::atomic::AtomicBool,
    pub(crate) subagent_prune_at: std::sync::atomic::AtomicI64,
}
impl App {
    pub async fn open(
        store: Store,
        factory: Arc<dyn DriverFactory>,
        token: String,
    ) -> Result<Arc<Self>, boblm_runner::Error> {
        if token.len() < 16 || token.bytes().any(|b| b.is_ascii_whitespace()) {
            return Err(boblm_runner::Error::Invalid(
                "bearer token must contain at least 16 non-whitespace bytes".into(),
            ));
        }
        let source = store.clone();
        let existing = tokio::task::spawn_blocking(move || source.reader()?.sessions()).await??;
        let mut sessions = BTreeMap::new();
        for session in existing {
            sessions.insert(
                session.id.clone(),
                RunningSession::start(store.clone(), &session.id, factory.clone()).await?,
            );
        }
        Ok(Arc::new(Self {
            role_routing: tokio::sync::Mutex::new(()),
            question_delivery: tokio::sync::Mutex::new(()),
            store,
            external_runners: factory.external_runners(),
            factory,
            sessions: Mutex::new(sessions),
            token,
            peer_operations: Mutex::new(PeerOperations::default()),
            scheduler: std::sync::Mutex::new(None),
            notification_task: std::sync::Mutex::new(None),
            notifications: std::sync::Mutex::new(Default::default()),
            browser_reloads: std::sync::Mutex::new(Default::default()),
            schedule_claim: tokio::sync::Mutex::new(()),
            role_subagents_enabled: std::sync::atomic::AtomicBool::new(
                boblm_tuning::ROLE_SUBAGENTS_ENABLED,
            ),
            subagent_prune_at: std::sync::atomic::AtomicI64::new(0),
        }))
    }
    pub(crate) fn role_subagents_enabled(&self) -> bool {
        self.role_subagents_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Test seam: the production default is `ROLE_SUBAGENTS_ENABLED`.
    #[doc(hidden)]
    pub fn set_role_subagents_enabled_for_tests(&self, enabled: bool) {
        self.role_subagents_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) async fn spawn_peer(
        self: &Arc<Self>,
        request: boblm_store::PeerSpawn,
    ) -> Result<(SessionInfo, bool), String> {
        let mut operations = self.peer_operations.lock().await;
        if operations.closing {
            return Err("Server is shutting down".into());
        }
        while let Some(outcome) = operations.tasks.try_join_next() {
            if let Err(error) = outcome {
                eprintln!("peer spawn task: {error}");
            }
        }
        if operations.tasks.len() >= boblm_tuning::MAX_PEER_SPAWNS_IN_FLIGHT {
            return Err(format!(
                "The peer could not be created because the limit of {} concurrent peer creations has been reached. Wait for current creations to finish, then retry with the same requestId.",
                boblm_tuning::MAX_PEER_SPAWNS_IN_FLIGHT
            ));
        }
        let (reply, receive) = tokio::sync::oneshot::channel();
        let app = self.clone();
        operations.tasks.spawn(async move {
            let outcome = async {
                let id = request.request_id.clone();
                let store = app.store.clone();
                let mut registry = app.sessions.lock().await;
                let created = tokio::task::spawn_blocking(move || store.spawn_peer(request))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                if !registry.contains_key(&id) {
                    let runner = RunningSession::start(app.store.clone(), &id, app.factory.clone())
                        .await
                        .map_err(|e| format!(
                            "Spawn {id} persisted but Runner is unavailable; inspect before retrying: {e}"
                        ))?;
                    registry.insert(id.clone(), runner);
                }
                let handle = registry.get(&id).ok_or("Spawned session missing")?.handle();
                drop(registry);
                // Only a new creation releases its first input. Retried receipts
                // never release a recovered or explicitly held queue.
                if created {
                    handle.resume().await.map_err(|e| format!(
                        "Spawn {id} persisted; inspect before retrying: {e}"
                    ))?;
                }
                let store = app.store.clone();
                let lookup = id.clone();
                let session = tokio::task::spawn_blocking(move || store.reader()?.sessions())
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|s| s.id == lookup)
                    .ok_or("Spawned session missing")?;
                let info = app.info(session).await.map_err(|_| format!(
                    "Spawn {id} persisted; could not read its state"
                ))?;
                Ok((info, created))
            }.await;
            let _ = reply.send(outcome);
        });
        drop(operations);
        receive.await.map_err(|_| {
            "Peer creation task stopped; inspect its request ID before retrying".to_string()
        })?
    }
    pub(crate) fn keep_scheduler(&self, task: tokio::task::JoinHandle<()>) {
        if let Ok(mut slot) = self.scheduler.lock()
            && let Some(old) = slot.replace(task)
        {
            old.abort();
        }
    }
    pub(crate) fn agent_token(&self, id: &str) -> String {
        crate::mcp::session_token(&self.token, id)
    }
    pub fn router(self: &Arc<Self>, root: Option<PathBuf>) -> Router {
        let commands = Router::new()
            .route("/commands/listSessions", post(list_sessions))
            .route("/commands/runtimeCapabilities", post(runtime_capabilities))
            .route("/commands/chatHistory", post(chat_history))
            .route(
                "/commands/subagentHistory",
                post(crate::subagent_history::history),
            )
            .route("/commands/addProject", post(add_project))
            .route("/commands/renameProject", post(rename_project))
            .route("/commands/listDirectories", post(list_directories))
            .route("/commands/removeProject", post(remove_project))
            .route("/commands/reorderProjects", post(reorder_projects))
            .route("/commands/reorderChats", post(reorder_chats))
            .route("/commands/createSession", post(create_session))
            .route("/commands/renameSession", post(rename_session))
            .route("/commands/deleteSession", post(delete_session))
            .route("/commands/submitSecret", post(submit_secret))
            .route("/commands/pushConfig", post(crate::notifications::config))
            .route(
                "/commands/subscribePush",
                post(crate::notifications::subscribe),
            )
            .route(
                "/commands/unsubscribePush",
                post(crate::notifications::unsubscribe),
            )
            .route(
                "/commands/chatPresence",
                post(crate::notifications::presence),
            )
            .route("/commands/listQuestions", post(crate::questions::list))
            .route("/commands/answerQuestion", post(crate::questions::answer))
            .route(
                "/commands/discussQuestionLater",
                post(crate::questions::discuss_later),
            )
            .route("/commands/sendMessage", post(send_message))
            .route("/commands/setModel", post(set_model))
            .route("/commands/retryMessage", post(retry_message))
            .route("/commands/stopSession", post(stop_session))
            .route("/commands/archiveSession", post(archive_session))
            .route("/commands/reopenSession", post(reopen_session))
            .route("/commands/resumeSession", post(resume_session))
            .route("/sessions/{id}/events", get(session_events))
            .route(
                "/sessions/{id}/uploads/{upload_id}",
                get(crate::uploads::download),
            )
            .with_state(self.clone())
            .merge(crate::sse::list_router(self.store.clone()))
            .layer(middleware::from_fn_with_state(self.clone(), authorize))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            // Chat history and the chat list are large JSON; event streams and files are left alone.
            .layer(CompressionLayer::new().compress_when(SizeAbove::new(1024).and(JsonOnly)));
        // Uploads stream to disk and enforce their own limit, so they skip the JSON cap.
        let uploads = Router::new()
            .route(
                "/sessions/{id}/uploads/{upload_id}",
                axum::routing::put(crate::uploads::upload),
            )
            .with_state(self.clone())
            .layer(middleware::from_fn_with_state(self.clone(), authorize))
            .layer(DefaultBodyLimit::disable());
        let router = Router::new()
            .merge(commands)
            .merge(uploads)
            .merge(crate::mcp::router(self.clone()));
        match root {
            Some(root) => {
                router.fallback_service(ServeDir::new(root).append_index_html_on_directories(true))
            }
            None => router.fallback(get(crate::assets::serve)),
        }
    }
    /// The chat's runner handle, for tests that drive a chat directly.
    pub async fn handle_for_tests(&self, id: &str) -> SessionHandle {
        self.handle(id).await.expect("the test chat exists")
    }
    pub(crate) async fn handle(&self, id: &str) -> Result<SessionHandle, ApiError> {
        self.sessions
            .lock()
            .await
            .get(id)
            .map(RunningSession::handle)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!(
                        "There is no chat with ID {} on this server. It may have been removed or the ID is wrong; reload the list to see the current chats.",
                        boblm_protocol::client::shown(id)
                    ),
                )
            })
    }
    /// A chat's published status with its account readings read fresh from the account row.
    pub(crate) async fn status_with_account(
        &self,
        handle: &SessionHandle,
        session_id: &str,
    ) -> Result<SessionStatus, ApiError> {
        let mut status = wire_status(handle.published_status()?)?;
        let store = self.store.clone();
        let id = session_id.to_owned();
        status.account_usage =
            tokio::task::spawn_blocking(move || store.reader()?.account_usage_for_session(&id))
                .await
                .map_err(ApiError::internal)??;
        Ok(status)
    }
    /// Archive a root and stop its subagent runners too. The store also cascades the
    /// database rows, so a runner that is not attached still ends archived.
    pub(crate) async fn archive_cascade(&self, id: &str, archived: bool) -> Result<(), ApiError> {
        let subagents = if archived {
            let store = self.store.clone();
            let id = id.to_owned();
            tokio::task::spawn_blocking(move || store.reader()?.subagents_of_root(&id))
                .await
                .map_err(ApiError::internal)??
        } else {
            Vec::new()
        };
        self.handle(id).await?.archive(archived).await?;
        if archived {
            for subagent in subagents {
                let handle = self
                    .sessions
                    .lock()
                    .await
                    .get(&subagent)
                    .map(|s| s.handle());
                if let Some(handle) = handle {
                    handle.archive(true).await?;
                }
            }
        }
        Ok(())
    }
    pub(crate) async fn info(&self, session: Session) -> Result<SessionInfo, ApiError> {
        let handle = self.handle(&session.id).await?;
        let status = self.status_with_account(&handle, &session.id).await?;
        let subagents = if session.parent_session_id.is_none() && session.root_session_id.is_none()
        {
            let store = self.store.clone();
            let id = session.id.clone();
            let counts = tokio::task::spawn_blocking(move || store.reader()?.subagent_summary(&id))
                .await
                .map_err(ApiError::internal)??;
            let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
            Some(SubagentSummary {
                open: count(counts.open),
                questions: count(counts.questions),
                failures: count(counts.failures),
                report_failures: count(counts.report_failures),
            })
        } else {
            None
        };
        let project_id = session.project_id.ok_or_else(|| {
            ApiError::internal(format!(
                "Chat {} has no project, which the database should never allow. Its stored record is damaged; archive it and start a new chat in a project.",
                session.id
            ))
        })?;
        Ok(SessionInfo {
            name: session.name,
            session_id: session.id,
            project_id,
            agent: Agent::parse(&session.agent).unwrap_or(Agent::Claude),
            working_directory: session.working_directory,
            agent_session_id: session.agent_session_id,
            parent_session_id: session.parent_session_id,
            root_session_id: session.root_session_id,
            role: session.role,
            subagents,
            created_at: session.created_at,
            status,
        })
    }
    pub async fn shutdown(&self) -> Result<(), boblm_runner::Error> {
        // No prompt may fire into a chat that is being shut down.
        if let Ok(mut slot) = self.scheduler.lock()
            && let Some(task) = slot.take()
        {
            task.abort();
        }
        if let Ok(mut slot) = self.notification_task.lock()
            && let Some(task) = slot.take()
        {
            task.abort();
        }
        let mut operations = self.peer_operations.lock().await;
        operations.closing = true;
        while let Some(outcome) = operations.tasks.join_next().await {
            if let Err(error) = outcome {
                eprintln!("peer spawn task: {error}");
            }
        }
        drop(operations);
        let sessions = std::mem::take(&mut *self.sessions.lock().await);
        let mut error = None;
        for (_, session) in sessions {
            if let Err(e) = session.shutdown().await {
                error = Some(e);
            }
        }
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(())
        }
    }
}
async fn authorize(
    State(app): State<Arc<App>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut values = request.headers().get_all("authorization").iter();
    let authorized = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|token| bool::from(token.as_bytes().ct_eq(app.token.as_bytes())))
        && values.next().is_none();
    if !authorized {
        return ApiError(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "The server token is missing or wrong. Send it as the header Authorization: Bearer <token>; in the browser, unlock with the token that `boblm init` printed.".into(),
        )
        .into_response();
    }
    next.run(request).await
}
pub(crate) fn body<T: serde::de::DeserializeOwned + CommandBody>(
    bytes: &Bytes,
) -> Result<T, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ApiError::invalid("The request body is not valid UTF-8 text. Commands take a JSON body.")
    })?;
    parse_command(text).map_err(ApiError::invalid)
}
async fn chat_history(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<ChatHistoryPage> {
    let request: ChatHistory = body(&bytes)?;
    let handle = app.handle(&request.session_id).await?;
    let store = app.store.clone();
    let id = request.session_id.clone();
    let newer = request.after.is_some();
    let position = request.before.or(request.after).map(|p| p.0 as i64);
    let page = tokio::task::spawn_blocking(move || {
        store.reader()?.history_page(
            &id,
            position,
            newer,
            i64::from(boblm_tuning::HISTORY_PAGE_SIZE),
        )
    })
    .await
    .map_err(|e| ApiError::internal_while("load chat history", e))?
    .map_err(|e| ApiError::internal_while("load chat history", e))?;
    let wire = |events: Vec<boblm_store::StoredEvent>| -> Result<Vec<StoredEvent>, ApiError> {
        events.into_iter().map(|e| {
            let payload: serde_json::Value = serde_json::from_str(&e.payload).map_err(ApiError::internal)?;
            serde_json::from_value(serde_json::json!({"sessionId":e.session,"seq":e.seq,"kind":e.kind,"payload":payload,"createdAt":e.created_at})).map_err(ApiError::internal)
        }).collect()
    };
    let status = app
        .status_with_account(&handle, &request.session_id)
        .await?;
    let mut reply = ChatHistoryPage {
        session_id: request.session_id,
        partial_id: page.partial_id,
        status,
        cursor: Sequence::try_from(page.cursor as u64).map_err(ApiError::internal)?,
        events: wire(page.events)?,
        context: wire(page.context)?,
        has_more: page.has_more,
    };
    // Large tool outputs can fill a page before its row limit. Keep its nearest edge.
    while serde_json::to_vec(&reply)
        .map_err(ApiError::internal)?
        .len()
        > MAX_BODY_BYTES
    {
        if reply.events.len() <= 1 {
            return Err(ApiError::invalid(
                "This chat message is too large to display. Shorten the tool output before reopening the chat.",
            ));
        }
        if newer {
            reply.events.pop();
        } else {
            reply.events.remove(0);
        }
        reply.has_more = true;
    }
    Ok(Json(reply))
}

/// Deployment checks bind this claim to the actual service PID before skipping
/// legacy idle guards. Provider ownership is fixed when App is constructed.
async fn runtime_capabilities(
    State(app): State<Arc<App>>,
    bytes: Bytes,
) -> ApiResult<serde_json::Value> {
    let _: Empty = body(&bytes)?;
    Ok(Json(
        serde_json::json!({"persistentRunners": app.external_runners, "processId": std::process::id()}),
    ))
}

async fn list_sessions(
    State(app): State<Arc<App>>,
    bytes: Bytes,
) -> ApiResult<ListSessionsResponse> {
    let _: Empty = body(&bytes)?;
    let store = app.store.clone();
    let snapshot = tokio::task::spawn_blocking(move || store.reader()?.session_list_snapshot())
        .await
        .map_err(ApiError::internal)??;
    let mut infos = Vec::new();
    for session in snapshot.sessions {
        infos.push(app.info(session).await?);
    }
    Ok(Json(ListSessionsResponse {
        cursor: Sequence::try_from(snapshot.cursor as u64).map_err(ApiError::invalid)?,
        models: model_catalog(),
        projects: snapshot.projects.into_iter().map(project_info).collect(),
        sessions: infos,
    }))
}
fn project_info(project: boblm_store::Project) -> ProjectInfo {
    ProjectInfo {
        project_id: project.id,
        path: project.path,
        name: project.name,
        created_at: project.created_at,
    }
}
impl From<boblm_store::Error> for ApiError {
    fn from(error: boblm_store::Error) -> Self {
        use boblm_store::Error as E;
        let conflict = |code: &'static str| Self(StatusCode::CONFLICT, code, error.to_string());
        match &error {
            E::ProjectMissing => Self(
                StatusCode::NOT_FOUND,
                "not_found",
                "There is no project with that ID. Reload the list; if the project is gone, add its folder again.".into(),
            ),
            E::ProjectHasChats(_) => conflict("project_in_use"),
            E::ProjectConflict | E::ProjectPathTaken(_) => conflict("project_conflict"),
            E::MessageConflict => conflict("message_conflict"),
            E::QuestionAnswered | E::QuestionConflict => conflict("question_conflict"),
            E::QuestionMissing => Self::invalid(error.to_string()),
            E::ChatDeletion(message) => Self::invalid(message.clone()),
            E::ReorderInvalid(message) => Self::invalid(message.clone()),
            _ => Self::internal(error),
        }
    }
}
async fn list_directories(bytes: Bytes) -> ApiResult<DirectoryListing> {
    let request: ListDirectories = body(&bytes)?;
    let listing = tokio::task::spawn_blocking(move || crate::directories::list(request.path))
        .await
        .map_err(|error| ApiError::internal_while("browse folders", error))?
        .map_err(ApiError::invalid)?;
    Ok(Json(listing))
}
async fn rename_project(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<ProjectInfo> {
    let request: RenameProject = body(&bytes)?;
    let id = boblm_runner::canonical_uuid(&request.project_id)?;
    let store = app.store.clone();
    let project =
        tokio::task::spawn_blocking(move || store.rename_project(&id, request.name.trim()))
            .await
            .map_err(ApiError::internal)??;
    Ok(Json(project_info(project)))
}
async fn add_project(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<ProjectInfo> {
    let request: AddProject = body(&bytes)?;
    let id = boblm_runner::canonical_uuid(&request.project_id)?;
    // One folder, one project: resolve `.`, `..`, doubled and trailing slashes and symlinks,
    // so two spellings of a folder cannot become two projects.
    let typed = request.path.trim();
    let resolved = tokio::fs::canonicalize(typed).await;
    let path = match resolved {
        Ok(path) if path.is_dir() => path.to_string_lossy().into_owned(),
        _ => {
            return Err(ApiError::invalid(format!(
                "{} is not an existing folder on this machine. Check the spelling, or create the folder first.",
                boblm_protocol::client::shown(typed)
            )));
        }
    };
    let name = request
        .name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| boblm_store::default_project_name(&path));
    let store = app.store.clone();
    let (project, _) = tokio::task::spawn_blocking(move || {
        store.add_project(&id, &path, &name, crate::schedule::now_ms())
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(project_info(project)))
}
/// Drag and drop in the sidebar: the new order of the projects, or of one project's chats.
async fn reorder_projects(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let request: ReorderProjects = body(&bytes)?;
    let ids = request
        .project_ids
        .iter()
        .map(|id| boblm_runner::canonical_uuid(id).map_err(ApiError::from))
        .collect::<Result<Vec<_>, _>>()?;
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.reorder_projects(&ids))
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(Empty {}))
}
async fn reorder_chats(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let request: ReorderChats = body(&bytes)?;
    let project = boblm_runner::canonical_uuid(&request.project_id)?;
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.reorder_sessions(&project, &request.session_ids))
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(Empty {}))
}
/// The owner's answer to a credential request. The body carries the secret, so a body that does
/// not parse is reported without quoting any of it.
async fn submit_secret(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<SecretReply> {
    let request: SubmitSecret = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or_else(|| {
            ApiError::invalid(
                "The request body is not what submitSecret expects: a JSON object with requestId and either value (text to save) or cancel: true.",
            )
        })?;
    request.validate().map_err(ApiError::invalid)?;
    let value = match request.cancel {
        Some(true) => None,
        _ => request.value,
    };
    Ok(Json(
        app.submit_secret(&request.request_id, value, crate::schedule::now_ms())
            .await?,
    ))
}
async fn remove_project(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let request: RemoveProject = body(&bytes)?;
    let id = boblm_runner::canonical_uuid(&request.project_id)?;
    let store = app.store.clone();
    let project = id.clone();
    let chats = tokio::task::spawn_blocking(move || {
        Ok::<_, boblm_store::Error>(
            store
                .reader()?
                .sessions()?
                .into_iter()
                .filter(|s| s.project_id.as_deref() == Some(project.as_str()))
                .map(|s| (s.id, s.archived))
                .collect::<Vec<_>>(),
        )
    })
    .await
    .map_err(ApiError::internal)??;
    // Nothing is deleted unless every chat is archived, so a refused removal changes nothing.
    let active = chats.iter().filter(|(_, archived)| !archived).count();
    if active > 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "project_in_use",
            format!(
                "This project still has {active} active chat(s), so it cannot be removed. Archive them first, then remove the project."
            ),
        ));
    }
    // Each chat goes through the same deletion as a single archived chat. If one fails, the rest stay
    // and the owner can try again; a chat already deleted does not block the retry.
    for (chat, _) in chats {
        delete_chat(&app, chat).await?;
    }
    let store = app.store.clone();
    // Removing what is already gone succeeds, so a retry after a lost reply is harmless.
    tokio::task::spawn_blocking(move || store.remove_project(&id))
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(Empty {}))
}
async fn create_session(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<SessionInfo> {
    let request: CreateSession = body(&bytes)?;
    let agent =
        check_choice(&request.model, request.effort.as_deref()).map_err(ApiError::invalid)?;
    let project_id = boblm_runner::canonical_uuid(&request.project_id)?;
    let mut registry = app.sessions.lock().await;
    let (store, id) = (app.store.clone(), request.session_id.clone());
    let session = tokio::task::spawn_blocking(move || -> Result<Session, ApiError> {
        let reader = store.reader()?;
        if let Some(session) = reader.sessions()?.into_iter().find(|s| s.id == id) {
            // The same ID is the same chat, if it is the one this request describes.
            let same = session.project_id.as_deref() == Some(&project_id)
                && session.agent == agent.as_str();
            return if same {
                Ok(session)
            } else {
                Err(ApiError::invalid(format!(
                    "The session ID {} already belongs to a different chat (another project or agent). Use a new ID for a new chat.",
                    boblm_protocol::client::shown(&id)
                )))
            };
        }
        let project = reader.project(&project_id)?.ok_or(boblm_store::Error::ProjectMissing)?;
        if !std::path::Path::new(&project.path).is_dir() {
            return Err(ApiError::invalid(format!(
                "The project folder {} no longer exists, so a chat cannot start there. Restore the folder, or remove the project and add the right one.",
                project.path
            )));
        }
        let account_key = if agent.as_str() == "claude" {
            Some(crate::claude_account::choose_plan(&store, crate::schedule::now_ms() / 1000).map_err(ApiError::invalid)?)
        } else { None };
        store.create_session_on_account(boblm_store::NewSession {
            name: &request.name,
            id: &id,
            project_id: &project_id,
            agent: agent.as_str(),
            model: &request.model,
            effort: request.effort.as_deref(),
            created_at: crate::schedule::now_ms(),
        }, account_key)?;
        store
            .reader()?
            .sessions()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| ApiError::internal("the new chat could not be read back"))
    })
    .await
    .map_err(ApiError::internal)??;
    if !registry.contains_key(&session.id) {
        registry.insert(
            session.id.clone(),
            RunningSession::start(app.store.clone(), &session.id, app.factory.clone()).await?,
        );
    }
    drop(registry);
    Ok(Json(app.info(session).await?))
}
async fn send_message(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<SendMessageResponse> {
    let request: SendMessage = body(&bytes)?;
    let message_id = boblm_runner::canonical_uuid(&request.message_id)?;
    let handle = app.handle(&request.session_id).await?;
    handle
        .send_with_attachments(&message_id, &request.content, &request.attachments)
        .await?;
    Ok(Json(SendMessageResponse {
        message_id,
        accepted: true,
        status: wire_status(handle.published_status()?)?,
    }))
}
async fn delete_session(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<Empty> {
    let request: SessionRef = body(&bytes)?;
    delete_chat(&app, request.session_id).await?;
    Ok(Json(Empty {}))
}
/// Delete one archived chat and queue its uploads for cleanup. An already deleted chat is not an error
/// for a caller that holds a receipt, so a retry after a lost reply succeeds.
pub(crate) async fn delete_chat(app: &App, session_id: String) -> Result<(), ApiError> {
    let store = app.store.clone();
    let root = session_id.clone();
    let subagents = tokio::task::spawn_blocking(move || store.reader()?.subagents_of_root(&root))
        .await
        .map_err(ApiError::internal)??;
    let mut registry = app.sessions.lock().await;
    for subagent in subagents {
        if let Some(session) = registry.get(&subagent) {
            session.handle().delete().await?;
            registry.remove(&subagent);
        }
    }
    if let Some(session) = registry.get(&session_id) {
        session.handle().delete().await?;
        registry.remove(&session_id);
    } else {
        let store = app.store.clone();
        let id = session_id;
        let deleted = tokio::task::spawn_blocking(move || store.reader()?.session_deleted(&id))
            .await
            .map_err(ApiError::internal)??;
        if !deleted {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                "not_found",
                "This chat does not exist. Reload the chat list.".into(),
            ));
        }
    }
    drop(registry);
    app.remove_deleted_uploads().await?;
    Ok(())
}
async fn rename_session(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<CommandResponse> {
    let request: RenameSession = body(&bytes)?;
    let handle = app.handle(&request.session_id).await?;
    handle.rename(&request.name).await?;
    Ok(Json(CommandResponse {
        status: wire_status(handle.status().await?)?,
    }))
}
async fn set_model(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<CommandResponse> {
    let request: SetModel = body(&bytes)?;
    let handle = app.handle(&request.session_id).await?;
    handle
        .set_model(&request.model, request.effort.as_deref())
        .await?;
    Ok(Json(CommandResponse {
        status: wire_status(handle.status().await?)?,
    }))
}
async fn retry_message(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<CommandResponse> {
    let request: RetryMessage = body(&bytes)?;
    let handle = app.handle(&request.session_id).await?;
    handle
        .retry(&boblm_runner::canonical_uuid(&request.message_id)?)
        .await?;
    Ok(Json(CommandResponse {
        status: wire_status(handle.status().await?)?,
    }))
}
macro_rules! session_command {
    ($name:ident,$method:ident $(,$arg:expr)?)=>{
        async fn $name(State(app):State<Arc<App>>,bytes:Bytes)->ApiResult<CommandResponse> {
            let request:SessionRef=body(&bytes)?;let handle=app.handle(&request.session_id).await?;
            handle.$method($($arg)?).await?;Ok(Json(CommandResponse{status:wire_status(handle.status().await?)?}))
        }
    }
}
session_command!(stop_session, stop);
async fn archive_session(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<CommandResponse> {
    let request: SessionRef = body(&bytes)?;
    app.archive_cascade(&request.session_id, true).await?;
    let handle = app.handle(&request.session_id).await?;
    Ok(Json(CommandResponse {
        status: wire_status(handle.status().await?)?,
    }))
}
session_command!(reopen_session, archive, false);
session_command!(resume_session, resume);
async fn session_events(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let after = crate::sse::last_event_id(&headers)
        .map_err(|_| ApiError::invalid("Invalid Last-Event-ID"))?;
    let stream = crate::sse::event_stream(app.handle(&id).await?, id, after);
    Ok(axum::response::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(boblm_tuning::SSE_KEEP_ALIVE)
                .text("keep-alive"),
        )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_internal_error_names_the_action_and_the_cause_without_debug_output() {
        let error = ApiError::internal_while("save the upload", "No space left on device");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.1, "internal");
        let message = error.to_string();
        assert!(
            message.contains("could not save the upload: No space left on device"),
            "{message}"
        );
        assert!(message.contains("fault on the server"), "{message}");
        assert!(
            !message.contains("ApiError(") && !message.contains("backtrace"),
            "{message}"
        );
    }
}
