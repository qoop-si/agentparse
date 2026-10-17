//! The one model every product parses into.
//!
//! A [`Session`] is an ordered list of [`Entry`] values. An entry is a [`Message`] (a role
//! plus ordered [`Part`]s) or an event (compaction, turn end, settings change, notice,
//! error, hook run, rewind) or [`Other`] for records this crate does not model. Tool calls
//! are parts of the assistant message that made them and carry their own result, so a
//! consumer never has to pair calls with results.
//!
//! Conventions:
//! - Timestamps are epoch milliseconds UTC.
//! - [`Usage`] uses one counting convention for every product (see its docs).
//! - Bulky raw payloads stay out: no base64 bodies, no encrypted blobs. The raw record is
//!   one step away through [`Entry::line`] (files) or the entry id (databases).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// The agent product that wrote a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    Opencode,
    Gemini,
}

impl Agent {
    pub const ALL: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Gemini];

    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Opencode => "opencode",
            Agent::Gemini => "gemini",
        }
    }
}

impl std::str::FromStr for Agent {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Agent::ALL
            .into_iter()
            .find(|a| a.as_str() == s.to_ascii_lowercase())
            .ok_or_else(|| format!("unknown agent {s:?} (claude, codex, opencode, gemini)"))
    }
}

/// One parsed session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub agent: Agent,
    /// Product session id: Claude child = `agentId`, Codex child = `payload.id`. Ids can repeat
    /// across files (resumed or migrated copies): `(agent, source)` identifies a session.
    pub id: String,
    pub source: Source,
    /// On-disk generation, e.g. `claude-jsonl`, `codex-paginated`, `opencode-v2`, `gemini-g2`.
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Git>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    /// Distinct models, in first-use order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// Sum of message and compaction usage. `cost_usd` is the product-recorded session cost
    /// when one exists (Claude `cost-state`), else the sum of recorded message costs.
    #[serde(default)]
    pub usage: Usage,
    pub entries: Vec<Entry>,
    /// Session-level keys of the source that no field above took, as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    /// Records that could not be read (corrupt lines, bad rows), with where they were.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
}

/// Where a session was read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A transcript file (Claude, Codex, Gemini) or an OpenCode JSON storage session file.
    File { path: PathBuf },
    /// A row set in an OpenCode SQLite database.
    Sqlite { path: PathBuf, session_id: String },
}

/// The session that spawned this one (subagents, child threads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentLink {
    pub session_id: String,
    /// The spawning tool call in the parent, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Subagent type, Codex agent path or nickname.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Git {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// 1-based source line, for files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
}

/// One item of the session timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Product id: Claude `message.id` for a merged assistant message, else the line uuid; Codex
    /// response id, item id or `ord:<n>`; OpenCode message or row id; Gemini message id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Claude only: `parentUuid` resolved to the entry that absorbed that line. Never invented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Turn grouping where recorded (Codex `turn_id`, Claude `promptId`, OpenCode triggering user message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    /// 1-based line of the first source record, for files (window-relative on tail reads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Keys of the source record(s) that no typed field took, as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    #[serde(flatten)]
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryKind {
    Message(Message),
    Compaction(Compaction),
    /// End of a turn with its outcome, or an interrupt marker.
    Turn(Turn),
    /// Model, agent, cwd or permission settings that changed.
    Settings(Settings),
    /// A harness or system record the model did not see.
    Notice(Notice),
    /// A hook program the harness ran.
    Hook(Hook),
    /// A provider/API error or retry.
    Error(ErrorInfo),
    /// History the product rolled back.
    Rewind(Rewind),
    /// A record this crate does not model, kept as recorded.
    Other(Other),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// `System` is harness text the model saw (Codex developer messages, OpenCode system rows).
    pub role: Role,
    /// Who produced the content.
    pub origin: Origin,
    /// The product's own label for content no human typed and no model generated
    /// (`task-notification`, `system-reminder`, `environment_context`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Sender when it is another agent (Codex agent path, Claude teammate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Agent or mode, only where the product records it per message (OpenCode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Codex `commentary` / `final_answer`; `queued` for input still waiting in the queue (Claude
    /// `queue-operation`, OpenCode `session_pending`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Typed by a human.
    Human,
    /// Generated by the model.
    Model,
    /// A slash command, shell escape (`!cmd`) or its output.
    Command,
    /// Context the harness injected: environment, AGENTS.md, reminders, skill bodies, file mentions.
    Context,
    /// Background task, hook or queue notifications delivered as input.
    Notification,
    /// A resume or away summary that is not a compaction record (Claude `away_summary`).
    Summary,
    /// Another agent: inter-agent messages, a subagent's prompt.
    Agent,
    /// Holds a tool result whose call is outside the parsed records.
    Tool,
}

