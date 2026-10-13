//! v2 `session_message` rows: a flat log where one assistant row holds several steps' content.

use super::tool::{Notice, split_report, v2_call};
use super::{Map, db::V2Row, record::*};
use crate::model::*;
use crate::util::base64_len;
use serde_json::Value;

/// A background shell job's final report, delivered later as a `synthetic` row.
struct Completion {
    shell_id: String,
    exit: Option<i32>,
    /// `completed`, `cancelled` or `error`.
    state: String,
    timed_out: bool,
    signal: Option<String>,
    truncated: bool,
    stdout: Option<String>,
    notice: Notice,
    ts: Option<i64>,
}

pub(super) fn v2_entries(rows: Vec<V2Row>) -> Vec<Entry> {
    let mut turn: Option<String> = None;
    let mut completions = Vec::new();
    let mut out = Vec::new();
    for row in rows {
        match row.ty.as_str() {
            "user" => {
                turn = Some(row.id.clone());
                out.push(v2_user(row));
            }
            "synthetic" => out.push(v2_synthetic(row, &mut completions)),
            "system" => out.push(v2_text_message(
                row,
                Role::System,
                Origin::Context,
                "system",
            )),
            "skill" => out.push(v2_text_message(row, Role::User, Origin::Context, "skill")),
            "shell" => out.push(v2_shell(row)),
            "assistant" => out.extend(v2_assistant(row, turn.clone())),
            "compaction" => out.extend(v2_compaction(row)),
            "idle" => out.push(v2_idle(row)),
            "agent-switched" | "model-switched" | "location-switched" => out.push(v2_switch(row)),
            _ => {
                let kind = EntryKind::Other(Other {
                    kind: format!("session_message.{}", row.ty),
                    data: Value::Object(row.data),
                });
                out.push(entry(Some(row.id), row.created, None, Map::new(), kind));
            }
        }
    }
    apply_completions(&mut out, &completions);
    out
}

fn v2_user(row: V2Row) -> Entry {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let ts = take_time(&mut data).or(created);
    let mut parts: Vec<Part> = take_str(&mut data, "text")
        .into_iter()
        .map(|text| Part::Text { text })
        .collect();
    // `description` and `mention` of each file, index aligned with the files.
    let mut file_extras = Vec::new();
    for file in take_objects(&mut data, "files") {
        let (media, leftover) = v2_file(file);
        parts.push(Part::Attachment(media));
        file_extras.push(leftover.map_or(Value::Null, Value::Object));
    }
    if file_extras.iter().any(|f| !f.is_null()) {
        data.insert("files".into(), Value::Array(file_extras));
    }
    for agent in take_objects(&mut data, "agents") {
        parts.push(other_part("agent", agent));
    }
    for skill in take_objects(&mut data, "skills") {
        parts.push(other_part("skill", skill));
    }
    let msg = Message::new(Role::User, Origin::Human, parts);
    // A prompt starts its turn, so its turn id is its own id.
    entry(
        Some(id.clone()),
        ts,
        Some(id),
        data,
        EntryKind::Message(msg),
    )
}

/// A user file `{data (base64), mime, source: {type, uri?}, name, description?, mention?}`;
/// the bytes are only sized. The keys no `Media` field takes come back as the leftover.
fn v2_file(mut f: Map) -> (Media, Option<Map>) {
    let source = obj(f.remove("source"));
    let uri = source
        .get("uri")
        .and_then(Value::as_str)
        .map(str::to_string);
    let bytes = take_str(&mut f, "data").map(|d| base64_len(&d));
    let media = Media {
        mime: take_str(&mut f, "mime"),
        name: take_str(&mut f, "name"),
        url: uri,
        bytes,
    };
    (media, (!f.is_empty()).then_some(f))
}

fn v2_text_message(row: V2Row, role: Role, origin: Origin, kind: &str) -> Entry {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let ts = take_time(&mut data).or(created);
    let text = take_str(&mut data, "text").unwrap_or_default();
    let msg = Message {
        kind: Some(kind.into()),
        ..Message::new(role, origin, vec![Part::Text { text }])
    };
    entry(Some(id), ts, None, data, EntryKind::Message(msg))
}

