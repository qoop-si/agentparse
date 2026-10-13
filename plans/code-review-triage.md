# Code review triage (round 1)

Reviews: `code-review-{claude,codex,opencode,gemini}.md` (per module), `code-review-cross.md` (consistency),
`code-review-codexcli.md` (Codex CLI, whole crate). Module findings go to that module's fixer; this file records the
cross-module decisions and who applies them.

## Cross-module decisions (`code-review-cross.md`)

| # | Decision | Who |
|---|---|---|
| 1 | `cargo fmt` once after every module lands | lead, last |
| 2 | Exit code on success: 0 when the product only reports plain success for exit 0 (Claude: no `is_error`, no `returnCodeInterpretation`, not backgrounded, not interrupted; Gemini as today). `ShellResult.exit_code` doc updated | lead (doc), Claude fixer |
| 3 | A child session's first prompt is `Origin::Agent` everywhere | OpenCode fixer |
| 4 | A prompt message's `turn_id` is its own id when it starts the turn | OpenCode fixer |
| 5 | `Shell.cwd` falls back to the session directory when the call names none | Gemini, OpenCode fixers |
| 6 | The "no result -> Interrupted" inference lives once in `Session::finish` (done, with a test); modules delete their own copies | lead (done), all fixers |
| 7 | Per-call leftovers: `Entry.extra.calls.<call_id>.{call,output}` (Codex shape); Claude moves `extra.results` there and keeps `tool_use` leftovers too. `ToolCall.extra` rejected | Claude fixer |
| 8 | One set of `take_*` / field helpers in `util.rs`, Message/ToolCall constructors, one data-URL-to-Media, one diff-stat counter, one Codex file-name parser shared with discover, one extra builder | cleanup pass after the fixers |
| 9 | Turn status vocabulary `completed` / `interrupted` / `failed`; interrupt text in `extra.marker` | OpenCode, Claude fixers |
| 10 | OpenCode `usage.cost_usd` = the session's recorded cost | OpenCode fixer |
| 11 | A Codex inter-agent message written by this session is `Origin::Model` | Codex fixer |
| 12 | `ShellResult` only on Shell/Process calls: Codex code cells move wall time/cell id to `data`; Claude `TaskOutput` gets a ShellResult | Codex, Claude fixers |
| 13 | Human-run command `input` = what was recorded (no invented objects) | OpenCode fixer |
| 14 | No compaction trigger unless recorded | OpenCode fixer |
| 15 | `FileChange.diff` doc names both formats | lead (done) |
| 16 | `strip_blobs` on tool inputs everywhere | OpenCode fixer |
| 17 | API/CLI: `opencode::parse_storage_file` private; tail `unwrap_or` removed; `SessionRef.parent_id` documented; `show <id>` takes the newest copy; rewind line wording | lead (done), OpenCode fixer (visibility) |
| Model requests | `ToolCall.extra` rejected; `Tool::Patch { Option }` rejected (no input means `Tool::Other`, files are in `output.files`); Gemini pgid/pid fields rejected (`data`); tool-attached binaries go to `ToolOutput.attachments`, echo duplicates dropped | as listed |
| Dead code | `<synthetic>` model filter removed from `finish`; Gemini unreachable `Snapshot` arm; garbled comment `codex/tools.rs:386`. `Origin::is_synthetic` stays (public helper for consumers) | lead (done), Gemini, Codex fixers |
