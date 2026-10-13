# Code review: Gemini module (src/gemini.rs, tests/gemini.rs)

Reviewer scope: `src/gemini.rs` (1603 lines) and `tests/gemini.rs` (1513 lines). No source file was edited.
Contract read: `plans/agentparse.md` (v2.1), `review-triage-1.md`, `review-triage-2.md`, `review-inputs.md`,
`research-gemini.md`, `src/model.rs`, `src/util.rs`.

How it was checked:
- Build and tests: release build clean for this module, `cargo test --release --test gemini` 58 passed.
- Upstream source fetched with `gh api` (google-gemini/gemini-cli main): `chatRecordingService.ts` (reducer 256-585, writer
  1422-1640), `scheduler/state-manager.ts` (`toCancelled`), `scheduler/scheduler.ts` (policy deny, user deny),
  `scheduler/tool-executor.ts` (truncation), `tools/shell.ts` (result text), `utils/fileUtils.ts`
  (`formatTruncatedToolOutput`), `ui/hooks/useHistoryManager.ts` (info/warning/error recording).
- Crafted inputs in `<scratchpad>/review-gemini/`
  (`t1.jsonl` to `t11.jsonl`, `gen*.py`), run through `./target/release/agentparse show --json`.
- Corpus: the 2 real local sessions parse (G2, header plus context only, cwd from `.project_root`, 0 warnings).
- Fuzz: 750 mutated or truncated inputs (3 fixtures, byte flips, deletions, cuts), 0 panics, 0 abnormal exits.
- Performance: G2 file with 1000 full snapshots, 267 MB: 1.6 s user, 275 MB RSS (linear, fine). G3 with N messages
  each written 3 times: 20k 0.35 s, 40k 1.57 s, 80k 7.06 s user (quadratic, finding 7).

Reducer verdict: the port matches the upstream loader on every rule I compared (classification order, upsert keeps
position, `$set.messages` clear-and-insert, `$rewindTo` unknown id clears all, `removeIds`, `orderIds` prefix-then-listed,
patch of an unknown id is a no-op, `result: null` replaces). The full view and Rewind placement are sound. The defects
are in what is built on top of it.

Status key: VERIFIED = reproduced with a crafted file or confirmed in upstream source. UNVERIFIED = reasoned from code only.

## 1. Correctness

### 1. [major] A truncated shell result loses its exit code and status (VERIFIED)
`src/gemini.rs:1520-1523` (`structured` check), `:1506`. Upstream truncates the WHOLE shell `llmContent`
(`tool-executor.ts:202-260`, `fileUtils.ts:788`), so a result over the 40k threshold is
`Output too large. Showing first N and last M characters. For full output see: <path>\n<head>...<tail>` where the head
starts with `Output: ` and the tail still ends with `Exit Code: 2\nProcess Group PGID: 9`. The parser only treats a body
starting with `Command: ` or `Output: ` as structured, so the banner makes the whole text `stdout`.
Scenario (`t11.jsonl`): a failing `make` with 100k chars of output and `Exit Code: 2` comes back `status: ok`,
`exit_code: None`, stdout containing the banner and the `Exit Code` line. The biggest failing builds are exactly the ones
that get truncated. `tests/gemini.rs:1089` (`shell_truncated_output_records_the_saved_path`) encodes an invented shape
(`Output: Output too large...`), which is why this passed.
Fix: in `parse_shell`, when `body` starts with `Output too large. Showing first`, drop that first line, then continue
with the normal structured path (the remaining body starts with `Output: `). The omitted-characters marker stays in
`stdout`, which matches `text`. Replace the test text with the real shape and assert `exit_code == Some(2)` and
`status == Error`.

### 2. [major] Rejected, cancelled or failed edits are reported as file changes (VERIFIED)
`src/gemini.rs:950-953` and `:1409-1430`. `file_change` runs on `resultDisplay.fileDiff` for any status. Upstream
`toCancelled` (`state-manager.ts:457-480`) copies the confirmation FileDiff (`fileDiff`, `filePath`, `originalContent`,
`newContent`) into `resultDisplay` of a call cancelled while awaiting approval. So `User denied execution.` on a
`replace` or `write_file` carries a full diff of a change that never happened.
Scenario (`t2.jsonl`): `status: cancelled` edit yields `files: [update /f +1 -0]` next to `status: interrupted`.
A consumer summing `FileChange` counts or listing touched files is wrong for every denied edit.
Fix: compute `files` only when the final `status == ToolStatus::Ok` (move the `file_change` call after `call_status`,
`.filter(|_| status == ToolStatus::Ok)`). Keep the diff in `data.resultDisplay` for non-Ok calls (do not strip `fileDiff`
there), so nothing is lost.

