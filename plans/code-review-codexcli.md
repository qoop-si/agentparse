1. **Major**, `src/codex/tools.rs:114`. An exec yields cell `7`; its first wait yields again; its second wait completes. The first wait overwrites the cell’s original exec ID, so subsequent children attach to the wait and the exec remains Pending. **Fix:** resolve each wait to its original exec before updating `cells`, and retain that root mapping across repeated yields. Verified against upstream yield semantics; runtime reproduction unexecuted.

2. **Major**, `src/codex/results.rs:63`. A patch requesting an update returns `apply_patch verification failed: ...` without an exit-code header. The parser marks it Ok and reports the requested files as changed. **Fix:** recognize upstream patch failure messages, including `failed to parse apply_patch:`, as Error; derive applied files only when the output confirms successful application. Upstream error formats verified; runtime reproduction unexecuted.

3. **Major**, `src/codex/results.rs:389` and `src/opencode/tool.rs:256`. External duration seconds of `u64::MAX`, or start/end timestamps of `i64::MIN`/`i64::MAX`, overflow before conversion, causing debug panics or incorrect release durations. The signed subtraction also occurs at `src/opencode/v2.rs:194` and `:236`. **Fix:** use checked multiplication/addition for Codex durations and `checked_sub` followed by `u64::try_from` for OpenCode durations. Preserve unconvertible source values in data. Statically verified.

4. **Major**, `src/opencode/tool.rs:211`. A tool input containing an inline base64 payload or `data:` URL survives unchanged in `ToolCall.input`, violating D10. **Fix:** apply `strip_blobs` to the input before constructing the stored call and typed view. Statically verified.

5. **Major**, `src/opencode/v2.rs:335` and `src/opencode/v1.rs:336`. Text and reasoning blocks discard remaining fields after conversion. Recorded provider item IDs, reasoning timestamps, and other metadata disappear, violating D4. **Fix:** return the typed part together with its sanitized leftover map, and store leftovers in `Entry.extra` indexed by source part position. Statically verified.

6. **Major**, `src/opencode.rs:113`. While parsing an active database, a writer can commit a new assistant message and its parts between `load_v1`’s part query and message query. The parser then emits the new message without its parts or usage. **Fix:** begin one deferred read transaction before the first query and perform all session reads within that snapshot. The connection is read-only; the concurrent failure scenario is statically established but unexecuted.

7. **Minor**, `src/codex/messages.rs:296`. A legacy assistant event containing `"hello"` followed by its response-item copy with ID `msg_1` is deduplicated without preserving `msg_1`. **Fix:** use `merge_copy(e, id, extra)` on this path, and register the item ID against the existing part where needed. Statically verified.

8. **Minor**, `src/gemini.rs:944`. A pending tool call with `result: null` and unmapped metadata loses that metadata because `call_data` runs only inside `resp.map`; the enclosing message also excludes `toolCalls` from extras. **Fix:** compute call data independently and create a default `ToolOutput` when metadata exists without a response, preserving the call’s status. Statically verified.

9. **Minor**, `src/discover.rs:213`. An unreadable session directory returns an empty discovery result without warnings, making inaccessible history indistinguishable from an empty corpus. **Fix:** propagate directory and directory-entry errors into `Found.warnings` while continuing other roots. Statically verified against the discovery contract.

10. **Minor**, `src/lib.rs:65`. Calling `parse_file_tail(path, u64::MAX)` overflows `max_bytes + 1`; debug builds panic, while release builds can seek to EOF and reject a valid session. **Fix:** use `max_bytes.saturating_add(1)` in the comparison so this request parses the whole file. Statically verified.

VERDICT: changes required