fn v2_synthetic(row: V2Row, completions: &mut Vec<Completion>) -> Entry {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let ts = take_time(&mut data).or(created);
    let text = take_str(&mut data, "text").unwrap_or_default();
    let meta = data.get("metadata").cloned().unwrap_or(Value::Null);
    let source = meta
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_string);
    if source.as_deref() == Some("shell") {
        if let Some(done) = completion_of(&meta, data.get("description"), &text, ts) {
            completions.push(done);
        }
    }
    let origin = if source.is_some() {
        Origin::Notification
    } else {
        Origin::Context
    };
    let msg = Message {
        kind: Some(source.unwrap_or_else(|| "synthetic".into())),
        ..Message::new(Role::User, origin, vec![Part::Text { text }])
    };
    entry(Some(id), ts, None, data, EntryKind::Message(msg))
}

fn completion_of(
    meta: &Value,
    description: Option<&Value>,
    text: &str,
    ts: Option<i64>,
) -> Option<Completion> {
    let get = |k: &str| meta.get(k).and_then(Value::as_str);
    let shell_id = get("shellID").or_else(|| get("jobID"))?.to_string();
    let state = get("state").unwrap_or_default().to_string();
    let job_id = get("jobID").unwrap_or(&shell_id);
    let command = description.and_then(Value::as_str).unwrap_or_default();
    let body = shell_report_body(
        text,
        &format!("<shell id=\"{job_id}\" state=\"{state}\" command=\"{command}\">\n"),
    );
    let (stdout, notice) = body.map(|b| split_report(&b)).unzip();
    Some(Completion {
        shell_id,
        exit: meta
            .get("exit")
            .and_then(Value::as_i64)
            .and_then(|e| i32::try_from(e).ok()),
        state,
        timed_out: meta.get("timeout") == Some(&Value::Bool(true)),
        signal: get("signal").map(str::to_string),
        truncated: meta.get("truncated") == Some(&Value::Bool(true)),
        stdout,
        notice: notice.unwrap_or_default(),
        ts,
    })
}

/// The report between `<shell ...>` and `</shell>`. The header is rebuilt from the row (the
/// command is not escaped, so it may contain `">`); the first `">` line is the fallback.
fn shell_report_body(text: &str, header: &str) -> Option<String> {
    let body = match text.strip_prefix(header) {
        Some(body) => body,
        None => {
            let open = text.find("<shell ")?;
            text[open..].split_once("\">\n")?.1
        }
    };
    let body = body.strip_suffix("\n</shell>").unwrap_or(body);
    Some(body.trim_end_matches('\n').to_string())
}

/// Enrich the background shell call a completion row reports on (matched by shell id).
fn apply_completions(entries: &mut [Entry], completions: &[Completion]) {
    for done in completions {
        let calls = entries.iter_mut().filter_map(|e| match &mut e.kind {
            EntryKind::Message(m) => Some(m.parts.iter_mut()),
            _ => None,
        });
        let call = calls.flatten().find_map(|p| match p {
            Part::ToolCall(c)
                if c.output
                    .as_ref()
                    .and_then(|o| o.shell.as_ref())
                    .and_then(|s| s.background_id.as_deref())
                    == Some(done.shell_id.as_str()) =>
            {
                Some(c)
            }
            _ => None,
        });
        let Some(call) = call else { continue };
        let started = call.started_at;
        let Some(output) = call.output.as_mut() else {
            continue;
        };
        output.truncated |= done.truncated;
        let Some(shell) = output.shell.as_mut() else {
            continue;
        };
        shell.exit_code = done.exit.or(done.notice.exit);
        shell.stdout = done.stdout.clone().or(shell.stdout.take());
        shell.interrupted = done.state == "cancelled";
        shell.timed_out = done.timed_out || done.notice.timed_out;
        shell.signal = done.signal.clone().or_else(|| done.notice.signal.clone());
        shell.duration_ms = span_ms(started, done.ts);
        call.status = if shell.interrupted {
            ToolStatus::Interrupted
        } else if done.state == "error" || shell.failed() {
            ToolStatus::Error
        } else {
            ToolStatus::Ok
        };
        if !done.state.is_empty() {
            call.status_detail = Some(done.state.clone());
        }
    }
}

