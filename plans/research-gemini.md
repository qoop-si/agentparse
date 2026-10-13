# Gemini CLI session format research

Scope: on-disk session format of google-gemini/gemini-cli, for `agentparse`. Verified on 2026-10-13
against upstream `main` (HEAD around v0.65.0-nightly), tags v0.9 to v0.63, the installed CLI
(`gemini --version` = 0.63.0, bundle at `~/.bun/install/global/node_modules/@google/gemini-cli/bundle`),
and the 2 local sessions. Everything below marked "src" is read from upstream source, not guessed.

## 1. Headline findings

1. There are THREE on-disk generations, and all three exist in the wild:
   - G1 `session-*.json`: one JSON object `{sessionId, projectHash, startTime, lastUpdated, messages[], summary?, directories?, kind?}`.
     Written v0.4.0 (Sep 2025) until v0.38.x. The whole file is rewritten on every save.
   - G2 `session-*.jsonl`, "snapshot" style: line 1 header, then bare message records (upsert by id), `{"$set":{...}}`
     lines, and `{"$set":{"messages":[ALL]}}` full snapshots that REPLACE the message list. First stable: v0.39.0
     (2026-04-23, PR #23749). This is what the 2 local files and the installed 0.63.0 write.
   - G3 `session-*.jsonl`, "append-only delta" style: same header, bare message records, `$set` for metadata only,
     plus `{"$patch":{...}}` and `{"$rewindTo":"<id>"}`. Merged 2026-10-01 (PR #29568), first in v0.64.0-preview.0
     (2026-10-06). NOT in stable v0.63.0 yet. Readers must accept all three mixed in one file (upstream's own loader does).
2. The same upstream loader (`createJsonlRecordAccumulator`) defines the exact semantics. Port it, do not reinvent it (section 4).
3. A `gemini` message is a whole model turn: reasoning (`thoughts[]`), text (`content`), tool calls WITH results
   (`toolCalls[]`), `tokens`, `model`. In G2/G3 `content` is later overwritten by the raw `Part[]` of the turn
   (thought parts, text, functionCall parts), and every tool result is ALSO echoed as a separate `type:"user"` message whose
   content is only `functionResponse` parts. Naive readers double count results and invent user turns.
4. Shell results are plain text, not structured: `Output: ...\nExit Code: N\nSignal: ...\nBackground PIDs: ...\nProcess Group PGID: ...`.
   Exit code is printed only when non-zero (modern). A non-zero exit still has `status:"success"`. No duration, no separate stderr (since v0.28).
5. The session file has NO cwd, NO git branch, NO CLI version, NO parent link for subagents (only the path), and model only per
   `gemini` message. cwd comes from the sibling `.project_root` file in the project temp dir.
6. The existing `qoop-history` Gemini parser reads only G1 and silently skips every G2/G3 file (section 9).

## 2. Where files live

```
~/.gemini/
  projects.json                       { "projects": { "<abs project root>": "<slug>" } }   (since v0.29.0, 2026-02)
  tmp/<slug>/                         slug = slugified basename, "-1","-2" on collision. Before v0.29: tmp/<sha256(projectRoot)>/
    .project_root                     raw abs project root, no trailing newline (the cwd!)
    logs.json                         [] or [{sessionId, messageId:number, timestamp, type:"user", message:string}]  (prompt log only)
    logs/                             (empty dir locally)
    chats/session-<YYYY-MM-DDTHH-MM>-<first 8 chars of sessionId>.jsonl     (collision: ...-<n>-<shortId>.jsonl)
    chats/<parentSessionId>/<agentId>.jsonl                                  subagent sessions (kind:"subagent")
    tool-outputs/session-<id>/...     saved full outputs of truncated shell results (path appears inside the result text)
    checkpoint-<tag>.json             /chat save checkpoints, separate format, ignore
  history/<slug>/                     shell history, ignore
```

- Filename minute stamp is local-time-free: `toISOString().slice(0,16)` so it is UTC. `YYYY-MM-DDTHH-MM`.
- The header `projectHash` is ALWAYS `sha256(projectRoot)` hex (src `getProjectHash`), even though the directory is now a slug.
  So to find the cwd: (1) `tmp/<slug>/.project_root`, (2) reverse lookup in `projects.json`, (3) match `sha256(candidate)` to `projectHash`,
  (4) parse the first user message `<session_context>` block (`Workspace Directories:` list). Local example: dir `qass-1`, hash starts `2e7ae02f...`.
- `listProjectChatFiles` accepts both `.json` and `.jsonl`. Deleting/cleanup code matches `-<shortId>.json` and `.jsonl`.
- Resume of a G1 `.json` file migrates it: writes a new `.jsonl` next to it (`file.json` + `l`), the old file stays. Expect duplicates
  of one sessionId (same id in `.json` and `.jsonl`): dedupe by sessionId, prefer `.jsonl`.
- `rewriteConversationFile` can leave `*.unreadable-<ms>` and `*.tmp-<pid>` siblings. Ignore anything not ending `.json`/`.jsonl`.

## 3. The 2 local files (exact structure, content replaced)

Both are G2, 2 lines, no assistant turn at all (sessions opened and closed, or driven by another tool).

```
L1 {"sessionId":"<uuid>","projectHash":"<64 hex>","startTime":"<iso>","lastUpdated":"<iso same as start>","kind":"main"}
L2 {"$set":{"messages":[{"id":"d04923d38bb0f6017037e74183378ef4","timestamp":"<iso>","type":"user",
                         "content":[{"text":"<session_context>\n...OS, date, temp dir, Workspace Directories, directory tree,\n<loaded_context> GEMINI.md imports ...</session_context>"}]}],
            "lastUpdated":"<iso>"}}
```

- Header keys exactly: `sessionId, projectHash, startTime, lastUpdated, kind`. No `summary`, `directories`, `memoryScratchpad`.
- `$set` here carries `messages` (full replacement, 1 element) plus `lastUpdated`. So in G2 `$set.messages` REPLACES the whole array,
  it does not patch it. No `$push`, `$unset`, `$inc` exist anywhere upstream; the only operators are `$set`, `$patch`, `$rewindTo`.
- The only message is a synthetic user turn `<session_context>` (2.8 KB, 0.8 KB). Its `id` is the SAME 32-hex value
  (`d04923d3...`) in both sessions: ids of synthetic context turns are deterministic, not unique across sessions. Scope ids to a session.
- `content` of a user message is always an array of `Part` in G2/G3 (`[{"text":...}]`). The `<session_context>` turn must be classified
  as context (upstream `isIgnoredUserContent` ignores empty text, text starting with `/`, `?`, `<session_context>`, `<hook_context>`).
- No trailing partial lines; each line ends `\n`. The writer is `fs.appendFileSync(JSON.stringify(rec)+'\n')`, so a crash can leave
  one truncated last line: skip unparsable lines (upstream does).

## 4. Record grammar and reduction (src: chatRecordingService.ts `createJsonlRecordAccumulator`)

Classification order per line (first match wins):

| test | record | effect |
|---|---|---|
| has string `$rewindTo` | `{"$rewindTo":"<messageId>"}` | delete that message and every message after it (insertion order). Unknown id: clear ALL messages |
| has object `$patch` | `{"$patch":{id?,content?,toolCalls?,updates?[],removeIds?[],orderIds?[]}}` | see below |
| has string `id` (and no `$patch`) | a full MessageRecord | upsert by id: `Map.set`, so an existing id KEEPS its original position and is replaced wholesale |
| has object `$set` | `{"$set":{...}}` | `memoryScratchpad` key resets staleness tracking. If `$set.messages` is an array: clear map, insert all (replace). Then shallow-merge ALL `$set` keys into metadata |
| has string `sessionId` and `projectHash` | header (`PartialMetadataRecord`) | shallow-merge into metadata (and `messages` if present). Normally line 1 |
| none | ignored | |

`$patch` (G3 only):
- `id` + `content?` + `toolCalls?: [{id, result?}]` is a single patch; `updates` is an array of the same shape.
  Apply: `content` replaces `message.content`; for each `toolCalls[i]` find the toolCall by id on that gemini message and replace its `result`
  (only if key `result` present; `result: null` is a valid replace).
- `removeIds`: delete those messages. `orderIds`: ids not listed keep their relative order FIRST, then listed ids in the listed order.
- Patches to unknown ids are no-ops.

Metadata fields merged from header and `$set`: `sessionId, projectHash, startTime, lastUpdated, summary, memoryScratchpad, directories, kind`.
`finalize` requires `sessionId` and `projectHash` or the file is "not a session"; then falls back to parsing the whole file as one G1 JSON
object (`parseLegacyContentFallback`, anything with a `sessionId` key). So: try JSONL reduce first, else whole-file JSON.

What writers emit (src):
- New session: header line `{sessionId, projectHash, startTime, lastUpdated, kind, directories}` (`kind`/`directories` omitted by JSON.stringify when undefined; `kind` was absent before v0.37.0; main sessions in v0.63 write `"kind":"main"` as seen locally).
- Every recorded message: the message line, then `{"$set":{"lastUpdated":"<iso>"}}`.
- The same message id is appended AGAIN whenever it mutates (tokens arrive, toolCalls added). Last line wins, position stays. A real session therefore has many duplicate-id lines: that is normal, not corruption.
- G2 (`updateMessagesFromHistory`): after history sync writes `{"$set":{"messages":[ALL current],"lastUpdated"}}`. File size is quadratic in turns.
  Because it replaces the list, anything removed by `/compress` or rollback disappears from the REDUCED state although the earlier bare lines are still in the file.
  A forensic reader that wants "everything ever said" must also keep a log of every id seen before the replacement (see 8.2).
- G3: the same sync writes `$patch` (content/toolCalls result updates, removeIds, orderIds) and `$rewindTo` for a pure tail rollback; new turns are appended as bare records.
- `summary`: `{"$set":{"summary":"<one line>"}}`, generated asynchronously by a summary service (v0.2x+, #14467); may be missing.
- `directories`: `{"$set":{"directories":[...]}}` after `/dir add` (#17454); subagent headers carry the workspace dirs.
- `memoryScratchpad`: `{"$set":{"memoryScratchpad":{version:1, workflowSummary?, toolSequence?[], touchedPaths?[], validationStatus?:"passed"|"failed"|"unknown"}}}` (v0.41.0+). Useful, cheap, optional.

## 5. Field reference (src: chatRecordingTypes.ts)

### 5.1 Session metadata

| field | type | opt | notes |
|---|---|---|---|
| sessionId | string uuid | no | equals the prompt id of the main agent; subagent: the agentId |
| projectHash | string | no | sha256 hex of the abs project root |
| startTime / lastUpdated | ISO-8601 UTC with ms, `Z` | startTime defaulted to now on read if absent | |
| kind | `"main"` or `"subagent"` | yes | absent in G1 and pre-v0.37 |
| summary | string | yes | one-line generated title (v0.2x+) |
| directories | string[] | yes | extra workspace dirs |
| memoryScratchpad | object | yes | see above |
| messages | MessageRecord[] | G1 only (and legacy `$set`) | |

Not stored anywhere: cwd, git branch, CLI version, OS, approval mode, auth type, session end reason.

### 5.2 MessageRecord

Common: `id: string` (UUID, or 32 hex for deterministic synthetic turns), `timestamp: ISO`, `content: PartListUnion`, `displayContent?: PartListUnion`
(v0.2x+, set only when the text the UI showed differs from what was sent, e.g. `@file` expansion or slash commands).

`type`:
| type | extra fields | meaning |
|---|---|---|
| `user` | none | user prompt, OR a synthetic turn: `<session_context>`, `<hook_context>`, tool responses (parts are `functionResponse`), interrupted-tool placeholders, `/command` text, binary injections |
| `gemini` | `toolCalls?`, `thoughts?`, `tokens?` (nullable), `model?` | a model turn |
| `info` | none | UI notices: compression result, other info. `content` is a string |
| `error` | none | interactive-only errors (`[API Error: ...]` etc.), v0.1x+ (#13300) |
| `warning` | none | interactive warnings |

`content` shapes seen: `string` (G1 gemini, info/error/warning), `Part[]` (user always in G2/G3; gemini in G2/G3 after history sync), and per the
type `PartListUnion` also a single `Part` or a mixed string/Part array (rare, tests use `result: 'raw-output'`). Normalize: string => `[{text}]`, object => `[obj]`.

`Part` (from @google/genai), keys actually relevant:
```
{ text?: string, thought?: true, thoughtSignature?: base64 string,
  inlineData?: { mimeType, data: base64, displayName? },        // images/pdf pasted or read_file on binary
  fileData?: { mimeType, fileUri },
  functionCall?: { id?: string, name, args: object },
  functionResponse?: { id?: string, name, response: { output?: string, error?: string, ...}, parts?: Part[] /* nested inlineData */ },
  executableCode?, codeExecutionResult? }
```
- `thought:true` text parts are REASONING and must not be concatenated into answer text. Format is `**Subject** description` when it came from a thought summary (see `convertSessionToClientHistory`: `**${subject}** ${description}`), but modern parts can be raw text.
- `functionCall.id` / `functionResponse.id` equal `ToolCallRecord.id`. Missing ids are synthesized upstream as `synth_<name>_<hash>` / `synth_orph_<name>_<hash>`; not persisted.
- `response` is `{output:string}` on success, `{error:string}` on failure, `{}` only with nested `parts`. Cancelled tools may have both `output` and `error` (`[Operation Cancelled] <reason>`).
- Large inline binary read via `read_file` is moved into `response.__binary_injection__`-style key (`BINARY_INJECTION_KEY`) and a following `gemini` "Binary content received. Proceeding with analysis." (thought part, `thoughtSignature` = synthetic) plus an `info` message holding the binary parts. Rare.

### 5.3 thoughts (gemini only)

`thoughts?: Array<{ subject: string, description: string, timestamp: ISO }>`. `subject` is `""` when the model emitted no `**bold**` heading
(src `parseThought`). Queued per response and attached to the next gemini message. `thoughts: []` is common (not absent). Duplicated as `thought:true` parts in G2/G3 `content`.
No thought text is encrypted; `thoughtSignature` on parts is an opaque base64 blob (keep it for round trip, never display).

### 5.4 tokens (gemini only, `TokensSummary | null`)

```
{ input: number   // promptTokenCount, INCLUDES cached
  output: number  // candidatesTokenCount, EXCLUDES thoughts
  cached: number  // cachedContentTokenCount (cache read)
  thoughts?: number // thoughtsTokenCount
  tool?: number     // toolUsePromptTokenCount (built-in tool prompt tokens, e.g. google search / url context)
  total: number }   // totalTokenCount = input + output + thoughts + tool
```
`null` or absent when usage metadata was missing (synthetic turns, errors, cancelled). Absent `thoughts`/`tool` => treat as 0.
Per message = per API response (one request per model turn). There is no cost, no cache_write, no per-tool-call usage, no latency.

### 5.5 model

`model?: string` only on `gemini` messages, e.g. `gemini-2.5-pro`, `gemini-3-pro-preview`, `gemini-2.5-flash`. Undefined for synthetic turns and for messages recorded via
`recordSyntheticMessage`. The "session model(s)" is the ordered set of distinct values. Auto-routing means it can change mid-session, and subagents use their own.

### 5.6 ToolCallRecord (inside `gemini.toolCalls[]`)

| field | type | opt | notes |
|---|---|---|---|
| id | string | no | typically `<toolName>-<epochMs>-<rand>`; matches Part `functionCall.id` |
| name | string | no | original name the MODEL requested (`originalRequestName ?? name`), so aliases/tail calls show the model's view |
| args | object | no | original model args (`originalRequestArgs ?? args`) |
| result | `PartListUnion \| null` | yes | normally `[ {functionResponse:{id,name,response}} (, ...sibling inlineData/fileData parts) ]`. null/absent while pending or in odd paths |
| status | string | no | recorded at completion, so only `success`, `error`, `cancelled` occur in practice. Type allows `validating`, `scheduled`, `executing`, `awaiting_approval` |
| timestamp | ISO | no | completion time (set when `recordCompletedToolCalls` runs), NOT start time |
| agentId | string | yes | for subagent tools: the child session's id (see 8.1) |
| displayName | string | yes | `Shell`, `Edit`, `WriteFile`, `ReadFile`, `FindFiles`, `ReadFolder`, `SearchText`, `GoogleSearch`, `WebFetch`, `ReadManyFiles`, ... (added by `recordToolCalls` from the registry) |
| description | string | yes | for shell: the command itself if <= threshold chars else the `description` arg; for edits a short "file: old => new"; else tool description |
| resultDisplay | `ToolResultDisplay` | yes | UI form of the result (5.7) |
| renderOutputAsMarkdown | boolean | yes | UI hint |

Status mapping facts (src `shell.ts`, `tool-executor.ts`):
- `error`: tool threw, validation failed (path not in workspace), or tool returned `error` (file not found, edit mismatch). `result[0].functionResponse.response = {error: message}`. `resultDisplay` is a short string.
- `cancelled`: the run's abort signal fired during/after execution (user Esc/Ctrl+C), or a confirmation was rejected. `response.error = "[Operation Cancelled] <reason>"`, reason e.g. `User cancelled tool execution.` / `Operation cancelled.`; partial tool output is kept in `response.output` next to `error`. Rejection vs abort is distinguishable only by the reason text.
- `success` includes shell commands with non-zero exit codes AND shell commands killed by the inactivity timeout (the tool returns text "Command was automatically cancelled because it exceeded the timeout of X minutes without output." with no `error`, so the executor records success). Only a user abort yields `cancelled`.
- A shell that fails to spawn (`result.error`) is `error`: the response keeps ONLY `{error: message}`, partial output is lost.
- Parallel calls: several toolCalls on one gemini message, `timestamp`s may be identical.
- Calls can be missing a result when the process died; also upstream synthesizes interrupted responses (`buildInterruptedToolResponseParts`) as extra `user` turns.

### 5.7 resultDisplay variants (`ToolResultDisplay`)

```
string                                   // most tools, also shell stdout/stderr merged text
FileDiff   { fileDiff: unifiedPatch, fileName, filePath, originalContent: string|null, newContent, diffStat?, isNewFile?, isBuildFile? }   // replace, write_file
           diffStat { model_added_lines, model_removed_lines, model_added_chars, model_removed_chars, user_added_lines, user_removed_lines, user_added_chars, user_removed_chars }
AnsiOutput = AnsiToken[][]  token {text,bold,italic,underline,dim,inverse,isUninitialized,fg,bg}   // interactive shell (PTY) in live mode; usually a string when recorded
TodoList   { todos: [{ description, status: pending|in_progress|completed|cancelled|blocked }] }    // write_todos
GrepResult { summary, matches: [{filePath, absolutePath, lineNumber, line, isContext?}], payload? } // grep_search (newer)
ListDirectoryResult { summary, files: string[], payload? }                                         // list_directory (newer)
ReadManyFilesResult { summary, files[], skipped?[{path,reason}], include?, excludes?, targetDir?, payload? }
SubagentProgress { isSubagentProgress:true, agentName, recentActivity:[{id,type:thought|tool_call,content,displayName?,description?,args?,status}], state?: running|completed|error|cancelled, result?, terminateReason? }
```
Discriminate by keys: `fileDiff`, `todos`, `matches`, `files`, `isSubagentProgress`, array-of-array. For `newContent`/`originalContent` the file bodies are stored in full: big sessions are mostly this.

## 6. Tool catalog (names, args, result text)

Names changed over time; map all to the same typed input. Param renames: v0.12 -> v0.15 (`absolute_path`/`path`/`directory` became `file_path`/`dir_path`);
`search_file_content` -> `grep_search` in v0.28.0; read_file `offset`/`limit` -> `start_line`/`end_line` and replace `expected_replacements` -> `allow_multiple` between v0.30 and v0.35;
`write_todos_list` (v0.9) -> `write_todos`; MCP naming `<server>__<tool>` -> `mcp_<server>_<tool>` (v0.40+).

| canonical | name(s) seen | args | success result text (llmContent) | notes |
|---|---|---|---|---|
| Shell | `run_shell_command` | `command`, `description?`, `dir_path?` (old: `directory`), `is_background?`, `delay_ms?`, `additional_permissions?` | see section 7 | |
| Read | `read_file` | `file_path` (old `absolute_path`), `start_line?`/`end_line?` (old `offset?`/`limit?`) | file text; truncated: `IMPORTANT: The file content has been truncated.\nStatus: Showing lines A-B of N total lines.\n...\n--- FILE CONTENT (truncated) ---\n<text>` | binary/images come back as `inlineData` sibling parts or a "Binary content ... read successfully" line |
| ReadMany | `read_many_files` | `include[]`, `exclude?[]`, `recursive?`, `useDefaultExcludes?`, `file_filtering_options?` | concatenated `--- <path> ---` sections | resultDisplay ReadManyFilesResult in newer |
| Write | `write_file` | `file_path`, `content` | `Successfully created and wrote to new file: <abs>.` / `Successfully overwrote file: <abs>.` | resultDisplay FileDiff (or `Created <rel>` string for new) |
| Edit | `replace` | `file_path`, `instruction`, `old_string`, `new_string`, `allow_multiple?` (old `expected_replacements?`), `modified_by_user?`, `ai_proposed_content?` | `Successfully modified file: <abs> (N replacements).` (+ updated snippet in newer), new file: `Created new file: <abs> with provided content.` | errors: `Failed to edit, Expected 1 occurrence but found N for old_string in file: ...`, `Error preparing edit: ...`. `old_string:""` means create file |
| Glob | `glob` | `pattern`, `dir_path?` (old `path?`), `case_sensitive?`, `respect_git_ignore?`, `respect_gemini_ignore?` | `Found N file(s) matching "<pattern>" ...` + file list; none: `No files found...` | |
| Search | `grep_search` (old `search_file_content`), `ripGrep` variant uses same name | `pattern`, `dir_path?`, `include_pattern?` (old `include`), `exclude_pattern?`, `names_only?`, `max_matches_per_file?`, `total_max_matches?`, rg-only: `case_sensitive?`, `fixed_strings?`, `context?`, `after?`, `before?`, `no_ignore?` | `Found N match(es) for pattern "<p>" <location> [(filter: "<glob>")]`, then `\n---\n`, then per-file blocks (`File: <rel>`, `L<line>: <text>`); names_only: `Found N files with matches for pattern ...`; none: `No matches found for pattern ...` | structured GrepResult in resultDisplay (newer) |
| List | `list_directory` | `dir_path` (old `path`), `ignore?[]`, `file_filtering_options?` | `Directory listing for <abs>:\n[DIR] name\nfile` ; empty: `Directory <abs> is empty.` | |
| WebFetch | `web_fetch` | `prompt` (URL(s) embedded in the prompt text, up to 20); experimental `url?` | model-summarized text, wrapped in `<untrusted_context>` in v0.49+ | URLs must be regex-extracted from `prompt` |
| WebSearch | `google_web_search` | `query` | `Web search results for "<q>":\n\n<answer with [1] citation markers>\n\nSources:\n[1] title (url)` | `sources` are in the tool result object, NOT persisted; only text |
| Memory | `save_memory` | `fact` | `Okay, I've remembered that: "<fact>"` | removed from current main (replaced by memory service/GEMINI.md flow), still present in old sessions |
| Todo | `write_todos` (v0.9: `write_todos_list`) | `todos:[{description,status}]` | `Successfully updated the todo list. The current list is now:\n1. [status] desc ...` | resultDisplay `{todos}` is the structured form |
| Docs/skills | `get_internal_docs{path?}`, `activate_skill{name}` | | skill body text | |
| Ask | `ask_user{questions:[{question,header,type,options?,multiSelect?,placeholder?}]}` | | answers text | |
| Plan | `enter_plan_mode{reason?}`, `exit_plan_mode{plan_filename}` | | | |
| Topic/Task | `update_topic{title,summary,strategic_intent}`, `complete_task` | | | subagent-internal mostly |
| Tracker | `tracker_create_task`, `tracker_update_task`, `tracker_get_task`, `tracker_list_tasks`, `tracker_add_dependency`, `tracker_visualize` | | | |
| Subagent | `invoke_agent` (v0.40+; before: tool named after the agent, e.g. `codebase_investigator`, `cli_help`, `generalist`) | `agent_name?`/`prompt` style inputs, free form | final agent answer text | has `agentId`, resultDisplay SubagentProgress |
| MCP | `mcp_<server>_<tool>` (v0.40+), earlier raw tool name or `<server>__<tool>` | arbitrary JSON | text of MCP content blocks; images as `inlineData` siblings | `read_mcp_resource{uri}`, `list_mcp_resources{}` are built-ins |

Only `args` + `name` are authoritative for typing. Never trust `displayName` for typing (it is a UI label, absent in some versions).

## 7. Shell in detail

Args (typed): `command: string`, `description?: string`, `dir_path?: string` (cwd override; old `directory`), `is_background?: boolean`, `delay_ms?: number`,
`additional_permissions?` (sandbox expansion request). There is NO `timeout` arg; the timeout is a global inactivity timer (`getShellToolInactivityTimeout`).
Executed as `bash -c` wrapped to capture background pids (`_bgpids_file=...` wrapper is NOT stored; `args.command` is the user command).

`result[0].functionResponse.response.output` text, by era:

- v0.9 to v0.27 (G1 era):
  ```
  Command: <cmd>
  Directory: <dir or (root)>
  Stdout: <text or (empty)>
  Stderr: <text or (empty)>
  Error: <msg or (none)>
  Exit Code: <n or (none)>
  Signal: <n or (none)>
  Background PIDs: <list or (none)>
  Process Group PGID: <n or (none)>
  ```
  (`Stdout:`/`Stderr:` present up to v0.26.0; `(none)` placeholders always present.)
- v0.28 to v0.48 (PTY era, stdout+stderr merged):
  ```
  Output: <combined text or (empty)>
  [Error: <msg>]            only if the spawn/exec reported an error
  [Exit Code: <n>]          only if non-zero
  [Signal: <name>]          only if killed by signal
  [Background PIDs: 1, 2]   only if any
  [Process Group PGID: <n>] when known
  ```
- v0.49+ : same body wrapped as `<untrusted_context>\n...\n</untrusted_context>` (closing tag inside output is escaped `&lt;/untrusted_context&gt;`).
- Background (`is_background:true` or Ctrl+B): `Command moved to background (PID: <n>). Output hidden. Press Ctrl+B to view.` (the `data.pid` etc. is NOT persisted).
- User abort: `Command was cancelled by user before it could complete. Below is the output before it was cancelled:\n<out>` or `... There was no output before it was cancelled.` with `status:"cancelled"` and `response.error`.
  Inactivity timeout: `Command was automatically cancelled because it exceeded the timeout of X.Y minutes without output.` (+ the same output suffix) with `status:"success"` and NO `error`.
- Output over the truncate threshold (default `DEFAULT_TRUNCATE_TOOL_OUTPUT_THRESHOLD = 40_000` chars, shell only): body becomes
  `Output too large. Showing first <20%> and last <80%> characters. For full output see: <abs path in tmp/<slug>/tool-outputs/session-<id>/...>\n<head>\n\n... [N characters omitted] ...\n\n<tail>`.
  The saved file is a full-fidelity artifact outside the session file; record its path, do not try to read it as part of the session.
- Context masking (v0.3x+): old tool outputs are later replaced via `$patch`/`$set` with a `<tool_output_masked>` snippet. After reduction the stored `result` may be a masked stub; the original is gone (unless an earlier line of the file had it; see 8.2).

Parsing rules for `ShellOutput`:
- `exit_code`: regex `^Exit Code: (-?\d+)$` (multiline); absent + `status:"success"` => 0 (modern); `Exit Code: (none)` => unknown/None (old).
- `signal`: `^Signal: (.+)$`, ignore `(none)`.
- `stdout`/`stderr`: separate only in v0.9..v0.26 text; merged `Output:` afterwards => put into `stdout`/`output` and mark `stderr: None`.
- `interrupted`: `status == cancelled`, or text starts `Command was cancelled by user`. The timeout text (`Command was automatically cancelled`) is a distinct `timed_out` outcome that still has `status:"success"`.
- `truncated`: text begins `Output too large. Showing first`; capture `full_output_path`.
- `duration`: not stored. Only per-toolCall `timestamp` (completion) exists; start time is unknowable except as the previous record's timestamp, which is unreliable.
- `resultDisplay` for shell is the visible output string (or `Command exited with code: N` when output empty), occasionally `AnsiOutput`.
- Background pids: `^Background PIDs: ([\d, ]+)$`; process group: `^Process Group PGID: (\d+)$`.

## 8. Special structures

### 8.1 Subagents
- Subagent turns are NOT in the parent file. Each subagent run writes `chats/<parentSessionId>/<agentId>.jsonl` (header `kind:"subagent"`, `sessionId == agentId`, `directories` = workspace dirs; no `session-` prefix, no timestamp in name) via the same recording service.
- Parent link: directory name `<parentSessionId>` (full id, sanitized) and, on the parent side, `ToolCallRecord.agentId == child sessionId` on the `invoke_agent`/agent-named tool call. The child header has no explicit `parentSessionId` field.
- Child messages use the same grammar (`user` prompt, `gemini` with toolCalls, ...). Child tokens are separate from the parent's: summing both is the real usage.
- Upstream UI hides `kind:"subagent"` files from `/resume`. A scanner that only globs `chats/session-*.jsonl` misses subagents entirely; one that globs `chats/**/*.jsonl` must use `kind`.
- `summary` may also be saved on subagent sessions.

### 8.2 Compaction (`/compress`, auto-compress), rewind, masking
- Compression result text is recorded as an `info` message ONLY via the UI path (`compression` history item). Its `content` is `itemData.text ?? ''`, so it is often EMPTY. Token counts before/after are NOT reliably stored.
- The compression itself is visible as history surgery: a synthetic `user` message whose text contains `<state_snapshot>...` (the model-written summary), followed by a synthetic `gemini` message `Got it. Thanks for the additional context!`, then the retained tail (truncated). In G2 the `$set.messages` snapshot after it drops all earlier messages; in G3 `$patch.removeIds` does.
  So `Compaction{summary}` = the text of the user message containing `<state_snapshot>`; `trigger` unknown (manual vs auto not recorded); `tokens_before/after` absent unless parsed from a non-empty `info` text of the form `Chat history compressed from X to Y tokens.`
- To keep the pre-compaction transcript, a reader must treat the file as an event log: remember every message id when first seen and keep it even if a later `$set.messages`, `$patch.removeIds` or `$rewindTo` drops it, marking it `superseded_by_compaction` / `rewound`. Reduced state alone loses it. Recommend offering both views (`Reduced` = what Gemini would resume with, `Full` = every id ever seen, in first-seen order).
- `$rewindTo` comes from the `/rewind` feature and from tail rollbacks (e.g. all tool calls of a turn declined): messages from that id onward are dropped from the live state.
- Tool output masking rewrites `toolCalls[].result` (and the echoed user functionResponse content) to `<tool_output_masked>` stubs.

### 8.3 Errors, interrupts, hooks, system notices
- `type:"error"`/`"warning"`/`"info"` messages are the only notice channel (`SystemNotice`/`Error`). Content is a UI string like `[API Error: ...]`; no error code, retry count or http status field.
- Interrupt: no explicit record. Visible as (a) a tool with `status:"cancelled"`, (b) synthetic user turn(s) of `functionResponse` parts with `response.error` (interrupted placeholder text, see `INTERRUPTED_RESPONSE_PLACEHOLDER`) and a synthetic model text turn, (c) a `gemini` message with no tokens.
- Hooks: only `<hook_context>` user turns (additional context injected by hooks). Hook execution events are not recorded.
- Model change: no record; derive from `model` changes between `gemini` messages.
- Attachments: `@file` references expand into user text parts (`--- Content from referenced files ---` blocks) with `displayContent` holding the short form; pasted images are `inlineData` parts.
- Slash commands typed by the user appear as `user` messages starting with `/` (kept in log, ignored by resume).

### 8.4 Ordering and pairing rules for a reader
1. Reduce (section 4) to an ordered message list.
2. For each `gemini` message build parts in this priority: if `content` is `Part[]` and contains `functionCall` or `thought:true` parts, it is the source of truth (modern; order is real interleaving); else legacy: `thoughts[]` as reasoning, then `content` text, then `toolCalls[]` as calls (interleaving lost).
3. Pair results: for each call id, prefer the `toolCalls[].result` (latest after patches); else look for a later `user` message with `functionResponse.id == id`. A `user` message whose parts are all `functionResponse` is a tool-result carrier, NOT a user prompt.
4. A `user` message that mixes functionResponse parts and text/inlineData parts (binary injection) is a tool carrier plus extra attachments.
5. Skip `user` messages that are `<session_context>`; surface them as `SystemNotice`/context entry (they carry OS, date, workspace dirs, directory tree, and the loaded GEMINI.md/AGENTS.md text).
6. Message `timestamp` of a `gemini` message is when the turn was first recorded (start of the response), tool `timestamp` is completion; sort by file order, not by timestamp.

## 9. Existing parser: ~/coding/qqoro/qoop-tui/crates/qoop-history/src/gemini.rs

What it does: `serde_json::from_str` of the whole file, requires `sessionId` and `messages[]`; keeps `type` `user`/`gemini` only; text = string or join of `[{text}]` with `\n`;
drops empty text; ts from `timestamp`. `dir.rs` explicitly skips Gemini ("newer format is not one this crate parses"). `locate.rs::locate_gemini` matches names ending `-<id8>.json` and confirms by parsing the whole file.

Consequences and gaps:
1. G2/G3 `.jsonl` files are invisible: locate looks for `.json` suffix only, and a multi-line JSONL fails whole-file JSON parse, so `parse_file` returns `None` (counted as skipped). Every session since v0.39.0 is unreadable.
2. Even for G1/G2 content: `flatten_content` joins ALL `text` parts, so a G2/G3 `gemini` message with `thought:true` parts leaks reasoning into assistant text. Must skip `thought == true`.
3. User turns consisting only of `functionResponse` produce empty text and are dropped (accidentally right); `<session_context>` user turns are emitted as real user messages (wrong, they are context).
4. Dropped entirely: tool calls and results, thoughts, tokens, model, `info/error/warning`, `displayContent`, `summary`, `kind`, `directories`, subagent files, a `gemini` message that has only toolCalls (empty text => skipped).
5. No `$patch/$rewindTo/$set.messages` semantics, no duplicate-id upsert (a duplicate-id line would yield duplicate messages).
6. `load_gemini` refuses files above `TAIL_WINDOW_BYTES` (8 MiB) with `TooLarge`. G2 files grow quadratically (full snapshot each turn), so real sessions exceed this quickly; stream-parse lines instead and keep only the reduction.
7. Dir assumption `tmp/<hash>` is outdated; locate walks the tree so it still works, but cwd/repo attribution (`dir.rs` matching a repo) needs `.project_root`.

## 10. Mapping to the proposed common model (critique)

Session
- `agent=Gemini`, `id=sessionId`, `source path`, `started_at=startTime`, `updated_at=lastUpdated` (header may lag; max of message timestamps is more accurate; G2 header is never rewritten, `lastUpdated` arrives via `$set`).
- `cwd`: from `.project_root`, else `projects.json`, else sha256 reverse match, else `<session_context>`. Mark provenance (Derived vs Recorded). `git_branch`: never recorded (None). `cli_version`: never recorded; expose `format_generation: G1|G2|G3` and `file_ext` instead, so users can reason about it.
- `title = summary` (optional) else first non-context user text.
- `model(s)`: ordered distinct `gemini.model`.
- parent session link: derive from path `chats/<parent>/<agent>.jsonl` + `header.kind == "subagent"`; also parent-side `toolCall.agentId`. Provide both directions.
- Extra session-level data worth a field: `directories`, `memory_scratchpad`, `project_hash`, `kind`.

Entry
- No `parent_id` in Gemini. History is a linear list. Set `parent_id = previous entry id` implicitly or leave None; do not invent a tree. Tool results should hang off their call (`ToolCall.output`), not be separate entries.
- One Gemini `gemini` record maps to ONE assistant `Message` with ordered `parts`: `Reasoning` (thoughts or thought parts), `Text`, `ToolCall(with paired output)`. Do NOT split into several entries per part.
- Gemini `user`-typed tool-response carriers must not become `Message{role:user}`. The proposed `role: user|assistant|system|developer` has no `tool` role; either add `Role::Tool` for orphan carriers or fold into the call and drop. Recommended: fold, and emit an orphan `Part::ToolResult{call_id, output}` only when no call matches.
- Events: `info` => `SystemNotice{level:info}`, `warning` => `SystemNotice{level:warn}`, `error` => `Error`. `<session_context>` => `SystemNotice{kind:context}` (or `Attachment`). `<hook_context>` => `Hook`. Compaction => see 8.2 (`Compaction{summary}` from `<state_snapshot>` user message; `trigger: Unknown`, tokens usually None). `$rewindTo`/removeIds => needs an event `Rewind{to_id, dropped_ids}` or the Full view flag; the proposed model has no rewind/branch event: ADD ONE.
- `Interrupt`: derive from cancelled tool calls and the interrupted placeholder turns; low confidence.
- `ModelChange`: derive when `model` differs from the previous gemini message.
- Usage: map `cache_read = tokens.cached`, `input = tokens.input - tokens.cached` if the common model defines `input` as uncached (document which!), `output = tokens.output`, `reasoning = tokens.thoughts`, `cache_write = None`, `cost = None`. EXTRA: `tool_prompt = tokens.tool` and `total` have no home; add `extra: Map<String,u64>` or explicit `tool_use_prompt` and keep `total` as a reported value to check sums. Gemini `output` EXCLUDES thoughts (unlike OpenAI-style output_tokens that include reasoning): normalize and document.
- Part types: `Text`, `Reasoning{text, signature: thoughtSignature}` (no encrypted blob distinct from signature), `Image` = `inlineData` image/*, `File` = `inlineData` other mime or `fileData`, `ToolCall`. Missing: `functionResponse.parts` nested multimodal results, `executableCode/codeExecutionResult` (not produced by the CLI; ignore).
- Message-level `displayContent` (what the user saw vs what was sent) has no home: add `display_text: Option<..>` on Message.

ToolCall
- Typed input map: Shell{command, cwd=dir_path, background=is_background, description, timeout=None}; Read{path, start_line, end_line} (old `offset/limit` and new `start_line/end_line` differ in semantics: keep the raw values and a `legacy_offset_limit` flag instead of converting); Write{path, content}; Edit{path, old, new, instruction, replace_all=allow_multiple, expected_replacements}; Search{pattern, dir, include, exclude, names_only, context...}; Glob{pattern, dir}; List{dir, ignore}; WebFetch{prompt, urls extracted}; WebSearch{query}; Todo{items}; Memory{fact}; Mcp{server, tool} parsed from `mcp_<server>_<tool>` (server names containing `_` are ambiguous, regex splits at first `_`; keep `raw_name`); Subagent{agent_name, agent_id}; AskUser; Plan; Skill{name}; Other.
- Output pairing: `ShellOutput{stdout|output, stderr: Option, exit_code: Option, signal, duration: None, interrupted, truncated, full_output_path, background_pids, pgid}` parsed from text (section 7). Structured outputs come from `resultDisplay`: `Diff{patch, original, new, stat, is_new_file}` for Edit/Write (ADD this variant, the proposed list only has ShellOutput/text/structured), `Todo`, `Search matches`, `Listing files`. Keep BOTH `llm_text` (what the model saw) and `display` (what the UI showed): they differ (shell, edit).
- Status: `success -> ok`, `error -> error`, `cancelled -> interrupted` (reasons `User cancelled tool execution.` / `Operation cancelled.`) or `rejected` (reason text about denial; not observed locally, verify on real data). Pending/awaiting are effectively never on disk; map `pending` when `result` is null.
- Shell non-zero exit is `ok` status with `exit_code != 0`; provide a derived `is_error` that ORs status error and exit_code != 0.
- Fields with no home: `displayName`, `description` (UI string), `renderOutputAsMarkdown`, `agentId` (use for Subagent link), per-call `timestamp` = end time. Add `ended_at` and leave `started_at` None.

Missing from the proposed model for Gemini: Rewind event, patch/supersede provenance ("full vs reduced" view), `display_text`, Diff output variant, per-message `tool_prompt` tokens, session `memory_scratchpad`, `format_generation`, subagent `agent_id` both directions, `project_hash`, a notion of synthetic messages (flag `synthetic: bool` for context, tool carriers, compaction summary, binary-injection ack).

## 11. Edge cases checklist (each should get a test)

1. Duplicate-id message lines: last wins, first position kept. G2/G3.
2. `$set.messages` replaces; messages absent from the snapshot vanish from reduced state (Full view keeps them).
3. `$patch` for unknown id is ignored; `$patch.toolCalls[].result: null`.
4. `$rewindTo` unknown id clears all messages.
5. G1 `.json` and a migrated `.jsonl` for the same sessionId.
6. `content` as string vs `[{text}]` vs mixed array vs single object.
7. `gemini.content == ""` with toolCalls only (very common).
8. `tokens: null`; `thoughts: []`; missing `thoughts/tool` in tokens.
9. Tool call with `result` null / missing; result as plain string; result with `error` AND `output` (cancelled).
10. Shell: exit 0 with no `Exit Code` line; `Exit Code: (none)` old; combined `Output:` vs `Stdout:/Stderr:`; truncated; background; cancelled; wrapped in `<untrusted_context>`.
11. Masked results (`<tool_output_masked>`).
12. Subagent file under `chats/<parent>/`, header `kind:"subagent"`.
13. `<session_context>` first message with deterministic id shared across sessions.
14. Trailing truncated JSON line; invalid UTF-8; CRLF in `fileDiff`.
15. Header missing `kind`; header missing `sessionId` (file not a session).
16. Huge `originalContent/newContent` in `FileDiff`: stream, do not hold twice.
17. Same sessionId resumed in another file name (resume reuses the original file; but G1->G2 migration creates a second file).

## 12. Fixtures (synthetic, upstream-faithful; all paths/prompts are placeholders)

- `~/coding/agentparse/plans/gemini-fixtures/gemini-old-session.json`: G1. user text, gemini text with thoughts+tokens+model, shell with exit code 1 (`Exit Code: 1`), shell success, `replace` edit with `FileDiff` resultDisplay, `read_file` error tool call, info and error messages, `summary`. Era: v0.30 to v0.38 (`Output:` shell text, `expected_replacements`).
- `~/coding/agentparse/plans/gemini-fixtures/gemini-new-session.jsonl`: G3 (v0.64+). header, `<session_context>` turn, duplicate-id upsert (g-1 appended twice), tool-response user carriers, `$patch` content update to `Part[]` with thought part and `functionCall`, `$patch` masking a tool result, edit with FileDiff, error tool call, cancelled shell, error message, `$set summary`, `$rewindTo` dropping `u-2` and `g-4`. Expected reduced ids: `ctx..001,u-1,g-1,u-tr-1,g-2,u-tr-2,g-3,e-1`; summary `Fix failing parser test`. (Verified with a 20-line reducer written from the upstream loader.)
- `~/coding/agentparse/plans/gemini-fixtures/gemini-snapshot-session.jsonl`: G2 (v0.39 to v0.63), mirrors the real local files plus an assistant turn: header, `$set.messages` single-element snapshot, bare message appends, final `$set.messages` full replacement where g-1 content has become `Part[]` with `functionCall` and a `user` functionResponse carrier is present. Expected reduced ids: `ctx..001,u-1,g-1,u-tr-1`.

## 13. Sources (upstream, main unless noted)

- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingTypes.ts (all record types: ConversationRecord, MessageRecord, ToolCallRecord, TokensSummary, RewindRecord, MessagePatchRecord, MetadataUpdateRecord)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingService.ts (loader/reducer `createJsonlRecordAccumulator`, writers, file naming, subagent dir, legacy fallback)
- https://github.com/google-gemini/gemini-cli/blob/v0.63.0/packages/core/src/services/chatRecordingService.ts (G2 writer: `updateMessagesFromHistory` writes `$set:{messages}`)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/core/geminiChat.ts (what is recorded: user/tool-response/synthetic turns, `recordCompletedToolCalls`, thoughts/tokens flush)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/sessionUtils.ts (`convertSessionToClientHistory`, `isIgnoredUserContent`, synthetic ids)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/config/storage.ts and projectRegistry.ts (slug dirs, `.project_root`, `projects.json`)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/tools/shell.ts, tools.ts (ToolResultDisplay), tool-names.ts, definitions/base-declarations.ts, edit.ts, write-file.ts, read-file.ts, web-fetch.ts, mcp-tool.ts
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/scheduler/tool-executor.ts (`convertToFunctionResponse`, error/cancel response shapes, truncation) and scheduler/types.ts (status enum)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/context/chatCompressionService.ts and packages/cli/src/ui/hooks/useHistoryManager.ts (compression + info/error/warning recording)
- https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/agents/local-executor.ts and local-invocation.ts (subagent id, `data.agentId`, `chat.initialize(undefined,'subagent')`)
- PRs by date: #5221 (2025-08-19 service), #6721 (2025-09-03 wired into chat, in v0.4.0), #14467 (summary), #17454 (directories), #17901 (slug temp dirs, in v0.29.0), #23903 (subagent kind, in v0.37.0), #23749 (JSONL, v0.39.0), #25873 (memoryScratchpad, v0.41.0), #29568 (append-only patches, v0.64.0-preview.0).
- Local evidence: `~/.gemini/tmp/qass-1/chats/session-2026-10-10T21-18-3c69a60b.jsonl`, `~/.gemini/tmp/slashtest/chats/session-2026-10-02T22-46-84c6ca20.jsonl`, `~/.gemini/projects.json`, `~/.gemini/tmp/*/.project_root`.

Unverified/open: exact CLI version that first wrote `displayContent` and `summary` (dates known, tags not pinned); whether `thoughts[].subject` was ever absent (treat as optional); behavior of the not-yet-released `ripGrep` result text; whether any non-interactive (`-p`) runs record `info/error` (the UI hook path is interactive only, so likely not).