impl Origin {
    /// True for content no human typed and no model generated.
    pub fn is_synthetic(self) -> bool {
        !matches!(self, Origin::Human | Origin::Model)
    }
}

// ponytail: most parts of a busy session are tool calls, so boxing the big variant saves nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// Model reasoning. `text` is `None` when only an opaque blob was stored.
    Reasoning {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// An encrypted blob or signature exists (never stored here).
        #[serde(default, skip_serializing_if = "is_false")]
        encrypted: bool,
    },
    Attachment(Media),
    ToolCall(ToolCall),
    /// A result whose call is outside the parsed records (tail reads, resumed files).
    ToolResult(ToolResult),
    /// A block this crate does not model, kept in place.
    Other {
        kind: String,
        data: Value,
    },
}

/// An image or file reference. Inline data is never kept, only its size.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Media {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// http(s) or file URL, or a path. Never a `data:` URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Decoded size of inline data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// One tool invocation with its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// Tool name as recorded.
    pub name: String,
    /// Input as recorded: parsed JSON arguments, or a string for freeform input.
    pub input: Value,
    /// Typed view of the input.
    pub tool: Tool,
    pub status: ToolStatus,
    /// The product's own status or denial label, as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<ToolOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Session spawned or addressed by this call (subagents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session: Option<String>,
    /// Calls made inside this one (Codex code-mode `exec` cells).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ToolCall>,
}

/// A tool result without its call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    pub output: ToolOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// No result recorded (still running, or the record is cut).
    Pending,
    Ok,
    /// The tool failed, or a shell command exited non-zero or timed out.
    Error,
    /// Denied by the user or a permission rule.
    Rejected,
    /// Aborted by the user or by the run ending.
    Interrupted,
}

