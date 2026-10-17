//! Reading rows from the SQLite database and files from the JSON storage tree.

use super::{Map, record::*};
use crate::model::*;
use crate::{Error, Result};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

pub(super) fn sql_err(e: rusqlite::Error) -> Error {
    Error::Sqlite(e.to_string())
}

pub(super) fn open(db: &Path) -> Result<Connection> {
    std::fs::metadata(db)?; // a missing file is an I/O error, not an empty database
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sql_err)?;
    conn.busy_timeout(Duration::from_secs(2)).map_err(sql_err)?;
    Ok(conn)
}

pub(super) fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(sql_err)
}

pub(super) fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(sql_err)?;
    let names = stmt
        .query_map([table], |r| r.get::<_, String>(0))
        .map_err(sql_err)?;
    names
        .collect::<std::result::Result<_, _>>()
        .map_err(sql_err)
}

fn cell(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(b) | ValueRef::Blob(b) => {
            Value::String(String::from_utf8_lossy(b).into_owned())
        }
    }
}

/// `SELECT *` rows as column-name maps, so older schemas with fewer columns need no special case.
fn rows_as_maps(conn: &Connection, query: &str, session_id: &str) -> Result<Vec<Map>> {
    let mut stmt = conn.prepare(query).map_err(sql_err)?;
    let names: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let rows = stmt
        .query_map(params![session_id], |r| {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| Ok((n.clone(), cell(r.get_ref(i)?))))
                .collect::<rusqlite::Result<Map>>()
        })
        .map_err(sql_err)?;
    rows.collect::<std::result::Result<_, _>>().map_err(sql_err)
}

/// The session row of `table`, or `None` when the table or the row does not exist.
pub(super) fn session_row(conn: &Connection, table: &str, id: &str) -> Result<Option<Map>> {
    if !has_table(conn, table)? {
        return Ok(None);
    }
    Ok(rows_as_maps(conn, &format!("SELECT * FROM {table} WHERE id = ?1"), id)?.pop())
}

