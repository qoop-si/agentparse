# Research: OpenAI Codex CLI session format (for `agentparse`)

Status: research only. Written 2026-10-13. Plain facts, measured where marked `M`, from upstream source where marked `U`.

## 0. Method, corpus, and the one caveat that matters

| Item | Value |
|---|---|
| Local corpus | `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<thread uuid>.jsonl`, 126 files (123 when I started, it is live), 54.7 MB, 8,371 lines, 0 unparsable lines, dates 2026-10-11 to 2026-10-13 |
| Producer | one `cli_version` only: **0.162.1** (matches the upstream release tag `rust-v0.162.1`, published 2026-10-09). `originator` is always `codex_exec`, `source` is `exec` (94 root threads) or `subagent` (32 children) |
| Model | one model: `gpt-6.1-sol`. Its catalog entry (`~/.codex/models_cache.json`) says `tool_mode: code_mode_only`, `shell_type: unified_exec`, `apply_patch_tool_type: freeform` |
| Upstream | `openai/codex` at tag `rust-v0.162.1` (main was pushed 2026-10-12). Source copies are in `<scratchpad>/codex-research/up/` |
| Scripts | `scan.py` (type/key census), `shape.py` (path/type union per record kind), `pair.py`, `pair2.py`, `late.py`, `tok.py`, `tok2.py`, `dedupe.py`, `execout.py`, `sizes.py` in `.../scratchpad/codex-research/`. Run with `uv run --no-project python <script>` |

**The caveat.** Every local file is the newest format (`history_mode: paginated`) written by a code-mode-only model. So the local corpus has NO `function_call` named `shell`/`shell_command`/`exec_command`, NO `apply_patch`, NO `update_plan`, NO `view_image`, NO MCP call, NO `compacted`, NO `local_shell_call`, NO `exec_command_end`/`patch_apply_end` events, no legacy-mode files, no `.zst`, no archived sessions. Those shapes below are marked `U` (upstream source only, never seen in a real file here). A parser must support them anyway because users have years of older rollouts. `agentparse` needs test fixtures for every `U` row, built from the upstream types, not from this machine.

## 1. What each store on disk is

| Path | What it is | Use for agentparse |
|---|---|---|
| `~/.codex/sessions/YYYY/MM/DD/rollout-<local ts>-<thread uuid>[_<rollout uuid>].jsonl` | The canonical transcript, one JSON object per line. `U`: rollouts older than 7 days may be rewritten to `*.jsonl.zst` by a background worker (zstd level 3); readers must accept both | Primary source |
| `~/.codex/archived_sessions/` | `U`: same file layout, moved here when a thread is archived (`threads.archived=1`). **Does not exist here** (0 archived threads) | Scan it too if present |
| `~/.codex/history.jsonl` | `U`: user prompt history for the TUI up-arrow, lines `{session_id, ts, text}`. **Does not exist here** (exec-only usage) | Ignore (user text only, no assistant) |
| `~/.codex/session_index.jsonl` | `U`: append-only thread renames `{id, thread_name, updated_at}`, newest wins. Does not exist here | Optional: gives a user-set title |
| `~/.codex/state_5.sqlite` | Thread index. Table `threads` (126 rows, matches files 1:1): `id, rollout_path, created_at(_ms), updated_at(_ms), source, model_provider, cwd, title, sandbox_policy, approval_mode, tokens_used, has_user_event, archived, git_sha, git_branch, git_origin_url, cli_version, first_user_message, agent_nickname, agent_role, agent_path, model, reasoning_effort, thread_source, preview, history_mode, name, originator, ...`. `title == first_user_message == preview` for all 126 rows, `name` is NULL (no renames). Also `thread_spawn_edges(parent_thread_id, child_thread_id, status)` (32 rows, all `open`), `thread_dynamic_tools`, `thread_attachments`, `projects`. Opens with WAL files next to it | Fast listing, title, parent links. Not authoritative, rebuildable from rollouts |
| `~/.codex/thread_history_1.sqlite` | Derived projection of the rollouts for the app UI. `thread_items(thread_id, turn_id, item_id, rollout_ordinal, created_at_ms, item_json, item_type, started_at_ms, completed_at_ms)` (2,024 rows), `thread_turns(thread_id, turn_id, rollout_ordinal, status, error_json, started_at, completed_at, duration_ms, first_user_item_id, final_agent_item_id, rollout_byte_offset, rollout_end_ordinal, rollout_end_byte_offset, root_turn_id)`. `item_json` is the **camelCase app-server `ThreadItem`** (`commandExecution`, `commandActions`, `aggregatedOutput`, `agentThreadId`...), a different spelling from the snake_case `item_completed` payload in the rollout | Do not parse. Useful only as an oracle to cross check our parser (it holds turn status `completed`/`interrupted` and byte offsets) |
| `~/.codex/logs_2.sqlite` | tracing logs (`logs` table, 2,074 rows: config loader, HTTP client, `session_loop{thread_id=...}` submissions). Not a conversation | Ignore |
| `~/.codex/goals_1.sqlite`, `memories_1.sqlite`, `queue_1.sqlite` | thread goals, memory pipeline outputs (`stage1_outputs`), queued follow-up inputs. All empty here | Ignore for now |
| `~/.codex/sqlite/codex-dev.db`, `.codex-global-state.json`, `config.toml`, `hooks.json`, `auth.json` | desktop app state, config, hooks (here: qoop hook script), credentials. `auth.json` is a secret, never read | Ignore |
| `~/.codex/models_cache.json` | model catalog: `slug, context_window, max_context_window, supported_reasoning_levels, tool_mode, shell_type, truncation_policy, ...`. **No prices** | Context window lookup. Cost must come from our own price table |

Rollout file name: `rollout-YYYY-MM-DDThh-mm-ss-<thread_id>.jsonl` (local time of creation). A reverted thread adds `_<rollout_id>` after the thread id `U` (`rollout_file_name.rs`). Time inside lines is UTC (`2026-10-11T14:50:06.579Z`), so the file name hour and the line hour differ by the TZ offset.

## 2. The line envelope and top-level types

Every line: `{"timestamp": "<RFC3339 ms UTC>", "ordinal": <u64>, "type": "<kind>", "payload": {...}}` and, only on some `response_item` lines, a sibling `"metadata": {...}` (see 3.9). `U`: `RolloutLine { timestamp, ordinal: Option<u64>, #[flatten] RolloutItem }` and `RolloutItem` is `#[serde(tag="type", content="payload")]` snake_case (`codex-rs/history/src/rollout_payload.rs`).

`ordinal` is dense and 0 based in a file: 126 of 126 files have contiguous ordinals and monotonic timestamps `M`. Use `ordinal` as the entry order and as the stable id for incremental parsing. In legacy files there is no `ordinal`.

### 2.1 Measured census (126 files, 8,371 lines)

| top `type` | lines | `U` RolloutItem variant | persisted in paginated mode |
|---|---:|---|---|
| `event_msg` | 3,393 | `EventMsg(EventMsg)` | only a whitelist, see 8 |
| `response_item` | 3,344 | `ResponseItem` (+ optional `metadata`) | yes (list in 8) |
| `token_usage_record` | 985 | `TokenUsageRecord` | yes |
| `world_state` | 201 | `WorldState` | yes |
| `turn_context` | 160 | `TurnContext` | yes |
| `session_meta` | 158 | `SessionMeta` | yes |
| `inter_agent_communication_metadata` | 130 | `InterAgentCommunicationMetadata{trigger_turn}` | yes |
| not seen: `compacted`, `inter_agent_communication`, `retained_context`, `security_risk_score`, `realtime_item` | 0 | `Compacted`, `InterAgentCommunication`, `RetainedContext`, `SecurityRiskScore`, `RealtimeItem` | yes |

### 2.2 `event_msg` payload types seen (3,393)

| `payload.type` | n | keys |
|---|---:|---|
| `item_completed` | 2,046 | `type, thread_id, turn_id, item, started_at_ms, completed_at_ms` |
| `token_count` | 1,028 | `type, info, rate_limits` |
| `task_started` | 160 | `type, turn_attribution, turn_id, root_turn_id, started_at, model_context_window, collaboration_mode_kind` |
| `task_complete` | 124 | `type, root_turn_id, turn_id, last_agent_message, started_at, completed_at, duration_ms, time_to_first_token_ms` |
| `thread_settings_applied` | 32 | `type, thread_id, thread_settings` |
| `turn_aborted` | 3 | `type, root_turn_id, turn_id, reason, started_at, completed_at, duration_ms` |

`item_completed.item.type` values seen: `CommandExecution` 917, `Reasoning` 525, `AgentMessage` 335, `SubAgentActivity` 149, `UserMessage` 94, `Extension` 19 (all `kind: "web.search"`), `CollabAgentToolCall` 8. The item type tags are **PascalCase** (`#[serde(tag="type")]` on `TurnItem` without rename). `U` full set: `UserMessage, FunctionCallOutput, HookPrompt, AgentMessage, Plan, Reasoning, CommandExecution, DynamicToolCall, CollabAgentToolCall, SubAgentActivity, WebSearch, ImageView, Extension, ImageGeneration, EnteredReviewMode, ExitedReviewMode, FileChange, McpToolCall, ContextCompaction`.

### 2.3 `response_item` payload types seen (3,344)

