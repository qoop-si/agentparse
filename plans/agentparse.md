# agentparse: one parser for Claude Code, Codex, OpenCode and Gemini CLI sessions

Status: plan v2.2, 2026-10-13 (v1 reviewed by 3 reviewers, triage in `review-triage-1.md`; v2 re-reviewed, triage in `review-triage-2.md`; code review decisions in `code-review-triage.md`).
Owner request: a new standalone library that parses every part of agent sessions (messages, reasoning, tool calls,
terminal commands in full detail, usage, compaction, subagents) from Claude Code, Codex, OpenCode (and Gemini CLI)
into one shared model.

## 0. Summary (5 lines)

1. New Rust crate `agentparse` at `~/coding/agentparse` (lib + small std-only CLI), edition 2024.
2. One model: `Session { meta, entries }`; an entry is a `Message` (role + origin + ordered parts) or an event.
3. Tool calls are one part each: raw input, a typed view (`Tool`), the paired result, status, timing, nested children.
4. Shell commands get first-class detail: command, argv, cwd, timeout, background id, exit code, stdout/stderr, duration, truncation.
5. Every recorded value has exactly one home (a typed field, or `data`/`extra`); unmodeled records stay as `Other`; bad lines become warnings.

Research (measured on real local data plus upstream source): `research-{claude,codex,opencode,gemini}.md`. Those are the
field-level truth. This plan fixes the shared model, conventions, API, per-product rules and phases.

## 1. Scope

In scope:
- Claude Code JSONL (main, subagent and workflow-agent files), Codex rollouts (flat era, legacy envelope, paginated,
  `.jsonl.zst`), OpenCode (SQLite v1 tables, SQLite v2 tables, legacy JSON tree), Gemini CLI (G1 `.json`, G2 `$set`
  JSONL, G3 `$patch`/`$rewindTo` JSONL).
- Discovery for all four (default roots + verified env overrides). Tail reads for JSONL products.
- CLI: `list`, `show [--json]` (readable transcript with full terminal detail), `stats` (corpus verification).
- Fixtures per format generation with semantic assertions, a corpus run with hard numbers.

Out of scope: price tables, live tailing, writing transcripts, indexing/search (wend), decrypting anything, reading
secrets tables, product config files.

## 2. Decisions

| # | Decision | Why |
|---|---|---|
| D1 | Rust, one crate, lib + `src/main.rs` CLI with std-only arg parsing | Consumers (`qoop-history`, `wend-core`) are Rust; one crate is minimal |
| D2 | Deps: `serde`, `serde_json`, `thiserror`, `rusqlite = ">=0.32, <0.41"` bundled, `ruzstd` | rusqlite range verified (R2 scratch crates): consumers pinned at 0.32 and 0.37 resolve to one `libsqlite3-sys`. `ruzstd` 0.9 decodes Codex's level-3 rollouts. No date crate (60 tested lines from qoop-history) |
| D3 | Flat entry list; tool calls are parts of the message that made them and carry their result | All four products have "message with parts"; removes Claude tool_result lines, Gemini functionResponse echoes, Codex split records |
| D4 | One home per recorded value: typed field, else `ToolOutput.data` / `Entry.extra` / `Session.extra` (unmapped keys only). Allowed overlaps: raw `input` plus the typed `tool` view, and `ToolOutput.text` (what the model saw) next to `ShellResult.stdout`/`stderr` (what the process wrote), which are often different texts | Lossless without storing bodies twice |
| D5 | Timestamps `Option<i64>` epoch ms UTC | serde friendly, matches qoop-history |
| D6 | One usage convention (4.1); product totals that do not fit stay in `extra` | Products count differently |
| D7 | Tool status normalized (4.3); the product's own label kept in `status_detail` | Uniform filtering without losing the source truth |
| D8 | Per-line/per-row decode problems become `Warning { line, message }`; I/O, not-a-session, and database open/query/schema failures are errors | Live files end mid-line; corpus drifts; a broken query must not look like an empty session |
| D9 | Unmodeled records stay in place: `EntryKind::Other` between entries, `Part::Other` inside a message | Parse everything without inventing types for product internals |
| D10 | No base64 and no encrypted blobs anywhere: attachments keep mime/name/url/size; `data`/`extra` replace inline base64 and `data:` URLs with `{"bytes": n}`; reasoning keeps `encrypted: bool` | Memory, privacy |
| D11 | A subagent/child is its own `Session` with `parent: ParentLink`; the spawning call carries `child_session` | All four store children separately |
| D12 | Entry order = source order (line order, Codex `ordinal`, OpenCode `seq` / `(time_created, id)`, Gemini first-seen order) | Never sort by id strings or timestamps (Claude timestamps go backwards in 10% of files) |
| D13 | Gemini and Claude keep all history: dropped/rewound/compacted messages stay as entries; `Rewind`/`Compaction` entries mark where context was cut | "Parse everything"; the active context is derivable, lost text is not |

