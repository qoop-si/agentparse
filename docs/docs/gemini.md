# Gemini CLI

Gemini CLI saves each chat as one file. Three generations of the format exist, and one file can mix them. agentparse reports the newest generation it finds in a file.

## Where the files are

| Kind | Path under the Gemini directory (`~/.gemini`, or `$GEMINI_CLI_HOME/.gemini`) |
|---|---|
| Chat | `tmp/<project>/chats/session-<UTC minute>-<short id>.json` or `.jsonl` |
| Subagent chat | `tmp/<project>/chats/<parent session id>/<agent id>.jsonl` |
| Project root | `tmp/<project>/.project_root`, the raw project path. Source of `Session.cwd` |

`<project>` is a slug of the project folder name. Older versions used a hash.

Resuming a `.json` chat with a newer CLI writes a `.jsonl` copy next to it and leaves the old file. Discovery lists only the `.jsonl` in that case. A subagent's `parent_id` is the folder name.

For a JSONL file, `Session.id` is the `sessionId` in the header line. For a `.json` file it is the `sessionId` in the document, or the file name if that is missing.

## Format generations

| `format` | File | How it is written | Reading it |
|---|---|---|---|
| `gemini-g1` | One `.json` document: `{sessionId, projectHash, startTime, lastUpdated, messages[], ...}` | The whole file is rewritten on every save. CLI 0.4 up to 0.38 | Parsed whole |
| `gemini-g2` | `.jsonl` with a header line, message records, and `$set` lines. A `$set` with `messages` carries a full snapshot that replaces the list | Snapshot style. CLI 0.39 and later | Records are replayed in order |
| `gemini-g3` | `.jsonl` with a header, message records, `$set` for metadata, plus `$patch` and `$rewindTo` lines | Append-only deltas. Newest | Records are replayed in order |

The format is `gemini-g3` once a `$patch` or `$rewindTo` line exists, or messages are appended without any snapshot. It is `gemini-g1` for one JSON document, and `gemini-g2` otherwise. A mixed or resumed file counts as its newest generation.

The replay follows the CLI's own rules:

| Record | Effect |
|---|---|
| A record with a string `id` | Insert, or replace the message with that id. A replaced message keeps its original position |
| `{"$set": {...}}` | Merge metadata. If it has `messages`, those replace the whole list |
| `{"$patch": {...}}` | Update a message's content or a tool call's result, remove ids, or reorder |
| `{"$rewindTo": "<id>"}` | Remove that message and everything after it |
| An unparsable line | A warning with the line number. Live files can end mid-line |

The same message id is appended again each time it changes, so duplicate ids in one file are normal.

## All history is kept

