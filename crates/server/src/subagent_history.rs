//! Read-only provider transcripts, authorized through the owning agent conversation.
use crate::app::{ApiError, ApiResult, App, body};
use axum::{Json, body::Bytes, extract::State};
use boblm_protocol::client::{Sequence, StoredEvent, SubagentHistory, SubagentHistoryPage};
use boblm_store::Session;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

type TranscriptPaths = BTreeMap<(PathBuf, String), PathBuf>;
static TRANSCRIPT_PATHS: LazyLock<Mutex<TranscriptPaths>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(crate) async fn history(
    State(app): State<Arc<App>>,
    bytes: Bytes,
) -> ApiResult<SubagentHistoryPage> {
    let request: SubagentHistory = body(&bytes)?;
    let store = app.store.clone();
    let reply = tokio::task::spawn_blocking(move || {
        let session = store
            .reader()
            .map_err(ApiError::internal)?
            .sessions()
            .map_err(ApiError::internal)?
            .into_iter()
            .find(|s| s.id == request.session_id)
            .ok_or_else(|| {
                ApiError::invalid(
                    "This chat no longer exists. Return to the chat list and choose another chat.",
                )
            })?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| ApiError::internal("The server home directory is unavailable"))?;
        let root = if session.agent == "codex" {
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions")
        } else {
            std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"))
                .join("projects")
        };
        load(&root, &session, &request)
    })
    .await
    .map_err(|e| ApiError::internal_while("read the subagent conversation", e))??;
    Ok(Json(reply))
}