## 3. The model (exact Rust, lives in `src/model.rs`)

Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`. Enums `#[serde(rename_all = "snake_case")]`. Every field that
serialization can omit carries `#[serde(default)]` (Option None, empty Vec, false, empty Usage).

```rust
pub enum Agent { Claude, Codex, Opencode, Gemini }

pub struct Session {
    pub agent: Agent,
    pub id: String,                  // product id; Claude child = agentId, Codex child = payload.id
    pub source: Source,              // (agent, source) identifies a session; ids can repeat across files
    pub format: String,              // "claude-jsonl", "codex-paginated" | "codex-legacy" | "codex-flat",
                                     // "opencode-v1" | "opencode-v2" | "opencode-v1+v2" | "opencode-json", "gemini-g1" | "gemini-g2" | "gemini-g3"
    pub parent: Option<ParentLink>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub git: Option<Git>,
    pub cli_version: Option<String>,
    pub models: Vec<String>,         // distinct, first-use order; never "<synthetic>"
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub usage: Usage,                // sum of message + compaction usage; cost = recorded session cost when one exists
    pub entries: Vec<Entry>,
    pub extra: Option<Value>,        // unmapped session-level keys (Codex session_meta rest, Gemini header/$set metadata,
                                     // OpenCode session row columns, Claude subagent meta)
    pub warnings: Vec<Warning>,
}

pub enum Source { File { path: PathBuf }, Sqlite { path: PathBuf, session_id: String } }
pub struct ParentLink { pub session_id: String, pub call_id: Option<String>, pub agent_type: Option<String> }
pub struct Git { pub branch: Option<String>, pub commit: Option<String>, pub remote: Option<String> }
pub struct Warning { pub line: Option<usize>, pub message: String }

pub struct Entry {
    pub id: Option<String>,          // Claude: message.id for merged assistant messages, else line uuid; Codex: response id
                                     // or item id or "ord:<n>"; OpenCode: message/row id; Gemini: message id
    pub parent_id: Option<String>,   // Claude only: parentUuid resolved to the entry that absorbed that line
    pub turn_id: Option<String>,     // Codex turn_id; Claude promptId carried forward; OpenCode triggering user message id
    pub ts: Option<i64>,
    pub line: Option<usize>,         // 1-based line of the first source record (window-relative for tail reads)
    pub extra: Option<Value>,        // unmapped keys of the source record(s); Claude merged message: extra.uuids
    #[serde(flatten)] pub kind: EntryKind,
}

#[serde(tag = "type")]
pub enum EntryKind {
    Message(Message),
    Compaction(Compaction),
    Turn(Turn),                      // end of a turn or an interrupt marker
    Settings(Settings),              // model/agent/cwd/effort/approval/sandbox changed (emitted on change only)
    Notice(Notice),                  // harness/system record the model did not see
    Hook(Hook),
    Error(ErrorInfo),
    Rewind(Rewind),
    Other(Other),
}

pub struct Message {
    pub role: Role,                  // user | assistant | system (system = harness text the model saw)
    pub origin: Origin,
    pub kind: Option<String>,        // the product's own label for non-human, non-model content
    pub author: Option<String>,      // another agent (Codex agent_message author, Claude teammate)
    pub model: Option<String>,
    pub agent: Option<String>,       // per-message agent/mode, only where recorded per message (OpenCode)
    pub phase: Option<String>,       // Codex commentary/final_answer; "queued" for Claude input still queued
    pub parts: Vec<Part>,
    pub usage: Option<Usage>,
    pub stop_reason: Option<String>,
}

pub enum Role { User, Assistant, System }
pub enum Origin { Human, Model, Command, Context, Notification, Summary, Agent, Tool }
// Command = a human-run slash command or shell escape and its output; Context = injected environment, AGENTS.md,
// reminders, skill bodies, file mentions; Notification = background task/hook/queue notices delivered as input;
// Summary = a resume/away summary not tied to a Compaction record; Agent = inter-agent messages and subagent prompts;
// Tool = a message holding an orphan tool result.

#[serde(tag = "type")]
pub enum Part {
    Text { text: String },
    Reasoning { text: Option<String>, encrypted: bool },
    Attachment(Media),
    ToolCall(ToolCall),
    ToolResult(ToolResult),          // only for results whose call is outside the parsed records
    Other { kind: String, data: Value },
}

pub struct Media { pub mime: Option<String>, pub name: Option<String>, pub url: Option<String>, pub bytes: Option<u64> }

pub struct ToolCall {
    pub id: String,
    pub name: String,                // as recorded
    pub input: Value,                // as recorded (string for freeform input)
    pub tool: Tool,                  // typed view
    pub status: ToolStatus,
    pub status_detail: Option<String>, // product status/denial label as recorded
    pub output: Option<ToolOutput>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub child_session: Option<String>,
    pub children: Vec<ToolCall>,     // Codex code-mode cell children
}
pub struct ToolResult { pub call_id: String, pub status: ToolStatus, pub status_detail: Option<String>, pub output: ToolOutput }
pub enum ToolStatus { Pending, Ok, Error, Rejected, Interrupted }

#[serde(tag = "kind")]
pub enum Tool {
    Shell(Shell),
    Process { id: Option<String>, action: ProcessAction, input: Option<String> },   // ProcessAction: Read | Write | Kill
    Read { path: String, offset: Option<u64>, limit: Option<u64> },                 // offset/limit as recorded
    Write { path: String, content: Option<String> },
    Edit { path: String, edits: Vec<TextEdit> },                                     // TextEdit { old, new, replace_all }
    Patch { patch: String },
    Search { pattern: String, path: Option<String>, glob: Option<String> },
    Glob { pattern: String, path: Option<String> },
    List { path: Option<String> },
    WebFetch { url: Option<String>, prompt: Option<String> },
    WebSearch { query: Option<String> },
    Todo { items: Vec<TodoItem> },                                                   // TodoItem { content, status, priority }
    Task { description: Option<String>, prompt: Option<String>, agent_type: Option<String> },
    Mcp { server: String, tool: String },
    Code { source: String },
    Question, Plan, Skill { name: Option<String> }, Other,
}

pub struct Shell { pub command: String, pub argv: Vec<String>, pub cwd: Option<String>, pub timeout_ms: Option<u64>,
                   pub background: bool, pub description: Option<String> }

pub struct ToolOutput {
    pub text: String,                // what the model saw
    pub shell: Option<ShellResult>,
    pub files: Vec<FileChange>,
    pub attachments: Vec<Media>,
    pub data: Option<Value>,         // structured result as recorded minus mapped keys (4.7)
    pub truncated: bool,
    pub full_output_path: Option<String>,
}

pub struct ShellResult {
    pub exit_code: Option<i32>,      // only from a recorded or printed code
    pub stdout: Option<String>,      // stdout, or the single interleaved stream when stderr is None
    pub stderr: Option<String>,      // Some only where the product records stderr separately
    pub duration_ms: Option<u64>,
    pub interrupted: bool,
    pub timed_out: bool,
    pub signal: Option<String>,
    pub background_id: Option<String>,
}

pub struct FileChange { pub path: String, pub op: FileOp, pub move_to: Option<String>, pub diff: Option<String>,
                        pub additions: Option<u64>, pub deletions: Option<u64> }   // FileOp: Add | Update | Delete | Move

pub struct Usage { pub input: u64, pub output: u64, pub reasoning: u64, pub cache_read: u64, pub cache_write: u64,
                   pub cost_usd: Option<f64> }

pub struct Compaction { pub summary: Option<String>, pub trigger: Option<String>, pub tokens_before: Option<u64>,
                        pub tokens_after: Option<u64>, pub logical_parent: Option<String>, pub usage: Option<Usage> }
pub struct Turn { pub status: Option<String>, pub duration_ms: Option<u64>, pub error: Option<String> }
pub struct Settings { pub model, agent, cwd, effort, approval, sandbox: Option<String> }
pub struct Notice { pub level: Option<String>, pub kind: Option<String>, pub text: String }
pub struct Hook { pub event, name, command: Option<String>, pub exit_code: Option<i32>, pub stdout, stderr: Option<String>,
                  pub duration_ms: Option<u64>, pub blocked: bool }
pub struct ErrorInfo { pub kind: Option<String>, pub message: String, pub status: Option<u16>, pub retry: Option<u32> }
pub struct Rewind { pub dropped: Vec<String>, pub turns: Option<u32> }
pub struct Other { pub kind: String, pub data: Value }
```

