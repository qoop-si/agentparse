# OpenCode parser fixtures

All content is synthetic (paths under `/work/demo`, fake ids, placeholder prompts, fake `demo-sonnet` / `demo-gpt` models). The numbers below were computed by querying the built databases, not typed by hand. Build a database with `sqlite3 out.db < v1.sql` (or `v2.sql`).

| file | what it is |
|---|---|
| `v1.sql` | opencode 1.18.35 era database: `project`, `session`, `message`, `part`, `todo` populated; `session_v2` and `session_message` exist (DDL copied from a real 2.0.27 database) but are EMPTY, as in a real 1.18.x file |
| `v2.sql` | opencode 2.0.27 era database: same DDL, one MIGRATED session pair present in BOTH the v1 tables and `session_v2`, plus two native v2 sessions that exist only in `session_v2` |
| `storage/` | legacy JSON tree (opencode before 1.2): one session |

Conventions: timestamps are epoch ms. `message.data` and `part.data` omit `id`, `sessionID`, `messageID` (taken from the row columns). `session_message.data` omits `id` and `type`. Order messages by `time_created, id` (v1) or `seq` (v2). Order parts by `id`. Token `total` exists only on v1 rows; compute it as input + output + reasoning + cache.read + cache.write.

Shared project row: `project.id = a1b2c3d4e5f60718293a4b5c6d7e8f9012345678`, worktree `/work/demo`, vcs `git`, name `demo` (both SQL files).

## v1.sql

Detection: `session_v2` exists but has 0 rows, so a reader that lists `session_v2` first must fall back to the v1 tables for every session here.

Row counts: session 2, message 26, part 70, todo 3, session_v2 0, session_message 0.

Top level sessions (parent_id NULL) a UI should list: 1. The child is linked only by `parent_id` and the title suffix `(@general subagent)`.

#### Session (a): main

| field | value |
|---|---|
| session.id | `ses_ed65ba2ffffeDtcAUZWJkZu6qD` |
| title | `Fix the failing test` |
| parent_id | NULL |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| project_id | `a1b2c3d4e5f60718293a4b5c6d7e8f9012345678` |
| agent / model | build / `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| messages | 23 total: 8 user, 15 assistant |
| part types | agent x1, compaction x1, file x1, patch x1, reasoning x2, retry x1, step-finish x12, step-start x14, text x13, tool x17 |
| step-finish parts | 12 |
| usage total (session columns, equals sum of step-finish and of message.tokens) | input 14330, output 1365, reasoning 400, cache.read 14000, cache.write 950, sum 31045 |
| cost USD (session column, equals sum of messages) | 0.0179 |

Tool calls (message ordinal is by `time_created, id`, 1-based):

| msg # | tool | status | exit | notes |
|---|---|---|---|---|
| 2 | bash | completed | 0 |  |
| 3 | bash | completed | 2 | truncated, outputPath `/home/demo/.local/share/opencode/tool-output/tool_c0ffee000001` |
| 4 | read | completed | - |  |
| 4 | grep | completed | - |  |
| 4 | glob | completed | - |  |
| 5 | edit | completed | - | filediff +1/-1, diagnostics {} |
| 5 | write | completed | - | diagnostics has 1 error |
| 6 | apply_patch | completed | - | files: add:src/mul.ts(+1/-0), update:src/math.ts(+1/-1), delete:src/old.ts(+0/-1) |
| 9 | webfetch | completed | - |  |
| 9 | websearch | completed | - |  |
| 10 | todowrite | completed | - | 3 todos, title '2 todos' |
| 10 | task | completed | - | metadata.sessionId = `ses_ed65a5c6ffccKWy8YrFgn0VuXa` |
| 10 | demo-server_search | completed | - | MCP, server `demo-server`, tool `search` |
| 10 | invalid | completed | - | input {tool,error} |
| 15 | bash | error | - | interrupted=true, partial metadata.output |
| 17 | bash | pending | - | raw partial JSON, no time (killed run) |
| 19 | bash | completed | 0 | user-run `!cmd`, ulid callID, no step parts |

Assistant errors:

| msg # | error.name | data.message | statusCode | message.finish |
|---|---|---|---|---|
| 13 | APIError | Service unavailable | 503 | error |
| 15 | MessageAbortedError | The operation was aborted. | - | (absent) |

Retry part: message #9, attempt 1, error `APIError` status 529, isRetryable True.

Compaction: user message #20 holds a `compaction` part (auto=True, overflow=False, tail_start_id = message #18). Paired assistant message #21 has parentID = that user message, `summary: true`, agent `compaction`. Summary text (exact):

```text
## Goal
Fix the failing test in /work/demo.

