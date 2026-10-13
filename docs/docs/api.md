# Rust API

Everything public is re-exported from the crate root, except the OpenCode functions, which live in `agentparse::opencode`. The model types (`Session`, `Entry`, `Tool`, and the rest) are described on [Sessions and Entries](#model).

```rust
use agentparse::{
    Agent, Error, Result, Roots, Session, SessionRef, Source, Warning,
    discover, parse, parse_file, parse_file_tail, parse_reader,
};
use agentparse::opencode;
```

The crate forbids `unsafe` code. All functions are synchronous and only read.

## Functions at a glance

| Function | Use it to |
|---|---|
| `parse_file(path)` | Parse one session file of any product |
| `parse_file_tail(path, max_bytes)` | Parse only the end of a large Claude Code or Codex file |
| `parse_reader(agent, reader, source)` | Parse lines you already hold, for a product you name |
| `parse(&SessionRef)` | Parse a session that `discover` found |
| `discover(&Roots)` | List sessions on disk without parsing them |
| `Roots::from_env()` | Build the default locations, honoring environment overrides |
| `opencode::list(db)` | List the sessions in one OpenCode database |
| `opencode::parse_db(db, id)` | Parse one session from an OpenCode database |
| `opencode::parse_storage_file(path)` | Parse one session from the old OpenCode JSON tree |

## parse_file

```rust
pub fn parse_file(path: &Path) -> Result<Session>
```

Parses one session file. The product and format are detected from the file extension and the first record, so you do not name the product.

| Input | Result |
|---|---|
| A `.jsonl.zst` file | Decoded and parsed as a Codex rollout |
| A `.json` file | A Gemini CLI G1 chat, or an OpenCode legacy session file |
| A `.jsonl` file | Claude Code, Codex or Gemini CLI, from the first parseable JSON object among the first 32 lines. A corrupt line near the top does not hide the format |
| Anything else | `Error::NotASession` |

An OpenCode database holds many sessions, so it is not a session file. Use `opencode::list` and `opencode::parse_db`.

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

## parse_file_tail

```rust
pub fn parse_file_tail(path: &Path, max_bytes: u64) -> Result<Session>
```

Parses only the last `max_bytes` of a JSONL transcript, for Claude Code and Codex. It reads from one byte early and drops the first line if the window starts in the middle of it, so a torn first line never produces a warning.

| Rule | Detail |
|---|---|
| Which products | Claude Code and Codex. Gemini CLI, OpenCode, and `.zst` files are parsed whole |
| Small files | A file no longer than `max_bytes + 1` is parsed whole |
| Line numbers | `Entry.line` and `Warning.line` count from the start of the window, not the file |
| Missing context | A result whose call is before the window stays as `Part::ToolResult`, with a warning |
| Session fields | Fields that live in the header of a file (a Codex session meta line, for example) are missing when the window does not contain it |

```rust
use agentparse::parse_file_tail;

fn main() -> Result<(), agentparse::Error> {
    let path = std::env::args().nth(1).expect("usage: tail <session file>");
    // The last 16 KiB of a Claude or Codex transcript.
    let session = parse_file_tail(path.as_ref(), 16 * 1024)?;
    println!("{} entries, {} warnings", session.entries.len(), session.warnings.len());
    Ok(())
}
```

## parse_reader

```rust
pub fn parse_reader(agent: Agent, reader: impl BufRead, source: Source) -> Result<Session>
```

Parses a transcript of a known product from any reader of lines. It works for Claude Code, Codex and Gemini CLI. For `Agent::Opencode` it returns `Error::NotASession`, because OpenCode sessions are not a line stream. `source` is only recorded in `Session.source`. Nothing is opened from it.

```rust
use agentparse::{Agent, Source, parse_reader};
use std::io::Cursor;

fn main() -> Result<(), agentparse::Error> {
    // Any slice of whole lines works, for example lines you already hold in memory.
    let lines = r#"{"type":"user","uuid":"u1","timestamp":"2026-10-12T09:00:00Z","sessionId":"s1","message":{"role":"user","content":"hello"}}"#;
    let source = Source::File { path: "in-memory.jsonl".into() };
    let session = parse_reader(Agent::Claude, Cursor::new(lines), source)?;
    println!("{} entries", session.entries.len());
    Ok(())
}
```

## parse

```rust
pub fn parse(r: &SessionRef) -> Result<Session>
```

Parses a session that `discover` returned. It picks the right path for you: a database row set goes to `opencode::parse_db`, an OpenCode JSON file to `opencode::parse_storage_file`, a `.zst` file through the Codex decoder, and everything else through `parse_reader` with the product from the reference.

## discover

```rust
pub fn discover(roots: &Roots) -> (Vec<SessionRef>, Vec<Warning>)
```

Lists sessions under the roots without parsing them. It never fails.

| Case | Result |
|---|---|
| A root that is `None` | That product is skipped |
| A directory that does not exist | Nothing found, no warning |
| A directory that exists but cannot be read | A `Warning` with the path and the error. An unreadable history never looks like an empty one |
| An OpenCode database that cannot be read | A `Warning` |

The returned list is in the order Claude Code, Codex, OpenCode, Gemini CLI. Inside a product the order is by path. Sort by `updated_at` if you need recency.

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

Where each product's sessions are found:

| Product | Layout |
|---|---|
| Claude Code | `<claude>/projects/<dir>/<id>.jsonl`, and `<claude>/projects/<dir>/<id>/subagents/**/agent-*.jsonl` up to 3 levels deep. A `journal.jsonl` is not listed |
| Codex | `rollout-*.jsonl` and `rollout-*.jsonl.zst` under `<codex>/sessions` and `<codex>/archived_sessions`, up to 4 levels deep |
| OpenCode | Every `opencode*.db` in the data directory (or the one database file given), then legacy JSON sessions in `storage/session/<project>/<id>.json` that no database already holds |
| Gemini CLI | `<gemini>/tmp/<project>/chats/session-*.json[l]`, and subagents in `chats/<parent session id>/*.jsonl`. When a `.json` has a `.jsonl` twin, only the `.jsonl` is listed |

## Roots

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub opencode: Option<PathBuf>,
    pub gemini: Option<PathBuf>,
}

