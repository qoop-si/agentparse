# OpenCode on-disk session format (research for agentparse)

Scope: sst/opencode (repo now `anomalyco/opencode`, GitHub redirects the old name). Product versions seen locally: `1.18.35` (v1 era) and `2.0.27` (v2 era, installed CLI reports `opencode v2.0.27`).
Method: local `~/.local/share/opencode/opencode.db` opened read-only, plus upstream source at tags `v1.18.35` and `v2.0.27` (branch `dev` has the same v2 layout). Every claim is tagged **[local]** (measured here), **[src]** (read in upstream source) or **[inferred]**.
Scratch scanners: `<scratchpad>/opencode-research/{scan,shapes,sm2,ord,sd}.py` (run with `uv run --no-project python`).

## 0. Summary of the most important findings

1. There are TWO generations of session storage inside one file, `opencode.db`. Legacy (v1): tables `session`, `message`, `part`, `todo` (rows hold a `data` JSON blob). Current (v2, written by opencode 2.x): tables `session_v2`, `session_message`, plus `instruction_*`, `session_pending`, `session_inbox`. **[local]** the 5 sessions written by 2.0.27 exist ONLY in the v2 tables; nothing wrote `session/message/part` after the upgrade.
2. v2 on upgrade copies every v1 session into `session_v2` and rewrites its messages into `session_message` (a lossy projection, see section 7). Local: 47 v1 sessions, 52 `session_v2` rows (47 migrated + 5 new). A parser must read v2 tables first and fall back to v1 tables only for sessions absent from `session_v2` (or when `session_v2` does not exist, i.e. opencode < 2.0).
3. The existing `qoop-history` parser reads only v1 tables. It is blind to every session created by opencode 2.x. It also keeps only text, tool names and patch file lists.
4. v1 message/part is a normalized graph (message rows plus part rows, ids sort chronologically). v2 `session_message` is a flat ordered log (`seq`) where one assistant row embeds `content[]` of text / reasoning / tool blocks. No separate step-start / step-finish / snapshot / patch parts exist in v2.
5. Tool results differ: v1 `state.output` is one string plus `state.metadata` (tool specific) plus `attachments`. v2 `state.content[]` is `[{type:"text",text}|{type:"file",uri,mime,name}]` plus optional `metadata`. v1 tool ids: `bash`, `task`, `apply_patch`, input key `filePath`. v2 renames to `shell`, `subagent`, `patch`, input key `path`.
6. Subagents: separate sessions linked by `session.parent_id`. The parent's `task` (v1) / `subagent` (v2) tool call carries the child id (`metadata.sessionId` v1, `metadata.sessionID` + `input.sessionID` v2). **[local]** no child sessions exist now; verified from source only.
7. Usage: `input` is NON-cached input, `output` EXCLUDES reasoning, `reasoning`, `cache.read`, `cache.write` are separate; v1 also has `total` (sum of all five, **[local]** matches exactly). `cost` is USD computed by opencode from models.dev prices (0 for free models).
8. Reasoning text is frequently empty: OpenAI reasoning is stored only as `metadata.openai.reasoningEncryptedContent` (v1) / `state.openai.reasoningEncryptedContent` (v2). **[local]** 101 of 101 reasoning parts have empty `text`. Anthropic stores `metadata.anthropic.signature`.
9. Timestamps are epoch milliseconds everywhere. Session ids are DESCENDING-sortable (`ses_` + inverted time), message/part ids ASCENDING (`msg_`, `prt_`). **[local]** verified. Order messages by `time_created, id` (v1) or `seq` (v2), never by id string alone across tables.
10. The pre-SQLite JSON tree (`storage/session`, `storage/message`, `storage/part`) is gone locally; only `storage/session_diff/*.json` (leftover, 6 files) and the marker `storage/migration` (content `2`) remain. The library should still support the legacy JSON layout (section 8) for old installs.
11. Compaction in v1 = a user message holding a `compaction` part plus a paired assistant message with `summary:true`. In v2 = one `session_message` of type `compaction` with `status`, `reason`, `summary`, `recent`.
12. No hidden thinking is lost, but the system prompt is not stored per message. v2 stores content-addressed instruction blobs (`instruction_state` to `instruction_blob`); v1 stores nothing (only `user.system` optional, absent locally).
13. Errors: v1 assistant `error` is `{name, data:{message,...}}` (8 named variants). v2 uses `{type, message, status?, response?:{body}}` on assistant and on `idle`. **[local]** 5 real v2 errors (HTTP 410 model deprecated).
14. Forks: Kilo Code CLI (`Kilo-Org/kilocode`, CLI 7.8.9) uses `~/.local/share/kilo/kilo.db` (env `KILO_DB`), v1-style `session/message/part` tables plus its own extras. Not installed locally. Other opencode channels use `opencode-<channel>.db`; `OPENCODE_DB` overrides.
15. Local data is thin: only `todowrite` tool calls exist in the live DB (54 parts). Real bash/edit/read/task shapes below come from upstream source, and tool-name frequencies from the user's older history seen through the `wend` index (names only, section 4.3).

## 1. Locations

| Path | What | State here |
|---|---|---|
| `~/.local/share/opencode/opencode.db` (+ `-wal`, `-shm`) | SQLite, all sessions (v1 and v2 tables) | 14 MB, WAL 6.5 MB live. Open read-only; WAL means you must read `-wal` too (use SQLite, not raw file copy) |
| `~/.local/share/opencode/opencode-<channel>.db` | non-latest release channels (dev, beta) [src v1.18.35 database.ts] | absent |
| `$OPENCODE_DB` (name or absolute path) | override [src] | n/a |
| `~/.local/share/opencode/opencode-next.db` | v2 alpha DB; v2 imports it once on upgrade [src v1-migration.bun.ts] | absent |
| `~/.local/share/opencode/storage/` | legacy JSON tree | only `session_diff/*.json` (6 files, 5 are `[]`, 1 has 1 FileDiff) and `migration` marker `2` |
| `~/.local/share/opencode/tool-output/` | full text of truncated tool output, files `tool_<id>`, 7 day retention, caps 2000 lines / 50 KiB [src tool-output.ts, v1 truncation-dir.ts] | empty. Paths referenced from old sessions will usually be gone: treat as optional |
| `~/.local/share/opencode/shell/<projectID>/` | v2 shell capture files (`Shell.Info.file`) [src shell.ts] | 3 empty dirs |
| `~/.local/share/opencode/snapshot/<projectID>/<hash>/` | bare git repos used for file snapshots (`step-start.snapshot`, `patch.hash`) [inferred from dir content: HEAD, objects, refs] | 10 dirs, 25 MB |
| `~/.local/share/opencode/log/opencode.log` | key=value text log, not session data | 27 MB |
| `~/.local/share/opencode/repos/` | git clones for plugins | empty |
| `~/.config/opencode/{opencode.jsonc,skills,commands,plugin}` | config, not sessions | n/a |
| `~/.local/share/kilo/kilo.db` | Kilo Code CLI [src Kilo-Org/kilocode packages/opencode/src/storage/db.ts, core/src/global.ts] | not installed |
| `~/.local/share/opencode-goal-plugin/` | a user plugin's own state (1145 files, goals.json) | not opencode core data |