Helpers: `Session::messages()`, `Session::tool_calls()` (depth first incl. children), `Message::text()`,
`Message::is_human()` (role User, origin Human), `ToolCall::failed()` (status Error/Rejected/Interrupted),
`Usage::total()` = input + output + cache_read + cache_write, `Usage::add()` (saturating).

## 4. Conventions

### 4.1 Usage

`input` = uncached prompt tokens, `output` = all generated tokens including reasoning, `reasoning` = part of output
(0 when unknown), `cache_read`, `cache_write`, `cost_usd` only when recorded. Arithmetic saturates; an inconsistent
record (cached > input) adds a warning.

| Product | input | output | reasoning | cache_read | cache_write | cost |
|---|---|---|---|---|---|---|
| Claude | `input_tokens` | `output_tokens` | `output_tokens_details.thinking_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens` (5m+1h split stays in `extra`) | session: latest `cost-state` |
| Codex | `input_tokens - cached_input_tokens - cache_write_input_tokens` | `output_tokens` | `reasoning_output_tokens` | `cached_input_tokens` | `cache_write_input_tokens` | none |
| OpenCode | `tokens.input` | `tokens.output + tokens.reasoning` | `tokens.reasoning` | `tokens.cache.read` | `tokens.cache.write` | `cost` |
| Gemini | `tokens.input - tokens.cached + tokens.tool` | `tokens.output + tokens.thoughts` | `tokens.thoughts` | `tokens.cached` | 0 | none |

