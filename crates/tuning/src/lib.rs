//! Everything a fork is likely to change, in one place and nothing else.
//!
//! These are compile-time constants: edit a value, rebuild, restart. Keep one
//! constant per line in alphabetical order so merges with upstream stay clean
//! unless both sides changed the same constant. Values that the browser contract
//! depends on (marked "contract") need `cargo run -p boblm-protocol --example
//! protocol-generate` after a change; `bin/check` fails until it is run.
use std::time::Duration;

mod roles;
pub use roles::*;

/// A model a chat can run on, and the agent that runs it. A chat's model decides its agent.
pub struct ModelSpec {
    pub id: &'static str,
    /// Short name people see in tight places. The id stays the machine-facing identifier.
    pub label: &'static str,
    /// `"claude"`, `"codex"` or `"pi"`.
    pub agent: &'static str,
    /// Reasoning efforts this model accepts, weakest first.
    pub efforts: &'static [&'static str],
}

/// A model's short label, or its id when the catalog no longer offers it.
pub fn model_label(id: &str) -> &str {
    MODELS.iter().find(|m| m.id == id).map_or(id, |m| m.label)
}
/// Human-facing model text: `label (id)` when the label differs, otherwise the id.
pub fn model_display(id: &str) -> String {
    match model_label(id) {
        label if label == id => id.to_owned(),
        label => format!("{label} ({id})"),
    }
}

