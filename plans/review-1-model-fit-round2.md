# Review 1, round 2: model fit (plan v2 + src/model.rs)

Checked: `agentparse.md` v2, `review-triage-1.md`, `src/model.rs`, `src/lib.rs`, `src/util.rs`, `src/patch.rs`.

## Round 1 blocker/major status

Fixed in plan v2 and expressible in model.rs: 1, 2, 4, 5, 6, 7, 10, 12, 15, 16, 17, 25, 26, 27, 28 (as "Other", accepted),
33. Accepted as rejected or narrowed: 24 (offsets as recorded), 28 (no config read).
Fixed in the plan, but model.rs still says the old rule: 3/11 (session cost), 13 (`data`), 34 (child id). Items 1 to 3
below.
Partly fixed: 18 (Codex clock), 19 (signal row), 10 (Codex compaction usage). Items 7 to 9 below.

## Open items

Items 1 to 4 are at the model level. Fix them before the parsers land, because the implementers treat `model.rs` as
read-only truth.

1. **[major] `Session::finish()` throws away the recorded session cost that v2 asks for.**
   Plan section 3 and 4.1 say Claude `Session.usage.cost_usd` = the latest `cost-state`. But `finish()` sets
   `self.usage = usage` unconditionally, and the doc says "Sum of every message's usage".
   Change in `finish()`: `let recorded = self.usage.cost_usd;` before the loop, then `usage.cost_usd = recorded.or(usage.cost_usd);`.
   Change the doc of `Session.usage` to: "Sum of message and compaction usage; `cost_usd` is the product's recorded
   session cost when one exists, else the sum of message costs."

2. **[major] Stale doc comments in model.rs contradict plan v2.**
   - `Session.id`: "Claude subagent files use their file stem (`agent-<id>`)". Change to "Claude child = `agentId`,
     Codex child = `session_meta.payload.id`".
   - `ToolOutput.data`: "(Claude `toolUseResult`, OpenCode `metadata`, Gemini `resultDisplay`, Codex completion event)".
     Change to the plan 4.7 text: "Claude `toolUseResult`; Codex the completion item(s); OpenCode the tool `state` minus
     `input`/`output`; Gemini the `ToolCallRecord` minus `args`/`result`. Mapped keys removed, base64 as `{bytes}`."
   - `Origin::Agent`: "a child's inherited prefix". Plan section 6 Codex drops the inherited prefix. Delete that clause.
   - Module header: "The raw record is one step away through `Entry::line`". Add "unmapped keys are in `extra`/`data`".
     Otherwise it reads as if nothing is kept.

3. **[minor, cheap now] `Origin::Summary` has no producer in v2.**
   Plan 4.2 maps Claude `away_summary` (a `system` line) to `Notice`, compaction summaries are absorbed into
   `Compaction`, and no other product has a resume summary message. Removing an enum variant later breaks consumers.
   Change: delete `Origin::Summary`. Re-add it only if P2 shows that `away_summary` text reaches the model.

4. **[minor] Codex merged item ids have no home (D4 "one home per value").**
   A response Message takes `Entry.id` = `response_id`. The item ids (`msg_`, `rs_`, `ctc_`, `fc_`) are lost, and the
   section 8 oracle (`thread_history_1.sqlite` `item_id`) needs them.
   Change in section 6 Codex: "merged item ids go to `extra.item_ids`, in part order (same rule as Claude
   `extra.uuids`)".

5. **[minor] Codex `thread_settings_applied` is unmapped.**
   Research-codex 3.4: one per child file, the child's first model, effort and cwd.
   Change in section 6 Codex: "`thread_settings_applied` -> `Settings`". Also state in section 3 `Settings`: "the first
   observed value counts as a change", so the initial approval and sandbox are not lost.

6. **[minor] Codex legacy assistant dedupe is unstated.**
   Research-codex section 9: in legacy mode, `event_msg agent_message`/`agent_reasoning` and `response_item` carry no
   shared id. The v2 rule "same id emit once" does not catch them, so text is emitted twice.
   Change in section 6 Codex: "legacy: assistant text and reasoning come from response items only; the
   `agent_message`/`agent_reasoning` events are skipped (they go to `extra` only if their text differs)".

