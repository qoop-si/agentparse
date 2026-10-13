# P4 cross-module review: one model, four parsers

Reviewer lens: do Claude, Codex, OpenCode and Gemini produce the same model the same way, and is the crate minimal.
Inputs: `plans/agentparse.md` v2.1, `plans/review-inputs.md` (incl. the Claude and Codex notes), `src/model.rs`,
`src/lib.rs`, `src/util.rs`, `src/patch.rs`, `src/discover.rs`, `src/main.rs`, all four parser modules (Claude final).

Evidence (all from the release binary, 2026-10-13):
- Every fixture dumped with `agentparse show <file|id> --json` (OpenCode: `v1.sql`/`v2.sql` built into scratch DBs, read via
  `OPENCODE_DB`), then tabulated with jq: shell calls, message role/origin/kind, non-message entries, session fields.
- `agentparse stats --json` on the local corpus: Codex 133 sessions, OpenCode 64, Gemini 2; 0 failures.
- Claude sample (150 recent main files, `show --json`): Bash calls by status/exit: `ok/none` 1581, `ok/none` with
  `returnCodeInterpretation` 19, `ok/0` (background notification) 15, `error/1` 46, `rejected/none` 49.
  Codex corpus exit codes: `0` 929, `1` 46, `2` 25. OpenCode: `0` 10, `3` 1.
- `cargo clippy --all-targets -- -D warnings`: clean. `cargo fmt --check`: FAILS (finding 1).

## 1. Consistency table

Legend: OK = same convention; DIV = divergence, fixed by the finding number in brackets.

