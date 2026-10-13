# Final gate: code review round 1

Scope: every blocker/major finding and every triage-assigned minor in `code-review-{claude,codex,opencode,gemini,cross,codexcli}.md`,
checked against the current code (sources stable since 11:37:46, release binary built then). No source file was edited.

How: read the code at each place, named the covering test, reran the reviewers' repro inputs
(`scratchpad/review-{claude,codex,opencode,gemini}/`) and crafted new ones (`scratchpad/final-gate/{claude,opencode,gemini}/`)
through `agentparse show <file|id> --json`. The sources changed during the gate (11:36 to 11:37: `model.rs`, `discover.rs`,
`codex/mod.rs`, `codex/messages.rs`, `tests/codex.rs`); every gate below was rerun after that change.

## Gates

| Check | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo test` | 229 passed, 0 failed (unit 24, claude 29, codex 33, gemini 82, opencode 60, doctest 1) |
| Debug binary on the 11 Codex review cases (overflow inputs) | 11 of 11 exit 0, no panic |
| Corpus `stats`, Claude (`~/.claude/projects`) | 19,144 sessions, 0 failures, 42 warnings, 13 orphan results (was 40), interrupted 751 (was 767), 55.5 s wall, 308 MB peak RSS, p50 1.7 ms, p99 22 ms |
| Corpus `stats`, Codex / OpenCode / Gemini | 135 / 70 / 2 sessions, 0 failures, 0 warnings, 0 orphans each; OpenCode children: 6 of 6 `agent_type: general`, first prompt `origin: agent` |

## Findings

| File | # | Title | Status | Evidence |
|---|---|---|---|---|
| claude | 1 | Result line before its tool_use becomes orphan + Interrupted | FIXED | `pending` map in `record`/`results`, folded in `assistant` (src/claude.rs:412-416), leftovers via `flush_orphans`. Test `a_result_written_before_its_call_waits_for_it`. Repro `f1-result-first.jsonl` (two results before a two-line message): Read ok, Bash error exit 2, `ended_at` = result line ts, 0 warnings, 0 orphans. Corpus: orphan results 40 -> 13 |
| claude | 2 | Same-uuid lines inside a file not deduped | FIXED | `seen: HashSet` skip in `record` (src/claude.rs:153-158), one warning in `finish` (969-974). Test `history_copies_inside_a_file_are_skipped`. Repro `f2-dup-uuid.jsonl` (prompt, hook, assistant, result copied with new parent/ts/cwd): 1 hook, 1 message with 2 parts, 1 call, `ended_at` of the original, no extra Settings, warning "4 lines repeat the uuid" |
| claude | 3 | Queue replay emits phantom "queued" prompts | FIXED | `queue_op` admits only `is_human_text` to the FIFO (src/claude.rs:304-326); `<` filter gone from `flush_queue`. Test `notifications_never_enter_the_human_queue`. Repro `f3-queue.jsonl` (2 notifications, human enqueue, dequeue, notification remove, a truly unsent enqueue): no phantom, only "never sent" is `phase: queued`, title stays "start" |
| claude | 4 | Most plan rules untested | FIXED | New tests cover every listed group: `bash_result_objects` (bashEditDiff, returnCodeInterpretation, persisted output, timedOutAfterMs), `denials_and_interrupts` (permission-rule, automode-*, cancelled, parallel-call and `[Tool call interrupted` texts), `task_notifications_settle_background_shells` (failed, killed, `(exit code 1: ...)`), `nested_subagent_links_to_its_parent_agent`, `messages_from_other_agents`, `images_keep_only_their_size`, `a_mixed_user_line_splits_by_kind`, `local_commands_and_hook_outcomes`, `unparsed_tool_input_is_read_from_its_raw_text`, plus findings 1 to 3 |
| claude | 5 | `api_error` ignores the line-level `cause` | FIXED | `api_error` reads `rec.cause` `/message` then `/code` (src/claude.rs:1337-1347). Test `api_error_message_from_the_line_cause` (message "ECONNRESET") |
| claude | 6 | Continuation lines lose own parentUuid and ts | FIXED | `continuation` writes `extra.lines.<uuid>.{parentUuid,ts}` (src/claude.rs:419-448). Test `merged_lines_and_call_blocks_keep_their_own_keys` |
| claude | 7 | First-value Settings inconsistent (first cwd silent) | FIXED | `envelope` calls `setting` for every cwd incl. the first (src/claude.rs:204-210); `Session.cwd` stays the first. Test `main_session_events` expects `Settings{cwd: /work/demo}` |
| claude | 8 | take-helpers exist three times | FIXED | claude.rs imports `take_*` from `util.rs`; no local copy left in src (only `codex/json.rs::take` returning Value, `opencode::take_objects`, both different shapes). `util::take_u64` reuses `unsigned`, the `u64_field` coercion |
| claude | 9 | `finish` does six jobs | FIXED | Split into `identity` (1013) and `title_of` (1056); `finish` is 55 lines |
| claude | 10a | `read_fields` deletes `file.base64` | FIXED | Line gone (src/claude/tools.rs:387-398). Test `images_keep_only_their_size` asserts `file.base64 == {"bytes": 5}` |
| claude | 10b | `&ENVELOPE[..1]` unnamed | FIXED | `METADATA_ENVELOPE` const (src/claude.rs:46) |
| claude | 10c | Queue doc comments claim leftovers were never sent | FIXED | Comments at 303 and 1091 now true after finding 3 (repro: only the unsent text is left) |
| cross | 2 | Claude Bash exit 0 on plain success; `ShellResult.exit_code` doc | FIXED | `shell_detail` sets `Some(0)` for an Ok object result without background id, interrupt or `returnCodeInterpretation` (src/claude/tools.rs:459-474); doc at src/model.rs:493-496 names Claude and Gemini. Tests `bash_result_objects` (b5 = 0, b2 soft = None, b4 background = None), `a_result_written_before_its_call_waits_for_it` |
| cross | 7 | Per-call leftovers in `extra.calls.<id>.{call,output}` | FIXED | `call_extra` (src/claude.rs:1290-1307) for `tool_use` rest (`assistant` 406-408) and result-line rest (`absorb_result_line` 591); no `extra.results` left. The constant `caller: {type: direct}` is dropped as noise, any other caller kept. Tests `main_session_tool_calls` (calls..output.permissionDecision), `merged_lines_and_call_blocks_keep_their_own_keys` (calls.c1.call.caller) |
| cross | 9 | Interrupt marker key `marker`; Turn status words | FIXED | `extra.marker` (src/claude.rs:732); Turn status only `interrupted` (turn_duration None). Test `main_session_events` (`marker["marker"]`) |
| cross | 12 | TaskOutput/BashOutput get a ShellResult | FIXED | `task_output` fills stdout and exit code for `task_type: local_bash` (src/claude/tools.rs:400-420). Test `main_session_tool_calls` (TaskOutput16: stdout "bg-done\n", exit 0). BashOutput has no recorded result shape: the 17.9k-file corpus has one BashOutput call and it is "No such tool available", so nothing to map |
| cross | D (ctx/queued) | `origin.kind` extraction duplicated in `ctx` and `queued` | FIXED | One `take_origin` (src/claude.rs:1231-1242) used by both |
| codex | 1 | Failed `apply_patch` is Ok with file changes | FIXED | `results.rs:263-283` `patch_answer`; test `failed_apply_patch_is_an_error_without_file_changes` (6 outputs); repro `t1-patch-fail.jsonl`: status `error`, no `files`, `status_detail` = first line |
| codex | 2 | Legacy assistant dedupe keeps the event copy | FIXED | test `legacy_assistant_text_comes_from_the_response_item`; repro `t2`: one message, `phase: final_answer`, `extra.item_ids: [msg_a1]`; `t3`: one text part `Part one. Part two.` |
| codex | 3 | Legacy image prompt becomes two human messages | FIXED | test `image_prompt_copies_are_one_human_message` (legacy and paginated); repro `t4`: one human message with the attachment |
| codex | 4 | Hook prompt emitted twice with one id | FIXED | test `hook_prompt_message_and_item_are_one_entry` (both orders); repro `t8`: one `user/notification` entry `msg_h1` with fragments and times |
| codex | 5 | `item_completed` times and turn id dropped | FIXED | test `item_envelopes_keep_their_times`; real subagent rollout from the review: 11 of 11 raw `started_at_ms` values present in the output |
| codex | 6 | Non-growing usage record dropped silently | FIXED | `messages.rs:433-454` (`records_seen`, warning); test `every_usage_record_counts`; repro `t5`: usage 200/20, warning `thread total fell from 500 to 110` |
| codex | 7 | Child tail window takes the parent identity | FIXED | `mod.rs:383` (meta only at ordinal 0 or none); test `child_tail_window_keeps_the_child_identity` |
| codex | 8 | Arithmetic overflow on external numbers | FIXED | `results.rs:406` checked, `messages.rs:447` saturating; test `huge_recorded_numbers_do_not_overflow` (debug); debug binary on all 11 review cases rc 0, `t9` gives `duration_ms: None` with raw kept in `data` |
| codex | 9 | Garbled / stale comments | FIXED | `tools.rs:377` one sentence; `mod.rs:72-73` states the in-place kind change |
| codex | 10 | Test gaps, fixture order | FIXED | `legacy-direct-tools.jsonl` lines 11-12 now event then item; one test per finding (`tests/codex.rs:1404-1777`) |
| codex | unverified a | Commands of a yielded cell attach to the wrong cell | FIXED | `tools.rs:196-201`; test `yielded_cell_keeps_its_background_commands` |
| codex | unverified b | `Plan` item text read twice | FIXED | test `plan_item_is_not_read_twice` |
| codexcli | 1 | Repeated waits lose the original exec | FIXED | `tools.rs:105-133` (`waits` map, yield keeps the root exec); test `repeated_waits_stay_on_the_cell_that_yielded` |
| codexcli | 2 | Failed patch is Ok (same as codex 1) | FIXED | see codex 1 |
| codexcli | 3 | Duration overflow (Codex part) | FIXED | see codex 8 |
| codexcli | 7 | Legacy event dedupe loses the item id | FIXED | repro `t2`: `item_ids: [msg_a1]` |
| codexcli | 9 | Unreadable session dir looks empty | FIXED | `discover.rs:120-133` (non-NotFound read errors become warnings); test `unreadable_dir_is_a_warning` (`cfg(unix)`). Per-entry readdir errors are still skipped by `e.ok()`, negligible |
| codexcli | 10 | `parse_file_tail(path, u64::MAX)` overflow | FIXED | `lib.rs:70` `saturating_add(1)`; no dedicated test (one-line guard) |
| cross | 1 | `cargo fmt --check` fails | FIXED | `cargo fmt --check` rc 0 |
| cross | 2 | One exit-code rule (doc) | FIXED | `model.rs:494-496` doc; Claude side in the Claude rows |
| cross | 6 | Interrupt inference in three copies | FIXED | `model.rs:771-795` `interrupt_abandoned_calls`; `close_calls`, `mark_interrupted`, `answered`, `last_model_slot`, `has_later`: 0 hits in `src`; unit test `finish_interrupts_only_abandoned_calls` |
| cross | 8 / D1 | Three private `take_*` sets | FIXED | one set `util.rs:201-249` (lenient, wrong type left in place); unit test `take_fields_leniently`; module copies gone (Codex keeps `take` returning `Value`, OpenCode keeps `take_nested`/`take_span` on top, as the review allowed) |
| cross | 11 | Codex own inter-agent message is `Origin::Agent` | FIXED | `messages.rs:186-197`; test `subagent_child_drops_its_inherited_prefix` asserts own message is `Assistant/Model`, no `Assistant/Agent` |
| cross | 12 | `ShellResult` only on Shell/Process (Codex part) | FIXED | `exec` cells: 8 of 8 in the fixtures have no `output.shell`. The asked doc line on `ToolOutput.shell` (`model.rs:472`) is missing (non-blocking) |
| cross | 15 | `FileChange.diff` doc too loose | FIXED | `model.rs:524-525` |
| cross | 17 | API/CLI items | FIXED | `lib.rs:67` `let Some(agent @ ..) else`; `discover.rs:59-60` `parent_id` doc; `main.rs:103-107` newest copy by `updated_at`; `main.rs:249` `N turns rolled back`; `parse_storage_file` visibility in the OpenCode rows |
| cross | MR1-3 | `ToolCall.extra`, `Tool::Patch{Option}`, Gemini pgid fields | WONTFIX-by-triage | rejected in triage |
| cross | D2 | `Message::new` / `ToolCall::new` | FIXED | `model.rs:825, 853`; every literal in the parsers uses `..X::new` except `opencode/tool.rs:307`, which sets every field (fine) |
| cross | D3 | One data-URL-to-Media | FIXED | `util.rs:283` used by Codex (`messages.rs:627`) and OpenCode (`record.rs:179`); unit test `data_url_media` |
| cross | D4 | Interrupt pass in `finish` | FIXED | see cross 6 |
| cross | D5 | One Codex file-name parser | FIXED | `codex/mod.rs:731` `thread_id`, called from `discover.rs:77` |
| cross | D6 | One diff-stat counter | FIXED | `util.rs:310` used by `patch.rs`, `claude/tools.rs`, `codex/results.rs`, `gemini/tool.rs`; unit test `diff_stat_skips_only_file_headers` |
| cross | D7 | `ShellResult::failed()` | FIXED | `model.rs:880`, used by Claude and OpenCode (`claude/tools.rs:475`, `opencode/v2.rs:245, 286`) |
| cross | D8 | One extra builder | FIXED | `util.rs:252` `as_extra` used by all four parsers; Claude keeps its own null pruning (`claude.rs:1576` `clean`) on top, which the review measured as worth it only there |
| cross | mcp | `mcp__server__tool` split | FIXED | `util.rs:262` `mcp_tool` |
| cross | dead | `<synthetic>` filter in `finish`, Codex garbled comment | FIXED | no `<synthetic>` in `model.rs`; `Origin::is_synthetic` kept per triage |
| opencode | 1 | migrated sessions repeat synthetic text | FIXED | `opencode.rs:137-146` 16-char prefix filter; test `migrated_synthetic_split_is_not_repeated`; repro `dup.db` session `ses_ed52a75f...`: synthetic text once (`#2`), no copy before the notice |
| opencode | 2 | background completion failures reported Ok | FIXED | `v2.rs:163-193, 237-252`; test `background_completions_cancelled_error_timeout_and_signal`; repro scen: cancel=interrupted, timeout=error+timed_out, err=error, status_detail = state, `truncated` OR-ed |
| opencode | 3 | `ParentLink.agent_type` None for v2 children | FIXED | `db.rs:228-285` agent from the spawning call, title fallback; fixture child title now bare; test `v2_native_child_links_to_the_subagent_call`; live DB: 6/6 children `agent_type: general`, call_id set |
| opencode | 4 | v2 shell notices in stdout, signal kill Ok | FIXED | `tool.rs:166-252` `notice_line` (4 forms), `clean_stream` strips the timeout sentence (wording matches upstream `shell.ts:230`); `ShellResult::failed` includes signal; tests `v2_shell_notices_stay_out_of_stdout`, `background_completions_...`; repro scen: `sig` error+SIGKILL stdout `y\ny`, `fgtimeout` stdout `partial`, `exit2` stdout `error: no rule` exit 2 |
| opencode | 5 | completion with `">` in the command corrupts stdout | FIXED | `v2.rs:174-207` header rebuilt from the row, `">\n` fallback; test `background_completion_of_a_command_containing_a_quote_bracket`; repro scen `quote`: stdout `""` |
| opencode | 6 | `location-switched` gives empty Settings | FIXED | `v2.rs:481-484`; test `location_switched_carries_the_directory`; repro scen: `Settings{cwd: "/new/dir"}` |
| opencode | 7 | denial detection narrower than upstream | FIXED | `tool.rs:351-361` `contains` over the 4 upstream phrases; test `denial_phrases_follow_opencodes_own_classification` |
| opencode | 8 | v1 `<metadata>` abort Ok; notice split mid-stream | FIXED | `tool.rs:212-238` end-anchored, 3 tag names; test `v1_tag_handling_is_anchored_to_the_end_of_the_output`; repro scen `01ULID`: interrupted, stdout `line1` |
| opencode | 9 | Running user shell row inferred Interrupted | OPEN (regression) | Fixed by the implementer (test `a_running_user_shell_row_is_pending_even_with_later_rows`, which has only a later `user` row), then reopened at 11:36 by the widened shared rule `model.rs:773-780` (metadata-only output is no result). Repro `final-gate/opencode/running/run.db`: running `!tail -f x`, then a user row, then an assistant row: call `sh_u` comes out `interrupted` |
| opencode | 10 | recorded session cost ignored | FIXED | `opencode.rs:165-171` (0 default falls back to the sum); test `recorded_session_cost_wins_unless_it_is_the_zero_default`; fixture v1 main: usage cost 0.0179 = column |
| opencode | 11 | one session read from several snapshots | FIXED | `opencode.rs:61-62, 117-119` `unchecked_transaction` in `list` and `parse_db`; read-only verified, race not testable |
| opencode | 12 | duration subtraction overflow | FIXED | `record.rs:64-66` `span_ms`, used at `tool.rs:343`, `v2.rs:242, 279`, no raw subtraction left; test `extreme_timestamps_do_not_overflow_durations` (debug build) |
| opencode | 13 | JSON-tree message/part file parses as a session | FIXED | `opencode.rs:191-199`, warning in `db.rs:338-340`; tests `storage_message_and_part_files_are_not_sessions`, `json_tree_session_without_a_message_directory_warns`; CLI on fixture message and part files: `not a session` |
| opencode | 14 | extra compaction summaries dropped | FIXED | `v1.rs:48-60, 90` `take_first`, error via `failure_entry` (`v1.rs:269`); test `compaction_keeps_only_the_first_summary_and_reports_its_error` |
| opencode | 15 | v1 `!cmd` entry id, marker keys dropped | PARTIAL | id = marker id, `extra.message_id`, `extra.user` done (`v1.rs:173-220`, test `v1_user_run_command_pair_is_one_command_message`); the marker's `model.modelID` is still taken and thrown away (`v1.rs:183`) |
| opencode | 16 | v2 user file `description`/`mention` dropped | FIXED | `v2.rs:71-80, 100-114`; test `v2_user_file_description_and_mention_are_kept` |
| opencode | 17 | partial tool input in different places | FIXED | `tool.rs:55-63`; test `v1_interrupted_and_pending_tools` (input = partial JSON string, no output) |
| opencode | 18 | test gaps | FIXED | each listed gap now has a test: migrated split, location-switched, completions (cancelled/error/timeout/signal/quote), v2 notices, v1 `<metadata>` abort, bare child title, real denial strings, `subtask_parts_stay_in_place`, multiple summaries, JSON-tree message file |
| opencode | 19 | duplicated helpers | FIXED | `media_from_url` shared in `util.rs:283`; map clones gone (`v1.rs:65`, `db.rs:344-348`); `time_of` used for every `time.created` chain; `span_ms` once. `OPAQUE_KEYS` (`record.rs:11`) kept on purpose: `has_opaque`/`prune_bookkeeping` need only the encrypted keys, `BLOB_KEYS` also holds `base64` |
| opencode | 20 | misleading comments | FIXED | `opencode.rs:8-11` states the synthetic rule; `tool.rs:166-168` lists all notice forms; `tests/opencode.rs:1-4` says later tests build rows inline |
| codexcli | 3 | OpenCode duration overflow | FIXED | same as opencode 12; raw timestamps stay in `started_at`/`ended_at` |
| codexcli | 4 | base64 in tool input | FIXED | `tool.rs:255` `strip_blobs(&mut raw.input)` before `classify`; test `tool_input_blobs_are_sized_before_the_typed_view` |
| codexcli | 5 | text/reasoning part leftovers dropped | FIXED | `record.rs:115-136`, `v1.rs:363-379`, `v2.rs:338-346, 388-396` to `Entry.extra.parts.<position>`; test `text_and_reasoning_leftovers_are_kept_by_part_position`; provider item ids and phase pruned as bookkeeping (accepted implementer decision) |
| codexcli | 6 | one read snapshot | FIXED | same as opencode 11 |
| cross | 3 | child first prompt `Origin::Agent` | FIXED | `opencode.rs:237-259` `finish_session` (DB and JSON tree); test `cross_review_origin_turn_cwd_and_trigger`; live DB 6/6 children first user `origin: agent` |
| cross | 4 | prompt `turn_id` = own id | FIXED | `v1.rs:157-161`, `v2.rs:88-95`; same test; live DB first prompt `turn_id` = own id |
| cross | 5 | `Shell.cwd` falls back to session dir | FIXED | `opencode.rs:250-255`; same test (bg call and human shell get `/work/demo`). `location-switched` does not update it, triage 5 did not ask for that |
| cross | 9 | Turn `succeeded` -> `completed` | FIXED | `v2.rs:440-444`; abort marker in `extra.marker` (`record.rs:251-254`) |
| cross | 10 | cost = recorded session cost | FIXED | same as opencode 10 |
| cross | 13 | Human-run command input as recorded | FIXED | `v2.rs:298` `Value::String(command)`; the `Message.kind: "shell"` half was not adopted by triage 13 (input only) |
| cross | 14 | v1 compaction trigger only when recorded | FIXED | `v1.rs:230-232`; same test (no `auto` -> None), fixture `auto: true` -> `auto`, test `compaction_keeps_only...` `auto: false` -> `manual` |
| cross | 16 | `strip_blobs` on inputs | FIXED | same as codexcli 4 |
| cross | 17 | `parse_storage_file` not pub | FIXED | `opencode.rs:180` `pub(crate)` |
| gemini | 1 | Truncated shell result loses exit code and status | PARTIAL | `t11.jsonl` now `exit_code 2`, `status error`, `process_group 9`; test `shell_truncated_output_keeps_its_exit_code_and_saved_path`. Still broken for v0.49+ text, where the banner precedes `<untrusted_context>` (shell.ts wraps, then tool-executor truncates): `final-gate/gemini/trunc-untrusted.jsonl` gives `status ok`, no exit code, wrapper and `Exit Code: 2` in stdout |
| gemini | 2 | Rejected/cancelled edits reported as file changes | FIXED | `t2.jsonl` c2 (cancelled): `files []`, diff kept in `data.resultDisplay.fileDiff`; `tool.rs` gates `resultDisplay` on `status == Ok` (`gemini.rs:543`); test `cancelled_edit_reports_no_file_change_but_keeps_the_diff_in_data` |
| gemini | 3 | Tail peeling eats `Error:` stdout lines; spawn error has no home | FIXED | `t1.jsonl` c1: stdout keeps `Error: Process completed...`, exit 1; `split_tail` peels `Error:` only after `Output: (empty)` or in the old era; error stored in `data.error`; tests `shell_stdout_ending_in_an_error_line_keeps_it`, `shell_spawn_error_line_is_stored` (see new item N1) |
| gemini | 4 | Rewind entries emitted for UI notices | FIXED | `t8.jsonl`: `removeIds:[w1]` gives no Rewind; `drop_ids` filters `info/warning/error` for removeIds, `$rewindTo`, `$set` (`reduce.rs:195-216`); test `notices_removed_by_the_sync_leave_no_rewind` |
| gemini | 5 | Echo binary parts duplicated as user message; `info` binary dropped | FIXED | `t3.jsonl`: one attachment on the call, no `tool_attachment` message, `i1#media` `binary_injection` message with `image/png 11 B`; tests `binary_echo_parts_are_not_a_user_message_and_not_duplicated`, `binary_echo_parts_reach_a_call_without_a_result`, `binary_parts_of_an_orphan_echo_stay_on_its_result`, `info_message_with_binary_parts_keeps_them` |
| gemini | 6 | `<state_snapshot>` by `contains` swallows prompts | FIXED | `t7.jsonl`: prompt stays human, no compaction; `Class::of` needs both tags plus a leading tag or the ack (`gemini.rs:103-117`); tests `a_prompt_mentioning_the_snapshot_tag_stays_a_prompt`, `a_snapshot_not_at_the_start_counts_when_the_ack_follows` |
| gemini | 7 | Quadratic `is_live` | FIXED | `live_set` mirror (`reduce.rs:64`); `g3_80000.jsonl` 0.33 s user (was 7.06 s), 20k/40k/80k = 0.08/0.16/0.33 s, linear; test `many_bare_records_reduce_in_linear_time` |
| gemini | 8 | Policy denial and `Reason:` prefix | FIXED | `t8.jsonl`: `rejected`, `cancelled: User denied execution.`; `cancel_reason` strips `Reason:`, `is_denial` = `denied` or policy error; tests `denied_calls_are_rejected_with_the_reason`, `user_aborts_are_interrupted_not_rejected` |
| gemini | 9 | FileChange counts misread `++`/`--` lines | FIXED | `t2.jsonl` c1: +2 -1 (was +1 -0); shared `util::diff_stat`; tests `file_diff_counts_hunk_lines_only` (CRLF), unit `diff_stat_skips_only_file_headers` |
| gemini | 10 | Shell edge cases (background phrase, before-start cancel, signal kill) | FIXED | `t1.jsonl` c4: no background id, exit 2; `t9.jsonl` c1: signal SIGTERM, `error`; c2: `interrupted true`, stdout None; tests `shell_output_mentioning_the_background_sentence_is_not_a_background_start`, `shell_cancelled_before_start_and_signal_kills` |
| gemini | 11 | Masked sibling flags calls truncated | FIXED | `t10.jsonl`: cA intact not truncated, cB truncated; `is_masked(result, call_id)` (`reduce.rs:349`); test `a_masked_sibling_does_not_mark_a_call_truncated` |
| gemini | 12 | Synthesised ids for id-less snapshot entries | FIXED | `replace_all` skips with a warning (`reduce.rs:162-170`); test `snapshot_entries_without_an_id_are_skipped_with_a_warning` |
| gemini | 13a | `thoughts[].timestamp` dropped | FIXED | documented choice in `part.rs:146` |
| gemini | 13b | `orderIds` repeated id duplicated | FIXED | `placed` set (`reduce.rs:256-262`); test `patch_remove_and_order_ids` (`["a","a"]`) |
| gemini | 13c | Patch order (`updates` before the patch) | FIXED | patch first, then updates (`reduce.rs:226-232`) |
| gemini | 13d | Header `messages` replace instead of merge | OPEN (nit) | unchanged (`reduce.rs:120-124`); no effect while the header is line 1, non-blocking |
| gemini | 13e | `read_file` offset base ambiguity undocumented | FIXED | comment `tool.rs:47-48` |
| gemini | Q1 | File size past ~1500 lines | FIXED | split into `gemini.rs` 690 + `reduce` 361, `tool` 383, `shell` 178, `part` 177 |
| gemini | Q2 | Dead `Class::Snapshot` arm | FIXED | `Class::label` returns `None` for Snapshot, which is the live Compaction path (`gemini.rs:120-129, 340`) |
| gemini | Q3 | `&MAPPED_MESSAGE_KEYS[..4]` coupling | FIXED | named `RECORD_KEYS` / `MODEL_KEYS` (`gemini.rs:36-38`) |
| gemini | Q4 | `Part::Other.kind` = alphabetically first key | FIXED | `OTHER_PART_KEYS` lookup else `unknown` (`part.rs:110-142`); test `unmodeled_parts_are_named_by_their_real_key` |
| gemini | Q5 | `crate::ErrorInfo` by path | FIXED | imported at top |
| gemini | Q6 | `read_response` overwrites sibling text | FIXED | `append` (`tool.rs:238`); test `sibling_text_before_the_function_response_is_kept` |
| gemini | Q7 | `originalContent`/`newContent` held twice | FIXED | write's `newContent` equal to `content` removed, diff/path removed when taken; doc `tool.rs:336-338`; test `write_file_content_is_not_kept_twice_in_data` |
| gemini | Q8 | Long `parse_shell` / `Build::call` | FIXED | `parse_shell` split into `aborted`/`structured`/`split_tail`/`strip_truncation_banner` |
| gemini | Q9 | Stale `is_denial` doc | FIXED | doc names the two real refusal strings (`tool.rs:288-289`) |
| gemini | T | Section 3 test gaps | PARTIAL (non-blocking) | almost every listed case now has a test (see names above plus `parallel_calls_without_ids_pair_by_name_in_order`, `echo_turn_with_several_responses_folds_every_call`); invented `Output: Output too large` and `Denied by policy.` gone; invented reason `User cancelled the operation.` still in fixture `tests/fixtures/gemini/gemini-new-session.jsonl:11` and `tests/gemini.rs:421` |
| codexcli | 8 | Pending call with `result: null` loses metadata | FIXED | metadata kept in `output.data` (test `pending_call_keeps_its_metadata_without_a_result`); after the 11:36 change to `model.rs:773-780` (unit test `finish_interrupts_only_abandoned_calls`, case `meta`) an abandoned call with metadata becomes Interrupted: `final-gate/gemini/abandoned.jsonl` c1 and c2 both `interrupted`. That shared change reopened opencode 9 |
| cross | 5 | Shell.cwd falls back to session dir (Gemini side) | FIXED | `default_shell_cwd` after `project_root`/context workspace (`gemini.rs:77-78, 630-646`), `Directory:` line still wins; test `shell_without_a_directory_runs_in_the_session_directory` |
| cross | MR4 | Echo binaries to `ToolOutput.attachments`, `Class::ToolAttachment` removed | FIXED | no `ToolAttachment` in src; `echo_media` + `Resp::add_media` dedupe by (mime, bytes); `t3.jsonl` |
| cross | dead | Unreachable `Class::Snapshot` arm | FIXED | same as Q2 |
| gemini | N1 (new) | Spawn error with recorded `success` gets exit 0 | OPEN (minor) | `final-gate/gemini/spawn.jsonl` (`Output: (empty)\nError: spawn nope ENOENT`): `exit_code 0`, `status ok`. shell.ts prints `Error:` only when `result.error` is set, and no exit code exists then; test `shell_spawn_error_line_is_stored` asserts the wrong `Some(0)` |

Counts: 118 checked, 111 FIXED, 3 PARTIAL, 3 OPEN, 1 WONTFIX-by-triage.

## Required before approval

1. **opencode 9, regression.** `src/opencode/v2.rs` `v2_shell`: a running user `!` shell has no text and no
   `ShellResult` yet, so the widened shared rule (`model.rs:773-780`, "metadata alone is not a result") marks it
   Interrupted once an assistant row follows. Upstream starts a user shell as a background shell, so say that in the
   result instead of narrowing the shared rule (which Gemini now relies on):
   ```rust
   shell: Some(ShellResult {
       background_id: (!finished).then(|| shell_id.clone()),
       ..result
   }),
   ```
   in place of `shell: finished.then_some(result)`. Add an `assistant` row after the `user` row in
   `a_running_user_shell_row_is_pending_even_with_later_rows` and assert `Pending` plus `background_id == "sh_u"`.
2. **gemini 1, partial.** `src/gemini/shell.rs:48-49`: since v0.49 `shell.ts` wraps the text in `<untrusted_context>`
   and the tool executor truncates afterwards (`tool-executor.ts:429, 485`, `formatTruncatedToolOutput`), so the banner
   is outermost. Repro `final-gate/gemini/trunc-untrusted.jsonl`: `status ok`, no exit code, wrapper and
   `Exit Code: 2` in stdout. Fix: strip the banner first, then unwrap:
   ```rust
   let unwrapped = unwrap_untrusted(strip_truncation_banner(text));
   let body = unwrapped.as_ref();
   ```
   Add the wrapped variant to `shell_truncated_output_keeps_its_exit_code_and_saved_path` (assert exit 2, `Error`).
3. **opencode 15, partial.** `src/opencode/v1.rs:183` takes the marker's `model.modelID` and throws it away (it lands
   neither in `Message.model` nor in `extra.user`). Fix: `let model = take_nested_str(&mut user_data, "model", "modelID");`
   and at `v1.rs:215` `Message { agent, model, ..Message::new(Role::User, Origin::Command, parts) }`, the rule
   `user_messages` already uses. Assert it in `v1_user_run_command_pair_is_one_command_message`.
