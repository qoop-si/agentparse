# agentparse

agentparse reads the session transcripts that Claude Code, Codex, OpenCode and Gemini CLI leave on disk and turns each one into the same detailed Rust model: messages, reasoning, tool calls with typed inputs and their results already paired, shell commands with exit code, stdout, stderr, duration and background ids, token usage, compaction, and subagent sessions. It is a read-only library plus a small command line tool. It finds sessions, parses them, and never writes to a transcript, never calls the network, and never reads credentials.

Docs: https://qoop-si.github.io/agentparse/

## Supported products

| Product | On-disk format | Default location | `Session.format` |
|---|---|---|---|
| Claude Code | JSONL, one file per session, subagents in their own files | `~/.claude/projects/<project>/<session id>.jsonl` | `claude-jsonl` |
| Codex | JSONL rollouts (flat, legacy and paginated generations), also `.jsonl.zst` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, `~/.codex/archived_sessions/` | `codex-flat`, `codex-legacy`, `codex-paginated` |
| OpenCode | SQLite database (v1 tables, v2 tables, or both for a migrated session), and the old JSON storage tree | `~/.local/share/opencode/opencode.db`, `~/.local/share/opencode/storage/` | `opencode-v1`, `opencode-v2`, `opencode-v1+v2`, `opencode-json` |
| Gemini CLI | `.json` (G1), `.jsonl` snapshot style (G2), `.jsonl` append-only style (G3) | `~/.gemini/tmp/<project>/chats/session-*.json[l]` | `gemini-g1`, `gemini-g2`, `gemini-g3` |

## Install

The crate is not on crates.io yet. Use a git dependency:

```toml
[dependencies]
agentparse = { git = "https://github.com/qoop-si/agentparse" }
```

Once it is published, `cargo add agentparse` will do. The library needs Rust 1.85 or newer (edition 2024).

To install the command line tool from a checkout:

```bash
git clone https://github.com/qoop-si/agentparse
cd agentparse
cargo install --path .
```

## Quick start

```rust
use agentparse::{Tool, parse_file};

fn main() -> Result<(), agentparse::Error> {
    let path = std::env::args().nth(1).expect("usage: quick_start <session file>");
    let session = parse_file(path.as_ref())?;
    for call in session.tool_calls() {
        let Tool::Shell(shell) = &call.tool else { continue };
        let result = call.output.as_ref().and_then(|o| o.shell.as_ref());
        println!("$ {}", shell.command);
        println!("  status {:?}, exit {:?}", call.status, result.and_then(|r| r.exit_code));
        if let Some(stderr) = result.and_then(|r| r.stderr.as_deref()) {
            println!("  stderr: {}", stderr.trim_end());
        }
    }
    Ok(())
}
```

`parse_file` detects the product and format from the path and the first record. `tool_calls()` walks every call in order, including calls nested inside Codex code cells. Run on a Claude Code fixture from this repository:

```
$ sh test.sh
  status Error, exit Some(3)
  stderr: err-line
```

Claude Code records stderr separately for some results. Codex, OpenCode and Gemini record one interleaved stream, which lands in `stdout` with `stderr` left as `None`. See [Shell Commands](https://qoop-si.github.io/agentparse/#shell).

## Command line

```bash
agentparse list  [--agent claude|codex|opencode|gemini] [--json]
agentparse show  <file | session id> [--json] [--full]
agentparse stats [--agent ...] [--json]
```

`list` finds sessions in the default locations, `show` prints one session as a readable transcript (or the full model as JSON), and `stats` parses everything it finds and prints counts and timings. Output of `show` on a Codex fixture, trimmed:

```
codex session 0190a000-0000-7000-8000-000000000001  [codex-paginated]
  title: run the demo checks
  cwd: /work/demo
  models: gpt-6.1-sol
  usage: in 30447 out 215 cache r38656 w0

-- user  2026-10-13 01:34:18
  run the demo checks

-- assistant gpt-6.1-sol  2026-10-13 01:34:21  [in 9882 out 94 cache r12288 w0]
  > code cell (3 lines)
    0.8s
    > $ ls -la   (in /work/demo)
      exit 0 | 0.1s
    > $ sh test.sh   (in /work/demo)  [error: failed]
      exit 3 | 0.0s
      output:
        err-line
        out-line
```

## The model at a glance

| Type | What it is |
|---|---|
| `Session` | One transcript: ids, source, format, title, cwd, git, models, times, summed `usage`, `entries`, `warnings` |
| `Entry` | One item of the timeline: an id, a timestamp, the source line, and an `EntryKind` |
| `EntryKind` | `Message`, `Compaction`, `Turn`, `Settings`, `Notice`, `Hook`, `Error`, `Rewind`, or `Other` |
| `Message` | A role, an `Origin`, and ordered `Part`s, plus model, usage and stop reason |
| `Part` | `Text`, `Reasoning`, `Attachment`, `ToolCall`, `ToolResult` (only when the call is missing), or `Other` |
| `ToolCall` | Raw input, a typed `Tool`, a `ToolStatus`, timing, an optional `ToolOutput`, `child_session`, nested `children` |
| `Tool` | Typed view of the input: `Shell`, `Process`, `Read`, `Write`, `Edit`, `Patch`, `Search`, `Glob`, `List`, `WebFetch`, `WebSearch`, `Todo`, `Task`, `Mcp`, `Code`, `Question`, `Plan`, `Skill`, `Other` |
| `ToolOutput` | The result text the model saw, an optional `ShellResult`, changed `files`, attachments, structured `data` |
| `ShellResult` | `exit_code`, `stdout`, `stderr`, `duration_ms`, `interrupted`, `timed_out`, `signal`, `background_id` |

A tool call is a part of the assistant message that made it and carries its own result, so you never pair calls with results yourself. A subagent is its own `Session` with a `parent` link, and the spawning call has `child_session`.

## Conventions in short

| Topic | Rule |
|---|---|
| Usage | One counting scheme for every product. `input` is uncached prompt tokens, `output` includes reasoning, `reasoning` is the part of `output` spent reasoning, plus `cache_read`, `cache_write` and an optional recorded `cost_usd` |
| Status | `ToolStatus` is `Pending`, `Ok`, `Error`, `Rejected` or `Interrupted`. A shell command with a non-zero exit code or a timeout is `Error`. The product's own label stays in `status_detail` |
| Origin | Every message says who produced it: `Human`, `Model`, `Command`, `Context`, `Notification`, `Summary`, `Agent` or `Tool`. `kind` keeps the product's own label for synthetic content |
| Time and order | Timestamps are epoch milliseconds UTC. Entries stay in source order, never sorted |
| Nothing lost, nothing bulky | Values with no typed field go to `extra` or `data`. Records the model does not cover stay as `Other`. Bad lines become `warnings`. Base64 bodies and encrypted blobs are never stored |

Details: [Usage and Status](https://qoop-si.github.io/agentparse/#conventions).

## License

MIT
