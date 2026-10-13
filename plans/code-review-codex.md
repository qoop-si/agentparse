# Code review: Codex module (`src/codex/*`, `tests/codex.rs`)

Reviewed: the final version written at 05:56 (plan v2.1 rules applied, 21 tests). Contract: `agentparse.md`
sections 2, 3, 4, 6 (Codex), `review-triage-1.md`, `review-triage-2.md`; truth: `research-codex.md` and the
upstream copies under `scratchpad/codex-research/up/`.

## What was run

- `cargo test --test codex`: 21 passed. `cargo clippy --all-targets -- -D warnings`: clean.
- Corpus oracle (`scratchpad/review-codex/oracle.py`, compares `agentparse show --json` with the raw lines of
  all 130 rollouts): every `function_call`/`custom_tool_call` has exactly one top-level ToolCall (891/891),
  every output text equals the raw output (891/891), every `CommandExecution` appears once with its exit code,
  stdout, duration and status (962/962), every assistant text once (349), one reasoning part per id (544),
  every `UserMessage` event once as a human message (98), session usage equals the last
  `thread_token_usage` (130/130 files), one Turn per `task_complete`/`turn_aborted`, no repeated entry id.
  0 failures. `agentparse stats --agent codex`: 0 failures, 0 warnings, 0 orphans.
- Speed: the largest rollout (1.1 MB) parses in 10 ms; 16 copies of it (17.6 MB) in 0.13 s. Linear, no
  quadratic path.
- Every finding below was reproduced with a small constructed rollout in
  `scratchpad/review-codex/cases/` (fake text only) run through the release binary, unless marked
  unverified.

The corpus is all one generation (0.162.1, paginated, code mode). Every defect below sits in a shape the
corpus does not have: legacy files, direct tools, images, hooks, tail windows.

## Findings

### 1. major: a failed `apply_patch` is Ok and reports file changes that never happened

`src/codex/results.rs:60-75` (Patch branch of `fill_output`) with `text_status` at `results.rs:270-279`.

Upstream returns patch failures as plain text with no exit header (`h_apply_patch.rs:325, 367, 465`:
`apply_patch verification failed: ...`, also `failed to parse apply_patch: ...`, `apply_patch handler received
invalid patch input`). `printed()` finds no header, `text_status` returns Ok for anything that is not
"aborted"/"rejected by user", and then `out.files = patch::parse(patch)`.

Scenario (`t1-patch-fail.jsonl`, legacy, no `patch_apply_end`, the default Limited persistence of every
pre-0.162 file): `custom_tool_call apply_patch` updating `a.txt`, output `apply_patch verification failed:
Failed to find expected lines in /w/a.txt`. Got: status `ok`, `files: ["a.txt"]`. Expected: status `error`,
`files: []`. A verification failure is one of the most common Codex tool errors, so this hits real old
sessions.

Fix: in the Patch branch, success is `p.exit == Some(0)` or the text contains
`Success. Updated the following files:` (`ap_lib.rs:830`). Anything else is `Error` (unless `text_status`
says Interrupted/Rejected), with `status_detail` = the first line, and `files` stays empty:

```rust
if call.status == ToolStatus::Pending {
    let ok = p.exit.map_or_else(|| text.contains("Success. Updated the following files:"), |c| c == 0);
    call.status = match text_status(&text) {
        ToolStatus::Ok if !ok => ToolStatus::Error,
        s => s,
    };
}
```

### 2. major: legacy assistant dedupe keeps the first copy, which in real files is the event (deviation from the plan; judged: reject)

`src/codex/messages.rs:293-311` (`agent_text`, the `take_unmatched(Role::Assistant, ...)` branch).

The plan (section 6 Codex) says legacy assistant text comes from response items only, with the
`agent_message` event matched by text and skipped. The code instead keeps whichever copy comes first and
merges only the second copy's `extra`.

Real order: the event comes first. Measured on all 130 rollouts, `AgentMessage` event before the assistant
response item 356 of 356. Legacy `agent_message` events come from the same emission
(`AgentMessageItem::as_legacy_events`, `protocol_src_legacy_events.rs:125`), so 0.1xx legacy files have the
same order. The legacy fixture has the reverse order (item, then event), which is why the tests pass.

Verified consequences:
- `t2-legacy-event-first.jsonl`: event `agent_message` "Hello there." (no `phase`), then the item `msg_a1`
  with `phase: final_answer`. Got: one message, `phase: null`, `extra` empty: the item id `msg_a1` and its
  phase are lost. Expected: `phase: final_answer`, `msg_a1` in `extra.item_ids`.
