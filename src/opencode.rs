//! OpenCode sessions: the SQLite database (v1 `session`/`message`/`part` tables, v2
//! `session_v2`/`session_message` tables, or both for a migrated session) and the legacy JSON
//! storage tree.
//!
//! Which rows win for one session id:
//! - only in `session_v2`: its `session_message` rows by `seq` (`opencode-v2`);
//! - only in `session`: its `message`/`part` rows by `(time_created, id)` (`opencode-v1`);
//! - in both (a migrated session): the lossless v1 rows first, then the v2 rows whose id is not
//!   a v1 message id, by `seq` (`opencode-v1+v2`), minus the `synthetic` rows the migration
//!   split out of a v1 user message (same first 16 id characters as that message). What is
//!   left is the migration notice (a `system` row) and everything written after the upgrade.
//!
//! The database is opened read-only and read through SQLite (the WAL included); the credential
//! and account tables are never touched.
//!
//! Layout: `db` reads rows and files, `v1` and `v2` turn message records into entries, `tool`
//! turns tool parts of both generations into [`ToolCall`]s, `record` holds the JSON helpers.

mod db;
mod record;
mod tool;
mod v1;
mod v2;

use crate::model::*;
use crate::{Error, Result, SessionRef};
use db::*;
use record::*;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use v1::v1_entries;
use v2::v2_entries;

type Map = serde_json::Map<String, Value>;

/// Session row columns that a typed `Session` field takes; the rest land in `Session.extra`.
const TYPED_COLUMNS: &[&str] = &[
    "id",
    "parent_id",
    "directory",
    "title",
    "version",
    "time_created",
    "time_updated",
];
/// Session row columns that hold JSON text.
const JSON_COLUMNS: &[&str] = &[
    "summary_diffs",
    "metadata",
    "revert",
    "permission",
    "model",
    "fork_boundary",
];

/// Every session in an OpenCode database: the `session_v2` rows plus the `session` rows that
/// are not in `session_v2`, newest first.
pub fn list(db: &Path) -> Result<Vec<SessionRef>> {
    let conn = open(db)?;
    // One deferred read transaction pins a single snapshot for every query below.
    let conn = conn.unchecked_transaction().map_err(sql_err)?;
    let has_v2 = has_table(&conn, "session_v2")?;
    let mut refs = Vec::new();
    for table in ["session_v2", "session"] {
        if !has_table(&conn, table)? {
            continue;
        }
        let cols = columns(&conn, table)?;
        let col = |name: &'static str| {
            if cols.iter().any(|c| c == name) {
                name
            } else {
                "NULL"
            }
        };
        let skip = if table == "session" && has_v2 {
            "WHERE id NOT IN (SELECT id FROM session_v2)"
        } else {
            ""
        };
        let query = format!(
            "SELECT id, {}, {} FROM {table} {skip} ORDER BY 3 DESC, 1",
            col("parent_id"),
            col("time_updated")
        );
        let mut stmt = conn.prepare(&query).map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(sql_err)?;
        for row in rows {
            let (id, parent_id, updated_at) = row.map_err(sql_err)?;
            refs.push(SessionRef {
                agent: Agent::Opencode,
                source: Source::Sqlite {
                    path: db.to_path_buf(),
                    session_id: id.clone(),
                },
                id,
                parent_id: parent_id.filter(|p| !p.is_empty()),
                updated_at,
            });
        }
    }
    Ok(refs)
}

/// One session from an OpenCode database. `Error::NotFound` when no table holds that id.
pub fn parse_db(db: &Path, session_id: &str) -> Result<Session> {
    let conn = open(db)?;
    // One deferred read transaction pins a single snapshot: a writer committing a message
    // between the part and message queries cannot split it from its parts.
    let conn = conn.unchecked_transaction().map_err(sql_err)?;
    let v1 = session_row(&conn, "session", session_id)?;
    let v2 = session_row(&conn, "session_v2", session_id)?;
    let (has_v1, has_v2) = (v1.is_some(), v2.is_some());
    let (mut row, format) = match (v1, v2) {
        (None, None) => return Err(Error::NotFound(session_id.to_string())),
        (Some(_), Some(v2)) => (v2, "opencode-v1+v2"),
        (Some(v1), None) => (v1, "opencode-v1"),
        (None, Some(v2)) => (v2, "opencode-v2"),
    };

    let mut warnings = Vec::new();
    let v1_messages = if has_v1 {
        load_v1(&conn, session_id, &mut warnings)?
    } else {
        Vec::new()
    };
    let v1_ids: HashSet<&str> = v1_messages.iter().map(|m| m.id.as_str()).collect();
    // The migration splits a user message's synthetic text into a `synthetic` row under a new
    // id that keeps the first 16 characters of the v1 id; that row repeats v1 content.
    let v1_prefixes: HashSet<&str> = v1_messages.iter().filter_map(|m| m.id.get(..16)).collect();
    let migrated_copy = |r: &V2Row| {
        v1_ids.contains(r.id.as_str())
            || r.ty == "synthetic" && r.id.get(..16).is_some_and(|p| v1_prefixes.contains(p))
    };
    let v2_rows: Vec<V2Row> = if has_v2 {
        let rows = load_v2(&conn, session_id, &mut warnings)?;
        rows.into_iter().filter(|r| !migrated_copy(r)).collect()
    } else {
        Vec::new()
    };

    let mut entries = v1_entries(v1_messages);
    entries.extend(v2_entries(v2_rows));
    entries.extend(todos_entry(&conn, session_id)?);

    let mut session = Session::new(
        Agent::Opencode,
        session_id,
        Source::Sqlite {
            path: db.to_path_buf(),
            session_id: session_id.to_string(),
        },
        format,
    );
    let parent = parent_link(&conn, &row, session_id)?;
    // The row's `cost` is the product's own running total for the whole session, so it is the
    // session cost; a 0 is the column default of
    // sessions that predate the usage columns, and then the rows' own costs are the truth.
    session.usage.cost_usd = row
        .remove("cost")
        .and_then(|c| c.as_f64())
        .filter(|c| *c > 0.0);
    fill_header(&mut session, row, parent);
    session.entries = entries;
    session.warnings = warnings;
    Ok(finish_session(session))
}