Attribution: Claude per API `message.id`, usage from the LAST line of that id (early chunks carry partial output);
Codex per `token_usage_record` (`response_id`), fallback `token_count.last_token_usage` deduped by cumulative total;
OpenCode per assistant message (step-finish tokens are never added again); Gemini per `gemini` record. Tests assert
`Session.usage` against each product's own total (Codex last `thread_token_usage`, OpenCode `session_v2.tokens_*`,
Gemini `tokens.total`).

### 4.2 Roles, origin, kind

- Human input: `Role::User, Origin::Human, kind: None`. Everything else a user-role record carries gets the matching
  origin and `kind` = the product label (Claude `origin.kind` or the leading tag name such as `task-notification`,
  `system-reminder`, `local-command-stdout`, `meta`; Codex `content_item_kinds[i]`; OpenCode `synthetic` or
  `metadata.source`; Gemini `session_context`, `hook_context`).
- A source record whose parts have different kinds is split into consecutive messages, ids suffixed `#1`, `#2`.
- `Role::System` = harness text the model saw (Codex `developer` messages, OpenCode v2 `system` rows). `Notice` = a record
  the model did not see (Claude `system` lines other than the mapped subtypes, Gemini `info`/`warning`).
- Tool results never become user messages; they fold into their call. A result whose call is not in the parsed records
  becomes `Part::ToolResult` inside `Message { role: User, origin: Tool }` plus a warning.
- Interrupt markers (Claude `[Request interrupted by user...]`, Codex `<turn_aborted>` + `turn_aborted`, OpenCode
  `MessageAbortedError` / idle `interrupted`) become `Turn { status: "interrupted" }`.
- Human-run commands (Claude `<bash-input>`/`<bash-stdout>`/`<bash-stderr>`, OpenCode v1 `!cmd` user+assistant pair,
  OpenCode v2 `shell` rows, Codex `user_shell` executions) become `Message { role: User, origin: Command }` holding one
  Shell `ToolCall` with its result. Slash commands (`<command-name>`) and their `<local-command-stdout>` are
  `origin: Command` text messages.
- Compaction: the summary carrier record is absorbed into `Compaction.summary` and not emitted again (Claude
  `isCompactSummary` line, OpenCode v1 `summary:true` assistant message, Gemini `<state_snapshot>` user message whose
  "Got it" ack gets `kind: "compaction_ack"`, Codex `compacted.message`).

### 4.3 Tool status

| Status | Claude | Codex | OpenCode | Gemini |
|---|---|---|---|---|
| Ok | result, `is_error` false (incl. `returnCodeInterpretation` soft failures, exit_code None) | output, exit 0, `completed` | `completed` | `success` without timeout text |
| Error | `is_error` true, `Exit code N`, hook blocked | `failed`, `Script failed`, `Script error:` | `error` (`tool.execution`) | `error`; `success` with non-zero `Exit Code` or the timeout text |
| Rejected | `toolDenialKind` user-rejected, permission-rule, automode-* | `declined` | error text "The user rejected permission" (verify) | `cancelled` with a denial reason |
| Interrupted | `toolDenialKind` interrupted/cancelled, session-ended text | `Script terminated`, turn aborted with no output | `metadata.interrupted`, `tool.interrupted`, pending/running in a finished run | `cancelled` user abort |
| Pending | no result, no later assistant message or turn end in the window | `Script running with cell ID` | `pending`/`running`, session still active | result null, nothing after |

A call with no result at all becomes Interrupted when a later assistant message or turn end exists in the parsed
window. A call whose result says it is still running (background id, yielded process, running cell) is never
inferred Interrupted: it stays Ok (background) or Pending (running cell) until its own completion record arrives.
Shell rule: non-zero `exit_code`, `timed_out` or a `signal` makes the status Error.