### 3. [major] Tail-line peeling eats real stdout lines, and the `Error:` value has no home (VERIFIED)
`src/gemini.rs:1477-1483`, `:1569-1585` (`split_tail`). In the modern text the tail keys are matched from the end by
prefix. A command whose last output line starts with `Error: ` (cobra, kubectl, helm, terraform, node, many CLIs) is
peeled as the tail `Error:` line when `Exit Code:` follows. Upstream prints `Error: ` only when `result.error` is set
(spawn failure, `shell.ts:906-910`), so for a normal failing command it is always a stdout line.
Scenario (`t1.jsonl`, `Output: tests...\nError: Process completed with exit code 1.\nExit Code: 1`): `stdout` is
`tests...`, the `Error:` line is gone from every structured field (it only survives in `ToolOutput.text`). The peeled
`Error:` value is never stored at all (`tail[0]` is unused), so a real spawn error is lost from structured data too.
A background-id false positive exists as well (finding 10).
Fix: for the `Output:` era peel `Error: ` only when everything before it is `Output: (empty)`; otherwise leave it in
stdout. Keep peeling it unconditionally for the old `Command:` era (always printed there). Store a peeled error in
`ShellParse.pids` renamed `data`, as `data["error"]`. Add tests for a stdout last line `Error: x` with exit 1 and for
`Output: (empty)\nError: spawn x ENOENT`.

### 4. [major] Rewind entries are emitted for UI notices (VERIFIED in source, mechanics reproduced with `t8.jsonl`)
`src/gemini.rs:272-279` (`drop_ids`), `:307-314`, `:282-286`. Upstream records `info`, `warning` and `error` through
`recordMessage` (`useHistoryManager.ts:93-114`), which calls `indexMessage` and puts them in `messageOrder`
(`chatRecordingService.ts:889-893`). The next `updateMessagesFromHistory` computes
`removedIds = previousMessageOrder not in history` (`:1550`) and writes `$patch.removeIds` (or `$rewindTo <noticeId>` for a
pure tail rollback, `:1566-1572`). Notices are never in the model history, so in G3 every notice is followed by a
removal of itself on the next sync.
Scenario (`t8.jsonl`): `warning` then a `gemini` turn then `{"$patch":{"removeIds":["w1"]}}` produces
`Rewind { dropped: ["w1"] }`. In a real G3 session each `info`/`error`/`warning` therefore gets a spurious Rewind,
although D13 and plan 4.2 define Rewind as "where context was cut". Consumers deriving active context from Rewind
entries would see cuts that did not happen. (No real G3 file exists locally, so this is source-verified only.)
Fix: in `drop_ids`, filter `dropped` to ids whose record `type` is `user` or `gemini`, and emit no `Slot::Rewind` when the
filtered list is empty. The live list still drops all of them. Same filter for `rewind_to` and `replace_all`. Add a test
with a notice removed via `removeIds` and via `$rewindTo`.

### 5. [major] Binary parts of an echo turn: duplicated as a fake user message, and the `info` binary carrier is dropped (VERIFIED, judges the flagged review input)
`src/gemini.rs:780-797` (`user_groups`, `Class::ToolAttachment`), `:374-387` (`harvest`), `:692-699` (`info` arm).
The flagged decision is wrong. Upstream stores the sibling binary parts in BOTH `toolCalls[].result` and the echo turn
(research 5.6, `[functionResponse, ...sibling inlineData]`), and `read_resp` already puts the `result` siblings into
`output.attachments`. The echo then adds the same image again as `Message { User, Context, tool_attachment }`.
Scenario (`t3.jsonl`): one `read_file` with a PNG gives `output.attachments = [image/png 11 B]` on the call AND a separate
`user (tool_attachment)` message with the same attachment: a user turn that never happened, double counted.
The opposite loss: an `info` message whose content is the binary parts (research 5.2, binary injection) becomes
`Notice { text: "" }` because only `join_text` is read; the mime and size are dropped (`t3.jsonl`, `i1`).
Fix: (a) when the echo turn has at least one folded (non-orphan) `functionResponse`, route its non-text media parts to the
call: in `harvest` store `functionResponse` plus the sibling media parts of that echo turn (`Value::Array` of them) so
`read_resp` returns them when the record has no `result`, and in `user_groups` skip media parts of such a turn instead of
creating a `ToolAttachment` group (keep `Class::ToolAttachment` only for the no-response case, or delete it). Deduplicate
by (mime, bytes) when both `result` and echo carry them. (b) For `info`/`warning` with media parts emit
`Message { User, Context, kind: "binary_injection", parts: [Attachment] }` right after the Notice, or put the attachments
in the Notice `extra`. Test both.