/// How long an unchanged account usage observation still refreshes its confirmation time.
pub const ACCOUNT_USAGE_CONFIRM_REFRESH: Duration = Duration::from_secs(5 * 60);
/// Longest the runner waits for Claude to acknowledge a message it wrote.
pub const ACK_DEADLINE: Duration = Duration::from_secs(60);
/// Acknowledgment window once a turn has produced its first result.
pub const ACK_DEADLINE_AFTER_RESULT: Duration = Duration::from_secs(30);
/// Acknowledgment window once the agent reports a started turn or a compaction in progress.
/// Codex says nothing else while the provider compacts a large thread (30 to 120 seconds seen).
pub const ACK_DEADLINE_DURING_COMPACTION: Duration = Duration::from_secs(300);
/// Maximum combined threads and background jobs collected in a single snapshot.
pub const BACKGROUND_JOB_SCAN_LIMIT: usize = 4096;
/// How often existing agent children are queried for background work.
pub const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Maximum duration of a background-work inventory request.
pub const BACKGROUND_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum time a browser waits for its initial token-validation request.
pub const BROWSER_CONNECT_TIMEOUT_MS: u32 = 15_000;
/// Maximum reload request receipts retained until the server restarts.
pub const BROWSER_RELOAD_RECEIPT_LIMIT: usize = 4096;
/// A queued page reload expires rather than interrupting a tab much later.
pub const BROWSER_RELOAD_TTL: Duration = Duration::from_secs(30);
/// Claude Code executable the drivers launch (looked up on PATH).
pub const CLAUDE_BINARY: &str = "claude";
/// Tools Claude may not use because they would wait on the owner (ADR 0010), comma separated.
pub const CLAUDE_DISALLOWED_TOOLS: &str = "Agent,Task,AskUserQuestion,EnterPlanMode,ExitPlanMode";
/// Efforts Claude Code accepts.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Extra arguments appended to every Claude launch.
pub const CLAUDE_EXTRA_ARGS: &[&str] = &["--prompt-suggestions", "true"];
/// BobLM subcommand used by Claude's background Bash hook.
pub const CLAUDE_HOOK_BACKGROUND_BASH: &str = "hook-background-bash";
/// Model passed at launch; the stored session choice is applied before the first turn.
pub const CLAUDE_LAUNCH_MODEL: &str = "sonnet";
/// Claude permission mode. Sessions bypass prompts because the owner is never waited on (ADR 0010).
pub const CLAUDE_PERMISSION_MODE: &str = "bypassPermissions";
/// Text appended to Claude's system prompt.
pub const CLAUDE_SYSTEM_PROMPT_APPEND: &str = "Work asynchronously. Prefer your harness's native background execution for commands and tools expected to take more than about half a second. Make the background command run the real work directly; never add &, nohup, setsid or disown inside it, so the job ends when the work ends and its completion wakes you. Return control promptly so new owner messages can be handled while work continues. Rely on completion notifications, then inspect the result before proceeding with dependent work. Continue independent work or end your turn while a job runs; never poll or busy-wait. Don't end a turn by announcing the next step: do it, or say exactly what you are waiting for. Keep genuinely quick reads and edits in the foreground, and respect dependencies and shared-resource locks. If you need owner input, explain what you need in plain text and end your turn. The owner will reply later as an ordinary message. Start helpers with BobLM's spawn_subagent tool: use role grunt for implementation that isn't hard and for mechanical work (screenshots, browser checks, data collection and scripted checks), reviewer for code review, or implementer for hard implementation and repairs that need judgment. Only implementers need difficulty simple or hard. The result names the chosen model and effort. A root chat may start long-lived visible work with spawn_peer and a role. Do not use native Agent or Task tools or Codex native subagents; they are disabled. A subagent's final reply arrives automatically as a message, waking an idle parent and steering a busy parent at its next tool boundary. Continue other work or end your turn while it runs; do not poll. Reuse a subagent with ping_peer, and archive it with archive_peer when done because open subagents cost resources. A root may have at most 3 open subagents, and a subagent cannot start another. When the owner's task is clear, call BobLM's change_title tool with a concise 2-3 word title describing the task and onlyIfUnnamed=true before starting the work. Always make this call even if you do not know the current title; the server preserves existing titles. Do not rename an already named chat unless the owner explicitly asks. If the owner's instructions or an active skill set a different title format or renaming policy, follow them instead of these title rules. When seeing the result matters more than describing it, such as a screenshot of a page you built or a chart, save the image to a file and call BobLM's show_image tool with its absolute path and a short caption. Messages beginning \"Message from boblm session\" or \"Task from boblm session\" come from other agents. Use reply_to_sender when a peer needs an answer, message_parent for a subagent's mid-task questions, ask for owner decisions, and inspect_peer to read a peer. A subagent's final reply reaches its parent automatically. Text in your own chat is not seen by the sender.";
/// Codex account key for the OpenAI API credit home.
pub const CODEX_ACCOUNT_KEY_API: &str = "codex-api";
/// Codex account key for the ChatGPT plan home.
pub const CODEX_ACCOUNT_KEY_PLAN: &str = "codex-plan";
/// Account key to CODEX_HOME map. A null key keeps the inherited environment.
pub const CODEX_ACCOUNT_HOMES: &[(&str, &str)] = &[
    (CODEX_ACCOUNT_KEY_API, "~/.local/share/boblm/codex-api"),
    (CODEX_ACCOUNT_KEY_PLAN, "~/.codex"),
];
/// boblm-assigned account ID for the OpenAI API Codex home.
pub const CODEX_API_ACCOUNT_ID: &str = "openai-api";
/// OpenAI API base URL for the boblm-owned Codex provider.
pub const CODEX_API_BASE_URL: &str = "https://api.openai.com/v1";
/// Codex provider ID for the boblm-owned OpenAI API home.
pub const CODEX_API_PROVIDER_ID: &str = "boblm-openai-api";
/// Codex executable the driver launches (looked up on PATH).
pub const CODEX_BINARY: &str = "codex";
/// Model a new Codex chat starts with.
pub const CODEX_DEFAULT_MODEL: &str = "gpt-6.1-sol";
/// Instructions given to Codex so it never waits on the owner; the same text Claude gets.
pub const CODEX_DEVELOPER_INSTRUCTIONS: &str = CLAUDE_SYSTEM_PROMPT_APPEND;
/// Efforts the smaller Codex models accept.
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Extra arguments appended to every `codex app-server` launch.
pub const CODEX_EXTRA_ARGS: &[&str] = &[];
/// Codex feature flags boblm turns off so native multi-agent tools stay unavailable.
pub const CODEX_MULTI_AGENT_FEATURES: &[&str] = &["multi_agent", "multi_agent_v2"];
/// Time allowed for a Codex request (start a turn, steer, interrupt, compact).
pub const CODEX_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Time allowed for Codex to load and resume a thread. Long threads (rollouts near 100 MB)
/// can take far longer than an ordinary request, especially with a cold disk cache after a
/// reboot.
pub const CODEX_RESUME_TIMEOUT: Duration = Duration::from_secs(300);
/// Time allowed for Codex to start, handshake and open a thread. A resume adds
/// CODEX_RESUME_TIMEOUT.
pub const CODEX_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Immediate retries after Codex explicitly rejects steering a stale turn.
pub const CODEX_STEER_RETRIES: usize = 1;
/// Pending requests the session task queues before senders wait.
pub const COMMAND_QUEUE_CAPACITY: usize = 64;
/// Time allowed for a Claude control request (model, effort, interrupt reply).
pub const CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Model a new chat starts with when the browser has no earlier choice.
pub const DEFAULT_MODEL: &str = "sonnet";
/// Port `boblm init` writes into a new config.
pub const DEFAULT_PORT: u16 = 3000;
/// Longest to keep reading Claude's output after it exits with nothing more ready.
pub const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);
/// Absolute limit on reading output after Claude exits, however much arrives.
pub const DRAIN_LIMIT: Duration = Duration::from_secs(5);
/// How long the runner waits for a Claude process to stop before giving up.
pub const DRIVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
/// Most events held between the Claude reader and the session task.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;
/// Largest page of stored events one read returns.
pub const EVENT_PAGE_LIMIT_MAX: i64 = 1024;
/// Provider allowance duration identifying the five-hour window, in minutes.
pub const FIVE_HOUR_USAGE_WINDOW_MINUTES: u64 = 300;
/// Most rendered rows kept in a chat window (contract).
pub const HISTORY_MAX_ROWS: usize = 400;
/// Messages fetched in one history page (contract).
pub const HISTORY_PAGE_SIZE: u16 = 50;
/// Browsers reset to latest history after this many milliseconds without a scroll-position change.
pub const HISTORY_RESUME_LATEST_MS: u32 = 900_000;
/// How long a wheel, touch or keyboard gesture may trigger adjacent history loading (contract).
pub const HISTORY_SCROLL_INTENT_MS: u32 = 1000;
/// Distance from an edge that starts loading an adjacent history page, in pixels (contract).
pub const HISTORY_SCROLL_THRESHOLD: u16 = 160;
/// Delay after an automatically woken root turn before checking whether it needs one nudge.
pub const IDLE_NUDGE_DELAY: Duration = Duration::from_secs(60);
/// How long a newly listed background job blocks an idle nudge as a future wake.
pub const IDLE_NUDGE_RECENT_JOB_WINDOW: Duration = Duration::from_secs(30 * 60);
/// Automated prompt sent once when an automatically woken root turn leaves nothing running.
pub const IDLE_NUDGE_TEXT: &str = "Your last turn ended but nothing is running that will wake you. If work remains, continue it now. If you're finished, or waiting on the owner or a peer, say so in one line and end your turn.";
/// Time allowed to write an interrupt to Claude.
pub const INTERRUPT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum running jobs rendered in the Jobs tab (contract).
pub const JOBS_VISIBLE_LIMIT: usize = 50;
/// Movement that cancels a sidebar long press (pixels; contract).
pub const LONG_PRESS_MOVE_PX: u16 = 8;
/// Hold duration for renaming a sidebar project (milliseconds; contract).
pub const LONG_PRESS_MS: u16 = 500;
/// Most bytes accepted in one agent stderr line, and characters in a diagnostic log message.
pub const MAX_AGENT_DIAGNOSTIC_BYTES: usize = 8192;
/// Largest image file an agent may show in a chat with the show_image tool, in bytes (25 MiB).
pub const MAX_AGENT_IMAGE_BYTES: u64 = 26_214_400;
/// Most attachments one message carries (contract).
pub const MAX_ATTACHMENTS: usize = 10;
/// Largest JSON request or event body (bytes; contract).
pub const MAX_BODY_BYTES: usize = 1_048_576;
/// Most characters in a chat title (contract).
pub const MAX_CHAT_NAME_CHARS: usize = 200;
/// Longest line of Claude output that is parsed; longer lines become unknown events (bytes).
pub const MAX_CLAUDE_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Most filesystem entries scanned by one folder-picker request.
pub const MAX_DIRECTORY_ENTRIES: usize = 2000;
/// Most events committed in one database transaction.
pub const MAX_EVENT_BATCH: usize = 256;
/// Longest reason a held queue shows beside Resume, in characters.
pub const MAX_HELD_REASON_CHARS: usize = 500;
/// Longest caption an agent may give a shown image, in characters.
pub const MAX_IMAGE_CAPTION_CHARS: usize = 200;
/// Largest message the runner accepts (bytes).
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
/// Longest message the browser may send (characters; contract).
pub const MAX_MESSAGE_CHARS: u32 = 65_536;
/// Longest text in a push notification, in characters.
pub const MAX_NOTIFICATION_MESSAGE_CHARS: usize = 500;
/// Largest unencrypted push payload, in bytes.
pub const MAX_NOTIFICATION_PAYLOAD_BYTES: usize = 3000;
/// Longest push notification title, in characters.
pub const MAX_NOTIFICATION_TITLE_CHARS: usize = 100;
/// Most open subagents one root chat may have at once.
pub const MAX_OPEN_SUBAGENTS_PER_ROOT: usize = 3;
/// Most agent-created sessions being set up at once.
pub const MAX_PEER_SPAWNS_IN_FLIGHT: usize = 64;
/// Credential requests one chat may have waiting for the owner at once.
pub const MAX_PENDING_SECRET_REQUESTS: usize = 5;
/// Most characters in a project display name (contract).
pub const MAX_PROJECT_NAME_CHARS: usize = 200;
/// Longest push subscription endpoint, in bytes.
pub const MAX_PUSH_ENDPOINT_CHARS: usize = 2048;
/// Longest answer to an agent question, in characters.
pub const MAX_QUESTION_ANSWER_CHARS: usize = 16_384;
/// Longest suggested answer to an agent question, in characters.
pub const MAX_QUESTION_CHOICE_CHARS: usize = 500;
/// Most suggested answers an agent question may offer.
pub const MAX_QUESTION_CHOICES: usize = 8;
/// Longest agent question, in characters.
pub const MAX_QUESTION_TEXT_CHARS: usize = 4000;
/// Most questions shown in a drawer, with unanswered ones first.
pub const MAX_QUESTIONS_LIST: i64 = 40;
/// Most projects or chats one reorder command may name.
pub const MAX_REORDER_ITEMS: usize = 1000;
/// Most active schedules one chat may have.
pub const MAX_SCHEDULES_PER_CHAT: usize = 20;
/// Furthest ahead a schedule may be set to first run.
pub const MAX_SCHEDULE_HORIZON: Duration = Duration::from_secs(365 * 24 * 60 * 60);
/// Longest credential value the owner may submit.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;
/// Longest session ID (characters; contract).
pub const MAX_SESSION_ID_CHARS: u32 = 128;
/// Largest SOPS file a credential may be written into.
pub const MAX_SOPS_FILE_BYTES: u64 = 1024 * 1024;
/// Largest single upload (bytes; contract).
pub const MAX_UPLOAD_BYTES: u64 = 52_428_800;
/// Longest working directory path (characters; contract).
pub const MAX_WORKING_DIRECTORY_CHARS: u32 = 4096;
/// Shortest repeat interval a schedule may have.
pub const MIN_SCHEDULE_INTERVAL: Duration = Duration::from_secs(60);
/// How long a role step with no known reset is skipped after a quota or billing rejection.
pub const ROLE_EXHAUSTION_RETRY: Duration = Duration::from_secs(60 * 60);
/// Effort for Haiku grunt work on the Claude plan.
pub const ROLE_GRUNT_HAIKU_EFFORT: &str = "high";
/// Effort for Terra grunt work on the Codex plan.
pub const ROLE_GRUNT_TERRA_EFFORT: &str = "medium";
/// Effort for a hard implementer subagent on every routed implementer model.
pub const ROLE_IMPLEMENTER_HARD_EFFORT: &str = "high";
/// Effort for a simple implementer subagent on every routed implementer model.
pub const ROLE_IMPLEMENTER_SIMPLE_EFFORT: &str = "medium";
/// Fixed effort for Opus planning and hard design questions.
pub const ROLE_PLANNER_EFFORT: &str = "high";
/// Daily share of the Claude weekly plan reserved for Opus, in percentage points.
pub const ROLE_R_POINTS_PER_DAY: f64 = 14.0;
/// Effort for the DeepSeek reviewer fallback.
pub const ROLE_REVIEWER_DEEPSEEK_EFFORT: &str = "high";
/// Effort for the Luna reviewer on the OpenAI API credit.
pub const ROLE_REVIEWER_LUNA_EFFORT: &str = "xhigh";
/// Whether role subagent tools are exposed. The cutover set this to true.
pub const ROLE_SUBAGENTS_ENABLED: bool = true;
/// Model ID for DeepSeek V4.1 Flash 1017, used by Pi and the implementer fallback.
pub const MODEL_DEEPSEEK_V41_FLASH: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";
/// Model ID for GPT-6.1 Sol, used by the Codex routing steps.
pub const MODEL_GPT_6_1_SOL: &str = "gpt-6.1-sol";
/// Model ID for GPT-6.1 Terra, used for mechanical work.
pub const MODEL_GPT_6_1_TERRA: &str = "gpt-6.1-terra";
/// Model ID for GPT-6 Luna, used by the reviewer API step.
pub const MODEL_GPT_6_LUNA: &str = "gpt-6-luna";
/// Newest Claude Haiku.
pub const MODEL_HAIKU: &str = "haiku";
/// Newest Claude Opus.
pub const MODEL_OPUS: &str = "opus";
/// Newest Claude Sonnet, used by the implementer Claude step.
pub const MODEL_SONNET: &str = "sonnet";
/// Every model the server offers, Claude's first. Models and efforts here are the only ones a
/// chat may be created with or switched to; checked against `codex app-server` `model/list`.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: MODEL_OPUS,
        label: "opus",
        agent: "claude",
        efforts: CLAUDE_EFFORTS,
    },
    ModelSpec {
        id: MODEL_SONNET,
        label: "sonnet",
        agent: "claude",
        efforts: CLAUDE_EFFORTS,
    },
    ModelSpec {
        id: MODEL_HAIKU,
        label: "haiku",
        agent: "claude",
        efforts: CLAUDE_EFFORTS,
    },
    ModelSpec {
        id: MODEL_GPT_6_1_SOL,
        label: "sol",
        agent: "codex",
        efforts: CODEX_EFFORTS,
    },
    ModelSpec {
        id: MODEL_GPT_6_1_TERRA,
        label: "terra",
        agent: "codex",
        efforts: CODEX_EFFORTS,
    },
    ModelSpec {
        id: MODEL_GPT_6_LUNA,
        label: "luna",
        agent: "codex",
        efforts: CODEX_EFFORTS,
    },
    ModelSpec {
        id: MODEL_DEEPSEEK_V41_FLASH,
        label: "dsf",
        agent: "pi",
        efforts: PI_EFFORTS,
    },
];
/// Most notifications drained in one scheduler tick.
pub const NOTIFICATION_BATCH_SIZE: i64 = 32;
/// How often the browser renews its visible-chat presence (milliseconds; contract).
pub const NOTIFICATION_HEARTBEAT_MS: u32 = 15_000;
/// How long a visible-chat presence suppresses a push without renewal.
pub const NOTIFICATION_PRESENCE_TTL: Duration = Duration::from_secs(45);
/// Time allowed for one push service request.
pub const NOTIFICATION_SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a push service may retain a notification for an offline device.
pub const NOTIFICATION_TTL: Duration = Duration::from_secs(3600);
/// Day of the month (UTC) the OpenAI API credit resets: the ChatGPT plan renewal date.
/// The routing step marked exhausted by a quota error stays skipped until then.
pub const OPENAI_API_CREDIT_RESET_DAY: u32 = 1;
/// Maximum unarchived API reviewers mid-turn or active in the last TPM window.
pub const OPENAI_API_MAX_ACTIVE_REVIEWERS: usize = 3;
/// How long a tokens-per-minute rejection keeps new reviewers off the API.
pub const OPENAI_API_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(2 * 60);
/// Fraction of the API TPM limit available to recent use plus projected reviewer requests.
/// The rest lets already running reviews finish as their contexts grow.
pub const OPENAI_API_TPM_BUDGET: f64 = 0.5;
/// Organization tokens-per-minute limit for the API reviewer model, including cached input.
pub const OPENAI_API_TPM_LIMIT: u64 = 2_000_000;
/// Rolling window for API token accounting and recently active reviewer reservations.
pub const OPENAI_API_TPM_WINDOW: Duration = Duration::from_secs(60);
/// Home-relative SOPS file holding the OpenAI API key used by web_search.
pub const OPENAI_SOPS_FILE: &str = "~/.config/sops/user-secrets.yaml";
/// Object-key path extracted from OPENAI_SOPS_FILE, never the whole document.
pub const OPENAI_SOPS_KEY: &[&str] = &["OPENAI_API_KEY"];
/// Cheapest non-deprecated OpenAI model that supports the hosted web_search tool
/// ($0.10 input and $0.50 output per million tokens at the time of writing).
pub const OPENAI_WEB_SEARCH_MODEL: &str = "gpt-6-luna";
/// Reasoning effort sent with a web_search request; low keeps cost and latency down.
pub const OPENAI_WEB_SEARCH_REASONING_EFFORT: &str = "low";
/// OpenAI Responses API endpoint the web_search tool posts to.
pub const OPENAI_WEB_SEARCH_URL: &str = "https://api.openai.com/v1/responses";
/// How old an upload that was never sent must be before the startup sweep deletes it.
pub const ORPHAN_UPLOAD_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// Pi context percentage at or above which boblm sends `/compact` before the next queued turn.
pub const PI_AUTO_COMPACT_PERCENT: u32 = 98;
/// Pi executable the driver launches (looked up on PATH).
pub const PI_BINARY: &str = "pi";
/// Thinking level used when a Pi chat has no explicit effort.
pub const PI_DEFAULT_EFFORT: &str = "high";
/// Thinking levels offered for Pi models: only high. Every Pi chat reasons, and some
/// providers disable reasoning unless the request asks for it. Pi sends these models a
/// Thinking levels offered for Pi models. Every Pi chat reasons, and DeepInfra disables
/// reasoning unless the request asks for it. Pi sends each level as DeepSeek's own
/// `reasoning_effort` (see PI_PROVIDER_COMPAT_THINKING_FORMAT).
pub const PI_EFFORTS: &[&str] = &["medium", "high"];
/// Maximum provider diagnostic characters retained in a Pi error sentence.
pub const PI_ERROR_MAX_CHARS: usize = 512;
/// Largest peer-tool HTTP response the Pi extension reads.
pub const PI_MCP_MAX_BYTES: usize = MAX_BODY_BYTES;
/// Time allowed for a Pi peer-tool HTTP request.
pub const PI_MCP_TIMEOUT: Duration = Duration::from_secs(30);
/// Context window for the offered DeepInfra DeepSeek models.
pub const PI_MODEL_CONTEXT_WINDOW: usize = 1_048_576;
/// DeepInfra list cached-input price in USD per million tokens, used only for Pi's estimate.
pub const PI_MODEL_COST_CACHE_READ: f64 = 0.006;
/// DeepInfra list uncached-input price in USD per million tokens, used only for Pi's estimate.
pub const PI_MODEL_COST_INPUT: f64 = 0.20;
/// DeepInfra list output price in USD per million tokens, used only for Pi's estimate.
pub const PI_MODEL_COST_OUTPUT: f64 = 0.60;
/// Maximum output tokens advertised to Pi for the offered DeepInfra models.
pub const PI_MODEL_MAX_TOKENS: usize = 131_072;
/// Provider that serves Pi's DeepSeek models.
pub const PI_PROVIDER: &str = "deepinfra";
/// Pi wire protocol for the provider. The OpenAI-compatible path lets Pi send DeepSeek's
/// native `reasoning_effort` instead of an Anthropic thinking budget DeepInfra ignores.
pub const PI_PROVIDER_API: &str = "openai-completions";
/// Environment variable that carries the provider key into the Pi child.
pub const PI_PROVIDER_API_KEY_ENV: &str = "DEEPINFRA_API_KEY";
/// DeepInfra OpenAI-compatible API base URL; Pi appends `/chat/completions`.
pub const PI_PROVIDER_BASE_URL: &str = "https://api.deepinfra.com/v1/openai";
/// Pi compat setting that sends `thinking` plus DeepSeek's native `reasoning_effort`.
pub const PI_PROVIDER_COMPAT_THINKING_FORMAT: &str = "deepseek";
/// Provider SDK retries inside one Pi request. They happen before `agent_end`, so they keep
/// turn boundaries reliable; Pi's own turn retry is disabled because it emits `agent_end` first.
pub const PI_PROVIDER_MAX_RETRIES: usize = 2;
/// Time allowed for a Pi control response.
pub const PI_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Home-relative SOPS file holding the Pi provider credential; resolve its symlink at spawn.
pub const PI_SOPS_FILE: &str = "~/.config/sops/user-secrets.yaml";
/// Object-key path extracted from PI_SOPS_FILE, never the whole document.
pub const PI_SOPS_KEY: &[&str] = &["DEEPINFRA_API_KEY"];
/// Time allowed for Pi startup, tool discovery and session validation.
pub const PI_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Instructions appended to Pi's system prompt for unattended chats.
pub const PI_SYSTEM_PROMPT_APPEND: &str = CLAUDE_SYSTEM_PROMPT_APPEND;
/// Oldest a queued, held or unconfirmed message may be before it expires unsent.
pub const QUEUED_MESSAGE_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);
/// Longest a runner sleeps before it rechecks its queue for expired messages.
pub const QUEUED_MESSAGE_SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// IANA time zone for usage report dates, weeks and daily allowance deltas.
pub const REPORT_TIME_ZONE: &str = "America/Toronto";

