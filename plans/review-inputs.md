# Inputs for the P4 code review (implementer decisions the plan did not settle)

## Gemini (impl-gemini)
- Format rule: g3 = any `$patch`/`$rewindTo`, or bare records with no `$set.messages`; g1 = one JSON document; else g2.
- Rewind emitted for `$set.messages` drops, `$rewindTo`, `removeIds`; on compaction `Rewind` precedes `Compaction`.
- `<hook_context>` kept as `Message { User, Context, kind: "hook_context" }` (Hook has no text field).
- Slash command detection: `/` + letter word without another `/` -> `Origin::Command`, kind `slash_command`.
- `<state_snapshot>` matched with `contains`; "Got it" ack gets `compaction_ack` when the previous entry is a Compaction;
  "Chat history compressed from X to Y tokens." fills the nearest open Compaction (unverified on real data).
- Thoughts rendered as `**subject** description`; `thoughts[].timestamp` dropped.
- read_file start_line/end_line -> offset = start, limit = end - start + 1; old offset/limit as recorded.
- replace_all = allow_multiple || expected_replacements > 1.
- FileChange counts from the fileDiff text; diffStat stays in data; fileDiff/filePath/isNewFile removed from data.
- Modern shell `Output:` on success with no Exit Code line -> exit 0; old `Exit Code: (none)` -> None.
- Background id = "moved to background (PID: n)" or first Background PID; pgid and pid list in `data`.
- cancelled with a denial word -> Rejected, else Interrupted; status_detail `cancelled: <reason>`.
- Subagent tools (invoke_agent, codebase_investigator, cli_help, generalist, any call with agentId) -> Task.
- REVIEW: binary parts left in a functionResponse echo turn become `Context/tool_attachment` user messages; they
  probably belong in the call's `output.attachments`.

## OpenCode (impl-opencode)
- Module split into `src/opencode.rs` + `src/opencode/{db,record,tool,v1,v2}.rs` (single file was 2100 lines).
- `Message.model` = bare `modelID`; `providerID` in entry `extra`.
- v1 `!cmd` pair -> one Command message; `Entry.id` = the assistant message id, `turn_id` = the user marker id.
- v1 compaction marker + summary message -> one Compaction; `tail_start_id`, `overflow` and leftover keys in `extra`.
- retry parts and assistant error -> Error entries right after the message (message, retries in part order, error).
- Mixed user parts split by origin (`#1`, `#2`); flagged user text (`synthetic`, `ignored`) -> Context; flagged
  assistant text -> `Part::Other` kind `text.synthetic` / `text.ignored`.
- v2 assistant error -> Error, `aborted` type -> Turn interrupted.
- Usage oracle: 60/64 sessions equal `session_v2.tokens_*`; the 4 root v2 sessions have an extra LLM call with no
  `session_message` row (likely title generation, unverified). `Session.usage` stays the sum of recorded rows.
- `subtask` part handled as `Part::Other` but no fixture covers it.
- Needed `serde_json` `float_roundtrip` so summed `cost_usd` survives a JSON round trip (added to Cargo.toml).
- Shell stdout excludes `<bash_metadata>`/`<shell_metadata>` notices and the v2 `Exited with code N` part; `text`
  keeps them; exit from `metadata.exit`, else the trailer.
- Rejected = error text starting "The user rejected permission" or "The user has specified a rule" (no real example).
- Background completion sets `duration_ms` = completion ts minus the call's `ran`; `ended_at` unchanged.
- ParentLink `agent_type` from the title suffix `(@x subagent)`; `call_id` looked up in the parent's rows.
- `Session.extra` holds every session column without a typed home (incl. `cost`, `tokens_*`).
- Dropped on purpose: provider `itemId`s, `file` part `source`, `apply_patch` `metadata.files[].before/after`; v2
  `instruction_*` tables not read.

## Codex (impl-codex)
- Group close: a tool output, user input or turn end also closes an open model response (else a whole legacy turn
  becomes one message). Usage without a record attaches to the last closed group or a usage-less compaction.