### 4.4 Tool name to `Tool`

| Tool | Claude | Codex | OpenCode (v1 / v2) | Gemini |
|---|---|---|---|---|
| Shell | `Bash`, `Monitor` (background) | `shell`, `shell_command`, `exec_command`, `local_shell_call`, child `CommandExecution` (non-interaction) | `bash` / `shell` | `run_shell_command` |
| Process | `BashOutput`, `TaskOutput` (read); `KillShell`, `KillBash`, `TaskStop` (kill) | `write_stdin` (write), `wait` (read; kill when `terminate`), `unified_exec_interaction` child (write) | | |
| Read | `Read` | `view_image` | `read` | `read_file` |
| Write | `Write` | | `write` | `write_file` |
| Edit | `Edit`, `MultiEdit` | | `edit` | `replace` |
| Patch | | `apply_patch` | `apply_patch` / `patch` | |
| Search | `Grep` | | `grep` | `grep_search`, `search_file_content` |
| Glob | `Glob` | | `glob` | `glob` |
| List | `LS` | | `list` | `list_directory` |
| WebFetch | `WebFetch` | `web__run` open/find, `web_search_call` open_page/find_in_page | `webfetch` | `web_fetch` |
| WebSearch | `WebSearch` | `web__run` search, `Extension web.search`, `web_search_call` search | `websearch` | `google_web_search` |
| Todo | `TodoWrite`, task list tools that write the list | `update_plan` | `todowrite` | `write_todos`, `write_todos_list` |
| Task | `Task`, `Agent`, `SendMessage` | `spawn_agent`, `send_message`, `followup_task`, `wait_agent` | `task` / `subagent` | `invoke_agent`, agent tools with `agentId` |
| Mcp | `mcp__<server>__<tool>` | `mcp__<server>__<tool>`, `McpToolCall` item | none typed (names are ambiguous with plugin tools) | `mcp_<server>_<tool>` (split at first `_`, raw name kept) |
| Code | | custom `exec` (code mode) | | |
| Question / Plan / Skill | `AskUserQuestion` / `EnterPlanMode`, `ExitPlanMode` / `Skill`, `SlashCommand` | `request_user_input` / none (`Plan` item is assistant text) / none | `question` / `plan_enter`, `plan_exit` / `skill` | `ask_user` / `enter_plan_mode`, `exit_plan_mode` / `activate_skill` |
| Other | `NotebookEdit`, `ToolSearch`, everything else | everything else | `invalid`, plugin tools, MCP | `read_many_files`, `save_memory`, tracker tools |

Typed inputs are lenient: aliases (`file_path`/`filePath`/`path`/`absolute_path`, `dir_path`/`directory`,
`old_string`/`oldString`), numbers as strings, `__unparsedToolInput`. A tool whose required typed field is missing
becomes `Tool::Other` (raw input still there). Never `deny_unknown_fields`.

### 4.5 Shell detail sources