| Concept | Claude | Codex | OpenCode | Gemini | Verdict |
|---|---|---|---|---|---|
| Shell.command / argv | `input.command`; argv empty | script of `[sh,-lc,x]` else quoted argv; argv kept | `input.command`; argv empty | `args.command`; argv empty | OK (argv only where recorded) |
| Shell.cwd | line `cwd` (always) | `workdir` else turn cwd (always) | v1 `workdir` else `path.cwd`; v2 `workdir` only, else None | `dir_path` or old `Directory:` line, else None | DIV [5] |
| Shell.timeout_ms / background | `timeout`; `run_in_background`, Monitor | `timeout_ms`; never set `background` (running state goes to `background_id`) | `timeout`; `background` | none; `is_background` | OK |
| ShellResult.exit_code on plain success | None (no code recorded) | 0 (recorded) | 0 (`metadata.exit`) | 0 (inferred: modern `Output:` without `Exit Code` line) | DIV [2]; model doc says "never inferred" |
| stdout / stderr | split from `toolUseResult`; empty stderr -> None (folded streams) | `aggregated_output` single; old split builds keep `Some("")` | single stream, harness trailers removed | `Output:` single; old `Stdout:/Stderr:` split, `(empty)` -> `Some("")` | OK by the doc rule ("stderr None = stdout may be interleaved") |
| duration_ms | none recorded | event `duration` (measured) | `end - start` | none recorded | OK |
| interrupted / timed_out | denial kind, `interrupted` flag; "Command timed out after" | `Script terminated`, turn abort | `metadata.interrupted`, notice text, bg `killed`/`timeout` | `cancelled`, cancel/timeout text | OK |
| ShellResult on non-Shell tools | none on `TaskOutput`/`BashOutput` (Process) | on `write_stdin` (Process) AND on `exec` (Code: wall time, cell id) | Shell only | Shell only | DIV [12] |
| Shell failure rule (non-zero or timeout -> Error) | yes | yes | yes | yes (over `success`) | OK, implemented 5 times [D7] |
| Pending -> Interrupted | later assistant msg or Turn (`close_calls`) | same, copy-pasted (`mark_interrupted`) | any later row (`later`) | later `gemini` msg (`has_later`) | DIV in code, same intent [6] |
| status_detail | denial kind or notification status only | product status word / cell header line | raw state status or error kind | raw `status`; `cancelled: <reason>` | OK ("as recorded") |
| Human-run command | `Message{User,Command,kind:"bash-input"}`, call name `bash-input`, input = command string | kind `user_shell`, name `CommandExecution`, input Null | kind None; v1 name `bash` + recorded input; v2 name `shell`, input `{"command":..}` invented | not recorded | DIV [13] |
| Subagent prompt in a child session | `Origin::Agent` | `Origin::Agent` (inherited / inter-agent) | `Origin::Human` | `Origin::Agent` | DIV [3] |
| Own inter-agent message | n/a (tool call) | `Assistant` + `Origin::Agent` | n/a | n/a | DIV [11] (Origin doc: Agent = another agent) |
| Message.kind | raw tag / origin.kind / subtype (`bash-input`, `system-reminder`, `meta`) | `content_item_kinds` (`agents_md.instructions`), message type (`NEW_TASK`) | `synthetic`, `ignored`, row type, `metadata.source` | `session_context`, `hook_context`, `slash_command`, `tool_attachment`, `compaction_ack` | OK by contract (raw labels; `origin` is the normalized axis) |
| Notice / Error / Other | system subtypes -> Notice; api errors -> Error; metadata lines -> Other | `error`/`stream_error` -> Error; no notices recorded; leftovers Other | retries/errors -> Error; todos -> Other | info/warning -> Notice; error -> Error (`kind: api_error` from text) | OK |
| Turn.status | `interrupted`; `turn_duration` -> None | `completed` / `failed` / `interrupted` | `succeeded` (raw idle outcome) / `failed` / `interrupted` | no turns recorded | DIV [9] |
| Interrupt marker text | `Entry.extra.text` | `Entry.extra.marker` | `Turn.error` (abort error message) | n/a | DIV [9] |
| Compaction | trigger, pre/post tokens, logical_parent, summary | summary, usage, rest in extra | trigger (v1 `manual` inferred when `auto` absent), summary, usage | summary (`<state_snapshot>`), tokens from info notice | OK, one nit [14] |
| Settings | on change; first cwd lives in `Session.cwd` | first `turn_context` counts | switch rows only | none | OK |
| Entry.id | `message.id` / uuid, `#n` | response/item id, `ord:<n>`, `#n`; call fallback `line:<n>` | row id, `#n` | message id, `#n`, `idx:<i>`, `<id>#call<n>` | OK (synthetic schemes differ, all unique) |
| Entry.turn_id | `promptId` on prompt and replies | turn id on prompt and replies | replies only (`parentID` / last user row); prompt None | None | DIV [4] |
| Per-call leftover keys | result line -> `extra.results[call_id]`; `tool_use` block leftovers DROPPED (`caller`) | `extra.calls.<id>.{call,output}` | in `ToolOutput.data` (one record) | in `ToolOutput.data` (one record) | DIV [7] |
| Entry.extra cleaning | drops null/empty 3 levels, strips blobs | strips blobs, keeps nulls | drops null session columns only, strips blobs | strips blobs, keeps nulls | DIV [D8] |
| FileChange.diff | `@@ -a,b +c,d @@` hunks, no file header | `unified_diff`; add/delete as `+`/`-` lines; apply_patch body | product `patch`/`diff` (with jsdiff header) or apply_patch body | jsdiff `fileDiff` with `Index:`/`---`/`+++` header | OK by doc, doc too vague [15] |
| Usage (plan 4.1) | matches | matches, warns cached > input | matches | matches, warns cached > input | OK |
| Session cost | recorded `cost-state` wins | none recorded | sum of messages; recorded `cost` column left in `extra` | none | DIV [10] |
| Title | custom > agent-name > description > ai-title > summary > first prompt | first prompt (finish) | row title | `$set.summary` > first prompt | OK |
| models | finish() over messages | same | same (user messages carry the selected model) | same | OK |
| ParentLink | session or parentAgentId, call_id, agentType | parent_thread_id, no call_id, agent_type = agent PATH | parent_id, call_id via DB, agent_type from title | dir name, no call_id, no type | OK (doc allows path); `SessionRef.parent_id` None for Codex [17] |
| child_session == child Session.id | agentId | thread id | session id | agentId == child sessionId | OK |

## 2. Findings

### 1. HIGH: `cargo fmt --check` fails, CI is red
`src/discover.rs` (19 hunks), `src/main.rs` (26), `src/util.rs` (12), `src/model.rs` (10), `src/lib.rs` (4),
`src/patch.rs` (2). The shared files were written at a wider width; the four parser modules pass. There is no
`rustfmt.toml`. Change: run `cargo fmt` once (default width, no config file). Plan 8.1 and `ci.yml` line 26 gate on it.