## Done
- Fixed the sign in add
- Added mul

## Next
- Update docs
```

Synthetic user text: message #18 (`The following tool was executed by the user`), followed by assistant message #19 with ONE `bash` tool and no step-start/step-finish.

Assistant `finish` values: (absent) x3, error x1, stop x4, tool-calls x7.

Todo table for (a) (final state): [0] Fix add / completed / high; [1] Add mul tests / in_progress / medium; [2] Update docs / pending / low.

Other facts for (a): part types seen are text, reasoning (anthropic: text non-empty + `metadata.anthropic.signature`; openai: empty text + `metadata.openai.reasoningEncryptedContent`), step-start, step-finish, tool, patch (3 files), file (`data:image/png;base64,...`, filename `failure.png`), agent (`general`), retry, compaction. Message #4 has TWO steps (two step-finish parts); its `tokens` and `cost` are the sum of both. Messages with `error` have zero tokens. Session columns: `summary_files` 3, `summary_diffs` one FileDiff, `permission` three default denies, model column is the LAST model (`demo-gpt`).

#### Session (b): child (subagent)

| field | value |
|---|---|
| session.id | `ses_ed65a5c6ffccKWy8YrFgn0VuXa` |
| title | `Find failing assertions (@general subagent)` |
| parent_id | `ses_ed65ba2ffffeDtcAUZWJkZu6qD` |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| project_id | `a1b2c3d4e5f60718293a4b5c6d7e8f9012345678` |
| agent / model | build / `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| messages | 3 total: 1 user, 2 assistant |
| part types | step-finish x2, step-start x2, text x2, tool x1 |
| step-finish parts | 2 |
| usage total (session columns, equals sum of step-finish and of message.tokens) | input 1100, output 55, reasoning 0, cache.read 0, cache.write 0, sum 1155 |
| cost USD (session column, equals sum of messages) | 0.0 |

Tool calls (message ordinal is by `time_created, id`, 1-based):

| msg # | tool | status | exit | notes |
|---|---|---|---|---|
| 2 | grep | completed | - |  |

Assistant `finish` values: stop x1, tool-calls x1.

Link check: the `task` tool in (a) has `metadata.sessionId` = child id; child `parent_id` = (a) id; child title = `<task description> (@general subagent)`; child first user text = the task `prompt`.

## v2.sql

Row counts: v1 session 2, message 6, part 16; session_v2 4, session_message 29.

Rule to assert: session ids present in both `session` and `session_v2` are the MIGRATED ones; the parser must read them from the v1 tables. Sessions only in `session_v2` are read from `session_message`.

Ids in both tables: `ses_ed52a633ff93Y7Vatlwl1zyPKr`, `ses_ed52a75fff9cZv864nVGS5jPGC`.

### Migrated pair (read from the v1 tables)

#### Migrated main, v1 side (WINS)

