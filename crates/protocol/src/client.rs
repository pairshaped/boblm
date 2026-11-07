//! Closed browser contract. Claude's extensible event payload stays JSON.
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub use boblm_tuning::{MAX_ATTACHMENTS, MAX_BODY_BYTES, MAX_UPLOAD_BYTES};
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "u64", into = "u64")]
pub struct Sequence(pub u64);
impl TryFrom<u64> for Sequence {
    type Error = &'static str;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value <= MAX_SAFE_INTEGER {
            Ok(Self(value))
        } else {
            Err("sequence exceeds JavaScript safe integer")
        }
    }
}
impl From<Sequence> for u64 {
    fn from(value: Sequence) -> Self {
        value.0
    }
}
impl JsonSchema for Sequence {
    fn schema_name() -> Cow<'static, str> {
        "Sequence".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"integer", "minimum":0, "maximum":MAX_SAFE_INTEGER})
    }
}
/// The agent that runs a chat. A chat's model decides it (ADR 0023).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Pi,
}
impl Agent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
        }
    }
}
/// One model a chat can use, from the catalog in the tuning crate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelOption {
    pub model: String,
    /// Short name for display; `model` stays the identifier to send.
    pub label: String,
    pub agent: Agent,
    pub efforts: Vec<String>,
}
fn agent_of(spec: &boblm_tuning::ModelSpec) -> Agent {
    Agent::parse(spec.agent).unwrap_or(Agent::Claude)
}
pub fn model_catalog() -> Vec<ModelOption> {
    boblm_tuning::MODELS
        .iter()
        .map(|spec| ModelOption {
            model: spec.id.to_owned(),
            label: spec.label.to_owned(),
            agent: agent_of(spec),
            efforts: spec.efforts.iter().map(|e| (*e).to_owned()).collect(),
        })
        .collect()
}
fn model_names() -> String {
    boblm_tuning::MODELS
        .iter()
        .map(|m| boblm_tuning::model_display(m.id))
        .collect::<Vec<_>>()
        .join(", ")
}
/// Check a model and effort against the catalog and return the agent that runs the model.
/// The message says what is wrong and what is allowed (ADR 0021).
pub fn check_choice(model: &str, effort: Option<&str>) -> Result<Agent, String> {
    let spec = boblm_tuning::MODELS
        .iter()
        .find(|m| m.id == model)
        .ok_or_else(|| {
            format!(
                "{} is not a model this server offers. Choose one of: {}.",
                shown(model),
                model_names()
            )
        })?;
    if let Some(effort) = effort
        && !spec.efforts.contains(&effort)
    {
        return Err(format!(
            "The {model} model does not accept the effort {}. It accepts: {}.",
            shown(effort),
            spec.efforts.join(", ")
        ));
    }
    Ok(agent_of(spec))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    Dispatching,
    Started,
    Rejected,
    Uncertain,
    /// Waited past QUEUED_MESSAGE_MAX_AGE and was never sent.
    Expired,
}
/// The state an expired message was in when it expired.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExpiredFrom {
    Queued,
    /// The agent never confirmed it, so it may already have been received.
    Uncertain,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Empty {}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionRef {
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub session_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateSession {
    #[serde(default)]
    #[schemars(length(max = boblm_tuning::MAX_CHAT_NAME_CHARS))]
    pub name: String,
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub session_id: String,
    /// The project the chat belongs to and works in.
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub project_id: String,
    /// Must be a model in the catalog; the model decides the agent.
    pub model: String,
    pub effort: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AddProject {
    /// Client-generated, so a retry cannot add the folder twice.
    pub project_id: String,
    /// Absolute path of a folder on this machine.
    #[schemars(length(min = 1, max = boblm_tuning::MAX_WORKING_DIRECTORY_CHARS), regex(pattern = "\\S"))]
    pub path: String,
    /// Shown in the sidebar; defaults to the folder's name.
    #[schemars(length(min = 1, max = 200))]
    pub name: Option<String>,
}
/// Change only the project's display name, preserving its folder and chats.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RenameProject {
    pub project_id: String,
    #[schemars(length(min = 1, max = boblm_tuning::MAX_PROJECT_NAME_CHARS))]
    pub name: String,
}
/// Browse folders on the server. Omit path to start at the server user's home.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ListDirectories {
    pub path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DirectoryInfo {
    pub name: String,
    pub path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub directories: Vec<DirectoryInfo>,
}
/// The projects in the order the owner dragged them to. Projects left out keep their places.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReorderProjects {
    pub project_ids: Vec<String>,
}
/// One project's chats in the order the owner dragged them to. Chats left out keep their places.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReorderChats {
    pub project_id: String,
    pub session_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RemoveProject {
    pub project_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RenameSession {
    pub session_id: String,
    #[schemars(length(min = 1, max = boblm_tuning::MAX_CHAT_NAME_CHARS))]
    pub name: String,
}
impl CommandBody for RenameSession {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_chat_name(&self.name, false)
    }
}
pub fn check_chat_name(name: &str, allow_empty: bool) -> Result<(), String> {
    let name = name.trim();
    if (!allow_empty && name.is_empty()) || name.chars().count() > boblm_tuning::MAX_CHAT_NAME_CHARS
    {
        return Err(format!(
            "The chat name must contain {} to {} characters. Shorten it or enter a name before saving.",
            if allow_empty { 0 } else { 1 },
            boblm_tuning::MAX_CHAT_NAME_CHARS
        ));
    }
    Ok(())
}
/// The owner's answer to a chat's request for a credential: the value to save, or a refusal.
/// Deliberately not `Debug`, so the value cannot reach a log by accident.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubmitSecret {
    pub request_id: String,
    /// The credential to save. Omit it to decline instead.
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SECRET_BYTES))]
    pub value: Option<String>,
    /// Decline the request (use instead of `value`).
    pub cancel: Option<bool>,
}
/// What became of a submitted answer. `status` is `saved`, `failed`, `canceled`, `expired` or
/// `closed` (already answered), and `message` says it in words for the owner.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretReply {
    pub status: String,
    pub message: String,
}
/// A folder chats work in.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectInfo {
    pub project_id: String,
    pub path: String,
    pub name: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SendMessage {
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub session_id: String,
    #[schemars(regex(
        pattern = "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    ))]
    pub message_id: String,
    /// May be empty when at least one attachment is present.
    #[schemars(length(max = boblm_tuning::MAX_MESSAGE_CHARS))]
    pub content: String,
    /// Upload IDs in the order the owner attached them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 10))]
    pub attachments: Vec<String>,
}
/// An uploaded file as shown to the owner. Also the reply to an upload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AttachmentInfo {
    #[schemars(regex(
        pattern = "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    ))]
    pub id: String,
    #[schemars(length(min = 1, max = 255))]
    pub name: String,
    #[schemars(length(min = 1, max = 255))]
    pub content_type: String,
    #[schemars(range(max = boblm_tuning::MAX_UPLOAD_BYTES))]
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetryMessage {
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub session_id: String,
    #[schemars(regex(
        pattern = "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    ))]
    pub message_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetModel {
    #[schemars(length(min = 1, max = boblm_tuning::MAX_SESSION_ID_CHARS), regex(pattern = "\\S"))]
    pub session_id: String,
    pub model: String,
    pub effort: Option<String>,
}
/// Decode an HTTP JSON body with the same size and field validation as the browser.
pub fn parse_command<T: serde::de::DeserializeOwned + CommandBody>(
    body: &str,
) -> Result<T, String> {
    if body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "The request body is {} bytes but the limit is {MAX_BODY_BYTES} (1 MiB). Shorten the message, or send large content as an attachment.",
            body.len()
        ));
    }
    let value: T = serde_json::from_str(body)
        .map_err(|error| format!("The request body is not what this command expects: {error}."))?;
    value.validate()?;
    Ok(value)
}
/// Read a window of displayable chat history. Omit both cursors for the latest page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChatHistory {
    pub session_id: String,
    pub before: Option<Sequence>,
    pub after: Option<Sequence>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChatHistoryPage {
    pub session_id: String,
    pub partial_id: Option<String>,
    pub status: SessionStatus,
    pub cursor: Sequence,
    pub events: Vec<StoredEvent>,
    pub context: Vec<StoredEvent>,
    pub has_more: bool,
}
impl CommandBody for ChatHistory {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        if self.before.is_some() && self.after.is_some() {
            return Err("History cannot be read in both directions at once. Send either before or after, or omit both for the latest messages.".into());
        }
        Ok(())
    }
}
/// Read a subagent transcript belonging to this chat. Cursors are transcript byte positions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubagentHistory {
    pub session_id: String,
    pub agent_key: String,
    pub before: Option<Sequence>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubagentHistoryPage {
    pub session_id: String,
    pub agent_key: String,
    pub events: Vec<StoredEvent>,
    pub has_more: bool,
    pub cursor: Sequence,
}
impl CommandBody for SubagentHistory {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        if self.agent_key.is_empty()
            || self.agent_key.len() > boblm_tuning::MAX_SESSION_ID_CHARS as usize
            || !self
                .agent_key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            return Err(
                "The subagent identifier is invalid. Return to the agent list and select it again."
                    .into(),
            );
        }
        Ok(())
    }
}
pub trait CommandBody {
    fn validate(&self) -> Result<(), String>;
}
impl CommandBody for Empty {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}
impl CommandBody for SessionRef {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)
    }
}
impl CommandBody for CreateSession {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_chat_name(&self.name, true)?;
        if self.project_id.trim().is_empty() {
            return Err("projectId is missing: a chat is created inside a project. Add a project (a folder on this machine) first.".into());
        }
        check_choice(&self.model, self.effort.as_deref()).map(|_| ())
    }
}
impl CommandBody for ReorderProjects {
    fn validate(&self) -> Result<(), String> {
        check_order_length("project", self.project_ids.len())?;
        self.project_ids
            .iter()
            .try_for_each(|id| check_uuid("projectIds entry", id))
    }
}
impl CommandBody for ReorderChats {
    fn validate(&self) -> Result<(), String> {
        check_uuid("projectId", &self.project_id)?;
        check_order_length("chat", self.session_ids.len())?;
        self.session_ids.iter().try_for_each(|id| check_session(id))
    }
}
fn check_order_length(what: &str, count: usize) -> Result<(), String> {
    if count > boblm_tuning::MAX_REORDER_ITEMS {
        return Err(format!(
            "The new order lists {count} {what}s but at most {} can be ordered at once.",
            boblm_tuning::MAX_REORDER_ITEMS
        ));
    }
    Ok(())
}
impl CommandBody for SubmitSecret {
    fn validate(&self) -> Result<(), String> {
        // Never quote what was sent: this body carries a credential.
        if !uuid_like(&self.request_id) {
            return Err("requestId must be the UUID of a credential request, like 123e4567-e89b-12d3-a456-426614174000.".into());
        }
        match (&self.value, self.cancel) {
            (Some(value), None | Some(false)) if !value.is_empty() => {
                if value.len() > boblm_tuning::MAX_SECRET_BYTES {
                    return Err(format!(
                        "The credential is {} bytes but the limit is {}.",
                        value.len(),
                        boblm_tuning::MAX_SECRET_BYTES
                    ));
                }
                Ok(())
            }
            (None, Some(true)) => Ok(()),
            _ => Err("Send either a non-empty value to save, or cancel: true to decline, not both and not neither.".into()),
        }
    }
}
impl CommandBody for ListDirectories {
    fn validate(&self) -> Result<(), String> {
        if let Some(path) = &self.path
            && (!path.starts_with('/')
                || path.contains('\0')
                || path.chars().count() > boblm_tuning::MAX_WORKING_DIRECTORY_CHARS as usize)
        {
            return Err(format!(
                "The folder path must start with /, contain no null characters and be at most {} characters. Choose a folder inside your home directory.",
                boblm_tuning::MAX_WORKING_DIRECTORY_CHARS
            ));
        }
        Ok(())
    }
}
impl CommandBody for AddProject {
    fn validate(&self) -> Result<(), String> {
        check_uuid("projectId", &self.project_id)?;
        let path = self.path.trim();
        if !path.starts_with('/')
            || path.chars().count() > boblm_tuning::MAX_WORKING_DIRECTORY_CHARS as usize
        {
            return Err(format!(
                "path must be the absolute path of a folder on this machine, starting with / and at most {} characters; got {}.",
                boblm_tuning::MAX_WORKING_DIRECTORY_CHARS,
                shown(&self.path)
            ));
        }
        if let Some(name) = &self.name
            && (name.trim().is_empty() || name.chars().count() > 200)
        {
            return Err(format!(
                "name must be 1 to 200 characters and not blank; got {}.",
                shown(name)
            ));
        }
        Ok(())
    }
}
impl CommandBody for RenameProject {
    fn validate(&self) -> Result<(), String> {
        check_uuid("projectId", &self.project_id)?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > boblm_tuning::MAX_PROJECT_NAME_CHARS
        {
            return Err(format!(
                "The project name must contain 1 to {} characters. Enter a shorter, nonblank name.",
                boblm_tuning::MAX_PROJECT_NAME_CHARS
            ));
        }
        Ok(())
    }
}
impl CommandBody for RemoveProject {
    fn validate(&self) -> Result<(), String> {
        check_uuid("projectId", &self.project_id)
    }
}
impl CommandBody for SendMessage {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_uuid("messageId", &self.message_id)?;
        if self.attachments.len() > MAX_ATTACHMENTS {
            return Err(format!(
                "A message can carry at most {MAX_ATTACHMENTS} attachments; this one lists {}. Send the rest in another message.",
                self.attachments.len()
            ));
        }
        for id in &self.attachments {
            check_uuid("each attachment ID", id)?;
        }
        if self.content.trim().is_empty() && self.attachments.is_empty() {
            return Err(
                "A message needs some text or at least one attachment; this one is empty.".into(),
            );
        }
        let length = self.content.chars().count();
        if length > boblm_tuning::MAX_MESSAGE_CHARS as usize {
            return Err(format!(
                "The message is {length} characters but the limit is {}. Shorten it, or attach the text as a file.",
                boblm_tuning::MAX_MESSAGE_CHARS
            ));
        }
        Ok(())
    }
}
impl CommandBody for RetryMessage {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_uuid("messageId", &self.message_id)
    }
}
impl CommandBody for SetModel {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_choice(&self.model, self.effort.as_deref()).map(|_| ())
    }
}