### 2. MEDIUM: one exit-code rule for a successful command
Claude leaves `exit_code` None on success (1581 of 1727 sampled Bash calls), Gemini infers 0 from the same kind of
evidence (`src/gemini.rs:1561-1563`), Codex and OpenCode record 0. `src/model.rs:491` says "never inferred", which the
Gemini code and plan 4.5 already contradict. A consumer filtering `exit_code != Some(0)` misclassifies 90% of Claude
commands. Recommendation: the Gemini rule wins and Claude follows it.
- `src/claude/tools.rs` `shell_detail` (434): after the error branch, when `status == Ok`, `shell.exit_code` is None,
  `!shell.interrupted`, `shell.background_id.is_none()` and the result had no `returnCodeInterpretation`, set
  `Some(0)`. Research 5.1: every non-zero exit is `is_error: true` with `Exit code N` (8,906), the only exception is
  `returnCodeInterpretation` (real code 1). Pass a `soft: bool` from `structured`/`shell_fields` for that key.
- `src/model.rs:491`: "Recorded or printed; also 0 when the product's success result would have printed any non-zero
  code (Claude Bash without `returnCodeInterpretation`, Gemini `Output:` format). Never otherwise inferred."
- Plan 4.5 exit-code row for Claude: add "0 on success without returnCodeInterpretation/background".

### 3. MEDIUM: OpenCode marks a subagent's prompt as `Origin::Human`
Child sessions `ses_ed65a5c6...` (v1) and `ses_ecd6a835...` (v2): first user message `origin: human`. Claude
(sidechain), Codex (inherited / inter-agent) and Gemini (`kind: subagent`) all say `Agent`, so `is_human()` and the
`finish()` title fallback are wrong only for OpenCode. Change in `src/opencode.rs` `fill_header` callers (154, 205):
when `session.parent.is_some()`, remap user messages with `Origin::Human` to `Origin::Agent` (one loop over
`session.entries` before `finish()`).

### 4. MEDIUM-LOW: OpenCode prompt messages carry no `turn_id`
Claude and Codex give the prompt the same `turn_id` as its replies; OpenCode gives the replies the prompt's id
(`parentID`, or the last user row in v2) but leaves the prompt itself None, so grouping by `turn_id` drops the prompt.
Change: `src/opencode/v1.rs` `user_messages` (147) and `src/opencode/v2.rs` `v2_user` (78): `turn_id = Some(id)` (the
unsplit record id). Doc `src/model.rs:142`: "OpenCode triggering user message id (v2: the last user row)". Gemini stays
None (no turn recorded).

### 5. MEDIUM: Shell.cwd missing where the command ran in the session directory
Claude, Codex and OpenCode v1 always fill the effective directory; Gemini (`src/gemini.rs:1132`) and OpenCode v2
(`src/opencode/v2.rs:287`, v2 shell rows at 252) fill it only when the call names one, although `Session.cwd` is known
(`/home/user/proj`, `/work/demo` in the fixtures). Change: fall back to the session cwd. Gemini: compute
`project_root(&source)` before `Build` and use it as the `dir_path` fallback. OpenCode v2: `ToolCtx.cwd` = the session
`directory`, updated by `location-switched` rows. Doc `Shell.cwd`: "the directory the command ran in".

### 6. MEDIUM: the "no result -> Interrupted" rule lives in three copies
`src/claude.rs:1063` `close_calls` and `src/codex/mod.rs:693` `mark_interrupted` are the same loop (Codex adds an
`answered` set, which `output.is_some()` covers); Gemini re-derives it with `last_model_slot`/`has_later`
(`src/gemini.rs:604-632, 841, 1371`). Change: one pass in `Session::finish` (`src/model.rs:719`): a top-level call with
`status == Pending` and `output.is_none()` before the last assistant message or `Turn` becomes `Interrupted` (plan
4.3 wording: "a call with no result at all"). Delete `close_calls`, `mark_interrupted`, `answered`, `last_model_slot`
and the `has_later` parameter (about 60 lines). OpenCode keeps its own `later` rule (plan: pending/running parts in a
finished run carry state data, so `output` is Some); the shared pass is a no-op for it.

