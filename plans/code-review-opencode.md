# P4 code review: OpenCode module

Scope: `src/opencode.rs`, `src/opencode/{db,record,tool,v1,v2}.rs`, `tests/opencode.rs`.
Truth used: `plans/agentparse.md` v2.1, both triage files, `plans/review-inputs.md`, `plans/research-opencode.md`,
the upstream source copy from the research (`scratchpad/opencode-research/up/`, tags v1.18.35 and v2.0.27), the
fixtures, and the live `~/.local/share/opencode/opencode.db` opened read-only.
Scratch scripts: `scratchpad/review-opencode/{oracle,scenarios}.py` plus the reproduction databases next to them.

## What holds (measured)

- `cargo test --test opencode`: 41/41 green. Clippy clean, rustfmt clean for these files.
- Real DB: `agentparse stats --agent opencode`: 64 sessions, 0 failures, 0 warnings, 317 entries, 0 orphan results,
  parse p50 1.12 ms, p99 5.91 ms. 47 sessions `opencode-v1+v2`, 17 `opencode-v2`.
- Usage oracle (`oracle.py`, every session vs `session_v2.tokens_*`): 60/64 equal. The 4 misses are root 2.0.27
  sessions with usage the rows do not carry (implementer already flagged it). Compaction usage counting toward the
  session total is now verified upstream: `session/compaction.ts` publishes `UsageRecorded { source: "compaction" }`
  and `projector.ts applyUsage` adds it to the session columns.
- Real realgen session: exit 3 with `out-line` and `err-line` in one stream, background job enriched by its
  completion row, edit file change, read typed view, all correct.
- Read-only open with a 2 s busy timeout, table names are constants, ids are bound parameters, credential tables
  never touched, WAL rows read through SQLite (test present).
- The 6-file split is justified (2165 lines, plan threshold ~1500); boundaries are clean and the compiler reports
  no dead code.

## Findings

### Correctness

**1. major: migrated sessions repeat synthetic text, out of place**
`src/opencode.rs:130-138` (and the module doc at lines 8-10).
The rule "append v2 rows whose id is not a v1 message id" assumes the migration only wrote rows under v1 ids plus
the trailing notice. Upstream `v1-migration.bun.ts` (`transformSession`) also splits a user message that has both
ordinary and `synthetic: true` text into a `user` row (v1 id) plus a `synthetic` row under a NEW id from
`syntheticID(row.id, used)`, which keeps the first 16 characters of the v1 id. Verified on the real DB that
`syntheticID` keeps the prefix (47/47 migration notices share the first 16 id characters and `time_created` with the
row before them). Reproduced (`scratchpad/review-opencode/dup/`): a migrated session with one `@README.md` mention
parses as `user#1` (Human), `user#2` (Context, synthetic) from v1, and then the same synthetic text AGAIN as a
Context message after the last v1 assistant message, before the migration notice. Every migrated session in which
the user mentioned a file or got a reminder is affected (synthetic parts are how v1 records file reads and
reminders; the local DB happens to have none).
Fix: also skip a v2 `synthetic` row whose id shares its first 16 characters with a v1 message id:
```rust
let v1_prefixes: HashSet<&str> = v1_messages.iter().filter_map(|m| m.id.get(..16)).collect();
let migrated_copy = |r: &V2Row| {
    v1_ids.contains(r.id.as_str())
        || r.ty == "synthetic" && r.id.get(..16).is_some_and(|p| v1_prefixes.contains(p))
};
// .filter(|r| !migrated_copy(r))
```
The `system` notice keeps a v1 prefix too but is type `system`, so it stays (as the plan wants). Update the module
doc and plan section 6 OpenCode with this rule, and add the reproduction as a test.

**2. major: background shell completions with a failure are reported Ok**
`src/opencode/v2.rs:121-135` (capture) and `:187-201` (apply).
`apply_completions` sets `timed_out` from `state == "timeout"` and `interrupted` from `state == "killed"`. Upstream
`tool/plugin/shell.ts notifyWhenDone` writes `state` = the job status, one of `completed`, `cancelled`, `error`
(`shell/result.ts notification`), and signals a timeout with `metadata.timeout: true` and a kill with
`metadata.signal`. Neither `timeout` nor `killed` ever appears. Reproduced (`scenarios.py`): a cancelled job, a job in
state `error`, and a job with `metadata.timeout: true` all come out `status: ok`, `timed_out: false`,
`interrupted: false`. `metadata.truncated` and `metadata.signal` of the completion are also ignored.
Fix: extend `Completion` with `timed_out: bool` (`metadata.timeout == true`), `signal: Option<String>`,
`truncated: bool`, then
```rust
shell.interrupted = done.state == "cancelled";
shell.timed_out = done.timed_out;
shell.signal = done.signal.clone();
call.status = if shell.interrupted {
    ToolStatus::Interrupted
} else if done.state == "error" || shell.timed_out || shell.signal.is_some()
    || shell.exit_code.is_some_and(|c| c != 0)
{
    ToolStatus::Error
} else {
    ToolStatus::Ok
};
// and: output.truncated |= done.truncated
```
Keep `status_detail` as the completion state so the product label survives.