/// Parse the JSON text of a row's `data` column; `None` when it is missing or not an object.
fn json_data(row: &mut Map) -> Option<Map> {
    match serde_json::from_str(row.remove("data")?.as_str()?) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

fn warn(message: String) -> Warning {
    Warning {
        line: None,
        message,
    }
}

pub(super) struct V1Message {
    pub(super) id: String,
    /// The `time_created` column, used when the JSON has no `time.created`.
    pub(super) created: Option<i64>,
    pub(super) data: Map,
    pub(super) parts: Vec<Map>,
}

pub(super) fn load_v1(
    conn: &Connection,
    session_id: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<V1Message>> {
    let mut parts: HashMap<String, Vec<Map>> = HashMap::new();
    for mut row in rows_as_maps(
        conn,
        "SELECT * FROM part WHERE session_id = ?1 ORDER BY message_id, id",
        session_id,
    )? {
        let (id, message_id) = (
            take_str(&mut row, "id").unwrap_or_default(),
            take_str(&mut row, "message_id"),
        );
        match (json_data(&mut row), message_id) {
            (Some(data), Some(mid)) => parts.entry(mid).or_default().push(data),
            _ => warnings.push(warn(format!("part {id}: unreadable row"))),
        }
    }
    let query = "SELECT * FROM message WHERE session_id = ?1 ORDER BY time_created, id";
    let mut messages = Vec::new();
    for mut row in rows_as_maps(conn, query, session_id)? {
        let id = take_str(&mut row, "id").unwrap_or_default();
        let Some(data) = json_data(&mut row) else {
            warnings.push(warn(format!("message {id}: invalid JSON")));
            continue;
        };
        let created = take_i64(&mut row, "time_created");
        messages.push(V1Message {
            parts: parts.remove(&id).unwrap_or_default(),
            id,
            created,
            data,
        });
    }
    Ok(messages)
}

pub(super) struct V2Row {
    pub(super) id: String,
    pub(super) ty: String,
    pub(super) created: Option<i64>,
    pub(super) data: Map,
}

pub(super) fn load_v2(
    conn: &Connection,
    session_id: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<V2Row>> {
    let mut rows = Vec::new();
    for mut row in rows_as_maps(
        conn,
        "SELECT * FROM session_message WHERE session_id = ?1 ORDER BY seq",
        session_id,
    )? {
        let id = take_str(&mut row, "id").unwrap_or_default();
        let Some(data) = json_data(&mut row) else {
            warnings.push(warn(format!("session_message {id}: invalid JSON")));
            continue;
        };
        let ty = take_str(&mut row, "type").unwrap_or_default();
        rows.push(V2Row {
            id,
            ty,
            created: take_i64(&mut row, "time_created"),
            data,
        });
    }
    Ok(rows)
}

/// Input still waiting in `session_pending` (typed while the session was busy), by admission.
pub(super) fn load_pending(conn: &Connection, session_id: &str) -> Result<Vec<V2Row>> {
    if !has_table(conn, "session_pending")? {
        return Ok(Vec::new());
    }
    let query = "SELECT * FROM session_pending WHERE session_id = ?1 ORDER BY admitted_seq";
    let mut rows = Vec::new();
    for mut row in rows_as_maps(conn, query, session_id)? {
        let id = take_str(&mut row, "id").unwrap_or_default();
        let Some(mut data) = json_data(&mut row) else {
            continue;
        };
        if let Some(delivery) = take_str(&mut row, "delivery") {
            data.insert("delivery".into(), Value::String(delivery));
        }
        rows.push(V2Row {
            id,
            ty: take_str(&mut row, "type").unwrap_or_default(),
            created: take_i64(&mut row, "time_created"),
            data,
        });
    }
    Ok(rows)
}

/// The child sessions of `id` as (id, title), oldest first, from either session table.
pub(super) fn children_of(conn: &Connection, id: &str) -> Result<Vec<(String, String)>> {
    let mut children: Vec<(String, String)> = Vec::new();
    for table in ["session_v2", "session"] {
        if !has_table(conn, table)? || !columns(conn, table)?.iter().any(|c| c == "parent_id") {
            continue;
        }
        let query = format!(
            "SELECT id, COALESCE(title, '') FROM {table} WHERE parent_id = ?1 ORDER BY time_created, id"
        );
        let mut stmt = conn.prepare(&query).map_err(sql_err)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql_err)?;
        for row in rows {
            let row = row.map_err(sql_err)?;
            if !children.iter().any(|(c, _)| *c == row.0) {
                children.push(row);
            }
        }
    }
    Ok(children)
}

pub(super) fn todos_entry(conn: &Connection, session_id: &str) -> Result<Option<Entry>> {
    if !has_table(conn, "todo")? {
        return Ok(None);
    }
    let rows = rows_as_maps(
        conn,
        "SELECT * FROM todo WHERE session_id = ?1 ORDER BY position",
        session_id,
    )?;
    let ts = rows
        .iter()
        .filter_map(|r| r.get("time_updated")?.as_i64())
        .max();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|mut r| json!({"content": r.remove("content"), "status": r.remove("status"), "priority": r.remove("priority")}))
        .collect();
    Ok((!items.is_empty()).then(|| todos_entry_from(Value::Array(items), ts)))
}

pub(super) fn todos_entry_from(items: Value, ts: Option<i64>) -> Entry {
    entry(
        None,
        ts,
        None,
        Map::new(),
        EntryKind::Other(Other {
            kind: "todos".into(),
            data: items,
        }),
    )
}

/// The parent link of a subagent session. The spawning call is looked up in the parent's rows.
/// A row whose JSON cannot be decoded only means no `call_id`; any other query failure is an error.
pub(super) fn parent_link(conn: &Connection, row: &Map, id: &str) -> Result<Option<ParentLink>> {
    let Some(session_id) = row
        .get("parent_id")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    else {
        return Ok(None);
    };
    // The spawning call: its id, and the agent it asked for (v1 `subagent_type`, v2 `agent`).
    // opencode 2.x titles a child with the bare description, so the title is only a fallback.
    let agent = |v: &str| {
        format!(
            "COALESCE(json_extract({v}, '$.state.input.subagent_type'), json_extract({v}, '$.state.input.agent'))"
        )
    };
    let spawned = |v: &str| {
        format!(
            "(json_extract({v}, '$.state.metadata.sessionId') = ?2 OR json_extract({v}, '$.state.metadata.sessionID') = ?2)"
        )
    };
    let lookups = [
        (
            "part",
            format!(
                "SELECT json_extract(data, '$.callID'), {} FROM part WHERE session_id = ?1 AND json_extract(data, '$.tool') IN ('task', 'subagent') AND {} LIMIT 1",
                agent("data"),
                spawned("data")
            ),
        ),
        (
            "session_message",
            format!(
                "SELECT json_extract(b.value, '$.id'), {} FROM session_message m, json_each(m.data, '$.content') b WHERE m.session_id = ?1 AND m.type = 'assistant' AND json_extract(b.value, '$.name') IN ('task', 'subagent') AND {} LIMIT 1",
                agent("b.value"),
                spawned("b.value")
            ),
        ),
    ];
    let mut spawn = (None, None);
    for (table, query) in lookups {
        if spawn.0.is_some() || !has_table(conn, table)? {
            continue;
        }
        let found = conn.query_row(&query, params![session_id, id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        });
        match found {
            Ok(found) => spawn = found,
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) if e.to_string().contains("malformed JSON") => {}
            Err(e) => return Err(sql_err(e)),
        }
    }
    let title = row.get("title").and_then(Value::as_str).unwrap_or_default();
    let from_title = title
        .rsplit_once("(@")
        .and_then(|(_, rest)| rest.strip_suffix(" subagent)"))
        .map(str::to_string);
    Ok(Some(ParentLink {
        session_id: session_id.to_string(),
        call_id: spawn.0,
        agent_type: spawn.1.or(from_title),
    }))
}