### 6. [minor] `<state_snapshot>` matched by `contains` swallows human prompts (VERIFIED)
`src/gemini.rs:565`. Upstream also uses `includes` to find an existing snapshot (`chatCompressionService.ts:588`), but there
the text is the model's own history. Here a human prompt that merely mentions the tag becomes a `Compaction` and the
prompt disappears (`t7.jsonl`: "please explain what <state_snapshot> tags are..." gives `compaction`, no human message,
no title). Plausible in sessions about this very feature.
Fix: require both `<state_snapshot>` and `</state_snapshot>` in the same text part, and prefer the part whose following
record is the "Got it. Thanks for the additional context!" model ack (look ahead one slot in `Build`). Test a human prompt
mentioning only the open tag.

### 7. [minor] Quadratic `is_live` (VERIFIED)
`src/gemini.rs:226-235`. Every bare record scans `live` from the end (`iter().rev().any`). Measured user time for N
messages written 3 times each: 20k 0.35 s, 40k 1.57 s, 80k 7.06 s. Real sessions are far smaller, so this is a ceiling,
not a live bug. `$patch.orderIds` (`:324`) repeats the scan per listed id (all history ids per sync), also O(n^2).
Fix: keep a `HashSet<String>` mirror of `live` (insert in `upsert`, `replace_all`, `orderIds`; remove in `drop_ids`) and use
it for `is_live`. Keep `Vec` for order.

### 8. [minor] Policy denial and the cancel reason text (VERIFIED in source)
`src/gemini.rs:1331-1352`, `:1367`.
- A policy denial is recorded as `status: error` with `Tool execution denied by policy.` (`scheduler.ts:667-682`,
  `policy.ts:37-46`), never `cancelled`. It maps to `Error`, but 4.3 puts a refusal in `Rejected`.
- The scheduler writes `[Operation Cancelled] Reason: <reason>` (`state-manager.ts:514`); only the
  tool-executor abort path omits `Reason:`. `cancel_reason` strips only `[Operation Cancelled]`, so
  `status_detail` becomes `cancelled: Reason: User denied execution.` (`t8.jsonl`).
- Real reasons are `User denied execution.` (denial, matched by "denied"), `Operation cancelled by user`,
  `Operation cancelled`, `User cancelled tool execution.`, `User cancelled operation` (siblings cancelled after a
  denial). The word list `rejected/declined/disallowed/not allowed/policy` is speculation, and the test at
  `tests/gemini.rs:1139` uses an invented reason (`Denied by policy.`).
Fix: strip an optional `Reason:` after the marker; reduce `is_denial` to `denied`; add `status == "error"` plus text
starting `Tool execution denied by policy` to `Rejected`. Rewrite the test with the real strings.

### 9. [minor] FileChange +/- counts misread content lines that start with `++` or `--` (VERIFIED)
`src/gemini.rs:1413-1416`. `starts_with('+') && !starts_with("+++")` skips an added line whose content starts with `++`
and a removed line whose content starts with `--` (SQL or Lua comment `-- x`, a YAML or Markdown `---` rule).
`t2.jsonl`: diff with removed `-- old sql comment`, added `++counter`, added `-- new` gives +1 -0, correct is +2 -1.
Fix: count only lines after the first `@@` hunk header (the `Index:`, `===`, `---`, `+++` header precedes it), no prefix
exclusion needed.

### 10. [minor] Shell text edge cases (VERIFIED where marked)
- `src/gemini.rs:1514`: `body.split_once(MOVED_TO_BACKGROUND)` matches the phrase anywhere. Upstream's background text
  is the entire content (`shell.ts:893`). Output that merely contains the sentence returns early with no stdout and no
  exit code (`t1.jsonl` c4, VERIFIED). Use `body.starts_with`.
