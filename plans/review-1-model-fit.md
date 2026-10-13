# Review 1: model fit and correctness

Reviewer lens: does every recorded thing in the four research files have a correct home in plan sections 3 and 4.
Inputs: `agentparse.md` (plan v1), `research-claude.md`, `research-codex.md`, `research-opencode.md`, `research-gemini.md`.
Severity: blocker = silent data loss or a wrong rule at scale; major = wrong or missing mapping for a core concept;
minor = local fix or clarification.

## A. Model shape

1. **[blocker] Gemini reduced view silently drops whole conversation prefixes.**
   Plan: section 6 Gemini ("port the upstream reducer"), D12, section 3 `Rewind`.
   Evidence: research-gemini section 4 ("`$set.messages` REPLACES the whole array"; "anything removed by `/compress` or
   rollback disappears from the REDUCED state although the earlier bare lines are still in the file"), section 8.2
   ("Reduced state alone loses it... offering both views").
   The plan keeps only the reduced state and a `Rewind` with ids, so every compressed or rewound session loses the text of
   all dropped turns, against Summary line 5.
   Change: Gemini emits the full view. Every message id ever seen, at its last version, in first-seen order (`orderIds` only
   reorders surviving ids). At the record where `$set.messages`, `$patch.removeIds` or `$rewindTo` removes ids, emit
   `Rewind { dropped }` after the dropped messages, which stay as entries. Rule text: "the reduced (resume) view = entries
   minus every id listed in a `Rewind.dropped`". Update the fixture expectation for `gemini-new-session.jsonl` to include
   `u-2`, `g-4` and the `Rewind` entry.

2. **[blocker] OpenCode migrated sessions resumed in 2.x lose all new turns.**
   Plan: section 6 OpenCode ("v1 ... for migrated sessions present in both"), section 10 risk 6.
   Evidence: research-opencode section 0 item 1 and section 2 ("nothing wrote `session/message/part` after the upgrade",
   `max(session.time_updated) < min(session_v2.time_created where version='2.0.27')`), section 12 item 19 (message ids are
   identical across v1 and v2 for migrated rows). This answers risk 6: yes, new data goes only to v2.
   Change rule: "Session id in both tables: entries = v1 rows, then every `session_message` row whose id is not a v1 message
   id, in `seq` order (this includes the migration `system` notice and any turns added under 2.x). `updated_at` = max of
   both tables."

3. **[major] No home for record fields outside the mapped ones; D6 points at a field that does not exist.**
   Plan: section 3 (`Entry`, `Session`, `Notice`, `ErrorInfo`, `Turn`, `Settings`), D6, D9, section 4.1 last paragraph
   ("`tokens.tool` stays in the message's raw data only": `Message` has no raw data).
   Evidence: research-claude section 2.1 (`permissionMode`, `effort`, `requestId`, `origin`, `promptSource`, `quotaLimits`,
   `stop_details`, `thinkingDurationMs`), section 6 (`stop_hook_summary.hookInfos`, `api_error.retryInMs/maxRetries`,
   `local_command.commandRun`, `scheduled_task_fire.cron`), section 11 item 16 ("keep a `serde_json::Value` extras map");
   research-codex section 3.1 (`base_instructions`, `source`, `depth`, `history_mode`), 3.4 (`time_to_first_token_ms`,
   `model_context_window`, `turn_attribution`), 3.9 (`metadata`); research-opencode section 5.1 (`variant`, `path`,
   `structured`), section 6 (`snapshot`, `providerState`, `retry`), session row columns (`slug`, `share_url`, `summary_*`,
   `revert`, `permission`); research-gemini section 5.1 (`directories`, `memoryScratchpad`, `projectHash`, `kind`),
   section 5.2 (`displayContent`).
   Change: add `pub extra: Option<serde_json::Value>` to `Entry` ("keys of the source record(s) that no field of `kind`
   took, as recorded, including the product usage object when `Usage` did not take all of it; None when nothing is left")
   and `pub extra: Option<serde_json::Value>` to `Session` (Codex first `session_meta` payload rest, Gemini header and
   `$set` metadata, OpenCode session row columns, Claude subagent `.meta.json`). Skip keys that only repeat a mapped field
   (Claude `wireToolInputs`, `session_id`). Rewrite D6 to "extra usage numbers stay in `Entry.extra`".

4. **[major] `Part` has no `Other`, so unmodeled blocks inside a message are lost or split the message.**
   Plan: section 3 `Part`, D9, section 6 OpenCode ("step-start/step-finish/snapshot/patch parts become usage/`Other`").
   `Other` is an `EntryKind`; emitting it from inside a message breaks the message into several entries and loses part order.
   Evidence: research-claude section 3 (`fallback` 65, `document` 16+2; `redacted_thinking`, `server_tool_use`,
   `web_search_tool_result` known); research-opencode section 5.2 (`step-start`, `step-finish`, `snapshot`, `patch`,
   `agent`, `subtask`, `retry` parts); research-codex section 3.8 (`encrypted_content` content item of `agent_message`),
   3.6 (`input_audio`); research-gemini section 5.2 (`executableCode`, `codeExecutionResult`).
   Change: add `Part::Other { kind: String, data: serde_json::Value }`. Rule: "an unmodeled block or part stays in place as
   `Part::Other`; `document` (pdf) is an `Attachment`; OpenCode `step-finish` tokens are never added to usage (the message
   `tokens` already equal them, research-opencode section 4.2)".

5. **[major] `synthetic: bool` throws away the classification the parser has to compute anyway.**
   Plan: section 3 `Message.synthetic`, section 4.2.
   Evidence: research-claude section 3 string kinds (task-notification 8,588, teammate-message 2,732, system-reminder 1,776,
   command-name 426, local-command-stdout 277, bash-input 101, compact summary 351), section 2.1 `origin.kind`,
   `promptSource`; research-codex section 3.6 `content_item_kinds` table; research-opencode section 6 `synthetic` rows with
   `metadata.source`; research-gemini sections 3 and 5.2 (`<session_context>`, `<hook_context>`, interrupted placeholders).
   All three critiques ask for the kind (claude 14.3, codex 11.4, gemini 10).
   Change: replace `pub synthetic: bool` with `pub synthetic: Option<String>`. None = typed by a human. `Some(kind)` = the
   product's own label (Claude `origin.kind`, or the leading tag name, or `meta`, `compact_summary`; Codex
   `content_item_kinds[i]`; OpenCode `synthetic` or `metadata.source`; Gemini tag name). Add `Message::is_human()` = role
   User, synthetic None, author None. Add rule: "a source message whose parts carry different kinds (Codex user `message`
   with mixed `content_item_kinds`, OpenCode v1 user message mixing typed text and `synthetic:true` text parts) is split
   into consecutive Messages, one per kind run, ids suffixed `#1`, `#2`".