## 2. Current vs legacy, and who writes what

| Era | opencode | Writes | Reads for history |
|---|---|---|---|
| JSON files | 0.x to ~1.1 | `storage/{project,session,message,part,todo,permission,session_share,session_diff}` | JSON tree |
| SQLite v1 | 1.2.0 (drizzle, `json-migration.ts` imports the JSON tree) to 1.19.x | `session`, `message`, `part`, `todo`, `project`, `permission`, `session_share` | those tables |
| SQLite v2 | 2.0.0 and later (local 2.0.27) | `session_v2`, `session_message`, `session_pending`, `session_inbox`, `instruction_*`, plus `project`, `kv` (`todo` still exists but its FK points at legacy `session`; v2 removed the `todowrite` tool) | `session_v2` + `session_message` |

* **[local]** `migration` table: 50 drizzle migrations; the first 38 (up to `20260622202450_simplify_session_input`) completed at ts 1791664251xxx, the last 12 (`20260804233008_loose_psylocke` ... `20261010180456_project_time_archived`) at 1791833906xxx. So `session_v2` and `session_message` come from the earlier batch (they already existed in the 1.18.x schema, the v2 engine was in the codebase behind the scenes) and the 2.0 upgrade added worktree, workspace_domain, session_inbox, viewed state, etc.
* **[local]** `kv` has `migration.v1-v2 = {"phase":"completed"}` (written at upgrade time, ts 1791833906057) and `models-dev:catalog` (a 5.9 MB cache of models.dev, NOT session data; has per model `cost`, `limit`, `reasoning`, useful for pricing but skip it).
* **[local]** proof that v2 stopped writing v1 tables: `max(session.time_updated)=1791832923513` < `min(session_v2.time_created where version='2.0.27')=1791836517883`.
* **[src]** The v1 to v2 migration is `packages/core/src/database/v1-migration.bun.ts` (`transformSession`). It is one-way and idempotent (`INSERT OR IGNORE`, cursor in `kv`). After it, v1 tables stay in the file as dead weight. `event` rows are DELETED by the migration ([local] `event` has 0 rows) and events are only persisted when `persist` is on for sync/replication ([src bus.ts]); the `session_message` rows are the durable truth. `event_sequence` has 52 rows (one per session, `seq` = last message seq).
* **Detection rule for the library:** if table `session_v2` exists, list sessions from it. For each id read `session_message`. Additionally list v1 `session` ids not present in `session_v2` (v1 DB never upgraded or partially migrated) and read them via `message`/`part`. Optionally prefer v1 for migrated sessions if you want the richer data (step parts, tool `time.compacted`, assistant `time.completed`, message `variant`, `tokens.total`): v1 rows for migrated sessions are still there and lossless, v2 rows are the lossy copy. Recommendation: for a migrated session (same id in both), prefer v1 tables; for new sessions (v2 only) use `session_message`.

## 3. Schema (exact, from `.schema`)

### 3.1 v2 (current) session tables

```sql
session_v2(id PK, project_id NOT NULL, workspace_id, parent_id, fork_session_id, fork_boundary /*json*/,
  slug NOT NULL, directory NOT NULL, path, title, version NOT NULL, share_url,
  summary_additions, summary_deletions, summary_files, summary_diffs /*json FileDiff.LegacyInfo[]*/,
  metadata /*json*/, cost real NOT NULL DEFAULT 0,
  tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write  /*int NOT NULL DEFAULT 0*/,
  revert /*json*/, permission /*json ruleset*/, agent, model /*json {id,providerID,variant?}*/,
  time_created NOT NULL, time_updated NOT NULL, time_compacting, time_archived,
  time_suspended, resume_attempts NOT NULL DEFAULT 0, time_idle, time_viewed, idle_outcome /*succeeded|failed|interrupted*/)
session_message(id PK, session_id NOT NULL, type NOT NULL, seq NOT NULL, time_created, time_updated, data /*json*/)
  UNIQUE(session_id, seq); idx(session_id,type,seq); idx(session_id,time_created,id)
session_pending(id PK, session_id, type, data json, delivery, admitted_seq, time_created)   /* queued or steered input not yet delivered */
session_inbox(id PK, session_id, type, payload json, delivery, enqueued_seq, time_created)  /* user, synthetic, compaction, move items */
instruction_state(session_id PK, epoch_start, through_seq, initial_values json, current_values json)
instruction_entry(session_id, key, value json, removed bool, time_created, time_updated)    PK(session_id,key)
instruction_blob(hash PK /*sha256 hex*/, value json)
event_sequence(aggregate_id PK, seq, owner_id)       event(id PK, aggregate_id, seq, type, data json, created)  /* event log, empty by default */
```

Notes:
* `session_message.data` EXCLUDES `id` and `type` (they are columns). `seq` is per session, dense from 0. **[local]** a migrated session has `seq` 0..n with the synthetic `system` notice last.
* `session_v2.tokens_*`/`cost` are running totals over all steps (updated by `session.usage.updated`). **[local]** migrated rows equal the v1 session totals (0 mismatches).
* `session_v2.model` is JSON text, `agent` plain text. **[local]** migrated: `{"id":"<model>","providerID":"<provider>","variant":"default"}`; the 5 new sessions have NULL agent/model (they failed before a step committed).
* `session_v2.directory` = cwd the session ran in. `path` = directory relative to the project worktree without leading slash (**[local]** `Users/<u>/proj` with worktree `/`); NULL on new rows here.
* `instruction_*`: **[local]** keys `core/codemode`, `core/skill-guidance`, `core/instructions` (AGENTS.md files as `[{path,content}]`), `core/date`, `core/environment` (cwd, platform, date). `initial_values`/`current_values` map key to blob hash; blob `value` is JSON (string or array). This is the only place the effective system context of a v2 session survives. `through_seq`/`epoch_start` = seq at which the context epoch began.

### 3.2 v1 (legacy) tables, still present

```sql
session(id PK, project_id NOT NULL, workspace_id, parent_id, slug NOT NULL, directory NOT NULL, path, title NOT NULL,
  version NOT NULL, share_url, summary_additions, summary_deletions, summary_files, summary_diffs json, metadata json,
  cost real, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, revert json,
  permission json, agent, model json, time_created, time_updated, time_compacting, time_archived)
message(id PK, session_id NOT NULL, time_created, time_updated, data json)   idx(session_id,time_created,id)
part(id PK, message_id NOT NULL, session_id NOT NULL, time_created, time_updated, data json)  idx(message_id,id), idx(session_id)
todo(session_id, content, status, priority, position, time_created, time_updated)  PK(session_id,position)
session_share(session_id PK, id, secret, url, ...)     permission(id, project_id, action, resource, ...)
```
* `message.data` / `part.data` OMIT `id`, `sessionID`, `messageID` (taken from columns). Rebuild: `{...data, id: row.id, sessionID: row.session_id, messageID: row.message_id}` [src message-v2.ts `info()`/`part()`].
* Part order: `ORDER BY message_id, id` [src hydrate()]. **[local]** id order equals time_created order in 148 of 148 messages.
* Older v1 DBs (1.2.x) lack `workspace_id`, `path`, `metadata`, usage columns, `agent`, `model` on `session` [src v1.2.0 session.sql.ts]; use `pragma_table_info` instead of fixed column lists (the existing parser already does this for `parent_id`).