### 7. MEDIUM: per-call leftover keys: one home, and Claude drops some
Claude `tool_use` block keys other than `id/name/input` are discarded (`src/claude/tools.rs:21-37`): the fixture has
`"caller":{"type":"direct"}` on 19 calls, the parsed JSON has 0 (D4 violation). Claude result-line leftovers go to
`extra.results[call_id]` (`src/claude.rs:543`), Codex uses `extra.calls.<call_id>.{call,output}`
(`src/codex/json.rs:87`). Decision (see 3.1): no `ToolCall.extra`; Codex's shape wins.
- `src/claude/tools.rs` `tool_call`: return the leftover `b` (minus `type`) next to the call; `assistant()` merges it
  as `extra.calls.<id>.call`.
- `src/claude.rs:542-547`: `results` -> `calls.<call_id>.output`.
- `src/model.rs` `Entry.extra` doc: "leftovers of separate call and result records: `calls.<call_id>.{call,output}`".
  OpenCode and Gemini keep theirs in `ToolOutput.data` (one record holds call and result, plan 4.7).

### 8. MEDIUM: three private copies of the `take_*` helpers, with different semantics
`src/claude.rs:1461-1530` (take_str/u64/bool/obj/array/ts), `src/codex/json.rs:8-30` (take, take_str, obj_str),
`src/opencode/record.rs:24-48` (take_with/str/i64/u64/f64/flag). They disagree: Codex `take_str` turns numbers into
strings, Claude's and OpenCode's leave them; Claude `take_u64` accepts floats and numeric strings, OpenCode's only
`as_u64`. So `"timeout": "120000"` is read by one parser and left in `extra` by another. Change: one set in `util.rs`
(`take_str`, `take_u64`, `take_i64`, `take_bool`, `take_obj`, `take_array`) with the leniency of the existing
`util::str_field`/`u64_field` (number-as-string accepted, unparseable value left in place); delete the three copies.
OpenCode keeps `take_nested`/`take_span` on top.

### 9. LOW: Turn status words and interrupt text
- `src/opencode/v2.rs:382`: map `outcome` `succeeded` -> `completed` (doc `src/model.rs:600` lists `completed`;
  `failed`/`interrupted` already match).
- Interrupt marker text: Claude `extra.text` (`src/claude.rs:699`), Codex `extra.marker` (`src/codex/mod.rs:570`).
  Rename Claude's key to `marker`. OpenCode's abort message in `Turn.error` stays (it is an error record).

### 10. LOW: OpenCode session cost should be the recorded one
Model doc (`src/model.rs:79`): the product-recorded session cost wins. Claude does that; OpenCode sums messages and
leaves the recorded `cost` column in `Session.extra` (fixture: usage `0.013399999999999999`, extra `0.0134`). Change in
`fill_header` (`src/opencode.rs:211`): `session.usage.cost_usd = take_f64(&mut row, "cost")` before `finish()`
(which keeps a preset cost). Token columns stay in `extra` (plan 4.1: tokens are the sum).

### 11. LOW: Codex own inter-agent message is `Origin::Agent`
`src/codex/messages.rs:176`: `Role::Assistant` with `Origin::Agent`. Origin doc: Agent = another agent. Change:
`let origin = if own { Origin::Model } else { Origin::Agent };` (author and kind stay).

### 12. LOW: `ToolOutput.shell` only for Shell and Process
Codex attaches a `ShellResult` to `exec` code cells (`src/codex/results.rs:100-108`: wall time as `duration_ms`, cell
id as `background_id`); the cell header text already holds both and the cell is not a command. Claude's Process calls
(`TaskOutput`/`BashOutput`) get no `ShellResult` although `toolUseResult.task.{output,exitCode}` is recorded, while
Codex `write_stdin` (Process) does. Change: drop the `shell` block from `cell_answer` (keep `truncated`, status); in
`src/claude/tools.rs` `structured` add `"TaskOutput" | "BashOutput"` -> `stdout`/`exitCode` from `task` (or the top
level for BashOutput) into `ShellResult`. Doc `ToolOutput.shell`: "Shell and Process calls".

