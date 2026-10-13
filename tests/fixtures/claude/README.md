# Claude Code fixtures

Small synthetic transcripts for the Claude Code parser. Every value is fake: prompts like "fix the failing
test", paths under `/work/demo`, uuid shaped ids with a recognisable prefix, `msg_01Demo...` message ids padded
with `x`, `toolu_01Demo...` tool ids, short fake base64 signatures. Envelope key names, key order, line
splitting (one content block per line, several lines sharing one `message.id`), `message` shapes and
`toolUseResult` shapes follow measurements on 17,826 real transcripts (CLI 2.1.143 to
2.1.296) and the two fresh real sessions described in `VERIFICATION.md` (CLI 2.1.296: Bash exit 3, Read, Edit). No real
prompt, code, path, username or hostname was copied. Every line parses with `jq` except the two deliberately
broken lines in `fork-and-corrupt.jsonl`.

```
tests/fixtures/claude/
  main-session.jsonl                                          91 lines, CLI 2.1.296 shapes
  main-session/subagents/agent-a1b2c3d4e5f607182.jsonl        5 lines, the subagent transcript
  main-session/subagents/agent-a1b2c3d4e5f607182.meta.json    sidecar (one JSON object, not JSONL)
  old-format.jsonl                                            20 lines, RECONSTRUCTED, not measured
  fork-and-corrupt.jsonl                                      12 lines, 2 of them broken on purpose
```

`main-session.jsonl` is over the 80 line guideline on purpose: the brief needs about 35 distinct shapes and
every tool call costs two lines.

## Conventions used in the tables

- "Line" is the 1 based line number in the file. `L8` means line 8.
- "Human prompt" means a `user` line that is not a `tool_result` carrier, not `isMeta`, not `isCompactSummary`,
  has `origin.kind` absent or `human`, is not a `<task-notification>`, `<command-name>`, `<local-command-stdout>`,
  `<bash-input>` or `<bash-stdout>` line, and is not an interrupt marker. Slash commands and `!` shell escapes
  are counted separately.
- "Assistant message" means one group of lines with the same `message.id`, in file order.
- Usage of a message is the usage of the LAST line of its group (equal to the max `output_tokens`). Never summed
  across the lines of a group.
- Session total, agentparse convention: input = `input_tokens`, output = `output_tokens`, reasoning =
  `output_tokens_details.thinking_tokens` (already inside output, never added again), cache_read =
  `cache_read_input_tokens`, cache_write = `cache_creation_input_tokens` (the 5m and 1h split is
  `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`). `<synthetic>` messages have zero
  usage, so including or excluding them does not change a total, but they are not model output.
- Tool status rules: `ok` = `is_error` false or absent; `error` = `is_error` true without a denial kind;
  `rejected` = `toolDenialKind` `user-rejected`; `interrupted` = `toolDenialKind` `interrupted`; `pending` = a
  `tool_use` with no `tool_result` anywhere in the file. The exit code exists only in the text, regex
  `^Exit code (\d+)\n`.
- Message ids are `msg_01DemoMainNN` followed by `x` up to 24 characters after `msg_`, for example
  `msg_01DemoMain01xxxxxxxxxxxxxx`. Tool ids are written out in full in the call table.

## Provenance, read this before trusting a shape

| Shape | Source |
|---|---|
| Envelope, assistant chunks, usage, Bash success, Bash exit 3, Read, Edit, prompt line, tool_result line, `permissionDecision`, `hook_success`, `last-prompt` | measured, key order copied from the fresh 2.1.296 sessions (`VERIFICATION.md`) |
| `Write` create, `WebFetch`, `WebSearch`, async `Agent` launch result, background Bash result, `compact_boundary`, compact summary, `compact_file_reference`, `api_error`, `turn_duration`, `stop_hook_summary`, `informational`, synthetic API error message, `file` and `total_tokens_reminder` attachments, `queue-operation`, `file-history-snapshot`, titles, `[Request interrupted ...]` lines, `<task-notification>`, `<bash-input>` and `<bash-stdout>`, `<command-name>` and `<local-command-*>` lines | shapes and key order taken from real samples in the corpus (CLI 2.1.156 to 2.1.296), content invented |
| `TaskCreate`, `TaskUpdate`, `TaskOutput` result JSON | from the research tool table. The human readable `content` text of these three results is invented |
| `Grep` and `Glob` results | RECONSTRUCTED. The current CLI answers `No such tool available` for both (search runs through Bash). Shapes are those of older builds: `{mode, filenames, numFiles}` and `{filenames, durationMs, numFiles, truncated}` |
| Early chunk usage (`output_tokens` 1, `output_tokens_details` null, `iterations` null) | modelled on the research ("early chunks carry a partial output_tokens, e.g. 1"); the exact null pattern is an assumption |
| Aborted message | `stop_reason` null on every line plus partial usage. The research lists an `isAbortedMidStream` key but not its position, so it is NOT emitted. A parser must infer the abort from `stop_reason: null` and the interrupt marker |
| `old-format.jsonl` | RECONSTRUCTED from upstream knowledge of pre 2.1.143 builds, which the research marks unverified |

