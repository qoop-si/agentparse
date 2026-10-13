# Plan review round 2: triage

Reviews: `review-1-model-fit-round2.md` (R1 round 2), `review-3-codex-round2.md` (Codex round 2). R2 approved its
model-level points in round 1 (serde, rusqlite range, ruzstd verified by experiment).

## Fixed now

| Finding | Fix |
|---|---|
| R1 model 1, Codex 4: recorded session cost overwritten | `Session::finish` keeps a cost the parser set (`model.rs`) |
| R1 model 2: stale doc comments (`Session.id`, `ToolOutput.data`, `Origin::Agent`) | updated in `model.rs` |
| R1 model 4: Codex item ids lost on merge | plan: `extra.item_ids` |
| Codex 5: usage duplicated when a response is split | plan: usage stays on the first piece |
| Codex 6 (partly): inference of Interrupted for running work | plan 4.3: never inferred for results that say "still running" |
| Codex 7 (partly): joined argv | plan: shell-quoted join |
| Codex 8 (partly): `history_base` silently incomplete | plan: warning added |
| Codex 10: tail always drops a line | `parse_file_tail` reads one byte early and drops only a partial line |
| Codex 11: corrupt first line hides the format | `sniff` scans the first 32 lines for a JSON object |
| Codex 12: blob stripping too broad / misses ImageGeneration | `strip_blobs` only replaces base64-looking values (test with an LSP `signature`); Codex parser strips ImageGeneration `result` |
| Codex 13: DB query failures not errors; missing named checks | D8 reworded; section 8 item 5 lists the named checks |
| Codex 14: text after `*** End Patch` | `patch::parse` stops at the marker (test) |
| R1 plan items (Codex `thread_settings_applied`, legacy dedupe, clock candidates, compaction usage, single Turn for aborts, `/root` fallback; Claude `<synthetic>` placeholders, `logical_parent` map, task deltas; Gemini subagent/title; OpenCode retry placement; human-run command ids; D4 wording) | applied to section 6 / D4 |

## Rejected

| Finding | Reason |
|---|---|
| Codex 1: keep every Gemini message revision | upserts of one id are incremental saves of the same in-flight message (tokens arrive, tool calls added); the last version is a superset. The one case where later content replaces earlier content (masking) is recovered. D13 already keeps removed messages |
| Codex 2: ordered delivery list, inferred-parent provenance on code-cell children | extra deliveries append to `output.text` in order; children are attached by position between call and output, which the research measured exact for 685 of 729 cells and explains the rest; the raw event is each child's `data` |
| Codex 3: unknown vs zero token counts, completeness flags | zero is documented as "not recorded"; no consumer needs the distinction; inconsistencies already warn |
| Codex 7 (rest): separate execution and wall durations | one `duration_ms` from the measured clock, the other value stays in `data` |
| Codex 8 (rest): per-record provenance, namespaced child ids | `line` + `extra.uuids`/`extra.item_ids` locate every source record; `(agent, source)` identifies a session |
| Codex 9: Gemini MCP split ambiguity | the split is a documented heuristic; the raw name stays in `ToolCall.name` |
| R1 model 3: delete `Origin::Summary` | Claude `away_summary` produces it |