| Field | Claude | Codex | OpenCode | Gemini |
|---|---|---|---|---|
| command / argv | `input.command` | argv from `CommandExecution.command` or `local_shell_call`; command = argv[2] only for `[sh, -c/-lc, cmd]` forms, else the shell-quoted join of argv; args `cmd`/`command` | `input.command` | `args.command` |
| cwd | `cwd` of the `tool_use` line | `CommandExecution.cwd` (file:// decoded), `workdir`, else `turn_context.cwd` | `input.workdir` else `assistant.path.cwd` | `args.dir_path`/`directory` |
| timeout | `input.timeout` ms | `timeout_ms` | `input.timeout` ms | none |
| background | `run_in_background`; result `backgroundTaskId` | output JSON with `session_id` (still running) | `input.background`, `metadata.shellID` | `is_background`, "moved to background (PID: n)" |
| exit code | `Exit code N` error text; timeout 143; 0 for a plain success object (no `returnCodeInterpretation`, not backgrounded or interrupted: the CLI marks every other non-zero exit as an error) | event `exit_code`, output JSON, text headers | `metadata.exit` | `Exit Code: N`; 0 only for modern success without timeout/background text |
| stdout / stderr | `toolUseResult.stdout`/`.stderr` (split); error results are text only | `aggregated_output` (single stream) | output (single stream) | `Output:` single; `Stdout:`/`Stderr:` up to v0.26 |
| duration | none | measured choice (section 6 Codex) | `state.time.end - start` | none |
| interrupted / timed out | denial kind / marker text; "Command timed out after" | `Script terminated`, turn aborted | `metadata.interrupted`, `<shell_metadata>` timeout text | `cancelled`; timeout text |
| truncated / full path | persisted-output marker per research | `Warning: truncated output`, persistence marker | `metadata.truncated`, `outputPath` | `Output too large`, saved path |

### 4.6 File changes per product

Claude: `Edit`/`MultiEdit` `structuredPatch` hunks -> one Update with the hunks rendered as unified diff; `Write` create
or update; `bashEditDiff.files[]` on Bash results. Codex: `apply_patch` input parsed (`src/patch.rs`), replaced by
`FileChange` items / `patch_apply_end.changes` when present. OpenCode: `edit` `metadata.filediff`, `apply_patch`
`metadata.files[]`, v2 `metadata.files[]`. Gemini: `resultDisplay.fileDiff` (+ `isNewFile`).

### 4.7 What `ToolOutput.data` is

Claude `toolUseResult` minus `stdout`, `stderr`, `file.content` when equal to the text, base64; Codex the completion
event item(s) minus `aggregated_output`; OpenCode the tool `state` minus `input`, `output`; Gemini the
`ToolCallRecord` minus `args`, `result`. Base64 becomes `{"bytes": n}` (D10).

## 5. Public API (`src/lib.rs`)

```rust
pub use model::*;
pub type Result<T> = std::result::Result<T, Error>;
pub enum Error { Io(io::Error), Sqlite(String), NotASession(String), NotFound(String) }   // with #[error] text

pub fn parse_file(path: &Path) -> Result<Session>;                       // detects agent and format
pub fn parse_file_tail(path: &Path, max_bytes: u64) -> Result<Session>;  // JSONL products; first torn line dropped
pub fn parse_reader(agent: Agent, reader: impl BufRead, source: Source) -> Result<Session>;
pub fn parse(r: &SessionRef) -> Result<Session>;
pub fn discover(roots: &Roots) -> (Vec<SessionRef>, Vec<Warning>);

pub struct Roots { pub claude, codex, opencode, gemini: Option<PathBuf> }
impl Roots { pub fn from_env() -> Roots }      // $HOME defaults; CLAUDE_CONFIG_DIR, CODEX_HOME, OPENCODE_DB only if verified
pub struct SessionRef { pub agent: Agent, pub id: String, pub source: Source, pub parent_id: Option<String>,
                        pub updated_at: Option<i64> }

pub mod opencode { pub fn list(db: &Path) -> Result<Vec<SessionRef>>; pub fn parse_db(db: &Path, id: &str) -> Result<Session>; }
```

Product modules are private except `opencode` (a database holds many sessions). SQLite opens read-only with a 2 s busy
timeout and reads the WAL through SQLite (never copies the file).

## 6. Per product rules (the research files hold the field details)

### Claude Code (`research-claude.md`)
- Regroup lines by `message.id` (file order); parts in order; usage from the last line of the id; `stop_reason` from
  the last line; partial message (null stop reason everywhere, aborted mid-stream) keeps its parts. A record split
  by origin keeps its usage on the first piece only.
- `tool_result` blocks fold into the call by `tool_use_id`; `toolUseResult` gives `ShellResult`, `FileChange`, `data`.
- Ids: merged assistant `Entry.id` = `message.id`, other lines keep `uuid`; a line-uuid to entry-id map resolves
  `parent_id` (a tool_result line maps to the message holding its call); `compact_boundary` uses `logicalParentUuid`
  as `Compaction.logical_parent`.
- `isApiErrorMessage` -> `Error`; other model `<synthetic>` placeholders ("No response requested.") -> `Notice { kind:
  "synthetic" }`. `TaskCreate`/`TaskUpdate` (list deltas) -> `Other`, not `Todo`. A human-run command's `ToolCall.id` =
  the `<bash-input>` line uuid. `logical_parent` resolves through the same line-uuid map as `parent_id`. `system` subtypes: `turn_duration` -> `Turn`, `api_error` ->
  `Error` with retry, `compact_boundary` -> `Compaction`, `local_command` -> Command message, hooks -> `Hook`, rest ->
  `Notice` with `kind`; `away_summary` -> `Message { role: System, origin: Summary, kind: "away_summary" }`. Hook attachments (`hook_success`, `hook_error`, ...) and `stop_hook_summary` -> `Hook`.
- Attachments: `queued_command` typed by a human -> user Message (Human); file mentions and injected context ->
  Message (Context) when they carry model-visible text, else `Other { kind: "attachment.<subtype>" }`.
- Queue: replay `queue-operation` FIFO (enqueue, dequeue, remove, popAll) as wend does; input still queued at the end
  becomes a human message with `phase: "queued"`; texts starting with `<` are notifications.
- Title: latest `custom-title` > `agent-name` > `ai-title` > old `summary` line > first human prompt; child: meta
  `description`. `Settings` when line `cwd`, `permissionMode`/`permission-mode`, `effort`, `agent-setting` change.
  `turn_id` = last user `promptId` carried forward. `file-history-snapshot`, `queue-operation`, `pr-link`,
  `worktree-state` and other metadata lines -> `Other`.
- Subagents: files under `<session>/subagents/` (and `subagents/workflows/<wf>/`); child id = `agentId`, parent =
  meta `parentAgentId` else line `sessionId`, `call_id` = meta `toolUseId`, `agent_type` = meta `agentType`. Parent call
  `child_session` = `toolUseResult.agentId` / `agent_id` / async notification `<task-id>`. `journal.jsonl` is not a session.
- Background: `backgroundTaskId` -> `ShellResult.background_id`; a later task-notification user line (Notification)
  enriches that call by `<tool-use-id>` (status, exit code from the summary when present).

### Codex (`research-codex.md` sections 6, 8, 9, 11)
- Era by first line: flat (no `type`), envelope (`history_mode` legacy/absent or paginated).
- One assistant Message per model response: assistant-side items up to the next `token_usage_record` (`Entry.id` =
  `response_id`, item ids in `extra.item_ids`); `phase` from the text item (split when two phases: usage stays on the
  first piece only); `model` = current `turn_context.model`. The record whose `response_id` equals a
  `compacted.compaction_response_id` is `Compaction.usage`, not a message.
- Dedupe: event and response item with the same id emit once; paginated user text from `UserMessage` events, legacy
  from `user_message` events (response copy matched by text); `content_item_kinds` drives origin/kind and splitting.
- Code mode: `exec` -> `Tool::Code`; status from the output header; extra outputs for the same `call_id` append;
  children = tool-like items between call and output (an item whose id equals a call id enriches that call); late
  `CommandExecution` joined by `process_id` to the cell whose output JSON has that `session_id` (also later
  `write_stdin` outputs); child `name` = item type, `input` = Null, `data` = the event.
- Shell clock: measure on the real corpus which recorded value (event `duration`, chunk `wall_time_seconds`) matches a
  known slow command; use it for `duration_ms`, keep the other in `data`; document in code.
- Legacy/Extended events (`exec_command_end`, `patch_apply_end`, `mcp_tool_call_end`, `web_search_end`) enrich the call
  with the same `call_id`.
- `turn_context` and `thread_settings_applied` -> `Settings` on change (the first value counts as a change);
  `task_complete`/`turn_aborted` -> `Turn` (the `<turn_aborted>` developer message folds into that one `Turn`); `compacted` -> `Compaction`
  (other compaction records fold in; `replacement_history` to `extra`); `thread_rolled_back` -> `Rewind { turns }`;
  `token_count` rate limits, `world_state`, `inter_agent_communication_metadata`, `SubAgentActivity completed` -> `Other`.
- `agent_message`: Assistant when `author` is this session's agent path (`session_meta.agent_path`, `/root` for a root
  thread), else User with `author`, origin Agent. Legacy files (no ids): assistant text from response items only, the
  `agent_message` event copy matched by text and skipped.