### 3.3 Shared tables

| Table | Columns | Local rows | Use |
|---|---|---|---|
| `project` | id (git root commit sha, or `global`), worktree, vcs, name, icon_*, sandboxes json, commands json, time_*, time_active, time_archived | 4 (`global` worktree `/`, three real dirs) | map `project_id` to repo root |
| `project_directory`, `worktree`, `workspace` | multi dir / worktree / remote workspace bindings | 0 | skip |
| `todo` | session_id, content, status (pending/in_progress/completed/cancelled), priority (high/medium/low), position | 141 rows / 47 sessions | final todo state per session. v2 also keeps it (`todo` unchanged) |
| `permission`, `session_share`, `account*`, `credential`, `control_account` | | 0 | NEVER read `credential`, `account.access_token`, `control_account.*` (secrets) |
| `kv` | key, value | 2 | skip (`models-dev:catalog`), `migration.v1-v2` |

## 4. Measured counts [local]

### 4.1 Rows

| Table | Rows | | Table | Rows |
|---|---|---|---|---|
| session (v1) | 47 | | session_v2 | 52 (47 @1.18.35, 5 @2.0.27) |
| message (v1) | 148 | | session_message | 210 |
| part (v1) | 501 | | todo | 141 |
| event | 0 | | event_sequence | 52 |
| instruction_state | 5 | | instruction_blob | 6 |
| session_pending / session_inbox | 0 / 0 | | project | 4 |
| sessions with parent_id | 0 | | sessions with revert / share / summary_diffs | 0 / 0 / 0 |

### 4.2 v1 and v2 distributions

| Source | Key | Counts |
|---|---|---|
| message.role | user / assistant | 47 / 101 |
| part.type | text 144, step-start 101, reasoning 101, step-finish 101, tool 54 | none of: file, patch, agent, subtask, snapshot, compaction, retry |
| part sequence per assistant msg | `step-start>reasoning>text>tool>step-finish` x50, `step-start>reasoning>text>step-finish` x47, `step-start>reasoning>tool>step-finish` x4; user msgs: `text` x47 | exactly one step per assistant message |
| assistant.finish | stop 47, tool-calls 54 | step-finish.reason identical |
| tool name | todowrite 54 | only tool locally |
| tool state.status | completed 54 | no pending/running/error locally |
| assistant model | one model, provider `opencode`, agent=mode=`build` for 101/101 | |
| text part flags | `synthetic`/`ignored` never present (0/144) | |
| text metadata | `metadata.openai.itemId` 97, `metadata.openai.phase="commentary"` 50 | |
| session_message.type | assistant 106, user 52, system 47, idle 5 | no compaction, shell, synthetic, skill, switch events |
| assistant content shapes | `[reasoning,text,tool]` 50, `[reasoning,text]` 47, `[reasoning,tool]` 4, `[]` 5 (error rows) | |
| idle.outcome | failed 5 | error = provider.invalid-request / HTTP 410 |

### 4.3 Historical tool usage (names only), from the `wend` index of 3781 earlier opencode sessions (those sessions are no longer in the live DB)

`bash` 22857, `read` 12299, `webfetch` 11414, `websearch` 7680, `edit` 6368, `grep` 2051, `todowrite` 1788, `write` 1293, `glob` 716, `task` 709, plugin tools `get_goal` 108 / `update_goal` 16 / `create_goal` 14 / `set_goal` 6 / `list_all_goals` 6 / ..., `skill` 70, `question` 35, `invalid` 25, MCP `shannon-tools_save_deliverable` 7. Content kinds in that index: thinking 33919, text 25106, tool_use 67469, image 64. No `apply_patch`, `list`, `lsp`, `codesearch`. Conclusion: bash/read/edit/webfetch/websearch/grep/task/write/glob/todowrite/skill/question/invalid plus MCP and plugin tools are the real-world set; the parser must keep unknown tool names as `Other` with raw JSON.

## 5. v1 shapes (tables `message`, `part`)

### 5.1 Message info (`message.data`)

User: `{role:"user", time:{created}, agent, model:{providerID, modelID, variant?}, summary?:{title?, body?, diffs:[FileDiff]}, format?:{type:"text"}|{type:"json_schema",schema,retryCount}, system?:string, tools?:{<name>:bool}}`
**[local]** only `role, time.created, agent, model.{providerID,modelID}, summary.diffs:[]`.

Assistant: `{role:"assistant", time:{created, completed?}, parentID, modelID, providerID, mode, agent, path:{cwd, root}, cost, tokens:{total?, input, output, reasoning, cache:{read, write}}, finish?, error?, summary?:bool, structured?:any, variant?}`
**[local]** all present except error/summary/structured/variant. `mode` is the legacy name of `agent` (equal 101/101). `path.root` is the project worktree (`/` for the `global` project). `summary:true` marks the compaction summary message. `parentID` = the user message that triggered the turn (many assistant messages can share one parent: one per tool-loop step).

`error` variants [src schema/v1/session.ts], discriminated by `name`:

| name | data |
|---|---|
| `APIError` | `{message, statusCode?, isRetryable, responseHeaders?, responseBody?, metadata?}` |
| `ProviderAuthError` | `{providerID, message}` |
| `MessageAbortedError` | `{message}` (user interrupt) |
| `MessageOutputLengthError` | `{}` |
| `ContextOverflowError` | `{message, responseBody?}` |
| `ContentFilterError` | `{message}` |
| `StructuredOutputError` | `{message, retries}` |
| `UnknownError` | `{message, ref?}` |

`finish` values seen/known: `stop`, `tool-calls`, `length`, `content-filter`, `error`, `unknown` (free string in v1; v2 normalizes to this set, anything else becomes `unknown`).

Session row to model: `title`, `directory`, `parent_id`, `slug` (human id like `adj-noun`), `version` (opencode version that created it; **[local]** 1.18.35 for all v1), `share_url`, `summary_*`, `revert` (`{messageID, partID?, snapshot?, diff?}`), `permission` (ruleset `[{permission, pattern, action: allow|deny|ask}]`, **[local]** 47/47 hold the 3 default denies `question`, `plan_enter`, `plan_exit`), `metadata` (host annotations, NULL locally), `time_compacting`, `time_archived`.

### 5.2 Part types (`part.data`), all carry `type`

