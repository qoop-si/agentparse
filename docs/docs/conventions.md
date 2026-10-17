# Usage and Status

The four products count tokens, name outcomes and label content in different ways. agentparse picks one convention for each and maps every product onto it. This page lists the rules.

## Usage

One counting scheme for every product:

| Field | Meaning |
|---|---|
| `input` | Uncached prompt tokens |
| `output` | Every generated token, including reasoning |
| `reasoning` | The part of `output` spent reasoning. 0 when the product does not say |
| `cache_read` | Prompt tokens read from cache |
| `cache_write` | Prompt tokens written to cache |
| `cost_usd` | Only when the product recorded a cost |

The billable prompt is `input + cache_read + cache_write`. `Usage::total()` adds `output` to that. Arithmetic saturates instead of overflowing. A record that contradicts itself (cached tokens larger than input) adds a warning.

How each product's own numbers map:

| Product | `input` | `output` | `reasoning` | `cache_read` | `cache_write` | Cost |
|---|---|---|---|---|---|---|
| Claude Code | `input_tokens` | `output_tokens` | `output_tokens_details.thinking_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens` (the 5 minute and 1 hour split stays in `extra`) | Session: the latest `cost-state` record |
| Codex | `input_tokens` minus `cached_input_tokens` minus `cache_write_input_tokens` | `output_tokens` | `reasoning_output_tokens` | `cached_input_tokens` | `cache_write_input_tokens` | none |
| OpenCode | `tokens.input` | `tokens.output` plus `tokens.reasoning` | `tokens.reasoning` | `tokens.cache.read` | `tokens.cache.write` | `cost` |
| Gemini CLI | `tokens.input` minus `tokens.cached` plus `tokens.tool` | `tokens.output` plus `tokens.thoughts` | `tokens.thoughts` | `tokens.cached` | 0 | none |

Where usage is attached:

| Product | One `Message.usage` per |
|---|---|
| Claude Code | API message id. Lines of one message are merged, and usage comes from the last line of that id, because early chunks carry partial output |
| Codex | Model response (`token_usage_record`), with the cumulative `token_count` as a fallback that is deduplicated by total |
| OpenCode | Assistant message. Step finish token counts are never added a second time |
| Gemini CLI | `gemini` record |

`Session.usage` is the sum of message usage and compaction usage. `cost_usd` on the session is the product's recorded session cost when one exists (Claude Code), else the sum of recorded message costs. If a product records a total that does not fit the scheme, it stays in `extra`.

A message that is split in two (for example a Codex response with two phases) keeps its usage on the first piece only, so nothing is counted twice.

## Tool status

`ToolStatus` is one of five values. The product's own label is kept in `ToolCall.status_detail`.

| Status | Claude Code | Codex | OpenCode | Gemini CLI |
|---|---|---|---|---|
| `Ok` | A result with `is_error` false. This includes soft failures flagged by `returnCodeInterpretation`, which have no exit code | An output, exit 0, or `completed` | `completed` | `success`, without timeout text |
| `Error` | `is_error` true, `Exit code N`, or a blocking hook | `failed`, `Script failed`, `Script error:` | `error` (`tool.execution`) | `error`. Also `success` with a non-zero `Exit Code` or the timeout text |
| `Rejected` | A denial kind of user rejected, permission rule, or automode | `declined` | An error text saying the user rejected permission | `cancelled` with a denial reason |
| `Interrupted` | A denial kind of interrupted or cancelled, or session-ended text | `Script terminated`, or a turn aborted with no output | `metadata.interrupted`, `tool.interrupted`, or pending or running in a run that finished | `cancelled` by a user abort |
| `Pending` | No result, and no later assistant message or turn end in the parsed window | `Script running with cell ID` | `pending` or `running` while the session is still active | A null result with nothing after it |

Rules that apply to every product:

- A shell command with a non-zero `exit_code`, or with `timed_out` set, is `Error`, whatever the product called it.
- A call with no result at all becomes `Interrupted` when a later assistant message or turn end exists in the parsed window. Without one, it stays `Pending`.
- A call whose result says it is still running (a background id, a yielded process, a running Codex cell) is never inferred `Interrupted`. It stays `Pending` until its own completion record arrives (a notification, a stop, a poll, the command item). A launch that is never reported on stays `Pending`.
- `ToolCall::failed()` is true for `Error`, `Rejected` and `Interrupted`.

## Roles, origin and kind

`Message.role` says which side of the conversation a message is on. `Message.origin` says who produced it.

| Origin | What it holds |
|---|---|
| `Human` | Text a person typed. `role: User`, `kind: None` |
| `Model` | Text the model generated |
| `Command` | A slash command, a shell escape and its output, and human-run shell commands |
| `Context` | Injected environment, AGENTS.md or CLAUDE.md text, reminders, skill bodies, file mentions |
| `Notification` | Background task, hook and queue notices delivered as input |
| `Summary` | A resume or away summary that is not a compaction record |
| `Agent` | Messages between agents, and a subagent's prompt |
| `Tool` | A tool result whose call is outside the parsed records |

