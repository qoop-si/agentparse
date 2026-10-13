# Code review: Claude Code module (P4)

Scope: `src/claude.rs` (1,592 lines), `src/claude/tools.rs` (540), `tests/claude.rs` (978, 15 tests), final
implementer version of 2026-10-13 06:07. Contract: plan v2.1 sections 2, 3, 4, 6, triage 1 and 2,
`research-claude.md`. Every finding below was reproduced on the real corpus (`~/.claude/projects`, 17,936
`.jsonl` files) unless marked unverified. Scripts: `scratchpad/review-claude/` (not in the repo). No real
prompt or code is quoted; sessions are named by an 8 character id prefix.

## What was verified and holds

| Check | Method | Result |
|---|---|---|
| Whole corpus parses | `agentparse stats --agent claude` | 17,922 sessions, 0 failures, 0 panics, 40 warnings, 11.3 GB in 70 s wall, peak RSS 289 MB |
| Largest file (203 MB, 27,205 lines) | `show --json`, `/usr/bin/time -l` | 0.46 s, 142 MB RSS |
| Regrouping by `message.id`, order, usage from the last line | independent raw scan vs `show --json`, 400 random files (250 main, 150 sub) | 0 order/count mismatches, 0 per-message usage mismatches |
| tool_use count | same 400 files | 45,065 raw = 45,065 parsed |
| Human prompt count (qoop-history rules + human `queued_command`) | same 400 files | 3,632 raw = 3,632 parsed |
| No-result calls | stats | 42 pending, 767 interrupted (16 of them wrong, finding 1) |
| `<synthetic>` "No response requested." | 154 real lines | all `isApiErrorMessage:false`, so `Notice{synthetic}` is right |
| `bashEditDiff` hunks | 7,438 results, 10,906 hunks | all `{oldStart,oldLines,newStart,newLines,lines}`; file keys only `filePath, hunks, created?, deleted?`; FileChanges produced on a real file |
| `tool_use.caller` dropped | 545,850 blocks | always `{"type":"direct"}`, no loss |
| Nested subagent | real `parentAgentId` meta | `ParentLink.session_id` = parent agent id, `call_id`, `agent_type` set |
| Last-wins `merge` of continuation lines | 31,129 multi-line groups | no earlier non-null value replaced by a later null |
| Envelope noise in `extra` | 83 main sessions | constant-valued per-entry keys are 0.2% of output bytes; not worth a change |

## Findings

### 1. major: a result line written before its tool_use line becomes an orphan and the call ends Interrupted

`src/claude.rs:480-527` (`fold`), `src/claude.rs:1063-1083` (`close_calls`).

Scenario (measured): the CLI wrote the `tool_result` line 1 to 3 lines BEFORE the assistant line holding its
`tool_use` in 27 results across 16 files (CLI 2.1.156, 2.1.170, 2.1.177; tools Workflow 12, Edit 6, Skill 3,
Read 2, ToolSearch 2, Write 1, ScheduleWakeup 1). Example session `b437df58`: result L6794, tool_use L6796.
`fold` finds no call, pushes an orphan `Message{User, Tool}` plus a warning; the call arrives two lines later
with no result, and `close_calls` turns it Interrupted. 16 real calls are reported `interrupted` with no output
although their result is in the file; 27 of the corpus's 40 "orphan" warnings are this case (13 are true orphans).

Fix: keep an unknown-call result instead of emitting it at once. In `fold`, when `self.calls` has no
`call_id`, store `(Head, block, tur, denial, rec)` in `pending: HashMap<String, ...>` and return. In
`assistant`, after inserting a `ToolCall` part (line 361), if `pending` holds that id, run the normal fold for
it (status, output, `ended_at` = the result line ts) and absorb the line like `absorb_result_line`. In `finish`,
before `close_calls`, emit what is left in `pending` as orphan messages with the warning, inserted at their
line position. Add a fixture of two lines in that order.

### 2. major: lines repeated with the same uuid inside one file are not deduplicated

`src/claude.rs:131-180` (`record`), `src/claude.rs:338-364` (`assistant` appends parts again).

Scenario (measured): 1,531 lines in 29 files repeat an earlier line's top-level `uuid` (history copies written
again into the same file; they differ only in `parentUuid`, `slug`, `cwd`, `gitBranch`, `promptId`; CLI
2.1.181 to 2.1.220). Effects: 1,444 `hook_success` copies give 2,888 `Hook` entries that share an id; 23
assistant copies append the same blocks to the already open message, so one message holds the same
`ToolCall` id twice (and doubled text/thinking), tool counts are inflated; 63 result copies refold and move
`ended_at` to the copy's timestamp. Research section 4, rule 5 says to dedupe history copies by line uuid first.