- `<turn_aborted>` message + `turn_aborted` event -> one `Turn { interrupted }`, marker text in `extra.marker`.
- Children `output.text` empty (the model saw it inside the cell output); command output in `shell.stdout`.
- `write_stdin` with empty `chars` -> `Process Read` (a poll), not Write.
- `wait(cell_id)` reopens the original cell and updates its state.
- Encrypted Fernet tokens (`gAAAAA...`, e.g. `spawn_agent` message) -> `{"bytes": n}` in `input`; `Task.prompt` None.
- Inter-agent: author == this session's agent path -> Assistant else User; origin Agent; `kind` = message type.
- Declined/unapplied patch changes go to `data`, not `files`.
- Shell clock: event `duration` == envelope `completed_at_ms - started_at_ms` (whole process, measured on a 7.8 s tsc
  and a 39.7 s polled diff); chunk `wall_time_seconds` is only the yield window. `duration_ms` = `duration`.
- Legacy dedupe: response item and event copy matched by text, the first one wins (plan says response item only;
  result text is identical).
- Remaining Other kinds: token_count, world_state, inter_agent_communication_metadata, task_started (no model type),
  SubAgentActivity completed.
- Leftover keys of call/output records -> `Entry.extra.calls.<call_id>.{call,output}` (no `ToolCall.extra`).
  Request: `ToolCall.extra: Option<Value>` instead.
- Old flat `token_count` payloads (about 0.33 to 0.39, no `info`) -> Other, no usage.
- Code-mode `FileChange` children -> `Tool::Other` with `files` filled. Request: `Tool::Patch { patch: Option<String> }`.

## Claude Code (impl-claude)
- `<synthetic>` without `isApiErrorMessage` -> `Notice { kind: "synthetic" }`; `isApiErrorMessage` -> Error.
- `hook_additional_context` -> Context message (model-visible text, no run details); other `hook_*` -> Hook;
  `stop_hook_summary` -> `Hook { event: "Stop", blocked: preventedContinuation }`.
- Attachment role from `renderedRole`; text = `rendered` when present else the attachment's own text field.
- `away_summary` -> `Message { System, Summary, kind: "away_summary" }` (plan text updated).
- Shell: successful object results carry no exit code -> `exit_code` None (320k calls); empty `stderr` -> None
  (Claude often folds stderr into stdout); on timeout the second line is a harness note, stdout after it.
- Deviations from the fixture README (plan wins): synthetic message is an Error entry, call #19 Interrupted, the
  subagent's first prompt is `Origin::Agent`, old `summary` lines whose leafUuid is not in the file do not set the title.
- Deliberate Other: metadata lines (last-prompt, mode, queue-operation, atis-latch, pr-link, file-history-*,
  bridge-session, worktree-state, relocated, frame-link, artifact-*, fork-context-ref, continued-in), structural
  attachments without rendered text, `fallback` parts (model switch marker).
- Corpus: 18,261 files 11.37 GB, 67.9 s, 0 panics, 340 failures = 339 workflow journal.jsonl (NotASession, not
  discovered by `discover`) + 1 zero-byte file; 40 warnings (orphan results); JSON round trip 17,932/17,932; jq
  cross-check 11/11.
- OPEN: a zero-byte transcript returns NotASession; through `parse(SessionRef)` the agent is known, so an empty session
  may be the better answer.
- Task notifications: `<status>` completed/failed/killed -> Ok/Error/Interrupted plus exit code from the summary;
  "(exit code N: ...)" soft failure stays Ok; output file -> `full_output_path`.
- Queue: a `remove` without content takes the queue front (measured; leftover queued input 545 -> 27, matches replay).
- Subagent iff the first conversation line has isSidechain; inline sidechains in old main files -> Origin::Agent.
- Result-line leftovers (permissionDecision, mcpMeta, ...) -> owning message `extra.results[call_id]`; line uuid ->
  `extra.uuids`. CONSISTENCY: Codex uses `extra.calls.<call_id>.{call,output}` for the same purpose.
- data drops: keys echoing the input, keys mapped to typed fields, Read file.content equal to the text, file.base64.
- Extras: null/empty values dropped down to 3 levels; envelope keys go to the session once, a line keeps one only
  when its value differs.