**3. major: `ParentLink.agent_type` is None for every real v2 child**
`src/opencode/db.rs:261-265`.
`agent_type` comes only from the title suffix `(@x subagent)`. opencode 2.x titles a child with the bare
`input.description` (`tool/plugin/subagent.ts:190`). Real DB: 0 of 6 child titles have the suffix, so all 6 parse
with `agent_type: None`, although the spawning call's `input.agent` is `general` in all 6. The v2 fixture child title
(`Locate callers (@general subagent)`) uses the v1 convention, so the test passes on data opencode 2.x never writes.
Fix: return the agent from the same lookup that finds the call: select
`json_extract(data, '$.state.input.subagent_type')` in the `part` query and
`json_extract(b.value, '$.state.input.agent')` in the `session_message` query next to the call id; fall back to the
title suffix. Change the native child fixture title to the bare description and assert `agent_type == "general"`.

**4. minor: v2 shell harness notices end up in `stdout`; a signal kill is Ok**
`src/opencode/tool.rs:123-129, 155-161, 299-308`; `src/opencode/v2.rs:150-161`.
Only `Exited with code N` is recognized. Upstream `shell/result.ts notice` also writes `Timed out before completion`,
`Killed by <signal>` and `Exited with code unknown`, and the tool appends
`\n\nCommand exceeded timeout of N ms. Retry ...` to the output on a timeout. The completion body joins output and
notice with a blank line. Reproduced: a foreground timeout gives
`stdout = "partial\n\nCommand exceeded timeout of 1000 ms. ...\nTimed out before completion"`; a `Killed by SIGKILL`
call keeps the notice in stdout and gets `status: ok`; a background `exit 2` completion keeps
`\n\nExited with code 2` in stdout (the existing test hides this by leaving the notice out of its completion text).
Fix: one helper used for both the last tool content part and the completion body, which splits off a trailing
notice in any of the four upstream forms (and strips the `Command exceeded timeout` sentence when `timeout` is set),
and parses the exit code from it. Treat `signal.is_some()` as Error in `tool_status` (one line in plan 4.3 next to
the shell rule).

**5. minor: a completion for a command containing `">` corrupts stdout**
`src/opencode/v2.rs:153`.
The body starts after the first `">`, but the header is
`<shell id="..." state="..." command="${command}">\n` with the command unescaped. Reproduced: command
`echo "hi">out.txt` with empty output gives `stdout = "out.txt\">"`.
Fix: strip the exact header built from the row itself (`metadata.jobID` else `shellID`, `metadata.state`, and
`description`, which upstream sets to the command), falling back to the first `">\n`; strip the trailing
`\n</shell>` with `strip_suffix`.

**6. minor: `location-switched` produces an empty `Settings`**
`src/opencode/v2.rs:420`.
v2.0.27 `location` is an object `{directory, workspaceID?}` (`schema/src/location.ts Ref`), so
`take_str(&mut data, "location")` is always None. Reproduced: the entry is `Settings {}` with the directory only in
`extra`. No test covers this row type.
Fix: `cwd: take_nested_str(&mut data, "location", "directory").or_else(|| take_str(&mut data, "location"))`.

**7. minor: denial detection is narrower than upstream's own**
`src/opencode/tool.rs:289-292`.
`is_rejection` uses `starts_with` on two phrases. The upstream TUI (`packages/tui/src/routes/session/index.tsx`,
v2.0.27) classifies a tool error as denied when it `includes` any of `QuestionRejectedError`, `rejected permission`,
`specified a rule`, `user dismissed`. A dismissed question (`The user dismissed this question`) is therefore Error
here, Rejected upstream.
Fix: `["rejected permission", "specified a rule", "QuestionRejectedError", "user dismissed"].iter().any(|m| message.contains(m))`.