/// A human-run `!command` row with its full result.
fn v2_shell(row: V2Row) -> Entry {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let (started, ended) = take_span(&mut data, &["created"], &["completed"]);
    let ts = started.or(created);
    let shell_id = take_str(&mut data, "shellID").unwrap_or_else(|| id.clone());
    let command = take_str(&mut data, "command").unwrap_or_default();
    let status = take_str(&mut data, "status").unwrap_or_default();
    let exit_code = take_i64(&mut data, "exit").and_then(|e| i32::try_from(e).ok());
    let mut out = obj(data.remove("output"));
    let stdout = take_str(&mut out, "output");
    let truncated = take_bool(&mut out, "truncated");
    let background = take_nested(&mut data, "metadata", "background", Value::as_bool) == Some(true);
    let (timed_out, interrupted) = (status == "timeout", status == "killed");
    let finished = matches!(status.as_str(), "exited" | "timeout" | "killed");
    let result = ShellResult {
        exit_code,
        stdout: stdout.clone(),
        duration_ms: span_ms(started, ended),
        interrupted,
        timed_out,
        ..Default::default()
    };
    let call_status = match status.as_str() {
        _ if interrupted => ToolStatus::Interrupted,
        _ if result.failed() => ToolStatus::Error,
        "exited" => ToolStatus::Ok,
        // A user shell can still run while later rows are appended.
        _ => ToolStatus::Pending,
    };
    let output = ToolOutput {
        text: stdout.unwrap_or_default(),
        // Upstream runs a user shell as a background shell: until it finishes, say so.
        shell: Some(ShellResult {
            background_id: (!finished).then(|| shell_id.clone()),
            ..result
        }),
        truncated,
        data: extra_of(out),
        ..Default::default()
    };
    let input = Value::String(command.clone());
    let tool = Tool::Shell(Shell {
        command,
        background,
        ..Default::default()
    });
    let call = ToolCall {
        status: call_status,
        status_detail: Some(status).filter(|s| !s.is_empty()),
        output: Some(output),
        started_at: started,
        ended_at: ended,
        ..ToolCall::new(shell_id, "shell".into(), input, tool)
    };
    let msg = Message::new(Role::User, Origin::Command, vec![Part::ToolCall(call)]);
    entry(Some(id), ts, None, data, EntryKind::Message(msg))
}

fn v2_assistant(row: V2Row, turn_id: Option<String>) -> Vec<Entry> {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let done = time_of(&data, "completed");
    let ts = take_time(&mut data).or(created);
    let model = take_nested_str(&mut data, "model", "id");
    let agent = take_str(&mut data, "agent");
    let usage = usage(data.remove("tokens"), take_f64(&mut data, "cost"));
    let stop_reason = take_str(&mut data, "finish");
    let blocks = take_objects(&mut data, "content");
    let (error, attempt) = split_retry(&mut data);

    let phase = blocks
        .iter()
        .rev()
        .filter(|b| is_part(b, "text"))
        .find_map(|b| phase_of(b.get("state")));
    // Leftover keys of text and reasoning blocks, by position in the message.
    let mut extras = Map::new();
    let mut parts = Vec::new();
    for block in blocks {
        let (part, leftover) = v2_block(block);
        note_leftover(&mut extras, parts.len(), leftover);
        parts.push(part);
    }
    attach_part_extras(&mut data, extras);
    let msg = Message {
        model,
        agent,
        phase,
        usage,
        stop_reason,
        ..Message::new(Role::Assistant, Origin::Model, parts)
    };
    let mut entries = vec![entry(
        Some(id),
        ts,
        turn_id.clone(),
        data,
        EntryKind::Message(msg),
    )];
    entries.extend(error.map(|e| failure_entry(v2_error(e), attempt, done.or(ts), turn_id)));
    entries
}

