//! Parse Claude Code, Codex, OpenCode and Gemini CLI session transcripts into one model.
//!
//! ```no_run
//! let session = agentparse::parse_file("rollout.jsonl".as_ref())?;
//! for call in session.tool_calls() {
//!     if let agentparse::Tool::Shell(shell) = &call.tool {
//!         let exit = call.output.as_ref().and_then(|o| o.shell.as_ref()).and_then(|s| s.exit_code);
//!         println!("$ {} -> {:?}", shell.command, exit);
//!     }
//! }
//! # Ok::<(), agentparse::Error>(())
//! ```

#![forbid(unsafe_code)]

mod claude;
mod codex;
mod discover;
mod gemini;
mod model;
pub mod opencode;
mod patch;
mod util;

pub use discover::{Roots, SessionRef, discover};
pub use model::*;

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

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

/// Parse one session file of any agent; the agent and format are detected from the path and
/// the first record. OpenCode databases hold many sessions: use [`opencode::parse_db`].
pub fn parse_file(path: &Path) -> Result<Session> {
    let source = Source::File {
        path: path.to_path_buf(),
    };
    if is_zst(path) {
        return codex::parse_reader(zst_reader(path)?, source);
    }
    let agent = sniff(path)?.ok_or_else(|| Error::NotASession(path.display().to_string()))?;
    if agent == Agent::Opencode {
        return opencode::parse_storage_file(path);
    }
    parse_reader(agent, BufReader::new(std::fs::File::open(path)?), source)
}

/// Parse only the last `max_bytes` of a JSONL transcript (Claude, Codex). The torn first line
/// is dropped; line numbers count from the start of the window. Other formats parse whole.
pub fn parse_file_tail(path: &Path, max_bytes: u64) -> Result<Session> {
    let agent = if is_zst(path) { None } else { sniff(path)? };
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let Some(agent @ (Agent::Claude | Agent::Codex)) = agent else {
        return parse_file(path);
    };
    if len <= max_bytes.saturating_add(1) {
        return parse_file(path);
    }
    // Read from one byte early: if that byte ends a line the window starts on a whole record.
    file.seek(SeekFrom::Start(len - max_bytes - 1))?;
    let mut reader = BufReader::new(file);
    reader.skip_until(b'\n')?;
    parse_reader(
        agent,
        reader,
        Source::File {
            path: path.to_path_buf(),
        },
    )
}

/// Parse a transcript of a known agent from any slice of its lines (Claude, Codex, Gemini).
pub fn parse_reader(agent: Agent, reader: impl BufRead, source: Source) -> Result<Session> {
    match agent {
        Agent::Claude => claude::parse_reader(reader, source),
        Agent::Codex => codex::parse_reader(reader, source),
        Agent::Gemini => gemini::parse_reader(reader, source),
        Agent::Opencode => Err(Error::NotASession(
            "opencode sessions live in a database or JSON storage, not a line stream".into(),
        )),
    }
}

/// Parse a discovered session.
pub fn parse(r: &SessionRef) -> Result<Session> {
    match &r.source {
        Source::Sqlite { path, session_id } => opencode::parse_db(path, session_id),
        Source::File { path } if r.agent == Agent::Opencode => opencode::parse_storage_file(path),
        Source::File { path } if is_zst(path) => parse_file(path),
        Source::File { path } => parse_reader(
            r.agent,
            BufReader::new(std::fs::File::open(path)?),
            r.source.clone(),
        ),
    }
}

fn is_zst(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "zst")
}

fn zst_reader(path: &Path) -> Result<impl BufRead> {
    let decoder = ruzstd::decoding::StreamingDecoder::new(std::fs::File::open(path)?)
        .map_err(|e| Error::NotASession(format!("{}: zstd: {e}", path.display())))?;
    Ok(BufReader::new(decoder))
}

/// Which product wrote this file, from its extension and first JSON record.
fn sniff(path: &Path) -> Result<Option<Agent>> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    if path.extension().is_some_and(|e| e == "json") {
        // Gemini G1 chats and OpenCode storage records are whole JSON documents.
        let mut text = String::new();
        reader.read_to_string(&mut text)?;
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let has = |k: &str| v.get(k).is_some();
        return Ok(if has("sessionId") && has("messages") {
            Some(Agent::Gemini)
        } else if has("id") && (has("projectID") || has("sessionID")) {
            Some(Agent::Opencode)
        } else {
            None
        });
    }
    // The first parseable JSON object among the first lines: a corrupt line must not hide the format.
    let mut v = serde_json::Value::Null;
    let mut line = Vec::new();
    for _ in 0..32 {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if let Ok(parsed @ serde_json::Value::Object(_)) = serde_json::from_slice(line.trim_ascii())
        {
            v = parsed;
            break;
        }
    }
    let has = |k: &str| v.get(k).is_some();
    let typed = v.get("type").is_some_and(|t| t.is_string());
    Ok(if has("sessionId") && has("projectHash") {
        Some(Agent::Gemini)
    } else if has("payload") && typed
        || has("record_type")
        || has("id") && has("timestamp") && !has("uuid") && !has("sessionId")
    {
        Some(Agent::Codex)
    } else if v.is_object() && (typed || has("sessionId")) {
        Some(Agent::Claude)
    } else {
        None
    })
}
