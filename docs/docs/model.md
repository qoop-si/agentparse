# Sessions and Entries

Every product parses into the same types. This page lists each one with all its fields. The types are re-exported from the crate root (`agentparse::Session`, `agentparse::Entry`, and so on).

## Shape

A `Session` holds an ordered list of `Entry` values. An entry is a `Message` (a role plus ordered `Part`s) or an event such as a compaction or a turn end. Tool calls are parts of the assistant message that made them, and each carries its own result.

```
Session
  entries: Vec<Entry>
    Entry { id, parent_id, turn_id, ts, line, extra, kind }
      Message { role, origin, parts, usage, ... }
        Part::Text, Part::Reasoning, Part::Attachment
        Part::ToolCall -> ToolCall { tool: Tool, output: ToolOutput { shell: ShellResult, files, data } }
      Compaction, Turn, Settings, Notice, Hook, Error, Rewind, Other
```

## Type index

| Type | Kind | Described in |
|---|---|---|
| `Agent` | enum | this page |
| `Session`, `Source`, `ParentLink`, `Git`, `Warning` | structs and enum | this page |
| `Entry`, `EntryKind` | struct, enum | this page |
| `Message`, `Role`, `Origin`, `Part`, `Media` | structs and enums | this page |
| `Compaction`, `Turn`, `Settings`, `Notice`, `Hook`, `ErrorInfo`, `Rewind`, `Other` | event structs | this page |
| `ToolCall`, `ToolResult`, `ToolStatus`, `ToolOutput`, `FileChange`, `FileOp` | tool result types | this page |
| `Usage` | struct | this page, rules in [Usage and Status](#conventions) |
| `Tool`, `ProcessAction`, `TextEdit`, `TodoItem` | typed tool input | [Tool Calls](#tools) |
| `Shell`, `ShellResult` | shell call and result | [Shell Commands](#shell) |
| `Roots`, `SessionRef`, `Error`, `Result` | discovery and errors | [Rust API](#api) |

## JSON shape

All model types implement `Serialize` and `Deserialize`. The JSON that `agentparse show --json` prints is the model itself.

| Rule | Detail |
|---|---|
| Enum names | `snake_case` (`ToolStatus::Ok` is `"ok"`). `Agent` is lowercase (`"claude"`, `"codex"`, `"opencode"`, `"gemini"`) |
| Entry kind | An `Entry` is flattened. The kind is the `"type"` key next to the entry fields: `message`, `compaction`, `turn`, `settings`, `notice`, `hook`, `error`, `rewind`, `other` |
| Part kind | `"type"` key: `text`, `reasoning`, `attachment`, `tool_call`, `tool_result`, `other` |
| Tool kind | `"kind"` key: `shell`, `process`, `read`, `write`, `edit`, `patch`, `search`, `glob`, `list`, `web_fetch`, `web_search`, `todo`, `task`, `mcp`, `code`, `question`, `plan`, `skill`, `other` |
| Source kind | `"kind"` key: `file` or `sqlite` |
| Omitted fields | A `None`, an empty list, and a `false` flag are left out. They read back as the default, so a round trip through JSON gives an equal value |

A real session header, from `agentparse show tests/fixtures/claude/main-session.jsonl --json` with `entries` removed:

```json
{
  "agent": "claude",
  "id": "3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11",
  "source": { "kind": "file", "path": "tests/fixtures/claude/main-session.jsonl" },
  "format": "claude-jsonl",
  "title": "demo-fix-session",
  "cwd": "/work/demo",
  "git": { "branch": "main" },
  "cli_version": "2.1.296",
  "models": ["claude-sonnet-5-5"],
  "started_at": 1791795600100,
  "updated_at": 1791795659200,
  "usage": { "input": 50, "output": 1936, "reasoning": 40, "cache_read": 300418, "cache_write": 5796 },
  "extra": { "ai-title": "Fix the failing demo test", "entrypoint": "cli", "userType": "external" }
}
```

## Agent

| Variant | `as_str()` |
|---|---|
| `Claude` | `claude` |
| `Codex` | `codex` |
| `Opencode` | `opencode` |
| `Gemini` | `gemini` |

`Agent::ALL` lists the four. `"codex".parse::<Agent>()` works and ignores case.

## Session

| Field | Type | Meaning |
|---|---|---|
| `agent` | `Agent` | The product that wrote the session |
| `id` | `String` | The product's session id. A Claude subagent uses its `agentId`, a Codex child thread its own thread id. Ids can repeat across files (resumed or migrated copies), so `(agent, source)` is what identifies a session |
| `source` | `Source` | Where it was read from |
| `format` | `String` | The on-disk generation: `claude-jsonl`, `codex-flat`, `codex-legacy`, `codex-paginated`, `opencode-v1`, `opencode-v2`, `opencode-v1+v2`, `opencode-json`, `gemini-g1`, `gemini-g2`, `gemini-g3` |
| `parent` | `Option<ParentLink>` | Set on a subagent or child session |
| `title` | `Option<String>` | The product's own title when it has one, else the first human prompt shortened to one line |
| `cwd` | `Option<String>` | Working directory |
| `git` | `Option<Git>` | Branch, commit and remote when recorded |
| `cli_version` | `Option<String>` | Version of the product that wrote the session |
| `models` | `Vec<String>` | Distinct models in order of first use. Placeholder models such as `<synthetic>` are left out |
| `started_at` | `Option<i64>` | Epoch milliseconds UTC. The recorded value, else the earliest entry time |
| `updated_at` | `Option<i64>` | Epoch milliseconds UTC. The recorded value or the latest entry time, whichever is later |
| `usage` | `Usage` | Sum of message and compaction usage. `cost_usd` is the product's recorded session cost when one exists, else the sum of recorded message costs |
| `entries` | `Vec<Entry>` | The timeline in source order |
| `extra` | `Option<Value>` | Session-level keys of the source that no field above took, as recorded |
| `warnings` | `Vec<Warning>` | Records that could not be read |

### Source

| Variant | Fields | Meaning |
|---|---|---|
| `File` | `path` | A transcript file (Claude Code, Codex, Gemini CLI) or an OpenCode JSON storage session file |
| `Sqlite` | `path`, `session_id` | A set of rows in an OpenCode database |

### ParentLink

| Field | Type | Meaning |
|---|---|---|
| `session_id` | `String` | The session that spawned this one |
| `call_id` | `Option<String>` | The spawning tool call in the parent, when recorded |
| `agent_type` | `Option<String>` | Subagent type, Codex agent path or nickname |

### Git

| Field | Type |
|---|---|
| `branch` | `Option<String>` |
| `commit` | `Option<String>` |
| `remote` | `Option<String>` |

### Warning

| Field | Type | Meaning |
|---|---|---|
| `line` | `Option<usize>` | 1-based source line, for files |
| `message` | `String` | What went wrong |

A corrupt JSON line, a bad database row, and a tool result without its call each become a warning. The rest of the session still parses. See [Usage and Status](#conventions) for what is an error and what is a warning.

## Entry

| Field | Type | Meaning |
|---|---|---|
| `id` | `Option<String>` | The product's id. Claude: the API `message.id` for an assistant message (`<message.id>#2`, `#3` for the pieces after a context line), else the line uuid. Codex: response id, item id, or `ord:<n>`. OpenCode: message or row id. Gemini: message id |
| `parent_id` | `Option<String>` | Claude only: `parentUuid` resolved to the entry that absorbed that line. Never invented |
| `turn_id` | `Option<String>` | Turn grouping where recorded: Codex `turn_id`, Claude `promptId`, OpenCode the triggering user message |
| `ts` | `Option<i64>` | Epoch milliseconds UTC |
| `line` | `Option<usize>` | 1-based line of the first source record, for files. Relative to the window on tail reads |
| `extra` | `Option<Value>` | Keys of the source record that no typed field took, as recorded |
| `kind` | `EntryKind` | What the entry is. Flattened into the entry as the `"type"` key |

### EntryKind

| Variant | Payload | Meaning |
|---|---|---|
| `Message` | `Message` | A user, assistant or system message |
| `Compaction` | `Compaction` | Context was summarized |
| `Turn` | `Turn` | End of a turn with its outcome, or an interrupt marker |
| `Settings` | `Settings` | Model, agent, cwd or permission settings changed |
| `Notice` | `Notice` | A harness or system record the model did not see |
| `Hook` | `Hook` | A hook program the harness ran |
| `Error` | `ErrorInfo` | A provider or API error, or a retry |
| `Rewind` | `Rewind` | History the product rolled back |
| `Other` | `Other` | A record this crate does not model, kept as recorded |

## Message

| Field | Type | Meaning |
|---|---|---|
| `role` | `Role` | `User`, `Assistant` or `System` |
| `origin` | `Origin` | Who produced the content |
| `kind` | `Option<String>` | The product's own label for content no human typed and no model generated (`task-notification`, `system-reminder`, `environment_context`, `local-command-stdout`, and so on) |
| `author` | `Option<String>` | The sender when it is another agent (a Codex agent path, a Claude teammate) |
| `model` | `Option<String>` | The model that wrote it |
| `agent` | `Option<String>` | Agent or mode, only where the product records it per message (OpenCode) |
| `phase` | `Option<String>` | Codex `commentary` or `final_answer`. `queued` for input that was still waiting in the queue (Claude `queue-operation`, OpenCode `session_pending`) |
| `parts` | `Vec<Part>` | Ordered content |
| `usage` | `Option<Usage>` | Tokens for the model call behind this message |
| `stop_reason` | `Option<String>` | Why generation stopped, as recorded |

### Role

| Value | Meaning |
|---|---|
| `User` | Input to the model |
| `Assistant` | Output of the model |
| `System` | Harness text the model saw (Codex developer messages, OpenCode system rows) |

### Origin

| Value | Meaning |
|---|---|
| `Human` | Typed by a person |
| `Model` | Generated by the model |
| `Command` | A slash command, a shell escape (`!cmd`), or its output |
| `Context` | Context the harness injected: environment, AGENTS.md, reminders, skill bodies, file mentions |
| `Notification` | Background task, hook or queue notices delivered as input |
| `Summary` | A resume or away summary that is not a compaction record |
| `Agent` | Another agent: inter-agent messages and a subagent's prompt |
| `Tool` | Holds a tool result whose call is outside the parsed records |

`Origin::is_synthetic()` is true for everything except `Human` and `Model`.

### Part

| Variant | Payload | Meaning |
|---|---|---|
| `Text` | `text` | Plain text |
| `Reasoning` | `text: Option<String>`, `encrypted: bool` | Model reasoning. `text` is `None` when only an opaque blob was stored. `encrypted` says such a blob or signature exists. The blob itself is never kept |
| `Attachment` | `Media` | An image or file reference |
| `ToolCall` | `ToolCall` | A tool invocation with its result |
| `ToolResult` | `ToolResult` | A result whose call is outside the parsed records, for example on tail reads |
| `Other` | `kind`, `data` | A block this crate does not model, kept in place |

### Media

| Field | Type | Meaning |
|---|---|---|
| `mime` | `Option<String>` | Media type |
| `name` | `Option<String>` | File name |
| `url` | `Option<String>` | An http(s) or file URL, or a path. Never a `data:` URL |
| `bytes` | `Option<u64>` | Decoded size of inline data. The data itself is never kept |

## Tool calls and results

A `ToolCall` is one invocation. A `Tool` is the typed view of its input (see [Tool Calls](#tools)).

### ToolCall

| Field | Type | Meaning |
|---|---|---|
| `id` | `String` | The call id |
| `name` | `String` | The tool name as recorded |
| `input` | `Value` | The input as recorded: parsed JSON arguments, or a string for freeform input |
| `tool` | `Tool` | The typed view of the input |
| `status` | `ToolStatus` | Normalized outcome |
| `status_detail` | `Option<String>` | The product's own status or denial label, as recorded |
| `output` | `Option<ToolOutput>` | The result. `None` while there is no result |
| `started_at` | `Option<i64>` | Epoch milliseconds UTC |
| `ended_at` | `Option<i64>` | Epoch milliseconds UTC |
| `child_session` | `Option<String>` | The session this call spawned or addressed (subagents) |
| `children` | `Vec<ToolCall>` | Calls made inside this one (Codex code-mode cells) |

`ToolCall::failed()` is true for `Error`, `Rejected` and `Interrupted`.

### ToolStatus

| Value | Meaning |
|---|---|
| `Pending` | No result recorded: still running, or the record is cut |
| `Ok` | Succeeded |
| `Error` | The tool failed, or a shell command exited non-zero or timed out |
| `Rejected` | Denied by the user or a permission rule |
| `Interrupted` | Aborted by the user, or by the run ending |

How each product's own status maps to these is in [Usage and Status](#conventions).

### ToolOutput

| Field | Type | Meaning |
|---|---|---|
| `text` | `String` | The result as the model saw it |
| `shell` | `Option<ShellResult>` | Exit code, streams and timing for shell commands |
| `files` | `Vec<FileChange>` | Files the call changed |
| `attachments` | `Vec<Media>` | Images and files in the result |
| `data` | `Option<Value>` | The structured result as recorded, minus values already in typed fields |
| `truncated` | `bool` | The output was cut by the product |
| `full_output_path` | `Option<String>` | Where the product saved the untruncated output |

### ToolResult

Only appears as `Part::ToolResult`, when the matching call is not in the parsed records.

| Field | Type |
|---|---|
| `call_id` | `String` |
| `status` | `ToolStatus` |
| `status_detail` | `Option<String>` |
| `output` | `ToolOutput` |

### FileChange

| Field | Type | Meaning |
|---|---|---|
| `path` | `String` | The changed file |
| `op` | `FileOp` | `Add`, `Update`, `Delete` or `Move` |
| `move_to` | `Option<String>` | The new path for a move |
| `diff` | `Option<String>` | A unified diff, or the patch body for this file |
| `additions` | `Option<u64>` | Added lines |
| `deletions` | `Option<u64>` | Removed lines |

## Usage

| Field | Type | Meaning |
|---|---|---|
| `input` | `u64` | Uncached prompt tokens |
| `output` | `u64` | Every generated token, including reasoning |
| `reasoning` | `u64` | The part of `output` spent reasoning. 0 when unknown |
| `cache_read` | `u64` | Prompt tokens read from cache |
| `cache_write` | `u64` | Prompt tokens written to cache |
| `cost_usd` | `Option<f64>` | Only when the product recorded a cost |

`Usage::total()` is `input + output + cache_read + cache_write`. `Usage::add()` adds another `Usage` with saturating arithmetic. The billable prompt is `input + cache_read + cache_write`. The per-product mapping is in [Usage and Status](#conventions).

## Events

### Compaction

| Field | Type | Meaning |
|---|---|---|
| `summary` | `Option<String>` | The summary text |
| `trigger` | `Option<String>` | `auto` or `manual`, when recorded |
| `tokens_before` | `Option<u64>` | Context size before |
| `tokens_after` | `Option<u64>` | Context size after |
| `logical_parent` | `Option<String>` | Claude: the entry the conversation logically continues from |
| `usage` | `Option<Usage>` | Usage of the model call that wrote the summary, when recorded separately |

The carrier record of a summary (a Claude compact summary line, an OpenCode summary message, a Codex `compacted` record) is absorbed into this entry and is not emitted a second time.

### Turn

| Field | Type | Meaning |
|---|---|---|
| `status` | `Option<String>` | `completed`, `interrupted`, `failed`, or the product's own word |
| `duration_ms` | `Option<u64>` | Turn duration |
| `error` | `Option<String>` | Error text for a failed turn |

### Settings

Emitted when a setting changes. All fields are `Option<String>`.

| Field | Meaning |
|---|---|
| `model` | Model |
| `agent` | Agent or mode |
| `cwd` | Working directory |
| `effort` | Reasoning effort |
| `approval` | Approval policy or permission mode |
| `sandbox` | Sandbox policy |

### Notice

| Field | Type | Meaning |
|---|---|---|
| `level` | `Option<String>` | `info`, `warning` or `error`, when recorded |
| `kind` | `Option<String>` | Product subtype |
| `text` | `String` | The notice text |

### Hook

| Field | Type | Meaning |
|---|---|---|
| `event` | `Option<String>` | Hook event, such as `PreToolUse` or `Stop` |
| `name` | `Option<String>` | Hook name |
| `command` | `Option<String>` | The hook command |
| `exit_code` | `Option<i32>` | Exit code |
| `stdout` | `Option<String>` | Standard output |
| `stderr` | `Option<String>` | Standard error |
| `duration_ms` | `Option<u64>` | Run time |
| `blocked` | `bool` | The hook blocked or cancelled the action |

### ErrorInfo

| Field | Type | Meaning |
|---|---|---|
| `kind` | `Option<String>` | Error type as recorded |
| `message` | `String` | Error text |
| `status` | `Option<u16>` | HTTP status when recorded |
| `retry` | `Option<u32>` | Retry attempt number when this is a retry notice |

### Rewind

| Field | Type | Meaning |
|---|---|---|
| `dropped` | `Vec<String>` | Entry ids that left the active history. They stay in `entries` |
| `turns` | `Option<u32>` | Number of turns rolled back, when that is what was recorded |

### Other

| Field | Type | Meaning |
|---|---|---|
| `kind` | `String` | The product's record type, such as `file-history-snapshot` or `world_state` |
| `data` | `Value` | The record as recorded |

## Helpers

| Method | Returns |
|---|---|
| `Session::messages()` | Iterator over every `Message`, in order |
| `Session::tool_calls()` | `Vec<&ToolCall>`, depth first, so nested `children` follow their parent |
| `Message::text()` | The `Text` parts joined with blank lines |
| `Message::is_human()` | True for role `User` with origin `Human` |
| `ToolCall::failed()` | True for `Error`, `Rejected`, `Interrupted` |
| `Origin::is_synthetic()` | True for any origin except `Human` and `Model` |
| `Usage::total()`, `Usage::add()` | Sum of the four token counts, saturating add |

## Subagents

A subagent or child thread is its own `Session` with `parent` set. The call that spawned it has `child_session` set to the child's id. `discover` lists Claude Code, Gemini CLI and OpenCode children as separate `SessionRef`s with `parent_id` set. A Codex child rollout is listed like any other rollout, and its `parent` link is only known after you parse it.

```rust
use agentparse::{Roots, discover, parse};

fn main() -> Result<(), agentparse::Error> {
    let (refs, _) = discover(&Roots::from_env());
    for child in refs.iter().filter(|r| r.parent_id.is_some()) {
        let session = parse(child)?;
        let link = session.parent.as_ref().expect("a child session has a parent link");
        println!("{} is a child of {} (call {:?})", session.id, link.session_id, link.call_id);
    }
    Ok(())
}
```

On the repository fixtures this prints one line per child:

```
a1b2c3d4e5f607182 is a child of 3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11 (call Some("toolu_01DemoAgent14xxxxxxxxxxx"))
ses_ecd6a835ff7fukfa0R96znWxUu is a child of ses_ecd6ad17ff80AEbRxnOg8B21Mn (call Some("call_demov207"))
ses_ed52a633ff93Y7Vatlwl1zyPKr is a child of ses_ed52a75fff9cZv864nVGS5jPGC (call Some("call_demom002"))
```

In `agentparse show`, a spawning call prints `-> session <child id>`.

## Next

- [Tool Calls](#tools)
- [Shell Commands](#shell)
- [Usage and Status](#conventions)
