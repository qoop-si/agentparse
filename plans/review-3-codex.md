1. **blocker | §§2 D4/D9, 3, 6: Known records silently lose fields.**  
   **Evidence:** `Other` preserves unknown record types, but recognized `Message`, `Part`, `Usage`, `Settings`, and `Compaction` have no extras. Research records Claude `permissionDecision`, Codex response metadata, Gemini `displayContent`, and OpenCode provider state. Section 4.1 references “message raw data,” which does not exist.  
   **Change:** Add source-specific extras to recognized records and parts, preserving unmapped fields after explicit binary redaction. Define exactly which fields are intentionally omitted. Remove the unconditional “lossless” claim.

2. **blocker | §§3, 4.1: Codex usage lacks a model-response identity.**  
   **Evidence:** Codex research §3.5: one `token_usage_record.response_id` covers reasoning, commentary, and calls emitted by one response. It is not usage for an individual assistant text message.  
   **Change:** Add a `ModelResponse { id, usage, entry_ids, sources }` collection or equivalent response grouping. Attach each usage record once, including responses containing only reasoning or calls. Preserve reported turn/thread cumulative totals separately.

3. **major | §4.1: Claude reasoning usage is wrongly declared unknown.**  
   **Evidence:** Claude research §4 records `usage.output_tokens_details.thinking_tokens`; §14 explicitly states that these tokens are already included in output.  
   **Change:** Set `reasoning` from `thinking_tokens` when present, without adding it to `output`. Represent absent reasoning counts as unknown rather than indistinguishable zero.

4. **major | §§3, 4.1: Gemini tool-prompt usage is incorrectly described as included in input.**  
   **Evidence:** Gemini research §5.4 defines reported total as `input + output + thoughts + tool`. The plan says `tool` is already inside input.  
   **Change:** Add `tool_prompt` and `reported_total` to usage. Keep `input = input - cached` and `output = output + thoughts`, then include `tool_prompt` separately in normalized total. Preserve raw usage for reconciliation.

5. **major | §§3, 4.1, 6: Session usage cannot always be a sum of message usage.**  
   **Evidence:** OpenCode research §6 records usage and cost on standalone v2 compaction rows. Claude research §9 records session cost snapshots. Codex tail reads can contain cumulative totals without the preceding responses.  
   **Change:** Distinguish computed usage from reported session totals and track completeness. Include compaction model calls exactly once. Preserve Claude recorded session cost separately from per-message cost.

6. **major | §§3, 4.1: Usage arithmetic and missing-value semantics are unspecified.**  
   **Evidence:** Unchecked `u64` subtraction can underflow on inconsistent records; `add()` can overflow. Default zero conflates an absent measurement with a recorded zero. Claude research §4 also says to take the last chunk’s usage, not the first repeated value.  
   **Change:** Specify checked arithmetic with warnings for inconsistent counters, optional unknown measurements, and `total = input + cache_read + cache_write + output + tool_prompt`. Explicitly use the last Claude chunk and dedupe Codex fallback cumulative records without adding repeated totals.

7. **blocker | §§3, 6: Gemini reduction deletes requested transcript history.**  
   **Evidence:** Gemini research §§4, 8.2: `$set.messages`, `removeIds`, rewind, and masking remove or replace earlier messages and outputs. Mapping only reduced state cannot recover them through a `Rewind` entry.  
   **Change:** Retain historical message/output revisions and mutation events with source provenance. Expose active reduced history as a derived view. Do not classify every removal as rewind; distinguish compaction, rollback, and masking.

8. **major | §§3, 6: Compaction cannot represent recorded context replacement.**  
   **Evidence:** Codex research §7 records `replacement_history`, window links, and resume metadata. OpenCode research §6 records compaction status, retained tail, usage, and error. Claude research §6 records preserved segments and logical parent links.  
   **Change:** Extend compaction with status, retained/replacement history references, usage, error, and source-specific metadata. Preserve Codex replacement items without counting them as newly generated messages or usage.

9. **blocker | §§3, 4.4, 6: Code cells cannot represent multiple deliveries or continuation.**  
   **Evidence:** Codex research §3.10 permits multiple outputs for one call through `notify`; §4.2 describes running cells resumed through `wait(cell_id)`. `output: Option<ToolOutput>` and `Code { language, source }` cannot represent this lifecycle.  
   **Change:** Store ordered output deliveries with timestamps/provenance. Add cell ID, execution state, wall time, and continuation links. Map code-mode `wait` explicitly. Derive execution status from output headers, never `custom_tool_call.status`.