- Child files: id = `payload.id`; parent = `parent_thread_id`; records below `subagent_history_start_ordinal` dropped
  (the ordinal stored in `Session.extra`); parent `spawn_agent` call `child_session` from `SubAgentActivity`.
- `.jsonl.zst` decoded with ruzstd; `history_base` kept in `Session.extra` plus a warning that earlier history lives in
  another rollout. ImageGeneration `result` base64 becomes `{"bytes": n}`. Shell clock candidates: event `duration`,
  item `started_at_ms`/`completed_at_ms`, chunk `wall_time_seconds`.

### OpenCode (`research-opencode.md` sections 11, 12)
- Session in v2 only -> `session_message` by `seq` (`opencode-v2`); v1 only -> `message`/`part` (`opencode-v1`); in both
  -> v1 rows then v2 rows whose id is not a v1 message id, minus `synthetic` rows whose id shares a v1 id's first 16
  characters (the migration's copies of injected text), by `seq` (`opencode-v1+v2`). JSON tree -> `opencode-json`.
- v1 assistant message = one step: parts map 1:1; `step-start`/`snapshot`/`patch` parts -> `Part::Other`; step-finish
  tokens never re-added; `retry` part and assistant `error` -> `Error` entries right after the message. Human-run
  command `ToolCall.id` = the tool `callID` (v1) or `shellID` (v2).
- Tool state: `pending`/`running` in a session whose later messages exist -> Interrupted; v2 `content[]` -> text +
  attachments; names from both generations (`bash|shell`, `task|subagent`, `apply_patch|patch`, `filePath|path`).
- v2 rows: `user`, `synthetic` (Notification/Context by `metadata.source`; background shell completion enriches the call
  by `shellID`), `system` (Role::System), `skill` (user, kind `skill`), `shell` (Command), `assistant`, `compaction`,
  `idle` (`Turn`), `agent-switched`/`model-switched`/`location-switched` (`Settings`).
- `todo` table rows -> a final `Other { kind: "todos" }`. Never read `credential`, `account`, `control_account`.

### Gemini (`research-gemini.md` sections 4, 7, 8, 10)
- Port the upstream reducer (`$set`, `$patch`, `$rewindTo`, id upsert keeping position) and keep the full view (D13).
- One assistant Message per `gemini` record (thoughts, text, tool calls with results); `functionResponse` echo user
  turns fold into calls; `info`/`warning` -> `Notice`, `error` -> `Error`; removals -> `Rewind { dropped }`.
- Shell text parsed per era (section 7 of the research); `resultDisplay` FileDiff -> `FileChange`; masked results
  recovered from earlier lines when present, else `truncated: true`.
- `ended_at` = `ToolCallRecord.timestamp`; cwd from `../.project_root` when parsing a file; title from `$set.summary`;
  subagent files (`chats/<parent>/<agent>.jsonl`, header `kind: subagent`) get `ParentLink`, the parent's call with
  `agentId` gets `child_session`. MCP `mcp_<server>_<tool>` splits at the first `_` after `mcp_` (raw name kept).

## 7. Crate layout

```
Cargo.toml rust-toolchain.toml README.md LICENSE .github/workflows/{ci,release-please}.yml release-please-config.json
src/lib.rs (API, Error, detection, tail)   src/model.rs   src/util.rs   src/patch.rs   src/discover.rs
src/claude.rs  src/codex.rs  src/opencode.rs  src/gemini.rs   (split into a directory only past ~1500 lines)
src/main.rs (CLI)
tests/fixtures/{claude,codex,opencode,gemini}/   tests/{claude,codex,opencode,gemini}.rs
docs/ (site from ~/coding/github-pages-template)
```

## 8. Verification (definition of done)

1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green; CI job at rusqlite 0.32.
2. Fixture tests assert exact semantic facts per the fixture READMEs: counts, roles/origins, every tool call's typed
   view, status, exit code, stdout/stderr, file changes, usage per message and per session, compaction, subagent links,
   warnings. Plus: JSON round trip of every parsed fixture, malformed lines, invalid UTF-8, torn tail, zst input.
3. Corpus run: `agentparse stats` over every discovered session: sessions, failures, warnings per 1k records, entries,
   tool calls by kind and status, shell exit-code histogram, `Other` kinds histogram (each reviewed: deliberate or gap),
   orphan results, p50/p90/p99 parse time, MB/s, peak RSS. Targets: 0 panics, 0 failures except documented
   not-a-session files.
4. Oracles: Codex `thread_history_1.sqlite` turns and `state_5.sqlite` titles; OpenCode `session_v2.tokens_*`; Claude
   usage and tool counts vs an independent jq scan.
5. Named semantic checks: Codex extra outputs for one call (notify) append, late process events attach, legacy
   duplicate text emitted once; Gemini rewind/removeIds keep dropped entries plus `Rewind`, masked result recovery;
   OpenCode migrated session resumed under v2 appends the new rows; SQLite read while the WAL holds uncommitted pages;
   every timestamp is epoch ms (seconds fields converted).
6. Real fresh sessions (`realgen.md`): exit code 3 and both `out-line` and `err-line` present (split for Claude object
   results, single stream elsewhere).
7. `agentparse show` renders one real session per agent readably.

## 9. Phases

| Phase | Work | Who | Gate |
|---|---|---|---|
| P1 | model.rs v2, util.rs, patch.rs, lib.rs, CI files | lead | builds, unit tests |
| P2 | claude.rs, codex.rs, opencode.rs, gemini.rs + tests | 4 implementers in separate copies, shared `CARGO_TARGET_DIR` (builds serialize on the lock); model.rs read-only, change requests go in the report | each: fixture tests green, own-agent corpus run with numbers |
| P3 | merge, discover.rs, main.rs, README, docs | lead + helper | `agentparse stats` over everything |
| P4 | code review: per-module correctness vs research, cross-module consistency, over-engineering, Codex CLI | 4 reviewers | fix to consensus |
| P5 | section 8 end to end; numbers to `plans/verification.md` | lead | targets met |