| type | keys (beyond ids) | notes |
|---|---|---|
| `text` | `text`, `synthetic?`, `ignored?`, `time?{start,end?}`, `metadata?{<provider>:{...}}` | user text and assistant text. `synthetic:true` = opencode-injected (reminders, "Called the Read tool with the following input: ...", "The following tool was executed by the user", "Summarize the task tool output above and continue with your task."). `ignored:true` = shown in UI, not sent to model. **[local]** metadata `openai.itemId`, `openai.phase` |
| `reasoning` | `text`, `time{start,end?}`, `metadata?` | **[local]** text empty, `metadata.openai.itemId` + `reasoningEncryptedContent` (1.4 KB to 11 KB). Anthropic: `metadata.anthropic.signature` |
| `tool` | `callID`, `tool`, `state`, `metadata?` | see 5.3 |
| `step-start` | `snapshot?` (git tree hash) | **[local]** no keys but `type` |
| `step-finish` | `reason`, `cost`, `tokens{total?,input,output,reasoning,cache{read,write}}`, `snapshot?` | per LLM call usage. Message tokens = sum of its steps |
| `file` | `mime`, `filename?`, `url`, `source?` | user attachment or tool attachment. `url` is `data:<mime>;base64,...` or `file:///abs/path` or `https://`. `source`: `{type:"file",path,text:{value,start,end}}` / `{type:"symbol",path,range,name,kind,text}` / `{type:"resource",clientName,uri,text}`. `mime` `text/plain` and `application/x-directory` get converted to synthetic text read results, not sent as files |
| `patch` | `hash` (snapshot git tree), `files:[abs path]` | written after a step that changed files (snapshot diff) |
| `snapshot` | `snapshot` (hash) | start marker |
| `agent` | `name`, `source?{value,start,end}` | user `@agent` mention |
| `subtask` | `prompt`, `description`, `agent`, `model?{providerID,modelID}`, `command?` | pending subagent request on a user message, executed into a `task` tool part on the following assistant message |
| `compaction` | `auto:bool`, `overflow?:bool`, `tail_start_id?` | on a USER message; the paired assistant (parentID = that user msg, `summary:true`) holds the summary text parts. `tail_start_id` = first message kept verbatim after the summary |
| `retry` | `attempt`, `error: APIError`, `time{created}` | provider retry notice inside an assistant message |

### 5.3 Tool part state

```jsonc
{ "type":"tool", "tool":"<name>", "callID":"<provider call id>", "metadata":{"<provider>":{"itemId":"..."},"providerExecuted"?:true},
  "state": ONE OF
  {"status":"pending",   "input":{...}, "raw":"<partial json string>"}
  {"status":"running",   "input":{...}, "title"?:"", "metadata"?:{...}, "time":{"start"}}
  {"status":"completed", "input":{...}, "output":"<string>", "title":"", "metadata":{...},
                         "time":{"start","end","compacted"?:ms}, "attachments"?:[FilePart]}
  {"status":"error",     "input":{...}, "error":"<string>", "metadata"?:{...}, "time":{"start","end"}} }
```
* `callID`: provider call id (`call_...`); a user-run shell command uses a fresh `ulid()`.
* `time.compacted` set means old output was pruned: output text is replaced by `[Old tool result content cleared]` when sent to the model, but `state.output` in the DB still holds the real text unless a later prune cleared it. v2 migration writes the placeholder text for these.
* `status:"error"` with `metadata.interrupted === true` and `metadata.output` = partial output of an interrupted command (stdout so far).
* An unfinished `pending`/`running` part left in the DB means the run was killed; opencode itself replays it as "[Tool execution was interrupted]". Map to `Interrupted`.
* **[local]** only completed `todowrite`: `input.todos:[{content,status,priority}]`, `output` = pretty-printed JSON of todos, `title` "N todos" (N = non-completed), `metadata{todos, truncated:false}`, `time{start,end}`.
* Every tool result passes a truncation wrapper [src tool/tool.ts]: `metadata.truncated:bool` and, when true, `metadata.outputPath` (file under `tool-output/`). `output` then contains the head/tail plus a hint.

### 5.4 Per tool shapes (v1 ids), from source at v1.18.35

| tool | `state.input` | `state.metadata` (completed) | `state.title` | output |
|---|---|---|---|---|
| `bash` | `{command, timeout?(ms), workdir?, description?}` | `{output:<tail preview>, exit:<int|null>, truncated:bool, outputPath?, description?}` | the command | stdout+stderr merged (one stream), tail-truncated to 2000 lines/50 KiB, may end with `<shell_metadata>...timeout/aborted notice</shell_metadata>`; `(no output)` if empty |
| `read` | `{filePath, offset?, limit?}` | file: `{preview, truncated, loaded:[AGENTS.md paths]}`; dir: `{preview, truncated, loaded:[]}` | relative path | numbered lines `N: text`, trailer "(Showing lines a-b of n...)". Image/PDF returns `attachments` |
| `edit` | `{filePath, oldString, newString, replaceAll?}` | `{diff:<unified>, filediff:{file,patch,additions,deletions}, diagnostics:{<file>:[LSP diag]}}` | relative path | "Edit applied successfully." plus LSP error block |
| `write` | `{filePath, content}` | `{filepath, exists:bool, diagnostics}` | relative path | "Wrote file successfully." plus LSP block |
| `apply_patch` | `{patchText}` | `{diff:<combined>, files:[{filePath, relativePath, type:add|update|delete|move, diff, before, after, additions, deletions, movePath?}], diagnostics}` | the output summary | "Success. Updated the following files:\nA/M/D path" |
| `grep` | `{pattern, path?, include?}` | `{matches:int, truncated:bool}` | pattern | grouped `path:\n  Line N: text` |
| `glob` | `{pattern, path?}` | `{count, truncated}` | relative search dir | abs paths one per line, or "No files found" |
| `task` | `{description, prompt, subagent_type, task_id?, command?, background?}` | `{parentSessionId, sessionId:<child ses_...>, model:{modelID,providerID}, background?:true, jobId?}` | description | `<task id="<childSessionId>" state="completed|running|error">[<summary>]<task_result>...</task_result></task>` |
| `todowrite` | `{todos:[{content,status,priority}]}` | `{todos, truncated}` | "N todos" | JSON of todos |
| `webfetch` | `{url, format?(text|markdown|html), timeout?(s)}` | `{}` (+ `truncated`) | `url (content-type)` | body converted to the format; media returns `attachments` |
| `websearch` | `{query, numResults?, livecrawl?, type?, contextMaxCharacters?}` | `{provider}` | `<provider> "<query>"` | provider text |
| `question` | `{questions:[Prompt]}` | `{answers:[[string]]}` | "Asked N question(s)" | sentence with answers |
| `skill` | `{name}` | `{name, dir}` | "Loaded skill: <name>" | `<skill_content>` block with file list |
| `lsp` | `{operation, filePath, line, character, query?}` | `{result}` | | JSON |
| `plan_enter`/`plan_exit` | `{}` | `{}` | "Switching to build agent" | |
| `invalid` | `{tool, error}` | `{}` | "Invalid Tool" | error text (model called a bad tool or bad args) |
| MCP tool | whatever the server declares | `{...server metadata, truncated, outputPath?}`; `attachments` for image/resource blobs | `""` | text parts joined by blank line. Name = `<server>_<tool>` with every char outside `[A-Za-z0-9_-]` replaced by `_` (**[local-history]** `shannon-tools_save_deliverable`). Server/tool split is ambiguous when either contains `_`: resolve from `opencode.jsonc` mcp keys, else split at first `_` as a heuristic |
| MCP resource tools | `list_mcp_resources{server?}`, `list_mcp_resource_templates{server?}`, `read_mcp_resource{server,uri}` | `{server, uri, attachments:n, truncated}` | | |
| plugin/custom tool | arbitrary | arbitrary | | e.g. goal plugin `get_goal`, `create_goal` |