| field | value |
|---|---|
| session.id | `ses_ed52a75fff9cZv864nVGS5jPGC` |
| title | `Update the readme` |
| parent_id | NULL |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| project_id | `a1b2c3d4e5f60718293a4b5c6d7e8f9012345678` |
| agent / model | build / `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| messages | 4 total: 1 user, 3 assistant |
| part types | reasoning x1, step-finish x3, step-start x3, text x3, tool x2 |
| step-finish parts | 3 |
| usage total (session columns, equals sum of step-finish and of message.tokens) | input 2400, output 130, reasoning 20, cache.read 0, cache.write 0, sum 2550 |
| cost USD (session column, equals sum of messages) | 0.0033 |

Tool calls (message ordinal is by `time_created, id`, 1-based):

| msg # | tool | status | exit | notes |
|---|---|---|---|---|
| 2 | bash | completed | 0 |  |
| 3 | task | completed | - | metadata.sessionId = `ses_ed52a633ff93Y7Vatlwl1zyPKr` |

Assistant `finish` values: stop x1, tool-calls x2.

#### Migrated child, v1 side (WINS)

| field | value |
|---|---|
| session.id | `ses_ed52a633ff93Y7Vatlwl1zyPKr` |
| title | `Summarize the changelog (@general subagent)` |
| parent_id | `ses_ed52a75fff9cZv864nVGS5jPGC` |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| project_id | `a1b2c3d4e5f60718293a4b5c6d7e8f9012345678` |
| agent / model | build / `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| messages | 2 total: 1 user, 1 assistant |
| part types | step-finish x1, step-start x1, text x2 |
| step-finish parts | 1 |
| usage total (session columns, equals sum of step-finish and of message.tokens) | input 300, output 10, reasoning 0, cache.read 0, cache.write 0, sum 310 |
| cost USD (session column, equals sum of messages) | 0.0004 |

Tool calls (message ordinal is by `time_created, id`, 1-based):

| msg # | tool | status | exit | notes |
|---|---|---|---|---|

Assistant `finish` values: stop x1.

#### Migrated main, v2 copy (must LOSE)

| field | value |
|---|---|
| session_v2.id | `ses_ed52a75fff9cZv864nVGS5jPGC` |
| title | `Update the readme` |
| parent_id | NULL |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| agent | build |
| model | `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| session_message rows | 5: assistant x3, system x1, user x1 |
| usage total (session columns) | input 2400, output 130, reasoning 20, cache.read 0, cache.write 0, sum 2550 |
| cost USD | 0.0033 |
| idle_outcome / time_idle | None / None |

Tool calls (seq of the owning row):

| seq | name | status | exit | notes |
|---|---|---|---|---|
| 1 | bash | completed | 0 | metadata.exit 0 |
| 2 | task | completed | - |  |

What differs from the v1 side, so a test can prove which source was used:

| assistant # | v1 time.completed | v2 time.completed (row time_updated, +2500) | v1 tokens.total | v2 tokens.total | v1 mode | v2 mode |
|---|---|---|---|---|---|---|
| 1 | 1791720002600 | 1791720005100 | 770 | absent | build | absent |
| 2 | 1791720007600 | 1791720010100 | 860 | absent | build | absent |
| 3 | 1791720010200 | 1791720012700 | 920 | absent | build | absent |

Also only in v1: `step-start`/`step-finish` parts (reason, per step usage), tool `title`, `state.output` (v2 has `content[]` text), user `agent`/`model`, assistant `path.cwd`, `parentID`. The v2 copy keeps v1 tool names (`bash`, `task`) and `metadata`, ends with a `system` row (the migration notice, seq 4), and its `model.variant` is `default`.

#### Migrated child, v2 copy (must LOSE)

| field | value |
|---|---|
| session_v2.id | `ses_ed52a633ff93Y7Vatlwl1zyPKr` |
| title | `Summarize the changelog (@general subagent)` |
| parent_id | `ses_ed52a75fff9cZv864nVGS5jPGC` |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 1.18.35 |
| agent | build |
| model | `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| session_message rows | 3: assistant x1, system x1, user x1 |
| usage total (session columns) | input 300, output 10, reasoning 0, cache.read 0, cache.write 0, sum 310 |
| cost USD | 0.0004 |
| idle_outcome / time_idle | None / None |

