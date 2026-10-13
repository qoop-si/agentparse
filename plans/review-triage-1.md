# Plan review round 1: triage

Reviews: `review-1-model-fit.md` (R1, 37 findings), `review-2-api-minimal.md` (R2, 18), `review-3-codex.md` (C, 35).
Every finding is accepted, merged into another, or rejected with a reason. Plan v2 (`agentparse.md`) applies the accepted ones.

## Accepted

| Topic | Findings | Decision in plan v2 |
|---|---|---|
| Fields outside the mapped ones | R1-3, C-1, R2-8 | `Entry.extra` and `Session.extra` hold the unmapped keys of the source record(s). One principle: every recorded value has exactly one home, a typed field or `data`/`extra`; `data`/`extra` never repeat a value already in a typed field. Envelope keys that repeat on every record and are already on `Session` are skipped. Raw `input` plus typed `tool` is the one allowed duplicate (input is the truth, the typed view is a convenience) |
| Unmodeled blocks inside a message | R1-4 | `Part::Other { kind, data }` |
| Product label of injected content | R1-5, C-19, C-20 | keep `Message.origin` (normalized enum) and add `Message.kind: Option<String>` = the product's own label (Claude `origin.kind` or leading tag, Codex `content_item_kinds[i]`, OpenCode `metadata.source`, Gemini tag). A source record with mixed kinds is split into consecutive messages |
| Gemini history | R1-1, C-7, R1-22 | full view: every message id ever seen, last version, first-seen order; dropped messages stay as entries, followed by `Rewind { dropped }` where the removal happened; masked results recovered from earlier lines when present |
| OpenCode migrated sessions | R1-2, C-28 | v1 rows first, then every `session_message` row whose id is not a v1 message id, in `seq` order |
| Usage formulas | R1-6, R1-7, C-3, C-4, R1-8, R1-9, C-6 | Gemini input = `input - cached + tool`; Claude reasoning = `thinking_tokens`; Codex uncached = `input - cached - cache_write` (cache_write inclusion unverified locally, all zero); saturating arithmetic; `total()` defined and asserted per product |
| Compaction usage and carriers | R1-10, R1-26, C-8, R2-2 | `Compaction.usage`, `Compaction.logical_parent`; one rule: the summary carrier record (Claude `isCompactSummary` line, OpenCode v1 `summary:true` message, Gemini `<state_snapshot>` message, Codex `compacted`) is absorbed into `Compaction` and not emitted again; Codex `replacement_history` goes to `extra` |
| Session cost | R1-11, C-5 | `Session.usage.cost_usd` = product-recorded session cost when one exists (Claude latest `cost-state`), else the sum of message costs |
| Tool status | R1-12, R1-13, R1-14, C-15, C-16, C-17 | `exit_code` only from a recorded or printed code. Shell status Error iff exit != 0 or timed out. New `ToolCall.status_detail` keeps the product's own status/denial label. Status table per product in 4.3. No result: Interrupted when a later assistant message or turn end exists in the parsed window, else Pending. `failed()` = status in {Error, Rejected, Interrupted} |
| Shell control and background | R1-15, R1-16, C-18 | `TaskOutput`/`BashOutput` -> Process Read, `TaskStop`/`KillShell` -> Process Kill, `Monitor` -> Shell background. Background completions (Claude task-notification, OpenCode v2 synthetic shell rows, Codex late events) stay as their own entries AND enrich the originating call's `ShellResult` by id |
| Human-run commands | R1-17, C-27 | `Message { role: User, origin: Command, parts: [ToolCall Shell with result] }` for Claude `<bash-input>`/`<bash-stdout>`, OpenCode v1 `!cmd` and v2 `shell` rows, Codex `user_shell` |
| Codex shell clock | R1-18, C-12 | implementer measures on the real corpus which recorded clock matches a known slow command and documents the choice; the other clock stays in `data` |
| Claude interrupts and timeouts | R1-19, C-17 | detection from `toolDenialKind`, marker lines and fixed texts per research; marker lines become `Turn { status: "interrupted" }`, not messages |
| Codex code mode | R1-20, C-9, C-10, C-11 | rules (a) to (h) of R1-20 adopted: cell status from the output header, extra outputs append, `wait` -> Process, stdin interaction -> Process Write, children = tool-like items between call and output, late events by process id, child `data` = the event |
| File changes | R1-21, R1-23 | per product rules in 4.6, including Claude `bashEditDiff` |
| Typed mapping gaps | R1-24, C-25, C-26, R1-28 | `NotebookEdit`, `read_many_files` -> Other; offsets kept as recorded (documented base differs); MCP typed only where the name scheme is unambiguous (`mcp__s__t`, Gemini `mcp_`), OpenCode non-builtin names -> Other |
| Codex response grouping | R1-25, C-2 | one assistant Message per model response (items up to the next `token_usage_record`), `Entry.id` = response id |
| Claude synthetic API errors | R1-27 | `isApiErrorMessage` / model `<synthetic>` -> `Error` entry, never a model |
| Per-product fixes | R1-29..32 | adopted into section 6 |
| Claude graph ids | R1-33, C-23, R2-1 | merged assistant `Entry.id` = `message.id`; line uuid map resolves `parent_id` to the absorbing entry; merged line uuids kept in `extra.uuids` |
| Subagent identity | R1-34, C-24 | Claude child id = `agentId`, parent = `parentAgentId` else `sessionId`, call = meta `toolUseId`; Codex child id = `payload.id` |
| Ids not unique | R1-35 | `(agent, source)` identifies a `SessionRef`; Gemini `.json`/`.jsonl` pair deduped preferring `.jsonl` |
| Base64 in data | R1-36 | replaced by `{"bytes": n}` in `data` and `extra` |
| Cuts | R1-37, R2 | delete `sidechain`, `ShellResult.merged` (stderr None means one stream), `Code.language`, `Rewind.to`; rename `ParentLink.agent` and `Task.agent` to `agent_type`; `Message.agent` only where the product records it per message |
| Orphan results | C-21 | `Part::ToolResult(ToolResult { call_id, status, output })` in a `Message { role: User, origin: Tool }`, no invented call |
| Errors and readers | C-30, C-31, R2 | `Error` variants with display text, `Sqlite(String)` (no rusqlite type in the public API); readers return `Result<Session>`; one public `parse_reader(agent, reader, source)` |
| Tail reads | R2-5 | `parse_file_tail(path, max_bytes)` for JSONL products; the torn first line is dropped silently; line numbers are window-relative (documented) |
| Queued input | R2-6, R1-29c | Claude queue FIFO replay as wend does; `queued_command` attachments typed by a human become user messages; input still queued at the end gets `phase: "queued"` |
| Discovery | C-32, R2 | discovery returns sessions plus warnings, an unreadable root never fails the rest; env overrides only where verified in source/docs |
| SQLite | R2 | read-only open, busy timeout 2 s, rusqlite range proven by R2's scratch crates (0.32 and 0.37 consumers resolve) plus CI job at 0.32 |
| Verification | C-34, C-35 | semantic fixture assertions for every generation; merged-stream products assert exit code and both markers in `stdout`; round trip, malformed input, invalid UTF-8, WAL, zst, peak RSS and timestamp unit checks |
| CLI | R2 | `list`, `show [--json]`, `stats` |

## Rejected

| Finding | Reason |
|---|---|
| C-33 defer docs site and release automation, optional SQLite feature, maintained date crate | owner rules require docs and release-please in every project; a feature flag is a config knob the owner rejects; the date parser is 60 tested lines reused from qoop-history |
| R2 drop `Entry.turn_id` | Codex turn grouping and Claude `promptId` are real, used grouping keys |
| R2 merge `Notice` and `Error` | errors are the most filtered record kind; keeping them separate costs one small struct |
| R2 make `format` an enum | an open string keeps new generations additive without a breaking change |
| R2 `locate` API with id allowlist | consumers filter `discover()` by id; the allowlist is a qoop CLI security rule, not a parser concern |
| C-14 typed fields for parsed command actions, tty, pgid, pid lists | kept losslessly in `data`; no second consumer needs them typed yet |
| C-22 `Message.recipient` | Codex recipient goes to `extra`; role/author already say who spoke |
| Codex `ModelResponse` side collection (C-2) | the per-response assistant Message already is that grouping |
| R2-7 match `typed_in` speed | out of scope: `typed_in` is a qoop fast path over raw text; this crate does full-detail parsing |