Fix: in `record`, after `Head` is built, skip a conversation line whose uuid was already seen in this file
(`seen: HashSet<String>` next to `uuids`; `uuids` alone is not enough because Settings/queue entries do not
insert). Keep the trace: push the skipped line number to the first entry's `extra.copies` or count it in one
warning. Note for consumers: the same copy problem across files (1.4% of `message.id`) remains a
caller-side dedupe, document that `Session.usage` of a resumed file includes copied history (unverified count).

### 3. major: queue replay emits phantom "queued" human prompts

`src/claude.rs:287-307` (`queue_op`), `src/claude.rs:1087-1110` (`flush_queue`).

Scenario (measured): replaying every `queue-operation` in the corpus with the current rules leaves 27 human
texts in the queue (13 files); 26 of them were delivered later in the same file (as a `promptSource: queued`
user line or a `queued_command` attachment). In parser output 23 of the 27 `phase: "queued"` messages repeat a
later human message, so the prompt is counted twice and could become the title. Root cause: `<...>`
notification texts are enqueued but never get a matching `dequeue`/`remove` (they are delivered another way),
so they sit at the front and every later `dequeue` pops the wrong item. Example `e857d7c5`: five stale
notifications from L2586 to L3581 at the front when the human enqueue at L3711 is dequeued at L3712.

Fix: notifications never enter the human FIFO. In `queue_op`, `enqueue` pushes only texts that do not start
with `<`; a `remove` whose content starts with `<` is ignored; `dequeue` and content-less `remove` pop the
front human text. Measured with this rule: 0 leftovers corpus-wide (from 27). The `<` filter in `flush_queue`
then becomes dead and goes. Add a test: notification enqueue, human enqueue, one dequeue, expect no queued
message.

### 4. major: most plan rules have no test

`tests/claude.rs`. Not covered by any fixture line or test (grep over fixtures and tests): `bashEditDiff`
FileChange (plan 4.6), `returnCodeInterpretation` soft failure stays Ok with no exit code (4.3),
`<persisted-output>` and `persistedOutputPath` (`truncated`, `full_output_path`, 4.5), denial kinds
`permission-rule` and `automode-*` (Rejected) and `cancelled` (Interrupted), the text fallbacks
`Cancelled: parallel tool call` and `[Tool call interrupted`, `timedOutAfterMs` (backgrounded, Ok),
task-notification `failed`, `killed` and `(exit code 1: ...)`, nested subagent `parentAgentId`, `<teammate-message`
and `origin.kind` coordinator/peer (Agent, author), image blocks (Media bytes, D10), a user line split into
`#1`/`#2` entries (4.2), `system/local_command`, `hook_blocking_error`, `__unparsedToolInput`, plus the cases of
findings 1 to 3. The rules work on spot-checked real files (bashEditDiff, persisted output, nested parent), so
this is a regression risk, not a known bug.

Fix: extend the inline-lines style of `v2_1_rules` with one small test per group above; no new fixture file needed.

### 5. minor: `api_error` looks for `cause` inside `error`; the line-level `cause` is ignored

`src/claude.rs:1262-1273`.

Scenario (measured): 279 real `api_error` lines; 40 have no message in `error` but a line-level `cause`
(`{code: "ECONNRESET", ...}`, 2.1.143 shape), 15 have neither. The pointers `/cause/message` and `/cause/code`
are applied to the `error` value, so they never match; these entries get the placeholder `"API error"`
(seen on session `24465be7`). Nothing is lost (`cause` stays in `extra`), but `ErrorInfo.message` is wrong.

Fix: look up `cause` on `rec`: after the `error` pointers, try `rec.pointer("/cause/message")` then
`rec.pointer("/cause/code")` (`rec` is still available in `api_error`).

### 6. minor: continuation lines of a merged message lose their own `parentUuid` and timestamp

`src/claude.rs:338-384`.

Scenario (measured on the 400 file sample): 27 of 50,720 non-first assistant lines have a `parentUuid` that
resolves to another entry (an attachment or other line between the chunks). `assistant` drops it silently,
while `absorb_result_line` (lines 537-539) records the same situation as `parentUuid` in `extra`. The
timestamps of non-first lines are dropped too (only `tool_use` lines keep theirs as `started_at`).