Snapshots and rewinds make messages leave the active conversation. agentparse still keeps them. A message that was dropped stays in `entries` in its original position, and a `Rewind` entry lists the ids that left the active history in `dropped`. See [Usage and Status](#conventions).

## How records become entries

| Source | Becomes |
|---|---|
| `user` record typed by a person | `Message` with origin `Human` |
| `user` record starting with `<session_context>` | `Message` with origin `Context` and `kind: "session_context"` |
| `user` record starting with `<hook_context>` | `Message` with origin `Context` and `kind: "hook_context"` |
| A slash command such as `/help` | `Message` with origin `Command` and `kind: "slash_command"` |
| A user turn holding a `<state_snapshot>` summary (history compression) | `Compaction` with the summary. A later "chat history compressed from X to Y tokens" notice fills `tokens_before` and `tokens_after`. The model's "Got it" reply gets `kind: "compaction_ack"` |
| `gemini` record | One assistant `Message` per record, with thoughts, text and tool calls |
| Echo `user` turns that only hold tool responses | Folded into the matching call. They are not user messages |
| A tool response whose call is not in the file | A message with origin `Tool` holding `Part::ToolResult`, plus a warning |
| `info`, `warning` records | `Notice` |
| `error` record | `Error` |
| Messages removed by `$rewindTo`, `$patch` or a snapshot | Stay in place, plus a `Rewind` entry |
| Subagent file, first user turn | Origin `Agent` instead of `Human` |

In a subagent chat, a call to another agent (a tool with an `agentId`) is a `Tool::Task` and carries the child id in `child_session`.

### Reasoning

Gemini stores thoughts as readable text, so `Part::Reasoning` has `text`. `encrypted` is true when the part also carries an opaque `thoughtSignature`. The signature itself is dropped.

### Tool results

A `gemini` record holds its tool calls together with their results.

| Field | Source |
|---|---|
| `ToolCall.name` and `input` | The name and arguments the model requested |
| `ToolCall.status_detail` | The recorded status: `success`, `error` or `cancelled` |
| `ToolCall.ended_at` | The record's timestamp, which is the completion time. There is no start time |
| `ToolOutput.text` | The result text the model saw |
| `ToolOutput.files` | A file diff in the result display, with a flag for new files |
| `ToolOutput.data` | The tool call record minus `args` and `result` |

If a result was replaced by a `<tool_output_masked>` placeholder, agentparse looks for the unmasked result on an earlier line and uses it. If there is none, the output is marked `truncated`.

### Shell results

Gemini CLI records shell output as plain text, so agentparse reads the text:

| Text | Becomes |
|---|---|
| `Exit Code: N` | `exit_code`. Gemini prints it only for a non-zero exit |
| `Output:` | `stdout`, one stream. Versions up to 0.26 have separate `Stdout:` and `Stderr:` sections |
| `Process Group PGID: n` | Kept in `output.data` |
| "moved to background (PID: n)" | `background_id` |
| Timeout text | `timed_out` |
| `Output too large` and a saved path | `truncated` and `full_output_path` |

A non-zero exit is recorded as status `success`, so agentparse sets the normalized status to `Error` from the exit code and keeps `success` in `status_detail`. There is no duration and no separate stderr in modern versions. See [Shell Commands](#shell).

## Usage

One `Message.usage` per `gemini` record.

| Field | Value |
|---|---|
| `input` | `tokens.input` minus `tokens.cached` plus `tokens.tool` |
| `output` | `tokens.output` plus `tokens.thoughts` |
| `reasoning` | `tokens.thoughts` |
| `cache_read` | `tokens.cached` |
| `cache_write` | 0 |
| `cost_usd` | `None`. Gemini records no cost |

## Session fields

| Field | Source |
|---|---|
| `title` | The generated `summary` in the metadata, else the first human prompt |
| `cwd` | `.project_root` next to the `chats` folder, else the first workspace directory in the `<session_context>` turn |
| `models` | The `model` of each `gemini` record, in order of first use |
| `started_at`, `updated_at` | `startTime` and `lastUpdated` |
| `extra` | Header keys no field took, such as `projectHash`, `kind` and `directories` |

Gemini does not record the CLI version or a git branch, so `cli_version` and `git` are always empty.

## Real example

`agentparse show tests/fixtures/gemini/gemini-new-session.jsonl`, trimmed:

```
gemini session aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee  [gemini-g3]
  title: Fix failing parser test
  cwd: /home/user/proj
  models: gemini-3-pro-preview
  usage: in 1100 out 123 cache r6000 w0 reasoning 20

-- user (session_context)  2026-10-12 09:00:00
  <session_context>
  This is the Gemini CLI. Workspace Directories:
    - /home/user/proj
  </session_context>

-- user  2026-10-12 09:00:05
  Run the tests and fix the failure

-- assistant gemini-3-pro-preview  2026-10-12 09:00:09  [in 500 out 50 cache r1500 w0 reasoning 20]
  [reasoning]
  Planning: Run tests first.
  I'll run the tests.
  > $ bun test   (in /home/user/proj)  [error: success]
    exit 1
    output:
      3 pass
      1 fail

-- assistant gemini-3-pro-preview  2026-10-12 09:00:20  [in 500 out 70 cache r2000 w0]
  Fixing the loop bound.
  > edit /home/user/proj/src/parser.ts (1 change)
      Successfully modified file: /home/user/proj/src/parser.ts (1 replacements).
    update /home/user/proj/src/parser.ts +1 -1
  > read /home/user/proj/src/missing.ts  [error: error]
      File not found: /home/user/proj/src/missing.ts

-- assistant gemini-3-pro-preview  2026-10-12 09:00:40
  Cancelled run follows.
  > $ sleep 600  [interrupted: cancelled: User cancelled the operation.]
    interrupted
== error api_error: [API Error: example error text]

-- user  2026-10-12 09:01:00
  Wrong prompt, ignore

-- assistant gemini-3-pro-preview  2026-10-12 09:01:03  [in 100 out 3 cache r2500 w0]
  Okay.
== rewind: 2 entries dropped
```

The shell command failed with exit code 1 although Gemini CLI called it `success`, so the status is `error`. The cancelled `sleep 600` is `interrupted`, with the cancel reason in `status_detail`. At the end, a rewind dropped two entries. They are still in the session, and the `Rewind` entry lists their ids.

The fixture `gemini-old-session.json` is a `gemini-g1` document, and `gemini-snapshot-session.jsonl` is `gemini-g2`.

## Known limits

| Limit | Detail |
|---|---|
| No version or branch | The file records no CLI version and no git branch |
| No cost, no cache writes | Gemini records neither |
| Weak shell data | Exit code, signal and background pid are read from result text. A zero exit code is only set for a success with no timeout or background text. There is no duration and no separate stderr in modern versions |
| Tool completion time only | `started_at` is empty. `ended_at` is the completion time |
| MCP names are a guess | `mcp_<server>_<tool>` is split at the first `_` after `mcp_`, and server names can contain `_`. The raw name is in `ToolCall.name` |
| Torn lines | A line cut off by a crash cannot be read. It shows up as a warning with its line number |
| Tail reads | `parse_file_tail` does not window Gemini files. It parses the whole file |