| `payload.type` | n | keys |
|---|---:|---|
| `message` | 967 | `type, id, role, content, internal_chat_message_metadata_passthrough, phase?` |
| `custom_tool_call` | 735 | `type, id, status, call_id, name, input, internal_chat_message_metadata_passthrough` |
| `custom_tool_call_output` | 735 | `type, id, call_id, output, internal_chat_message_metadata_passthrough` |
| `reasoning` | 525 | `type, id, summary, encrypted_content, internal_chat_message_metadata_passthrough` |
| `agent_message` | 130 | `type, id, author, recipient, content, internal_chat_message_metadata_passthrough` |
| `function_call` | 126 | `type, id, name, namespace, arguments, call_id, internal_chat_message_metadata_passthrough` |
| `function_call_output` | 126 | `type, id, call_id, output, internal_chat_message_metadata_passthrough` |

`U` full `ResponseItem` set (tag `type`, snake_case): `message, agent_message, reasoning, local_shell_call, function_call, tool_search_call, function_call_output, tool_search_output, custom_tool_call, custom_tool_call_output, web_search_call, image_generation_call, compaction (alias compaction_summary), context_compaction, configuration_update, additional_tools`, plus `compaction_trigger` and unknown (`serde(other)` = `Other`) which are never persisted. An unknown `type` must not fail a parse.

## 3. Exact field shapes (structure only, content replaced)

### 3.1 `session_meta` (158 lines; 126 files; 32 child files have two)

```json
{"timestamp":"<rfc3339>","ordinal":0,"type":"session_meta","payload":{
 "creator_user_id":"<id>","creator_account_id":"<id>",
 "session_id":"<root thread uuid>","id":"<this thread uuid>",
 "timestamp":"<rfc3339 of creation>","cwd":"/path/to/project",
 "runtime_workspace_roots":["/path/to/project"],
 "originator":"codex_exec","cli_version":"0.162.1",
 "source":"exec" | {"subagent":{"thread_spawn":{"parent_thread_id":"<uuid>","depth":1,"agent_path":"/root/<task>","agent_nickname":"<name>","agent_role":null}}},
 "thread_source":"user" | "subagent",
 "model_provider":"openai",
 "base_instructions":{"text":"<system prompt, ~20 KB>","provenance":{"type":"<kind>","model":"<slug>"}},
 "history_mode":"paginated",
 "context_window":{"window_id":"<uuid>"},
 "git":{"commit_hash":"<sha>","branch":"<name>","repository_url":"<sanitized url>"},
 // children only:
 "forked_from_id":"<parent uuid>","parent_thread_id":"<parent uuid>","agent_nickname":"<name>","agent_path":"/root/<task>",
 "subagent_history_start_ordinal":9,"multi_agent_version":"v2"}}
```

| Fact `M`/`U` | Detail |
|---|---|
| `id` vs `session_id` | `id` = this thread. `session_id` = ROOT thread id. For a root thread they are equal; **for a child they differ** (all 32 children). `U`: `SessionMeta.session_id` doc "equal to the root thread's ID". So `session_id` is NOT a unique file id. Use `id` |
| two metas in a child file | ordinal 0 = child's own meta; ordinal 1 = a copy of the ROOT thread's meta (`id == session_id`). Skip ordinal 1 as identity |
| `git` | present in 129 of 158 lines (the non-git cwd has none). `repository_url` is missing in 6 of those. It is a sibling of the flattened `SessionMeta` fields (`SessionMetaLine { #[flatten] meta, git }`) |
| `source` | string `exec` here. `U` `SessionSource`: `cli, vscode (default), exec, mcp, custom(<s>), internal(...), subagent(...), unknown`. Subagent object also may be `review`, `compact`, `memory_consolidation`, `other` |
| `thread_source` | `user`, `subagent`; `U`: also `guardian_review`, `feature:<x>`, `memory_consolidation` |
| `history_mode` | `legacy` or `paginated` (default `legacy` when absent). Decides which events exist (see 8) |
| `base_instructions.text` | the full system prompt, ~20 KB per line; session_meta lines are 6.6% of all bytes. Do not copy into every model object; keep behind a lazy field |
| `dynamic_tools`, `selected_capability_roots`, `memory_mode`, `history_base`, `forked_from_ordinal_exclusive` | `U` optional, not seen. `history_base` means the file's prefix lives in another rollout (HistoryPosition{thread_id, end_ordinal_exclusive, end_byte_offset}) |
| `instructions` | `U` older field, replaced: user instructions moved to turn context / world_state; absent here. Old flat format had `instructions` (see 9) |

### 3.2 `turn_context` (160): per turn model, cwd, permissions

```json
{"type":"turn_context","payload":{"turn_id":"<uuid>","root_turn_id":"<uuid>","disabled_plugin_ids":[],
 "cwd":"/path/to/project","approval_policy":"never","approvals_reviewer":"user",
 "sandbox_policy":{"type":"read-only"},
 "permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"},
 "model":"gpt-6.1-sol","comp_hash":"<hash>",
 "collaboration_mode":{"mode":"default","settings":{"model":"<slug>","reasoning_effort":null|"low","developer_instructions":null}},
 "multi_agent_version":"v2","realtime_active":false,"summary":"none","effort":"low"}}
```

`effort` only present on the 34 child-agent turns (value `low`); root turns have none and `collaboration_mode.settings.reasoning_effort: null`. `summary` is the placeholder `"none"` (all 160). `U` also: `active_permission_profile`, `file_system_sandbox_policy`, `cyber_access_program`. `approval_policy` `U` values (kebab-case): `untrusted`, `on-request` (default, alias `on-failure`), `granular{...}`, `never`. `sandbox_policy.type` seen: `read-only`; `U` others: `workspace-write`, `danger-full-access`, `external-sandbox`. `network`: `restricted` (seen).

### 3.3 `world_state` (201): model-visible context baseline (diffed)

`{"full": true|false, "state": {...}}`. `full:true` = baseline snapshot (126, one per file), `full:false` = patch (75). Keys seen in `state`: `agents_md{directory,text}`, `apps_instructions`, `collaboration_mode{mode,model,instructions}`, `context_window_guidance`, `environments{environments.local{cwd,status,shell}, current_date, timezone, filesystem, subagents}`, `environments_instructions`, `git_attribution`, `host_skills{body,includeInstructions}`, `managed_developer_instructions`, `model`, `model_catalog`, `multi_agent_mode{mode,usage_hint_hash}`, `multi_agent_usage_hint`, `permissions{instructions,approved_command_prefixes}`, `persistent_mode`, `plugins_instructions`, `realtime{active}`, `skills{includeInstructions}`. Free metadata worth keeping: `environments.local.shell` (`zsh`), `timezone`, `current_date`. 8.3% of bytes; skip by default.

### 3.4 Turn lifecycle events

```json
{"type":"event_msg","payload":{"type":"task_started","turn_id":"<uuid>","root_turn_id":"<uuid>","started_at":1791730206,
 "model_context_window":475000,"collaboration_mode_kind":"default",
 "turn_attribution":{"turn_id":"<uuid>","turn_trigger":"exec","parent_turn_id":null|"<uuid>","initiating_agent_path":null|"/root","root_turn_id":"<uuid>"}}}
{"type":"event_msg","payload":{"type":"task_complete","turn_id":"<uuid>","root_turn_id":"<uuid>","last_agent_message":"<final text>","started_at":1,"completed_at":2,"duration_ms":1,"time_to_first_token_ms":1}}
{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"<uuid>","root_turn_id":"<uuid>","reason":"interrupted","started_at":1,"completed_at":2,"duration_ms":1}}
{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"<uuid>","thread_settings":{"model":"<slug>","model_provider_id":"openai","approval_policy":"never","approvals_reviewer":"user","permission_profile":{...},"cwd":"/path","runtime_workspace_roots":["/path"],"reasoning_effort":"low","reasoning_summary":"none","collaboration_mode":{...},"disabled_plugin_ids":[]}}}
```

| Fact | Detail |
|---|---|
| names | wire names are `task_started`/`task_complete` (v1); `U` also accepts aliases `turn_started`/`turn_complete`. Parser should accept both |
| times | `started_at`/`completed_at` are Unix **seconds**; `duration_ms`, `time_to_first_token_ms` ms. item_completed uses `started_at_ms`/`completed_at_ms` epoch ms |
| `model_context_window` | 258,400 (104 turns) or 475,000 (56 turns) here; the catalog says 500,000 for `gpt-6.1-sol` (effective window is a percentage). Use the per-turn value |
| `turn_trigger` | `exec` for all 157 here. `parent_turn_id` and `initiating_agent_path` (`/root`) set only on the 34 child turns that were started by a parent message |
| `root_turn_id` | groups a whole multi agent run: child turns carry the parent's root turn id |
| `task_complete` count 124 vs `task_started` 160 | 32 of the starts are inherited copies in child prefixes (3.12), 3 turns were aborted, 1 file was still running |
| `turn_aborted.reason` | seen only `interrupted`. `U` `TurnAbortReason`: `interrupted, replaced, review_ended, budget_limited`. `U` also optional `error: ErrorEvent` on `turn_aborted` and `task_complete` (terminal error). Not seen locally |
| abort marker | right before each `turn_aborted` Codex writes a `developer` `message` containing `<turn_aborted>\nThe previous turn was interrupted on purpose. Any running unified exec processes may still be running ...\n</turn_aborted>` (content kind `generic.turn_aborted`). No dangling tool call in the 3 local aborts (the interrupt landed between model calls) |
| `thread_settings_applied` | 32, one per child file, at the child's own start. This is where the child's model/effort/cwd is first stated |