**8. minor: aborted human-run `!cmd` (v1) is Ok with the notice in stdout; notice split matches mid-stream**
`src/opencode/tool.rs:265-287`.
v1 `session/prompt.ts` appends `\n\n<metadata>\nUser aborted the command\n</metadata>` to a human-run command, not
`<shell_metadata>`. Reproduced: status `ok`, `interrupted: false`, the tag in stdout. Separately, `split_notice` uses
`rfind` anywhere in the stream, so output that merely contains `<shell_metadata>` (for example a `cat` of a log)
is cut there.
Fix: split only when the trimmed stream ends with the closing tag, for each pair
`<shell_metadata>`, `<bash_metadata>`, `<metadata>`.

**9. minor: a user shell row still running is inferred Interrupted**
`src/opencode/v2.rs:223-230`.
Upstream starts a user `!` shell as a background shell with `timeout: 0` (`session/shell.ts`) and updates the row in
place on `session.shell.ended` (`message-updater.ts`), so later rows can exist while it runs (inferred from source,
not seen locally). Plan 4.3: a result that says it is still running is never inferred Interrupted.
`metadata.background` is also not mapped to `Shell.background`.
Fix: `"running" => ToolStatus::Pending` regardless of `later`; `background: metadata.background == true`.

**10. minor: recorded session cost is ignored**
`src/opencode.rs:37-45, 211-229`.
Plan triage-1 (R1-11, C-5): `Session.usage.cost_usd` = the product-recorded session cost when one exists, else the
sum. OpenCode records it (`session_v2.cost`, `session.cost`, maintained by `projector.ts applyUsage`), but the parser
sums messages and leaves `cost` in `Session.extra`. Not observable locally (all costs 0); on the 4 root sessions
whose columns exceed the rows, the sum would be wrong whenever the model is paid.
Fix: in `fill_header`, `session.usage.cost_usd = take_f64(&mut row, "cost");` (`Session::finish` keeps a cost the
parser set), so `cost` leaves `extra`. Keep `tokens_*` in `extra` as the oracle.

**11. minor: one session is read from several snapshots**
`src/opencode.rs:112-142`, `src/opencode/db.rs:113-217`.
`parse_db` runs up to 9 separate autocommit SELECTs (two session rows, parts, messages, session_message, todo, parent
lookups). On the live WAL database each one sees the newest commit, so a message committed between the `part` and
`message` queries appears without its parts, and the header can be newer than the rows. Verified by reading; the race
was not reproduced.
Fix: `let tx = conn.unchecked_transaction().map_err(sql_err)?;` right after `open` and run every query through `tx`
(a deferred transaction on a read-only connection pins one snapshot at the first SELECT); same in `list`.

**12. minor: duration subtraction panics in debug builds**
`src/opencode/v2.rs:194, 236`, `src/opencode/tool.rs:256`.
`u64::try_from(end - start)` overflows on extreme timestamps. Reproduced with a debug build:
`panicked at src/opencode/v2.rs:236:46: attempt to subtract with overflow`. Three copies of the same expression.
Fix: one helper in `record.rs`, used at all three sites:
```rust
pub(super) fn span_ms(start: Option<i64>, end: Option<i64>) -> Option<u64> {
    u64::try_from(end?.checked_sub(start?)?).ok()
}
```

**13. minor: a JSON-tree message or part file parses as an empty session**
`src/opencode.rs:162-173`.
`parse_storage_file` accepts any object with `id`. Reproduced: `agentparse show
tests/fixtures/opencode/storage/message/<sid>/<mid>.json` returns `Ok` with `id` = the message id, 0 entries, and
`extra {"role": "user"}`; same for a part file. Also a session file whose `message/<id>/` directory is missing
yields 0 entries with no warning.
Fix: return `NotASession` unless the file sits at `session/<project>/<id>.json`
(`path.parent().and_then(Path::parent).and_then(Path::file_name) == Some("session".as_ref())`); add a warning when
the message directory does not exist.

**14. minor: extra compaction summaries are silently dropped**
`src/opencode/v1.rs:84-87`.
`take_where` removes every later `summary: true` reply to the marker from its slot, and only the first is used, so
a second summary message (for example after a failed one) disappears. A failed summary's `error` stays buried in
`extra.summary_message` instead of becoming an `Error` entry. Verified by reading; no fixture.
Fix: take only the first match (leave the rest in their slots so they parse as normal assistant messages) and pass
the summary's `error` through `failure_entry`.

**15. minor: v1 `!cmd` entry id leaves references dangling and drops the marker's keys**
`src/opencode/v1.rs:160-193`.
The Command entry's `id` is the assistant message id and the user marker id is only `turn_id`. In the v1 fixture the
compaction's `extra.tail_start_id` (`msg_129a8912c049JK26fFvV1Qqo98`) points at that marker, and no entry has that
id. The marker message's own unmapped keys (`agent`, `model`, `summary`) are discarded, against D4.
Fix: `Entry.id` = the marker id, the assistant id in `extra.message_id`, and the marker's leftover keys merged into
`extra` under `user`.