impl Roots {
    pub fn from_env() -> Roots;
}
```

| Field | Holds |
|---|---|
| `claude` | The Claude config directory (`~/.claude`). Sessions are under `projects/` |
| `codex` | The Codex home (`~/.codex`). Rollouts are under `sessions/` and `archived_sessions/` |
| `opencode` | The OpenCode data directory (`~/.local/share/opencode`) or one database file |
| `gemini` | The Gemini directory (`~/.gemini`). Chats are under `tmp/<project>/chats/` |

`Roots::default()` has every field `None`, which discovers nothing. Fill in only the products you want.

`Roots::from_env()` returns the default locations and honors the overrides the products read themselves:

| Product | Variables |
|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR`, else `~/.claude` |
| Codex | `CODEX_HOME`, else `~/.codex` |
| OpenCode | `OPENCODE_DB` (an absolute path is the database, a bare name is looked up in the data directory), and `XDG_DATA_HOME` (data directory is `$XDG_DATA_HOME/opencode`), else `~/.local/share/opencode` |
| Gemini CLI | `GEMINI_CLI_HOME` (uses `$GEMINI_CLI_HOME/.gemini`), else `~/.gemini` |

`~` is `HOME`, or `USERPROFILE` when `HOME` is unset. An empty variable counts as unset.