fn check_session(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.chars().count() > boblm_tuning::MAX_SESSION_ID_CHARS as usize
    {
        Err(format!(
            "sessionId must be 1 to {} characters and not blank; got {}.",
            boblm_tuning::MAX_SESSION_ID_CHARS,
            shown(value)
        ))
    } else {
        Ok(())
    }
}
/// Quote a client-supplied value for an error message without echoing a huge one.
pub fn shown(value: &str) -> String {
    let mut text: String = value.chars().take(40).collect();
    if value.chars().count() > 40 {
        text.push_str("...");
    }
    format!("{text:?}")
}
fn check_uuid(what: &str, value: &str) -> Result<(), String> {
    if uuid_like(value) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be a UUID like 123e4567-e89b-12d3-a456-426614174000; got {}.",
            shown(value)
        ))
    }
}
fn uuid_like(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionStatus {
    /// Latest provider prediction; transient and absent when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<crate::jobs::JobSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Box<crate::usage::UsageSnapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_usage: Option<crate::usage::AccountUsage>,
    pub selection_pending: bool,
    /// Why the queue is held, when the last driver error caused it; shown beside Resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_reason: Option<String>,
    pub queue_held: bool,
    pub running: bool,
    pub turn_active: bool,
    pub stopping: bool,
    pub queued: u32,
    pub dispatching: u32,
    pub uncertain: u32,
    pub rejected: u32,
    pub archived: bool,
    pub model: String,
    pub effort: Option<String>,
}
/// Counts a root's subagents for its sidebar entry and alert badge.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubagentSummary {
    /// Subagents that are not archived.
    pub open: u32,
    /// Unanswered questions from subagents.
    pub questions: u32,
    /// Subagents whose latest turn ended in failure.
    pub failures: u32,
    /// Subagents with a failed report delivery recorded in their history.
    pub report_failures: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionInfo {
    pub name: String,
    pub session_id: String,
    pub project_id: String,
    pub agent: Agent,
    pub working_directory: String,
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Present on a root chat; absent on a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents: Option<SubagentSummary>,
    pub created_at: i64,
    pub status: SessionStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageAppendedEvent {
    pub uuid: String,
    pub role: Role,
    pub content: String,
    pub delivery_state: DeliveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_from: Option<ExpiredFrom>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryChangedEvent {
    pub uuid: String,
    pub delivery_state: DeliveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_from: Option<ExpiredFrom>,
    pub retry_count: Option<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionCreatedEvent {
    pub id: String,
    pub working_directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionChangedEvent {
    pub agent_session_id: String,
    /// The conversation this one replaced; null for the first. A value means `/clear`
    /// started a new conversation in the same chat.
    pub previous: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveChangedEvent {
    pub archived: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelEffortChangedEvent {
    pub model: String,
    pub effort: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRejectedEvent {
    pub uuid: String,
    pub retry_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretRequestedEvent {
    pub request_id: String,
    /// The SOPS file the value would be saved into.
    pub file: String,
    /// The path of keys inside that file.
    pub key: Vec<String>,
    pub expires_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretResolvedEvent {
    pub request_id: String,
    /// `saved`, `canceled`, `expired` or `failed`.
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriverErrorEvent {
    pub message: String,
}
macro_rules! events {
    ($($kind:ident($payload:ty)),+ $(,)?) => {
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
        pub enum StoredEvent { $($kind { session_id: String, seq: Sequence, created_at: i64, payload: $payload }),+ }
        impl StoredEvent { pub fn seq(&self) -> Sequence { match self { $(Self::$kind { seq, .. } => *seq),+ } } }
    }
}
events! {
    SessionRenamed(SessionRenamedEvent), SessionCreated(SessionCreatedEvent), MessageAppended(MessageAppendedEvent), DeliveryChanged(DeliveryChangedEvent),
    AgentSessionChanged(AgentSessionChangedEvent), ArchiveChanged(ArchiveChangedEvent), ModelEffortChanged(ModelEffortChangedEvent),
    DeliveryRejected(DeliveryRejectedEvent), AgentEvent(serde_json::Value), DriverError(DriverErrorEvent), ModelError(DriverErrorEvent),
    StateSummary(SessionStatus), SessionStopping(Empty), SessionStopped(Empty), DeliveryTimeout(Empty), StopTimeout(Empty),
    QuestionsChanged(serde_json::Value), SecretRequested(SecretRequestedEvent), SecretResolved(SecretResolvedEvent)
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EventPage {
    pub session_id: String,
    pub cursor: Sequence,
    pub events: Vec<StoredEvent>,
    pub status: SessionStatus,
    pub has_more: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ListSessionsResponse {
    /// Resume the global chat-list stream after this snapshot.
    #[serde(default)]
    pub cursor: Sequence,
    pub models: Vec<ModelOption>,
    pub projects: Vec<ProjectInfo>,
    pub sessions: Vec<SessionInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub message_id: String,
    pub accepted: bool,
    pub status: SessionStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommandResponse {
    pub status: SessionStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRenamedEvent {
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SessionListEvent {
    Deleted {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: Empty,
    },
    Created {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: SessionCreatedEvent,
    },
    Renamed {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: SessionRenamedEvent,
    },
    Archived {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: ArchiveChangedEvent,
    },
    StateSummary {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: SessionStatus,
    },
    /// A root chat's subagent summary changed in a way no other list event shows.
    SubagentsChanged {
        session_id: String,
        seq: Sequence,
        created_at: i64,
        payload: Empty,
    },
}
impl SessionListEvent {
    pub fn seq(&self) -> Sequence {
        match self {
            Self::Deleted { seq, .. }
            | Self::Created { seq, .. }
            | Self::Renamed { seq, .. }
            | Self::Archived { seq, .. }
            | Self::StateSummary { seq, .. }
            | Self::SubagentsChanged { seq, .. } => *seq,
        }
    }
}
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct ContractSchemas {
    session_list_event: SessionListEvent,
    empty: Empty,
    session_ref: SessionRef,
    subagent_history: SubagentHistory,
    subagent_history_page: SubagentHistoryPage,
    create_session: CreateSession,
    rename_session: RenameSession,
    send_message: SendMessage,
    attachment: AttachmentInfo,
    retry_message: RetryMessage,
    set_model: SetModel,
    add_project: AddProject,
    rename_project: RenameProject,
    list_directories: ListDirectories,
    directory_listing: DirectoryListing,
    remove_project: RemoveProject,
    reorder_projects: ReorderProjects,
    reorder_chats: ReorderChats,
    push_config: PushConfigReply,
    subscribe_push: SubscribePush,
    unsubscribe_push: UnsubscribePush,
    chat_presence: ChatPresence,
    chat_presence_reply: ChatPresenceReply,
    questions: QuestionsReply,
    answer_question: AnswerQuestion,
    discuss_question_later: DiscussQuestionLater,
    submit_secret: SubmitSecret,
    secret_reply: SecretReply,
    project: ProjectInfo,
    error: ErrorInfo,
    list_sessions: ListSessionsResponse,
    session_info: SessionInfo,
    send_response: SendMessageResponse,
    command: CommandResponse,
    event_page: EventPage,
    chat_history: ChatHistory,
    chat_history_page: ChatHistoryPage,
    delivery_state: DeliveryState,
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn http_bodies_validate_without_an_envelope() {
        let body = json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001","content":"hello"});
        assert!(parse_command::<SendMessage>(&body.to_string()).is_ok());
        for (field, value) in [
            ("content", json!(false)),
            ("messageId", json!("bad")),
            ("sessionId", json!("")),
            ("content", json!(" ")),
        ] {
            let mut invalid = body.clone();
            invalid[field] = value;
            assert!(parse_command::<SendMessage>(&invalid.to_string()).is_err());
        }
        assert!(parse_command::<SendMessage>(&"x".repeat(MAX_BODY_BYTES + 1)).is_err());
        assert!(parse_command::<SendMessage>("{}").is_err());
    }

    #[test]
    fn validation_messages_name_the_field_the_value_and_the_limit() {
        let send = |patch: serde_json::Value| {
            let mut body = json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001","content":"hi"});
            for (key, value) in patch.as_object().unwrap() {
                body[key] = value.clone();
            }
            parse_command::<SendMessage>(&body.to_string()).unwrap_err()
        };
        let bad_id = send(json!({"messageId":"not-a-uuid"}));
        assert!(
            bad_id.contains("messageId must be a UUID like 123e4567"),
            "{bad_id}"
        );
        assert!(bad_id.contains("\"not-a-uuid\""), "{bad_id}");
        let many = send(json!({"attachments": vec!["10000000-0000-4000-8000-000000000001"; 12]}));
        assert!(
            many.contains("at most 10 attachments") && many.contains("lists 12"),
            "{many}"
        );
        let empty = send(json!({"content":"  "}));
        assert!(
            empty.contains("needs some text or at least one attachment"),
            "{empty}"
        );
        let long = send(json!({"content":"x".repeat(65537)}));
        assert!(
            long.contains("65537 characters") && long.contains("limit is 65536"),
            "{long}"
        );
        let session = send(json!({"sessionId":""}));
        assert!(
            session.contains("sessionId must be 1 to 128 characters"),
            "{session}"
        );
        let huge = send(json!({"messageId":"y".repeat(500)}));
        assert!(
            huge.len() < 400,
            "an oversized value is not echoed back whole: {huge}"
        );
        let syntax = parse_command::<SendMessage>("{not json").unwrap_err();
        assert!(
            syntax.starts_with("The request body is not what this command expects"),
            "{syntax}"
        );
        let oversized = parse_command::<SendMessage>(&"x".repeat(MAX_BODY_BYTES + 1)).unwrap_err();
        assert!(
            oversized.contains("1 MiB") && oversized.contains("attachment"),
            "{oversized}"
        );
    }

    #[test]
    fn a_model_decides_the_agent_and_a_bad_choice_lists_what_is_allowed() {
        assert_eq!(check_choice("sonnet", Some("high")), Ok(Agent::Claude));
        assert_eq!(
            check_choice("deepseek-ai/DeepSeek-V4.1-Flash-1017", Some("high")),
            Ok(Agent::Pi)
        );
        assert!(check_choice("deepseek-ai/DeepSeek-V4.1-Flash-1017", Some("low")).is_err());
        assert_eq!(check_choice("gpt-6-luna", Some("low")), Ok(Agent::Codex));
        assert_eq!(check_choice("gpt-6.1-sol", Some("max")), Ok(Agent::Codex));
        assert!(check_choice("gpt-6.1-sol", Some("ultra")).is_err());
        assert_eq!(check_choice("opus", None), Ok(Agent::Claude));
        let unknown = check_choice("gpt-9", None).unwrap_err();
        assert!(
            unknown.contains("\"gpt-9\" is not a model this server offers")
                && unknown.contains("opus")
                && unknown.contains("gpt-6-luna"),
            "{unknown}"
        );
        let effort = check_choice("gpt-6-luna", Some("ultra")).unwrap_err();
        assert!(
            effort.contains("gpt-6-luna model does not accept")
                && effort.contains("low, medium, high, xhigh, max"),
            "{effort}"
        );
        let claude = check_choice("sonnet", Some("ultra")).unwrap_err();
        assert!(claude.contains("sonnet model does not accept"), "{claude}");
        let catalog = model_catalog();
        assert!(
            catalog.iter().any(|m| m.agent == Agent::Claude)
                && catalog.iter().any(|m| m.agent == Agent::Codex)
        );
    }

    #[test]
    fn the_catalog_has_seven_models_with_labels_and_refuses_retired_ones() {
        let labels: Vec<_> = model_catalog()
            .into_iter()
            .map(|m| (m.model, m.label))
            .collect();
        assert_eq!(
            labels,
            [
                ("opus", "opus"),
                ("sonnet", "sonnet"),
                ("haiku", "haiku"),
                ("gpt-6.1-sol", "sol"),
                ("gpt-6.1-terra", "terra"),
                ("gpt-6-luna", "luna"),
                ("deepseek-ai/DeepSeek-V4.1-Flash-1017", "dsf"),
            ]
            .map(|(m, l)| (m.to_owned(), l.to_owned()))
        );
        assert_eq!(
            boblm_tuning::model_display("gpt-6.1-sol"),
            "sol (gpt-6.1-sol)"
        );
        assert_eq!(boblm_tuning::model_display("haiku"), "haiku");
        assert_eq!(check_choice("haiku", Some("high")), Ok(Agent::Claude));
        assert_eq!(
            check_choice("gpt-6.1-terra", Some("medium")),
            Ok(Agent::Codex)
        );
        assert!(check_choice("gpt-6.1-terra", Some("ultra")).is_err());
        assert!(check_choice("gpt-5", None).is_err());
    }

    #[test]
    fn creating_a_chat_needs_a_project_and_projects_need_a_real_looking_folder() {
        let create = |patch: serde_json::Value| {
            let mut body = json!({"sessionId":"s","projectId":"p","model":"sonnet","effort":null});
            for (key, value) in patch.as_object().unwrap() {
                body[key] = value.clone();
            }
            parse_command::<CreateSession>(&body.to_string())
        };
        assert!(create(json!({})).is_ok());
        let none = create(json!({"projectId":" "})).unwrap_err();
        assert!(none.contains("Add a project"), "{none}");
        assert!(create(json!({"model":"gpt-6-luna","effort":"low"})).is_ok());
        assert!(
            create(json!({"model":"nope"}))
                .unwrap_err()
                .contains("not a model this server offers")
        );
        assert!(
            parse_command::<CreateSession>(
                r#"{"sessionId":"s","workingDirectory":"/x","model":"sonnet","effort":null}"#
            )
            .is_err(),
            "a folder is no longer accepted here"
        );
        let id = "20000000-0000-4000-8000-000000000001";
        let add = |path: &str, name: serde_json::Value| {
            parse_command::<AddProject>(
                &json!({"projectId":id,"path":path,"name":name}).to_string(),
            )
        };
        assert!(add("/home/me/app", json!(null)).is_ok());
        assert!(add("/home/me/app", json!("App")).is_ok());
        assert!(
            add("relative/path", json!(null))
                .unwrap_err()
                .contains("absolute path")
        );
        assert!(add("/x", json!("  ")).unwrap_err().contains("name must be"));
        assert!(
            parse_command::<AddProject>(&json!({"projectId":"nope","path":"/x"}).to_string())
                .unwrap_err()
                .contains("projectId must be a UUID")
        );
        assert!(
            parse_command::<SetModel>(
                &json!({"sessionId":"s","model":"nope","effort":null}).to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn attachments_allow_empty_text_but_not_empty_messages() {
        let id = "10000000-0000-4000-8000-000000000001";
        let body = |content: &str, attachments: serde_json::Value| {
            json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001","content":content,"attachments":attachments}).to_string()
        };
        assert!(parse_command::<SendMessage>(&body("", json!([id]))).is_ok());
        assert!(parse_command::<SendMessage>(&body("text", json!([id, id]))).is_ok());
        assert!(parse_command::<SendMessage>(&body("", json!([]))).is_err());
        assert!(parse_command::<SendMessage>(&body("text", json!(["nope"]))).is_err());
        assert!(parse_command::<SendMessage>(&body("text", json!(vec![id; 11]))).is_err());
        // The field is optional, so existing clients keep working.
        let plain = json!({"sessionId":"s","messageId":"00000000-0000-4000-8000-000000000001","content":"hi"});
        assert!(parse_command::<SendMessage>(&plain.to_string()).is_ok());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Pending,
    Answered,
    DiscussLater,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct QuestionInfo {
    pub id: String,
    pub session_id: String,
    pub question: String,
    pub state: QuestionState,
    pub choices: Vec<String>,
    pub answer: Option<String>,
    pub created_at: i64,
    pub answered_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct QuestionsReply {
    pub pending_count: u32,
    pub questions: Vec<QuestionInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnswerQuestion {
    pub session_id: String,
    pub question_id: String,
    pub answer: String,
}
impl CommandBody for AnswerQuestion {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_uuid("questionId", &self.question_id)?;
        if self.answer.trim().is_empty()
            || self.answer.chars().count() > boblm_tuning::MAX_QUESTION_ANSWER_CHARS
        {
            return Err(format!(
                "An answer needs 1 to {} characters. Shorten the answer and try again.",
                boblm_tuning::MAX_QUESTION_ANSWER_CHARS
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PushConfigReply {
    pub public_key: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubscribePush {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UnsubscribePush {
    pub endpoint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChatPresence {
    pub client_id: String,
    pub session_id: Option<String>,
    /// This page understands targeted reload replies.
    #[serde(default)]
    pub reload_capable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChatPresenceReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reload_id: Option<String>,
}
impl CommandBody for SubscribePush {
    fn validate(&self) -> Result<(), String> {
        UnsubscribePush {
            endpoint: self.endpoint.clone(),
        }
        .validate()
    }
}
impl CommandBody for UnsubscribePush {
    fn validate(&self) -> Result<(), String> {
        if self.endpoint.len() > boblm_tuning::MAX_PUSH_ENDPOINT_CHARS
            || !self.endpoint.starts_with("https://")
        {
            return Err("A push subscription needs an HTTPS endpoint within the length limit. Enable notifications again in this browser.".into());
        }
        Ok(())
    }
}
impl CommandBody for ChatPresence {
    fn validate(&self) -> Result<(), String> {
        check_uuid("clientId", &self.client_id)?;
        if let Some(session) = &self.session_id {
            check_session(session)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiscussQuestionLater {
    pub session_id: String,
    pub question_id: String,
}
impl CommandBody for DiscussQuestionLater {
    fn validate(&self) -> Result<(), String> {
        check_session(&self.session_id)?;
        check_uuid("questionId", &self.question_id)
    }
}