- `Command was cancelled by user before it could start.` (`shell.ts:544`) is not matched by `CANCELLED` (`...could
  complete.`), so it is `Ok` with the sentence as stdout (VERIFIED, `t9.jsonl` c2). Match the shared prefix
  `Command was cancelled by user before it could`.
- A process killed by a signal has no exit code and no `Exit Code` line, only `Signal: SIGTERM`; `failed` ignores the
  signal, so the call stays `Ok` (`t9.jsonl` c1, VERIFIED). Treat `signal.is_some()` as failed.

### 11. [minor] One masked result flags sibling calls as truncated (VERIFIED)
`src/gemini.rs:392-403` (`is_masked`), `:916-924` (`result_for`), `:360-390` (`harvest`). The upstream masking sync sets
`tc.result = turnParts`, the whole echo turn, on EVERY call of a parallel batch (`chatRecordingService.ts:1511-1540`).
`is_masked` tests every `functionResponse` of that array, so call A with an intact result is `truncated: true` because
B is masked (`t10.jsonl`), and A's intact text is never harvested as an original.
Fix: make `is_masked` take the call id and test only the `functionResponse` whose `id` equals it (the one `read_resp`
selects), in `harvest` and `result_for` alike.

### 12. [minor] Synthesised message ids for id-less snapshot entries (UNVERIFIED effect, upstream behavior verified)
`src/gemini.rs:247-250`. Upstream skips snapshot entries without a string `id` (`isMessageRecord`, `:470-476`). Here they get
`idx:{i}`, so entry 0 of two different snapshots collapses into one message and the id is invented. Fix: skip with a
warning like upstream, or use `idx:{line}:{i}`.

### 13. [nit] Smaller semantic differences
- `thoughts[].timestamp` is dropped (declared input). D4 says one home per value; put the array in `Entry.extra` or accept
  and document. (`src/gemini.rs:518-533`)
- `$patch.orderIds` with a repeated id duplicates it in `live` (`:324`; upstream dedupes through a Map). UNVERIFIED effect
  (only `Rewind.dropped` would show it). Use `HashSet` dedupe with finding 7.
- Patch application order is `updates` then the single patch (`:289-299`); upstream applies the single patch first
  (`chatRecordingService.ts:358-375`). Only differs when both touch one id.
- Header `messages` replaces the live list (`:198-201`); upstream only merges them (`:497-520`). Header is line 1 in practice.
- `read_file`: `end_line` is converted to a count and merged into `limit` while `start_line` (1 based) and old `offset`
  (0 based) share `offset` (`:1137-1149`). Triage 1 said "as recorded". Raw `input` keeps the truth, so acceptable;
  state the base ambiguity in the doc comment.

## 2. Judgement of the implementer decisions in `review-inputs.md`

| Decision | Verdict |
|---|---|
| g1/g2/g3 format rule | Accept. Matches upstream writers; header-only goes to g2. |
| Rewind for `$set.messages` drops, `$rewindTo`, `removeIds`; Rewind before Compaction | Accept the structure, fix finding 4 (notice ids). |
| `<hook_context>` as `Context` message | Accept (Hook has no text field). |
| Slash command rule | Accept. Note a part split: a second text part starting `/word` inside one human record becomes its own Command message. |
| `<state_snapshot>` via `contains`, ack label, token notice linking | Fix finding 6. The "compressed from X to Y" parse is still unverified upstream. |
| Thoughts as `**subject** description` | Accept; timestamp loss in finding 13. |
| read_file start/end to offset/limit | Accept with the doc note (finding 13). |
| `replace_all = allow_multiple or expected_replacements > 1` | Accept. |
| FileChange counts from text, `diffStat` in data | Accept the choice, fix the counter (finding 9) and status gating (finding 2). |
| Exit 0 for modern success without an `Exit Code` line | Verified correct (`shell.ts:912`: printed only when non-zero), but a signal kill must not be Ok (finding 10). |
| Background id from `moved to background (PID: n)` or first Background PID | Accept, `starts_with` fix in finding 10. |
| Cancelled plus denial word is Rejected | Accept for `denied`, widen to policy errors, drop the invented words (finding 8). |
| Subagent tools are `Task` | Accept. |
| Binary echo parts as `Context/tool_attachment` | Reject, finding 5. |

## 3. Test gaps against research section 11 and the findings

