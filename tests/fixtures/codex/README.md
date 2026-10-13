# Codex CLI fixtures

Small synthetic rollout files for the Codex parser. Every value is fake: prompts like "list the files",
paths under `/work/demo`, git remote `https://example.com/demo.git`, uuid-shaped ids, and the encrypted
blob `gAAAAAB0ZmFrZQ==`. Shapes (key names, key order, nesting, value types, `file://` cwd, `{secs,nanos}`
durations, PascalCase item tags) follow measured real rollouts and the upstream source at `rust-v0.162.1`
(older shapes from the 0.6, 0.24 and 0.114 source copies). Every line parses with `jq` except the two
deliberately broken lines in `corrupt-tail.jsonl`.

"Records" below means JSONL lines. "Ordinal" means the `ordinal` field (paginated files only), which equals
the 0 based line index. "Dedupe" means the logical item after merging the event and the response item that
describe the same thing.

## Overview

| File | Era imitated | Lines | Main thing it exercises |
|---|---|---:|---|
| `paginated-code-mode.jsonl` | 0.162.1, `history_mode: paginated`, code mode `exec` | 31 | the full paginated turn: context messages, dedupe, exec cells with children, truncation, usage |
| `paginated-late-exec.jsonl` | 0.162.1 paginated | 24 | a command that outlives its script, `write_stdin`, a late `CommandExecution` with no open call |
| `paginated-subagent-parent.jsonl` | 0.162.1 paginated, multi agent v2 | 39 | `spawn_agent`, `send_message`, `wait_agent`, `SubAgentActivity`, `CollabAgentToolCall`, inter agent messages |
| `paginated-subagent-child.jsonl` | 0.162.1 paginated, child thread | 32 | two `session_meta` lines, inherited prefix, `thread_settings_applied`, FINAL_ANSWER |
| `paginated-interrupt.jsonl` | 0.162.1 paginated | 29 | `<turn_aborted>` marker plus `turn_aborted` event, then a second turn |
| `legacy-direct-tools.jsonl` | envelope format, no `history_mode`, no `ordinal`, `cli_version` 0.120.0, Extended event persistence | 46 | every direct tool shape, duplicated events, compaction, abort |
| `flat-era.jsonl` | pre envelope, Rust CLI up to about 0.32 (no `cli_version` is recorded) | 12 | flat first line, bare response items, `record_type: state` lines |
| `corrupt-tail.jsonl` | 0.162.1 paginated | 12 | a non JSON line in the middle and a truncated last line |
| `exit3.jsonl` | 0.162.1 paginated, recorded by `codex exec` (see `VERIFICATION.md`) | 29 | a real rollout, sanitized: one cell runs `ls -la`, `sh test.sh` (exit 3, stdout and stderr interleaved) and `cat a.txt`, a second cell runs `apply_patch` (a `FileChange` item) |
| `paginated-code-mode.jsonl.zst` | same as `paginated-code-mode.jsonl` | | that file compressed with `zstd -3`, as Codex stores rollouts older than 7 days |

Notes on faithfulness: `legacy-direct-tools.jsonl` is a composite. No real version writes `shell`,
`exec_command`, `local_shell_call` and `apply_patch` together, they are combined so one file covers every
direct shape. Its `exec_command_end` has `stdout`, `stderr` and `formatted_output` filled in; real 0.114 and
later builds blank those three before persisting, so a parser must accept both. Cwd values in the legacy file
are plain paths (`PathUri` `file://` is only used by the 0.162 events).

## Facts a parser test should assert

### `paginated-code-mode.jsonl`