- `t3-legacy-multipart.jsonl`: upstream emits one event per content part. Two events "Part one. ", "Part
  two." and one item with both parts. Got: one message with three text parts ("Part one. ", "Part two.",
  "Part one. Part two."). Expected: one text. Multi-part content is rare: 356 of 356 local messages have
  one part.

Judgement: the deviation is not acceptable for assistant text. Fix, as the plan says: in legacy mode an
`agent_message` event never creates an entry. Buffer `(text, phase, extra)`. When the response item arrives,
remove the buffered events whose texts, concatenated in order, equal the item text, take their phase when
the item has none, merge their `extra`, then `add_part` with the item id as usual. Flush still-buffered
events as text parts at `close_group` (fallback, so nothing is lost when an item is missing). The
`remember(Role::Assistant, ..)` / `take_unmatched(Role::Assistant, ..)` pair then goes away.

For user prompts the order is item first, then event (99 of 99), so "first copy wins" keeps the item. That
is acceptable once the match key in finding 3 is fixed (the item copy also holds the image attachment).

### 3. major: a legacy prompt with a pasted image becomes two human messages

`src/codex/messages.rs:78-93` (`input_run`), `97-116` (`user_event`), `543-552` (`text_of`).

Upstream builds the user response item with tag texts around a local image: `[input_text prompt,
input_text "<image name=[Image #1] path=\"...\">", input_image, input_text "</image>"]`
(`local_image_content_items`, `protocol_src_models.rs:1846`). The legacy `user_message` event `message` is
only the text inputs, joined with `""` (`UserMessageItem::message`, `protocol_src_items.rs:538`).
`text_of` joins every text part with `"\n"`, tags included, so the texts never match. `sniff` does not treat
`<image ...>` as context (no `_` in the tag), so the tags stay in the Human run.

Scenario (`t4-legacy-image.jsonl`): item as above, then `user_message` "what is in this screenshot" with
`local_images`. Got: two `user/human` messages (one with the tag texts, one with only the prompt). Expected:
one. This hits every image paste in pre-`content_item_kinds` legacy files. Paginated files depend on the
`content_item_kinds` the tag texts get (none recorded locally, unverified). Several text inputs also miss,
because of `"\n"` against `""`.

Fix: the match key of an input run is its text parts minus the image/audio tag texts (`<image>`,
`</image>`, `<image name=...>`, `<audio>`, `</audio>`, `<audio name=...>`, the upstream
`is_*_open/close_tag_text` shapes), joined with `""`. The event key is its message as is.

### 4. major: a hook prompt is emitted twice with the same entry id (shape from upstream, not observed locally)

`src/codex/messages.rs:146-157` (`hook_prompt`) together with `classify` (`messages.rs:511`).

Upstream records a hook prompt as a user `message` (`id: msg_..`, content
`<hook_prompt hook_run_id="..">text</hook_prompt>`, no passthrough, `build_hook_prompt_message`,
`protocol_src_items.rs:675`). Paginated mode also persists the `HookPrompt` item, which
`parse_hook_prompt_message` builds from that message with the same id. The message goes through `sniff` →
kind `hook_prompt` → `Notification`, and the item pushes a second entry.

Scenario (`t8-hook.jsonl`): the message `msg_h1` then the `HookPrompt` item `msg_h1`. Got: two
`user/notification` entries, both `id: msg_h1` (one with the XML text, one with the fragment text). Expected:
one entry.

Fix: index input messages by id (for example insert `Ref { entry, part: 0 }` into `items` in `input_run` when
the id exists). `hook_prompt` merges into that entry's `extra` (fragments, hook run ids) when the id is known,
and pushes only otherwise.

### 5. minor: `item_completed` times and turn id are dropped for every non-tool item (lost data, D4)

`src/codex/mod.rs:292-346`: `started_at_ms`/`completed_at_ms` are removed from `p` at lines 294-297. Only the
tool-item branch (line 340) uses them. Every other branch extends the item with a `p` that no longer holds
them. The SubAgentActivity/CollabAgentToolCall/FunctionCallOutput branch (331-339) and the fallback (344)
also drop `turn_id` and the rest of `p`.

Verified on a real rollout with subagents: the raw file has 31 `started_at_ms` values; the parsed JSON has none
for AgentMessage (5), UserMessage (1), Reasoning (6) and SubAgentActivity (4) items. Tool items keep them as
`started_at`/`ended_at`.