pub(super) fn read_json(path: &Path) -> Result<Option<Value>> {
    Ok(serde_json::from_slice(&std::fs::read(path)?).ok())
}

/// A JSON file that may be absent or broken: a missing file is `None`, a broken one adds a warning.
pub(super) fn read_json_lenient(path: &Path, warnings: &mut Vec<Warning>) -> Option<Value> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| warnings.push(warn(format!("{}: {e}", path.display()))))
            .ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warnings.push(warn(format!("{}: {e}", path.display())));
            None
        }
    }
}

/// The `*.json` objects of a directory, sorted by file name (ids sort chronologically).
fn json_objects(dir: &Path, warnings: &mut Vec<Warning>) -> Vec<(String, Map)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            match read_json_lenient(&path, warnings) {
                Some(Value::Object(m)) => Some((stem, m)),
                Some(_) => {
                    warnings.push(warn(format!("{}: not a JSON object", path.display())));
                    None
                }
                None => None,
            }
        })
        .collect()
}

pub(super) fn load_json_messages(
    root: &Path,
    session_id: &str,
    warnings: &mut Vec<Warning>,
) -> Vec<V1Message> {
    let dir = root.join("message").join(session_id);
    if !dir.is_dir() {
        warnings.push(warn(format!("{}: no message directory", dir.display())));
    }
    let mut messages: Vec<V1Message> = json_objects(&dir, warnings)
        .into_iter()
        .map(|(stem, data)| {
            let created = time_of(&data, "created");
            let id = data
                .get("id")
                .and_then(Value::as_str)
                .map_or(stem, str::to_string);
            V1Message {
                id,
                created,
                data,
                parts: Vec::new(),
            }
        })
        .collect();
    messages.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    for m in &mut messages {
        m.parts = json_objects(&root.join("part").join(&m.id), warnings)
            .into_iter()
            .map(|(_, p)| p)
            .collect();
    }
    messages
}

/// Session info JSON keys to the column names of the database row.
pub(super) fn row_from_info(mut info: Map) -> Map {
    if let Some(Value::Object(time)) = info.remove("time") {
        info.extend(time.into_iter().map(|(k, v)| (format!("time_{k}"), v)));
    }
    if let Some(parent) = info.remove("parentID") {
        info.insert("parent_id".into(), parent);
    }
    info
}