| Fact | Expected |
|---|---|
| Identity | one `session_meta` at ordinal 0, `id == session_id == 019f1a10-0001-7000-8000-000000000001`, `cli_version` 0.162.1, `history_mode` paginated, git branch `main`, repo `https://example.com/demo.git`, cwd `/work/demo` |
| Ordinals | 0 to 30, contiguous, 31 records; timestamps non decreasing |
| User messages | 1 real: "list the files" (the `UserMessage` event at ordinal 7 and the `user.text` response item at ordinal 6, event id does not equal the item id) |
| Harness context messages (not user text) | 2: the developer message (ordinal 2, kinds `host_skills`, `permissions`, `collaboration_mode`) and the user message with kinds `agents_md.instructions` + `environments.environment_context` (ordinal 3) |
| Assistant messages after dedupe | 2: one `commentary` ("I will read the README and run the checks.") and one `final_answer` ("The demo directory has README.md and src/. One check failed."). Raw records: 2 `AgentMessage` events + 2 assistant `message` items, same ids pairwise |
| Reasoning after dedupe | 2, each with `summary: []`, no readable text, `encrypted_content` `gAAAAAB0ZmFrZQ==`. Raw records: 2 `Reasoning` events + 2 `reasoning` items, same ids pairwise |
| Top level tool calls | 2, both `custom_tool_call` named `exec` (ordinals 12 and 20), each with one `custom_tool_call_output` (ordinals 16 and 23); every output `call_id` matches |
| Children of exec cell 1 | 2 `CommandExecution` events (ordinals 14, 15), in order: (a) `cat README.md; rg TODO src`, process_id "40101", exit 0, status `completed`, duration `{0, 142000}`, `parsed_cmd` = [read name README.md, search query TODO path src]; (b) `./run-checks.sh`, process_id "40102", exit 1, status `failed`, duration `{0, 2318000}`, `parsed_cmd` = [unknown]. Both `source` `unified_exec_startup`, cwd `file:///work/demo` (decodes to `/work/demo`), `command[0..2]` = `/bin/zsh`, `-lc`, the command |
| Children of exec cell 2 | 1 `Extension` kind `web.search` (ordinal 22), query "example query", action type `search`, 1 `text_result` (example.com) |
| Model visible output of cell 1 | item 0 header `Script completed`, wall time 1.2 seconds; item 1 JSON with `exit_code` 0 and `original_token_count` 12; item 2 starts with `Warning: truncated output (original token count: 6120)` then `Total output lines: 2`, its JSON has `exit_code` 1 and `original_token_count` 5871 and an `output` cut mid word. So 3 items, status Script completed |
| Model visible output of cell 2 | 2 items, header wall time 2.3 seconds, plain text second item (not JSON) |
| Usage | 3 `token_usage_record` lines (ordinals 13, 21, 27), per record input/cached/output/reasoning: 12000/4000/300/100, 13000/12000/200/50, 14000/13000/80/0. Sum: input 39000, cached 29000, output 580, reasoning 150, total_tokens 39580 (total = input + output). Uncached input 10000. Last `thread_token_usage` equals the sum. `cache_write_input_tokens` 0 everywhere |
| `token_count` | 4 events (ordinals 17, 24, 28, 29); the last two have identical totals (re-emission). Do not sum them. Last `total_token_usage` equals the record sum |
| Usage attribution | record 1 owns reasoning + commentary + exec call 1; record 2 owns reasoning + exec call 2; record 3 owns the final answer |
| Turns | 1 `task_started`, 1 `task_complete` (`last_agent_message` equals the final answer), 1 `turn_context` (model `gpt-6.1-sol`, sandbox `read-only`, approval `never`), 0 interrupts |

### `paginated-late-exec.jsonl`

| Fact | Expected |
|---|---|
| Ordinals | 0 to 23, 24 records |
| User messages | 1 ("start the dev server and check it"); 1 harness context message (agents_md + environment_context) |
| Assistant messages | 1 (`final_answer`, "The dev server started and answered GET / with 200."); reasoning 2 (encrypted only) |
| Top level tool calls | 2 `exec` cells: cell A (ordinal 8, `tools.exec_command` of `bun run dev` with `yield_time_ms:1000`) and cell B (ordinal 14, `tools.write_stdin` with `session_id:64391`, `chars:""`) |
| Cell A output | header `Script completed`, wall time 1.0 seconds; JSON has `session_id` 64391 and NO `exit_code` |
| Cell B output | JSON again has `session_id` 64391, no `exit_code`; cell B has no `CommandExecution` event (an empty `write_stdin` writes none) |
| Late event | the only `CommandExecution` is at ordinal 18, after cell B's output (ordinal 16) and with no open `exec` call: process_id "64391", command `bun run dev`, exit 0, status `completed`, duration `{12, 500000000}`. Join by `process_id == session_id` to cell A's `exec_command` child, set that child to exit 0 completed |
| Exit codes known | exactly 1 (the late event, 0). Cell A and cell B children are "running" until the late event lands |
| Usage | 3 records: 9000/3000/120/40, 9400/9000/90/20, 9700/9400/40/0. Sum input 28100, cached 21400, output 250, reasoning 60, total 28350 |
| Turns | 1 start, 1 complete, 0 interrupts |

### `paginated-subagent-parent.jsonl`