`Message.kind` holds the product's own label for anything that is not human and not model, so you can tell content apart without parsing text. Examples seen in the fixtures: `task-notification`, `system-reminder`, `local-command-stdout`, `command-name`, `meta`, `environments.environment_context`, `agents_md.instructions`, `session_context`.

| Rule | Detail |
|---|---|
| Mixed records are split | If one source record holds parts of different kinds, it becomes consecutive messages with ids suffixed `#1`, `#2` |
| System versus notice | `Role::System` is harness text the model saw (Codex developer messages, OpenCode system rows). A `Notice` entry is a record the model did not see |
| Tool results are not user messages | They fold into their call. A result without a call becomes `Part::ToolResult` inside a message with origin `Tool`, plus a warning |
| Interrupts | An interrupt marker (a Claude Code interrupt text, a Codex aborted turn, an OpenCode abort) becomes `Turn` with status `interrupted` |
| Human-run commands | One message with `role: User`, `origin: Command`, holding one `Shell` call and its result. Slash commands and their output are `Command` text messages |
| Compaction | The summary carrier record is absorbed into `Compaction.summary` and not emitted again. Gemini CLI's "Got it" acknowledgment gets `kind: "compaction_ack"` |

## Ids, turns and order

| Topic | Rule |
|---|---|
| Order | Entries stay in source order: line order, Codex `ordinal`, OpenCode `seq` or `(time_created, id)`, Gemini first-seen order. They are never sorted by id strings or timestamps, because timestamps can go backwards inside a file |
| `Entry.id` | The product's id. See the field description on [Sessions and Entries](#model). Ids are not unique across sessions (Gemini context turns share an id between sessions), so scope them to a session |
| `Entry.parent_id` | Claude Code only. `parentUuid` resolved to the entry that absorbed that line. Never invented |
| `Entry.turn_id` | Codex `turn_id`, Claude Code `promptId` carried forward, OpenCode the triggering user message |
| `Entry.line` | 1-based line of the first source record. Relative to the window on tail reads. `None` for databases |
| Session identity | `(agent, source)`. A session id can repeat when a session was resumed or migrated into a second file |
| Timestamps | Epoch milliseconds UTC everywhere. Products that record seconds are converted |

## Nothing lost, one home per value

Each recorded value has exactly one home: a typed field, or else `extra` or `data`.

| Place | What goes there |
|---|---|
| Typed field | Everything the model has a field for |
| `Session.extra` | Session-level keys of the source that no field took, such as Codex session metadata, Gemini header data, OpenCode session row columns |
| `Entry.extra` | Keys of the source record that no typed field took. A Claude Code message merged from several lines lists the line ids in `extra.uuids` |
| `ToolOutput.data` | The structured tool result, minus values already in typed fields (see [Tool Calls](#tools)) |
| `Entry` of kind `Other`, `Part::Other` | Records and blocks the model does not cover. Kept in place, with the product's own kind name |

Two overlaps are deliberate: the raw `ToolCall.input` next to the typed `tool`, and `ToolOutput.text` next to `ShellResult.stdout` and `stderr`.

## What is deliberately not stored

| Not stored | What you get instead |
|---|---|
| Base64 bodies (images, PDFs) | `Media` keeps `mime`, `name`, `url` and `bytes`. Inside `extra` and `data`, a base64 value becomes `{"bytes": n}` |
| `data:` URLs | Replaced the same way, never kept in `Media.url` |
| Encrypted reasoning and signatures | `Part::Reasoning { encrypted: true }`. The blob is dropped |
| Credentials | OpenCode credential and account tables are never read |
| Untruncated tool output | `ToolOutput.truncated` and `full_output_path` point to the product's own copy |

The raw record is still one step away: `Entry.line` gives the line in a file, and the entry id finds the row in a database.

## History that was cut

Claude Code and Gemini CLI can drop or rewrite history. agentparse keeps all of it. Dropped, rewound and compacted messages stay in `entries`, and a `Rewind` or `Compaction` entry marks where the context was cut. You can work out the active context from those markers. You cannot recover lost text from an index that threw it away.

## Warnings and errors

| Problem | Result |
|---|---|
| A corrupt or torn JSON line, a bad database row, a result without its call | A `Warning` on the session. The rest of the session parses |
| A file that is not a session of any product | `Error::NotASession` |
| An unreadable file, or a failure to open the database | `Error::Io` or `Error::Sqlite` |
| A database query or schema failure | `Error::Sqlite`. A broken query never looks like an empty session |
| A session id missing from a database | `Error::NotFound` |
| A missing product directory during discovery | Nothing found. Not an error and not a warning |
| An unreadable directory during discovery | A `Warning` returned next to the found sessions |

Live transcripts often end mid-line while the agent is writing. That torn last line is a warning, not a failure.

## Next

- [Claude Code](#claude)
- [Codex](#codex)
- [OpenCode](#opencode)
- [Gemini CLI](#gemini)