Covered well: duplicate ids (1), snapshot replace (2), patch unknown id and `result: null` (3), rewind unknown id (4),
content shapes (6), empty gemini content (7), null tokens (8), missing result and plain string result (9), most shell eras
(10), masked recovery in G3 (11), subagent file (12), shared context id (13), torn line and invalid UTF-8 (14), header
cases (15).

Gaps:
- (14) CRLF inside `fileDiff` is not asserted. (16) no large `FileDiff` or large file test; no timing test.
- `orderIds` is never exercised for real: `tests/gemini.rs:549` orders `["c"]` over `[a, c]`, which is already the result,
  so a no-op implementation passes. Use `orderIds: ["a"]` and assert `Rewind.dropped` after a `$rewindTo` (`t6.jsonl`
  gives `[u3, u1]`).
- Parallel calls with no `id` on the `functionCall` parts and same names (works, `t4.jsonl`, but untested); an echo turn
  holding several `functionResponse` parts (masking sync shape, finding 11).
- Missing for the findings above: truncated shell with exit code, stdout last line `Error: x`, spawn error with
  `Output: (empty)`, phrase "moved to background" inside output, cancelled `replace` with FileDiff, real denial strings
  (`User denied execution.`, policy error), `Reason:` prefix, `before it could start`, signal kill, FileDiff counts with
  `--`/`++` lines, binary echo no-duplicate, `info` with `inlineData`, notice removed by `removeIds`/`$rewindTo`,
  human prompt containing the snapshot tag, snapshot with only one of the two tags.
- Several shell fixtures invent text that upstream never writes (`Output: Output too large`, `Denied by policy.` as a
  cancel reason, `User cancelled the operation.`). Derive fixtures from `shell.ts`, `state-manager.ts` and `fileUtils.ts`.
- Section 11 item 5 (G1 `.json` plus migrated `.jsonl` of one session) belongs to `discover.rs`, not tested here.

## 4. Code quality

- Size: `src/gemini.rs` is 1603 lines, past the plan's "split into a directory only past ~1500 lines" (section 7), and
  opencode already split. Natural seams: reducer (lines 98-407, no dependency on entries) to `gemini/reduce.rs`; shell
  text (1469-1603) plus `truncation`/`call_status` to `gemini/shell.rs`.
- Dead arm: `Class::Snapshot => (Origin::Summary, Some("state_snapshot"))` (`:582`) is unreachable, `user()` builds a
  `Compaction` before calling `origin()` (`:737-748`). Remove it.
- Fragile coupling: `&MAPPED_MESSAGE_KEYS[..4]` (`:685`) depends on array order; use two named constants.
- `Part::Other.kind` is `o.keys().next()` (`:505-508`); `serde_json::Map` is sorted unless `preserve_order`, so the kind is
  the alphabetically first key, not the part type. Pick the first key among known part keys, else `"unknown"`.
- `crate::ErrorInfo` is used by path (`:702`) while every other type is imported at the top.
- `read_response` overwrites `resp.output` (`:1316`), discarding sibling text parts that came before the chosen
  `functionResponse` in the same result array; use `append`.
- `call_data` keeps `originalContent` and `newContent` of a FileDiff in `data` (`:1395-1399`) next to `FileChange.diff`:
  whole-file bodies (research 16 says never hold twice). For `write_file`, `newContent` equals `input.content`. Strip the
  duplicated one or document the choice.
- Duplication with util: none significant (`str_field`, `u64_field`, `bool_field`, `strip_blobs`, `base64_len` are reused).
  The small helpers `array` and `unmapped` exist only here. `util::secs_ms` is unused crate-wide (compiler warning, not
  this module).
- Long functions: `Build::call` (50 lines) and `parse_shell` (65 lines) are the only ones worth splitting; `parse_shell`
  splitting the cancelled, background, old era and modern era branches would also make findings 1, 3 and 10 local fixes.
- Comments are accurate. One stale claim: `is_denial` doc mentions a refusal reason set that upstream does not produce
  (finding 8).

## 5. Not defects (checked)

No panics found (no `unwrap`/`expect`, all slicing is by `find`/`get`/`strip_prefix` on char boundaries, 750 mutated
inputs clean). Usage formula, cached greater than input warning, per-message attribution, `Session.usage` sum, first-seen
order, `ended_at`, cwd from `.project_root`, subagent parent link and `child_session`, G1/G2/G3 mixed files, id-less
`functionCall` pairing by name, upsert keeping position, masked recovery from an earlier line all behave as the plan says.
G2 snapshot cost is linear in file bytes.

VERDICT: changes required
