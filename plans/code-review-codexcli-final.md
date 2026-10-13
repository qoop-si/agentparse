Earlier findings 1 through 10:

| Finding | Status | Current location |
|---|---|---|
| 1. Repeated waits | Fixed | `src/codex/tools.rs:107` |
| 2. Failed patches | Fixed | `src/codex/results.rs:267` |
| 3. Duration overflow | Fixed | `src/codex/results.rs:406`, `src/opencode/record.rs:65` |
| 4. OpenCode input blobs | Fixed | `src/opencode/tool.rs:255` |
| 5. OpenCode part leftovers | Fixed | `src/opencode/v1.rs:371`, `src/opencode/v2.rs:390` |
| 6. Database snapshot | Fixed | `src/opencode.rs:119` |
| 7. Legacy assistant item ID | Fixed | `src/codex/messages.rs:348` |
| 8. Pending Gemini metadata | Fixed, with regression below | `src/gemini.rs:548` |
| 9. Discovery warnings | Partly open | `src/discover.rs:124` |
| 10. Tail-size overflow | Fixed | `src/lib.rs:70` |

The major findings spot-checked from the other reviews are fixed, including Codex assistant multipart deduplication, image prompts, hook prompts, item timestamps, usage records, and child-tail identity. All 32 existing compiled Codex tests passed. No files were modified.

1. **Major**, `src/gemini.rs:564`, `src/model.rs:773`. A call with `result:null` and `displayName` or `description` receives a metadata-only `ToolOutput`. When a later assistant message appears, abandonment detection skips it because `output.is_some()`, leaving the call Pending indefinitely. **Exact fix:** distinguish metadata-only output from an actual response when marking abandoned calls. Preserve the metadata, but mark a result-less Gemini call Interrupted when a later assistant message exists; retain Pending for calls with actual running results.

2. **Minor**, `src/codex/messages.rs:219`, `src/codex/messages.rs:316`. Buffered assistant events retain no source timestamp, line, or turn. If an event has no matching response item, such as when that item is corrupt or missing, a subsequent `task_complete` flushes it using the completion record’s context. The fallback message receives the wrong timestamp and source line. **Exact fix:** store the event’s `Rec` and turn alongside `AgentEvent`, and use that saved context when emitting unmatched events.

3. **Minor**, `src/discover.rs:124`. Earlier finding 9 remains partly open: `read_dir` itself now produces warnings, but individual iterator errors are silently discarded by `e.ok()`. A directory enumeration failure can omit sessions without any warning. **Exact fix:** explicitly match each directory-entry result, collect successful paths, and call `self.warn` with the directory and error for every failed entry.

VERDICT: changes required