## `main-session.jsonl` (CLI 2.1.296, entrypoint cli)

Session id `3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11` (also on every conversation line as `sessionId`), cwd
`/work/demo`, git branch `main`, version `2.1.296`, model `claude-sonnet-5-5` (`advisorModel`
`claude-opus-5-5`, `effort` medium). First timestamp `2026-10-12T09:00:00.100Z` (L2; L1 has none), last
`2026-10-12T09:00:59.200Z` (L88). Roots (`parentUuid` null): L2 and the compact boundary L83.

| Fact | Expected |
|---|---|
| Lines by type | 91 lines: `assistant` 31, `user` 38, `system` 7, `attachment` 5, `ai-title` 2, `last-prompt` 2, `queue-operation` 2, `permission-mode` 1, `atis-latch` 1, `file-history-snapshot` 1, `custom-title` 1 |
| Human prompts | 7. L3 `fix the failing test in @test.sh`, L33 `summarize the demo docs`, L44 `and track it as a task` (`promptSource` queued, typed while the agent was busy), L60 `clean the build directory`, L64 `leave it, just run cargo build`, L68 `run cargo test instead`, L86 `continue with the failing test` |
| Other user lines (31) | 18 `tool_result` carriers; 3 `isMeta` (L73 local command caveat, L77 skill body, L78 system reminder); 2 slash commands (L74 `/model`, L76 `/demo-skill` with args `triage the flaky test`); 2 command output lines (L75 `<local-command-stdout>Set model to Sonnet 5.5`, L81 `<bash-stdout>hi\n` with empty `<bash-stderr>`); 1 shell escape (L80 `<bash-input>echo hi`); 2 task notifications (L55 agent, L56 background shell, `origin.kind` task-notification); 2 interrupt markers (L67 `[Request interrupted by user for tool use]`, L72 `[Request interrupted by user]`); 1 compact summary (L84). 7 + 18 + 3 + 2 + 2 + 1 + 2 + 2 + 1 = 38 |
| Assistant messages after regrouping | 24: 23 model messages plus 1 `<synthetic>`. 31 assistant lines in total |
| Title | `custom-title` L90 `demo-fix-session` wins. `ai-title` L6 `Fix failing test` then L89 `Fix the failing demo test` (latest wins when there is no custom title) |
| Warnings expected | none, no corrupt lines |
| Session total (23 model messages) | input 50, output 1936, reasoning 40, cache_read 300418, cache_write 5796 (5m 200, 1h 5596). Subagent usage is NOT in this total, see the subagent section |
| Dedupe traps | message 1 (L8 to L10): early lines carry `output_tokens` 1 and `output_tokens_details` null, the final line carries 118 and `thinking_tokens` 40. Message 3 (L16 to L17): both lines carry the full usage and `stop_reason` tool_use (back filled style, equal after dedupe). Message 22 (L69 to L71): never finished |
| Graph | forks at L10 (children L11 hook attachment and L12 result) and at L26 (children L27 and L29), both from tool calls, not rewinds. No dangling parent. L12 result `parentUuid` and `sourceToolAssistantUUID` both equal the uuid of L10 |
| last-prompt | L5 and L91, `leafUuid` of L91 is the uuid of L88 |

Assistant messages (usage after dedupe, from the last line of each group):