## B. Usage (section 4.1)

6. **[major] Gemini input formula is wrong; `tokens.tool` is not inside `input`.**
   Evidence: research-gemini section 5.4: `total = input + output + thoughts + tool`. If `tool` were inside `input`,
   `total` would count it twice.
   Change: Gemini input = `tokens.input - tokens.cached + tokens.tool` (tool-use prompt tokens are uncached prompt input).
   Delete the sentence "`tokens.tool` ... a prompt-side count already inside `input`".

7. **[major] Claude reasoning is recorded, the plan says "unknown (0)".**
   Evidence: research-claude section 4 usage shape `output_tokens_details:{thinking_tokens} | null` (object on 28%),
   section 11 item 3 ("`thinking_tokens` in usage is the only size signal"), section 14 item 5 (it is inside `output_tokens`).
   Change: Claude reasoning = `usage.output_tokens_details.thinking_tokens` when present, else 0.

8. **[minor] Codex `cache_write` may be double counted.**
   Evidence: research-codex section 3.5: `total_tokens == input_tokens + output_tokens`, `input_tokens` includes cached;
   `cache_write_input_tokens` is 0 in every local record, so whether it is also inside `input_tokens` is unverified.
   Change: if cache writes are part of the prompt total, input = `input_tokens - cached_input_tokens -
   cache_write_input_tokens` (saturating). Decide with the check in finding 9 on an upstream fixture, not by guess.