10. **major | §§3, 6: Nested code-mode calls are assigned more certainty than the records support.**  
    **Evidence:** Codex research §6: nested events lack parent `call_id`; empty stdin polls can produce no event; late events arrive outside the cell’s call/output interval. JavaScript can loop, branch, and run tools concurrently.  
    **Change:** Preserve standalone execution events and mark inferred parent links with their basis. Join late events by process identity. Do not invent exact child input objects or ordering from regex matches in arbitrary JavaScript.

11. **major | §§3, 4.5: Model-visible and persisted tool output are conflated.**  
    **Evidence:** Codex research §§4.2, 5 distinguishes truncated model-visible output from `aggregated_output`, which has its own persistence truncation. OpenCode pruned outputs also differ from stored output.  
    **Change:** Separate model-visible deliveries from persisted execution output. Track truncation independently, including original token count and persistence marker. Preserve both forms instead of overwriting one.

12. **major | §§3, 4.5: Shell timing collapses different measurements.**  
    **Evidence:** Codex research §5 distinguishes nanosecond-resolution execution duration, unified-exec wall time, and whole-cell wall time. Claude timestamps are write times and can move backward.  
    **Change:** Store execution duration separately from tool/cell wall time, retaining submillisecond precision. Label timestamp-derived durations as estimates and reject negative differences with warnings.

13. **major | §§3, 4.5: Codex argv extraction is incorrectly generalized.**  
    **Evidence:** `command[2]` describes the measured zsh invocation, while Codex supports other shells and arbitrary argv. `yield_time_ms` controls waiting, not whether a command was launched in the background.  
    **Change:** Preserve argv as authoritative. Extract shell source only for recognized shell invocation forms; otherwise leave it absent. Record yielded/running process state separately from an explicit background request.

14. **major | §§3, 4.5: Recorded terminal details have no typed home.**  
    **Evidence:** Codex research §5 lists source, parsed actions, interaction input, tty/login, yield budget, output budget, and permissions. Gemini research §7 records multiple background PIDs and PGID.  
    **Change:** Add execution source, parsed command actions, stdin interactions, process links, optional execution options, PID list, and PGID. Keep unsupported nested request details unknown rather than inferring defaults.

15. **major | §§4.3, 6: A completed turn does not prove an unfinished process was interrupted.**  
    **Evidence:** Codex research §3.4 quotes the abort marker warning that unified-exec processes may remain running. Claude and OpenCode background jobs can outlive a turn.  
    **Change:** Change status only on explicit interruption, termination, or recorded recovery evidence. Otherwise retain running or unknown outcome. Scope aborted-turn handling to the affected calls and preserve detached process state.

16. **major | §§2 D7, 4.3, 4.5: Tool status and command outcome are conflated.**  
    **Evidence:** Gemini records nonzero shell exits and inactivity timeouts as tool success. Claude soft failures can have `is_error:false`, and `timedOutAfterMs` can mean a command moved to background.  
    **Change:** Preserve reported tool status separately from process outcome. Let `failed()` derive failure from explicit errors, nonzero exit, timeout, and interruption. Do not turn background handoff into timeout or infer exit zero from a Gemini success result that explicitly reports timeout/signal.

17. **major | §§4.3, 4.5: Claude interruption detection uses an ineffective source.**  
    **Evidence:** Claude research §5.1: object `toolUseResult.interrupted` is never true in the measured corpus. Interrupts and denials appear in `toolDenialKind`, result text, and interruption markers.  
    **Change:** Use explicit denial/interruption fields and marker text before generic `is_error`. Preserve denial kind and cancelled-sibling distinctions. Treat “session ended before result” as unknown outcome with recorded interruption metadata.

18. **major | §§3, 4.4, 6: Background completion is not linked to its originating call.**  
    **Evidence:** Claude research §§5.1, 10 records `TaskOutput`, `TaskStop`, and task notifications; OpenCode v2 records synthetic background completion messages.  
    **Change:** Map current Claude process tools, including `TaskOutput`, `TaskStop`, and `Monitor`. Add background lifecycle records linked by task/process ID and originating call ID. Keep intermediate poll deliveries and final completion.