### 3.5 Token usage: two parallel records

```json
{"type":"token_usage_record","payload":{"thread_id":"<uuid>","turn_id":"<uuid>","session_id":"<root uuid>","root_turn_id":"<uuid>","response_id":"resp_<id>",
 "usage":{"input_tokens":32344,"cached_input_tokens":20608,"cache_write_input_tokens":0,"output_tokens":92,"reasoning_output_tokens":0,"total_tokens":32436},
 "turn_token_usage":{ same 6 keys, cumulative within the turn },
 "thread_token_usage":{ same 6 keys, cumulative within the thread }}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{6 keys},"last_token_usage":{6 keys},"model_context_window":475000},
 "rate_limits":{"limit_id":"<id>","limit_name":null,"primary":{"used_percent":<f>,"window_minutes":300,"resets_at":1791740000},"secondary":{...same, longer window},"credits":{"has_credits":false,"unlimited":false,"balance":"<s>"},"individual_limit":null,"spend_control_reached":null,"plan_type":"<plan>","rate_limit_reached_type":null}}}
```

Measured relations (all 126 files):
- `usage.total_tokens == input_tokens + output_tokens` for every record. `cached_input_tokens <= input_tokens` always. So **`input_tokens` already INCLUDES cached tokens**, and `reasoning_output_tokens` is a SUBSET of `output_tokens` (476 records have non-zero reasoning). This is the OpenAI convention, opposite of Anthropic where cache_read is added on top. Uncached input = `input - cached`.
- `cache_write_input_tokens` is new (all 0 here); `#[serde(default)]` upstream.
- Sum of `usage` over a file's records equals the last `thread_token_usage` in 126 of 126 files. Child records use the child's own `thread_id` and start from zero, so child usage is NOT included in the parent's totals (no double count when summing across files).
- `token_usage_record` = one per model response (`response_id`), authoritative, 985. `token_count` = 1,028; every `last_token_usage` equals some record's `usage` and the last `total_token_usage` equals the last `thread_token_usage` (126 of 126). `token_count` is re-emitted with identical totals (43 extra, e.g. after a reasoning or message item), so **never sum `token_count`**. Dedupe by cumulative `total_tokens` or just use the records.
- Ordering: the record is written right after the response's output items (message, reasoning, calls) and before the tool output/exec events of those calls. Attribute a record to the assistant items since the previous record, not to the following tool outputs.
- `rate_limits`: account level snapshot (`primary.window_minutes` 300 = 5 hour window, `secondary.window_minutes` 10080 = weekly, `used_percent`, `resets_at` epoch seconds, `plan_type`, credits). Not session cost. Keep as a session event stream, not per message.
- No cost or price anywhere. `models_cache.json` has no prices.
- `U` older files (before 0.153.0, 2026-09-03) have only `token_count` (cumulative), no record; `info` can be `null`. `cache_write_input_tokens` absent before ~0.145.

### 3.6 `response_item` / `message`

```json
{"type":"response_item","payload":{"type":"message","id":"msg_<id>","role":"developer|user|assistant",
 "content":[{"type":"input_text","text":"<text>"} | {"type":"output_text","text":"<text>"}],
 "phase":"commentary|final_answer",          // assistant only, 334 of 334 assistant messages here
 "internal_chat_message_metadata_passthrough":{"turn_id":"<uuid>","create_time":1791730210.5459678,"content_item_kinds":["user.text"]}},
 "metadata":{"client_authored":false,"user_input_order":0,"retained_source":{...},"inherited_user_message":true,"mcp_attribution":{"status":"none"}}}
```

Roles measured: developer 381, user 252, assistant 334 (commentary 210 + final_answer 125; `partial_answer` is the third `U` value). `content[].type`: `input_text` (developer, user), `output_text` (assistant); `U` also `input_image{image_url|file_id, detail}`, `input_audio`.

`content_item_kinds` is index aligned with `content[]` and classifies each part. Values measured:

| role | kind | meaning | keep as |
|---|---|---|---|
| user | `user.text` | what the human typed (123 incl. 32 inherited copies) | user message |
| user | `environments.environment_context` | `<environment_context><cwd>..<shell>..<current_date>..<timezone>..<filesystem>..</environment_context>` XML-ish | system notice / drop |
| user | `agents_md.instructions` | `# AGENTS.md instructions for <dir>\n...` | system notice / drop |
| developer | `host_skills.instructions`, `permissions.instructions`, `collaboration_mode.instructions`, `plugins.recommendations`, `multi_agent.role_instructions`, `multi_agent.mode_instructions` | harness prompt pieces | drop (or `developer` role, hidden) |
| developer | `generic.turn_aborted` | the interrupt marker above | Interrupt event |
| assistant | `unknown` | model text | assistant message |

So the **user/developer message split is not enough**: real user text and injected context both have role `user`. Use `content_item_kinds` (U: `ContentItemKind`) to classify. When `content_item_kinds` is missing (older files) fall back to tag sniffing: leading `<environment_context>`, `# AGENTS.md instructions for`, `<recommended_plugins>`, `<skills_instructions>`, `<user_instructions>`, `<turn_aborted>`, `<hook_prompt ...>`.

### 3.7 `response_item` / `reasoning`

`{"type":"reasoning","id":"rs_<id>","summary":[],"encrypted_content":"<base64-ish blob 1.6 KB to 8.9 KB, median 1.9 KB>"}`.
All 525: `summary` is `[]`, there is no `content` key, only `encrypted_content`. The paired `item_completed` `Reasoning` item is `{"type":"Reasoning","id":"rs_<id>","summary_text":[],"raw_content":[]}`: both arrays empty in 525 of 525 (turn_context.summary is `none`, so no summaries are requested). **There is no readable reasoning text in this corpus.** `U`: `summary: [{"type":"summary_text","text":..}]`, `content: [{"type":"reasoning_text"|"text","text":..}]` appear only when the model returned summaries/raw content (older models, `reasoning_summary: auto|concise|detailed`). Model them as `Reasoning { text: Option<String> (joined summary or content), encrypted: Option<String>, id }`. Count `encrypted` length for "reasoning present" stats.

### 3.8 `response_item` / `agent_message` and `inter_agent_communication_metadata` (multi agent v2)

```json
{"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}
{"type":"response_item","payload":{"type":"agent_message","id":"amsg_<uuid>","author":"/root/<task>","recipient":"/root",
 "content":[{"type":"input_text","text":"Message Type: MESSAGE|NEW_TASK|FINAL_ANSWER\nTask name: /root\nSender: /root/<task>\nPayload:\n<maybe plaintext>"},{"type":"encrypted_content","encrypted_content":"<blob>"}],
 "internal_chat_message_metadata_passthrough":{"turn_id":"<uuid>","create_time":1.0}},"metadata":{"client_authored":false,"user_input_order":0}}
```