| Fact | Expected |
|---|---|
| Identity | `id == session_id == 019f1a10-0003-7000-8000-000000000003`, no parent. Ordinals 0 to 38 |
| User messages | 1 ("review the demo project with a helper"); 1 harness context message plus the developer message |
| Assistant messages | 2 (`commentary` "I will ask a helper to review it.", `final_answer` "The helper approved the demo project."); reasoning 2 |
| Tool calls | 3 `function_call`, all `namespace` `collaboration`: `spawn_agent` (ordinal 12, arguments `{"task_name":"review","message":"gAAAAAB0ZmFrZQ=="}`), `send_message` (19), `wait_agent` (24, `{"timeout_ms":10000}`). 0 `exec` cells, 0 `CommandExecution` |
| Call outputs | `spawn_agent` -> `{"task_name":"/root/review"}`; `send_message` -> empty string `""`; `wait_agent` -> `{"message":"Wait completed.","timed_out":false}` |
| `SubAgentActivity` | 3: `started` (ordinal 14, `id` equals the spawn call_id `call_0b18866d5a04e6c01aa41099b7cb8415`, model `gpt-6.1-sol`, effort `low`), `interacted` (21, id equals the send_message call_id), `completed` (33, id `subagent-completed-019f1a10-0004-7000-8000-000000000004`). All have `agent_thread_id` 019f1a10-0004-7000-8000-000000000004 and `agent_path` `/root/review` |
| `CollabAgentToolCall` | 1 (ordinal 26), `tool` `wait`, status `completed`, id equals the wait_agent call_id |
| Inter agent messages | 2 `agent_message` items (ordinals 30 and 32), each preceded by an `inter_agent_communication_metadata` line (ordinals 29, 31) with `trigger_turn` false. Author `/root/review`, recipient `/root`. First: `Message Type: MESSAGE`, 2 content parts (`input_text` + `encrypted_content`). Second: `Message Type: FINAL_ANSWER`, plaintext only, payload `VERDICT: APPROVE` |
| Subagent link | the child to open is the file whose `session_meta.id` equals 019f1a10-0004-7000-8000-000000000004 (`paginated-subagent-child.jsonl`) |
| Usage | 4 records: 8000/2000/150/30, 8300/8000/60/10, 8500/8300/30/0, 9000/8500/20/0. Sum input 33800, cached 26800, output 260, reasoning 40, total 34060. Child usage is NOT included |
| Turns | 1 start, 1 complete, 0 interrupts |

### `paginated-subagent-child.jsonl`