User-run shell (`!cmd` in the TUI) [src prompt.ts shellImpl]: a user message with one synthetic text part "The following tool was executed by the user", then an assistant message (`mode`=`agent`=chosen agent, tokens/cost 0) holding ONE `bash` tool part with `input:{command}` and NO step-start/step-finish. Detect via the synthetic text on the parent user message.

Sub-agent recognition in v1: parent tool part `tool=="task"`; child session `parent_id == parent session id`; `metadata.sessionId == child.id`; child title is `"<description> (@<agent> subagent)"`. Child user message text = the `prompt`. Resumed tasks reuse `task_id`.

## 6. v2 shapes (table `session_message`, column `data`; `id`, `type`, `seq` are columns)

Common: `time:{created}` (+ `completed`), `metadata?` (free). Timestamps ms.

| `type` | data keys | Meaning |
|---|---|---|
| `user` | `text`, `files?:[{data:<base64>, mime, source:{type:"inline"}|{type:"uri",uri}, name?, description?, mention?:{start,end,text}}]`, `agents?:[{name, mention?}]`, `skills?:[{id,name,text?,mention?}]` | prompt. **[local]** `files:[]` present on new rows, absent on migrated rows |
| `synthetic` | `text`, `description?` | opencode injected text as if from user: background shell/subagent completion notices (`<shell id=.. state=.. command=..>`, `<task ...>`), reminders. `metadata.source`="shell", `shellID`, `jobID`, `state`, exit info |
| `system` | `text` (model-facing update, frozen), `description?` | **[local]** 47 rows, all the migration notice "The available tools have changed. The `todowrite` tool is no longer available..." appended AFTER the last message of each migrated session. Not user content: drop or mark `SystemNotice` |
| `skill` | `skill` (id), `name`, `text` | a skill activation |
| `shell` | `shellID`, `command`, `status: running|exited|timeout|killed`, `exit?`, `output?:{output, cursor, size, truncated}`, `time{created, completed?}` | user-run `!` command (the v2 equivalent of the v1 synthetic bash pattern). Full detail |
| `assistant` | `agent`, `model:{id, providerID, variant?}`, `content:[Text|Reasoning|Tool]`, `snapshot?{start?,end?,files?}`, `finish?`, `rawFinish?`, `providerState?`, `cost?`, `tokens?{input,output,reasoning,cache{read,write}}`, `error?`, `retry?{attempt,at,error}`, `time{created, streamed?, completed?}` | one LLM step |
| `compaction` | `status: running|completed|failed`, `reason: auto|manual`, `summary`, `recent`, `model?`, `providerState?`, `providerContext?`, `cost?`, `tokens?` (usage of the compaction call), `error?` on failed | `summary` = model summary text; `recent` = serialized verbatim tail |
| `idle` | `outcome: succeeded|failed|interrupted`, `error?` | turn boundary marker: all assistant rows since the previous `idle` form one turn. **[local]** 5 rows, `failed` |
| `agent-switched` | `agent`, `previous?` | mid session agent change |
| `model-switched` | `model:{id,providerID,variant?}`, `previous?` | |
| `location-switched` | `location`, `projectID?`, `subpath?`, `previous?` | session moved to another directory/worktree |

Assistant content blocks:

| block | keys |
|---|---|
| `text` | `text`, `state?` (provider state, e.g. `openai.itemId`, `openai.phase`) |
| `reasoning` | `text`, `state?` (**[local]** `openai.reasoningEncryptedContent`), `time?{created,completed?}` |
| `tool` | `id` (call id), `name`, `executed?` (provider executed tool), `providerState?`, `providerResultState?`, `state` (below), `time{created, ran?, completed?}` |

Tool state: `{status:"streaming", input:"<partial json string>"}` | `{status:"running", input:{}, metadata:{}}` | `{status:"completed", input:{}, content:[{type:"text",text}|{type:"file",uri,mime,name?}] (non empty), metadata?:{}}` | `{status:"error", input:{}, error:{type, message, status?, response?}, content?, metadata?}`.
No `title` and no separate `output` string in v2. `content` is what the model saw. Migrated failed/interrupted v1 tools: error `{type:"tool.execution"|"tool.interrupted", message}`.

Error object: `{type:string, message:string, status?:100..599, response?:{body:string}}`. Types seen in source/migration: `provider.invalid-request`, `provider.auth`, `provider.content-filter`, `provider.invalid-output`, `provider.error`, `aborted`, `tool.execution`, `tool.interrupted`, `unknown`.

