# Claude Code session format: measured research

Source of every number: a full streaming scan of `~/.claude/projects` on the owner's Mac
(scanner scripts and raw `stats*.json` live in the research scratchpad, not in this repo).
Scan date 2026-10-13. Corpus: 17,826 `.jsonl` files, about 11 GB, 2,398,152 JSON lines, 0 corrupt lines,
0 blank lines, 0 non-object lines. CLI versions present: 2.1.143 (2026-05-17) to 2.1.296 (2026-10-13), 27 distinct versions.
Everything older than 2.1.143 (the `summary` line, `progress` lines, `Task`/`TodoWrite`/`MultiEdit`/`LS`/`Glob`
tool names) is NOT in this corpus; where I mention it, it is marked "unverified, from memory".

## 1. File layout

| Kind | Path | Files | Lines | Notes |
|---|---|---|---|---|
| main session | `projects/<escaped-cwd>/<sessionId>.jsonl` | 1,341 | 1,433,101 with sessionId | file stem always equals `sessionId` of every line (1,433,101 of 1,433,101) |
| subagent | `projects/<escaped-cwd>/<sessionId>/subagents/agent-<agentId>.jsonl` | 7,046 | | every line `isSidechain:true`, `sessionId` = PARENT session id (932,619 of 932,619), `agentId` = file stem after `agent-` (7,048 of 7,049) |
| subagent meta | `.../subagents/agent-<agentId>.meta.json` | about 7,049 | | JSON, keys below |
| workflow subagent | `.../subagents/workflows/<wf_id>/agent-<name>-<hex>.jsonl` (+ `.meta.json`) | 9,439 | | same line shape as a subagent; name part is optional in the file name |
| workflow journal | `.../subagents/workflows/<wf_id>/journal.jsonl` | 333 | 17,722 | only `started` / `result` lines (section 7) |
| workflow run summary | `<sessionId>/workflows/<wf_id>.json`, `workflows/scripts/*.js` | | | not JSONL; keys `runId, workflowName, status, summary, result, agentCount, totalTokens, totalToolCalls, durationMs, phases, logs, script` |
| spilled tool output | `<sessionId>/tool-results/<taskId>.txt` (also `.json`, `.jpg`, `.pdf`, `pdf-<uuid>/` dirs) | 1,492 txt, 508 jpg, 219 pdf | | referenced by `<persisted-output>` text and `toolUseResult.persistedOutputPath` |
| file history | `~/.claude/file-history/<sessionId>/<hash>@v<N>` | | | blobs referenced by `file-history-delta.backup.backupFileName` |
| legacy index | `projects/<dir>/sessions-index.json` | 5 | | stale (March 2026), do not rely on it |
| desktop marker | `<sessionId>.desktop-released.json` | 1 | | `{v, releasedAt, reason}` |

Directory name escaping: absolute cwd with every `/` and `.` replaced by `-` (so `/a/.claude/wt` becomes `-a--claude-wt`). Lossy and not reversible;
take the real cwd from the `cwd` key on the lines.

Line sizes (bytes): 94.6% under 10 KB, 4.9% 10 to 100 KB, 0.4% 100 KB to 1 MB, 675 lines 1 to 10 MB, none over 10 MB; largest file has a 6 MB line.
Causes: inline base64 images/PDFs (`source.data`), `Read` of big files (`toolUseResult.file.content` AND `message.content` duplicate the text),
`Edit.originalFile`, `Agent` prompts. A parser must not use fixed small line buffers.

Subagent `.meta.json` keys (all optional, measured on 400 sampled files): `agentType`, `description` (always), `toolUseId` (75%), `spawnDepth`,
`parentAgentId` (nested subagent, file still lives flat in the same `subagents/` dir), `requestShape`, `requestNonInteractive`, `model`, and for team members
`name, color, teamName, permissionMode, planModeRequired, taskKind, customAgentType`.

## 2. Top-level line types (measured)

Counts by `type` (all files). "main" = main session files, "sub" = subagent + workflow subagent files.

| `type` | Count | Where | Role |
|---|---|---|---|
| `assistant` | 988,936 | main 498,562; sub 490,374 | one line per content block of an API message (section 4) |
| `user` | 581,142 | main 265,317; sub 315,825 | human prompt, tool_result carrier, meta/injected text, compact summary |
| `attachment` | 339,274 | main 217,633; sub 121,641 | harness-injected context, hook results, queued input (53 attachment subtypes, section 8) |
| `system` | 40,726 | main 40,708; sub 18 | 12 subtypes (section 6) |
| `last-prompt` | 73,228 | main | `{lastPrompt, leafUuid, sessionId}` |
| `mode` | 70,147 | main | `{mode:"normal"}` always |
| `ai-title` | 67,884 | main | `{aiTitle}`, re-emitted many times, latest wins |
| `permission-mode` | 67,344 | main | `{permissionMode}`: bypassPermissions 65,117, auto 787, acceptEdits 597, plan 568, dontAsk 264, default 11 |
| `queue-operation` | 39,361 | main | typed-while-busy input (section 5) |
| `atis-latch` | 31,863 | main | `{atis:""}` internal, ignore |
| `pr-link` | 26,695 | main | `{prNumber, prUrl, prRepository, timestamp}` |
| `file-history-snapshot` | 19,770 | main | `{messageId, snapshot:{messageId, trackedFileBackups:{<path>:{...}}, timestamp}, isSnapshotUpdate}` (dynamic keys = file paths) |
| `agent-name` | 11,197 | main | `{agentName}` |
| `started` / `result` | 9,106 / 8,616 | workflow journals | section 7 |
| `bridge-session` | 5,524 | main | `{bridgeSessionId, lastSequenceNum, ownerAccountUuid?, ownerOrganizationUuid?}` |
| `custom-title` | 3,848 | main | `{customTitle}` user set via /rename |
| `file-history-delta` | 3,726 | main | `{messageId, snapshotMessageId, trackingPath, backup:{backupFileName|null, version, backupTime, realParentDir?}, timestamp}` |
| `worktree-state` | 3,122 | main | `{worktreeSession: null | {originalCwd, worktreePath, worktreeName, worktreeBranch, originalBranch, originalHeadCommit, sessionId, preEnterOriginalCwd?, enteredExisting?}}` |
| `relocated` | 2,447 | main | `{sessionId, relocatedCwd}` |
| `cost-state` | 1,616 | main | session cost snapshot (section 9) |
| `agent-setting` | 1,402 | main | `{agentSetting}` (agent type this session runs as) |
| `frame-link` | 979 | main | `{path, frameUrl?, title?, artifactCount, timestamp}` (artifacts) |
| `artifact-autoreact-ledger` / `artifact-comment-monitor` | 165 / 22 | main | artifact bookkeeping, ignore |
| `fork-context-ref` | 7 | sub | `{agentId, parentSessionId, parentLastUuid, contextLength}` |
| `continued-in` | 5 | main | `{timestamp, sessionId, continuedInSessionId}` |

Not seen at all: `summary`, `progress`, `result` outside workflow journals, `custom-title` variants other than above.

Key facts:
- 16% of all lines (378,478) have NO `timestamp`: `last-prompt, mode, ai-title, permission-mode, atis-latch, file-history-snapshot, agent-name, started, result, bridge-session, custom-title, worktree-state, relocated, cost-state, agent-setting, artifact-*, fork-context-ref`. They also have no `uuid` / `parentUuid`. (`queue-operation, pr-link, frame-link, file-history-delta, continued-in` do have a timestamp but no uuid.)
- Only `user`, `assistant`, `system`, `attachment` carry the conversation envelope.
- Lines of ONE session are never mixed across sessionIds inside a file (17,492 files with 1 sessionId, 334 with 0 because they are metadata-only).

### 2.1 Envelope keys on conversation lines (presence %)