| # | message.id | lines | blocks | stop_reason | input | output | reasoning | cache_read | cache_write (5m / 1h) |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `msg_01DemoMain01` | 8,9,10 | thinking+text+tool_use | tool_use | 6 | 118 | 40 | 9991 | 1200 (200 / 1000) |
| 2 | `msg_01DemoMain02` | 14 | tool_use | tool_use | 2 | 87 | 0 | 10191 | 1431 (0 / 1431) |
| 3 | `msg_01DemoMain03` | 16,17 | text+tool_use | tool_use | 2 | 118 | 0 | 11622 | 136 (0 / 136) |
| 4 | `msg_01DemoMain04` | 19 | tool_use | tool_use | 2 | 171 | 0 | 11758 | 161 (0 / 161) |
| 5 | `msg_01DemoMain05` | 22 | tool_use | tool_use | 2 | 90 | 0 | 11919 | 150 (0 / 150) |
| 6 | `msg_01DemoMain06` | 24 | tool_use | tool_use | 2 | 140 | 0 | 12069 | 120 (0 / 120) |
| 7 | `msg_01DemoMain07` | 26,27 | tool_use+tool_use | tool_use | 2 | 160 | 0 | 12189 | 210 (0 / 210) |
| 8 | `msg_01DemoMain08` | 30 | text | end_turn | 2 | 69 | 0 | 12399 | 98 (0 / 98) |
| 9 | `msg_01DemoMain09` | 34 | tool_use | tool_use | 2 | 95 | 0 | 12600 | 300 (0 / 300) |
| 10 | `msg_01DemoMain10` | 37 | tool_use | tool_use | 2 | 72 | 0 | 12900 | 180 (0 / 180) |
| 11 | `msg_01DemoMain11` | 39 | tool_use | tool_use | 2 | 64 | 0 | 13100 | 90 (0 / 90) |
| 12 | `msg_01DemoMain12` | 41 | text | end_turn | 2 | 55 | 0 | 13300 | 60 (0 / 60) |
| 13 | `msg_01DemoMain13` | 45 | tool_use | tool_use | 2 | 80 | 0 | 13400 | 120 (0 / 120) |
| 14 | `msg_01DemoMain14` | 47 | tool_use | tool_use | 2 | 60 | 0 | 13500 | 90 (0 / 90) |
| 15 | `msg_01DemoMain15` | 49 | tool_use | tool_use | 2 | 110 | 0 | 13600 | 140 (0 / 140) |
| 16 | `msg_01DemoMain16` | 51 | tool_use | tool_use | 2 | 98 | 0 | 13750 | 110 (0 / 110) |
| 17 | `msg_01DemoMain17` | 53 | text | end_turn | 2 | 40 | 0 | 13860 | 50 (0 / 50) |
| 18 | `msg_01DemoMain18` | 57 | tool_use | tool_use | 2 | 66 | 0 | 13950 | 700 (0 / 700) |
| 19 | `msg_01DemoMain19` | 59 | text | end_turn | 2 | 52 | 0 | 14650 | 60 (0 / 60) |
| 20 | `msg_01DemoMain20` | 61,62 | text+tool_use | tool_use | 2 | 75 | 0 | 14710 | 130 (0 / 130) |
| 21 | `msg_01DemoMain21` | 65 | tool_use | tool_use | 2 | 70 | 0 | 14850 | 100 (0 / 100) |
| 22 | `msg_01DemoMain22` | 69,70,71 | thinking+text+tool_use | null | 2 | 1 | - | 15010 | 90 (0 / 90) |
| 23 | `msg_01DemoMain23` | 79 | text | end_turn | 2 | 45 | 0 | 15100 | 70 (0 / 70) |
| 24 | `5a0e7c1d-2b4f-4c3a-9d6e-7f8a9b0c1d2e` | 88 | text | stop_sequence | 0 | 0 | - | 0 | 0 (0 / 0) |

Message 22 is the aborted stream: `stop_reason` null on all three lines, output 1 (partial), no
`output_tokens_details`; it still counts in the total with output 1, and its complete Bash `tool_use` (L71) has
no result. Message 24 is the synthetic API error: UUID shaped id, model `<synthetic>`, `isApiErrorMessage`
true, `error` `rate_limit`, `apiErrorStatus` 429, text `You've hit your session limit · resets 11pm` (contains
a non ASCII middle dot), usage all 0, `stop_reason` `stop_sequence`.

Tool calls, 19 in total: 14 ok (one of them backgrounded), 2 error, 1 rejected, 1 interrupted, 1 pending. 18
`tool_result` blocks. Pair by id, never by position.