19. **major | §§3, 4.2: Synthetic classification belongs partly on content parts.**  
    **Evidence:** Codex `content_item_kinds` is aligned per content item. OpenCode v1 has per-text `synthetic` and `ignored` flags. Claude records mixed tool-result and other content blocks.  
    **Change:** Add part-level origin and visibility, preserving source role. Fold only result blocks and retain remaining text/attachments. Avoid reducing a mixed message to one synthetic boolean.

20. **major | §4.2: Human slash commands and shell escapes are incorrectly treated as injected text.**  
    **Evidence:** Claude research §§3, 6 records human slash commands in `<command-name>` wrappers and user shell escapes in `<bash-input>`. Gemini records human `/command` messages even when resume ignores them.  
    **Change:** Separate origin from representation: human command invocation, harness output, context injection, and peer message must remain distinct. Expand Claude classification to recorded `origin`, `promptSource`, task notifications, teammate messages, and interruption markers.

21. **major | §§3, 4.2: Orphan results are fabricated as tool calls inside user messages.**  
    **Evidence:** Gemini research §10 recommends an orphan result part; an orphan has a recorded call reference but no known call name or input. The plan also contradicts its own “tool results are never user messages” rule.  
    **Change:** Add `ToolResult { call_id, output, sources }` as a standalone entry or part. Preserve it without inventing a call, empty name, or user author.

22. **major | §3: Inter-agent messages lose recipient and encrypted-content meaning.**  
    **Evidence:** Codex research §3.8 records author, recipient, message type, trigger-turn metadata, and encrypted payload parts. Claude records coordinator and teammate origins.  
    **Change:** Add explicit inter-agent origin, recipient, message type, and opaque-content metadata. Distinguish encrypted payload placeholders from readable task text. Preserve tool namespace so namespaced calls retain identity.

23. **major | §§3, 6: Merging Claude chunks breaks graph identity.**  
    **Evidence:** Claude research §§4, 5: merged API messages contain several line UUIDs; tool results and later records point to individual line UUIDs. Codex merged items likewise combine several source ordinals.  
    **Change:** Add `sources: Vec<SourceRecordRef>` with line/ordinal, original ID, kind, and timestamp. Preserve all Claude line UUID aliases and parent edges. Define entry IDs independently of these source identities.

24. **major | §§2 D11, 3, 6: Subagent identity and relationships are underspecified.**  
    **Evidence:** Claude children are identified by `(root sessionId, agentId)` and nested parent agents live in sidecars. Codex distinguishes root `session_id` from child `id`; waits may address multiple agents.  
    **Change:** Define a namespaced session identity and retain root ID, agent ID, immediate parent, depth, path, and nickname separately. Replace singular `child_session` with typed spawn/address relationships supporting multiple targets. Read Claude sidecars and ignore inherited Codex root metadata for child identity.

25. **major | §§3, 4.4: Several typed tool mappings cannot represent their inputs.**  
    **Evidence:** Gemini `read_many_files` takes include/exclude arrays, not one path; its modern line range differs from legacy offset/limit. Claude `NotebookEdit` uses cell operations rather than old/new text. Codex web tools support search, open, and find, often in batches.  
    **Change:** Add explicit multi-file read, line-range, notebook-edit, and web-action types. Preserve qualifiers for searches and edits. Use `Other` with raw input when typing fails instead of inventing required values.

26. **major | §4.4: MCP server splitting is presented as authoritative when it is ambiguous.**  
    **Evidence:** OpenCode research §5.4 permits underscores in both server and tool names and recommends configured prefixes. Generic `<server>__<tool>` patterns can also collide with non-MCP namespaces.  
    **Change:** Prefer explicit event server/tool fields or known configured prefixes. Otherwise retain an unresolved namespaced tool with optional inferred split and provenance. Do not classify all unknown underscore names as MCP.

27. **major | §§3, 6: Standalone user terminal commands lack a defined mapping.**  
    **Evidence:** OpenCode research §6 records v2 `shell` rows outside assistant messages; Codex execution source includes `user_shell`; Claude records shell escapes and local command output.  
    **Change:** Add a standalone command entry carrying execution origin and lifecycle. Map these sources explicitly instead of fabricating assistant authorship or leaving core terminal data in `Other`.

