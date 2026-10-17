# Claude Code

Claude Code writes one JSONL file per session. Every line is one JSON object. agentparse reports this format as `claude-jsonl`.

## Where the files are

| Kind | Path under the config directory (`~/.claude`, or `CLAUDE_CONFIG_DIR`) |
|---|---|
| Main session | `projects/<project>/<session id>.jsonl` |
| Subagent | `projects/<project>/<session id>/subagents/agent-<agent id>.jsonl` |
| Subagent sidecar | `projects/<project>/<session id>/subagents/agent-<agent id>.meta.json` (one JSON object, not JSONL) |
| Workflow subagent | `projects/<project>/<session id>/subagents/workflows/<workflow id>/agent-<name>-<hex>.jsonl`, same line shape as a subagent |
| Workflow journal | `.../workflows/<workflow id>/journal.jsonl`. Not a session. Discovery skips it |

`<project>` is the working directory with every `/` and `.` replaced by `-`. That is lossy and cannot be reversed, so agentparse takes the real directory from the `cwd` key on the lines (`Session.cwd`).

`discover` lists main sessions and subagents. A subagent's `parent_id` is the `parentAgentId` from its sidecar when it has one (a nested subagent), otherwise the parent session id taken from the directory name.

## How lines become entries

Claude Code writes one line per content block, and several lines can share one API `message.id`. agentparse merges those lines back into one assistant message, in file order.

| Source | Becomes |
|---|---|
| `assistant` lines with the same `message.id` | One `Message` (role `Assistant`, origin `Model`). `Entry.id` is the `message.id`, and the line ids are listed in `extra.uuids` |
| `thinking` blocks | `Part::Reasoning`. `encrypted` is true when only a signature was stored |
| `tool_use` block plus its `tool_result` | One `Part::ToolCall`. The result is folded in by `tool_use_id`. `toolUseResult` fills `ShellResult`, `FileChange` and `ToolOutput.data` |
| `user` line typed by a person | `Message` with origin `Human` |
| `user` line with a task notification | `Message` with origin `Notification` and `kind: "task-notification"` |
| `<command-name>` and `<local-command-stdout>` text | `Message` with origin `Command` |
| `<bash-input>`, `<bash-stdout>`, `<bash-stderr>` | One `Command` message holding a `Shell` call. The call id is the `<bash-input>` line uuid |
| Injected text: reminders, skill bodies, file mentions, `isMeta` lines | `Message` with origin `Context` and a `kind` such as `system-reminder` or `meta` |
| `system` `turn_duration` | `Turn` |
| `system` `api_error` | `Error`, with the retry number |
| `system` `compact_boundary` and the compact summary line | `Compaction` with `trigger`, `tokens_before`, `tokens_after` and `logical_parent` |
| `system` `local_command` | A `Command` message |
| Other `system` subtypes | `Notice`, with the subtype in `kind` |
| `away_summary` | `Message` with role `System`, origin `Summary`, `kind: "away_summary"` |
| Hook attachments (`hook_success`, `hook_error`, ...) and `stop_hook_summary` | `Hook` |
| API error messages (`isApiErrorMessage`) | `Error` |
| Other placeholder messages from the model `<synthetic>` (such as "No response requested.") | `Notice` with `kind: "synthetic"` |
| `attachment` lines the UI shows as one line (`file`, `directory`, `nested_memory`, `compact_file_reference`, `invoked_skills`, `diagnostics`, `goal_status`, `task_status`) | `Notice` with the attachment type as `kind` and that line as `text` (`Read a.rs (12 lines)`). The record stays in `extra` |
| other `attachment` lines with model-visible text | `Message` with origin `Context`. Without such text: `Other` with kind `attachment.<subtype>` |
| `queue-operation` lines | Replayed in order (enqueue, dequeue, remove, popAll). Input still queued at the end becomes a human message with `phase: "queued"`. Texts that start with `<` are notifications. The raw lines also stay as `Other` |
| `file-history-snapshot`, `last-prompt`, `pr-link`, `worktree-state`, and other metadata lines | `Other` |
| `TaskCreate`, `TaskUpdate` | `Tool::Todo` with one item (the subject or `#<taskId>`, and its status): the call names one task, not the list |
| `Workflow` | `Tool::Task` with `agent_type: "workflow"`; the description is the `summary` of the launch result |
| A typed `/compact` (or any slash command) written as a prompt and again as `<command-name>` | One human message: the expanded line is dropped when the human message just before it holds the same text |
| A prompt sent again after Esc or a rewind | The first try and everything below it are dropped, with a warning, when the sibling prompt is on the chain that goes on (only prompt heads are dropped) |

Settings entries appear when the working directory, permission mode, effort or agent setting changes.

### Ids and links