/// One session from legacy JSON storage, given its `storage/session/<project>/<id>.json` file.
/// Messages come from `storage/message/<id>/`, parts from `storage/part/<messageID>/`.
pub(crate) fn parse_storage_file(path: &Path) -> Result<Session> {
    let not_a_session = || Error::NotASession(path.display().to_string());
    let info = match read_json(path)? {
        Some(Value::Object(info)) => info,
        _ => return Err(not_a_session()),
    };
    let id = info
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(not_a_session)?
        .to_string();
    // Only `storage/session/<project>/<id>.json` is a session; message and part files carry
    // an `id` too but are records inside one.
    let in_session_dir =
        path.ancestors().nth(2).and_then(Path::file_name) == Some("session".as_ref());
    let root = path
        .ancestors()
        .nth(3)
        .filter(|_| in_session_dir)
        .ok_or_else(not_a_session)?;

    let mut warnings = Vec::new();
    let messages = load_json_messages(root, &id, &mut warnings);
    let mut entries = v1_entries(messages);
    let todos = read_json_lenient(&root.join("todo").join(format!("{id}.json")), &mut warnings);
    entries.extend(
        todos
            .filter(|t| t.as_array().is_some_and(|a| !a.is_empty()))
            .map(|t| todos_entry_from(t, None)),
    );

    let mut row = row_from_info(info);
    if let Some(diff) = read_json_lenient(
        &root.join("session_diff").join(format!("{id}.json")),
        &mut warnings,
    ) {
        row.insert("session_diff".into(), diff);
    }
    let mut session = Session::new(
        Agent::Opencode,
        id,
        Source::File {
            path: path.to_path_buf(),
        },
        "opencode-json",
    );
    let parent = take_str(&mut row, "parent_id").map(|session_id| ParentLink {
        session_id,
        call_id: None,
        agent_type: None,
    });
    fill_header(&mut session, row, parent);
    session.entries = entries;
    session.warnings = warnings;
    Ok(finish_session(session))
}

/// Everything that needs the whole session: a child's first prompt comes from another agent
/// (the parent's call), and a shell call that names no directory ran in the session directory.
fn finish_session(mut session: Session) -> Session {
    let child = session.parent.is_some();
    let mut first_prompt = child;
    for entry in &mut session.entries {
        let EntryKind::Message(m) = &mut entry.kind else {
            continue;
        };
        if first_prompt && m.is_human() {
            m.origin = Origin::Agent;
            first_prompt = false;
        }
        for part in &mut m.parts {
            if let Part::ToolCall(call) = part {
                if let Tool::Shell(shell) = &mut call.tool {
                    shell.cwd = shell.cwd.take().or_else(|| session.cwd.clone());
                }
            }
        }
    }
    session.finish()
}

fn fill_header(session: &mut Session, mut row: Map, parent: Option<ParentLink>) {
    session.title = take_str(&mut row, "title").filter(|t| !t.is_empty());
    session.cwd = take_str(&mut row, "directory").filter(|d| !d.is_empty());
    session.cli_version = take_str(&mut row, "version");
    session.started_at = take_i64(&mut row, "time_created");
    session.updated_at = take_i64(&mut row, "time_updated");
    session.parent = parent;
    row.retain(|k, v| !v.is_null() && !TYPED_COLUMNS.contains(&k.as_str()));
    for column in JSON_COLUMNS {
        let parsed = row
            .get(*column)
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        if let Some(v) = parsed {
            row.insert((*column).to_string(), v);
        }
    }
    session.extra = extra_of(row);
}