**16. minor: v2 user file `description` and `mention` are dropped**
`src/opencode/v2.rs:82-92`.
`Prompt.FileAttachment` (`schema/src/prompt.ts`) has `description` and `mention {start, end, text}`; `v2_file`
discards every key it does not map. Against D4.
Fix: return the leftover map from `v2_file` and, when any file has leftovers, keep them in the entry `extra` as
`files` (index aligned).

**17. minor: partial tool input lands in different places per generation**
`src/opencode/tool.rs:49-99, 199`.
A v2 `streaming` call keeps its partial JSON string as `input`; a v1 `pending` call keeps `input: {}` and puts the
same partial string in `output.data.raw`, which also gives a call with no result an `output` (fixture call
`call_demo0016`). Consumers reading `output.is_some()` as "has a result" are misled.
Fix: in `raw_v1_tool`, when `input` is an empty object and `state.raw` is a non-empty string, use
`Value::String(raw)` as `input`.

### Test gaps

**18. minor: missing or unrealistic coverage** (`tests/opencode.rs`, fixtures)
- No migrated session whose v1 user message mixes ordinary and synthetic text (finding 1).
- No `location-switched` row at all (finding 6).
- Background completions only with `state: completed` and a body without the upstream notice; no `cancelled`,
  `error`, `metadata.timeout`, `metadata.signal`, or a command containing `">` (findings 2, 4, 5).
- No v2 tool content with `Timed out before completion` / `Killed by ...` as the last part (finding 4).
- No human-run v1 `!cmd` abort (finding 8).
- Native v2 child fixture title carries the v1 `(@agent subagent)` suffix that 2.0.27 never writes (finding 3).
- Rejection tests use the implementer's own strings; no question dismissal (finding 7).
- `subtask` part, multiple compaction summaries, JSON-tree message file given to `parse_file` (findings 13, 14).

### Code quality

**19. minor: small duplications of shared helpers**
- `record.rs:10-14` `OPAQUE_KEYS` repeats a subset of `util.rs:197` `BLOB_KEYS`; make `BLOB_KEYS` `pub(crate)` and use it.
- `record.rs:179-204` `media_from_url` re-implements the `data:` URL sizing that `util::strip_blobs` does; share one
  `data_url_bytes(s) -> Option<(mime, u64)>` in `util.rs`.
- `v1.rs:53` and `db.rs:330` clone a whole message map into a `Value` only to call `bool_field` / `str_field`; use
  `m.data.get("summary") == Some(&Value::Bool(true))` and `data.get("id").and_then(Value::as_str)`.
- `time.created` / `time.completed` are read with the same hand-written chain in `v1.rs:161-166`, `v1.rs:269-272`,
  `v2.rs:275-278`, `db.rs:326-329`; use `take_time` / one `time_of(data, key)`.
- Three copies of the duration expression (finding 12).

**20. minor: misleading comments**
- `opencode.rs:8-10`: "The migration notice and everything written after the upgrade are such rows" is false (finding 1).
- `tool.rs:223-224` (`shell_result`): describes only one notice form per generation (findings 4, 8).
- `tests/opencode.rs:1-3`: "every fact asserted here is listed in the fixture README", but about a third of the
  tests build their own rows inline.

## Implementer decisions (`review-inputs.md`)

Accepted: module split; bare `modelID` with `providerID` in extra; retry/error placement; split of mixed user parts
and `text.synthetic` / `text.ignored` parts; `aborted` to an interrupted Turn; keeping `Session.usage` the sum of rows
(the 4 oracle misses are real unrecorded usage); background `duration_ms` from `ran`; dropping provider item ids,
`apply_patch` `before`/`after` bodies, file part `source`, and the `instruction_*` tables; `float_roundtrip`.
Rejected or amended: `!cmd` entry id (finding 15), title-suffix `agent_type` (finding 3), shell notice handling
(findings 4, 8), rejection phrases (finding 7), every session column in extra (finding 10, `cost` gets its typed home).

## Unverified

- Finding 9 (a user shell still running while later rows are appended) is inferred from upstream source.
- Finding 11 (snapshot race) is from reading the code; not reproduced.
- Whether a v2 user abort writes both an assistant `aborted` error and an `idle interrupted` row (two Turn entries
  for one interrupt) could not be checked locally or in the source copy.

VERDICT: changes required