9. **[minor] `Usage::total()` is undefined and nothing checks the formulas.**
   Change: define `total() = input + output + cache_read + cache_write` (reasoning is inside output). P2 fixture tests
   assert `total()` equals the product's own total where one is recorded: Codex `total_tokens` (research-codex 3.5),
   OpenCode v1 `tokens.total` (research-opencode section 0 item 7), Gemini `tokens.total` (research-gemini 5.4).
   This check would have caught findings 6 and 8.

10. **[major] Compaction usage has no home; the plan's own oracle (section 8 item 4) will not match.**
    Evidence: research-opencode section 6 (`compaction` row has `cost?`, `tokens?` of the compaction call), section 5.1
    (v1 `summary:true` assistant carries tokens and cost and is folded into `Compaction` by the plan); research-codex
    section 7 (`compaction_response_id`, `latest_token_usage_record`).
    Change: add `pub usage: Option<Usage>` to `Compaction`. `Session.usage` = sum of Message usage plus Compaction usage.
    Codex: the `token_usage_record` whose `response_id == compacted.compaction_response_id` goes to the Compaction, not to
    the next assistant message.

11. **[minor] Claude does record a session cost.**
    Evidence: research-claude section 9 `cost-state` (1,616 lines, `totalCostUSD`, per model `costUSD`); section 4
    (1.4% of `message.id` appear in 2+ files because resume copies history).
    Change: Claude `Session.usage.cost_usd` = last `cost-state.totalCostUSD` in the file (message usage keeps `cost_usd`
    None); verify in P5 whether a resumed file's snapshot covers only its own process. Document that cross-session totals
    must dedupe Claude assistant entries by `Entry.id` (= `message.id`, finding 33).

## C. Status (D7, section 4.3)

12. **[major] D7 misfires on Claude soft failures and misses timeouts.**
    Plan: D7, section 4.3, section 4.5 exit code row (lists `returnCodeInterpretation` as an exit code source; Gemini
    "absent = 0 when success").
    Evidence: research-claude section 5.1 (soft failure: `is_error:false`, `returnCodeInterpretation` "No matches found"
    3,316 times, real exit code not stored); research-gemini sections 5.6 and 7 (inactivity timeout is `status:"success"`
    with no `Exit Code` line; background notice has no exit code).
    Change rule text: "`exit_code` is set only from a recorded or printed code. Claude with `returnCodeInterpretation`:
    `exit_code` None, status Ok. Gemini: `exit_code` 0 only when status is success AND the body has the modern `Output:`
    header AND it is not the timeout text AND not the background notice. D7: a shell call with non-zero `exit_code` OR
    `timed_out` is Error; `interrupted` is Interrupted." `ToolCall::failed()` then = status in {Error, Rejected,
    Interrupted}; drop its redundant "or non-zero exit".

13. **[major] The product's own status is overwritten by D7 and is not kept anywhere.**
    Evidence: research-claude sections 2.1 and 5.1 (`toolDenialKind`, 7 values, is a line key, not inside `toolUseResult`);
    research-gemini 5.6 (`status` and cancel reason are on `ToolCallRecord`, not in `resultDisplay`, which the plan uses as
    `data`); research-opencode 5.3 and 6 (`state.status`, `error.type` `tool.interrupted` / `tool.execution` sit outside
    `metadata`, which the plan uses as `data`).
    Change: add `pub status_detail: Option<String>` to `ToolCall` = the product status or denial label as recorded (Claude
    `toolDenialKind`, else `is_error`; Codex item `status`; OpenCode `state.status` plus `error.type`; Gemini `status` plus
    cancel reason). Define `ToolOutput.data` per product: Claude `toolUseResult`; Codex the merged event item(s); OpenCode
    the `state` object minus `input` and `output` (keeps `title`, `metadata`, `time`, `error`); Gemini the
    `ToolCallRecord` minus `args` and `result` (keeps `displayName`, `description`, `resultDisplay`).