Fix: when `idx` already existed, compute `self.resolve(parent)`; if it differs from the entry id, record it
like `absorb_result_line` does (for example `extra.parents[<line uuid>] = <parentUuid>`). Keep the last line's
timestamp as `extra.ts_end` if a consumer needs the stream end; otherwise state the drop in the module doc.

### 7. minor: first-value Settings are inconsistent

`src/claude.rs:186-191` vs `217-236`. The first `cwd` only seeds `current` and emits no `Settings`, while the
first `permission-mode`, `effort` and `agent-setting` do emit one (the fixture test expects
`Settings{approval: "default"}` and `Settings{effort: "medium"}`). A consumer replaying `Settings` gets every
starting value except cwd. Fix: pick one rule; simplest is to drop the `first_cwd` special case and let
`setting` emit the first cwd too (Codex already treats the first value as a change), then `session.cwd` stays
the first value.

### 8. minor (quality): JSON take-helpers exist three times

`src/claude.rs:1461-1516` (`take_str`, `take_u64`, `take_bool`, `take_obj`, `take_array`),
`src/codex/json.rs:13`, `src/opencode/record.rs:30-38`. `take_u64` also repeats the number/string coercion of
`util::u64_field`. Fix: one set in `util.rs` (`take_str`, `take_u64` reusing the `u64_field` coercion,
`take_bool`, `take_obj`), used by all parsers.

### 9. minor (quality): `finish` does six jobs in 100 lines

`src/claude.rs:940-1041`: journal check, call closing, subagent identity and meta, title choice, session extra,
cost. Fix: move subagent identity (`agent_id`, meta, `ParentLink`) and title selection into two small
functions; no behaviour change.

### 10. nit

- `src/claude/tools.rs:404`: `read_fields` deletes `file.base64`; `strip_blobs` would keep it as
  `{"bytes": n}` as D10 says. Remove the line.
- `src/claude.rs:197`: `&ENVELOPE[..1]` means "only sessionId"; name it (`&["sessionId"]`).
- `src/claude.rs:286` and `1085`: the doc comments claim leftover queue items "were typed but never sent";
  finding 3 shows that is usually false until fixed.

## Design question: exit code 0 for a plain Claude Bash success

Question: is exit 0 certain when `is_error` is false, there is no `returnCodeInterpretation`, and the call was
neither backgrounded nor interrupted? Yes, and I recommend setting `exit_code: Some(0)` there, like Gemini.

Producer evidence (the installed CLI 2.1.238 binary, Bash tool `call`): the result is interpreted as
`m = interpret(command, code, stdout)`; if `m.isError` and the stop was not a user interrupt, the tool throws
and the transcript gets `is_error: true` with `Exit code N`. The default interpreter is
`isError = code != 0`. The per-command table (`grep`, `rg`, `egrep`, `fgrep`, `find`, `diff`, `test`, `[`) is
`isError = code >= 2` with a message only for code 1, and that message is what lands in
`returnCodeInterpretation`. So a non-error object result has code 0, unless it carries
`returnCodeInterpretation` (code 1), `interrupted: true` (user interrupt), or `backgroundTaskId` (no code yet,
incl. `timedOutAfterMs`).

Corpus evidence: `returnCodeInterpretation` values are only the three code-1 messages ("No matches found"
3,423, "Files differ" 76, "Some directories were inaccessible" 18); `"interrupted":true` occurs 0 times; the six
Bash commands that end in `false` or `exit N` were all backgrounded, none came back as a non-error foreground
result. Unverified: the source of versions other than 2.1.238 (the corpus spans 2.1.143 to 2.1.296), though the
data shows no counterexample.

Change: in `src/claude/tools.rs` `shell_detail` (line 434), when `status == Ok`, the result was an object
(`shell.stdout.is_some()` from `shell_fields`), `background_id` is None, `interrupted` is false and `out.data`
has no `returnCodeInterpretation`, set `exit_code = Some(0)`. Update the `ShellResult.exit_code` doc in
`model.rs` ("recorded, printed, or fixed by the producer's rule") and plan 4.5's Claude row. Keep soft failures
at None as plan 4.3 says: code 1 is equally certain, but plan 4.3's shell rule (non-zero exit is Error) would
then flip them to Error, which the plan rejects.

## VERDICT: changes required

Findings 1 to 3 give wrong output on real files (16 calls with a wrong status, duplicated calls and hooks in 29
files, 23 to 26 phantom human prompts) and finding 4 leaves most plan rules untested. The rest is minor.