/// Typed view of a tool call's input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tool {
    Shell(Shell),
    /// Poll, write to, or kill a running shell.
    Process {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        action: ProcessAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<String>,
    },
    /// `offset`/`limit` as recorded; their base (line or byte, 0 or 1) is the product's.
    Read {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u64>,
    },
    Write {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    Edit {
        path: String,
        edits: Vec<TextEdit>,
    },
    /// An `apply_patch` envelope; per-file changes are in the output's `files`.
    Patch {
        patch: String,
    },
    Search {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        glob: Option<String>,
    },
    Glob {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    List {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    WebFetch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
    WebSearch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    Todo {
        items: Vec<TodoItem>,
    },
    /// Spawn of, or message to, a subagent.
    Task {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
    },
    Mcp {
        server: String,
        tool: String,
    },
    /// A JavaScript program the model wrote for the harness to run (Codex code mode).
    Code {
        source: String,
    },
    Question,
    Plan,
    Skill {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessAction {
    Read,
    Write,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Shell {
    /// The command line as the model wrote it.
    pub command: String,
    /// Exact argv when recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub old: String,
    pub new: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub replace_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// A tool call's result.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The result as the model saw it.
    pub text: String,
    /// What the process did; set on Shell and Process calls only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellResult>,
    /// Files the call changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Media>,
    /// Structured result as recorded, minus values already in typed fields: Claude
    /// `toolUseResult`, Codex completion event(s), OpenCode tool `state` without input/output,
    /// Gemini `ToolCallRecord` without args/result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// Where the product saved the untruncated output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
}

/// What a shell command did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShellResult {
    /// A recorded or printed code, or 0 where the product only reports plain success for a
    /// zero exit (Claude and Gemini success results). `None` when it is truly unknown
    /// (backgrounded, interrupted, soft failures the product reinterpreted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Standard output, or the single interleaved stream when `stderr` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    /// Set only where the product records stderr separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub interrupted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// Shell, task or process id when the command kept running in the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub op: FileOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
    /// The change as the product recorded it: a unified diff (Claude, OpenCode, Gemini, Codex
    /// `FileChange` items) or the `apply_patch` body for this file (hunks without line numbers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Add,
    Update,
    Delete,
    Move,
}

/// Token counts in one convention for every product:
/// `input` is uncached prompt tokens, `output` is every generated token including
/// reasoning, `reasoning` is the part of `output` spent reasoning (0 when unknown).
/// Billable prompt = `input + cache_read + cache_write`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
    /// Only when the product recorded a cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    pub fn add(&mut self, other: &Usage) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.cost_usd = match (self.cost_usd, other.cost_usd) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Compaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// `auto` or `manual` when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
    /// Claude `logicalParentUuid`: the entry the conversation logically continues from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_parent: Option<String>,
    /// Usage of the model call that wrote the summary, when recorded separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Turn {
    /// `completed`, `interrupted`, `failed`, or the product's own word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Background agents still running when the turn ended (Claude `turn_duration`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_agents: Option<u32>,
    /// Background workflows still running when the turn ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_workflows: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Approval policy or permission mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Notice {
    /// `info`, `warning`, `error` when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Product subtype (Claude `system` subtype, attachment type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Hook {
    /// Hook event (`PreToolUse`, `Stop`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The hook blocked or cancelled the action.
    #[serde(default, skip_serializing_if = "is_false")]
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub message: String,
    /// HTTP status when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Retry attempt number when this is a retry notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rewind {
    /// Entry ids that left the active history (they stay in `entries`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
    /// Number of turns rolled back, when that is what was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Other {
    /// Product record type, e.g. `file-history-snapshot`, `world_state`.
    pub kind: String,
    pub data: Value,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl Session {
    /// An empty session; parsers push entries and then call [`Session::finish`].
    pub(crate) fn new(
        agent: Agent,
        id: impl Into<String>,
        source: Source,
        format: &str,
    ) -> Session {
        Session {
            agent,
            id: id.into(),
            source,
            format: format.to_string(),
            parent: None,
            title: None,
            cwd: None,
            git: None,
            cli_version: None,
            models: Vec::new(),
            started_at: None,
            updated_at: None,
            usage: Usage::default(),
            entries: Vec::new(),
            extra: None,
            warnings: Vec::new(),
        }
    }

    /// Fill the derived fields: calls left without a result become Interrupted when the
    /// conversation went on after them, usage sum (messages and compactions; a session cost the
    /// parser already set wins), models in first-use order, first/last timestamps when the product recorded none, and a title from the
    /// first human prompt when the product recorded none.
    pub(crate) fn finish(mut self) -> Session {
        self.interrupt_abandoned_calls();
        let mut usage = Usage::default();
        for e in &self.entries {
            match &e.kind {
                EntryKind::Message(m) => {
                    if let Some(u) = &m.usage {
                        usage.add(u);
                    }
                    if let Some(model) = &m.model {
                        if !self.models.contains(model) {
                            self.models.push(model.clone());
                        }
                    }
                }
                EntryKind::Compaction(c) => {
                    if let Some(u) = &c.usage {
                        usage.add(u);
                    }
                }
                _ => {}
            }
        }
        usage.cost_usd = self.usage.cost_usd.or(usage.cost_usd);
        self.usage = usage;
        let times = self.entries.iter().filter_map(|e| e.ts);
        self.started_at = self.started_at.or_else(|| times.clone().min());
        self.updated_at = self.updated_at.max(times.max());
        if self.title.is_none() {
            let first = self
                .messages()
                .find(|m| m.origin == Origin::Human)
                .map(Message::text);
            self.title = first.and_then(|t| crate::util::one_line(&t, 120));
        }
        self
    }

    /// A call with no result is Interrupted once a later assistant message or turn end shows the
    /// run moved on without it. Calls whose result says they keep running are untouched.
    fn interrupt_abandoned_calls(&mut self) {
        fn mark(call: &mut ToolCall) {
            // Metadata alone (a display name, a description) is not a result.
            let produced = call.output.as_ref().is_some_and(|o| {
                !o.text.is_empty()
                    || o.shell.is_some()
                    || !o.files.is_empty()
                    || !o.attachments.is_empty()
            });
            if call.status == ToolStatus::Pending && !produced {
                call.status = ToolStatus::Interrupted;
            }
            call.children.iter_mut().for_each(mark);
        }
        let mut moved_on = false;
        for entry in self.entries.iter_mut().rev() {
            match &mut entry.kind {
                EntryKind::Message(m) => {
                    if moved_on {
                        for part in &mut m.parts {
                            if let Part::ToolCall(call) = part {
                                mark(call);
                            }
                        }
                    }
                    moved_on |= m.role == Role::Assistant;
                }
                EntryKind::Turn(_) => moved_on = true,
                _ => {}
            }
        }
    }

    /// Every message, in order.
    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.entries.iter().filter_map(|e| match &e.kind {
            EntryKind::Message(m) => Some(m),
            _ => None,
        })
    }

    /// Every tool call, depth first (nested children follow their parent).
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        fn walk<'a>(call: &'a ToolCall, out: &mut Vec<&'a ToolCall>) {
            out.push(call);
            call.children.iter().for_each(|c| walk(c, out));
        }
        let mut out = Vec::new();
        for m in self.messages() {
            for p in &m.parts {
                if let Part::ToolCall(c) = p {
                    walk(c, &mut out);
                }
            }
        }
        out
    }
}

impl Message {
    /// A message with these parts and nothing else set.
    pub(crate) fn new(role: Role, origin: Origin, parts: Vec<Part>) -> Message {
        Message {
            role,
            origin,
            kind: None,
            author: None,
            model: None,
            agent: None,
            phase: None,
            parts,
            usage: None,
            stop_reason: None,
        }
    }

    /// Typed by a human.
    pub fn is_human(&self) -> bool {
        self.role == Role::User && self.origin == Origin::Human
    }

    /// Text parts joined with blank lines.
    pub fn text(&self) -> String {
        crate::util::join_texts(&self.parts, "\n\n")
    }
}

impl ToolCall {
    /// A call without a result yet.
    pub(crate) fn new(id: String, name: String, input: Value, tool: Tool) -> ToolCall {
        ToolCall {
            id,
            name,
            input,
            tool,
            status: ToolStatus::Pending,
            status_detail: None,
            output: None,
            started_at: None,
            ended_at: None,
            child_session: None,
            children: Vec::new(),
        }
    }

    /// The call did not succeed (error, non-zero exit, timeout, rejected, interrupted).
    pub fn failed(&self) -> bool {
        matches!(
            self.status,
            ToolStatus::Error | ToolStatus::Rejected | ToolStatus::Interrupted
        )
    }
}

impl ShellResult {
    /// The command did not succeed although it ran: non-zero exit, timeout or a signal.
    pub(crate) fn failed(&self) -> bool {
        self.timed_out || self.signal.is_some() || self.exit_code.is_some_and(|c| c != 0)
    }
}

impl FileChange {
    /// A change with no diff or counts recorded.
    pub(crate) fn new(path: String, op: FileOp) -> FileChange {
        FileChange {
            path,
            op,
            move_to: None,
            diff: None,
            additions: None,
            deletions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_json_round_trips_with_flattened_kind() {
        let shell = Tool::Shell(Shell {
            command: "ls".into(),
            ..Default::default()
        });
        let call = ToolCall {
            status: ToolStatus::Error,
            output: Some(ToolOutput {
                text: "x".into(),
                shell: Some(ShellResult {
                    exit_code: Some(3),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..ToolCall::new(
                "c1".into(),
                "Bash".into(),
                serde_json::json!({"command": "ls"}),
                shell,
            )
        };
        let parts = vec![
            Part::Reasoning {
                text: None,
                encrypted: true,
            },
            Part::Text { text: "hi".into() },
            Part::ToolCall(call),
        ];
        let entry = Entry {
            id: Some("u1".into()),
            parent_id: None,
            turn_id: None,
            ts: Some(1),
            line: Some(4),
            extra: None,
            kind: EntryKind::Message(Message {
                model: Some("m".into()),
                usage: Some(Usage {
                    input: 1,
                    output: 2,
                    ..Default::default()
                }),
                ..Message::new(Role::Assistant, Origin::Model, parts)
            }),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["parts"][2]["type"], "tool_call");
        assert_eq!(json["parts"][2]["tool"]["kind"], "shell");
        let back: Entry = serde_json::from_value(json).unwrap();
        assert_eq!(back, entry);

        let other = Entry {
            id: None,
            parent_id: None,
            turn_id: None,
            ts: None,
            line: None,
            extra: None,
            kind: EntryKind::Other(Other {
                kind: "x".into(),
                data: serde_json::json!({"type": "inner"}),
            }),
        };
        let back: Entry = serde_json::from_str(&serde_json::to_string(&other).unwrap()).unwrap();
        assert_eq!(back, other);
    }

    #[test]
    fn finish_interrupts_only_abandoned_calls() {
        let call = |id: &str, status, output: Option<ToolOutput>| ToolCall {
            status,
            output,
            ..ToolCall::new(id.into(), "Bash".into(), Value::Null, Tool::Other)
        };
        let assistant = |calls: Vec<ToolCall>| Entry {
            id: None,
            parent_id: None,
            turn_id: None,
            ts: None,
            line: None,
            extra: None,
            kind: EntryKind::Message(Message::new(
                Role::Assistant,
                Origin::Model,
                calls.into_iter().map(Part::ToolCall).collect(),
            )),
        };
        let running = Some(ToolOutput {
            text: "Script running with cell ID 7".into(),
            ..Default::default()
        });
        let metadata_only = Some(ToolOutput {
            data: Some(Value::Null),
            ..Default::default()
        });
        let mut s = Session::new(Agent::Codex, "s", Source::File { path: "x".into() }, "t");
        s.entries = vec![
            assistant(vec![
                call("dead", ToolStatus::Pending, None),
                call("cell", ToolStatus::Pending, running),
                call("meta", ToolStatus::Pending, metadata_only),
            ]),
            assistant(vec![call("last", ToolStatus::Pending, None)]),
        ];
        let s = s.finish();
        let status: Vec<ToolStatus> = s.tool_calls().iter().map(|c| c.status).collect();
        assert_eq!(
            status,
            [
                ToolStatus::Interrupted,
                ToolStatus::Pending,
                ToolStatus::Interrupted,
                ToolStatus::Pending
            ]
        );
    }

    #[test]
    fn usage_adds_and_keeps_cost_optional() {
        let mut a = Usage {
            input: 1,
            ..Default::default()
        };
        a.add(&Usage {
            output: 2,
            ..Default::default()
        });
        assert_eq!((a.total(), a.cost_usd), (3, None));
        a.add(&Usage {
            cost_usd: Some(0.5),
            ..Default::default()
        });
        assert_eq!(a.cost_usd, Some(0.5));
    }
}