14. **[minor] Status table incomplete for outcomes the research names.**
    Evidence: research-claude section 5.1 table; research-opencode section 11.2 (denial appears as error text, inferred);
    research-gemini 5.6.
    Change, add to 4.3: Claude `toolDenialKind` user-rejected, permission-rule, automode-* -> Rejected; interrupted, the
    session-ended text, cancelled (sibling errored) -> Interrupted; hook blocked -> Error. Claude user text lines
    `[Request interrupted by user]` and `[Request interrupted by user for tool use]` (with `interruptedMessageId`) ->
    `Turn { status: "interrupted" }`, not a user message. OpenCode error text starting `The user rejected permission` ->
    Rejected (verify on real data). Gemini cancelled -> Rejected when the reason names a denial, else Interrupted.
    Make "Pending becomes Interrupted" concrete: a call without a result is Interrupted when a later `Turn` entry or a
    later assistant Message exists in the parsed window, else Pending.

## D. Shell and terminal detail

15. **[major] Claude's current shell control tools map to `Other`.**
    Plan: section 4.4 Process and Shell rows.
    Evidence: research-claude section 10 (`TaskOutput` 104, `TaskStop` 302, `Monitor` 1,266; `BashOutput` 6, `KillShell`
    0) and section 5.1 background row.
    Change 4.4 Claude cells: Process = `TaskOutput` (read, id `task_id`), `TaskStop` (kill, id `task_id`), `BashOutput`
    (read, `bash_id`), `KillShell`/`KillBash` (kill). Shell adds `Monitor` (`command`, `timeout_ms`, background true).
    Task adds `SendMessage` (agent = `to`).

16. **[major] Background and async completions are not joined back to their call.**
    Plan: section 4.5 background row, section 6 Claude and OpenCode.
    Evidence: research-claude 5.1 background row (`<task-notification>` with `<tool-use-id>`, `<status>completed|failed|
    killed`, summary "exit code N", `<output-file>`), section 7 async `Agent` (`<task-id>` = agentId, `<result>`);
    research-opencode section 6 (`synthetic` rows with `metadata.source="shell"`, `shellID`, `state`, exit info; v2 shell
    `metadata.shellID`); research-codex section 6 late event (already handled by the plan).
    Change rule: "A completion notice naming an earlier call (Claude `<task-notification>` by `<tool-use-id>`; OpenCode v2
    `synthetic` by `metadata.shellID`/`jobID`; Codex late `CommandExecution` by `process_id`) updates that call:
    `output.shell.exit_code`, status (completed -> Ok, failed -> Error, killed -> Interrupted), `ended_at`,
    `full_output_path` = `<output-file>`; for an async `Agent` it sets `child_session` = `<task-id>`. The notice itself
    stays as a synthetic user Message because the model saw it."