| Key | assistant | user | attachment | system | Notes |
|---|---|---|---|---|---|
| `type, uuid, parentUuid, timestamp, sessionId, version, cwd, gitBranch, isSidechain, userType, entrypoint` | 100% | 100% | 100% | 100% | `parentUuid` null on roots (17,934 roots total) |
| `userType` | `external` always | | | | |
| `entrypoint` | cli 987,484; sdk-cli 1,429; sdk-ts 12; claude-desktop 11 | | | | |
| `slug` | 41% | 42% | 36% | 40% | random 3-word name (`word-word-word`), absent in 81% of lines since 2.1.28x; do not depend |
| `agentId` | 50% | 54% | 36% | rare | present iff `isSidechain:true` (492,621 of 492,621) |
| `requestId` | 99.9% | | | | Anthropic `req_...`; missing on `<synthetic>` and a few others |
| `promptId` | | 98% | | | groups all lines caused by one human prompt (null on first attachment lines) |
| `sourceToolAssistantUUID` | | 91% | | | on tool_result lines: uuid of the assistant LINE that holds the tool_use (equals `parentUuid`) |
| `toolUseResult` | | 71% (413,546) | | | structured result (section 3) |
| `isMeta` | | 1.3% (true only) | | false/true | injected, not typed by a human |
| `isCompactSummary`, `isVisibleInTranscriptOnly` | | 354 each | | | section 6 |
| `origin` | | 21,390 | | | `{kind: human 11,934 | task-notification 8,725 | coordinator 527 | peer 194 | auto-continuation 10, producer?, runId?, from?, name?, senderTaskId?, body?, handback?, msg_id?}` |
| `promptSource` | | 21,425 | | | typed 12,347; system 8,361; queued 590; sdk 117; suggestion_accepted 10 |
| `permissionMode` | | 4% | | | `bypassPermissions 22,101, auto 324, acceptEdits 314, dontAsk 177, plan 106, default 24` |
| `permissionDecision` | | 19,592 | | | `{decision: accept|reject, source: config|user_permanent|user_temporary|user_reject, reasonType?: mode|classifier|rule|subcommandResults|safetyCheck}` since 2.1.295 |
| `toolDenialKind` | | 1,064 | | | `permission-rule 846, user-rejected 142, automode-parsing-error 29, automode-blocked 25, interrupted 19, automode-unavailable 2, cancelled 1` |
| `interruptedMessageId` | | 400 | | | `msg_...` of the assistant message that was interrupted |
| `imagePasteIds` | | 670 | | | list of ints for `[Image #N]` placeholders |
| `mcpMeta` | | 1,134 | | | `{structuredContent}` or `{_meta}` from an MCP result |
| `sourceToolUseID` | | 1,053 | | | on `isMeta` skill text lines: the `Skill` tool_use id that loaded it |
| `turnOrigin` | | 6,292 | | | `task_notification 3,245; human 3,015; sdk 41; peer 40; scheduled 30` |
| `turnPosition` | | 5,739 | | | `{promptIndex, turnIndex}` |
| `toolEndsTurn`, `turnCompanion`, `classifierBoundary`, `queuePriority`, `queueSkipAttachments`, `queueTranscriptOnly`, `queueOrigin`, `scheduledTaskId`, `scheduledFireId`, `serverClassifierContext`, `userFeedback` | | rare (under 4k each) | | | internal, keep in a raw/extra map |
| `sessionKind` | `bg` (19,899) | `bg` (10,686) | | | background session marker |
| `teamName`, `agentName` | 9,030 | 4,938 | 2,877 | 307 | agent-team members (2.1.181 to 2.1.206) |
| `session_id` (snake case duplicate of sessionId) | 37% | 31% | 50% | | appears from 2.1.201 |
| `effort` | 52% | | | | `high 516,659; medium 2,409; xhigh 1,079; low 20` (2.1.216+) |
| `perTurnEffort` | 22% | | | | same enum or null |
| `thinkingDisplay`, `thinkingDurationMs` | 1% | | | | `updates 10,984; omitted 23` (2.1.295+) |
| `requestedModel`, `advisorModel` | 3%, 14% | | | | requested vs served model; advisor model id |
| `apiBlockIndex` | 27% | | | | index of this block inside the API message (2.1.260+) |
| `isApiErrorMessage`, `error`, `apiErrorStatus`, `errorDetails` | 1,496 / 1,326 / 727 / 2 | | | | synthetic error messages (section 6) |
| `isAbortedMidStream` (19), `truncatedAfterOutput` (25), `supersedesUuids` (3), `healsDistinctCarrier` (1) | | | | | stream abort / retry markers |
| `quotaLimits` | 237 | | | | `{status, resetsAt (epoch s), rateLimitType: five_hour..., overageStatus, overageDisabledReason, isUsingOverage, unifiedRateLimitFallbackAvailable}` |
| `wireToolInputs`, `wireIngestContext` | 12%, 1% | | | | maps `tool_use_id => {input as sent}` / `tool_use_id => {cwd}`; echo, prefer `message.content[].input` |
| `attributionAgent` (38%), `attributionSkill` (7%), `attributionMcpServer/Tool` (5%), `attributionPlugin` (1%) | | | | | which agent/skill/MCP server was in scope; NOT reliably the tool of this line (a `navigate` call carried `attributionMcpTool: tabs_context_mcp`) |
| `rendered`, `renderedRole`, `renderedInHumanTurn`, `renderedBesideToolResult` | | | 42% / 6% / 1% | | exact text blocks the harness showed the model for this attachment (2.1.260+) |

## 3. Message content blocks

`message.content` on assistant lines is always a list (988,936). On user lines it is a string (48,843) or a list (532,299).

| Block | Count | Where | Shape |
|---|---|---|---|
| `text` | 209,558 asst; 3,615 user | | `{type, text}` |
| `thinking` | 251,864 | asst | `{type:"thinking", thinking:"<text>", signature:"<opaque>"}`. **`thinking` is the EMPTY string in 97.4% (245,268 of 251,864)**; text exists in 6,587 blocks (mostly 2.1.28x; 2.1.140 to 2.1.220 had 0). Only the signature and `usage.output_tokens_details.thinking_tokens` survive. 9 blocks have no signature. |
| `tool_use` | 529,004 | asst | `{type, id:"toolu_...", name, input:{...}, caller:{type:"direct"}}` (`caller` on 99.9%) |
| `tool_result` | 528,957 | user | `{type, tool_use_id, content: str|list, is_error?}`; `is_error` present on 62.7% of blocks (false 314,709, true 17,103), absent on the rest, treat missing as false |
| `image` | 1,349 user; 11,291 inside tool_result | | `{type, source:{type:"base64", media_type, data}}` |
| `document` | 16 user; 2 inside tool_result | | same `source` shape, `media_type` application/pdf |
| `tool_reference` | 9,483 inside tool_result | | `{type, tool_name}` result of `ToolSearch` |
| `fallback` | 65 | asst | `{type:"fallback", from:{model}, to:{model}}` model fallback marker (2.1.201 to 2.1.210) |
| `redacted_thinking`, `server_tool_use`, `web_search_tool_result`, `citations` | 0 | | not present in this corpus |

`tool_result.content` shape: string 488,092 (92.3%), list 40,865 (7.7%; items `text` 59,459, `image` 11,291, `tool_reference` 9,483, `document` 2).

Lines holding more than one block: 99.9% hold exactly one block (assistant: 990,637 of 991,808 lines have 1 block; 917 have 2, 182 have 3, 72 have 4 or 5).
User lines: every one of 530,589 tool_result lines holds exactly 1 tool_result block (parallel results are SEPARATE lines); 63 lines mix a tool_result with other blocks.

Human prompt forms on `user` lines: plain string (typed text, slash command XML), or list of `text` + `image` blocks (pasted images produce `[Image #N]` / `[Image: source: /path]` text companions).

String-content `user` line kinds (prefix test, 16,579 string lines classified): `<task-notification>` 8,588; `<teammate-message` 2,732; `<system-reminder>` 1,776;
`[Image` 1,748; `<local-command-caveat>` 479; `<command-name>` 426 (slash command invocation: `<command-name>/x</command-name><command-message>x</command-message><command-args>..</command-args>`);
compact summary 351; `<local-command-stdout>` 277; `<bash-input>` 101 + `<bash-stdout>` 101 (user `!cmd` shell escapes).

## 4. One API message across several lines, usage, dedupe

Facts (473,570 distinct `message.id` in per-file groups):

