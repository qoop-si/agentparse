# Codex

Codex writes one JSONL rollout file per thread. Each line is a record. agentparse reads three generations of the format and optional zstd compression.

## Where the files are

| Kind | Path under the Codex home (`~/.codex`, or `CODEX_HOME`) |
|---|---|
| Rollout | `sessions/YYYY/MM/DD/rollout-<local time>-<thread uuid>.jsonl` |
| Rollout copy | The same name with `_<rollout uuid>` after the thread uuid |
| Compressed rollout | `rollout-....jsonl.zst`, decoded on the fly |
| Archived | `archived_sessions/`, same layout |

`discover` walks `sessions` and `archived_sessions` up to four directories deep and takes the thread uuid from the file name as the session id. The same thread can show up twice (a live and an archived copy), so identify a session by `(agent, source)`, not by id.

Codex also keeps SQLite databases, a prompt history file and config. agentparse reads none of them. Everything comes from the rollout files.

## Format generations

`Session.format` says which generation a file is.

| `format` | How it is recognized | Notes |
|---|---|---|
| `codex-flat` | The first line has no `type` key | The earliest era |
| `codex-legacy` | Envelope records `{timestamp, type, payload}` with `history_mode` legacy or absent | Items carry no ids. User text comes from `user_message` events. Assistant text from response items, and the matching `agent_message` event copy is skipped |
| `codex-paginated` | Envelope records with `history_mode: paginated`, each line carrying an `ordinal` | The newest generation. User text comes from `UserMessage` events |

The envelope looks like this. `ordinal` is dense and starts at 0, and it gives entry order:

```json
{"timestamp":"2026-10-13T01:34:18.596Z","ordinal":8,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"run the demo checks"}]}}
```

## How records become entries

| Source | Becomes |
|---|---|
| Assistant-side items up to the next `token_usage_record` | One `Message` (role `Assistant`) per model response. `Entry.id` is the response id, and the item ids are in `extra.item_ids`. `model` is the current turn model |
| Text item `phase` | `Message.phase`: `commentary` or `final_answer`. A response with two phases is split in two, and usage stays on the first piece |
| Developer and context messages | `Message` with role `System` or `User` and origin `Context`. `kind` is the Codex content label, for example `host_skills.instructions`, `permissions.instructions`, `agents_md.instructions`, `environments.environment_context` |
| Reasoning items | `Part::Reasoning`. Codex stores encrypted reasoning, so `text` is usually `None` and `encrypted` is true |
| `function_call`, `custom_tool_call`, `local_shell_call` and their outputs | `Part::ToolCall` with the output attached by `call_id` |
| Extra outputs for the same `call_id` | Appended to the same call |
| Command, file change, MCP, web search and collab items | Enrich the call with the same id, or become a child call. See below |
| `turn_context`, `thread_settings_applied` | `Settings`, emitted when a value changes. The first value counts as a change |
| `task_complete`, `turn_aborted` | `Turn`. The `<turn_aborted>` developer message folds into that one `Turn` |
| `compacted` | `Compaction`. The record whose `response_id` matches the compaction response becomes `Compaction.usage`, not a message. `replacement_history` goes to `extra` |
| `thread_rolled_back` | `Rewind` with `turns` |
| `token_count` (rate limits), `world_state`, `inter_agent_communication_metadata`, a completed `SubAgentActivity`, `task_started` | `Other` |

The same text can arrive both as an event and as a response item. It is emitted once.

### Agent messages

An `agent_message` is `Assistant` when its `author` is this session's own agent path (`/root` for a root thread). Otherwise it is a `User` message with origin `Agent` and `author` set.

### Code mode

Newer models run tools through a JavaScript program (the custom `exec` tool). That program is a `Tool::Code` call. The tool calls it made are its `children`, in order.