### 13. LOW: synthesized calls for human-run commands
OpenCode v2 invents `input: {"command": ..}` (`src/opencode/v2.rs:251`), which was never recorded. Change to
`Value::String(command)` like Claude's `bash-input`. Set `Message.kind: Some("shell")` (the row type) there so all
recorded labels are kept; v1 has no label (None is right).

### 14. LOW: OpenCode v1 compaction trigger inferred
`src/opencode/v1.rs:202`: a marker without `auto` becomes `"manual"`. Doc says "when recorded". Change: `auto: true`
-> `auto`, `auto: false` -> `manual`, absent -> None.

### 15. LOW: `FileChange.diff` shapes are documented too loosely
Claude: hunks without file header; Gemini/OpenCode: jsdiff text with `Index:`/`---`/`+++` header; Codex `unified_diff`;
apply_patch bodies (Codex, OpenCode fallback) have `@@ <context>` lines without numbers. Normalizing is not worth it.
Change only the doc at `src/model.rs:519`: "the product's diff text for this file: unified hunks, possibly preceded by
the product's file header lines, or the apply_patch hunk body (context `@@` lines without line numbers)". See D6 for
the shared count.

### 16. LOW: OpenCode tool input is not blob-stripped
`src/opencode/tool.rs:55, 106` keep `state.input` raw; Claude, Codex and Gemini run `strip_blobs` on input (D10:
no base64 anywhere). Change: `strip_blobs(&mut input)` in both raw readers.

### 17. LOW: public API and CLI
- `src/opencode.rs:162` `pub fn parse_storage_file`: not in plan section 5 and `parse_file` already routes OpenCode
  JSON files to it. Make it `pub(crate)`.
- `src/lib.rs:72` `agent.unwrap_or(Agent::Codex)` is dead (the line above returned for every other case). Use
  `let Some(agent @ (Agent::Claude | Agent::Codex)) = agent else { return parse_file(path) };`.
- `src/discover.rs:124-134` sets `parent_id: None` for every Codex rollout, while the parsed session has
  `parent_thread_id`. Document on `SessionRef.parent_id`: "when known from the path or database (never for Codex)".
  Reading each `session_meta` would cost a big first line per file.
- `src/main.rs:90`: `show <id>` takes the first ref with that id; ids repeat (Codex rollouts of one thread, resumed
  copies). Take the one with the highest `updated_at`.
- `src/main.rs:183`: a Codex `Rewind { turns: 2 }` prints "2 entries dropped". Print "N turns rolled back" when
  `turns` is set.
- Empty file (Claude OPEN item): Codex ("no Codex rollout records") and Gemini (no header) also return `NotASession`
  for a zero-byte file, so today all agree. Keep `NotASession`; no change.

## 3. Model change requests

1. `ToolCall.extra: Option<Value>`: REJECT. Only Claude and Codex have separate call/result records with leftovers;
   OpenCode and Gemini would leave it empty (one record, leftovers in `data` per plan 4.7). Unifying on Codex's
   `Entry.extra.calls.<call_id>.{call,output}` costs no field and touches only Claude (finding 7).
2. `Tool::Patch { patch: Option<String> }`: REJECT. A code-mode `FileChange` child has no input at all; plan 4.4 says a
   missing required typed field is `Tool::Other`, and `output.files` already carries the changes. An Option would
   make every Patch consumer handle a None that exists for one Codex child shape.
3. Gemini pgid / pid list fields: REJECT. `background_id` holds the first pid, `process_group` and `background_pids`
   stay in `data`; no other product records them.
4. A home for tool-attached binaries: ACCEPT, no model change. `ToolOutput.attachments` is that home (Claude result
   images, Codex output media, OpenCode `state.attachments` use it). Gemini: the echo turn's binary parts repeat the
   siblings that `read_resp` already puts into the call's attachments when the call record has a `result`; drop them
   there, and append them to that call's `output.attachments` when the record has no result. Remove
   `Class::ToolAttachment` (`src/gemini.rs:553, 581, 788`).

## 4. Duplication to extract (only real copies)