7. **[minor] Codex shell clock: the candidate list misses the simplest clock.**
   Change in section 6 Codex: candidates = `item_completed` envelope `completed_at_ms - started_at_ms`, event
   `duration`, chunk `wall_time_seconds`. Children always take `started_at`/`ended_at` from the envelope ms fields.

8. **[minor] Section 4.5 still has no signal row.**
   Add a row "signal": OpenCode v2 `metadata.signal`; Gemini `^Signal: (.+)$` ignoring `(none)`; Claude and Codex none.

9. **[minor] Where Codex compaction usage goes is unstated.**
   Change in section 6 Codex: "a `token_usage_record` whose `response_id == compacted.compaction_response_id`, or one
   that follows no assistant items, attaches to the `Compaction` (or to an `Other` when no compaction is near), never to
   the next response". Note that the `total()` check cannot settle whether `cache_write` is inside `input_tokens`: both
   readings give `input + output`. Leave the formula as is and mark it unverified until a record with `cache_write > 0`
   exists.

10. **[minor] The Claude `<synthetic>` rule mislabels fillers.**
    Research-claude section 6: 153 `<synthetic>` messages say "No response requested." and are not errors.
    Change in section 6 Claude: "`isApiErrorMessage: true` -> `Error`; a `<synthetic>` message without it -> `Notice
    { kind: "synthetic" }`".

11. **[minor] `Compaction.logical_parent` must be resolved like `parent_id`.**
    `logicalParentUuid` is a line uuid that may point at a folded tool_result line or a merged chunk.
    Change in section 6 Claude: "resolved through the same line-uuid map".

12. **[minor] Codex `<turn_aborted>` would produce two Turn entries.**
    Plan 4.2 lists both the developer message and the event. Change: "the `turn_aborted` event -> `Turn`; the
    `<turn_aborted>` developer message stays a `Role::System` message with kind `generic.turn_aborted` (the model saw
    it)".

13. **[minor] Gemini subagent link and title are missing from section 6 Gemini.**
    Add: "child = `chats/<parentId>/<agentId>.jsonl` with header `kind: subagent`; `ParentLink.session_id` = the parent
    directory name; the parent call `child_session` = `toolCall.agentId`; `title` = `summary` when set".

14. **[minor] Codex `agent_message` role for a root thread.**
    A root `session_meta` has no `agent_path`. Add: "this session's agent path = `agent_path`, else `/root`".

15. **[minor] The Todo row is ambiguous for Claude task tools.**
    Row text: "task list tools that write the list". `TaskCreate`/`TaskUpdate` are deltas, not the full list.
    Change: Todo = full-list writers only (`TodoWrite`, `todowrite`, `write_todos`, `update_plan`); `TaskCreate`,
    `TaskUpdate`, `TaskList`, `TaskGet` -> Other.

16. **[minor] OpenCode `retry` part placement.**
    Plan: "`retry` part -> `Error { retry }`" from inside a message. Say "an `Error` entry right after the message", as the
    plan already does for assistant `error`.

17. **[minor] `ToolCall.id` for human-run commands has no source.**
    Claude `<bash-input>` and OpenCode v2 `shell` rows carry no call id. Add: "id = the source record id (Claude line
    uuid, OpenCode `shellID` or row id, Codex item id)".

18. **[minor] D4 says bodies are never stored twice, but shell output is.**
    `ToolOutput.text` and `ShellResult.stdout` hold the same text for OpenCode, for most Claude results (stderr empty) and
    for Codex direct calls. Keep both (consumers can rely on `stdout`). Reword D4 to "`data`/`extra` never repeat a typed
    value" so the plan does not claim more than it does.

## Section 6 vs research

No contradictions remain beyond items 5, 6, 10, 12 and 13 above.

VERDICT: changes required (items 1 to 3 are small edits to model.rs; the rest are plan text the implementers can apply in P2)