130 lines (53 `/root -> /root/<name>`, 77 `/root/<name> -> /root`). 108 have `[input_text, encrypted_content]`, 22 are plaintext only. `Message Type` values seen: `MESSAGE`, `NEW_TASK`, `FINAL_ANSWER`. For NEW_TASK and MESSAGE the **payload text is the encrypted blob** (cannot be read), only the header is visible. FINAL_ANSWER payloads are plaintext (and the child's last `AgentMessage` item carries the same text). The matching `function_call` args are also encrypted: `spawn_agent` arguments `{"task_name":"<name>","message":"gAAAAA..."}`. `inter_agent_communication_metadata` precedes each `agent_message` line (`trigger_turn` says whether it starts a new turn in the recipient). `U`: a plaintext `inter_agent_communication` top-level type `{id, author, recipient, other_recipients, content, encrypted_content?, trigger_turn}` exists for unencrypted setups. Not seen.

### 3.9 The sibling `metadata` object (`CodexHarnessMetadata`, 1,768 lines carry it)

| key | seen | meaning |
|---|---|---|
| `client_authored` | all, always `false` | developer message supplied by an app-server client |
| `fallback_token_limit_override` | 861 (every `*_output`), value 12000 | the history truncation token budget used for that tool output |
| `user_input_order` | 717, on user.text, assistant, agent_message, function_call | acceptance/delivery order, 0..13; a monotone counter shared by user input and assistant deliveries |
| `retained_source{id{message_id,turn_id,role},revision,complete}` | 461 (every assistant message and every `user.text` message, not the injected context messages) | pointer to the retained context record for this message; `id.message_id` equals the response item `id` |
| `inherited_user_message` | 64 | the copy of the parent's user/developer messages inside a child file. **Copied parent context, not child authored** |
| `mcp_attribution{status}` | 284 | `{"status":"none"}` only; `U` has `attribution_error`, `sources` |

### 3.10 `response_item` call and output shapes

```json
// code mode exec (the only direct tool for this model; 735 calls)
{"type":"custom_tool_call","id":"ctc_<id>","status":"completed","call_id":"call_<hex>","name":"exec","input":"<javascript source>"}
{"type":"custom_tool_call_output","id":"ctco_<uuid>","call_id":"call_<hex>","output":[{"type":"input_text","text":"Script completed\nWall time 4.5 seconds\nOutput:\n"},{"type":"input_text","text":"<json string or plain text from text(...)>"}, ...]}
// namespaced function tools (collaboration; 126 calls)
{"type":"function_call","id":"fc_<id>","name":"spawn_agent","namespace":"collaboration","arguments":"{\"task_name\":\"<name>\",\"message\":\"<enc>\"}","call_id":"call_<hex>"}
{"type":"function_call_output","id":"fco_<id>","call_id":"call_<hex>","output":"{\"task_name\":\"/root/<name>\"}"}
```

- `custom_tool_call.status` is `completed` in 735 of 735 (it is the model response status, not the tool result status).
- `output` of `custom_tool_call_output` is ALWAYS an array of `input_text` here (622 with 2 items, 100 with 3, 12 with 4, 1 with 7). `function_call_output.output` is a plain string here. `U`: `FunctionCallOutputBody` is untagged: a string OR an array of content items `input_text | input_image{image_url|file_id,detail} | input_audio | encrypted_content`. The internal `success` flag is NOT serialized. So a failed tool is only knowable from the text (or from the matching event). Support both string and array everywhere.
- Every `call_id` has exactly one output (861 outputs, 0 unpaired calls, 0 unpaired outputs, 0 call ids with 2 outputs). `U`: code mode `notify(...)` injects EXTRA `custom_tool_call_output` lines for the same `call_id` and a yielded script is continued by the `wait` tool, so allow 1:N.
- `function_call.encrypted_function_args` `U` optional.

### 3.11 `event_msg` / `item_completed` item shapes (paginated mode)

```json
// UserMessage (94). content[].type is lowercase "text"; text_elements always [] here
{"type":"UserMessage","id":"<uuid7>","content":[{"type":"text","text":"<text>","text_elements":[]}]}
// AgentMessage (335). content[].type is capitalized "Text". phase present
{"type":"AgentMessage","id":"msg_<id>","content":[{"type":"Text","text":"<text>"}],"phase":"commentary|final_answer"}
// Reasoning (525)
{"type":"Reasoning","id":"rs_<id>","summary_text":[],"raw_content":[]}
// CommandExecution (917), the shell record, details in 5
{"type":"CommandExecution","id":"exec-<uuid>","process_id":"45132","command":["/bin/zsh","-lc","<cmd>"],"cwd":"file:///path/to/dir",
 "parsed_cmd":[{"type":"read","cmd":"cat x","name":"x","path":"x"} | {"type":"list_files","cmd":"ls","path":null} | {"type":"search","cmd":"rg a b","query":"a","path":"b"} | {"type":"unknown","cmd":"<cmd>"}],
 "source":"unified_exec_startup","status":"completed|failed","aggregated_output":"<combined stdout+stderr>","exit_code":0,"duration":{"secs":0,"nanos":140333}}
// Extension web search (19): standalone web.run
{"type":"Extension","kind":"web.search","id":"exec-<uuid>","query":"<q or url or 'pattern'>","action":{"type":"search","query":null,"queries":["<q>"]} | {"type":"openPage","url":"<url>"} | {"type":"findInPage","url":null,"pattern":"<p>"} | {"type":"other"},
 "results":[{"type":"text_result","domain":"<d>","ref_id":"turn0search0","snippet":"<s>","title":"<t>","url":"<u>","thumbnail_url":"<u>"}]}
// SubAgentActivity (149)
{"type":"SubAgentActivity","model":"gpt-6.1-sol"|null,"reasoning_effort":"low"|null,"id":"call_<hex>|subagent-completed-<child uuid>","kind":"started|interacted|completed","agent_thread_id":"<child uuid>","agent_path":"/root/<task>"}
// CollabAgentToolCall (8): the wait tool
{"type":"CollabAgentToolCall","id":"call_<hex>","tool":"wait","status":"completed","sender_thread_id":"<uuid>","receiver_thread_ids":[],"receiver_agents":[],"agents_states":{}}
```

Notes: `UserMessage` item and `AgentMessage` item content type spellings differ (`text` vs `Text`); matching must be case insensitive or just read `.text`. `Extension.action` here uses camelCase types (`openPage`, `findInPage`) but the snake_case `WebSearchAction` in `response_item/web_search_call` uses `open_page`, `find_in_page`; accept both. `Extension` has no `status` key. `SubAgentActivity` `interrupted` `U` kind also exists. `U` `Interrupted/Failed` states for collab: `CollabAgentToolCallStatus in_progress|completed|failed|interrupted`.

`U` item shapes not seen locally:

| item | fields (snake_case unless noted) |
|---|---|
| `FileChange` | `id, changes: {<path>: {"type":"add","content"} | {"type":"delete","content"} | {"type":"update","unified_diff","move_path"}}, status: completed|failed|declined, auto_approved, stdout, stderr` (apply_patch result, includes full diffs) |
| `McpToolCall` | **camelCase**: `id, server, tool, arguments, connectorId, mcpAppResourceUri, mcpAppUi, linkId, appName, actionName, pluginId, readOnlyHint, status (inProgress|completed|failed), result: CallToolResult{content[],structured_content,is_error,_meta}, error{message}, duration` |
| `DynamicToolCall` | `id, namespace, tool, arguments, status, content_items, success, error, duration` |
| `ImageView` | `id, path` (view_image tool) |
| `ImageGeneration` / `Extension kind:"image_gen.generation"` | `id, status, revised_prompt, result (base64), saved_path` |
| `Extension kind:"clock.sleep"` | camelCase `id, durationMs` |
| `Plan` | `id, text` (plan mode text) |
| `FunctionCallOutput` | `id, name, namespace, output` |
| `HookPrompt` | `id, fragments:[{text, hookRunId}]` |
| `EnteredReviewMode` / `ExitedReviewMode` | `id, target, user_facing_hint` / `id, review_output` |
| `ContextCompaction` | `id` only (the summary lives in the `compacted` line) |
| `CollabAgentToolCall` full | `tool: spawn_agent|send_input|resume_agent|wait|close_agent|send_message|followup_task|interrupt_agent|list_agents`, `prompt, model, reasoning_effort, agents_states: {thread_id: AgentStatus}`; `AgentStatus`: `pending_init|running|interrupted|{completed: <msg>}|{errored: <msg>}|shutdown|not_found` |

### 3.12 Subagent (child thread) file layout `M`

32 child files, all `depth: 1`, 23 distinct parent threads, `agent_nickname` is a scientist name (Banach, Fermat, Dirac...), `agent_role` null, `agent_path` `/root/<task_name>`.

```
ordinal 0   session_meta  (child: id=<child>, session_id=<root>, parent_thread_id, forked_from_id, subagent_history_start_ordinal=N)
ordinal 1   session_meta  (copy of the ROOT meta: id==session_id)
2 .. N-1    INHERITED prefix: task_started(parent's turn), developer msgs, user msgs (metadata.inherited_user_message=true), world_state, turn_context(parent's turn), the original user prompt, a trailing developer msg
N           thread_settings_applied   <-- the child's own history starts here (N = 9, 10 or 11)
N+1 ...     task_started(child turn, turn_attribution.parent_turn_id/initiating_agent_path="/root", trigger "exec"), turn_context, inter_agent_communication_metadata, agent_message(NEW_TASK, encrypted), ...tool calls..., agent_message(FINAL_ANSWER)
```

Rules: records with `ordinal < subagent_history_start_ordinal` are inherited model context, not child activity (`U` doc: "stay out of child turn/item projection"). The child's real task prompt is encrypted, so a child has NO readable user message of its own. Its first readable text is its own commentary. The `inherited_user_message` copies must not be shown as a user message of the child (wend does show them, see 10). Parent link: `payload.parent_thread_id` and `payload.source.subagent.thread_spawn.parent_thread_id` and `state_5.thread_spawn_edges`. Reverse link: parent `SubAgentActivity{kind:"started"}.agent_thread_id` and `.id == function_call.call_id` of the `spawn_agent`. Child file name carries the child thread id, so the join key is `agent_thread_id == session_meta.id`.

## 4. Tools

### 4.1 Tool names seen (`M`) and where they can appear

| Record | name | n | arguments / input |
|---|---|---:|---|
| `custom_tool_call` | `exec` | 735 | `input` is JavaScript source (code mode, `U` `PUBLIC_TOOL_NAME="exec"`), optional first line `// @exec: {"yield_time_ms":..,"max_output_tokens":..}` |
| `function_call` ns `collaboration` | `spawn_agent` | 32 | `{"task_name":"<s>","message":"<enc>"}`; `U` also `agent_type, model, reasoning_effort, fork_turns` |
| | `send_message` | 84 | `{"target":"<task name>","message":"<enc>"}` |
| | `followup_task` | 2 | `{"target":"<task name>","message":"<enc>"}` |
| | `wait_agent` | 8 | `{"timeout_ms":10000}` |
| nested inside `exec` source (not separate records) | `tools.exec_command` 904 refs, `tools.write_stdin` 16, `tools.web__run` 19 | | see 4.3 |

`U` tools that exist upstream but never appeared (needed for older/other-model files):

| name | record | args |
|---|---|---|
| `exec_command` (direct, non code mode) | `function_call` | `{cmd, workdir?, shell?, login?, tty?, yield_time_ms?, timeout_ms?, max_output_tokens?, sandbox_permissions?, additional_permissions?, justification?, prefix_rule?:[..], environment_id?}` |
| `write_stdin` | `function_call` | `{session_id:int, chars:"", yield_time_ms?, max_output_tokens?}` |
| `shell` (0.3x to ~0.14x) | `function_call` | `{command:[argv..], workdir?, timeout_ms?, sandbox_permissions?, justification?, prefix_rule?}` |
| `shell_command` | `function_call` | `{command:"<string>", workdir?, login?, timeout_ms?, sandbox_permissions?, justification?, prefix_rule?}` |
| `local_shell_call` | `response_item` own type | `{call_id?, status: completed|in_progress|incomplete, action:{type:"exec",command:[..],timeout_ms,working_directory,env,user}}` |
| `apply_patch` | `custom_tool_call` (freeform) or old `function_call` with `{input}` | `input` = patch text, grammar in 4.5 |
| `update_plan` | `function_call` | `{explanation?, plan:[{step:"<s>", status: pending|in_progress|completed}]}` |
| `view_image` | `function_call` | `{path, detail?, environment_id?}` |
| `request_user_input`, `request_permissions` | `function_call` | question lists / permission profile |
| `wait` (code mode continuation) | `function_call` | `{cell_id, yield_time_ms?, max_tokens?, terminate?}` |
| `list_agents`, `interrupt_agent`, `close_agent`, `resume_agent`, `send_input` | `function_call` ns `collaboration` | `{path_prefix?}` etc. |
| MCP tools | `function_call` named `mcp__<server>__<tool>` (older: `<server>__<tool>`); in code mode `tools.mcp__<server>__<tool>(...)` inside `exec`. The local catalog printed by `ALL_TOOLS` shows names like `mcp__codex_apps__<connector>_<tool>` | JSON args; completion arrives as `item_completed McpToolCall` (camelCase) |
| `web_search_call` | `response_item` own type | `{id, status, action: search|open_page|find_in_page}` (hosted search; local corpus uses the Extension path instead) |
| `tool_search_call` / `tool_search_output` | own types | `{call_id, execution, arguments}` / `{tools:[..]}` |
| `image_generation_call` | own type | `{id, status, revised_prompt, result}` |

### 4.2 Code mode `exec` (what 100% of local tool use is)

The model writes JavaScript; the host runs it and the script calls nested tools through `await tools.<name>(args)` and prints with `text(...)`. Helpers (`U`, `code-mode-protocol/src/description.rs`): `text(v)`, `image(v)`, `store(k,v)`/`load(k)`, `notify(v)`, `exit()`, `yield_control()`, `ALL_TOOLS`, `tools.tool_search(...)`.

Input examples (shape only):
```js
text(await tools.exec_command({cmd:"<shell command>", max_output_tokens:8000}));
const r = await tools.exec_command({cmd:"<cmd>", yield_time_ms:1000, max_output_tokens:15000}); store("s", r.session_id); text(r);
text(await tools.write_stdin({session_id:20076, chars:"", yield_time_ms:1000, max_output_tokens:2500}));
text(await tools.web__run({search_query:[{q:"<q>"}], response_length:"long"}));   // also {open:[{ref_id|url,lineno}]}, {find:[{ref_id,pattern}]}
```
Keys seen in nested `exec_command` calls (count of occurrences): `cmd` 891, `max_output_tokens` 664, `yield_time_ms` 25, `workdir` 16. 45 of 735 scripts contain control flow (`for`/`const`/`await Promise...`), so a script can run many commands, conditionally, in parallel. Argument keys are object-literal keys, sometimes unquoted JS (`{cmd:"..."}`) and sometimes JSON quoted (`"max_output_tokens":8000`) in the same script. Do not `serde_json` the source. Treat the source as opaque text for display; take the structured facts (command, cwd, exit code, output, duration) from the `CommandExecution` events, and only fall back to a tolerant regex on `cmd:` when the events are missing.

Output (`custom_tool_call_output.output[]`, 735 of 735 begin `Script completed`):
- item 0, always: `"<status>\nWall time <n.n> seconds\nOutput:\n"`. `U` status strings: `Script completed`, `Script failed`, `Script running with cell ID <id>`, `Script terminated`. A thrown error adds an item `"Script error:\n<text>"`. Newer builds may append `(code-mode <x> seconds; overhead <y> seconds)` after the wall time. Only `Script completed` is seen here; **a failed script would look like a normal output**, so parse the first line for status.
- items 1..: one per `text()`/`image()` call. For `text(await tools.exec_command(...))` the text is a JSON string `{"chunk_id","wall_time_seconds","exit_code","original_token_count","output"}` (770 seen), or, when the process is still alive after `yield_time_ms`, `{"chunk_id","wall_time_seconds","session_id","original_token_count","output"}` with `session_id` instead of `exit_code` (17 seen). `text(r.output)` gives plain text; `text(web__run result)` gives plain text (68 plain items), `text(ALL_TOOLS)` a JSON list.
- Truncation: 127 items start with `Warning: truncated output (original token count: N)\nTotal output lines: M\n\n` followed by the (truncated) JSON. The inner `output` is truncated to the `max_output_tokens` the model asked for. This is what the MODEL saw. The full text is in the `CommandExecution.aggregated_output` event (up to 64 KiB). Prefer the event for display, keep the model-visible copy as `model_output`.

### 4.3 Direct (non code mode) output formats `U`

| Tool | `function_call_output.output` text |
|---|---|
| `exec_command` / `write_stdin` | `Chunk ID: <hex>\nWall time: 0.1234 seconds\nProcess exited with code 0\n` or `Process running with session ID <n>\n`, then `Original token count: <n>\nOutput:\n<text>`. Large outputs: `Warning: truncated output (original token count: N)\n...` |
| `shell` (structured JSON, verified in source at 0.114) | JSON string `{"output":"<text>","metadata":{"exit_code":0,"duration_seconds":0.1}}` |
| `shell_command`, `apply_patch` (freeform) | `Exit code: 0\nWall time: 0.1 seconds\n[Total output lines: N\n]Output:\n<text>` |
| MCP tool | `Wall time: 0.0100 seconds\nOutput:\n<text>`, or JSON of `structured_content`, or content items |
| `apply_patch` success body | `Success. Updated the following files:\nA <path>\nM <path>\nD <path>` |

### 4.4 Collaboration (multi agent v2) call outputs `M`

| call | `function_call_output.output` | event |
|---|---|---|
| `spawn_agent` | `{"task_name":"/root/<name>"}` | `SubAgentActivity` `started`, `id == call_id` |
| `send_message`, `followup_task` | `""` (empty string, 86 of 86) | `SubAgentActivity` `interacted`, `id == call_id` (86) |
| `wait_agent` | `{"message":"Wait completed.","timed_out":false}` or `{"message":"Wait timed out.","timed_out":true}` | `CollabAgentToolCall` `tool:"wait"`, `id == call_id` (8) |
| (child finishes) | none | `SubAgentActivity` `completed`, `id = "subagent-completed-<child uuid>"`, no call id (31) |

### 4.5 apply_patch grammar and result `U`

Input is the raw patch text (freeform tool, not JSON). Lark grammar from `codex-rs/core/assets/tools/apply_patch.lark`:
```
*** Begin Patch
[*** Environment ID: <id>]            (0.162 only, optional)
*** Add File: <path>        followed by lines starting with "+"
*** Delete File: <path>
*** Update File: <path>
[*** Move to: <new path>]
@@ [<context line>]                   change_context
 <context> | -<removed> | +<added>    change_line
[*** End of File]
*** End Patch
```
Result is reported as: `EventMsg::PatchApplyEnd {call_id, turn_id, stdout, stderr, success, changes:{path: add|delete|update}, status: completed|failed|declined}` (legacy mode, and Extended mode in old versions) or the paginated `item_completed FileChange` item above. `call_id` of `PatchApplyEnd` equals the `custom_tool_call.call_id`. The `changes` map holds full file content for add/delete and `unified_diff` for update, so it is the reliable typed edit record. Parse the patch text only for the call input view.

## 5. Shell and terminal command detail: where every field lives

| Field | Paginated 0.162 (measured) | Direct tool call (`U`) | Legacy event `exec_command_end` (`U`, Extended only, not persisted by default) |
|---|---|---|---|
| command | `CommandExecution.command` argv `["/bin/zsh","-lc","<cmd>"]` (910 of 910 use zsh -lc). Real command = element 2. Script `cmd:` has it too | `function_call.arguments.cmd` (exec_command), `.command[]` (shell), `.command` string (shell_command), `local_shell_call.action.command[]` | `command[]` |
| cwd / workdir | `CommandExecution.cwd` is a **`file://` URL** (`PathUri`, 910 of 910), e.g. `file:///path/to/dir`; decode to a path. Request side: `workdir:` key inside the script (16 uses) | `arguments.workdir`; `local_shell_call.action.working_directory` | `cwd` (PathUri) |
| session cwd fallback | `turn_context.cwd` | same | same |
| timeout | NOT recorded in the event. Only in the call: `timeout_ms` (U exec_command), `yield_time_ms` (how long the model waits before getting control, default 10,000 exec / 250 write_stdin) | `timeout_ms` | none |
| output limit requested | `max_output_tokens` in script (664 uses) | `max_output_tokens` | none |
| tty / login / shell | not recorded (`tty` defaults false, `shell`, `login` only in call args) | args | none |
| exit code | `CommandExecution.exit_code` i32 (848 zero, 44 one, 25 two). For long running processes it is absent from the first output JSON (has `session_id`), present on the late event | in the text header | `exit_code` |
| status | `CommandExecution.status`: `completed` (848) when exit 0, `failed` (69) when exit non-zero. `U` also `declined` (approval refused), `in_progress` | text | `status: completed|failed|declined` |
| duration | `CommandExecution.duration` `{secs, nanos}` Rust `Duration`. Values here are very small (microseconds to ms) because they time process execution only; the wall time the model saw is `wall_time_seconds` in the output JSON and the `Wall time N seconds` header (script total). Do not mix them | `Wall time: N seconds` in text; legacy `duration_seconds` in metadata | `duration` |
| stdout / stderr | **No separate fields.** `aggregated_output` is one interleaved string (pty or pipes merged). 909 of 909 events have it, max 65,536 chars | text body | `aggregated_output`, `formatted_output`; `U` legacy had `stdout`, `stderr` separately before merge |
| truncation | (a) model side: `Warning: truncated output (original token count: N)` + `original_token_count` in the output JSON; (b) persistence side: `aggregated_output` longer than 64 KiB is cut in the middle with `\n... command output truncated for persistence ...\n` (34 outputs affected) | head+tail marker `Total output lines: N` | n/a |
| sandbox / escalation | NOT on the event. Per turn: `turn_context.sandbox_policy`, `permission_profile`, `approval_policy`, `approvals_reviewer`. Per call (only inside the script / args): `sandbox_permissions` (`use_default|with_additional_permissions|require_escalated`), `additional_permissions`, `justification`, `prefix_rule` | args | none |
| process / session id | `CommandExecution.process_id` (string, e.g. "45132"); equals `session_id` in the exec output JSON and the `session_id` argument of `write_stdin` | `session_id` | `process_id` |
| source | `CommandExecution.source`: only `unified_exec_startup` (910) seen. `U` enum: `agent`, `user_shell` (the `!cmd` shell escape in the TUI), `unified_exec_startup`, `unified_exec_interaction` (a `write_stdin` with non empty chars; has `interaction_input`) | n/a | `source` |
| semantic class | `parsed_cmd[]` (Codex's own classifier): `read{cmd,name,path}` 1,225, `search{cmd,query,path}` 390, `list_files{cmd,path}` 50, `unknown{cmd}` 444. One shell line can yield several actions (a `cat a; cat b; rg x` gives 3). `path` can be null | n/a | `parsed_cmd` |
| plugin | `plugin_id`, `script_path` optional `U` | | |

Use `parsed_cmd` to fill typed Read/Search/List inputs while keeping the raw command; it is free, deterministic and comes from Codex itself.

## 6. Pairing and dedupe rules (all verified `M` unless noted)

| Need | Rule |
|---|---|
| tool call to its output | `call_id` (exact, 1:1 locally, 861 pairs, 0 orphans). Allow 1:N `U` (notify, wait) |
| exec script to its nested shell/web records | **No `call_id` on the events.** Events appear between the `custom_tool_call` and its `custom_tool_call_output`, in order. Group every `item_completed` of type `CommandExecution` or `Extension` that occurs after an open `exec` call and before its output (685 of 729 calls: event count equals the regex count of `tools.exec_command|write_stdin|web__run` in the source; the rest differ because of loops, `write_stdin` with empty chars (no event), a failed script, or a late event). Use the event `id` (`exec-<uuid>`) as the nested call id |
| late event | A process that outlived its script (output JSON had `session_id`) completes later: 12 `CommandExecution` events arrived with no open call. Join them by `process_id == session_id` to the earlier output (12 of 12 matched). Attach to the original nested call, set its status when it lands |
| `SubAgentActivity` / `CollabAgentToolCall` to the call | `item.id == function_call.call_id` for `started`, `interacted`, `CollabAgentToolCall`; `completed` has the synthetic id `subagent-completed-<child thread id>` |
| turn grouping | `turn_id` on events and `internal_chat_message_metadata_passthrough.turn_id` on response items (not on every message: 936 of 968; the 32 without are the trailing developer message inside each child's inherited prefix). `task_started`/`task_complete`/`turn_aborted` bracket the turn. Tool outputs also carry the turn id |
| assistant text dedupe | `item_completed AgentMessage` and `response_item message(assistant)` are the SAME message: equal `id` (`msg_...`), equal text, in 126 of 126 files. The event comes 1 line before the response item (859 of 859 pairs, incl. reasoning). Emit one. Prefer the response item (has `phase` too, and `create_time`) but both carry `phase`; the event adds `started_at_ms`/`completed_at_ms` |
| reasoning dedupe | `item_completed Reasoning` and `response_item reasoning` share `id` (`rs_...`): equal in 126 of 126. Emit one; the response item has the `encrypted_content` |
| user text dedupe | `UserMessage` event id is a UUIDv7 and does NOT equal the response item id (`msg_<uuid>`; they only share the time prefix). Match by text and position: the event text equals the `content[i].text` of the user `message` whose `content_item_kinds[i] == "user.text"` and `inherited_user_message` is false: 94 of 94 root user messages match exactly, and exactly one per root thread. Trust the **event** for user text (it is clean, has `text_elements`) |
| usage dedupe | use `token_usage_record` per `response_id`; ignore repeated `token_count` |
| child prefix | drop records with `ordinal < subagent_history_start_ordinal` (except identity from ordinal 0) |
| per-turn model | `turn_context.model`, applies to all records until the next `turn_context` |

Order inside one model response (as written): reasoning event, reasoning item, [commentary AgentMessage event + item], `custom_tool_call`, `token_usage_record`, nested `CommandExecution` events, `custom_tool_call_output`, `token_count`. The `token_usage_record` comes right after the call line, so it is the usage of the response that produced the call.

## 7. Reasoning, compaction, interrupts, subagents, instructions

- **Reasoning**: encrypted only (3.7). `summary`/`content` readable only for models that return them.
- **Compaction `U`** (never occurred locally): top-level `compacted` line, payload `{message: "<summary text>", replacement_history?: [ResponseItem...], replacement_history_metadata?: [CodexHarnessMetadata...], guardian_history?, retained_context?, mcp_resource_origins?, window_number, first_window_id, previous_window_id, window_id, compaction_response_id, latest_token_usage_record, resume_metadata{multi_agent_version,last_started_turn_id,turn_attribution,previous_turn_settings}}`. `replacement_history` is the new full model context (so a compacted file's logical conversation is not the concatenation of lines). Also `item_completed ContextCompaction{id}` and (legacy) `event_msg context_compacted`; `response_item` may carry `compaction{encrypted_content}` / `context_compaction`. `ResponseItem::Compaction` encrypted content is opaque. There is no `tokens_before/after` and no `trigger` (auto vs manual) field; before/after can be derived from `latest_token_usage_record` vs the next record. `SessionMeta.context_window.window_id` starts a context window id chain.
- **Rollback `U`**: `event_msg thread_rolled_back{num_turns}` drops the last N user turns (legacy marker). `thread/revert` makes a new `rollout_id` file with `history_base`.
- **Interrupts**: `turn_aborted` + developer `<turn_aborted>` marker (3.4). `U`: interrupted tool calls get synthetic "aborted" outputs; not verified here.
- **Errors `U`**: `event_msg error{message, codex_error_info}` is NOT persisted in paginated mode (transient list in 8); terminal turn errors are in `task_complete.error` / `turn_aborted.error` (absent locally). `stream_error`, `warning`, `model_reroute`, `deprecation_notice` are not persisted.
- **Instructions and context**: system prompt = `session_meta.base_instructions.text`; harness developer messages and AGENTS.md are `response_item` messages with the kinds in 3.6; the same data in structured form is in `world_state`. User instructions are NOT in `session_meta` any more.
- **Plan mode / todo**: `update_plan` is a `function_call` (args above) `U`; its output is the string `Plan updated`; `EventMsg::PlanUpdate` is not persisted. Plan mode text is `item_completed Plan{id,text}`.
- **Hooks `U`**: `HookPrompt` item, `<hook_prompt hook_run_id="..">` fragments in user messages, `hook_started/hook_completed` events not persisted. This machine has hooks configured (`hooks.json`) but none produced a record.
- **Titles**: not in the rollout. Use `state_5.threads.title` (= first user message, 126 of 126) or compute the first `UserMessage`; renames only in `session_index.jsonl` `U`.

## 8. What can actually be persisted (the rollout policy) `U`

Source: `codex-rs/rollout/src/policy.rs` at `rust-v0.162.1`. `persisted_rollout_item(item, history_mode)`.

| Rollout item | Persisted? |
|---|---|
| `session_meta`, `turn_context`, `compacted`, `token_usage_record`, `world_state`, `retained_context`, `security_risk_score`, `inter_agent_communication(_metadata)` | always |
| `response_item` | yes for: `message, agent_message, reasoning, local_shell_call, function_call, tool_search_call, function_call_output, tool_search_output, custom_tool_call, custom_tool_call_output, web_search_call, image_generation_call, configuration_update, compaction, context_compaction, additional_tools`. No for `compaction_trigger`, unknown |
| `realtime_item` | only `paginated` |

`event_msg` by mode:

| Event | `legacy` | `paginated` |
|---|---|---|
| `token_count`, `thread_goal_updated`, `thread_rolled_back`, `turn_aborted`, `task_started`, `task_complete`, `thread_settings_applied` | yes | yes |
| `item_completed` | only item types `FunctionCallOutput`, `Plan`, `Extension(Sleep)`, and `SubAgentActivity` of kind `completed` | **every** item type (command output truncated at 64 KiB, MCP results at 64 KiB) |
| `user_message`, `agent_message`, `agent_reasoning`, `agent_reasoning_raw_content`, `entered_review_mode`, `exited_review_mode`, `patch_apply_end`, `context_compacted`, `mcp_tool_call_end`, `web_search_end`, `image_generation_end` | yes | no (replaced by `item_completed`) |
| `sub_agent_activity` | yes unless kind `completed` | no (as item) |
| `exec_command_begin/end/output_delta`, `terminal_interaction`, `view_image_tool_call`, `patch_apply_begin/updated`, `turn_diff`, `error`, `warning`, `stream_error`, `plan_update`, `request_user_input`, `exec_approval_request`, `apply_patch_approval_request`, `collab_*_begin/end`, deltas, hooks, `item_started`, realtime, `model_reroute`, `guardian_*`, `session_configured`, `mcp_startup_*`, `shutdown_complete` | **no (transient)** | no |

Consequence: in current Codex, shell results exist ONLY as `item_completed CommandExecution` (paginated) or as text inside `function_call_output` (legacy). `exec_command_end` never reaches disk in 0.162. In older versions (about 0.114 to 0.14x, `EventPersistenceMode::Extended`, chosen by the client) `exec_command_end`, `patch_apply_end`, `mcp_tool_call_end`, `web_search_end`, `view_image_tool_call`, `error`, and collab `*_end` events were persisted; in default `Limited` mode they were not. Support them as optional enrichment keyed by `call_id`.

## 9. Version drift `U` (dated by release tag, only what I verified in source) and which shape each version writes

| Era (tag) | Shape |
|---|---|
| TypeScript CLI (before the Rust CLI, 2025-04/05) | `U` single pretty JSON file per session, not JSONL. Not verified, not in corpus. Treat unknown as skip |
| Rust, up to ~0.32 | **No envelope.** Line 1 = `SessionMeta` flat `{"id","timestamp","instructions"[, "git"]}`; later lines are bare `ResponseItem` objects (`{"type":"message",...}`) and `{"record_type":"state", ...}` state lines (verified in 0.6 to 0.24 `rollout.rs`). Persisted: message, reasoning, local_shell_call, function_call(_output), custom_tool_call(_output) (0.30 policy). No events at all, no turn_context |
| ~0.33 on (`RolloutLine` in `protocol` crate at 0.33.0) | Envelope `{"timestamp","type","payload"}` with `type` in `session_meta, response_item, event_msg, turn_context, compacted`. `session_meta.payload = {id, timestamp, cwd, originator, cli_version, instructions, git?}` (git flattened into meta line) |
| ~0.9x | Persisted events whitelist: `user_message, agent_message, agent_reasoning(_raw_content), token_count, context_compacted, entered/exited_review_mode, thread_rolled_back, undo_completed, turn_aborted`. `ExecCommandEnd` etc. NOT persisted (verified 0.90 `policy.rs`). `GhostSnapshot`, `Compaction` response items appear |
| ~0.114 | `EventPersistenceMode::{Limited, Extended}`; Extended adds `exec_command_end, patch_apply_end, mcp_tool_call_end, web_search_end, view_image_tool_call, error, collab *_end, dynamic_tool_call_*`. `turn_started/complete` added to Limited |
| 0.138 to 0.142 | `SubAgentActivity` first appears in protocol at 0.142.0 (2026-06-22) |
| **0.143.0 (2026-07-08)** | `world_state` and `ThreadHistoryMode` (`legacy|paginated`) exist; file may declare `history_mode: paginated` |
| **0.145.0 (2026-07-21)** | per line `ordinal`; `subagent_history_start_ordinal`; `cache_write_input_tokens` appears |
| **0.153.0 (2026-09-03)** | `token_usage_record` top-level line |
| 0.162.1 (this corpus) | everything above. `sandbox_policy` still present beside `permission_profile` |

Parsing strategy by era: key on the presence of `type` at top level. No `type` and has `id` + `timestamp` on line 1 is the flat era. Then read `payload.history_mode` (default legacy): legacy reads `event_msg user_message/agent_message/agent_reasoning` plus `response_item`; paginated reads `item_completed`. Dedupe rules differ: in legacy mode the user and agent messages come both as `event_msg user_message|agent_message` and `response_item message`, with NO shared id (`UserMessageEvent` has no id; ordering and text equality only) `U`.

## 10. The two existing parsers: hard won edge cases, and what they drop

Files: `~/coding/wend/crates/wend-core/src/parser/codex.rs` (+ `index/mod.rs::assemble_codex`) and `~/coding/qqoro/qoop-tui/crates/qoop-history/src/codex.rs`. Same logic, qoop's is a trimmed copy.

| Lesson they encode | Verdict against the measured data |
|---|---|
| Conversation comes from `event_msg/item_completed` `UserMessage` and `AgentMessage`; `response_item` messages are only a fallback when no completed items exist (two pass) | Correct for paginated: events are clean text, `response_item` user messages are wrapped in context blocks. But the fallback path is actually the **legacy-mode** path where `event_msg user_message/agent_message` is the primary and is ignored by both parsers. They never look at `user_message`/`agent_message` events |
| Content type spelled `text` for user and `Text` for agent; read `.text` regardless | Verified (3.11) |
| Strip `<recommended_plugins>` and `<environment_context>` blocks from user text in the fallback | Incomplete: also `# AGENTS.md instructions for ...` (79 messages), `<skills_instructions>`, `<permissions instructions>`, `<collaboration_mode>`, `<turn_aborted>`, `<user_instructions>`. Use `content_item_kinds` instead |
| Tool items become name-only nodes: `CommandExecution` named from `parsed_cmd[0].name` or `exec`, `McpToolCall` as `server/tool`, `Extension` by `kind`, `FileChange` paths only (content can be whole files) | Good caution. `agentparse` wants the full detail, so keep content but behind lazy/size capped fields. `parsed_cmd[0].name` is only set for `read` (`name` is the file), `unknown` has no `name`, so the node name falls back to `exec` for 444 of 2,109 actions: weak. `McpToolCall` reads snake `server/tool/arguments`: matches the camelCase item (those names are single words) |
| `session_id` taken from the first `session_meta.payload.session_id`; cwd from the first meta | **Bug for children**: in the 32 subagent files `session_id` is the ROOT id, so child sessions collide with their parent in wend's index. Use `payload.id` |
| Title = first user message, one line, 100 chars | Same as `state_5.threads.title`. For a child file the first "user" text is the inherited parent prompt |
| `ContextCompaction` item and top-level `compacted` set a `has_compaction` flag | Fine as a flag, but the summary/replacement history is dropped |
| Skips `Reasoning` ("encrypted server-side") | Verified true for this model; wrong for models that emit summaries |
| Ignore: `token_usage_record`, `token_count`, `turn_context`, `world_state`, `task_*`, `turn_aborted`, `SubAgentActivity`, `CollabAgentToolCall`, `custom_tool_call(_output)`, `function_call(_output)`, `agent_message` | All of that is what `agentparse` must model. Notably the wend index loses: model, effort, git branch (`git_branch: None`), tokens, the tool OUTPUT, exit codes, subagent links, interrupts |
| Corrupt line tolerance: skip and count | Keep. Live files can have a partial last line |
| `discover_codex` only matches `*.jsonl` | misses `.jsonl.zst` (`U` after 7 days) and `archived_sessions/` |
| qoop-history `route_line` ignores `ordinal`; messages have no ids | Add ordinal as entry id |

## 11. Critique of the proposed common model for Codex

Proposed: `Session`, `Entry{id,parent_id,ts,kind}`, `Message{role,parts,model,usage}`, events, `Part`, `ToolCall`, `ToolOutput`, `Usage`.

### What fits

| Model piece | Codex source |
|---|---|
| Session.id, cwd, cli_version, git_branch | `session_meta.id`, `cwd`, `cli_version`, `git.branch` (also `commit_hash`, `repository_url`) |
| Session.model(s) | distinct `turn_context.model` (+ `thread_settings_applied.model`, `collaboration_mode.settings.model`) |
| Session.started/updated | `session_meta.timestamp` / last line timestamp or `task_complete.completed_at` |
| Message role | `user`, `assistant`, `developer`; `system` does not occur (the system prompt is `base_instructions`) |
| Part::Text / Reasoning / Image / ToolCall | yes (Image only `U`) |
| Interrupt event | `turn_aborted` |
| Compaction event | `compacted` (`U`) |
| Subagent parent link | `parent_thread_id` / `forked_from_id` / `agent_path` |

### What does not fit, and what is missing

1. **`parent_id` tree: does not exist.** Codex is a flat, ordinal ordered log. Only cross THREAD links exist. Use `ordinal` as the id and give Entry a `turn_id` and `root_turn_id`; keep `parent_id` None. Add `Turn` boundaries (`turn_id, started_at, completed_at, duration_ms, time_to_first_token_ms, status completed|interrupted|error, model_context_window, trigger`) as entries or a side table.
2. **One logical item = several records.** Assistant text is an event AND a response item; a tool call is a response item call + N events + a response item output; usage is a separate record. Entry needs `sources: Vec<{ordinal, kind}>` (provenance) so consumers can map back to the file, and the parser must do the merge in section 6. Do not expose the raw duplicates.
3. **Message.usage per message is wrong for Codex.** Usage is per model RESPONSE (`response_id`), and one response yields several entries (reasoning, commentary, call). Put `Usage` + `response_id` on a `Response`/`ModelCall` grouping or on the first assistant entry of the response, and add session level `usage_total` plus the cumulative `thread_token_usage`. Fields: `input_tokens` INCLUDES `cached_input_tokens` and `reasoning_output_tokens` is INCLUDED in `output_tokens`. The common `Usage` must declare its convention (recommend storing OpenAI style raw numbers plus helper `uncached_input()`, with a flag `input_includes_cache: bool`; Anthropic is false). Add `cache_write`. `cost` is always None for Codex (compute from a price table keyed by model slug, none is on disk).
4. **Message role is not enough.** Real user text vs injected context (environment_context, AGENTS.md, skills, permissions, collaboration mode, turn_aborted marker, hook prompt, inherited parent prompt) share roles. Add `Message.origin: User | Assistant | HarnessContext{kind} | InterAgent{author,recipient} | Inherited` (driven by `content_item_kinds` and `metadata.inherited_user_message`), and `Message.hidden: bool` for context. Keep `phase: commentary|partial_answer|final_answer` on assistant messages (Codex decides turn end by the `final_answer` phase).
5. **Inter agent messages** (`agent_message`, `author`, `recipient`, `Message Type`, plain or `encrypted_content`) need a Part/Message kind with `encrypted: Option<blob>` and `readable: bool`. Do not map them to user.
6. **Reasoning**: `Reasoning{text: Option, encrypted: Option<String>, id}`; for Codex 0.162 text is always None. Also `Reasoning` may be summary only (`summary`) or raw (`content`); keep `kind: summary|raw`.
7. **ToolCall nesting (biggest mismatch).** One Codex `exec` call is a JavaScript program that makes 0..N nested tool invocations, with its own status line, wall time, `store/load` state, possible yield and `wait` continuation (`cell_id`). Required: `ToolCall::kind = CodeCell { source: String, cell_id: Option, status: completed|failed|running|terminated, wall_time_s, children: Vec<ToolCall> }`, each child a normal typed `ToolCall` (Shell/WebSearch/WriteStdin) with `id = exec-<uuid>`, plus `model_visible_output` (the JSON/text the model saw) distinct from the full `output` (the event). Pair children by order and `process_id`.
8. **Shell typed input** `Shell{command,cwd,timeout,background,description}`: Codex has no `description`; `background` ~ "yielded" (`yield_time_ms`, `session_id` returned). Add `argv: Vec<String>` (raw `["/bin/zsh","-lc",cmd]`), `shell_cmd: String` (element 2), `tty`, `login`, `yield_time_ms`, `max_output_tokens`, `sandbox_permissions`, `justification`, `prefix_rule`, `process_id`, `source (agent|user_shell|startup|interaction)`, `actions: Vec<ParsedAction{Read{name,path},Search{query,path},ListFiles{path},Unknown}>`, `interaction_input` (for stdin writes).
9. **ShellOutput** `{stdout,stderr,exit_code,duration,interrupted,truncated}`: Codex gives ONE `aggregated_output` (no stdout/stderr split), so declare `stdout: None, stderr: None, combined: Some`. Add `status: completed|failed|declined|in_progress`, `duration` as `{secs,nanos}` (and a separate `wall_time` seen by the model), `truncated: {model_side: Option<original_token_count>, persistence_side: bool}`. `exit_code` may be absent (still running; arrives later).
10. **Status mapping**: `completed`+exit 0 = ok; `failed` (non zero exit) = error; `declined` = rejected; turn aborted without output = interrupted; in-flight = pending. Note: `custom_tool_call.status="completed"` is NOT the tool status. A non zero exit inside a script still gives `Script completed`.
11. **Edit/Patch**: apply_patch grammar plus `FileChange{changes: path -> add|delete|update(unified_diff, move_path)}`. The common `Patch` input should hold raw patch text AND parsed hunks; the typed result is the per-path change map. `Write`/`Edit`/`Read` have no Codex native tools: file reads and writes occur through shell (`parsed_cmd` gives Read/Search/List) and apply_patch.
12. **Subagent / Task**: `Task{agent_thread_id, agent_path, nickname, model, reasoning_effort, task_name, depth, status (started/interacted/completed/interrupted), final_message}` and `Session.kind = Root | Subagent{parent_thread_id, depth, agent_path, nickname, role} | Fork`. Child session has `inherited_prefix_end_ordinal` (drop earlier records) and `session_id` (root) separate from `id`. Child prompt is often encrypted: `Task.prompt: Option<String>` with `prompt_encrypted: bool`. Tree is cross file, so `agentparse` needs a `resolve_children(root)` helper using `agent_thread_id == session_meta.id` and `state_5.sqlite.thread_spawn_edges` as accelerator.
13. **Missing event kinds to add**: `Turn(Start|Complete|Abort)`, `ThreadSettings{model,effort,cwd,approval,sandbox,permission_profile}` (from `turn_context` and `thread_settings_applied`; it is a per turn thing, so the "ModelChange" event should be derived by diffing), `RateLimits{plan,primary%,secondary%,resets_at}`, `ContextWindow{size}`, `WorldState` (optional), `HookPrompt`, `Plan`, `ReviewMode`, `Rollback{num_turns}`, `ContextCompaction`.
14. **Timestamps**: three clocks: line `timestamp` (write time), item `started_at_ms/completed_at_ms`, response `create_time` (float seconds). Provide `ts` (line) and `started_at/ended_at` when present.
15. **Truncation and size**: keep big fields lazy: `base_instructions` (20 KB per meta), `world_state` (22 KB avg), `aggregated_output` (up to 64 KiB), `encrypted_content` (up to 8.9 KB). Byte shares: item_completed 36.6%, custom_tool_call_output 29.4%, message 10.8%, world_state 8.3%, session_meta 6.6%. A streaming parser that skips `world_state`/`session_meta.base_instructions` by default saves ~15% of bytes.
16. **Session.title**: none in the file. `Option<String>` derived (first `UserMessage`), source `state_5.threads.title`.
17. **Cost**: not recoverable; `Usage.cost = None`.
18. **Raw escape hatch**: unknown `response_item.type`, `event_msg.type`, `item.type`, `kind` must round trip as `Raw(serde_json::Value)` with `ordinal`.

### Recommended mapping (Codex record to common model)

| Codex record | Common model |
|---|---|
| `session_meta` (first) | `Session{id=payload.id, root_id=payload.session_id, cwd, cli_version, git, started_at, source, kind, parent}` ; `base_instructions` lazy |
| `turn_context` | `ThreadSettings` on the turn (model, cwd, approval, sandbox, effort); update `Session.models` |
| `task_started` / `task_complete` / `turn_aborted` | `Turn` records; `task_complete.last_agent_message` = final answer check; aborted becomes `Interrupt` event |
| `UserMessage` event | `Message{role:user, origin:User, parts:[Text]}` (emit once) |
| `response_item message` role=user with kind other than `user.text` | `Message{origin:HarnessContext{kind}, hidden:true}` |
| `response_item message` role=developer | `HarnessContext` hidden; `generic.turn_aborted` becomes `Interrupt` marker |
| `AgentMessage` event + `message(assistant)` | one `Message{role:assistant, phase, parts:[Text]}` (dedupe by id) |
| `Reasoning` event + `reasoning` | `Part::Reasoning{text:None|joined, encrypted}` on the same response's assistant entry |
| `custom_tool_call exec` + output + nested events | `ToolCall::CodeCell{children}` ; output paired by `call_id`; children from `CommandExecution`/`Extension` events |
| `CommandExecution` | child `ToolCall::Shell` + `ShellOutput` (combined) + `parsed_cmd` actions |
| `Extension web.search` | child `ToolCall::WebSearch{action, results}` |
| `function_call` namespace `collaboration` + `SubAgentActivity`/`CollabAgentToolCall` | `ToolCall::Task` (+ `Session` link) |
| `function_call` / `custom_tool_call` other names `U` | typed by name: `exec_command`/`shell`/`shell_command`/`local_shell_call` -> Shell; `apply_patch` -> Patch; `update_plan` -> Todo; `view_image` -> Read(image); `mcp__x__y` -> Mcp{server,tool}; else Other |
| `token_usage_record` | `Usage` for the response (`response_id`), plus running totals |
| `token_count` | `RateLimits` + context fill (`info.total_token_usage`); do not add to usage |
| `compacted` `U` | `Compaction{summary=message, replacement_history, window ids}` |
| `world_state`, `thread_settings_applied`, `inter_agent_communication_metadata` | side records / `ThreadSettings`; metadata flag `trigger_turn` attaches to the next `agent_message` |

## 12. Not verified / open

| Item | Status |
|---|---|
| Real examples of `compacted`, `FileChange`, `McpToolCall`, `apply_patch`, `update_plan`, `view_image`, `local_shell_call`, `function_call exec_command`, images, `.zst`, archived, legacy mode, flat-era files | none on this machine, shapes taken from upstream source at the tags named. Build fixtures from the types, ideally from a real old install |
| Synthetic output for an interrupted in-flight call | not verified |
| Exact tag where default `Limited` persistence started whitelisting events (about 0.4x to 0.9x) | verified at 0.30 (no events), 0.54/0.90 (events whitelist), 0.114 (modes) only |
| `Script running with cell ID` plus `wait` flow, `notify` extra outputs | from source, not seen |
| Corpus is live: counts above come from the 126 file snapshot (some derived tables were first measured at 123 files and re-checked) | stable for shape, not for totals |
| No secrets or real prompts were copied into this file; placeholders only | |