Fix: read the times with `p.get(..)` instead of `p.remove(..)`, remove them only in the tool-item branch,
and `item.extend(p)` before `enrich`/`other` in the branches at 331-339 and 344 (keep the `turn_id` there as
well).

### 6. minor: a `token_usage_record` whose thread total does not grow is dropped silently

`src/codex/messages.rs:382-399` (`usage_record`, line 389).

The cumulative-total gate meant to dedupe `token_count` also gates the authoritative records. Scenario
(`t5-usage-drop.jsonl`): record `r1` with `thread_token_usage.total_tokens` 500, then record `r2` with 110
(a counter that restarted). Got: `r2` usage lost, no warning, its message keeps the item id `m2` and has no
usage. Expected: both records counted, plus a warning about the non-monotonic total. A real trigger is
unverified: upstream restores totals on resume from `compacted.latest_token_usage_record`
(`history_lib.rs:300`), so resumed paginated files should grow. The silent drop still breaks "every recorded
value has one home".

Fix: never gate records. Add a `records_seen: bool` set by `usage_record`, and let `token_count` apply usage
only when `!records_seen && total > cum_total`. In `usage_record`, warn when the thread total is below
`cum_total`, then set `cum_total` to it.

### 7. minor: a tail window that starts at ordinal 1 of a child file takes the parent's identity

`src/codex/mod.rs:350-353` (`session_meta` takes the first meta it sees) with `mod.rs:193` (the ordinal
filter needs ordinal 0's `subagent_history_start_ordinal`).

A child file's line 1 is its own meta (about 23 KB here). Line 2 (ordinal 1) is a copy of the root meta. When
`parse_file_tail` starts anywhere inside line 1, the torn line is dropped and the copy becomes the identity.
Verified by cutting a real child rollout after line 1: `id` became the parent's id, `parent` became null,
and the inherited prefix (ordinals 2 to 8) was parsed as the child's own (37 entries instead of 27). Two
sessions then share one id.

Fix: accept a `session_meta` as identity only when `self.rec.ordinal` is `None` or `Some(0)`; otherwise
`other("session_meta", p)`. The id then falls back to the file name, which is the child's id.

### 8. minor: arithmetic overflow on external numbers (panic in debug builds)

`src/codex/results.rs:389` `secs * 1000 + nanos / 1_000_000`, and `src/codex/messages.rs:388`
`self.cum_total + u64_field(..)`.

Scenario (`t9-overflow.jsonl`): `duration: {"secs": 18446744073709551615}`. The debug binary panics
"attempt to multiply with overflow" at `results.rs:389`. The release binary wraps to
18446744073709550616 ms. A consumer built in debug mode (for example its tests) panics on such a line.

Fix: `secs.checked_mul(1000)?.checked_add(nanos / 1_000_000)` and `self.cum_total.saturating_add(..)`.

### 9. minor (quality): comments that lie

- `src/codex/tools.rs:386`: the doc comment is garbled by an edit:
  "`... else the argv joined and shell-quoted.|-lc, script]`, else the argv joined.`". Fix: "`command` is
  the script of `[sh, -c|-lc, script]`, else the argv joined and shell-quoted."
- `src/codex/mod.rs:76`: "Entries and parts are only ever appended". `make_compaction` (`mod.rs:609`) now
  turns an entry from Message into Compaction in place. That is safe, because only part-less entries are
  converted, but the comment should say so.

### 10. test gaps

- `tests/fixtures/codex/legacy-direct-tools.jsonl` writes the assistant `message` item before its
  `agent_message` event. Real files use the reverse order (finding 2). Reorder the fixture to event, then
  item, and assert the item id (`extra.item_ids`) and the phase.
- No test covers: a failed `apply_patch` text (1); multi-part legacy assistant content (2); a legacy image
  prompt (3); a `HookPrompt` item with its message (4); item times on messages (5); a non-monotonic record
  (6); a child tail window (7); an overflowing `duration` (8). Each is a few records with the existing
  `session()` helper in `tests/codex.rs`.

## Unverified (no local data, no conclusive upstream source)

- A yielded cell (`Script running with cell ID`) keeps running in the background. Commands it runs between
  the yield and the next `wait` are attached to `open_cells.last()`, which may be another open cell
  (`tools.rs:191-196`). Not observable in the corpus (no yielded cell).
- A `Plan` item (`mod.rs:323-328`) is added as a text part. If the same plan text is also in the response's
  assistant `message`, it appears twice.
- Paginated image prompts: whether finding 3 also hits them depends on the `content_item_kinds` given to the
  tag texts.

VERDICT: changes required