28. **major | §6, risk 6: Preferring OpenCode v1 solely because both rows exist can omit resumed history.**  
    **Evidence:** OpenCode research §2 says migration leaves v1 tables as dead weight while v2 is the durable truth. Its richer-v1 recommendation does not establish that migrated sessions never receive later v2 activity.  
    **Change:** Use v2 for active ordering and post-migration records, enriching matched historical records from v1. Permit v1-only preference only after verifying that v2 contains no later activity. Test a migrated session resumed under v2.

29. **major | §§3, 5: Serde omission rules do not guarantee deserialization round trips.**  
    **Evidence:** The plan specifies defaults for `Option` only, while omitting empty vectors and false booleans. Missing nonoptional vectors, booleans, and zero-default usage fields require explicit serde defaults.  
    **Change:** Add `#[serde(default)]` wherever serialization can omit a field, implement `Default` for usage, and test empty/false/zero round trips. Internally tagged newtype variants containing these structs are valid; keep flatten only where reserved keys cannot collide, and prohibit flattened extras from shadowing them.

30. **blocker | §5: The error enum does not compile as written.**  
    **Evidence:** `#[derive(thiserror::Error)]` variants have neither `#[error(...)]` nor transparent annotations. SQLite errors are also reduced to strings.  
    **Change:** Add display annotations, preserve underlying I/O and SQLite errors with `#[source]`/`#[from]`, and distinguish invalid session, decode/decompression failure, and missing session. Treat database query/schema failures as errors rather than successful empty sessions.

31. **major | §5: Reader APIs promise success despite I/O failure and missing identity.**  
    **Evidence:** `parse_reader(...) -> Session` cannot propagate `BufRead` errors. A tail can omit session metadata, calls, earlier usage, Gemini reducer state, or Codex history references.  
    **Change:** Return `Result<Session>` for complete parsing. Add a separate fragment API requiring identity/context and returning explicit completeness plus unresolved references. Give Gemini a streaming reader API instead of requiring an entire JSONL file as a string.

32. **major | §§1, 5, 6: Source resolution and discovery are insufficiently specified.**  
    **Evidence:** Codex research §3.1 records `history_base` referencing another rollout. OpenCode legacy sessions span session/message/part files. Gemini may have G1 and migrated JSONL files sharing an ID.  
    **Change:** Add an explicit storage-tree source and a resolver interface for referenced history and child sessions. Specify archive/channel scanning, duplicate-file selection, and optional-root failure handling. Return discovery warnings rather than failing all agents when one root is unreadable.

33. **minor | §§2 D2, 7: Delete unnecessary supporting infrastructure from the first library milestone.**  
    **Evidence:** A docs site, release automation, four CLI commands, a custom RFC3339 parser, and a broad bundled SQLite version range all add work unrelated to transcript correctness.  
    **Change:** Defer the docs site and release automation; keep corpus tooling internal initially. Replace the handwritten timestamp parser with a maintained parser. Make SQLite support optional and test the claimed consumer dependency compatibility in real Cargo graphs before selecting the range.

34. **major | §8: Counts and a zero-failure corpus run cannot verify fidelity.**  
    **Evidence:** A parser can drop reasoning, calls, usage, or pre-compaction messages while reporting successful parsing. The local Codex corpus exercises only paginated code mode, and research explicitly marks many formats as upstream-only.  
    **Change:** Add exact semantic fixtures for every supported generation, response usage grouping, duplicate outputs, `notify`/`wait`, late processes, arbitrary argv, encrypted agent messages, inherited prefixes, compaction replacement, Gemini masking/rewind, and migrated OpenCode resume. Assert preserved fields and provenance, not just counts.

35. **major | §8: The real-session stderr requirement is impossible for merged-stream formats.**  
    **Evidence:** Codex, OpenCode, and modern Gemini record one combined output stream. Separate stderr is unavailable.  
    **Change:** Assert exit code 3 and both known output markers in combined output with `merged:true`; require separate stderr only where recorded. Add round-trip, malformed-input, partial-line, invalid UTF-8, WAL consistency, compressed-file, peak-memory, and timestamp-unit checks. Freeze or document corpus snapshots before comparing oracle totals.

VERDICT: changes required.