| Fact | Expected |
|---|---|
| Records | 32, ordinals 0 to 31 |
| `session_meta` | 2. Ordinal 0 is the child: `id` 019f1a10-0004-7000-8000-000000000004, `session_id` 019f1a10-0003-7000-8000-000000000003 (the ROOT, differs from `id`), `forked_from_id` and `parent_thread_id` equal the parent id, `source.subagent.thread_spawn` with `depth` 1, `agent_path` `/root/review`, `agent_nickname` Banach, `agent_role` null, `thread_source` `subagent`, `subagent_history_start_ordinal` 9, `multi_agent_version` v2. Ordinal 1 is a copy of the root meta (`id == session_id`), not an identity |
| Inherited prefix | ordinals 2 to 8: a parent `task_started` (turn 019f1a11-0003-...), developer message, context user message (`inherited_user_message` true), `world_state`, parent `turn_context`, the parent's user prompt (kind `user.text`, `inherited_user_message` true, ordinal 7), a developer `multi_agent.role_instructions` message (ordinal 8, no `turn_id`) |
| Own history | starts at ordinal 9 `thread_settings_applied` (model `gpt-6.1-sol`, effort `low`, cwd `/work/demo`) |
| User messages of the child | 0 after dropping the inherited prefix (1 raw `user.text` line, but it is inherited). No `UserMessage` event exists |
| Own turn | `task_started` at ordinal 10, turn 019f1a11-0004-..., `parent_turn_id` = the parent's turn, `initiating_agent_path` `/root`, `root_turn_id` = the parent's turn; `turn_context` at 13 with `effort` `low`; 1 `task_complete` at 31 (2 `task_started` lines in the file, only 1 is the child's) |
| Inter agent messages | `NEW_TASK` (ordinal 15, author `/root`, recipient `/root/review`, encrypted second part, metadata `trigger_turn` true at ordinal 14) and `FINAL_ANSWER` (ordinal 26, plaintext `VERDICT: APPROVE`, metadata at 25 with `trigger_turn` false) |
| Assistant messages | 2: `commentary` "Reviewing the demo files." and `final_answer` "VERDICT: APPROVE" (same text as the FINAL_ANSWER payload and the parent's `last_agent_message` of the child) |
| Tool calls | 1 `exec` cell (ordinal 20) with 1 child: `cat README.md`, process_id "50201", exit 0, `completed`, parsed_cmd read README.md |
| Usage | 2 records on the child's own `thread_id`: 7000/1000/60/10 and 7300/7000/10/0. Sum input 14300, cached 8000, output 70, reasoning 10, total 14370 (starts from zero, independent of the parent) |
| Parent link | `payload.parent_thread_id` = `payload.source.subagent.thread_spawn.parent_thread_id` = 019f1a10-0003-7000-8000-000000000003, equals the id in `paginated-subagent-parent.jsonl` |
| Interrupts | 0 |

### `paginated-interrupt.jsonl`

| Fact | Expected |
|---|---|
| Ordinals | 0 to 28, 29 records |
| User messages | 2: "run the long build" (turn 1), "never mind, just list the files" (turn 2) |
| Assistant messages | 2: `commentary` "Starting the build." and `final_answer` "The directory has README.md and src." |
| Interrupts | 1: developer message at ordinal 16 with text starting `<turn_aborted>` and kind `generic.turn_aborted`, then the `turn_aborted` event at ordinal 17 (`reason` `interrupted`, `turn_id` = `root_turn_id` = turn 1, `started_at` < `completed_at`). The marker is NOT a user message |
| Turns | 2 `task_started` (ordinals 1 and 18), 1 `task_complete` (turn 2). Turn 1 ends by abort, status interrupted; turn 2 completed |
| Tool calls | 1 `exec` cell (ordinal 12, `bun run build`), output at 14 has `session_id` 70001 and no `exit_code`; it has no `CommandExecution` child, so it is unfinished when the turn is aborted |
| Usage | 2 records: 6000/1000/80/20 and 6200/6000/30/0. Sum input 12200, cached 7000, output 110, reasoning 20, total 12310 |

### `legacy-direct-tools.jsonl`

| Fact | Expected |
|---|---|
| Format | envelope lines `{"timestamp","type","payload"}` with NO `ordinal`; `session_meta` has no `history_mode` (default legacy), `cli_version` 0.120.0, `source` `cli`, `id` only (no `session_id`), git present. 46 lines: 1 `session_meta`, 1 `turn_context`, 28 `response_item`, 15 `event_msg`, 1 `compacted` |
| User messages | 2 real: "list the files" and "run the slow tests". Each appears as a `response_item` message and an `event_msg` `user_message` with no shared id (match by text and order) |
| Harness context messages | 3 `user` role response items with no `content_item_kinds`: `<user_instructions>` block, `<environment_context>` block (tag sniffing), and the `<turn_aborted>` marker (see interrupts) |
| Assistant messages | 2: `commentary` "I will list the files and edit the code." and `final_answer` "Done: the files are listed and the code is edited." Each is an `event_msg` `agent_message` followed by its `response_item` (the order Codex writes), no shared id |
| Reasoning | 1 after dedupe: summary `[{"type":"summary_text","text":"Plan the file listing."}]`, content `[{"type":"reasoning_text","text":"Raw thought about listing files."}]`, `encrypted_content` the fake blob. Events `agent_reasoning` (1) and `agent_reasoning_raw_content` (1) carry the same two texts |
| Tool calls with a call_id | 10, in order: `shell` (argv `["bash","-lc","ls"]`), `shell_command` (string `rg missing src`), `exec_command` (`sleep 1; echo ok`), `write_stdin` (session_id 64391), `local_shell_call` (action exec, argv `git status --short`), `apply_patch` (`custom_tool_call`), `update_plan`, `view_image`, `mcp__demo__search`, and a final `shell` (`sleep 600`) in turn 2. Plus 1 `web_search_call` (no call_id, status `completed`, action `search`, query "example query") |
| Outputs | 9 paired outputs (8 `function_call_output`, 1 `custom_tool_call_output`); the final `shell` (turn 2) has none, it is the interrupted call |
| Exit codes | `shell` 0 (`metadata.exit_code`, `duration_seconds` 0.1); `shell_command` 1 (`Exit code: 1`, wall time 0.2); `exec_command` running with session ID 64391 (no exit code in its output), later 0 from `exec_command_end` (matched by `call_id`); `write_stdin` 0 (`Process exited with code 0`, original token count 1); `local_shell_call` 0 |
| `exec_command_end` | 1 event: `call_id` equals the `exec_command` call_id, `process_id` "64391", `exit_code` 0, `status` `completed`, `duration` `{1, 105000000}`, `aggregated_output` "ok\n", `stdout` "ok\n", `stderr` "", `formatted_output` "ok\n", plain path cwd `/work/demo` |
| `apply_patch` | input has 3 file sections: Add `docs/new.txt` (2 `+` lines), Update `src/lib.rs` with `*** Move to: src/core.rs` and 2 `@@` hunks, Delete `old.txt`. Output text `Success. Updated the following files:\nA docs/new.txt\nM src/core.rs\nD old.txt\n`. `patch_apply_end` (same `call_id`): `success` true, `status` `completed`, `changes` map with 3 absolute paths: `add` (content), `update` (`unified_diff`, `move_path` `/work/demo/src/core.rs`), `delete` (content) |
| `update_plan` | args: explanation "Two steps.", plan of 2 steps (`completed`, `in_progress`); output `Plan updated` |
| `view_image` | args `{"path":"/work/demo/shot.png"}`; output is an array with 1 `input_image` (data URL), not a string |
| MCP | name `mcp__demo__search` (server `demo`, tool `search`), args `{"query":"demo"}`, output `Wall time: 0.0100 seconds\nOutput:\n3 results for demo` |
| Usage | 2 `token_count` events: first has `info` null (rate limits only); second has total input 11600, cached 6200, output 260, reasoning 60, total_tokens 11860, last 6400/5200/120/20 (total 6520), `model_context_window` 272000. No `cache_write_input_tokens` key and no `token_usage_record` lines |
| Compaction | 1 `compacted` line (message "Synthetic summary: listed files, edited code, searched the web.", `replacement_history` of 2 messages) plus 1 `context_compacted` event |
| Interrupts | 1: user role `<turn_aborted>` marker message, then `turn_aborted` event (`reason` `interrupted`, `turn_id` = turn 2) |
| Turns | 2 `task_started`, 1 `task_complete` (turn 1, `last_agent_message` equals the final answer); turn 2 is aborted |

### `flat-era.jsonl`

| Fact | Expected |
|---|---|
| Format | 12 lines, no envelope. Line 1 has no `type`: `{"id","timestamp","instructions","git"}` (instructions "Synthetic user instructions.", git branch `main`). Later lines are bare response items (`type` is `message`, `reasoning`, `function_call`, `function_call_output`) |
| State lines | 3 lines `{"record_type":"state"}` (lines 2, 8, 12), carry no data, must be skipped without error |
| Messages | 1 user ("list the files"), 2 assistant (final text "The directory has README.md and src. missing.txt does not exist.") |
| Reasoning | 1, summary text "Plan the listing.", no `content` key |
| Tool calls | 2 `function_call` named `shell` with argv commands; outputs are JSON strings: exit 0 (`duration_seconds` 0.1) and exit 1 (`cat: missing.txt: No such file or directory`) |
| Usage, models, turns | none exist in this era (0 usage, no model, no turn events, no timestamps except line 1) |

### `corrupt-tail.jsonl`

| Fact | Expected |
|---|---|
| Lines | 12 physical lines (11 end with a newline, the last has none) |
| Valid JSON lines | 10, ordinals 0 to 9 contiguous |
| Bad lines | exactly 2: line 6 is plain text `this line is not json at all`, line 12 is a `task_started` event cut off mid object (no closing braces). A tolerant parser skips both and reports 2 skipped lines |
| Content | 1 user message ("say hello"), 1 assistant message (`final_answer` "Hello."), 1 usage record 3000/1000/5/0 (total 3005), 1 `task_complete`. The truncated `task_started` for a second turn must not produce a turn |

### `exit3.jsonl`

Recorded with `codex exec` 0.162.1 in a scratch repo, then sanitized: paths became `/work/demo`, the prompt,
every context block and every assistant text became placeholders, `ls -la` output became `total 0\n`, the
system prompt, `world_state` and account ids became fakes. Ids, ordinals, timestamps, tool records and the
`sh test.sh` and `cat a.txt` outputs are as recorded.

| Fact | Expected |
|---|---|
| Cell 1 | 3 `CommandExecution` children: `ls -la` (exit 0, `parsed_cmd` list_files), `sh test.sh` (exit 3, status `failed`, `aggregated_output` `err-line\nout-line\n`, one stream, stderr first), `cat a.txt` (exit 0, output `one\ntwo\nthree\n`). Script header `Script completed`, wall time 0.8 seconds |
| Cell 2 | 1 `FileChange` child: `/work/demo/a.txt` update, diff `-two` / `+two edited`, `move_path` null, status `completed`. Cell output item 1 is `{}` |
| Usage | 3 records; last `thread_token_usage` input 69103, cached 38656, output 215, reasoning 0 |
| Turns | 1 `task_started`, 1 `task_complete` |