v2 tool names and inputs [src core/src/tool/plugin/*.ts]:

| name | input | metadata | notes |
|---|---|---|---|
| `shell` | `{command, workdir?, timeout?(ms), background?}` | `{truncated, exit?, signal?, timeout?, status?:"completed"|"running", shellID?}` | background returns "Command moved to the background (shell ID: sh_...)", completion arrives as a `synthetic` message. Default timeout 120000 ms. Output file path in `shell/<projectID>/` |
| `read` | `{path, offset?, limit?}` | `{truncated}` | content: numbered text or media `file` |
| `edit` | `{path, oldString, newString, replaceAll?}` | `{files:[FileDiff{file,patch,additions,deletions,status}]}` | structured output also has `replacements` |
| `write` | `{path, content}` | `{files:[FileDiff]}` | |
| `patch` | `{patchText}` | `{files:[FileDiff]}` | output has `applied:[{type:add|update|delete,...}]` |
| `grep` | `{pattern, path?, include?, literal?, caseSensitive?, limit?}` | `{matches, truncated}` | |
| `glob` | `{pattern, path?, hidden?, limit?}` | `{count, truncated}` | |
| `webfetch` | `{url, format?, timeout?}` | `{contentType}` (or the input on permission ask) | |
| `websearch` | `{query}` | `{provider}` | |
| `subagent` | `{agent, description, prompt, model?("provider/model#variant"), sessionID?, background?}` | `{sessionID:<child>, status:"completed"|"running"}` | link to child session |
| `question` | `{questions:[...]}` | `{answers}` | |
| `skill` | `{id}` | `{name, directory}` | |
| `list_mcp_resources`, `read_mcp_resource` | | | |
| MCP | `<ns>_<tool>` (`ns` = server with non `[A-Za-z0-9_-]` replaced by `_`) | | |
| `opencode`-ns tools | `session_rename`, `session_move`, `worktree_create/list/remove`, `models` | | |
| `todowrite` | REMOVED in v2 as a tool; todos are still in the `todo` table | | |

Usage in v2: `tokens` omitted while a step is in flight or failed; `cost` USD number. `tokens.total` is not stored; compute `input+output+reasoning+cache.read+cache.write` (matches v1 `total` locally).

## 7. What the v1 to v2 migration drops (lossy list) [src v1-migration.bun.ts transformSession]

| v1 data | Fate in `session_message` |
|---|---|
| user message `agent`, `model`, `tools`, `system`, `format`, `summary` | dropped (session-level `agent`/`model` kept) |
| assistant `mode`, `path.cwd/root`, `parentID`, `structured`, `summary` flag | dropped (`summary:true` messages are folded into the `compaction` row) |
| assistant `time.completed` | replaced by the message ROW's `time_updated` (not the original value) |
| assistant `variant` | stored as `model.variant`, default `"default"` |
| `step-start`, `step-finish` parts | dropped; `snapshot` hashes folded into `snapshot.{start,end}`; per step `reason`, `cost` and tokens are only on the message |
| `tokens.total` | dropped |
| `snapshot`, `patch` parts | folded into `snapshot.{start,files}` |
| `retry`, `subtask`, `agent` parts | `retry`/`subtask` dropped; `agent` becomes `agents[]`; a user message with ONLY subtask parts disappears, and the paired assistant `task` message is dropped |
| user `file` parts with non data: URL | replaced by text "[Attachment unavailable after migration: <name> (<mime>)]" |
| text `ignored:true` | dropped; `synthetic:true` becomes a separate `synthetic` row |
| tool `title`, `state.time.start/end` | `title` dropped; start/end kept as `time.created/completed` |
| tool `attachments` | become `content[]` file items with `uri` |
| tool pending / running | converted to error `tool.interrupted` |
| tool names | NOT renamed in stored rows (still `bash`, `task`, `apply_patch`) although new tools are `shell`/`subagent`/`patch`; the `system` notice explains the rename to the model |
| compaction | user(compaction part) + assistant(summary) fold into one `compaction{status:"completed", reason, summary, recent}`; error or unfinished summaries vanish |
| messages with decode failure | skipped with a warning (not stored) |
| reasoning `metadata` | kept as `state` |

Consequence: for migrated sessions the stored tool block still uses v1 names and v1 `metadata`, but `content` replaces `output`. A tool block's v1 vs v2 identity cannot be told from `type`; use the tool `name` and whether `state.metadata` has v1 keys. Rule: dispatch on `name` with BOTH name families (`bash|shell`, `task|subagent`, `apply_patch|patch`) and BOTH path keys (`filePath|path`).

## 8. Legacy JSON layout (opencode < 1.2, plus leftovers) [src storage.ts v1.18.35 migrations 1 and 2, json-migration.ts v1.2.0]

Root `~/.local/share/opencode/storage/`. File name = id, `.json`, pretty printed. Contents are the FULL objects (include `id`, `sessionID`, `messageID`, unlike DB rows).

| Path | Content |
|---|---|
| `project/<projectID>.json` | `{id, vcs:"git", worktree, time:{created,initialized}}` (`global` for non git dirs) |
| `session/<projectID>/<sessionID>.json` | session Info: `{id, projectID, directory, parentID?, title, version, time:{created,updated,compacting?}, summary?:{additions,deletions,files?}, share?:{url}, revert?}` |
| `message/<sessionID>/<messageID>.json` | message Info (same as section 5.1 plus `id`, `sessionID`) |
| `part/<messageID>/<partID>.json` | part (same as 5.2 plus ids) |
| `session_diff/<sessionID>.json` | `FileDiff[]` `{file, patch, additions, deletions, status?}` (split out of `session.summary.diffs` by storage migration 2). **[local]** present: 6 files |
| `todo/<sessionID>.json` | `[{id?, content, status, priority}]` |
| `permission/<projectID>.json`, `session_share/<sessionID>.json` | rulesets / share info |
| `migration` | integer marker of storage migrations applied (**[local]** `2`) |

Pre-1.0 layout (migration 1 source): `~/.local/share/opencode/project/<cwdHash>/storage/session/{info/<sid>.json, message/<sid>/<mid>.json, part/<sid>/<mid>/<pid>.json}`. Message and part payloads are the same schema. Parts are ordered by id; messages by id (ascending ids).
**[local]** none of the JSON tree remains except `session_diff`, so tests need fixtures; build them from the schemas above.

## 9. Existing parser review: `qoop-tui/crates/qoop-history/src/opencode.rs` (+ model.rs, lib.rs, locate.rs, dir.rs)

| Aspect | Current behaviour | Gap |
|---|---|---|
| Tables | `session`, `message`, `part` only | **Misses all sessions created by opencode >= 2.0** (`session_v2`/`session_message`). Locally the 5 newest sessions would be invisible to `top_sessions`, `read_session`, `typed` |
| Model | `Message{role: user|assistant, text, ts}` | no tool I/O, usage, reasoning, errors, subagents, compaction, cwd/title/model |
| Message set | user + assistant rows; others skipped and counted as "corrupt" | `system`, `synthetic`, `compaction`, `idle`, `shell` types of v2 do not exist for it |
| Parts kept | `text` (including synthetic ones in `read_session`; `typed()` filters `synthetic`), `tool` (name only), `patch` (file list as "edited files:") | drops `reasoning`, `file`, `step-*`, `snapshot`, `agent`, `subtask`, `compaction`, `retry`, tool input/output/error/metadata, `ignored` flag is not honored, timestamps of parts |
| Text join | parts joined by `\n`, ordered by `time_created, id` | fine |
| Ts | `data.time.created` else column | ok (ms) |
| Subagents | `top_sessions` excludes `parent_id IS NOT NULL` (via `pragma_table_info`) | no parent link retained |
| Locking | `OPEN_READONLY` + 2 s busy timeout | good; note a read-only connection cannot create `-shm` if absent; the live WAL is read through SQLite which is correct |
| Tests | synthetic schema without `workspace_id`, `path`, `agent`... | needs v2 fixtures |

## 10. Version drift

| Marker | Facts |
|---|---|
| `session.version` / `session_v2.version` | version that CREATED the session; a session resumed by a newer app keeps the old value. **[local]** v1 rows `1.18.35` x47 (migrated rows keep it), new rows `2.0.27` x5 |
| Releases seen on GitHub | v1.18.31 (2026-09-14) ... v1.18.35 (2026-10-06), v1.19.0 (2026-10-11); v2.0.x current |
| Schema changes (drizzle migration names, first seen order) | 20260227 `workspace_id`; 20260428 `session.path`; 20260510 usage columns (cost, tokens_*) on session; 20260511 `session.metadata`; 20260601 normalize storage paths; 20260602 `project_directory`; 20260603/0604 `session_message` projection + inbox + event sourced input; 20260605 context snapshot + `agent`; 20260622 reset v2 session state; 20260804 loose_psylocke; 20260808 workspace_domain; 20260811 execution claim attempts; 20260812 session_inbox; 20260812 worktree; 20260819 viewed state (`time_viewed`); 20260910 clear_v1_session_permission (v1 `permission` set NULL in v2); 20260923 `project.time_active`; 20261010 `project.time_archived` |
| Part/message field additions over time [src] | `tokens.total`, `cost` per step, `time.compacted` on tool state, `compaction.tail_start_id` + `overflow`, `ContentFilterError`, `StructuredOutputError` + `format`/`structured`, `retry` part, `variant`, `RetryPart`, `file.source.type=resource`. Everything optional, so use lenient serde (`#[serde(default)]`, unknown keys kept in a `raw` field) |
| Tool renames | v1 `bash`, `task`, `apply_patch`, `filePath` to v2 `shell`, `subagent`, `patch`, `path`; v2 skill input `id` (v1 `name`); subagent input `agent`/`sessionID` (v1 `subagent_type`/`task_id`) |
| DB name | `opencode.db` (latest channel); `opencode-<channel>.db` otherwise; Kilo `kilo.db` |

## 11. Critique of the proposed common model for OpenCode

### 11.1 What fits

* `Session`: id, source path (the DB file plus session id; there is no per session file), cwd (`directory`), title, parent link (`parent_id`), started/updated (`time_created`/`time_updated`), cli_version (`version`), models (per assistant message; session level `model` is only the LAST selected), agent (`agent`).
* `Entry`: v1 has real `id` and `parentID` (assistant to user); v2 has id plus `seq` only (no parent). `parent_id` must be Option and derived (v2: previous entry).
* Message / Text / Reasoning / File / ToolCall / Usage / Compaction: all map.

### 11.2 What does not fit or is missing

| Issue | Detail | Proposal |
|---|---|---|
| Storage is a DB, not a file | "source path" cannot identify a session; reading needs a connection and WAL | `SourceRef::SqliteRow{db_path, session_id, generation: V1|V2}` |
| Two generations | same session may exist in both tables | `Session.source_generation`, prefer V1 rows for migrated ids (richer), document it |
| `Message` vs `Step` | v1 assistant message = ONE step; many assistant messages per user turn; v2 `idle` marks turn end | add `turn_id` (v1: user message id = `parentID`; v2: index since last `idle`) and `Entry.step_index`. Do not merge steps into one message: usage and finish are per step |
| `git_branch` | not stored anywhere. `project.vcs`/`worktree` only | leave None; optionally read via `project.worktree` |
| Model switch events | v1 stores model per message (no event); v2 has `model-switched`/`agent-switched`/`location-switched` | derive `ModelChange` entries in v1 by diffing consecutive assistant models; map v2 events 1:1 |
| Role set | v2 `synthetic` and `system` rows are neither user nor assistant | keep `Role::User` + `origin: Synthetic` flag for `synthetic`; `system` row becomes `SystemNotice`; v1 `text.synthetic` becomes the same flag on the part |
| Hidden vs shown | `ignored:true` text is UI only | `Part::Text{hidden: bool}` |
| Reasoning | `text` usually empty; payload is opaque provider state | `Reasoning{text, encrypted: Option<Opaque{provider, kind, bytes_len}>}`, store provider state map as `provider_state: Value`; never log the encrypted blob |
| Tool call pairing | already paired (call, input, result in one block); streaming/pending states exist | `ToolCall.status` needs `Pending(raw partial json)`, `Running`, `Ok`, `Error`, `Interrupted`; the proposed `rejected` does not exist for opencode (permission denial appears as `error` text such as "The user rejected permission..." [inferred, not seen locally]) |
| Typed input names | v1/v2 differ (`filePath`/`path`, `bash`/`shell`, `task`/`subagent`) | normalize through a single `ToolKind` classifier, keep `raw_name` |
| Shell output | one merged stream, no stdout/stderr split; `exit` may be null (killed/timeout); v1 puts timeout/abort notices INSIDE the output as `<shell_metadata>`; v2 has `timeout`, `signal`, `status:"running"` for background jobs | `ShellOutput{combined: String, stdout: None, stderr: None, exit_code: Option<i32>, signal: Option<String>, timed_out: bool, interrupted: bool, truncated: bool, full_output_path: Option<PathBuf>, background: Option<ShellId>}`. No duration field is stored; compute `time.end-time.start` |
| Shell input | `workdir`, `timeout`, `description`(v1 only, optional), `background`(v2) | fits `Shell{command,cwd,timeout,background,description}` |
| Tool output attachments | images/PDF as `file` parts (v1) or `content[].file` (v2) with data URLs | `ToolResult{text, files:[FileRef]}`; do not inline base64 by default |
| Edit/Write/Patch results | v1 `metadata.filediff{file,patch,additions,deletions}`, `diff`, LSP `diagnostics`; v2 `metadata.files[]` | typed `FileChange{path, patch, additions, deletions, status}` + `diagnostics: Value` |
| Subagent link | child session id inside tool metadata; not in the message stream; child has its own `parent_id` | `ToolCall::Task{agent, description, prompt, child_session: Option<SessionId>, background}` and `Session.parent` both; child session entries are NOT embedded in the parent |
| Compaction | v1 two messages (+ `tail_start_id`, `auto`, `overflow`); v2 single row with `summary`, `recent`, usage, status failed/running | `Compaction{summary, trigger: Auto|Manual|Overflow, tail_start: Option<EntryId>, recent_text: Option<String>, status, usage: Option<Usage>}`. `tokens_before/after` do NOT exist; use the compaction call's own usage only |
| Usage | per step (assistant) and cumulative (session); cost is a computed USD number; `total` v1 only | `Usage{input, output, reasoning, cache_read, cache_write, total: Option<u64>, cost_usd: Option<f64>}`; document semantic: `input` = non cached; add `Usage::input_incl_cache()` |
| Errors | v1 named variants, v2 `{type,message,status,response.body}`; error often attaches to the assistant message together with `finish:"error"` and EMPTY content | `Entry::Error{kind, message, status, body}` emitted in addition to the (possibly empty) message; map v1 `MessageAbortedError` and v2 `idle.outcome=interrupted` to `Interrupt` |
| Retry | v1 `retry` part (attempt, error); v2 `assistant.retry` | `Entry::Retry{attempt, error}` |
| Todo | table `todo` (final state) and `todowrite` calls (v1 only) | `ToolCall::Todo{items}` from input; session level `todos` from table |
| Snapshot / revert / diffs | `step-start.snapshot`, `patch{hash,files}`, `session.revert`, `summary_diffs`, `session_diff` | optional `Entry::Snapshot{hash}` / `Entry::FilesChanged{files}`; `Session.revert: Option<Value>`. Git objects live in `snapshot/` (do not read) |
| System prompt | not stored in v1; v2 in `instruction_*` | `Session.instructions: Vec<{key, text}>` optional, lazily resolved |
| Share / workspace | `share_url`, `workspace_id`, `fork_session_id`, `fork_boundary` | `Session.extra: Value` (pass through unknown columns) |
| Timestamps | ms integers; v1 migrated `completed` is wrong (row `time_updated`) | `ts` in ms; for migrated rows prefer v1 data |
| Ordering | v1: `time_created, id` for messages, `id` for parts; v2: `seq` | keep an explicit `ordinal` per entry |
| IDs | session ids sort DESC by time, others ASC | never sort sessions by id and assume age |
| Secrets | `credential`, `account` tables, encrypted reasoning blobs, `session_share.secret` | never read these tables |

### 11.3 Recommended mapping tables

Session

| common | v2 source | v1 source |
|---|---|---|
| id | `session_v2.id` | `session.id` |
| cwd | `directory` | `directory` |
| title | `title` (may be NULL) | `title` |
| cli_version | `version` | `version` |
| started_at / updated_at | `time_created` / `time_updated` | same |
| parent | `parent_id` | `parent_id` |
| models | distinct `assistant.model` | distinct `assistant.providerID/modelID` |
| agent | `agent` | `agent` or first user `agent` |
| git_branch | None | None |
| usage totals | `tokens_*`, `cost` | same |
| project root | `project.worktree` | same |

Entry kinds

| common | v1 | v2 |
|---|---|---|
| Message(user) | `message` role user, parts text/file/agent | `user` |
| Message(user, synthetic) | text `synthetic:true` | `synthetic` |
| Message(assistant) | `message` role assistant, parts reasoning/text/tool in id order | `assistant.content[]` |
| usage | `message.tokens/cost` (equal to the step-finish) | `assistant.tokens/cost` |
| Compaction | user msg with `compaction` part + assistant `summary:true` | `compaction` |
| Error | `assistant.error` | `assistant.error`, `idle.error`, tool `state.error`, `compaction.error` |
| Interrupt | `MessageAbortedError`, tool `error` with `metadata.interrupted`, pending/running leftovers | `idle.outcome=interrupted`, `tool.interrupted` |
| SystemNotice | none | `system`, `synthetic` with `metadata.source` |
| ModelChange | derived | `model-switched` / `agent-switched` / `location-switched` |
| Shell (user run) | assistant with single `bash` tool, parent synthetic text | `shell` |
| Hook / Attachment | none / `file` parts | none / `user.files` |

ToolCall typing (classify on normalized name, accept both generations)

| ToolKind | names | input mapping |
|---|---|---|
| Shell | `bash`, `shell` | command, workdir to cwd, timeout, background, description |
| Read | `read` | `filePath|path`, offset, limit |
| Write | `write` | `filePath|path`, content |
| Edit | `edit` | `filePath|path`, oldString, newString, replaceAll |
| Patch | `apply_patch`, `patch` | `patchText` (apply_patch format, not unified diff) |
| Search | `grep` | pattern, path, include, literal?, caseSensitive?, limit? |
| Glob | `glob` | pattern, path, hidden?, limit? |
| List | none as a tool in opencode; `read` on a directory is the list; keep `List` variant empty for this agent | |
| WebFetch / WebSearch | `webfetch`, `websearch` | url/format/timeout, query (+ numResults, type) |
| Todo | `todowrite` (v1) | todos |
| Task | `task`, `subagent` | description, prompt, `subagent_type|agent`, `task_id|sessionID`, background, model |
| Question | `question` | questions; output answers |
| Skill | `skill` | name or id |
| Mcp | anything matching a configured server prefix `<server>_`, or `list_mcp_resources`/`read_mcp_resource` | server, tool, raw input |
| Lsp | `lsp` | operation, filePath, line, character |
| Other | `invalid` (keep: it is a failed call, `input{tool,error}`), plugin tools, unknown | raw |

## 12. Edge cases and parser rules

1. WAL: the DB is live. Use SQLite (read-only URI, busy timeout), never copy the main file alone. Snapshot with `PRAGMA query_only=1`.
2. `data` columns are TEXT JSON; `json_extract` works. A `session_message.data` object has no `id`/`type`: add from columns.
3. Empty assistant `content: []` with `finish:"error"` and an `error` object is a real failed request (5 locally). Emit an Error entry; do not drop the message silently.
4. `tokens`/`cost` absent on in-flight or failed v2 steps; v1 cost may be integer `0` while others are float.
5. v1 `step-finish.tokens.total` may be undefined on old rows; compute.
6. Reasoning with empty `text` is normal; mark `encrypted` rather than skipping.
7. Dangling tool parts (`pending`/`running`) mean the process died.
8. `state.output` can be huge (up to 50 KiB per call after truncation, but `metadata.output` in bash duplicates a preview); `state.attachments[].url` can be a multi MB data URL. Stream/limit, offer `include_blobs` flag.
9. Tool `callID` format is provider dependent (`call_...`, `toolu_...`, ulid for user shell); do not parse.
10. `session.directory` can be `""` in very old rows [src directoryColumn comment]; `path` is relative, no leading slash.
11. Project `global` has worktree `/`; the real cwd is `directory`, never `project.worktree`.
12. Subagent children appear as normal rows in `session`/`session_v2`; they have no marker other than `parent_id` and the title suffix `(@<agent> subagent)`. Filter or link them explicitly.
13. Time order across `session_message` types is by `seq`; `time_created` of the system migration notice equals the last message's time, so sorting by time alone would be unstable (ties).
14. Migrated `system` notice must not be treated as the user's or the model's speech.
15. Truncated outputs reference `tool-output/tool_*` files that may be deleted after 7 days: resolve lazily, tolerate absence.
16. `project_id` may reference a missing project in old data; v2 migration re-parents to `global`.
17. Free-text `title` can be NULL in v2 (not NOT NULL as in v1).
18. A v1 user message may consist only of `subtask`/`agent`/`compaction` parts and have no text.
19. Idempotency: ids are stable across v1 and v2 for migrated rows (message id == session_message id), so de-duplicate on id.

## 13. Upstream references (all `anomalyco/opencode`; Kilo `Kilo-Org/kilocode`)

* v1 schemas: `packages/schema/src/v1/session.ts` @v2.0.27 (parts, messages, errors); at v1.18.35 `packages/opencode/src/session/message-v2.ts`, `session/session.ts`, `session/processor.ts`, `session/compaction.ts`, `session/prompt.ts`
* v2 schemas: `packages/schema/src/session-message.ts`, `session-event.ts`, `session-error.ts`, `session-inbox.ts`, `session-revert.ts`, `session.ts`, `token-usage.ts`, `shell.ts`, `prompt.ts`, `tool.ts`
* Drizzle tables: `packages/core/src/session/sql.ts`, `packages/core/src/event/sql.ts`, `packages/core/src/database/migration/*.ts`
* Migration: `packages/core/src/database/v1-migration.bun.ts` (`transformSession`, `migrateTool`, `migrateError`, `legacyToolNotice`)
* Projection of events into messages: `packages/core/src/session/projector.ts`, `message-updater.ts`, `packages/core/src/bus.ts`
* Tools v1: `packages/opencode/src/tool/{shell,read,edit,write,apply_patch,grep,glob,task,todo,webfetch,websearch,question,skill,invalid,lsp,plan,truncate}.ts`, `session/tools.ts` (MCP wrapping)
* Tools v2: `packages/core/src/tool/plugin/{shell,read,edit,write,patch,grep,glob,webfetch,websearch,subagent,question,skill,mcp-resource,opencode}.ts`, `tool/mcp.ts`, `tool-output.ts`, `shell/result.ts`
* Usage: `packages/opencode/src/session/session.ts getUsage` (v1), `packages/core/src/session/usage.ts` (v2)
* Legacy JSON: `packages/opencode/src/storage/storage.ts` @v1.18.35, `storage/json-migration.ts` @v1.2.0, `session/session.sql.ts` @v1.2.0
* Kilo: `packages/opencode/src/storage/db.ts`, `packages/core/src/global.ts`, `packages/core/src/session/sql.ts` (v1 tables plus `session_message`, `session_input`, `session_context_epoch`)

## 14. Next steps for the library

1. Build fixtures: (a) real v2 DB rows trimmed and anonymized from this machine (5 error sessions and 3 migrated todowrite sessions), (b) synthetic v1 rows for bash/edit/task/compaction/error built from sections 5 and 6, (c) JSON tree fixture from section 8.
2. Implement `Generation::{Json, SqliteV1, SqliteV2}` detection by `sqlite_master` and `pragma_table_info`.
3. Reader returns lazily: session list first (cheap columns), entries on demand, blobs (`file` data URLs, encrypted reasoning, `tool-output` files) behind flags.
4. Extend `qoop-history` (or replace it with agentparse) to list from `session_v2`; until then opencode 2.x sessions are invisible to it.