| Field | Value |
|---|---|
| `Entry.id` | `message.id` for a merged assistant message, else the line `uuid` |
| `Entry.parent_id` | `parentUuid`, resolved to the entry that absorbed that line. A `tool_result` line maps to the message holding its call |
| `Entry.turn_id` | The `promptId` of the last user line, carried forward |
| `Compaction.logical_parent` | `logicalParentUuid`, resolved the same way |

### Title

The first of these that exists: the latest `custom-title`, then `agent-name`, then `ai-title`, then an old `summary` line, then the first human prompt. A subagent uses the `description` from its sidecar.

### Usage

Usage is attached once per API message, taken from the last line of that id. Early chunks carry partial output. `stop_reason` also comes from the last line. A message with no stop reason anywhere (aborted mid-stream) keeps its parts. Session cost is the latest `cost-state` record. See [Usage and Status](#conventions).

## Subagents

Each subagent file is its own `Session`.

| Link | Value |
|---|---|
| `Session.id` | The `agentId` |
| `ParentLink.session_id` | The sidecar `parentAgentId`, else the line `sessionId` |
| `ParentLink.call_id` | The sidecar `toolUseId` |
| `ParentLink.agent_type` | The sidecar `agentType` |
| `ToolCall.child_session` on the parent's call | `toolUseResult.agentId`, or `agent_id`, or the `<task-id>` of an async notification |

## Background commands

A background launch (`Bash` with `run_in_background`, an async `Agent`, `Monitor`, `Workflow`) is not an outcome: the call is `Pending`, without `ended_at`, and its `backgroundTaskId` (or `agentId`, `taskId`) is indexed. A later task notification message (origin `Notification`) names the call by its tool use id, or by `<task-id>` when it has none, and ends it: `completed` and `done` are `Ok`, `failed` is `Error`, `killed` and `stopped` are `Interrupted`, with the exit code from the summary when it has one. `running` and an agent that reports while its own background work still runs change nothing. Other records end a launch too: a successful `TaskStop`, `system` `agents_killed`, a `turn_duration` whose `pendingBackgroundAgentCount` (or `pendingWorkflowCount`) is 0, a `task_status` attachment, and a Monitor past its `timeout_ms` (judged against the newest record of the file). `BashOutput` and `TaskOutput` calls become `Tool::Process` reads of the same id.

## Real example

`agentparse show tests/fixtures/claude/main-session.jsonl`, trimmed:

```
claude session 3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11  [claude-jsonl]
  title: demo-fix-session
  cwd: /work/demo
  branch: main
  models: claude-sonnet-5-5
  usage: in 50 out 1936 cache r300418 w5796 reasoning 40
== hook SessionStart /work/demo/.claude/hooks/session-start.sh exit 0

-- user  2026-10-12 09:00:02
  fix the failing test in @test.sh

-- assistant claude-sonnet-5-5  2026-10-12 09:00:05  [in 2 out 87 cache r10191 w1431]
  > $ sh test.sh   (in /work/demo)  [error]
    exit 3
    output:
      out-line
      err-line

-- assistant claude-sonnet-5-5  2026-10-12 09:00:27  [in 2 out 110 cache r13600 w140]
  > task general-purpose: Survey demo tests
      Async agent launched successfully.
    -> session a1b2c3d4e5f607182

-- assistant claude-sonnet-5-5  2026-10-12 09:00:41  [in 2 out 75 cache r14710 w130]
  I will remove the build directory.
  > $ rm -rf build   (in /work/demo)  [rejected: user-rejected]

== compaction (auto) 177627 -> 3771 tokens  2026-10-12 09:00:54
```

## Known limits

| Limit | Detail |
|---|---|
| Failed commands have merged streams | An error result is text only. The exit code is read from the `Exit code N` text, and `stderr` stays `None`. Successful results with an object keep `stdout` and `stderr` apart and get exit code 0 (the CLI reports every other non-zero exit as an error), except soft failures carrying `returnCodeInterpretation`, which keep `None` |
| No shell duration | Claude Code does not record how long a command ran, so `ShellResult.duration_ms` is `None`. Use the call's `started_at` and `ended_at` for a rough figure |
| Timestamps can go backwards | Order is always file order, never sorted by time |
| Some lines have no time | Metadata lines (titles, permission mode, cost state) carry no timestamp in the source, so there is nothing to put in `ts` |
| Very large lines | Single lines can be several megabytes, mostly inline images and big file reads. The parser does not use a fixed line buffer. Base64 bodies are not kept |
| Old transcripts | Transcripts from before CLI 2.1.143 (for example the `summary` and `progress` lines, and older tool names) are handled, but the repository fixture for them is reconstructed from the old format, not recorded from a real old session |
| Tail reads | `parse_file_tail` can cut a session so that a result has no call in the window. That result stays as `Part::ToolResult` with a warning |