/// The row's error and its retry attempt. `retry.error` repeats `error`, so it is dropped
/// when equal and kept under `retry` when it differs; `retry.at` stays in `data`.
fn split_retry(data: &mut Map) -> (Option<Value>, Option<u32>) {
    let mut retry = obj(data.remove("retry"));
    let attempt = take_u64(&mut retry, "attempt").and_then(|a| u32::try_from(a).ok());
    let retry_error = retry.remove("error");
    let error = match (data.remove("error"), retry_error) {
        (Some(e), Some(r)) if e != r => {
            retry.insert("error".into(), r);
            Some(e)
        }
        (e, r) => e.or(r),
    };
    if !retry.is_empty() {
        data.insert("retry".into(), Value::Object(retry));
    }
    (error, attempt)
}

fn v2_block(mut b: Map) -> (Part, Option<Value>) {
    let kind = take_str(&mut b, "type").unwrap_or_default();
    match kind.as_str() {
        "text" => {
            let text = take_str(&mut b, "text").unwrap_or_default();
            (Part::Text { text }, part_leftover(b))
        }
        "reasoning" => {
            let text = take_str(&mut b, "text").filter(|t| !t.is_empty());
            let encrypted = b.get("state").is_some_and(has_opaque);
            (Part::Reasoning { text, encrypted }, part_leftover(b))
        }
        "tool" => {
            let (call, leftover) = v2_call(b);
            (Part::ToolCall(call), leftover)
        }
        _ => (other_part(kind, b), None),
    }
}

fn v2_compaction(row: V2Row) -> Vec<Entry> {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let (started, _) = take_span(&mut data, &["created"], &["completed"]);
    let ts = started.or(created);
    let compaction = Compaction {
        summary: take_str(&mut data, "summary"),
        trigger: take_str(&mut data, "reason"),
        usage: usage(data.remove("tokens"), take_f64(&mut data, "cost")),
        ..Default::default()
    };
    let error = data.remove("error");
    let mut entries = vec![entry(
        Some(id),
        ts,
        None,
        data,
        EntryKind::Compaction(compaction),
    )];
    entries.extend(error.map(|e| failure_entry(v2_error(e), None, ts, None)));
    entries
}

fn v2_idle(row: V2Row) -> Entry {
    let V2Row {
        id,
        created,
        mut data,
        ..
    } = row;
    let ts = take_time(&mut data).or(created);
    // `succeeded` is opencode's word for the plan's `completed`.
    let status = take_str(&mut data, "outcome").map(|o| match o.as_str() {
        "succeeded" => "completed".to_string(),
        _ => o,
    });
    let mut error = obj(data.remove("error"));
    let message = take_str(&mut error, "message");
    if !error.is_empty() {
        data.insert("error".into(), Value::Object(error));
    }
    entry(
        Some(id),
        ts,
        None,
        data,
        EntryKind::Turn(Turn {
            status,
            duration_ms: None,
            error: message,
        }),
    )
}

/// `agent-switched`, `model-switched`, `location-switched`.
fn v2_switch(row: V2Row) -> Entry {
    let V2Row {
        id,
        created,
        ty,
        mut data,
    } = row;
    let ts = take_time(&mut data).or(created);
    let settings = match ty.as_str() {
        "agent-switched" => Settings {
            agent: take_str(&mut data, "agent"),
            ..Default::default()
        },
        "model-switched" => Settings {
            model: take_nested_str(&mut data, "model", "id"),
            ..Default::default()
        },
        _ => Settings {
            cwd: take_nested_str(&mut data, "location", "directory")
                .or_else(|| take_str(&mut data, "location")),
            ..Default::default()
        },
    };
    entry(Some(id), ts, None, data, EntryKind::Settings(settings))
}