## SessionRef

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub agent: Agent,
    pub id: String,
    pub source: Source,
    pub parent_id: Option<String>,
    pub updated_at: Option<i64>,
}
```

A session found on disk and not parsed yet.

| Field | Meaning |
|---|---|
| `agent` | The product |
| `id` | The session id. It can repeat across files, so `(agent, source)` identifies the session |
| `source` | A file, or a database plus session id |
| `parent_id` | The parent session id for a Claude Code, Gemini CLI or OpenCode child. `None` for a main session, and for every Codex rollout (a Codex child's parent is only known after parsing) |
| `updated_at` | Last write in epoch milliseconds: the file modification time, or the database's own update time |

## The OpenCode module

```rust
pub fn list(db: &Path) -> Result<Vec<SessionRef>>
pub fn parse_db(db: &Path, session_id: &str) -> Result<Session>
pub fn parse_storage_file(path: &Path) -> Result<Session>
```

| Function | Behavior |
|---|---|
| `list` | Every session in the database: all `session_v2` rows, plus `session` rows that are not in `session_v2`. Newest first. `parent_id` is set for subagent sessions |
| `parse_db` | One session by id. `Error::NotFound` when no table holds the id. A session in both generations is merged (format `opencode-v1+v2`) |
| `parse_storage_file` | One session from the old JSON tree, given its `storage/session/<project>/<id>.json` file. Messages and parts are read from the sibling `message` and `part` folders |

The database is opened read-only with a 2 second busy timeout, and read through SQLite so the write-ahead log is included. The file is never copied, and credential tables are never read. A failure to open, query or read the schema is an `Error::Sqlite`, never an empty session.

```rust
use agentparse::opencode;
use std::path::Path;

fn main() -> Result<(), agentparse::Error> {
    let db = std::env::args().nth(1).expect("usage: opencode_db <opencode.db>");
    for r in opencode::list(Path::new(&db))? {
        let session = opencode::parse_db(Path::new(&db), &r.id)?;
        println!("{} {} {} entries", session.id, session.format, session.entries.len());
    }
    Ok(())
}
```

## Error

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(String),
    #[error("not a session: {0}")]
    NotASession(String),
    #[error("session not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

| Variant | When |
|---|---|
| `Io` | A file cannot be opened or read |
| `Sqlite` | A database cannot be opened, queried, or has an unexpected schema |
| `NotASession` | The file is not a session of any supported product, or `parse_reader` was called with `Agent::Opencode`, or a `.zst` file cannot be decoded |
| `NotFound` | `opencode::parse_db` was given an id that no table holds |

Bad lines and bad rows inside an otherwise good session are not errors. They are `Session.warnings`.

```rust
use agentparse::{Error, parse_file};

fn main() {
    let path = std::env::args().nth(1).expect("usage: errors <file>");
    match parse_file(path.as_ref()) {
        Ok(session) => println!("{} warnings", session.warnings.len()),
        Err(Error::NotASession(what)) => println!("skip: {what}"),
        Err(Error::NotFound(id)) => println!("no such session: {id}"),
        Err(e @ (Error::Io(_) | Error::Sqlite(_))) => println!("cannot read: {e}"),
    }
}
```

## Model helpers

| Item | Signature |
|---|---|
| `Session::messages` | `fn messages(&self) -> impl Iterator<Item = &Message>` |
| `Session::tool_calls` | `fn tool_calls(&self) -> Vec<&ToolCall>`. Depth first, so nested `children` follow their parent |
| `Message::text` | `fn text(&self) -> String`. `Text` parts joined with blank lines |
| `Message::is_human` | `fn is_human(&self) -> bool`. Role `User` and origin `Human` |
| `ToolCall::failed` | `fn failed(&self) -> bool`. `Error`, `Rejected` or `Interrupted` |
| `Origin::is_synthetic` | `fn is_synthetic(self) -> bool`. Any origin except `Human` and `Model` |
| `Usage::total` | `fn total(&self) -> u64`. `input + output + cache_read + cache_write`, saturating |
| `Usage::add` | `fn add(&mut self, other: &Usage)`. Saturating. `cost_usd` stays `None` only if neither side has one |
| `Agent::as_str` | `fn as_str(self) -> &'static str`. `claude`, `codex`, `opencode`, `gemini` |
| `Agent::ALL` | `[Agent; 4]` |
| `Agent: FromStr` | Parses the four names, ignoring case. The error is a `String` |

## JSON round trip

Every model type implements `Serialize` and `Deserialize`, and a session survives a round trip unchanged. Omitted fields (`None`, empty lists, `false`) read back as their defaults.

```rust
use agentparse::{Session, parse_file};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: json <file>");
    let session = parse_file(path.as_ref())?;
    let text = serde_json::to_string(&session)?;
    let back: Session = serde_json::from_str(&text)?;
    assert_eq!(back, session);
    println!("{} bytes of JSON", text.len());
    Ok(())
}
```

This needs `serde_json` in your own `Cargo.toml`.

## Next

- [CLI](#cli)