/// Longest a server waits to connect to a newly launched session runner.
pub const RUNNER_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest local runner request or reply, including JSON escaping of message text.
pub const RUNNER_FRAME_BYTES: usize = MAX_BODY_BYTES * 8;
/// How often the server refreshes a remote runner's published status.
pub const RUNNER_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Local runner interface version. Incompatible runners must drain before upgrade.
pub const RUNNER_PROTOCOL_VERSION: u32 = 1;
/// Conversation silence required after a turn result before a requested runner restart.
pub const RUNNER_QUIET_PERIOD: Duration = Duration::from_secs(20);
/// Shortest wait before the server restarts the same chat's missing runner again.
pub const RUNNER_RELAUNCH_INTERVAL: Duration = Duration::from_secs(30);
/// Longest a local runner request may wait before its result becomes uncertain.
pub const RUNNER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Default maximum wait for one chat to become quiet before restarting its runner.
pub const RUNNER_RESTART_WAIT: Duration = Duration::from_secs(30 * 60);
/// How often the scheduler looks for prompts that are due.
pub const SCHEDULER_TICK: Duration = Duration::from_secs(2);
/// How late a one-shot schedule may fire after its time (after downtime or while archived).
pub const SCHEDULE_MISSED_GRACE: Duration = Duration::from_secs(60 * 60);
/// How late a repeating schedule may fire; later than this it skips to its next time.
pub const SCHEDULE_ON_TIME_SLACK: Duration = Duration::from_secs(30);
/// How long a credential request waits for the owner before it expires.
pub const SECRET_REQUEST_TTL: Duration = Duration::from_secs(10 * 60);
/// How long the runner waits for Claude to take a written message.
pub const SEND_WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// Time integration tests allow a fresh server to apply durable migrations and announce its listener.
pub const SERVER_STARTUP_TEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The SOPS command that writes credentials into encrypted files.
pub const SOPS_BINARY: &str = "sops";
/// Longest one SOPS command may run before it is killed.
pub const SOPS_TIMEOUT: Duration = Duration::from_secs(10);
/// How long SQLite waits on a locked database before failing. Every chat's runner and
/// the web server hold their own connection, so a slow commit can stall the others for
/// seconds. Keep this below RUNNER_REQUEST_TIMEOUT, or a wait turns into an uncertain outcome.
pub const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval between keep-alive comments on event streams.
pub const SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);
/// How often an idle event stream looks for new events.
pub const SSE_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Longest the runner waits for a stopped turn to report its result.
pub const STOP_RESULT_DEADLINE: Duration = Duration::from_secs(30);
/// Maximum parent links checked when authorizing a nested Codex subagent transcript.
pub const SUBAGENT_ANCESTRY_LIMIT: usize = 16;
/// How long a subagent may sit idle before boblm archives it as a safety net.
pub const SUBAGENT_IDLE_ARCHIVE_AGE: Duration = Duration::from_secs(8 * 60 * 60);
/// How often an open subagent conversation refreshes (contract).
pub const SUBAGENT_POLL_MS: u32 = 2000;
/// How long an archived subagent is kept before the prune deletes it.
pub const SUBAGENT_PRUNE_AGE: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// How often the prune looks for archived subagents old enough to delete.
pub const SUBAGENT_PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Runaway backstop: maximum model calls since a subagent's latest parent message.
pub const SUBAGENT_ROUND_MAX_CALLS: i64 = 3_000;
/// Runaway backstop: maximum input, cache read/write and output tokens in one round.
pub const SUBAGENT_ROUND_MAX_TOKENS: i64 = 400_000_000;
/// Maximum transcript paths retained in the provider file index.
pub const SUBAGENT_TRANSCRIPT_CACHE_LIMIT: usize = 2048;
/// Maximum bytes scanned from a transcript for one history request.
pub const SUBAGENT_TRANSCRIPT_READ_BYTES: usize = 4 * 1024 * 1024;
/// Longest `tailscale serve status` may take when `boblm init` looks for a URL.
pub const TAILSCALE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// Usage percentage at which context and weekly meter fills turn red.
pub const USAGE_HIGH_PERCENT: u32 = 90;
/// Minimum interval between subscription usage snapshots from an existing child.
pub const USAGE_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Short deadline for optional read-only telemetry controls; expiration never stops an agent.
pub const USAGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest page body web_fetch reads before it stops, in bytes.
pub const WEB_FETCH_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Longest text web_fetch returns after converting a page, in characters.
pub const WEB_FETCH_MAX_CHARS: usize = 100_000;
/// Most redirects web_fetch follows before it refuses the chain.
pub const WEB_FETCH_MAX_REDIRECTS: usize = 5;
/// Longest URL web_fetch accepts, in characters.
pub const WEB_FETCH_MAX_URL_CHARS: usize = 4096;
/// Time allowed for one web_fetch request.
pub const WEB_FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// User-Agent web_fetch sends so servers can identify boblm.
pub const WEB_FETCH_USER_AGENT: &str = "boblm/0.1 (web_fetch)";
/// Longest answer text web_search returns, in characters.
pub const WEB_SEARCH_MAX_ANSWER_CHARS: usize = 40_000;
/// Longest query web_search accepts, in characters.
pub const WEB_SEARCH_MAX_QUERY_CHARS: usize = 2_000;
/// Largest OpenAI reply web_search reads before it stops, in bytes.
pub const WEB_SEARCH_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Most cited sources web_search lists.
pub const WEB_SEARCH_MAX_SOURCES: usize = 50;
/// Time allowed for one web_search request.
pub const WEB_SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
/// Minutes in the provider's weekly subscription window.
pub const WEEKLY_USAGE_WINDOW_MINUTES: u64 = 10_080;