fn unavailable() -> ApiError {
    ApiError::invalid(
        "This subagent's transcript is not available yet. Keep this view open to check again, or return to the agent list.",
    )
}
fn transcript_error(error: io::Error) -> ApiError {
    ApiError::invalid(format!(
        "The subagent transcript could not be read: {error}. Keep this view open to check again, or return to the agent list."
    ))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= boblm_tuning::MAX_SESSION_ID_CHARS as usize
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
fn file_within(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let root = root.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The transcript path is outside the provider's session directory",
        ));
    }
    Ok(path)
}
fn first_record(root: &Path, path: &Path) -> io::Result<Value> {
    let mut line = String::new();
    BufReader::new(File::open(file_within(root, path)?)?)
        .take(boblm_tuning::MAX_BODY_BYTES as u64)
        .read_line(&mut line)?;
    serde_json::from_str(&line).map_err(io::Error::other)
}
fn remember(key: (PathBuf, String), path: &Path) -> io::Result<()> {
    let mut index = TRANSCRIPT_PATHS
        .lock()
        .map_err(|_| io::Error::other("The transcript index is unavailable"))?;
    if index.len() >= boblm_tuning::SUBAGENT_TRANSCRIPT_CACHE_LIMIT {
        index.clear();
    }
    index.insert(key, path.to_owned());
    Ok(())
}
fn codex_path(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !identifier(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The agent thread identifier is invalid",
        ));
    }
    let key = (root.to_owned(), id.to_owned());
    if let Some(path) = TRANSCRIPT_PATHS
        .lock()
        .map_err(|_| io::Error::other("The transcript index is unavailable"))?
        .get(&key)
        .cloned()
    {
        return file_within(root, &path);
    }
    let suffix = format!("-{id}.jsonl");
    let mut directories = vec![(root.to_owned(), 0)];
    while let Some((directory, depth)) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() && depth < 3 {
                directories.push((entry.path(), depth + 1));
            } else if kind.is_file() && entry.file_name().to_string_lossy().ends_with(&suffix) {
                let path = file_within(root, &entry.path())?;
                remember(key, &path)?;
                return Ok(path);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "The agent thread transcript has not been written",
    ))
}
pub(crate) fn codex_meta(root: &Path, id: &str) -> io::Result<(PathBuf, Value)> {
    let path = codex_path(root, id)?;
    let record = first_record(root, &path)?;
    let meta = record.get("payload").cloned().unwrap_or(Value::Null);
    if record["type"] != "session_meta" || meta["id"] != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The transcript does not identify the requested agent thread",
        ));
    }
    Ok((path, meta))
}
pub(crate) struct TranscriptWindow {
    pub(crate) rows: Vec<(u64, Value)>,
    has_earlier: bool,
    end: u64,
}
pub(crate) fn records(path: &Path, before: Option<u64>) -> io::Result<TranscriptWindow> {
    let mut file = File::open(path)?;
    let end = before
        .unwrap_or(file.metadata()?.len())
        .min(file.metadata()?.len());
    let start = end.saturating_sub(boblm_tuning::SUBAGENT_TRANSCRIPT_READ_BYTES as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(end - start).read_to_end(&mut bytes)?;
    let mut offset = start;
    let mut found = Vec::new();
    for (i, line) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        // A writer's unfinished final record and a split first record are not messages yet.
        if !(i == 0 && start > 0)
            && line.ends_with(b"\n")
            && let Ok(value) = serde_json::from_slice::<Value>(line)
        {
            found.push((offset, value));
        }
        offset += line.len() as u64;
    }
    Ok(TranscriptWindow {
        rows: found,
        has_earlier: start > 0,
        end,
    })
}
fn claude_path(root: &Path, parent: &str, cwd: &str, key: &str) -> io::Result<PathBuf> {
    let project: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let directory = root.join(project);
    let key_path = (directory.join(parent), key.to_owned());
    if let Some(path) = TRANSCRIPT_PATHS
        .lock()
        .map_err(|_| io::Error::other("The transcript index is unavailable"))?
        .get(&key_path)
        .cloned()
    {
        return file_within(root, &path);
    }
    let parent_path = file_within(root, &directory.join(format!("{parent}.jsonl")))?;
    let parent_rows = records(&parent_path, None)?.rows;
    let mut prompt = None;
    for (_, record) in &parent_rows {
        if let Some(content) = record["message"]["content"].as_array() {
            for block in content {
                if block["type"] == "tool_use" && block["id"] == key {
                    prompt = block["input"]["prompt"].as_str();
                }
                if block["type"] == "tool_result"
                    && block["tool_use_id"] == key
                    && let Some(agent) = record["toolUseResult"]["agentId"]
                        .as_str()
                        .filter(|id| identifier(id))
                {
                    let path = file_within(
                        root,
                        &directory
                            .join(parent)
                            .join("subagents")
                            .join(format!("agent-{agent}.jsonl")),
                    )?;
                    remember(key_path, &path)?;
                    return Ok(path);
                }
            }
        }
    }
    // Foreground Claude agents may not publish their ID until they finish. Match a unique
    // launch prompt inside this parent's subagent directory, never another conversation.
    let prompt = prompt.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "The agent launch is not in the recent parent transcript",
        )
    })?;
    let mut matched = None;
    for entry in fs::read_dir(directory.join(parent).join("subagents"))? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry.path().extension().is_none_or(|ext| ext != "jsonl")
        {
            continue;
        }
        let first = first_record(root, &entry.path())?;
        let text = first["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                first["message"]["content"].as_array().map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            });
        if first["sessionId"] == parent
            && first["type"] == "user"
            && text.as_deref() == Some(prompt)
        {
            if matched.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "More than one subagent has the same launch prompt",
                ));
            }
            matched = Some(entry.path());
        }
    }
    let path = file_within(
        root,
        &matched.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "The subagent transcript has not been written",
            )
        })?,
    )?;
    remember(key_path, &path)?;
    Ok(path)
}
fn event(record: &Value, codex: bool) -> Option<Value> {
    if !codex {
        if !matches!(record["type"].as_str(), Some("assistant" | "user")) {
            return None;
        }
        let mut message = record["message"].clone();
        if let Some(text) = message["content"].as_str().map(str::to_owned) {
            message["content"] = json!([{"type":"text","text":text}]);
        }
        if message["id"].is_null() {
            message["id"] = record["uuid"].clone();
        }
        return Some(json!({"type":record["type"], "message":message}));
    }
    if record["type"] != "response_item" {
        return None;
    }
    let item = &record["payload"];
    match item["type"].as_str()? {
        "message" => {
            let role = item["role"].as_str()?;
            if !matches!(role, "assistant" | "user") {
                return None;
            }
            let blocks: Vec<Value> = item["content"]
                .as_array()?
                .iter()
                .filter_map(|b| {
                    b["text"]
                        .as_str()
                        .map(|text| json!({"type":"text","text":text}))
                })
                .collect();
            if blocks.is_empty() {
                return None;
            }
            Some(json!({"type":role,"message":{"content":blocks}}))
        }
        "function_call" | "custom_tool_call" => Some(
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":item["call_id"],"name":item["name"],"input":item["arguments"].as_str().and_then(|text| serde_json::from_str::<Value>(text).ok()).unwrap_or_else(|| item["input"].clone())}]}}),
        ),
        "function_call_output" | "custom_tool_call_output" => Some(
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":item["call_id"],"content":item["output"]}]}}),
        ),
        _ => None,
    }
}
fn load(
    root: &Path,
    session: &Session,
    request: &SubagentHistory,
) -> Result<SubagentHistoryPage, ApiError> {
    let parent = session
        .agent_session_id
        .as_deref()
        .filter(|id| identifier(id))
        .ok_or_else(unavailable)?;
    if !identifier(&request.agent_key) {
        return Err(ApiError::invalid(
            "The subagent identifier is invalid. Return to the agent list.",
        ));
    }
    let codex = session.agent == "codex";
    let (path, ordinal) = if codex {
        let (path, meta) = codex_meta(root, &request.agent_key).map_err(transcript_error)?;
        let mut ancestor = meta.clone();
        let mut allowed = false;
        for _ in 0..boblm_tuning::SUBAGENT_ANCESTRY_LIMIT {
            let id = ancestor["parent_thread_id"].as_str().or_else(|| {
                ancestor["source"]["subagent"]["thread_spawn"]["parent_thread_id"].as_str()
            });
            if id == Some(parent) {
                allowed = true;
                break;
            }
            let Some(id) = id else {
                break;
            };
            let Ok((_, next)) = codex_meta(root, id) else {
                break;
            };
            ancestor = next;
        }
        if !allowed {
            return Err(ApiError::invalid(
                "This subagent does not belong to the selected chat. Return to its agent list.",
            ));
        }
        (path, meta["subagent_history_start_ordinal"].as_u64())
    } else if session.agent == "claude" {
        let path = claude_path(root, parent, &session.working_directory, &request.agent_key)
            .map_err(transcript_error)?;
        let first = first_record(root, &path).map_err(transcript_error)?;
        if first["sessionId"] != parent {
            return Err(ApiError::invalid(
                "This subagent transcript belongs to another chat. Return to the agent list.",
            ));
        }
        (path, None)
    } else {
        return Err(ApiError::invalid(
            "This agent does not expose subagent transcripts. Return to the agent list.",
        ));
    };
    let window = records(&path, request.before.map(|p| p.0))
        .map_err(|e| ApiError::internal_while("read the subagent transcript", e))?;
    let mut events = Vec::new();
    for (position, record) in window.rows {
        if ordinal.is_some_and(|start| {
            record["ordinal"]
                .as_u64()
                .is_some_and(|value| value < start)
        }) {
            continue;
        }
        let Some(mut payload) = event(&record, codex) else {
            continue;
        };
        if payload["type"] == "assistant" && payload["message"]["id"].is_null() {
            payload["message"]["id"] = json!(format!("transcript-{position}"));
        }
        payload["transcript_timestamp"] = record["timestamp"].clone();
        events.push(StoredEvent::AgentEvent {
            session_id: request.session_id.clone(),
            seq: Sequence(position + 1),
            created_at: 0,
            payload,
        });
    }
    let count = usize::from(boblm_tuning::HISTORY_PAGE_SIZE);
    let has_more = window.has_earlier || events.len() > count;
    let keep_from = events.len().saturating_sub(count);
    events.drain(..keep_from);
    Ok(SubagentHistoryPage {
        session_id: request.session_id.clone(),
        agent_key: request.agent_key.clone(),
        events,
        has_more,
        cursor: Sequence(window.end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(parent: &str, agent: &str) -> Session {
        Session {
            id: "owner".into(),
            working_directory: "/work".into(),
            agent_session_id: Some(parent.into()),
            created_at: 0,
            archived: false,
            model: "fixture".into(),
            effort: None,
            name: "Parent".into(),
            applied_model: "fixture".into(),
            applied_effort: None,
            project_id: Some("p".into()),
            agent: agent.into(),
            account_id: None,
            parent_session_id: None,
            root_session_id: None,
            role: None,
            account_key: None,
        }
    }
    fn write(path: &Path, rows: Vec<Value>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            rows.into_iter()
                .map(|v| format!("{v}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn codex_transcript_is_parent_bound_paged_and_excludes_inherited_history() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let path = root.join("2026/10/24/rollout-test-child.jsonl");
        let mut rows = vec![
            json!({"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","subagent_history_start_ordinal":10}}),
        ];
        for i in 0..150 {
            rows.push(json!({"type":"response_item","ordinal":i,"payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":format!("message {i}")}]}}));
        }
        write(&path, rows);
        let mut request = SubagentHistory {
            session_id: "owner".into(),
            agent_key: "child".into(),
            before: None,
        };
        let page = load(root, &session("parent", "codex"), &request).unwrap();
        assert_eq!(page.events.len(), 50);
        assert!(page.has_more);
        request.before = Some(Sequence(page.events[0].seq().0 - 1));
        let older = load(root, &session("parent", "codex"), &request).unwrap();
        assert_eq!(older.events.len(), 50);
        assert!(older.events.last().unwrap().seq().0 < page.events[0].seq().0);
        assert!(load(root, &session("unrelated", "codex"), &request).is_err());
    }
    #[test]
    fn claude_running_prompt_match_and_completed_id_resolve_same_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let parent_path = root.join("-work/parent.jsonl");
        let launch = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-1","input":{"prompt":"Do the task"}}]}});
        write(&parent_path, vec![launch.clone()]);
        write(
            &root.join("-work/parent/subagents/agent-child.jsonl"),
            vec![
                json!({"type":"user","sessionId":"parent","uuid":"u","message":{"content":"Do the task"}}),
                json!({"type":"assistant","uuid":"a","message":{"content":[{"type":"text","text":"Working"}]}}),
            ],
        );
        let request = SubagentHistory {
            session_id: "owner".into(),
            agent_key: "tool-1".into(),
            before: None,
        };
        let running = load(root, &session("parent", "claude"), &request).unwrap();
        assert_eq!(running.events.len(), 2);
        write(
            &parent_path,
            vec![
                launch,
                json!({"type":"user","toolUseResult":{"agentId":"child"},"message":{"content":[{"type":"tool_result","tool_use_id":"tool-1"}]}}),
            ],
        );
        assert_eq!(
            load(root, &session("parent", "claude"), &request)
                .unwrap()
                .events
                .len(),
            2
        );
        assert!(load(root, &session("other", "claude"), &request).is_err());
    }
    #[test]
    fn incomplete_final_line_is_not_shown_and_traversal_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("partial.jsonl");
        fs::write(&path, "{\"type\":\"complete\"}\n{\"unfinished\":").unwrap();
        assert_eq!(records(&path, None).unwrap().rows.len(), 1);
        assert!(codex_path(temp.path(), "../escape").is_err());
    }
}