| Fact | Measured |
|---|---|
| Lines per `message.id` | 1: 136,549; 2: 196,681; 3: 120,159; 4: 12,143; 5: 4,462; 6 to 12+: about 3,800 |
| Blocks per line | 1 (see section 3). A message with thinking + text + 2 tool_use is 4 lines. |
| `requestId` per `message.id` | always exactly 1 (467,922 of 467,922) |
| Adjacent lines | 288,292 groups adjacent; 49,822 groups interleaved (tool_result user lines in between when the model made parallel tool calls and results streamed back) |
| `parentUuid` chain | 288,691 groups chained (line N's parent is line N-1 of the same message); 49,423 not chained (the interleaved case; sibling tool_use lines hang off the same parent) |
| `apiBlockIndex` (2.1.260+) | present in the 94,135 multi-line groups that have it; always increasing from 0 and contiguous (except 168 starting nonzero) |
| `stop_reason` across the lines of a group | last line only: 142,361; all lines the same non-null value: 159,906 (newer CLIs back-fill); all null: 33,375 (aborted/interrupted stream); mixed: 1,379 |
| `usage` across the lines of a group | identical on all lines: 193,277 groups; two variants: 143,744 groups (early chunks carry a partial `output_tokens`, e.g. 1, the final chunk carries the real one) |
| Which line has the max `output_tokens` | the LAST line in 337,017 of 337,021 groups |
| `input_tokens` first vs last line | equal in 336,975 of 337,021 |
| Same `message.id` in more than one FILE | 6,527 of 467,922 ids (1.4%), mostly resumed sessions that copy history (`main` x 6,331, `sub` x 189, `main+sub` x 7) |
| Same line `uuid` in more than one main file | 28,449 of 995,274 uuids (2.9%): resume/fork copies |

Dedupe rules (recommended):
1. Group assistant lines by `message.id`, within a file, in file order. Keep each line's `uuid` / `timestamp` / block.
2. Message usage = the usage of the LAST line of the group (equivalently max `output_tokens`). NEVER sum usage across lines, and never sum a group's `iterations`.
3. Total usage of a session = sum over distinct `message.id` (dedupe across files too: key `message.id`, fallback `requestId`). Measured over the corpus with this rule: output 370,652,065; input 121,591,745; cache read 109,292,202,096; cache write 2,676,645,161 tokens (475,243 id-in-file pairs; counts a resumed copy twice, so dedupe across files for real totals).
4. `stop_reason` of the message = last non-null value over the group.
5. For cross-file dedupe of history copies use `uuid` of the line first, then `message.id`.
6. `<synthetic>` assistant messages (1,496) have a UUID-shaped `message.id` (not `msg_...`), zero usage, `stop_reason` `stop_sequence` (1,200) or `refusal` (296), `isApiErrorMessage:true`. They are not model output.

`message` (assistant) key presence: `id, type:"message", role, model, content, stop_reason, stop_sequence, stop_details, usage` 100%;
`container` null 24%; `context_management` (obj with `applied_edits[]` on 1,448; null on 24%); `diagnostics` (obj 15,615: `cache_miss_reason:{type, cache_missed_input_tokens?}` with type `previous_message_not_found 5,949, tools_changed 4,471, messages_changed 2,406, unavailable 1,347, system_changed 1,106, model_changed 336`);
`input_transformations[]` (2.1.280+; `{type:"thinking_dropped", path, reason: prefix_binding_mismatch|end_user_binding_mismatch|model_binding_mismatch}`);
`stop_details` obj 445 `{type:"refusal", category, explanation, recommended_model, fallback_*}`.
`stop_reason` values over all lines: `tool_use` 645,920; `end_turn` 48,223; `stop_sequence` 1,200; `refusal` 464; `max_tokens` 78; null 293,050.

`message.usage` shape:
```
{ input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens,   // 100%
  cache_creation:{ephemeral_1h_input_tokens, ephemeral_5m_input_tokens},                // 99.98%
  output_tokens_details:{thinking_tokens} | null,                                       // 28% obj
  server_tool_use:{web_search_requests, web_fetch_requests},                            // 70%
  service_tier:"standard"|null, speed:"standard"|null, inference_geo:"not_available"|"global"|""|null,
  iterations:[{type:"message"|"fallback_message", input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens, cache_creation, model?}] | null,
  fallback_credit: null }
```
Models seen on `message.model`: claude-opus-4-8 364,200; claude-opus-5 211,859; claude-opus-5-5 126,565; claude-sonnet-5 102,359; claude-sonnet-5-5 64,705; claude-sonnet-4-6 51,625;
claude-haiku-4-5-20251001 30,058; claude-opus-4-7 21,471; claude-fable-5 7,174; claude-fable-5-1 5,916; `<synthetic>` 1,496; claude-haiku-5-5 624; and third party backends via proxy:
muse-spark-1.3 568, qwen38 292, deepseek-deepseek-v4.1-flash 12, deepseek-deepseek-v4-pro 9, gemma4:12b-mlx 3. `model` is a free string.
A `[1m]` context suffix appears only in `cost-state.modelUsage` keys and `requestedModel` (39 times), not in `message.model`.

## 5. Pairing tool_use and tool_result

- `assistant` line block `tool_use.id` (`toolu_...`) equals `user` line `message.content[].tool_result.tool_use_id`. Exactly one tool_result block per user line, one user line per call.
- The result line has `parentUuid == sourceToolAssistantUUID ==` uuid of the assistant LINE (not the message) that contains that tool_use. Parallel calls: tool_use lines are siblings in the chain; each result hangs off its own tool_use line.
- Result line is normally the very next relevant line, but hooks, attachments and other results can sit between. Always pair by id, never by position.
- Unmatched: 93 tool_use blocks have no result (41 main, 42 subagent, 10 workflow); 39 tool_results have no tool_use in the same file. Causes: crash/kill, session ended (the CLI later writes a synthetic result, see below), or copy of history.
- Tool name for a result is not on the result line; join through the id. The `toolUseResult` shape depends on the tool name, so you must keep an id => name map while streaming (never a global one, ids are per file).
- Error rate: 17,103 of 528,957 results are `is_error:true` (3.2%): Bash 3.4%, Read 1.9%, Edit 2.3%, WebFetch 4.2%, StructuredOutput 11.3%, browser_batch 9.3%.

### 5.1 Result states (how each outcome is represented)

| Outcome | `tool_result.is_error` | `tool_result.content` | `toolUseResult` | Extra line keys |
|---|---|---|---|---|
| success | false / absent | tool text | object (or string/list for MCP and some tools) | |
| Bash non-zero exit | true (8,906) | `"Exit code N\n<combined stdout+stderr>"` | STRING `"Error: Exit code N\n..."` | exit code ONLY in the text, regex `^Exit code (\d+)\n`. Values: 1 6,300; 2 595; 128 116; 127 81; 130+ 485; 124 20 |
| Bash timeout | true | `"Exit code 143\nCommand timed out after 2m 0s"` | `"Error: Exit code 143\nCommand timed out after ..."` | |
| Bash "soft failure" (grep no match etc.) | false | stdout text | object with `returnCodeInterpretation` ("No matches found" 3,316, "Files differ" 72, "Some directories were inaccessible" 18) | real exit code NOT stored (it is 1 for all three known interpretations) |
| Bash backgrounded | false | `"Command running in background with ID: <id>. Output is being written to: <path>"` | object with `backgroundTaskId` (6,454), `backgroundCwdHint`, `assistantAutoBackgrounded` (2.1.143 to 2.1.177), `timedOutAfterMs` (787: foreground timed out and was moved to background) | later completion is a `user` line with `origin.kind:"task-notification"` and string content `<task-notification><task-id>..</task-id><tool-use-id>toolu_..</tool-use-id><output-file>..</output-file><status>completed|failed|killed</status><summary>Background command "..." completed (exit code 0)</summary></task-notification>`; output is in the `output-file`; poll tools are `TaskOutput` (`task_id, block, timeout`) and `Monitor`; stop is `TaskStop` (`task_id`) |
| Large output | false | `"<persisted-output>\nOutput too large (48.5KB). Full output saved to: <path>\n\n<preview>"` (1,381: Bash 1,315, WebFetch 37, crw_scrape 16, ExitPlanMode 6) | object keeps full `stdout` plus `persistedOutputPath`, `persistedOutputSize` (Bash) | full text in `tool-results/<taskId>.txt` |
| User rejected | true | `"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait..."` (214: Bash 101, AskUserQuestion 69, Agent 12, ...) | STRING `"User rejected tool use"` (198) or `"Error: The user doesn't want ..."` (16, these end with the user's feedback text) | `toolDenialKind:"user-rejected"` (142); `userFeedback` on 5 lines |
| Denied by rule/classifier | true | `"Permission to use Bash with command ... has been denied..."` | `"Error: Permission to use ..."` (893 Bash) | `toolDenialKind: permission-rule 846 / automode-blocked 25 / automode-parsing-error 29 / automode-unavailable 2` |
| Interrupted by user | true | `"[Request interrupted by user for tool use]"` | same string | `toolDenialKind:"interrupted"` (19) and usually a following user text line `[Request interrupted by user for tool use]` (184) or `[Request interrupted by user]` (519, interrupt during streaming); those text lines carry `interruptedMessageId` (400) and their parent is always an assistant line |
| Session ended before result | true | `"[Tool call interrupted: the session ended before this call's result was recorded, so its outcome is unknown. ...]"` | same string | `toolDenialKind:"interrupted"`; written on resume (2.1.296) |
| Cancelled sibling | true | `"<tool_use_error>Cancelled: parallel tool call Bash(...) errored</tool_use_error>"` | `"Cancelled: parallel tool call ..."` (211) | `toolDenialKind:"cancelled"` (1) |
| Input validation / tool error | true | `"<tool_use_error>InputValidationError: ...</tool_use_error>"`, `"<tool_use_error>File has not been read yet..."`, `"File does not exist..."`, `"String to replace not found..."` | `"Error: ..."` or `"InputValidationError: ..."` string | |
| Hook blocked | true | `"PreToolUse:Bash hook error: [...]"` | `"Error: PreToolUse:Bash hook error..."` | plus `attachment hook_*` lines |

Rules of thumb: when `toolUseResult` is a string it is the failure/denial text (prefix `Error: ` is added to the content text); when it is an object it is the success shape.
Bash `toolUseResult.interrupted` exists on all 257,666 object results but is never true in this corpus; do not use it as the interrupt signal.
Bash object `stdout`/`stderr` are separate; `tool_result.content` is stdout when stderr is empty (215,268 of 229,729), and stdout+stderr concatenated when stderr is non-empty (27,967 of 29,013). So stderr is recoverable only from the object.
Duration of a Bash call is NOT recorded; only derivable from `tool_use` line timestamp to result line timestamp (timestamps are write times and are not monotonic, see 11).

## 6. System lines, compaction, errors

`system` subtypes (40,726): `turn_duration` 18,124; `stop_hook_summary` 14,318; `away_summary` 6,282; `informational` 575; `local_command` 469; `compact_boundary` 354;
`api_error` 279; `scheduled_task_fire` 201; `model_refusal_no_fallback` 58; `bridge_status` 39; `agents_killed` 18; `model_refusal_fallback` 9.
`system` lines have `parentUuid` and are graph nodes (children hang off them); `content` text exists on 7,987 of them.

| Subtype | Fields beyond the envelope (presence) |
|---|---|
| `turn_duration` | `durationMs` 100%, `messageCount` 100%, `isMeta`, `pendingBackgroundAgentCount` 12%, `pendingWorkflowCount` 2% (to 2.1.238), `teamName/agentName` |
| `stop_hook_summary` | `hookCount, hookInfos[{command, durationMs, promptText?}], hookErrors[], hasOutput, preventedContinuation, stopReason, toolUseID, level:"suggestion", hookAdditionalContext[] (89%)`. `toolUseID` is NOT a tool_use id (0 of 14,378 match) |
| `away_summary` | `content` (recap text), `isMeta` |
| `informational` | `content`, `level: notice 533 / warning 37 / info 5` |
| `local_command` | `content` (the command output text), `commandRun:{command, args}` (12.6%, 2.1.272+), `level:"info"` |
| `compact_boundary` | `content:"Conversation compacted"`, `level:"info"`, `logicalParentUuid`, `compactMetadata` (below); `parentUuid` is null |
| `api_error` | `error` (obj; shapes vary by version: `{status, headers, requestID, error:{...}}` in 2.1.143, `{formatted, message, connection:{code,isSSLError,message}, isNetworkDown, requestId}` later), `retryAttempt, maxRetries, retryInMs, level:"error"`, `cause` (2.1.143 only), `source:"connection_retry"` |
| `scheduled_task_fire` | `cron, cronKind, prompt, taskId, taskKind:"loop"` (30; rest bare) |
| `model_refusal_fallback` / `_no_fallback` | `originalModel, fallbackModel?, direction?, trigger:"refusal", apiRefusalCategory, apiRefusalExplanation, refusedUserMessageUuid, retractedMessageUuids[], requestId, scope?` |
| `agents_killed`, `bridge_status` | bare (`bridge_status` has `url`) |

`compact_boundary.compactMetadata`: `trigger: auto 213 / manual 142`, `preTokens`, `postTokens`, `durationMs` (all 100%), `preCompactDiscoveredTools[]` (87%), `cumulativeDroppedTokens` (58%, 2.1.197+),
`preservedSegment:{headUuid, tailUuid, anchorUuid}` and `preservedMessages:{anchorUuid, uuids[], allUuids[]}` (72%, 2.1.156+), `precomputed:bool` (2.1.170 to 2.1.193). 18 boundaries are inside subagent files.
Sequence on disk, in order:
1. `system/compact_boundary` (`parentUuid:null`, `logicalParentUuid` = last pre-compaction line)
2. `user` with `isCompactSummary:true`, `isVisibleInTranscriptOnly:true`, string content starting `This session is being continued from a previous conversation that ran out of context.` (4 KB to 36 KB; 351 of 354 are strings)
3. zero or more `attachment/compact_file_reference` (535 total: `{filename, displayPath}`) and re-injected `file` / `plan_file_reference` / `invoked_skills` attachments
4. for manual compaction: `user` string `<command-name>/compact</command-name>...` and `<local-command-stdout>Compacted (ctrl+o to see full summary)</local-command-stdout>` lines (the latter can come before the boundary).
Pre-compaction history stays in the same file (nothing is deleted), so the file is the full record and "context window" is a derived view. Timestamps around the boundary are not monotonic (attachment `23.983` precedes its summary `24.044`).

Synthetic API error messages: `assistant` line with `message.model:"<synthetic>"`, `isApiErrorMessage:true`, `error` (`rate_limit 619, invalid_request 393, server_error 231, unknown 74, authentication_failed 6, model_not_found 3`), `apiErrorStatus` (429 618, 529 65, 500 25, 400 12, ...), text such as `API Error: ...`, `You've hit your session limit...`, `Prompt is too long`, `No response requested.` (153, a non-error filler); `quotaLimits` on rate limit ones.

Slash commands: `user` string `<command-name>/x</command-name>` + `<command-message>` + `<command-args>`; for skill commands a following `user` line with `isMeta:true`, content list `text` starting `Base directory for this skill: <dir>` and `sourceToolUseID` = the `Skill` tool_use id when invoked through the Skill tool. `Skill` result: `toolUseResult:{success, commandName, allowedTools[], status?:"forked", agentId?, result?}`, content `"Launching skill: <name>"`.

## 7. Subagents, links, workflows, teams

| Link | How |
|---|---|
| parent => child file | `Agent` tool_use `id` == child `meta.toolUseId` (present on 75% of metas; 84% of those resolve inside the parent file; the rest were spawned from a nested subagent or by another mechanism). Parent `toolUseResult.agentId` (or `agent_id`) == child file stem (70% of children appear in a parent result; async/team/workflow spawns and nested spawns account for the rest) |
| child file identity | `(sessionId=parent session, agentId)`; every child line has `isSidechain:true`, `agentId`, `parentUuid:null` on its first line (7,047 of 7,049) |
| nesting | `meta.parentAgentId` + `meta.spawnDepth`; the file is still stored flat in the root session's `subagents/` dir |
| result into parent | sync (only up to 2.1.284): tool_result for the `Agent` call has `toolUseResult:{status:"completed", agentId, agentType, prompt, content:[{type:"text",text}], totalDurationMs, totalTokens, totalToolUseCount, usage{...}, toolStats{readCount, searchCount, bashCount, editFileCount, linesAdded, linesRemoved, otherToolCount}, resolvedModel}` and the content text ends with `agentId: <id> (use SendMessage ...)`. Async (`status:"async_launched"` 3,299; `isAsync, outputFile, canReadOutputFile, description, resolvedModel`): result arrives later as a `user` line `origin.kind:"task-notification"` with `<task-id>` (= agentId), `<tool-use-id>`, `<output-file>`, `<status>`, `<summary>`, `<result>`. Team member (`status:"teammate_spawned"` 1,494): adds `teammate_id, agent_id, agent_type, name, color, model, team_name, tmux_*, is_splitpane, plan_mode_required`; messages between members arrive as user strings `<teammate-message teammate_id=".." summary="..">..` and `SendMessage` calls (`toolUseResult:{success, message, routing{sender,target,content,summary}, msg_id?, resumedAgentId?}`) |
| `agentType` values on results | general-purpose 1,585; code-reviewer 390; Explore 326; Plan 61; fork 19; claude-code-guide 5; claude 4. `Agent` input `subagent_type` 91.6% present; absent means general-purpose |
| forked agent | `fork-context-ref` first line `{agentId, parentSessionId, parentLastUuid, contextLength}`: child inherits parent context by reference (7 files), do not copy |
| attribution | child assistant lines carry `attributionAgent` (the agent type) and teams `agentName/teamName` |

Subagent meta example (structure only): `{"agentType":"Explore","description":"<text>","toolUseId":"toolu_...","spawnDepth":1,"requestShape":"background","requestNonInteractive":true,"model":"sonnet"}`.

Workflow journal lines (workflow subagent dirs): `{"type":"started","key":"v2:<hash>","agentId":"<id>"}` 9,106 and `{"type":"result","key":"v2:<hash>","agentId":"<id>","result":<arbitrary JSON from the structured output tool>}` 8,616. `Workflow` tool: input `{script|scriptPath, name, args, description, resumeFromRunId, run_in_background}`, result `{runId, scriptPath, status:"async_launched", summary, taskId, taskType, transcriptDir, workflowName}`.
`StructuredOutput` tool (8,705 calls) is the structured final answer of a workflow agent; its input is arbitrary schema-defined JSON; 11.3% of calls are errors (schema mismatch: `"Error: Output does not match required schema"`).

Subagent token cost is NOT counted in the parent lines; read it from the child file lines (dedupe by `message.id`) or from the `Agent` result `usage`/`totalTokens` (up to 2.1.284).

## 8. Attachments (339,274 lines, 53 subtypes)

Shape: `{type:"attachment", uuid, parentUuid, timestamp, ..., attachment:{type:"<subtype>", ...}, rendered?:[{content:"<system-reminder>..."}]}`.
Most are harness-injected context shown to the model inside system reminders, not user speech. Counts:
`total_tokens_reminder` 157,708 (`{text:"<total_tokens>N tokens left</total_tokens>"}`); `hook_success` 53,650; `task_reminder` 18,609; `deferred_tools_delta` 17,922; `skill_listing` 17,688;
`deferred_tools_record` 11,271; `queued_command` 8,451; `hook_additional_context` 7,584; `diagnostics` 5,587; `environment` 5,159; `edited_text_file` 4,100; `prompt_snapshot` 3,627; `silent_turn_reminder` 3,063;
`mcp_instructions_delta` 2,903; `agent_listing_delta` 2,468; `date` 2,094; `batching_reminder_sent` 1,962; `session_context` 1,908; `model` 1,878; `instructions` 1,859; `bash_output_audience_note` 1,341;
`credential_org` 1,300; `auto_mode` 1,250; `command_permissions` 996; `file` 927; `date_change` 861; `language` 741; `nested_memory` 672; `compact_file_reference` 535; `read_truncation_notice` 253;
`invoked_skills` 194; `goal_status` 159; `plan_file_reference` 89; `plan_mode` 80; `plan_mode_exit` 76; `thinking_drop` 46; `hook_non_blocking_error` 45; `task_status` 32; `dynamic_skill` 31; `context_tip` 27;
`directory` 23; `async_hook_response` 21; `remote_session_change` 18; `plan_mode_reentry` 15; `hook_cancelled` 13; `team_context` 12; `hook_blocking_error` 6; `hook_system_message` 6; `thinking_stripped` 4; `auto_mode_exit` 4;
`ultrathink_effort` 3; `skill_mention` 2; `workflow_keyword_request` 1.

Useful shapes (structure only):
- `hook_success`: `{hookName:"PreToolUse:Bash", hookEvent, toolUseID (the tool_use id the hook ran for), command, stdout, stderr, exitCode:0, durationMs, content:""}`. Hook events seen: PreToolUse 39,899; SubagentStart 7,907; UserPromptSubmit 3,300; SessionStart 2,629; PostToolUse 116. All 53,851 `exitCode` are 0; failures use `hook_non_blocking_error` / `hook_blocking_error` / `hook_cancelled`.
- `hook_additional_context`: `{hookEvent, hookName, toolUseID, content:[<text>]}` (SubagentStart 7,133; SessionStart 304; PostToolUse 175).
- `queued_command`: `{prompt: str | [text/image blocks], commandMode:"prompt", origin?:{kind,...}, isMeta?, source_uuid?, usage?:{durationMs, toolUses, totalTokens}, delivery_id?, humanTurn?, imagePasteIds?}`: human input typed while the agent was busy and folded into the running turn.
- `file`: `{filename, displayPath, content:{type:"text", file:{filePath, content, numLines, startLine, totalLines}}}` (an @-mentioned file; also `base64` variants).
- `edited_text_file`: `{filename, snippet:"<cat -n text>", displayPath?}` file changed on disk outside the agent.
- `task_reminder`: `{itemCount, content:[{id, subject, description, status, activeForm, blocks[], blockedBy[], owner}]}` current task list state.
- `diagnostics`: `{isNew, files:[{uri, diagnostics:[{message, severity, source, code, range}]}]}` IDE/LSP diagnostics.
- `environment`: `{snapshot:{workingDirectory, isGitRepo, isWorktree, platform, shell, osVersion, additionalWorkingDirectories[], scratchpadDirectory?}, changes?:[{field, from, added, removed}]}`.
- `nested_memory`: `{path, displayPath, content:{type, path, content, rawContent?, parent?, contentDiffersFromDisk}}` CLAUDE.md files loaded.
- `deferred_tools_delta`: tool search bookkeeping `{addedNames[], addedLines[], removedNames[], readdedNames[], surfacedNames[], failedMcpServers[], needsAuthMcpServers[], pendingMcpServers[], ...}`.
- `skill_listing`: `{content, skillCount, names[], isInitial}`; `invoked_skills`: `{skills:[{name, path, content}]}`.

## 9. Session metadata and the parentUuid graph

| Item | Source | Rule |
|---|---|---|
| session id | `sessionId` on lines = file stem (main) or parent dir (subagent) | one per file |
| title | `custom-title.customTitle` (user, /rename) beats `agent-name.agentName`; `ai-title.aiTitle` is re-emitted many times (67,884 lines for 1,341 main files), latest wins | no `summary` line type exists in this corpus |
| first/last prompt | `last-prompt.lastPrompt` (first 200 chars or so) + `leafUuid` (uuid of the conversation leaf at that time). `last-prompt` also emitted 73,228 times, latest wins | |
| cwd | `cwd` on each line; more than one value inside a file in 6.6% of files (553 with 2, 253 with 3, 380 with 4+); `relocated.relocatedCwd`, `worktree-state`, `EnterWorktree` (`toolUseResult:{worktreePath, worktreeBranch, message}`) explain moves | take cwd per line, session cwd = first |
| git branch | `gitBranch` on each line, more than one value in 1.3% of files (value can be `HEAD`) | |
| cli version | `version` on each line; more than one version in a file in 100 of 17,883 files (resumed with a newer CLI) | min/max per session |
| model | `message.model` of assistant lines; `requestedModel` shows the requested one when different; `cost-state.modelUsage` keys | many per session |
| started/updated | min/max `timestamp` over conversation lines (metadata lines have none) | |
| mode | `permission-mode.permissionMode` (latest wins), plus per-user-line `permissionMode` | |
| PR | `pr-link` | |
| cost | `cost-state` snapshot: `{totalCostUSD, totalAPIDuration, totalAPIDurationWithoutRetries, totalToolDuration, totalDuration, totalLinesAdded, totalLinesRemoved, startTime (epoch ms), hasUnknownModelCost, modelUsage:{<model[1m]>:{inputTokens, outputTokens, thinkingTokens, cacheReadInputTokens, cacheCreationInputTokens, webSearchRequests, costUSD}}}`. There is NO per-message cost in transcripts, only this per-session snapshot (1,616 lines in 1,341 files) |
| continuation | `worktree-state.worktreeSession.sessionId`, `continued-in.continuedInSessionId`, `bridge-session` | links between session files |

Graph facts (per file): 17,934 roots (`parentUuid:null`), 250 files with more than one root (compaction boundaries, subagent starts), 95 files with 147 dangling `parentUuid` (parent not in the same file: truncated or history copy), 5,164 files with forks, 67,219 nodes with more than one child.
Forks come from (a) parallel tool calls (a tool_use line has the next sibling chunk AND its result as children) and (b) rewinds/resume branches. File order is the authoritative chronology, the tree is only needed to reconstruct a branch;
`last-prompt.leafUuid` points at the tip the CLI considered current. Do not sort by timestamp: 1,803 of 17,883 files (10%) contain 19,569 conversation lines whose timestamp is earlier than the previous line's.

`queue-operation` (39,361): `{operation: enqueue 19,893 | dequeue 9,644 | remove 9,811 | popAll 13, content?, timestamp, sessionId, reason?: absorbed_mid_turn 3,077 | delivered_to_agent 50, commandUuid?, deliveryId?}`. enqueue = text typed while busy. dequeue = taken as its own turn (a `user` line follows). remove = folded into the running turn (a `queued_command` attachment follows). popAll = returned to the input box unsent. Strings starting with `<` are notifications, not human text.

## 10. Per-tool reference

Input keys are `tool_use.input`; `toolUseResult` (TUR) keys are the structured result on the result line. Presence in parentheses when not 100%. Counts are tool_use counts over the corpus.
General warning: models sometimes emit malformed input. Seen: `__unparsedToolInput:{raw,len}` (Read 182, StructuredOutput 8, SendUserFile, ReportFindings, AskUserQuestion, browser_batch), misspelled keys (`Bash2`, `limter`, `new_new_string`, `timeout_ms` on Bash as string), numbers as strings (`Agent.run_in_background` string 8.8%, `Monitor.persistent` string, `max_results` string, `offset` string/list), unknown tool names (`bash` 36, `bash_placeholder`, `Bash2`). Typed inputs must be lenient (accept string/number/bool coercions, keep raw).

| Tool (count) | Input | TUR success shape | Notes |
|---|---|---|---|
| `Bash` (325,442) | `command`, `description` (80%), `timeout` (ms int, 13%), `run_in_background` (2%), `dangerouslyDisableSandbox` (0.6%) | `{stdout, stderr, interrupted, isImage, noOutputExpected}` always; optional `backgroundTaskId`, `backgroundCwdHint`, `assistantAutoBackgrounded`, `timedOutAfterMs`, `returnCodeInterpretation`, `persistedOutputPath`, `persistedOutputSize`, `staleReadFileStateHint`, `ghRateLimitHint`, `gitOperation{commit{sha,kind:committed|cherry-picked|amended,branch?}, push{branch}, branch{ref,action}, pr{number,url,action}}`, `bashEditDiff{files[{filePath,hunks[],created?,deleted?}], changedFiles[], moreFiles, shared, unavailable}` (2.1.272+), `dangerouslyDisableSandbox` | no `cwd`, no `env`, no exit code, no duration. cwd = line `cwd`. Also 11,057 string TURs (errors, section 5.1) |
| `Read` (80,403) | `file_path`, `offset` (int, 41%), `limit` (int, 45%), `pages` (pdf, 75 uses) | discriminated by TUR `type`: `text` 37,147 `{file:{filePath, content, numLines, startLine, totalLines, truncatedByTokenCap?}}`; `image` 5,705 `{file:{type:"image/png|jpeg|gif", base64, originalSize, dimensions{originalWidth,originalHeight,displayWidth,displayHeight}}}`; `parts` 64 (pdf pages `{file:{count, outputDir, originalSize}}`); `pdf` 18; `notebook` 1 `{file:{cells[{cellType,source,cell_id,language}]}}`; `file_unchanged` 1; `firstPage` | `message.content` text is `cat -n` format (line number, TAB, text); TUR `file.content` is raw text without numbers. Both are in the line, so each file read is stored twice. Image reads: result content is `[{type:"image",...}]` |
| `Edit` (33,504) | `file_path`, `old_string`, `new_string`, `replace_all` | `{filePath, oldString, newString, originalFile (null 71%, full pre-edit text 29%), structuredPatch[{oldStart,oldLines,newStart,newLines,lines[]}], userModified, replaceAll, staleRecovered?, contentNotInModelContext?, memdirStamped?}` | `lines` are unified-diff lines prefixed ` `, `+`, `-`. `userModified:true` when the user changed the proposal in the permission dialog. Content text: `The file <path> has been updated successfully.` |
| `Write` (8,492) | `file_path`, `content` | `{type:"create"|"update", filePath, content, structuredPatch[] (empty for create), originalFile (null 90%), userModified, memdirStamped?}` | |
| `NotebookEdit` (1) | `notebook_path, cell_id, cell_type, edit_mode, new_source` | `{cell_id, cell_type, edit_mode, language, new_source, notebook_path, original_file, updated_file, error?}` | |
| `Grep` (16, old) | `pattern, path, output_mode, glob, -n, -A, -B, -i, head_limit` | string TUR | search now happens through Bash |
| `WebFetch` (13,083) | `url`, `prompt`, `offset`? | `{url, bytes, code, codeText, durationMs, result}` (10,476); 546 string TURs are errors | `result` is the LLM-processed summary, not the page |
| `WebSearch` (10,985) | `query`, `allowed_domains?`, `blocked_domains?`, `mode?: standard|extended`, `max_results?` | `{query, results:[{tool_use_id, content:[{title,url}]}], durationSeconds, searchCount}` | content text is a markdown rendering |
| `Agent` (7,609) (old name `Task`, unverified: 0 in corpus) | `description, prompt, subagent_type (92%), model (48%), name (24%), run_in_background (bool 25% or string 9%), isolation ("worktree"), mode, effort` | section 7 | |
| `SendMessage` (3,847) | `to` (alias `recipient`), `message` (str or obj), `summary`, `type`, `recipient_kind` | `{success, message, routing{...}, msg_id?, pin?, resumedAgentId?}` | agent to agent / teammate |
| `ToolSearch` (4,895) | `query`, `max_results` | `{matches[], query, total_deferred_tools}`; content = list of `tool_reference` blocks | loads deferred tool schemas; later tool_use of those tools becomes valid |
| `Skill` (1,005) | `skill`, `args` (69%) | `{success, commandName, allowedTools[], status?, agentId?, result?}` | |
| `TaskCreate` (843) / `TaskUpdate` (1,416) / `TaskList` (68) / `TaskGet` (17) | `subject, description, activeForm` / `taskId, status, addBlockedBy, owner, ...` | `{task:{id,subject}}` / `{success, taskId, updatedFields[], statusChange{from,to}, error?}` / `{tasks[{id,subject,status,blockedBy[]}]}` | replaces `TodoWrite` (unverified: 0 in corpus). Status: pending, in_progress, completed, deleted |
| `TaskOutput` (104) / `TaskStop` (302) | `task_id, block, timeout` / `task_id` | `{retrieval_status, task{task_id, task_type, status:running|completed, description, output, exitCode, prompt?, result?}}` / `{task_id, task_type, command, message}` | background shell and agent control; replaces `BashOutput` (6 left, `bash_id`) and `KillShell` (unverified: 0) |
| `Monitor` (1,266) | `command, description, persistent, timeout_ms` | `{taskId, timeoutMs, persistent}` | streams events back as `task-notification` user lines |
| `AskUserQuestion` (836) | `questions:[{question, header, multiSelect, options:[{label, description, preview?}]}]` | `{questions[], answers:{<question text>:<answer text>}, annotations:{<question text>:{preview?, notes?}}}` (dynamic keys = question text); content text `Your questions have been answered: ...`; rejection = `User rejected tool use` | |
| `ExitPlanMode` (57) / `EnterPlanMode` (9) | `plan, planFilePath, allowedPrompts[{tool,prompt}]` | `{plan, isAgent, filePath, hasTaskTool, planWasEdited?}` / `{message}` | approval shows in content text and a `plan_mode*` attachment |
| `EnterWorktree` (13) / `ExitWorktree` (6) | `name|path` / `action` | `{worktreePath, worktreeBranch, message}` / `{action, discardedCommits?, discardedFiles?, originalCwd, worktreePath, worktreeBranch, message}` | |
| `ScheduleWakeup` (378), `CronCreate/List/Delete` (5), `PushNotification` (24), `SendUserFile` (58), `Artifact` (84), `ReportFindings` (64), `SubagentHandback` (20), `ListAgents` (193), `Workflow` (373), `StructuredOutput` (8,705), `SendFeedback` (4), `LSP` (1), `ListMcpResourcesTool` / `ReadMcpResourceTool` | see the compact lists in the scan; all return small JSON objects | | treat as `Other` with raw input and raw result |
| `mcp__<server>__<tool>` (24,823 calls, 111 tools) | free-form per server | TUR is a LIST of MCP content blocks (`[{type:"text",text}|{type:"image",...}]`) in most cases (7,195 `computer`, ...), a JSON STRING in others (crw_scrape 1,844), rarely an object (4). The same data is in `tool_result.content` | Server list (calls): claude-in-chrome 14,998; playwright 3,905; claude_ai_fastcrw 3,351; yargi-mcp 1,113; crw 858; x-ads 465; shepherd 53; plugin_figma_figma 32; vcvrack 28; plugin_firebase_firebase 16; ctx 3; claude_ai_Seltz 1. Parse: strip `mcp__`, split at the first `__`; server part is a sanitised name (`.` and `:` and spaces become `_`, dashes survive). `attributionMcpServer` keeps the raw name (`claude.ai fastcrw`, `plugin:figma:figma`) but is unreliable per call |

Tool names seen overall: 45 distinct non-MCP names (about 41 real built-ins plus hallucinated `bash`, `bash_placeholder`, `bash-placeholder`, `Bash2`) and 111 distinct MCP tool names (24,823 calls). Real built-ins: Bash, Read, Edit, WebFetch, WebSearch, StructuredOutput, Write, Agent, ToolSearch, SendMessage, TaskUpdate, Monitor, Skill, TaskCreate, AskUserQuestion, ScheduleWakeup, Workflow, TaskStop, ListAgents, TaskOutput, Artifact, TaskList, ReportFindings, SendUserFile, ExitPlanMode, PushNotification, SubagentHandback, TaskGet, Grep, EnterWorktree, EnterPlanMode, ExitWorktree, BashOutput, SendFeedback, ListMcpResourcesTool, CronList, CronCreate, CronDelete, LSP, NotebookEdit, ReadMcpResourceTool, ReadMcpResourceDirTool.
Absent here but known from older/other builds (unverified): `Task` (renamed `Agent`), `TodoWrite`, `MultiEdit`, `LS`, `Glob`, `KillShell`/`KillBash`, `SlashCommand`, `WebSearch` server blocks. Keep an alias table and an `Other` fallback.

## 11. Edge cases and traps (all measured)

1. One line = one content block; regroup by `message.id` (section 4). Interleaving with tool_result lines is common (15%).
2. Early streamed chunks carry partial `output_tokens`; last chunk is truth; never sum.
3. Thinking text is empty in 97% of blocks (signature only). Do not model reasoning as always having text. `thinking_tokens` in usage is the only size signal.
4. Non-human user lines: `isMeta` (7,463), `origin.kind != human`, `<task-notification>`, `<teammate-message>`, `<system-reminder>`, `<local-command-*>`, `<bash-input>`, compact summary, "[Request interrupted by user..." markers, `Another Claude session s...` / coordinator messages. Wend and qoop-history both filter on these (see 12).
5. Tool result text is on `user` lines; a "user message" is only human if no tool_result block, not `isMeta`, `origin.kind` absent or `human`, and not one of the marker strings.
6. No exit code field; regex on text (section 5.1). `is_error:false` can hide exit 1 (`returnCodeInterpretation`).
7. `toolUseResult` is object OR string OR list depending on tool/outcome; a string means error/denial.
8. Timestamps are write times, not monotonic in 10% of files; metadata lines have none.
9. Resume/fork copies history lines with the same `uuid` into the new file: 2.9% of uuids appear in 2+ main files, 1.4% of `message.id`.
10. `sessionId` of subagent lines is the parent's; do not treat subagent files as sessions of their own. Their `parentUuid:null` first line is a root, not a new conversation.
11. Parent `Agent` result may never show a subagent run (crash, async): discover subagents by listing the dir and `meta.json`, link by `toolUseId` when present.
12. Data hazards: lines up to 6 MB; base64 images in 1,349 user blocks and 11,291 tool_result blocks and in `toolUseResult.file.base64`; a `Read` stores file content twice; `Edit.originalFile` can hold a full file.
13. Dynamic JSON keys: `file-history-snapshot.snapshot.trackedFileBackups.<path>`, `AskUserQuestion` answers/annotations keyed by question text, `wireToolInputs.<toolu_id>`, `cost-state.modelUsage.<model>`; do not model as structs.
14. Model-invented input (see section 10 warning) and tool names not in the tool list (`bash`), results then are `<tool_use_error>Error: No such tool available: bash</tool_use_error>`.
15. Secrets/user content: prompts, file content, hook stdout and env-derived hints (`rtk` rewrite hook) are all stored verbatim; the parser should offer redaction hooks.
16. Unknown future fields appear every release (24 new user keys and 25 new assistant keys appeared between 2.1.143 and 2.1.296). Never use `deny_unknown_fields`; keep a `serde_json::Value` extras map on lines and on blocks.

## 12. What the existing parsers handle and drop

`~/coding/wend/crates/wend-core/src/parser/{mod,routing,content}.rs`, `model.rs` (index/search oriented):
- Tolerates corrupt/blank lines, keeps 1-based `line_no`, file order = chronological (comment "verified: boundary timestamps are monotonic by line", true for boundaries only).
- Keeps ALL `system/*`, `attachment`, `progress`, unknown types as traversable graph nodes with empty text because they parent real turns ("dropping them breaks recovery").
- `isCompactSummary` is top-level (not under `message`). Compaction handled from `system/compact_boundary`: `logicalParentUuid`, `compactMetadata.{trigger, preTokens, postTokens}` (it reads `postTokens` as optional; it exists on 100% here).
- Attachment text: `attachment.content.file.content` (file), plain string `content` when non-empty, `edited_text_file.snippet`; empty-content `hook_success` stores node without block.
- Content: string or blocks; `thinking` kept for render but not indexed (reads `thinking` or `text`); `tool_use` indexes name + all string leaves of input; `tool_result` recurses into `content`; `image` stores only `media_type` and approximated byte length (never base64); `tool_reference` indexed by `tool_name`; unknown block types are dropped (`document`, `fallback` are lost).
- Routes `worktree-state` (continuation link via nested `sessionId`), `bridge-session`, `ai-title` (latest wins) vs `custom-title`; skips `agent-name, last-prompt, permission-mode, mode, pr-link, file-history-snapshot, queue-operation, agent-setting`.
- Drops: all of `toolUseResult`, `usage`, `model`, `stop_reason`, `requestId`, `message.id` (no regrouping), `is_error`/status semantics beyond a bool, subagent files, `origin`, hook details, cost.

`~/coding/qqoro/qoop-tui/crates/qoop-history/src/{claude,model}.rs` (text of the human/assistant conversation only):
- Hard-won rules: `isMeta` lines are no message (skill text); `isCompactSummary` and `origin.kind != "human"` user lines are nobody's message; texts starting `This session is being continued` or `[Request interrupted by user` are not typed; a `user` line made only of `tool_result` blocks is the assistant side (role flip), not the person.
- Queue handling: `queue-operation` enqueue/dequeue/remove/popAll is replayed with a FIFO so that text typed while the agent was busy gets the time it was typed (the `user` line is stamped when the agent took it), a still-queued text counts as typed, `popAll` clears it, `queued_command` attachments with `origin.kind == human` become a user message, texts beginning with `<` are notifications and skipped.
- Tool use is flattened to `name + string leaves` capped at 2,000 chars with a `[…]` marker; thinking and images are excluded; unknown/corrupt lines counted not fatal.
- Drops everything structured: tool results, usage, models, subagents, system events.

## 13. Version drift (2.1.143 to 2.1.296, measured by first/last version a field occurs)

| Change | Versions |
|---|---|
| `stop_details`, `usage.iterations[].model` | stop_details from 2.1.156; iteration model 2.1.201 to 2.1.210 only |
| `gitOperation` on Bash results, `dangerouslyDisableSandbox` | 2.1.156 |
| `attributionMcpServer/Tool` | 2.1.156 |
| `interruptedMessageId` | 2.1.156 |
| `promptSource` | 2.1.168 |
| `Agent` result `resolvedModel` | 2.1.177 |
| team fields (`teamName`, `agentName`, `teammate_*`, `tmux_*`) | 2.1.181 to 2.1.206 in lines; `Agent.toolUseResult` team fields persist to 2.1.296 |
| `session_id` snake-case duplicate, `toolDenialKind`, `effort` (assistant, 2.1.216) | 2.1.201 |
| `Bash.timedOutAfterMs`, `backgroundCwdHint` | 2.1.210 |
| `apiBlockIndex`, `rendered` (attachment), `truncatedAfterOutput` | 2.1.260 |
| `bashEditDiff` | 2.1.272 |
| `wireToolInputs`, `wireIngestContext`, `perTurnEffort` | 2.1.272 |
| `input_transformations`, `advisorModel`, `turnOrigin`, `queueTranscriptOnly`, `scheduledTaskId` | 2.1.280 |
| `turnPosition`, `classifierBoundary`, `contentNotInModelContext` | 2.1.284 |
| `permissionDecision`, `requestedModel`, `thinkingDurationMs`, `usage.fallback_credit` | 2.1.295 |
| `thinkingDisplay`, `summarizedTurnPromptUuids` | 2.1.296 |
| `Agent` sync `completed` result (content, usage, toolStats, totalTokens) | gone after 2.1.284; 2.1.295+ are `async_launched` plus notification only |
| `compactMetadata.preservedSegment/preservedMessages` | from 2.1.156; `cumulativeDroppedTokens` from 2.1.197; `precomputed` 2.1.170 to 2.1.193 |
| `api_error.error` shape | 2.1.143 raw SDK error (`status, headers, error{type,request_id}`); 2.1.156+ `{formatted, message, connection, isNetworkDown, rateLimits}` |
| `stop_hook_summary` frequency | fell sharply after 2.1.220 (hooks reconfigured) |
| thinking text | empty in 2.1.143 to 2.1.224 (1 non-empty of 162.5k); some text returns in 2.1.260 (210) and 2.1.280 (6,426) |
| `slug` | present in about 72% of lines at 2.1.14x, about 28% at 2.1.28x |
| `origin` on user lines | 0.7% at 2.1.14x, about 5% at 2.1.28x |
| tool renames | `Task` => `Agent`, `TodoWrite` => `TaskCreate/Update`, `KillShell` => `TaskStop`, `BashOutput` => `TaskOutput`, search tools folded into Bash (all unverified for the pre-2.1.143 side) |

Nothing in 2.1.143 to 2.1.296 changed the envelope (`uuid, parentUuid, type, message.content` blocks, `tool_use`/`tool_result` ids) incompatibly. The drift is additive; the risky ones are fields that disappear (Agent sync result) and enum growth (`attachment.type`, `system.subtype`, `toolDenialKind`).

## 14. Critique of the proposed common model for Claude Code

Fits as is: Session id/cwd/git_branch/cli_version/model(s)/title/timestamps; Entry with id and parent_id; Part Text / Image / ToolCall; Usage fields; Compaction with trigger and tokens.

Does not fit, needs change:
1. **Entry granularity.** On disk 1 line = 1 block of an API message. The model should have one `Message` entry per `message.id` (parts in `apiBlockIndex`/file order) and keep `source_lines: Vec<LineRef{line_no, uuid, timestamp}>`. `Entry.id` should be `message.id` for assistant messages and `uuid` for user lines; keep `uuids` for graph parent resolution (children point at specific line uuids, not at message ids). Per-line timestamps differ inside one message (give the message `ts_start`/`ts_end`).
2. **tool_result is not a user message.** 100% of tool_results sit on `user` lines. The parser must fold each result into its ToolCall (by `tool_use_id`) and emit no user Message for it. A ToolCall output must keep `result_line_uuid`, `result_ts`, and the separate `text` (what the model saw) vs `structured` (`toolUseResult`) vs `images`.
3. **Role `developer` has no Claude equivalent.** Needed instead: a `MessageOrigin` on user messages: `Human | TaskNotification | Coordinator | Peer | Teammate | AutoContinuation | SlashCommand | LocalCommandOutput | LocalCommandCaveat | ShellEscape | SkillBody | CompactSummary | InterruptMarker | SystemReminder | Meta(other)`. Without this every consumer re-learns section 11 point 4.
4. **Reasoning.** `Reasoning{text, signature}` must make `text` optional and add `redacted: bool`; also keep `thinking_tokens` and `thinkingDurationMs`/`thinkingDisplay`. 97% will be signature only.
5. **Usage.** Add `cache_write_5m`, `cache_write_1h` (the proposed `cache_write` alone loses the split), `thinking_tokens` (it is inside `output_tokens`, do not add), `web_search_requests`, `web_fetch_requests`, `service_tier`, `speed`, `inference_geo`, and `iterations[]` (per API call incl. `fallback_message`). Cost: Claude Code stores no per-message cost; session-level `cost-state` only, so `cost` must be `Option` and sourced separately (`SessionCost{total_usd, per_model, api_duration, tool_duration, lines_added, lines_removed}`).
6. **Dedupe/regrouping flags.** Add `Message.partial: bool` (stream aborted, `stop_reason` null on every line, `isAbortedMidStream`, `truncatedAfterOutput`) and `stop_reason`/`stop_details`/`request_id`/`synthetic: bool` + `api_error{status,kind}` for `<synthetic>` messages. Add cross-file `duplicate_of` for resumed copies.
7. **Subagents.** `parent session link` is not enough. Need `Subagent{agent_id, parent_session_id, parent_tool_use_id (Option), parent_agent_id (Option), depth, agent_type, description, model, name/team, mode (sync|async|team|workflow|fork)}` and the ToolCall `Subagent` should point to it (`agent_id`) plus the returned `{status, total_tokens, tool_use_count, duration_ms, tool_stats}`. A subagent session is addressed by `(session_id, agent_id)`, not by a file-stem session id. Forked agents reference parent context (`fork-context-ref`).
8. **Shell.** Proposed `ShellOutput{exit_code, duration, truncated}` needs: `exit_code: Option<i32>` with `exit_code_source: Reported | ParsedFromText | InferredFromInterpretation | Unknown`, `stdout`, `stderr` (only from the object), `combined` (content text), `interrupted`, `timed_out_after_ms`, `background_task_id`, `persisted_output{path,size}`, `return_code_interpretation`, `git_operation`, `bash_edit_diff`, `no_output_expected`, `is_image`. Input: `command`, `description`, `timeout_ms`, `run_in_background`, `sandbox_disabled`; no `cwd` on the call, take it from the surrounding line `cwd`. Background shells need a separate `BackgroundTask{id, tool_use_id, output_file, status, exit_code, completed_at}` assembled from `backgroundTaskId` + later `task-notification` + `TaskOutput`/`TaskStop` calls.
9. **Call status.** `ok/error/rejected/interrupted/pending` is almost enough; add `Denied{kind: permission-rule|automode-blocked|...}` (distinct from user rejection, with `toolDenialKind`), `Cancelled` (sibling failed), `Backgrounded`, `SessionEnded` (unknown outcome), and `rejected.feedback: Option<String>`.
10. **Typed inputs** to add: `Skill{skill,args}`, `ToolSearch{query,max_results}` (with `tool_reference` outputs), `AskUserQuestion{questions, answers}`, `Plan{enter|exit, plan, file}`, `Task*` (task list ops), `SendMessage{to, summary, message}`, `Monitor`, `Workflow`, `Worktree{enter|exit}`, `NotebookEdit`. `Edit` output needs `structured_patch` hunks and `user_modified`; `Write` needs `create|update`; `Read` output needs `kind: text|image|pdf|notebook|unchanged` with `start_line,num_lines,total_lines,truncated`. `Mcp{server,tool}` also needs `raw_server` and typed content blocks result.
11. **Events missing.** `Hook{event, name, command, exit_code, duration, stdout, stderr, blocking}` (hook_success/_error/_cancelled + stop_hook_summary), `TurnDuration{duration_ms, message_count}`, `ApiRetry{attempt,max,retry_in_ms,status}`, `ModelFallback{from,to,reason}` and `Refusal{category,explanation,retracted_uuids}`, `RateLimit/Quota`, `PermissionModeChange`, `QueuedInput{op,text,ts,reason}`, `AwaySummary`, `LocalCommand{command,args,output}`, `ScheduledFire`, `PrLink`, `FileSnapshot` (file-history), `WorktreeEnter/Exit/Relocate`, `ContextInjection{kind, text}` for the 53 attachment types (with `rendered` text when present), `FileMention{path, content}`, `TaskListState`, `Title{source: ai|custom|agent}`. A catch-all `Unknown{type, subtype, raw}` is required (new types appear each release).
12. **Compaction.** `Compaction{summary, trigger, tokens_before/after}` is right; add `duration_ms`, `preserved_segment{head,tail,anchor}`, `dropped_tokens`, `discovered_tools`, and note that the summary is in the NEXT user line (`isCompactSummary`), and the boundary has `parent_id = None` plus `logical_parent_id`. Pre-compaction entries remain valid history; model them with a `compaction_epoch` counter so consumers can reconstruct "what the model saw".
13. **Graph.** `parent_id` is not a list: keep it, but ordering must come from file order; expose `fork_group`/`branch` detection separately because 5,164 of 17,883 files have forks and 95 have dangling parents.
14. **Raw fidelity.** Every Entry/Part needs `raw: Option<serde_json::Value>` or an `extras` map for the unmodelled keys (permissionDecision, wire*, attribution*, session_id, etc.) so nothing is dropped.
15. **Identity fields** to add to Session: `entrypoint` (cli/sdk-cli/sdk-ts/claude-desktop), `permission_mode`, `slug`, `effort`, `sessionKind`, `worktree{...}`, `continued_from/in`, `bridge_session_id`, `pr_links[]`, `kind: Main|Subagent|WorkflowAgent`.
16. **Large payloads.** Offer lazy/streaming access to bulky fields (`images`, `Read` content, `originalFile`, `persisted` files on disk) so a session of 6 MB lines does not blow memory; `Image{media_type, bytes_len, data: Lazy}`.

## 15. Open questions (not determined from this corpus)

- Exact semantics of `wireToolInputs` (it equals `content[].input` in the samples checked; the difference may be post-hook rewrite).
- Whether `attributionMcpTool` is "last MCP tool used" or "tool in scope"; it was wrong for the call on the same line.
- Pre-2.1.143 shapes (`summary` lines, `progress` lines, `Task`-era results, `todos` key in TodoWrite result): need an older corpus.
- Meaning of `atis-latch`, `classifierBoundary`, `turnCompanion`, `serverClassifier*`: internal, left in extras.

## 16. Appendix: structure-only examples (values replaced by placeholders)

Assistant chunk (one block per line; same `message.id` repeats on the sibling lines):
```json
{"parentUuid":"<uuid>","isSidechain":false,"type":"assistant","uuid":"<uuid>","timestamp":"<iso>","requestId":"req_<id>","apiBlockIndex":1,
 "message":{"id":"msg_<id>","type":"message","role":"assistant","model":"claude-<model>","stop_reason":"tool_use","stop_sequence":null,"stop_details":null,
  "content":[{"type":"tool_use","id":"toolu_<id>","name":"Bash","input":{"command":"<cmd>","description":"<text>","timeout":120000},"caller":{"type":"direct"}}],
  "usage":{"input_tokens":6,"output_tokens":113,"cache_creation_input_tokens":0,"cache_read_input_tokens":9991,
           "cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens_details":{"thinking_tokens":0},
           "server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","speed":"standard","inference_geo":"global","iterations":[{"type":"message","input_tokens":6,"output_tokens":113}]}},
 "userType":"external","entrypoint":"cli","cwd":"/path/to/proj","sessionId":"<uuid>","version":"2.1.x","gitBranch":"main"}
```
Thinking block: `{"type":"thinking","thinking":"","signature":"<opaque>"}`.

Bash tool_result, success and failure:
```json
{"type":"user","parentUuid":"<assistant line uuid>","sourceToolAssistantUUID":"<same>","promptId":"<uuid>","uuid":"<uuid>","timestamp":"<iso>",
 "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_<id>","content":"<stdout>","is_error":false}]},
 "toolUseResult":{"stdout":"<stdout>","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false}}
{"type":"user","...":"...",
 "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_<id>","content":"Exit code 1\n<combined output>","is_error":true}]},
 "toolUseResult":"Error: Exit code 1\n<combined output>"}
```

Edit result:
```json
{"toolUseResult":{"filePath":"/path/to/file","oldString":"<text>","newString":"<text>","originalFile":null,
  "structuredPatch":[{"oldStart":125,"oldLines":7,"newStart":125,"newLines":7,"lines":[" <ctx>","-<old>","+<new>"]}],"userModified":false,"replaceAll":false}}
```

Read result (text):
```json
{"toolUseResult":{"type":"text","file":{"filePath":"/path/to/file","content":"<raw text>","numLines":110,"startLine":120,"totalLines":830}}}
```

Rejection:
```json
{"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_<id>","content":"The user doesn't want to proceed with this tool use. ...","is_error":true}]},
 "toolUseResult":"User rejected tool use","toolDenialKind":"user-rejected"}
{"type":"user","interruptedMessageId":"msg_<id>","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}
```

Compaction triple:
```json
{"type":"system","subtype":"compact_boundary","parentUuid":null,"logicalParentUuid":"<uuid>","content":"Conversation compacted","level":"info","isMeta":false,
 "compactMetadata":{"trigger":"auto","preTokens":177627,"postTokens":3771,"durationMs":124704,"preCompactDiscoveredTools":["<tool>"],
                    "preservedSegment":{"headUuid":"<uuid>","tailUuid":"<uuid>","anchorUuid":"<uuid>"},"preservedMessages":{"anchorUuid":"<uuid>","uuids":["<uuid>"],"allUuids":["<uuid>"]}}}
{"type":"user","parentUuid":"<boundary uuid>","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"role":"user","content":"This session is being continued from a previous conversation ... <summary>"}}
{"type":"attachment","parentUuid":"<summary uuid>","attachment":{"type":"compact_file_reference","filename":"/path/to/file","displayPath":"<rel>"}}
```

Subagent first line and meta:
```json
{"parentUuid":null,"isSidechain":true,"agentId":"<hex17>","promptId":"<uuid>","type":"user","sessionId":"<PARENT session uuid>","message":{"role":"user","content":"<task prompt>"}}
{"agentType":"Explore","description":"<text>","toolUseId":"toolu_<id of parent Agent call>","spawnDepth":1,"parentAgentId":"<hex17, nested only>","model":"sonnet"}
```

Queue operation and task notification:
```json
{"type":"queue-operation","operation":"enqueue","timestamp":"<iso>","sessionId":"<uuid>","content":"<text typed while busy>"}
{"type":"user","origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification>\n<task-id>..</task-id>\n<tool-use-id>toolu_..</tool-use-id>\n<output-file>/path</output-file>\n<status>completed</status>\n<summary>..</summary>\n</task-notification>"}}
```

Hook attachment:
```json
{"type":"attachment","attachment":{"type":"hook_success","hookName":"PreToolUse:Bash","hookEvent":"PreToolUse","toolUseID":"toolu_<id>","command":"/path/to/hook.sh","stdout":"<json>","stderr":"","exitCode":0,"durationMs":49,"content":""}}
```

Synthetic API error:
```json
{"type":"assistant","isApiErrorMessage":true,"error":"rate_limit","apiErrorStatus":429,"requestId":"req_<id>",
 "message":{"id":"<uuid>","model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"<error text>"}],"usage":{"input_tokens":0,"output_tokens":0,"service_tier":null}}}
```
