# Introduction

agentparse reads the session transcripts that coding agents leave on disk and turns each one into a single Rust model. It supports Claude Code, Codex, OpenCode and Gemini CLI.

Each product stores sessions differently. Claude Code writes one JSONL line per content block. Codex writes rollouts with a separate record for every event. OpenCode keeps rows in SQLite. Gemini CLI rewrites or patches a chat file. agentparse hides all of that behind one set of types, so the code that reads a session does not depend on which agent wrote it.

## What you get

| You want | Where it is |
|---|---|
| The conversation, in order | `Session.entries`, each a `Message` or an event |
| Who wrote a message | `Message.role` and `Message.origin` (human, model, command, context, notification, summary, agent, tool) |
| Model reasoning | `Part::Reasoning`, with text when the product stored it |
| Every tool call with its result | `Part::ToolCall`, a `ToolCall` that carries its own `ToolOutput` |
| A typed view of the tool input | `ToolCall.tool`, a `Tool` such as `Shell`, `Edit` or `Task` |
| Shell commands in full | `Tool::Shell` for the command, `ToolOutput.shell` for exit code, stdout, stderr, duration, background id |
| Files a call changed | `ToolOutput.files`, a list of `FileChange` |
| Token usage and cost | `Message.usage` per message, `Session.usage` as the sum |
| Compaction | `EntryKind::Compaction` with the summary and token counts |
| Subagents | A child `Session` with a `parent` link, and `ToolCall.child_session` on the spawning call |
| Everything else the product recorded | `extra`, `ToolOutput.data`, `Other` entries and parts |

## How it works

You give agentparse a file, a discovered session, or an OpenCode database id. It detects the product and format, reads the records, and returns a `Session`. Records it cannot read become `warnings` on the session instead of failing the parse. The input is only ever read: transcripts are opened read-only and SQLite databases are opened read-only.

A tool call is a part of the assistant message that made it. The result is attached to the call, so there is no pairing step. When a result has no call in the parsed records (for example when you parse only the tail of a file), it stays as a `Part::ToolResult` with a warning.

## Supported products

| Product | Format | `Session.format` values |
|---|---|---|
| Claude Code | JSONL, one file per session and per subagent | `claude-jsonl` |
| Codex | JSONL rollouts, also `.jsonl.zst` | `codex-flat`, `codex-legacy`, `codex-paginated` |
| OpenCode | SQLite database, and the old JSON storage tree | `opencode-v1`, `opencode-v2`, `opencode-v1+v2`, `opencode-json` |
| Gemini CLI | `.json` and `.jsonl` chat files | `gemini-g1`, `gemini-g2`, `gemini-g3` |

The per-product pages explain where each product keeps files and how its records map to the model: [Claude Code](#claude), [Codex](#codex), [OpenCode](#opencode), [Gemini CLI](#gemini).

## What it does not do

| Not included | Why |
|---|---|
| Price tables | Cost is only reported when the product recorded it (`Usage.cost_usd`) |
| Live tailing | It parses what is on disk. `parse_file_tail` reads the end of a large file, but nothing watches for changes |
| Writing or editing transcripts | Everything is read-only |
| Search or indexing | It parses sessions. Indexing is for the tool built on top |
| Decrypting anything | Encrypted reasoning is flagged (`encrypted: true`) and never stored |
| Credentials and product config | OpenCode credential and account tables are never read, and config files are out of scope |

## Two ways to use it

| Use | Start here |
|---|---|
| As a Rust library | [Quick Start](#quick-start), then [Rust API](#api) |
| As a command line tool | [CLI](#cli) |

## Next

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Sessions and Entries](#model)