4. **gemini N1, new minor.** `src/gemini/shell.rs:132` infers exit 0 after a peeled spawn error
   (`Output: (empty)\nError: spawn nope ENOENT`): no process ever exited. Fix:
   `if ok && tail[EXIT_CODE].is_none() && tail[SIGNAL].is_none() && tail[ERROR].is_none()`, and make
   `shell_spawn_error_line_is_stored` expect `exit_code == None` (upstream records that call as `error` anyway,
   `shell.ts:1113-1120`).

## Non-blocking

- Doc lines: `ToolOutput.shell` (`model.rs:472`) "Shell and Process calls only" (cross 12); `util::take_bool` doc says
  "not a bool" but it also reads `"true"`/`"false"` strings; `plans/agentparse.md:416` add "minus `synthetic` rows
  sharing a v1 id's first 16 characters" (opencode 1); plan 4.3 shell rule (`plans/agentparse.md:277`) add `signal`
  (opencode 4).
- Gemini test text: the invented reason `User cancelled the operation.` is still in
  `tests/fixtures/gemini/gemini-new-session.jsonl:11` and `tests/gemini.rs:421`; use `User cancelled tool execution.`.
- gemini 13d (header `messages` replace instead of merge): no effect while the header is line 1; leave it.

## Cleanliness

