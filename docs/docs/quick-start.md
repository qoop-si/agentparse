# Quick Start

This page parses a session, reads the conversation, and finds sessions on disk. The examples were run on the synthetic fixtures in `tests/fixtures/` of the repository. Every snippet compiles against the current crate.

## Parse one file

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

`parse_file` detects the product and the format on its own. `Session::tool_calls()` returns every call in order, including calls nested inside Codex code cells. Each call carries its result in `call.output`, so there is nothing to pair.

On the Codex fixture `tests/fixtures/codex/real-exit3.jsonl`:

```
$ ls -la
  status Ok, exit Some(0)
$ sh test.sh
  status Error, exit Some(3)
$ cat a.txt
  status Ok, exit Some(0)
```

`sh test.sh` exits with 3, so its status is `Error`. Codex records stdout and stderr as one stream, so there is no `stderr` line. On a Claude Code fixture that recorded both streams (`tests/fixtures/claude/old-format.jsonl`):

```
$ sh test.sh
  status Error, exit Some(3)
  stderr: err-line
```

## Read the conversation

A session is a list of entries. Most are messages. The rest are events such as turn ends, compactions and settings changes.

```rust
use agentparse::{EntryKind, Origin, Part, parse_file};

fn main() -> Result<(), agentparse::Error> {
    let path = std::env::args().nth(1).expect("usage: walk <file>");
    let session = parse_file(path.as_ref())?;
    for entry in &session.entries {
        match &entry.kind {
            EntryKind::Message(m) if m.is_human() => println!("human: {}", m.text()),
            EntryKind::Message(m) if m.origin == Origin::Model => {
                let calls = m.parts.iter().filter(|p| matches!(p, Part::ToolCall(_))).count();
                println!("model: {} chars, {calls} tool calls", m.text().len());
            }
            EntryKind::Compaction(c) => println!("compaction: {:?} to {:?}", c.tokens_before, c.tokens_after),
            EntryKind::Turn(t) => println!("turn {:?}", t.status),
            _ => {}
        }
    }
    Ok(())
}
```

On `tests/fixtures/claude/main-session.jsonl`, trimmed:

```
human: fix the failing test in @test.sh
model: 43 chars, 1 tool calls
model: 0 chars, 1 tool calls
model: 39 chars, 1 tool calls
model: 86 chars, 0 tool calls
turn None
human: summarize the demo docs
```

`Message::is_human()` is true only for text a person typed. Messages the harness injected (reminders, environment context, command output) have another `Origin`, so they do not show up as human prompts. See [Usage and Status](#conventions) for the origin rules.

## Find sessions on disk

`discover` lists sessions without parsing them. `parse` turns one `SessionRef` into a `Session`.

```rust
use agentparse::{Roots, Usage, discover, parse};

fn main() {
    let (refs, warnings) = discover(&Roots::from_env());
    for w in &warnings {
        eprintln!("warning: {}", w.message);
    }
    let mut total = Usage::default();
    for r in &refs {
        match parse(r) {
            Ok(session) => total.add(&session.usage),
            Err(e) => eprintln!("{} {}: {e}", r.agent.as_str(), r.id),
        }
    }
    println!("{} sessions, {} tokens, cost {:?}", refs.len(), total.total(), total.cost_usd);
}
```

A missing product directory is not an error, it just finds nothing. A directory that exists but cannot be read becomes a warning, so an unreadable history never looks like an empty one.

To try it without touching your own sessions, lay a copy of the fixtures out like each product does and point the product variables at it (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `OPENCODE_DB`). With the repository fixtures laid out that way (the OpenCode database built with `sqlite3 opencode.db < tests/fixtures/opencode/v2.sql`), the program prints:

```
8 sessions, 415792 tokens, cost Some(0.0177)
```

The cost comes only from OpenCode, the one product among the fixtures that records it per message. The others add nothing, and a sum of recorded costs stays `None` when no message has one.

## Use the command line

```bash
agentparse list
agentparse show path/to/session.jsonl
agentparse show <session id> --json
agentparse stats
```

See [CLI](#cli) for every command and real output.

## Next

- [Sessions and Entries](#model): every type and field
- [Tool Calls](#tools): the `Tool` variants and name mapping per product
- [Shell Commands](#shell): exit code, streams, duration, background jobs
- [Rust API](#api): every public function