- D1 `take_*` helpers to `util.rs` (finding 8). Largest item, about 100 lines removed.
- D2 `Message::new(role, origin)` in `model.rs` (all other fields empty) replacing `src/claude.rs:1170-1190`
  (`user_message`, `model_message`), `src/codex/messages.rs:478`, `src/opencode/record.rs:114` and the two full
  literals in `src/gemini.rs:750, 879`. Same for `ToolCall::new(id, name, input, tool)` (Pending, no output): six
  full literals (`claude/tools.rs:24`, `claude.rs` bash-input, `codex/tools.rs:38`, `codex/results.rs:434`,
  `opencode/v2.rs:248`, `gemini.rs:961`).
- D3 data URL to `Media`: `src/codex/messages.rs:587` and `src/opencode/record.rs:179` are two implementations of the
  same parse. One `util::media_from_url(url, mime, name)`.
- D4 interrupt pass in `Session::finish` (finding 6).
- D5 rollout file name to thread id: `src/codex/mod.rs:718` (`id_from_path`) and `src/discover.rs:124-131` repeat the
  `rollout-` / `get(20..)` / `split('_')` parse. Keep one `pub(crate) fn thread_id(file_name)` in `codex`, call it from
  discover, and push through `Found::push` like the other products.
- D6 `+`/`-` line counting: `src/claude/tools.rs:384-388`, `src/codex/results.rs:513-517`, `src/gemini.rs:1413-1417`,
  `src/patch.rs:58`. One `util::diff_stat(diff) -> (u64, u64)` that skips header lines before the first `@@`; it also
  fixes the `+++`/`---` filter in Codex and Gemini, which drops a real added line whose text starts with `++`.
- D7 `ShellResult::failed()` (non-zero exit or timed out): the plan 4.3 shell rule, written out in
  `claude/tools.rs:451`, `codex/results.rs:356`, `opencode/tool.rs:304`, `opencode/v2.rs:197, 226`, `gemini.rs:1361`.
- D8 extra builders: Codex `as_extra`, OpenCode `extra_of`, Gemini `unmapped` tail, Claude `tidy`/`clean` all
  "strip blobs, None when empty", but only Claude drops null/empty values. One `util::extra(map) -> Option<Value>`
  that drops nulls and empty strings/arrays/objects, strips blobs and returns None when empty; all four call it.
- Optional, tiny: `mcp__<server>__<tool>` split is identical in `src/claude/tools.rs:145` and `src/codex/tools.rs:360`.
- Do NOT extract: the exit-code/shell text parsers (four different product formats), usage mapping, path-key lists.

## 5. Over-engineering and dead code

- `Origin::is_synthetic()` (`src/model.rs:235-240`): no caller in src or tests, not in the plan's helper list. Delete.
- `src/gemini.rs:582` `Class::Snapshot => (Origin::Summary, ..)`: unreachable, `user()` turns Snapshot into a
  Compaction before calling `origin()`. Delete the arm (match Snapshot with the Orphan/ToolAttachment cleanup).
- `src/model.rs:728` `model != "<synthetic>"`: Claude routes every `<synthetic>` message to Notice/Error before it can
  become a Message. Delete the condition.
- `src/claude.rs:717-733` (`ctx`) and `775-783` (`queued`) extract `origin.kind` / `name|from` identically. One helper.
- `src/codex/tools.rs:386`: garbled doc comment (text pasted twice). Fix to one sentence.
- Model fields: every field has at least one producer (single-producer ones are plan-specified: `Entry.parent_id`,
  `Compaction.logical_parent`, `Hook` (Claude); `Message.agent` (OpenCode); `Tool::Code`, `children`,
  `Rewind.turns`, `Settings.sandbox`, `Git.commit/remote` (Codex); `Rewind.dropped`, `signal` (Gemini, OpenCode)).
  No field is filled by all four the same derivable way. Keep the model as is.
- Module splits (Codex 5 files / 2726 lines, OpenCode 6 / 2165, Claude 2 / 2132, Gemini 1 / 1603) follow the plan's
  "split past ~1500 lines" rule. No change.

## 6. Order of work

Finding 1 first (mechanical). Then D1/D2/D8 (shared helpers, so later fixes land on them), then findings 2 to 7,
then the LOW items. Each parser's fixture tests need updating for 2 (Claude exit 0), 3, 4, 7, 9, 13, 14.

VERDICT: changes required