The crate is in good shape: one set of JSON/media/diff/extra helpers in `util.rs`, shared `Message::new`,
`ToolCall::new`, `FileChange::new`, `ShellResult::failed` and the interrupt pass in `model.rs`, no `unwrap` outside
tests, one lint allow (`large_enum_variant`), every module under ~800 lines except `claude.rs` (1,589, already split
with `claude/tools.rs`). Small things an owner would still trim, none blocking:

- `opencode/v1.rs:34-46` `take_where` returns indices its only caller drops (`for (_, m) in pair`); return `Vec<V1Message>`.
- `opencode/tool.rs:276` `raw.metadata.remove("signal")` only ever discards a value; delete the line.
- `opencode/tool.rs:10-13` `ToolCtx` is a one-field struct now always `None` on the v2 and v1-user paths; an `Option<&str>` does.
- `opencode/v1.rs:359-361` repeats the `id`/`sessionID`/`messageID` removal `record.rs:70` already does.
- `opencode/v1.rs:200`: when the marker has `agent`, the assistant's `agent`/`mode` stay in `extra` too (two homes).
- `gemini/tool.rs:345-351` `call_data` repeats the key filter of `gemini/part.rs:170` `unmapped`.
- `main.rs`: `agentparse stats --help` runs the full stats instead of printing the usage.

