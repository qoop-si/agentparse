# Verification

How the parser was checked against real transcripts before the first release.

Summary:
1. 230 tests green; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean.
2. A local corpus of 19,395 real sessions across four products: 0 failures, 0 panics, 53 s, peak RSS 310 MB.
3. Codex oracles from Codex's own databases: usage 135/135, titles 135/135, turn counts 133/133 finished sessions.
4. Fresh real sessions: `sh test.sh` gives exit 3, status error, and both `out-line` and `err-line` in Claude, Codex, OpenCode.
5. Regression harness: 535 frozen inputs parse to byte-identical JSON before and after a cleanup pass.

All numbers measured on 2026-10-13 on one developer machine with the release build.

## 1. Build checks

| Check | Result |
|---|---|
| `cargo test` | 230 passed, 0 failed (lib 24, claude 29, codex 33, gemini 82, opencode 61, doc 1) |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean (default rustfmt config) |

## 2. Corpus run (`agentparse stats --json`, `/usr/bin/time -l`)

| Product | Sessions | Failures | Warnings | Orphan results | Entries | p50 ms | p99 ms | MB/s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Claude Code | 19,188 | 0 | 42 | 13 | 1,309,384 | 1.62 | 21.84 | 256 |
| Codex | 135 | 0 | 0 | 0 | 4,215 | 2.03 | 4.51 | 213 |
| OpenCode | 70 | 0 | 0 | 0 | 340 | 0.42 | 0.61 | n/a (SQLite) |
| Gemini CLI | 2 | 0 | 0 | 0 | 2 | 0.37 | 0.37 | n/a |

Wall time 53.2 s (28.9 s user CPU), peak RSS 310 MB. Claude tool calls: ok 541,086, error 17,459, rejected 1,435,
interrupted 751, pending 38. Claude shell exit codes: 0 260,577, unknown 75,170 (backgrounded, interrupted, soft
failures), 1 8,524, 2 630, 143 402. Orphan results (13) are results whose call lives in another file (resumed copies).
The Claude fixtures cite an earlier format survey of 17,826 transcript files (1,341 main sessions, 7,046 subagents,
9,439 workflow subagents); that was a separate scan made before this run, so its count differs from the 19,188 here.

## 3. Oracles

| Oracle | Source | Result |
|---|---|---|
| Codex usage | last `thread_token_usage` per rollout vs `Session.usage` (input + cache = input_tokens, output) | 135/135 |
| Codex titles | `state_5.sqlite threads.title` | 135/135 (first 50 chars) |
| Codex turns | `thread_history_1.sqlite thread_turns` | 133/133 finished; 2 sessions were being written during the run |
| Codex item ids | every `thread_items.item_id` found in the parsed session | 2130/2130 |
| OpenCode usage | `session_v2.tokens_*` | 60/64 at the time; the 4 others are root v2 sessions with an extra model call that has no stored row |
| Claude counts | independent jq/Python scan of 400 random files: assistant messages, usage per message id, tool_use count, human prompts | all equal |

## 4. Real sessions made for this check

A scratch git project with a 3 line `a.txt` and a `test.sh` that prints `out-line` to stdout, `err-line` to stderr
and exits 3. Each CLI ran the prompt "Run `ls -la`, then run `sh test.sh` and tell me its exit code, then read a.txt
and change its second line to 'two edited'." A stricter variant asked for exactly `sh test.sh` as the whole command,
because models like to append `; echo $?`, which hides the real exit code.

| Product | Command | Status | Exit | Output |
|---|---|---|---|---|
| Claude Code | `sh test.sh` | error | 3 | `out-line` and `err-line` in the single text stream |
| Codex | `sh test.sh` (argv `/bin/zsh -lc`) | error | 3 | both markers in `stdout`, `stderr` None; `apply_patch` -> `a.txt` update +1 -1 |
| OpenCode | `sh test.sh` | error | 3 | both markers in `stdout`; a background `sleep 2; echo bg-done` job completed with exit 0 after 3.0 s |
| Gemini CLI | not run | | | the CLI had no login on that machine; the 2 local sessions and 3 upstream-faithful fixtures are covered |

The Codex session, sanitized, is `tests/fixtures/codex/exit3.jsonl`.

## 5. Regression harness

300 real Claude files, all Codex rollouts, every OpenCode session in the live database, the Gemini sessions and every
fixture, parsed with `show --json` before and after a cleanup pass and the final fixes: 535 outputs, 0 differ. (It
caught one regression, a JSON-storage id leak, which was reverted and now has its own test.)

## 6. Docs

The `docs/` site was rendered in a headless browser (`introduction` and `shell` pages checked by screenshot); every
Rust snippet in the README and docs compiles against the crate; every example output comes from the fixtures.