### Native v2 sessions (read from `session_message`)

#### Native main

| field | value |
|---|---|
| session_v2.id | `ses_ecd6ad17ff80AEbRxnOg8B21Mn` |
| title | `Add a retry helper` |
| parent_id | NULL |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 2.0.27 |
| agent | plan |
| model | `{"id":"demo-sonnet","providerID":"anthropic"}` |
| session_message rows | 18: agent-switched x1, assistant x6, compaction x1, idle x2, model-switched x1, shell x1, skill x1, synthetic x2, system x1, user x2 |
| usage total (session columns) | input 9350, output 1060, reasoning 150, cache.read 5700, cache.write 600, sum 16860 |
| cost USD | 0.0134 |
| idle_outcome / time_idle | failed / 1791850059900 |

Tool calls (seq of the owning row):

| seq | name | status | exit | notes |
|---|---|---|---|---|
| 4 | (shell row) | exited | 0 | user `!` command `git status --short`, output.size 14 |
| 5 | shell | completed | 0 | metadata.exit 0 |
| 5 | read | completed | - | input key `path` |
| 6 | edit | completed | - | files: modified:/work/demo/src/net.ts(+5/-0) |
| 6 | patch | completed | - | files: modified:/work/demo/src/index.ts(+1/-0), deleted:/work/demo/src/old.ts(+0/-1) |
| 7 | subagent | completed | - | metadata.status completed; metadata.sessionID `ses_ecd6a835ff7fukfa0R96znWxUu` |
| 7 | read | error | - | error.type tool.execution; input key `path` |
| 7 | shell | completed | - | metadata.status running, shellID sh_bg00000001 |

Idle row seq 10: outcome succeeded.

Row seq 12: `agent-switched` {"agent": "plan", "previous": "build"}

Row seq 13: `model-switched` {"model": {"id": "demo-sonnet", "providerID": "anthropic"}, "previous": {"id": "demo-gpt", "providerID": "openai", "variant": "default"}}

Compaction row seq 15: status completed, reason auto, usage {'input': 5000, 'output': 400, 'reasoning': 0, 'cache': {'read': 0, 'write': 0}}, cost 0.005. Summary (exact):

```text
## Goal
Add a retry helper to /work/demo.

## Done
- retry in src/net.ts

## Next
- README section
```

`recent` = `'user: now plan the docs\nassistant: Plan: update README.md with a retry section.'`

Assistant error row seq 16: content [], finish error, error type `provider.error` status 503, retry attempt 2.

Idle row seq 17: outcome failed, error type `provider.invalid-request` status 410 message `Model demo-sonnet has been deprecated.` response.body present.

Usage note: for native sessions the session columns equal the sum over `assistant` rows PLUS the `compaction` row usage (fixture choice, the upstream rule for compaction usage is unverified). Rows with `tokens` absent (the failed assistant row) add nothing. Session `agent`/`model` columns hold the LAST selection (`plan`, `demo-sonnet`), matching the `agent-switched` and `model-switched` rows.

Row details for native main, by seq:

| seq | type | details |
|---|---|---|
| 0 | user | text `add a retry helper to src/net.ts`; files[1] (mime image/png, source inline); agents ['general'] |
| 1 | synthetic | text `Reminder: keep changes small.` |
| 2 | system | notice, not user speech |
| 3 | skill | skill `demo-skill` |
| 4 | shell | `git status --short` status exited exit 0 |
| 5 | assistant | content reasoning/text/tool:shell/tool:read; finish tool-calls; tokens {"input": 1500, "output": 120, "reasoning": 60, "cache": {"read": 0, "write": 0}}; cost 0.0 |
| 6 | assistant | content tool:edit/tool:patch; finish tool-calls; tokens {"input": 400, "output": 260, "reasoning": 0, "cache": {"read": 1600, "write": 0}}; cost 0.0021 |
| 7 | assistant | content tool:subagent/tool:read/tool:shell; finish tool-calls; tokens {"input": 350, "output": 90, "reasoning": 0, "cache": {"read": 2000, "write": 0}}; cost 0.0018 |
| 8 | synthetic | text `<shell id="sh_bg00000001" state="completed" command="bun run build">\nbuild ok\n</shell>` (the form opencode 2.x writes), description `bun run build`; metadata source shell, shellID = jobID `sh_bg00000001`, state completed, exit 0, truncated false |
| 9 | assistant | content text; finish stop; tokens {"input": 300, "output": 40, "reasoning": 0, "cache": {"read": 2100, "write": 0}}; cost 0.0012 |
| 10 | idle |  |
| 11 | user | text `now plan the docs`; files[] |
| 12 | agent-switched |  |
| 13 | model-switched |  |
| 14 | assistant | content reasoning/text; finish stop; tokens {"input": 1800, "output": 150, "reasoning": 90, "cache": {"read": 0, "write": 600}}; cost 0.0033 |
| 15 | compaction |  |
| 16 | assistant | content ; finish error; tokens null; cost None |
| 17 | idle |  |

#### Native child (subagent)

| field | value |
|---|---|
| session_v2.id | `ses_ecd6a835ff7fukfa0R96znWxUu` |
| title | `Locate callers` |
| parent_id | `ses_ecd6ad17ff80AEbRxnOg8B21Mn` |
| directory (cwd) | `/work/demo` |
| path | `work/demo` |
| version | 2.0.27 |
| agent | general |
| model | `{"id":"demo-gpt","providerID":"openai","variant":"default"}` |
| session_message rows | 3: assistant x1, idle x1, user x1 |
| usage total (session columns) | input 500, output 40, reasoning 0, cache.read 0, cache.write 0, sum 540 |
| cost USD | 0.0006 |
| idle_outcome / time_idle | succeeded / 1791850022000 |

Tool calls (seq of the owning row):

| seq | name | status | exit | notes |
|---|---|---|---|---|
| 1 | grep | completed | - |  |

Idle row seq 2: outcome succeeded.

Link check: the `subagent` tool in native main has `metadata.sessionID` = native child id and `input.sessionID` = same; child `parent_id` = native main id; child title = `Locate callers` (opencode 2.x titles a child with the bare description; the `(@agent subagent)` suffix is the v1 convention, kept on the migrated pair). The child's agent type is only recorded in the spawning call's `input.agent` (`general`).

## storage/ (legacy JSON tree)

Project `e4d909c290d0fb1ca068ffaddf22cbd0cd8aaaaa` (`project/<id>.json`), worktree `/work/demo`. Session `ses_79a8c3d52ffeLegacyDemoSess01` at `session/e4d909c290d0fb1ca068ffaddf22cbd0cd8aaaaa/ses_79a8c3d52ffeLegacyDemoSess01.json`: title `Legacy fix the greeting`, directory `/work/demo`, version `0.15.31`, no `parentID`. Full objects WITH `id`, `sessionID`, `messageID`.

| role | message id | parts in id order |
|---|---|---|
| user | `msg_79a8c3e10001LegacyUserMsg001` | text |
| assistant | `msg_79a8c4210001LegacyAsstMsg001` | step-start, reasoning, text, tool:bash, tool:edit, step-finish |

Counts: 1 user, 1 assistant message. Tools: `bash` completed exit 0 (`cat hello.txt`), `edit` completed (`/work/demo/hello.txt`, diff in metadata, no filediff). Usage (step-finish, equals assistant `tokens`): input 900, output 140, reasoning 30, cache.read 1000, cache.write 200, cost 0.0123; no `total` key (compute it: 2270).

Other files: `todo/ses_79a8c3d52ffeLegacyDemoSess01.json` (1 completed item), `session_diff/ses_79a8c3d52ffeLegacyDemoSess01.json` (1 FileDiff `/work/demo/hello.txt` +1/-1), `migration` (content `2`).