| Part | Rule |
|---|---|
| Cell call | `Tool::Code` with the program in `source`. Its status comes from the output header (`Script completed`, `Script failed`, `Script terminated`, `Script running with cell ID`) |
| Child calls | Tool-like items between the call and its output. `name` is the item type (for example `CommandExecution`), `input` is null, and `output.data` is the event |
| Late command events | Joined to the cell whose output JSON has the same `session_id`, including later `write_stdin` outputs |
| `Tool::Process` | `write_stdin` (write) and `wait` (read, or kill with `terminate`) |

`Session::tool_calls()` returns the cell and then its children, so a shell command inside a cell is easy to find.

### Shell details

Codex records one merged stream, so `ShellResult.stderr` is always `None` and `stdout` holds both streams. The order of lines in it is the order Codex captured them, not a reliable split. The exit code comes from the command event, the output JSON or the text headers. Codex records several clocks (the event duration, item start and end times, chunk wall time). `duration_ms` uses one of them, and the others stay in `output.data`. See [Shell Commands](#shell).

## Usage

`Message.usage` is attached per model response. `input` is `input_tokens` minus `cached_input_tokens` minus `cache_write_input_tokens`, so it counts only uncached tokens. Codex records no cost. See [Usage and Status](#conventions).

## Subagents

A child thread has its own rollout file.

| Link | Value |
|---|---|
| `Session.id` | The child's `payload.id` |
| `ParentLink.session_id` | `parent_thread_id` |
| `ParentLink.agent_type` | The agent path or nickname, for example `/root/review` |
| `ToolCall.child_session` on the parent's `spawn_agent` call | Taken from the `SubAgentActivity` item |

A child rollout starts with a copy of history from the parent. Records before `subagent_history_start_ordinal` are dropped, and that ordinal is kept in `Session.extra`.

## Compressed rollouts

A `.jsonl.zst` file is decoded as it is read. If the file records `history_base`, it is kept in `Session.extra` and a warning says that earlier history lives in another rollout. Image generation results that carry base64 become `{"bytes": n}`.

`parse_file_tail` does not window a `.zst` file. It parses the whole file.

## Real example

`agentparse show tests/fixtures/codex/exit3.jsonl`, trimmed:

```
codex session 0190a000-0000-7000-8000-000000000001  [codex-paginated]
  title: run the demo checks
  cwd: /work/demo
  branch: main
  models: gpt-6.1-sol
  usage: in 30447 out 215 cache r38656 w0
   (task_started)

-- system (host_skills.instructions)  2026-10-13 01:34:18
  Synthetic context block 0.

-- user  2026-10-13 01:34:18
  run the demo checks

-- assistant gpt-6.1-sol  2026-10-13 01:34:21  [in 9882 out 94 cache r12288 w0]
  > code cell (3 lines)
    0.8s
    > $ ls -la   (in /work/demo)
      exit 0 | 0.1s
    > $ sh test.sh   (in /work/demo)  [error: failed]
      exit 3 | 0.0s
      output:
        err-line
        out-line
    > $ cat a.txt   (in /work/demo)
      exit 0 | 0.0s
      output:
        one
        two
        three

-- assistant gpt-6.1-sol  2026-10-13 01:34:27  [in 10227 out 87 cache r13184 w0]
  > code cell (1 lines)
    0.2s
    > FileChange
      update /work/demo/a.txt +1 -1
== turn completed 14.7s
```

The `0.8s` under the code cell is the cell's own run time. The three shell commands are its children.

## Known limits

| Limit | Detail |
|---|---|
| One stream | No separate stderr. See above |
| Titles | A rollout carries no title, so `title` is the first human prompt |
| Older shapes are tested on built fixtures | The newest generation has been checked against real rollouts. Older shapes (flat, legacy, direct `shell` calls, `local_shell_call`, `apply_patch`, MCP calls, compaction) are covered by fixtures built from the record types Codex defines |
| Earlier history | A child rollout or a compressed rollout can start mid-history. A warning says so when the file records it |