17. **[major] Commands the human ran have no home.**
    Evidence: research-claude section 3 (`<bash-input>` 101 + `<bash-stdout>` 101); research-codex section 5 source row
    (`user_shell` = the `!cmd` escape); research-opencode section 5.4 last paragraph (v1 synthetic "The following tool was
    executed by the user" + zero-token assistant with one `bash` part), section 6 `shell` row (`shellID, command, status
    running|exited|timeout|killed, exit, output{output,cursor,size,truncated}`).
    Change rule: "A command the human ran becomes `Message { role: User, synthetic: None }` with one `Part::ToolCall`
    (`tool: Shell`, full `ShellResult`): Claude `<bash-input>` line plus the following `<bash-stdout>`/`<bash-stderr>`
    line; Codex `CommandExecution` with `source: user_shell`; OpenCode v1 synthetic user message plus its zero-token
    assistant `bash` message merged into one; OpenCode v2 `shell` row (status -> exit_code / timed_out / interrupted,
    `output.truncated`)."

18. **[major] Codex `duration_ms` source is the wrong clock.**
    Plan: section 4.5 duration row (`duration {secs,nanos}` first).
    Evidence: research-codex section 5 duration row: it times process execution only, values are microseconds to
    milliseconds, the model saw seconds, "Do not mix them".
    Change: `duration_ms` = wall clock of the call, sources in order: `item_completed` envelope `completed_at_ms -
    started_at_ms`; exec output JSON `wall_time_seconds`; `Wall time:` header (direct calls); legacy `duration_seconds`.
    `duration {secs,nanos}` stays in `data`. Children also take `started_at`/`ended_at` from the envelope. Verify in P2
    that the envelope times are not as small as `duration`; if they are, drop that source.

19. **[major] Section 4.5 interrupt row is wrong for Claude; timeout and signal rows are missing.**
    Evidence: research-claude 5.1 (`toolUseResult.interrupted` "never true in this corpus; do not use it as the interrupt
    signal"; timeout text `Command timed out after`, `timedOutAfterMs`), 11 item 8 (timestamps are write times, not
    monotonic in 10% of files); research-opencode 5.4 (v1 `<shell_metadata>` timeout/abort notice inside output), section 6
    (v2 `metadata.signal`, `metadata.timeout`); research-gemini section 7 (`Signal:` line, timeout text, `Command was
    cancelled by user`).
    Change rows: interrupted, Claude = `toolDenialKind == "interrupted"` or result text `[Request interrupted by user for
    tool use]` or the session-ended text (OR `interrupted:true`). New row timed_out: Claude `Command timed out after` or
    `timedOutAfterMs`; Codex none; OpenCode v1 `<shell_metadata>` timeout notice, v2 `metadata.timeout`; Gemini `Command was
    automatically cancelled because it exceeded the timeout`. New row signal: OpenCode v2 `metadata.signal`; Gemini
    `^Signal: (.+)$` ignoring `(none)`; Claude and Codex none. Claude duration: none (do not derive from write times);
    `started_at`/`ended_at` = call line and result line timestamps.

20. **[minor] Codex code mode details are underspecified.**
    Evidence: research-codex 4.2 (status line `Script completed|failed|running with cell ID|terminated`, `Script error:`
    item, `Wall time`), 3.10 (`notify` adds extra outputs, 1:N), 4.1 (`wait{cell_id, terminate}`), section 5 source row
    (`unified_exec_interaction` with `interaction_input`), 2.2/3.11 (McpToolCall, FileChange, DynamicToolCall, ImageView
    items), section 6 late event, section 8 (Extended-mode `exec_command_end`, `patch_apply_end`, `mcp_tool_call_end`).
    Change: (a) cell status from the first output line: completed -> Ok, failed or a `Script error:` item -> Error,
    terminated -> Interrupted, running -> Pending; fill `output.shell` with `duration_ms` = Wall time and `background_id` =
    cell id. (b) Extra `custom_tool_call_output` lines with the same `call_id` append to `output.text`. (c) `wait` ->
    `Process { id: cell_id, action: Read }` (Kill when `terminate`). (d) `CommandExecution` with source
    `unified_exec_interaction` -> `Process { id: process_id, action: Write, input: interaction_input }`, not Shell.
    (e) Children = every tool-like item (CommandExecution, Extension, McpToolCall, DynamicToolCall, FileChange, ImageView,
    ImageGeneration, WebSearch) between the cell call and its output; an item whose `id` equals a `call_id` enriches that
    call instead. (f) A late `CommandExecution` is appended as a child of the cell whose output JSON has
    `session_id == process_id` (also search later `write_stdin` outputs). (g) A child has `name` = item type, `input` =
    Null, `data` = the event. (h) Legacy `exec_command_end`, `patch_apply_end`, `mcp_tool_call_end` enrich the call with
    the same `call_id`.

21. **[minor] Claude `bashEditDiff` is a file change record the plan ignores.**
    Evidence: research-claude section 10 Bash row (`bashEditDiff{files[{filePath, hunks[], created?, deleted?}]}`,
    2.1.272+).
    Change: map each file to `FileChange { op: Add if created, Delete if deleted, else Update, diff from hunks }` in the
    Bash call's `output.files`.

22. **[minor] Gemini masked results lose output that is still in the file.**
    Evidence: research-gemini section 7 (masking via `$patch`/`$set`), 8.2, 11 item 11.
    Change: when the final result is a `<tool_output_masked>` stub and an earlier line of the file had an unmasked result
    for the same call id, use the earlier one; else keep the stub and set `truncated: true`. Fits finding 1.

## E. Edits and reads

23. **[minor] Per-product `FileChange` rules are missing.**
    Evidence: research-claude section 10 Edit/Write (`structuredPatch` hunks, `type: create|update`); research-gemini 5.7
    (`FileDiff.isNewFile`, `diffStat.model_*` and `user_*`); research-codex 3.11 and 4.5 (`add`/`delete` carry full
    `content`, `update` carries `unified_diff` and `move_path`).
    Change: Claude op Add when TUR `type == "create"`, else Update; `diff` = hunks rendered with `@@ -a,b +c,d @@` headers;
    additions/deletions counted from `+`/`-` lines. Gemini op Add when `isNewFile`, path = `filePath`, diff = `fileDiff`,
    additions = `model_added_lines + user_added_lines`, deletions likewise. Codex add/delete: `diff` = the content as all
    `+` or all `-` lines (do not store the body a second time). Map `NotebookEdit` to `Other` (no old/new strings; `Edit`
    would be misleading).

24. **[minor] `Read.offset` has different bases per product; `read_many_files` does not fit `Read`.**
    Evidence: research-gemini section 6 (`offset/limit` replaced by `start_line/end_line`, semantics differ), research-claude
    section 10 Read (`offset`, `limit`, `pages`), research-opencode 5.4 and 6.
    Change: define `Read.offset` = 1-based first line, `limit` = line count. Gemini `start_line/end_line` -> offset =
    `start_line`, limit = `end_line - start_line + 1`; old Gemini `offset` is 0-based, add 1; confirm the OpenCode base from
    source in P2. Map `read_many_files` (many globs in `include[]`) to `Other`.

## F. Mapping gaps per product

25. **[major] Codex message boundaries and usage attribution are undefined.**
    Plan: section 6 Codex, section 4.1 ("attached to the assistant message of that response").
    Evidence: research-codex section 6 last paragraph (order within one response: reasoning, AgentMessage,
    `custom_tool_call`, `token_usage_record`, nested events, output), 3.5 ("attribute a record to the assistant items since
    the previous record").
    Change rule: "One assistant Message per model response: the run of assistant-side response items (reasoning, assistant
    `message`, `function_call`, `custom_tool_call`, `local_shell_call`, `web_search_call`, `image_generation_call`,
    `tool_search_call`) up to the next `token_usage_record`, which gives `usage`. `Entry.id` = `response_id` (fallback: first
    item id). `phase` = the text item's phase; a response with two text items of different phases is split. `model` = the
    current `turn_context.model`."

26. **[major] Codex and Gemini compaction are not mapped; the summary carrier rule is ambiguous.**
    Evidence: research-codex section 7 (`compacted{message, replacement_history, window ids, compaction_response_id}`,
    `item_completed ContextCompaction`, legacy `event_msg context_compacted`, response items `compaction` and
    `context_compaction`); research-gemini 8.2 (`<state_snapshot>` user message plus "Got it" ack, info text
    `Chat history compressed from X to Y tokens.`).
    Change: Codex `compacted` -> `Compaction { summary: message }`; the other four records of the same compaction fold into
    it; `replacement_history` goes to `Entry.extra`. Gemini user message containing `<state_snapshot>` -> `Compaction
    { summary }`, the ack message gets `synthetic: Some("compaction_ack")`, tokens before/after from the info text when
    present. One rule for all four: the summary carrier (Claude `isCompactSummary` line, OpenCode v1 `summary:true`
    assistant, Gemini `<state_snapshot>` user) is absorbed into `Compaction` and not emitted again as a Message (section
    4.2 currently also lists the Claude summary line as a synthetic user message).

27. **[major] Claude `<synthetic>` API error messages would become assistant messages.**
    Evidence: research-claude section 4 rule 6 (1,496 messages, model `<synthetic>`, zero usage, "They are not model
    output"), section 6 synthetic API errors (`error` kind, `apiErrorStatus`).
    Change: an assistant line with `isApiErrorMessage: true` or model `<synthetic>` -> `Error { kind: <error>, status:
    apiErrorStatus, message: <text> }`; never added to `Session.models`.

28. **[major] OpenCode "split at the first `_`" turns plugin tools into fake MCP calls.**
    Plan: section 4.4 Mcp row, OpenCode cell.
    Evidence: research-opencode 4.3 (plugin tools `get_goal` 108, `update_goal`, `create_goal`, `set_goal`,
    `list_all_goals`), 5.4 MCP row ("ambiguous ... resolve from `opencode.jsonc` mcp keys").
    Change: an OpenCode name is `Mcp` only when the prefix before `_` equals a configured MCP server name (`mcp` keys of
    `opencode.json[c]`, read by `parse_db` when present) or it is `list_mcp_resources`/`read_mcp_resource`; otherwise
    `Other`.

29. **[minor] Claude mapping fixes.**
    Evidence: research-claude section 9 (title order; no `summary` line in the corpus; cwd changes in 6.6% of files;
    `relocated`, `permission-mode`), section 2 (`agent-setting`), section 8 (`queued_command`), section 1 (workflow files,
    `journal.jsonl`), section 2.1 (`promptId` only on user lines).
    Change: (a) title = latest `custom-title` > latest `agent-name` > latest `ai-title` > `summary` (old CLIs) > first human
    prompt; child title = meta `description`. (b) Emit `Settings` when line `cwd`, `permissionMode` (or a `permission-mode`
    line), `effort` or `agent-setting` changes, like Codex `turn_context`; `Shell.cwd` = `cwd` of the `tool_use` line
    (section 4.5 says "session/entry cwd" but `Entry` has no cwd). (c) `attachment/queued_command` with `commandMode:
    "prompt"` and text not starting with `<` -> user Message (typed input folded into a running turn); other attachments ->
    `Other { kind: "attachment.<subtype>" }`. (d) Discovery: `subagents/workflows/<wf>/agent-*.jsonl` are child sessions;
    `journal.jsonl` is not a session. (e) `turn_id`: carry the last user `promptId` forward to following entries.

30. **[minor] Codex mapping fixes.**
    Evidence: research-codex 3.8, 3.12, section 9 (legacy), 3.11 (`Plan` item), 4.4.
    Change: (a) `agent_message`: role Assistant when `author` is this session's agent path, else User with `author` set.
    (b) Section 4.2 calls the inherited child prefix synthetic, section 6 drops it: keep the drop and store
    `subagent_history_start_ordinal` in `Session.extra` so it is visible. (c) Legacy mode has no ids on
    `event_msg user_message/agent_message`: take assistant text and reasoning from response items only, user text from the
    `user_message` event, and skip the response item copy matched by text. (d) `item_completed Plan{text}` is model text:
    assistant Text part, not `Tool::Plan` (section 4.4 lists it as a tool). (e) `SubAgentActivity completed` (no call id)
    -> `Other`.

31. **[minor] OpenCode mapping fixes.**
    Evidence: research-opencode section 6 (`system`, `skill` rows), 5.1 errors, 5.2 text `metadata.openai.phase`.
    Change: (a) section 3 says OpenCode `system` -> `Notice`, section 4.2 says `Role::System` Message. Pick
    `Message { role: System, synthetic: Some("system") }` and add the rule: Role::System = harness text the model saw,
    `Notice` = a record the model did not see. (b) v2 `skill` row -> user Message, `synthetic: Some("skill")`.
    (c) Assistant `error` -> an `Error` entry after the (possibly empty) message; `MessageAbortedError` and
    `idle.outcome = interrupted` -> `Turn { status: "interrupted" }`. (d) Text part `metadata.openai.phase` -> `Message.phase`.

32. **[minor] Gemini mapping fixes.**
    Evidence: research-gemini section 2 (`.json` and migrated `.jsonl` share a sessionId; cwd chain), 5.6 (`timestamp` is
    completion time).
    Change: (a) discovery dedupes by sessionId, preferring `.jsonl`; (b) `ended_at` = `ToolCallRecord.timestamp`,
    `started_at` None; (c) `parse_file` reads `../.project_root` for cwd; `parse_str` leaves cwd None unless
    `<session_context>` names a workspace directory.

## G. Ids, ordering, subagents

33. **[major] Claude `parent_id` dangles after folding and merging.**
    Plan: section 3 `Entry.id` ("Claude uuid"), `Entry.parent_id`, D3.
    Evidence: research-claude section 5 (the result line `parentUuid` = the assistant LINE uuid holding the `tool_use`),
    section 4 (one message spans several lines; 49,423 groups not chained), section 9 graph facts. After results are folded
    and chunks merged, most `parentUuid` values name a line that is no Entry.
    Change: `Entry.id` of a merged assistant message = `message.id` (research-claude 14.1; also the cross-file usage dedupe
    key, section 4 rule 3); user, system and attachment lines keep `uuid`. Keep a line-uuid to entry-id map while parsing;
    `parent_id` = the entry that absorbed the line named by `parentUuid` (a tool_result line maps to the Message holding its
    call, a chunk line to its merged Message); `compact_boundary` uses `logicalParentUuid`; None when outside the file.

34. **[major] Claude child session identity is wrong as written.**
    Plan: section 3 `Session.id` ("Claude file stem"), `ParentLink`, D11.
    Evidence: research-claude section 1 (subagent line `sessionId` = parent session, `agentId` = file stem after `agent-`;
    meta `toolUseId`, `parentAgentId`, `spawnDepth`, `agentType`), section 11 item 10, section 7 (async spawns link only
    through the notification).
    Change: Claude child `Session.id` = `agentId`. `ParentLink.session_id` = meta `parentAgentId` when set (nested), else
    line `sessionId`. `ParentLink.call_id` = meta `toolUseId`. `ParentLink.agent` = meta `agentType`. The parent's `Agent`
    call `child_session` = `toolUseResult.agentId` (or `agent_id`, or `<task-id>` of the async notification, finding 16).

35. **[minor] Session ids are not unique per agent.**
    Evidence: research-codex section 1 (a reverted thread writes a second rollout `_<rollout_id>` with the same thread id,
    `history_base`), research-gemini section 2 (`.json` and `.jsonl` with one sessionId).
    Change: document in section 5 that `(agent, source)` identifies a `SessionRef`, not `id`; discovery dedupes only the
    Gemini pair.

36. **[minor] D10 contradicts what `data` will hold.**
    Evidence: research-claude section 10 Read image TUR `file.base64` (5,705), 11 item 12; research-codex 3.11
    ImageGeneration `result (base64)`; research-gemini 5.2 `inlineData` in result parts.
    Change D10: in `ToolOutput.data` and `Entry.extra` replace these with `{"bytes": <decoded length>}`: Claude
    `toolUseResult.file.base64`, Codex `result` of ImageGeneration and `image_generation_call`, any `inlineData.data`, any
    `data:` URL string. Text file bodies stay as recorded.

## H. Opposite pressure: fields to cut or narrow

37. **[minor] Remove or narrow these.**
    - `Message.sidechain`: research-claude section 1, every line of a subagent file has `isSidechain:true` and `agentId`
      is present exactly when it is true, so it equals `Session.parent.is_some()`; no other product has it. Delete;
      re-add only if P2 finds `isSidechain:true` lines inside a main file from older CLIs.
    - `ShellResult.merged`: equals `stderr.is_none()` once the rule "a product that records one stream sets `stderr`
      None" holds. Delete.
    - `Tool::Code.language`: only Codex, always JavaScript (research-codex 4.2). Make it `Code { source: String }`.
    - `Rewind.to`: for `$rewindTo` it is `dropped[0]`; Codex `thread_rolled_back` has no id. Keep `dropped` and `turns`.
    - `Message.agent` vs `ParentLink.agent`: for Claude and Codex children the value repeats on every message. Define
      `Message.agent` as the per-message agent/mode only where the product records it per message (OpenCode); do not copy
      `ParentLink.agent` into it.
    - `ToolCall::failed()` "or non-zero exit": redundant after finding 12.

VERDICT: changes required