| # | Use | Result | Name | Tool id | Status | Exit | Recorded output and notes |
|---|---|---|---|---|---|---|---|
| 1 | L10 | L12 | Bash | `toolu_01DemoBash01xxxxxxxxxxxx` | ok | none | `toolUseResult` object: `stdout` is the 3 line `ls -la` text, `stderr` "", `interrupted` false, `isImage` false, `noOutputExpected` false. A `PreToolUse:Bash` `hook_success` (L11, command `/work/demo/.claude/hooks/pre-bash.sh`, `exitCode` 0, `durationMs` 49, `stderr` "") has `toolUseID` equal to this id |
| 2 | L14 | L15 | Bash | `toolu_01DemoBash02xxxxxxxxxxxx` | error | 3 | `is_error` true, content `Exit code 3\nout-line\nerr-line`, `toolUseResult` is the STRING `Error: Exit code 3\nout-line\nerr-line`. stdout and stderr are merged, there is no stderr field |
| 3 | L17 | L18 | Read | `toolu_01DemoRead03xxxxxxxxxxxx` | ok | none | `toolUseResult` `{type text, file {filePath /work/demo/a.txt, content "one\ntwo\nthree\n", numLines 4, startLine 1, totalLines 4}}`. `message.content` is the `cat -n` text `1\tone\n2\ttwo\n3\tthree\n4\t`, different from `file.content`. Result has no `is_error` key |
| 4 | L19 | L20 | Edit | `toolu_01DemoEdit04xxxxxxxxxxxx` | ok | none | `oldString` `two\nthree`, `newString` `two edited\nthree`, `originalFile` `one\ntwo\nthree\n`, one hunk `{oldStart 1, oldLines 3, newStart 1, newLines 3, lines [" one","-two","+two edited"," three"]}`, `userModified` false, `replaceAll` false. L21 `file-history-snapshot` follows |
| 5 | L22 | L23 | Bash | `toolu_01DemoBash05xxxxxxxxxxxx` | error | 143 | input `timeout` 120000, content `Exit code 143\nCommand timed out after 2m 0s`, `toolUseResult` string `Error: Exit code 143\nCommand timed out after 2m 0s`. A timeout is only detectable from the text |
| 6 | L24 | L25 | Write | `toolu_01DemoWrite06xxxxxxxxxxx` | ok | none | `toolUseResult` `{type create, filePath /work/demo/notes.md, content, structuredPatch [], originalFile null, userModified false}` |
| 7 | L26 | L29 | Grep | `toolu_01DemoGrep07xxxxxxxxxxxx` | ok | none | RECONSTRUCTED shape `{mode files_with_matches, filenames [/work/demo/test.sh, /work/demo/notes.md], numFiles 2}`. Parallel pair with #8, its result arrives second |
| 8 | L27 | L28 | Glob | `toolu_01DemoGlob08xxxxxxxxxxxx` | ok | none | RECONSTRUCTED shape `{filenames [/work/demo/test.sh], durationMs 12, numFiles 1, truncated false}`. Its result arrives FIRST (L28 before L29), each result on its own `user` line |
| 9 | L34 | L35 | WebFetch | `toolu_01DemoWebFetch09xxxxxxxx` | ok | none | `{bytes 3396, code 200, codeText OK, result, durationMs 1288, url https://example.com/docs}` |
| 10 | L37 | L38 | WebSearch | `toolu_01DemoWebSearch10xxxxxxx` | ok | none | `{query, results [{tool_use_id srvtoolu_01DemoSearch000000000, content [{title,url}, {title,url}]}, "Synthetic search summary."], durationSeconds 4.5, searchCount 1}`. `results` mixes an object and a string |
| 11 | L39 | L40 | mcp__demo__search | `toolu_01Demomcpdemosearch11xxx` | ok | none | server `demo`, tool `search`. `toolUseResult` is a LIST `[{"type":"text","text":"[{...}]"}]`, `tool_result.content` is the same list, no `permissionDecision` key |
| 12 | L45 | L46 | TaskCreate | `toolu_01DemoTaskCreate12xxxxxx` | ok | none | `{task {id "1", subject "Check demo docs"}}` |
| 13 | L47 | L48 | TaskUpdate | `toolu_01DemoTaskUpdate13xxxxxx` | ok | none | `{success true, taskId "1", updatedFields ["status"], statusChange {from pending, to completed}}` |
| 14 | L49 | L50 | Agent | `toolu_01DemoAgent14xxxxxxxxxxx` | ok | none | async launch: input `run_in_background` true, `subagent_type` general-purpose; `toolUseResult` `{isAsync true, status async_launched, agentId a1b2c3d4e5f607182, description Survey demo tests, resolvedModel, prompt, outputFile, canReadOutputFile, canContinueAgent}`. See "Subagent link" |
| 15 | L51 | L52 | Bash | `toolu_01DemoBash15xxxxxxxxxxxx` | ok (backgrounded) | none | input `run_in_background` true, command `sleep 2; echo bg-done`; content `Command running in background with ID: bk7d2m9qa. Output is being written to: /tmp/claude-1000/-work-demo/<session id>/tasks/bk7d2m9qa.output. ...`; `toolUseResult.backgroundTaskId` `bk7d2m9qa`, `stdout` "", `stderr` "". Background id `bk7d2m9qa` |
| 16 | L57 | L58 | TaskOutput | `toolu_01DemoTaskOutput16xxxxxx` | ok | none | poll of the background shell. Input `{task_id bk7d2m9qa, block true, timeout 30000}`; result `{retrieval_status success, task {task_id bk7d2m9qa, task_type local_bash, status completed, description, output "bg-done\n", exitCode 0}}`. L56 is the `<task-notification>` for the same id (`<tool-use-id>` = #15, `<status>completed</status>`, summary `Background command "sleep 2; echo bg-done" completed (exit code 0)`) |
| 17 | L62 | L63 | Bash | `toolu_01DemoBash17xxxxxxxxxxxx` | rejected | none | `is_error` true, content is the fixed text `The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.`, `toolUseResult` string `User rejected tool use`, `toolDenialKind` `user-rejected`, no `permissionDecision`. Command `rm -rf build` |
| 18 | L65 | L66 | Bash | `toolu_01DemoBash18xxxxxxxxxxxx` | interrupted | none | content and `toolUseResult` both `[Request interrupted by user for tool use]`, `toolDenialKind` `interrupted`. L67 is the text marker, `interruptedMessageId` = `msg_01DemoMain21xxxxxxxxxxxxxx` (the message of #18). Command `cargo build` |
| 19 | L71 | none | Bash | `toolu_01DemoBash19xxxxxxxxxxxx` | pending | none | complete `tool_use` (`cargo test`) inside the aborted message 22, never answered. L72 `[Request interrupted by user]` has `interruptedMessageId` `msg_01DemoMain22xxxxxxxxxxxxxx` |

Other facts:

| Fact | Expected |
|---|---|
| Compaction | L83 `system/compact_boundary`: `parentUuid` null, `logicalParentUuid` `c0de0000-0a0a-4b0b-8c0c-000000000058` (the uuid of L82, the last line before the boundary), `content` `Conversation compacted`, `level` info, `compactMetadata` `{trigger auto, preTokens 177627, postTokens 3771, cumulativeDroppedTokens 173856, durationMs 124704, preservedSegment {headUuid = uuid of L72, anchorUuid = uuid of L84, tailUuid = uuid of L82}, preservedMessages {anchorUuid = uuid of L84, uuids [L72, L82], allUuids [L72, L82]}}`. L84 is the summary: `isCompactSummary` true, `isVisibleInTranscriptOnly` true, `parentUuid` = boundary uuid, string content starting `This session is being continued from a previous conversation that ran out of context.`. L85 `attachment/compact_file_reference` (`/work/demo/a.txt`, `a.txt`). Lines before L83 stay in the file, they are still history |
| System lines | L31 `stop_hook_summary` (`hookCount` 1, `hookInfos` [{command /work/demo/.claude/hooks/stop.sh, durationMs 404}], `hookErrors` [], `level` suggestion, `toolUseID` is NOT a tool_use id); L32, L43, L54 `turn_duration` (`durationMs` 41250, 18800, 9100; `messageCount` 24, 9, 7; `isMeta` false); L82 `informational` (`level` notice, `content` `agents-md: no CLAUDE.md found; AGENTS.md`); L83 `compact_boundary`; L87 `api_error` (`level` error, `error.status` 529, `error.formatted` `529 Overloaded`, `retryAttempt` 1, `maxRetries` 10, `retryInMs` 569.1047397886507, `error.requestId` `req_011DemoOverloaded0000x`). L87 is the parent of the synthetic message L88 |
| Attachments (5) | L2 `hook_success` SessionStart:startup (`hookEvent` SessionStart, `command` `/work/demo/.claude/hooks/session-start.sh`, `stdout` and `content` `demo session context loaded`, `stderr` "", `exitCode` 0, `durationMs` 120, `rendered` has one system reminder); L4 `file` mention (`filename` `/work/demo/test.sh`, `displayPath` `test.sh`, `content.file` numLines 4 startLine 1 totalLines 4, two `rendered` entries, `renderedRole` system); L11 `hook_success` PreToolUse:Bash (see call 1); L13 `total_tokens_reminder` (`text` `<total_tokens>148000 tokens left</total_tokens>`); L85 `compact_file_reference` |
| Queue pair | L36 `queue-operation` enqueue, `content` `and track it as a task`, 2026-10-12T09:00:18.900Z; L42 dequeue, no content, 09:00:22.200Z. After the dequeue the text becomes its own turn, the human prompt at L44 (`promptSource` queued) |
| File history | L21 `file-history-snapshot`, `messageId` and `snapshot.messageId` = uuid of L3, `isSnapshotUpdate` true, `trackedFileBackups` has one dynamic key `/work/demo/a.txt` `{backupFileName 9f3c1a2b4d5e6f70@v1, version 1, backupTime, realParentDir}` |
| Ignorable lines | L1 `permission-mode` (default), L7 `atis-latch`, L5 and L91 `last-prompt`, L6 and L89 `ai-title`, L90 `custom-title`: none has a top level `uuid`, `parentUuid` or `timestamp` |
| Slash command and shell escape | L73 `isMeta` caveat, L74 `<command-name>/model</command-name>` with `<command-message>model` and `<command-args>sonnet`, L75 `<local-command-stdout>Set model to Sonnet 5.5</local-command-stdout>`. L76 `/demo-skill` (args `triage the flaky test`), L77 `isMeta` list content starting `Base directory for this skill: /work/demo/.claude/skills/demo-skill`, L78 `isMeta` `<system-reminder>` string, L79 the answer. L80 `<bash-input>echo hi</bash-input>`, L81 `<bash-stdout>hi\n</bash-stdout><bash-stderr></bash-stderr>` |
| Subagent link | `Agent` tool_use id (#14, L49) = `meta.toolUseId` in `main-session/subagents/agent-a1b2c3d4e5f607182.meta.json` = `<tool-use-id>` in the L55 notification. L50 `toolUseResult.agentId` = L55 `<task-id>` = the file stem after `agent-` = `agentId` on every subagent line. Subagent timestamps 09:00:29.000Z to 09:00:31.200Z sit between L50 and L55 |

## `main-session/subagents/agent-a1b2c3d4e5f607182.jsonl`

Directory layout mirrors `projects/<cwd>/<sessionId>/subagents/`. Here the "session" directory is named
`main-session` (the real name would be the session uuid). `sessionId` on every line is the PARENT session id
`3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11`, not a session of its own.

| Fact | Expected |
|---|---|
| Lines | 5: L1 user prompt, L2 thinking, L3 Bash tool_use, L4 tool_result, L5 final text. Every line `isSidechain` true and carries `agentId` `a1b2c3d4e5f607182`. L1 `parentUuid` null (a root, not a new conversation) |
| Human prompts | 1 (L1, string content `List the test files in /work/demo and report how many there are.`, identical to the parent `Agent` input `prompt`). No `origin` key on this line |
| Assistant messages | 2. `msg_01DemoSub01` (L2 thinking with partial usage, L3 tool_use with final usage: input 4, output 95, reasoning 20, cache_read 5000, cache_write 300 = 0 5m + 300 1h, stop_reason tool_use); `msg_01DemoSub02` (L5: input 2, output 30, cache_read 5300, cache_write 60 all 1h, end_turn). Assistant lines carry `attributionAgent` general-purpose |
| Subagent total | input 6, output 125, reasoning 20, cache_read 10300, cache_write 360 (1h 360). Not part of the parent total; parent plus subagent would be input 56, output 2061, reasoning 60, cache_read 310718, cache_write 6156 |
| Tool call | 1: `Bash` `toolu_01DemoSubBash01xxxxxxxxx` (L3 to L4), command `ls /work/demo/tests`, ok, no exit code, `stdout` `a_test.rs\nb_test.rs\nc_test.rs`, `stderr` "" |
| Final answer | `The demo project has 3 test files.`, same text as the `<result>` in the parent notification L55 |
| Sidecar meta | `{agentType general-purpose, description Survey demo tests, toolUseId toolu_01DemoAgent14xxxxxxxxxxx, spawnDepth 1, requestShape background, requestNonInteractive true, model sonnet}` |
| Warnings | none |

## `old-format.jsonl` (RECONSTRUCTED, version `2.0.31`, not verified against a real older corpus)

Shapes follow upstream knowledge of builds older than the measured corpus: envelope key order `parentUuid,
isSidechain, userType, cwd, sessionId, version, gitBranch, type, message, uuid, timestamp` (assistant has
`requestId` before `uuid`, tool results have `toolUseResult` after `timestamp`), no `caller` on `tool_use`, no
`cache_creation` split, no `output_tokens_details`, `service_tier` only in usage, `summary` lines, `progress`
lines, inline sidechain lines. Treat every fact here as "what the parser must tolerate", not as proof of what
Claude Code wrote. Session id `9d41c7a2-5e3b-4f60-8a17-2c9b6d0e4f35`, model `claude-sonnet-4-5-20250929`.

| Fact | Expected |
|---|---|
| Lines by type | 20: `summary` 2 (L1, L2), `user` 8, `assistant` 7, `progress` 2 (L9, L10), `system` 1 (L19) |
| Leading `summary` lines | L1 `Fix failing test in demo project`, `leafUuid` `01d00000-0a0a-4b0b-8c0c-00000000ffff`; L2 `Demo project setup`, `leafUuid` `...fffe`. Neither leaf exists in the file (they point into previous sessions). No uuid, parentUuid or timestamp on these lines. Title if derived: latest or first summary text, parser choice |
| Human prompts | 1 (L3 `fix the failing test`). L16 is a sidechain prompt, not counted |
| Assistant messages after regrouping | 5 on the main chain: L4, L7 and L8 (one message over two lines), L12, L14, L20. Plus 1 sidechain message (L17). Lines holding several blocks: L4 (text, TodoWrite, LS) and L14 (MultiEdit, Task) |
| Per message usage (main chain) | see table below. `msg_01OldMsgTwo` L7 has partial `output_tokens` 8, L8 has 55 |
| Session total (main chain only) | input 22, output 405, reasoning 0 (absent), cache_read 21700, cache_write 6050. Including the sidechain message (input 3, output 12, cache_write 800): input 25, output 417, cache_read 21700, cache_write 6850 |
| Inline sidechain | L16 (`isSidechain` true, `parentUuid` null, string prompt `Count the test files in /work/demo.`) and L17 (assistant `There are 3 test files.`). The `Task` result L18 hangs off L14 with `parentUuid` of the `Task` line |
| Progress and system | L9 `progress` `data.type` hook_progress (`hookEvent` PreToolUse, `hookName` PreToolUse:Bash, `parentToolUseID` = Bash tool id); L10 `progress` `data.type` bash_progress (`toolUseID` `bash-progress-0`, `elapsedTimeSeconds` 2, `totalLines` 1); L19 `system` with no `subtype`, `level` info, `toolUseID` = Task tool id. Progress and system lines are graph nodes, not messages |
| Warnings | none |

Tool calls (all 6 on the main chain):

| Use | Result | Name | Tool id | Status | Exit | Notes |
|---|---|---|---|---|---|---|
| L4 | L5 | TodoWrite | `toolu_01OldDemoTodoxxxxxxxxxxx` | ok | none | `toolUseResult` `{oldTodos [], newTodos [{content, status in_progress, priority high, id "1"}]}` |
| L4 | L6 | LS | `toolu_01OldDemoLSxxxxxxxxxxxxx` | ok | none | content and `toolUseResult` are both the string `- /work/demo/\n  - a.txt\n  - test.sh\n` |
| L8 | L11 | Bash | `toolu_01OldDemoBashxxxxxxxxxxx` | error | 3 | `is_error` true, content `Exit code 3\nout-line\nerr-line`, but `toolUseResult` here is an OBJECT `{stdout "out-line", stdoutLines 1, stderr "err-line", stderrLines 1, interrupted false, isImage false}`, so stderr is recoverable. Rule of thumb: object or string, accept both on error |
| L12 | L13 | Read | `toolu_01OldDemoReadxxxxxxxxxxx` | ok | none | `cat -n` text with 6 space padded numbers; `file` numLines 3, totalLines 3 |
| L14 | L15 | MultiEdit | `toolu_01OldDemoMultixxxxxxxxxx` | ok | none | input `edits` (2), result `{filePath, edits, originalFileContents, structuredPatch (1 hunk), userModified false}` |
| L14 | L18 | Task | `toolu_01OldDemoTaskxxxxxxxxxxx` | ok | none | input `{description Count tests, prompt, subagent_type general-purpose}`, result `{content [{type text, text}], totalDurationMs 4210, totalTokens 5120, totalToolUseCount 0, usage, wasInterrupted false}`, result content is a list of text blocks. Link to the sidechain L16 and L17 is by position and `isSidechain`, there is no agent id |

Assistant messages after regrouping:

| # | message.id | lines | blocks | stop_reason | input | output | reasoning | cache_read | cache_write (5m / 1h) |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `msg_01OldMsgOne` | 4 | text+tool_use+tool_use | tool_use | 4 | 120 | - | 0 | 5000 (- / -) |
| 2 | `msg_01OldMsgTwo` | 7,8 | text+tool_use | tool_use | 6 | 55 | - | 5000 | 300 (- / -) |
| 3 | `msg_01OldMsgThree` | 12 | tool_use | tool_use | 4 | 40 | - | 5300 | 200 (- / -) |
| 4 | `msg_01OldMsgFour` | 14 | tool_use+tool_use | tool_use | 4 | 150 | - | 5500 | 400 (- / -) |
| 5 | `msg_01OldMsgFive` | 20 | text | end_turn | 4 | 40 | - | 5900 | 150 (- / -) |

## `fork-and-corrupt.jsonl` (2.1.296 shapes, session `5c7e1b90-3d24-4a8f-b6c1-8e9f0a1b2c3d`)

Text only conversation. The last line has no trailing newline. 12 physical lines, 10 parse as JSON objects.

| Line | Content | uuid (last 4) | parentUuid (last 4) | timestamp |
|---|---|---|---|---|
| L1 | user `fix the failing test` | 0001 | null (root) | 10:00:02.000 |
| L2 | assistant `Looking at the test now.` | 0003 | 0001 | 10:00:03.000 |
| L3 | user `try approach one` | 0004 | 0003 | 10:00:05.000 |
| L4 | assistant `Approach one applied.` | 0006 | 0004 | 10:00:06.000 |
| L5 | user `actually try approach two instead` | 0007 | 0003 | 10:00:08.000 |
| L6 | assistant `Approach two applied.` | 0009 | 0007 | 10:00:09.000 |
| L7 | user `thanks, looks good` | 000a | 0009 | 09:15:09.000 (goes BACKWARDS, 45 minutes before L6) |
| L8 | assistant `Glad it helps.` | 000c | `dead` (full `f0f00000-0a0a-4b0b-8c0c-00000000dead`, not in the file) | 10:00:10.000 |
| L9 | `last-prompt`, `leafUuid` = uuid of L6 (`...0009`) | none | none | none |
| L10 | NOT JSON: `this line is not json: disk full while writing transcript` | | | |
| L11 | assistant `Anything else?` | 000d | 000c | 10:00:11.000 |
| L12 | TRUNCATED user line (cut inside the `content` string, no closing quote or brace, no newline at EOF) | | | |

All dates are 2026-10-12.

| Fact | Expected |
|---|---|
| Warnings | exactly 2: L10 not valid JSON, L12 truncated final line. Everything else parses. 10 parsed lines (4 user, 5 assistant, 1 `last-prompt`) |
| Human prompts | 4 (L1, L3, L5, L7). The truncated L12 is not counted |
| Assistant messages | 5 (L2, L4, L6, L8, L11), one line each |
| Fork | one: the uuid ending `0003` (L2) has TWO children, L3 and L5. L5 is the rewound branch. Branch A: L3, L4. Branch B: L5, L6, L7 |
| Current tip | `last-prompt.leafUuid` points at L6, so the branch the CLI considered current is L1, L2, L5, L6 (L7 hangs below it). Branch A (L3, L4) is abandoned. File order alone would pick L11 as the last entry, which is wrong |
| Dangling parent | L8 (`parentUuid` `f0f00000-0a0a-4b0b-8c0c-00000000dead`), and its descendant L11 forms an orphan subtree. A parser must keep them, not fail |
| Non monotonic timestamp | L7 is earlier than L6. Do not sort by timestamp, keep file order |
| Roots | one, L1 |
| Session total | input 15, output 95, reasoning 0, cache_read 10000, cache_write 500 (all 1h). All 5 messages count, including the abandoned branch and the dangling one, because all were billed |

Per message usage:

| # | message.id | lines | blocks | stop_reason | input | output | reasoning | cache_read | cache_write (5m / 1h) |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `msg_01Fork01` | 2 | text | end_turn | 3 | 20 | 0 | 2000 | 100 (0 / 100) |
| 2 | `msg_01Fork02` | 4 | text | end_turn | 3 | 25 | 0 | 2000 | 100 (0 / 100) |
| 3 | `msg_01Fork03` | 6 | text | end_turn | 3 | 30 | 0 | 2000 | 100 (0 / 100) |
| 4 | `msg_01Fork04` | 8 | text | end_turn | 3 | 12 | 0 | 2000 | 100 (0 / 100) |
| 5 | `msg_01Fork05` | 11 | text | end_turn | 3 | 8 | 0 | 2000 | 100 (0 / 100) |