Round 1 verdict: changes required (items 1 to 4; 1 and 2 are wrong output on real shapes, 3 and 4 are one-line fixes).

## Round 2

I checked the code as it stood at 11:49 (last edit `src/opencode/v1.rs`). I rebuilt the release and debug binaries from that state.

| Check | Result |
|---|---|
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo test` | 230 passed (unit 24, claude 29, codex 33, gemini 82, opencode 61, doctest 1) |
| Debug binary on every repro input (Codex cases, Gemini t1 to t11, final-gate Claude and Gemini files) | 0 failures, 0 panics |
| Corpus `stats` | Claude 19,192 sessions, Codex 135, OpenCode 70, Gemini 2: 0 failures. Claude still has 42 warnings, 13 orphan results and 751 interrupted calls; the other three have 0 warnings and 0 orphans |

| Item | Status | Evidence |
|---|---|---|
| 1 Running user shell (opencode 9) | FIXED | `running/run.db`: `sh_u` is `pending`, `status_detail: running`, `shell.background_id: sh_u` |
| 2 Truncated wrapped Gemini shell (gemini 1) | FIXED | `trunc-untrusted.jsonl`: `error`, exit 2, stdout `head line ... tail line`, `truncated`, saved path, `process_group: 9` |
| 3 v1 `!cmd` marker model (opencode 15) | FIXED | `v1.rs:182, 215`. Test `v1_user_run_command_pair_is_one_command_message` asserts `model == demo-gpt`. In the v1 fixture DB the command entry has `model: demo-gpt` |
| 4 Spawn error exit code (gemini N1) | FIXED | `spawn.jsonl`: no `exit_code`, `data.error` kept. `spawn-error.jsonl` (the real `error` status): `error`, no exit code |
| codexcli-final 1 Gemini metadata-only output | FIXED | `model.rs` `produced` check (unit test case `meta`). `abandoned.jsonl`: c1 (has metadata) and c2 (bare) are both `interrupted`, and c1's metadata stays in `data` |
| codexcli-final 2 Buffered legacy event context | FIXED | `AgentEvent` keeps its own `rec` and `turn` (`codex/mod.rs:105-111`). Test `unmatched_legacy_event_keeps_its_own_time_and_line` |
| codexcli-final 3 Per-entry `read_dir` errors | FIXED | `discover.rs:124-133` warns on each failed entry (hard to provoke in a test, checked by reading) |
| Doc lines (`ToolOutput.shell`, `take_bool`, plan 277 and 416) | FIXED | `model.rs:472`, `util.rs:231`, `plans/agentparse.md:277, 416` |
| Invented Gemini cancel reason | FIXED | 0 hits for `User cancelled the operation` in `tests/` |
| `take_where` indices, `remove("signal")`, `ToolCtx` | FIXED | `take_where` returns `Vec<V1Message>`. `remove("signal")` and `ToolCtx` are gone (0 hits) |
| `call_data` vs `unmapped` | FIXED | both use the shared `rest` (`gemini/part.rs:178`, `gemini/tool.rs:346`) |
| `stats --help` | FIXED | prints the usage, exit 0 |
| `agent`/`mode` two homes (`v1.rs:197`) | FIXED | `take_agent` removes them from the assistant data. Fixture command entry: `agent: build`, no `agent`/`mode` left in `extra` |
| id/sessionID/messageID strip in `v1_part` | Kept, accepted | Removing it leaked JSON-tree part ids. It now carries a comment (`v1.rs:356`) and test `json_tree_part_ids_do_not_leak_into_extra_or_data`. The comment explains why it is not the same strip as `record.rs:70` |

Nit, not blocking: in `command_entry` (`v1.rs:197-198`), when the marker and the assistant name different agents, the
assistant's agent is dropped with no home. In practice they are the same value. If it ever matters, keep it in `extra.agent`.

VERDICT: approve
