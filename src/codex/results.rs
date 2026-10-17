//! What a tool output, a completion event or a tool item says about the call.

use super::js;
use super::messages::{content_part, texts};
use super::tools::{argv_shell, exit_of, same_command, tool_for, web_item};
use super::{PERSIST_CUT, TRUNCATED, Times};
use crate::util::{
    Obj, base64_len, bool_field, diff_stat, file_url_path, obj_str, str_field, stripped, take_str,
};
use crate::{
    FileChange, FileOp, Media, Part, ProcessAction, ShellResult, Tool, ToolCall, ToolOutput,
    ToolStatus, patch,
};
use serde_json::{Value, json};

/// What filling a call's output found out for the parser's indexes.
#[derive(Default)]
pub(super) struct Answer {
    pub(super) cell_done: bool,
    pub(super) cell_id: Option<String>,
    pub(super) running: Vec<String>,
    pub(super) exit: Option<i32>,
}

pub(super) fn output_mut(call: &mut ToolCall) -> &mut ToolOutput {
    call.output.get_or_insert_with(ToolOutput::default)
}

/// One event lands in `data` as is; more become an array in arrival order.
pub(super) fn push_data(out: &mut ToolOutput, v: Value) {
    out.data = Some(match out.data.take() {
        None => v,
        Some(Value::Array(mut all)) => {
            all.push(v);
            Value::Array(all)
        }
        Some(first) => Value::Array(vec![first, v]),
    });
}

pub(super) fn fill_output(
    call: &mut ToolCall,
    body: Value,
    ts: Option<i64>,
    cwd: Option<String>,
) -> Answer {
    let (items, attachments) = body_items(body);
    let text = join_items(&items);
    call.ended_at = ts.or(call.ended_at);
    let out = output_mut(call);
    out.attachments.extend(attachments);
    if out.text.is_empty() {
        out.text = text.clone();
    } else {
        out.text.push('\n');
        out.text.push_str(&text);
    }
    match &call.tool {
        Tool::Code { .. } => cell_answer(call, &items, cwd),
        Tool::Process { .. } if call.name == "wait" => cell_answer(call, &items, cwd),
        Tool::Shell(_) | Tool::Process { .. } => shell_answer(call, &text),
        Tool::Patch { patch } => {
            let files = patch::parse(patch);
            patch_answer(call, &text, files);
            Answer::default()
        }
        _ => {
            if call.status == ToolStatus::Pending {
                call.status = text_status(&text);
            }
            Answer::default()
        }
    }
}

/// Code-mode output: item 0 is `<status>\nWall time <s> seconds\nOutput:\n`, then one item per
/// `text()` call; an exec_command result that is still running has `session_id`, no `exit_code`.
/// A cell is not a shell, so its wall time, yielded cell id and still-running process ids go to
/// `data` (a later delivery overwrites them).
fn cell_answer(call: &mut ToolCall, items: &[String], cwd: Option<String>) -> Answer {
    let cell = read_cell(items);
    if let Some((status, detail)) = &cell.status {
        call.status = *status;
        call.status_detail = Some(detail.clone());
    } else if call.status == ToolStatus::Pending {
        // No header: the plain text of an aborted or declined cell.
        call.status = items.first().map_or(ToolStatus::Ok, |t| text_status(t));
    }
    unrecorded_commands(call, items, cell.status.is_some(), cwd);
    if cell.error {
        call.status = ToolStatus::Error;
    }
    let out = output_mut(call);
    out.truncated |= cell.truncated;
    let facts = [
        ("wall_time_ms", cell.wall_ms.map(Value::from)),
        ("cell_id", cell.cell_id.clone().map(Value::from)),
        (
            "running_session_ids",
            (!cell.running.is_empty()).then(|| json!(cell.running)),
        ),
    ];
    let facts: Obj = (facts.into_iter())
        .filter_map(|(k, v)| Some((k.to_owned(), v?)))
        .collect();
    if !facts.is_empty() {
        if let Value::Object(data) = out.data.get_or_insert_with(|| json!({})) {
            data.extend(facts);
        }
    }
    Answer {
        cell_done: cell.status.is_some(),
        cell_id: cell.cell_id,
        running: cell.running,
        exit: None,
    }
}

/// What a script printed for an `exec_command` or `write_stdin` call that finished:
/// `{"chunk_id", "wall_time_seconds", "exit_code", "output"}`. One that is still running has a
/// `session_id` and no `exit_code`.
struct Chunk {
    exit: i32,
    wall_ms: Option<u64>,
    output: Option<String>,
}

fn finished_chunk(printed: &str) -> Option<Chunk> {
    let json = (printed.find('{')).and_then(|i| serde_json::from_str::<Value>(&printed[i..]).ok());
    if let Some(chunk) = json {
        let exit = i32::try_from(chunk.get("exit_code")?.as_i64()?).ok()?;
        let wall = chunk.get("wall_time_seconds").and_then(Value::as_f64);
        return Some(Chunk {
            exit,
            wall_ms: wall.map(|s| (s * 1000.0).round() as u64),
            output: str_field(&chunk, &["output"]),
        });
    }
    // A cut chunk (`Warning: truncated output` and part of the JSON gone) still has its code:
    // a quote inside the output is escaped, so this key text only matches the key itself.
    let rest = &printed[printed.find("\"exit_code\":")? + "\"exit_code\":".len()..];
    let rest = rest.trim_start();
    let digits = rest.strip_prefix('-').unwrap_or(rest);
    let len = digits.bytes().take_while(u8::is_ascii_digit).count();
    let end = rest.len() - digits.len() + len;
    Some(Chunk {
        exit: rest[..end].parse().ok()?,
        wall_ms: None,
        output: None,
    })
}

/// `{"session_id", "output", ...}` printed for a command that is still running (no `exit_code`).
fn running_chunk(printed: &str) -> Option<(String, Option<String>)> {
    let chunk: Value = serde_json::from_str(&printed[printed.find('{')?..]).ok()?;
    chunk.get("exit_code").is_none().then_some(())?;
    Some((
        str_field(&chunk, &["session_id"])?,
        str_field(&chunk, &["output"]),
    ))
}

/// A command Codex refuses before it starts (sandbox policy) leaves no `CommandExecution` item,
/// so the cell's printed chunk is its only trace. Each such chunk becomes a child call, placed
/// after the recorded child of the call before it. The calls come from the script and pair with
/// the printed items by position, so a script with any other statement is left alone.
fn unrecorded_commands(call: &mut ToolCall, items: &[String], headed: bool, cwd: Option<String>) {
    let Tool::Code { source } = &call.tool else {
        return;
    };
    let Some(calls) = js::invocations(source).filter(|_| headed) else {
        return;
    };
    let mut printed = items.get(1..).unwrap_or_default();
    if let Some((last, rest)) = printed.split_last() {
        if last.starts_with("Script error:") {
            printed = rest;
        }
    }
    // A finished cell printed one item per call, a cut one fewer.
    if printed.len() > calls.len() || (printed.len() < calls.len() && call.status == ToolStatus::Ok)
    {
        return;
    }
    let mut taken = vec![false; call.children.len()];
    let (mut anchor, mut fresh) = (0, Vec::new());
    for (i, ((name, arg), text)) in calls.into_iter().zip(printed).enumerate() {
        if !matches!(name.as_str(), "exec_command" | "write_stdin") {
            continue;
        }
        let tool = tool_for(&name, &arg, cwd.clone());
        let chunk = finished_chunk(text);
        // A recorded item is this call's when it ran the same command and ended the same way
        // (the same command twice can have one call refused and one run).
        let recorded = (call.children.iter().enumerate()).position(|(j, c)| {
            let ended = chunk.as_ref().zip(exit_of(c));
            !taken[j] && same_command(&c.tool, &tool) && ended.is_none_or(|(k, e)| k.exit == e)
        });
        if let Some(j) = recorded {
            (taken[j], anchor) = (true, j + 1);
            continue;
        }
        let Some(chunk) = chunk else {
            // The process outlived the cell: a running child until its command item lands.
            if let (true, Some((session, output))) = (name == "exec_command", running_chunk(text)) {
                let shell = ShellResult {
                    stdout: output,
                    background_id: Some(session),
                    ..Default::default()
                };
                let output = ToolOutput {
                    shell: Some(shell),
                    ..Default::default()
                };
                fresh.push((
                    anchor,
                    ToolCall {
                        output: Some(output),
                        ..ToolCall::new(format!("{}#{i}", call.id), name, arg, tool)
                    },
                ));
            }
            continue;
        };
        let exit = Some(chunk.exit);
        let shell = ShellResult {
            exit_code: exit,
            stdout: chunk.output,
            // A poll's wall time is how long it waited, not how long the command ran.
            duration_ms: chunk.wall_ms.filter(|_| matches!(tool, Tool::Shell(_))),
            ..Default::default()
        };
        fresh.push((
            anchor,
            ToolCall {
                status: exec_status(None, exit),
                output: Some(ToolOutput {
                    shell: Some(shell),
                    ..Default::default()
                }),
                ..ToolCall::new(format!("{}#{i}", call.id), name, arg, tool)
            },
        ));
    }
    for (at, child) in fresh.into_iter().rev() {
        call.children.insert(at, child);
    }
}

#[derive(Default)]
pub(super) struct Cell {
    pub(super) status: Option<(ToolStatus, String)>,
    wall_ms: Option<u64>,
    cell_id: Option<String>,
    error: bool,
    truncated: bool,
    running: Vec<String>,
}

pub(super) fn read_cell(items: &[String]) -> Cell {
    let mut cell = Cell::default();
    let mut header = items.first().map(|h| h.lines()).into_iter().flatten();
    if let Some(line) = header.next() {
        let status = match line {
            "Script completed" => Some(ToolStatus::Ok),
            "Script failed" => Some(ToolStatus::Error),
            "Script terminated" => Some(ToolStatus::Interrupted),
            _ => line.strip_prefix("Script running with cell ID ").map(|id| {
                cell.cell_id = Some(id.trim().to_owned());
                ToolStatus::Pending
            }),
        };
        cell.status = status.map(|s| (s, line.to_owned()));
    }
    if cell.status.is_some() {
        cell.wall_ms = header.find_map(wall_time);
    }
    for item in items.iter().skip(usize::from(cell.status.is_some())) {
        cell.error |= item.starts_with("Script error:");
        cell.truncated |= item.starts_with(TRUNCATED);
        let json = item
            .find('{')
            .and_then(|i| serde_json::from_str::<Value>(&item[i..]).ok());
        if let Some(chunk) = json.filter(|j| j.get("exit_code").is_none()) {
            cell.running.extend(str_field(&chunk, &["session_id"]));
        }
    }
    cell
}

/// `Wall time 1.2 seconds` (code mode) or `Wall time: 0.1234 seconds` (direct tools).
fn wall_time(line: &str) -> Option<u64> {
    let rest = line
        .strip_prefix("Wall time")?
        .trim_start_matches(':')
        .trim_start();
    let secs: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some((secs * 1000.0).round() as u64)
}

/// Facts printed in a direct tool's output text.
#[derive(Default)]
struct Printed {
    exit: Option<i32>,
    wall_ms: Option<u64>,
    session: Option<String>,
    truncated: bool,
    body: Option<String>,
}

/// Parse `{"output", "metadata": {exit_code, duration_seconds}}` (old `shell`) or header lines
/// (`Exit code: N`, `Process exited with code N`, `Wall time: N seconds`, ...) up to `Output:`.
fn printed(text: &str) -> Printed {
    if let Some(p) = printed_json(text) {
        return p;
    }
    let mut p = Printed::default();
    let mut rest = text;
    while !rest.is_empty() {
        let (line, tail) = rest.split_once('\n').unwrap_or((rest, ""));
        if line.trim_end() == "Output:" {
            p.body = Some(tail.to_owned());
            break;
        }
        if !header_line(&mut p, line.trim_end()) {
            return Printed::default();
        }
        rest = tail;
    }
    p
}

fn printed_json(text: &str) -> Option<Printed> {
    if !text.trim_start().starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(text).ok()?;
    let meta = v.get("metadata")?;
    Some(Printed {
        exit: meta
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok()),
        wall_ms: meta
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .map(|s| (s * 1000.0).round() as u64),
        body: Some(str_field(&v, &["output"])?),
        ..Default::default()
    })
}

fn header_line(p: &mut Printed, line: &str) -> bool {
    let code = |s: &str| s.trim().parse::<i32>().ok();
    if let Some(c) = line
        .strip_prefix("Exit code:")
        .or_else(|| line.strip_prefix("Process exited with code"))
    {
        p.exit = code(c);
    } else if let Some(id) = line.strip_prefix("Process running with session ID") {
        p.session = Some(id.trim().to_owned());
    } else if let Some(ms) = wall_time(line) {
        p.wall_ms = Some(ms);
    } else if line.starts_with(TRUNCATED) || line.starts_with("Total output lines:") {
        p.truncated = true;
    } else {
        return line.is_empty()
            || line.starts_with("Chunk ID:")
            || line.starts_with("Original token count:");
    }
    true
}

fn shell_answer(call: &mut ToolCall, text: &str) -> Answer {
    let p = printed(text);
    if call.status == ToolStatus::Pending {
        // A process that outlived its call keeps running: its exit ends the call (`process_exit`,
        // or the command item).
        let live = p.session.is_some() && matches!(call.tool, Tool::Shell(_));
        call.status = match p.exit {
            Some(0) => ToolStatus::Ok,
            Some(_) => ToolStatus::Error,
            None if live => ToolStatus::Pending,
            None => text_status(text),
        };
        if call.status == ToolStatus::Pending {
            call.ended_at = None;
        }
    }
    let plain_ok = call.status == ToolStatus::Ok;
    let out = output_mut(call);
    out.truncated |= p.truncated;
    let shell = out.shell.get_or_insert_with(ShellResult::default);
    shell.exit_code = shell.exit_code.or(p.exit);
    // The printed wall time is the command's run time only when it exited within the call.
    shell.duration_ms = shell.duration_ms.or(p.exit.and(p.wall_ms));
    shell.background_id = shell.background_id.take().or_else(|| p.session.clone());
    if shell.stdout.is_none() {
        shell.stdout = p.body.or_else(|| plain_ok.then(|| text.to_owned()));
    }
    let running = p.session.filter(|_| p.exit.is_none());
    Answer {
        running: running.into_iter().collect(),
        exit: p.exit,
        ..Default::default()
    }
}

/// A direct `apply_patch` output. It applied only when it says so: an exit code 0 header or the
/// `Success. Updated the following files:` report. Failures are plain text (`apply_patch
/// verification failed: ...`, `failed to parse apply_patch: ...`), so anything else is an error
/// and changes nothing.
fn patch_answer(call: &mut ToolCall, text: &str, files: Vec<FileChange>) {
    if call.status == ToolStatus::Pending {
        let reported = text.contains("Success. Updated the following files:");
        let applied = printed(text).exit.map_or(reported, |c| c == 0);
        call.status = match text_status(text) {
            ToolStatus::Ok if !applied => {
                call.status_detail = text.lines().next().map(str::to_owned);
                ToolStatus::Error
            }
            status => status,
        };
    }
    let ok = call.status == ToolStatus::Ok;
    let out = output_mut(call);
    if out.files.is_empty() && ok {
        out.files = files;
    }
}

/// Status of a tool output text with no structured status.
pub(super) fn text_status(text: &str) -> ToolStatus {
    let t = text.trim();
    if t == "aborted" || t.starts_with("aborted by user") {
        ToolStatus::Interrupted
    } else if t.contains("rejected by user") {
        ToolStatus::Rejected
    } else {
        ToolStatus::Ok
    }
}

/// `exec_command_end`, or a `CommandExecution` item whose id is a direct call's id.
pub(super) fn shell_end(call: &mut ToolCall, ev: &mut Obj) {
    let aggregated = take_str(ev, "aggregated_output");
    let has = |k: &str| {
        ev.get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    // Builds from 0.114 on blank stdout and stderr before persisting; older ones kept them apart.
    let split = has("stdout") || has("stderr");
    let exit = ev
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|c| i32::try_from(c).ok());
    let detail = obj_str(ev, "status");
    call.status = exec_status(detail.as_deref(), exit);
    call.status_detail = detail;
    let out = output_mut(call);
    out.truncated |= aggregated
        .as_deref()
        .is_some_and(|a| a.contains(PERSIST_CUT));
    let shell = out.shell.get_or_insert_with(ShellResult::default);
    shell.exit_code = exit.or(shell.exit_code);
    shell.duration_ms = ev
        .get("duration")
        .and_then(duration_ms)
        .or(shell.duration_ms);
    if split {
        shell.stdout = take_str(ev, "stdout");
        shell.stderr = Some(take_str(ev, "stderr").unwrap_or_default());
    } else if aggregated.is_some() {
        shell.stdout = aggregated;
    }
}

pub(super) fn patch_end(call: &mut ToolCall, ev: &mut Obj) {
    let detail = obj_str(ev, "status");
    let failed = ev.get("success") == Some(&Value::Bool(false));
    call.status = if failed {
        ToolStatus::Error
    } else {
        exec_status(detail.as_deref(), None)
    };
    call.status_detail = detail;
    applied_files(call, ev);
}

/// Changes become `files` once applied; those of a failed or declined patch stay in the event.
fn applied_files(call: &mut ToolCall, ev: &mut Obj) {
    if call.status != ToolStatus::Ok {
        return;
    }
    if let Some(changes) = ev.remove("changes") {
        output_mut(call).files = file_changes(&changes);
    }
}

pub(super) fn mcp_error(result: &Value) -> bool {
    bool_field(result, &["is_error", "isError"])
}

/// A product status word; `None` for words of success and unknown words.
pub(super) fn status_word(word: &str) -> Option<ToolStatus> {
    match word {
        "failed" | "errored" | "error" => Some(ToolStatus::Error),
        "declined" => Some(ToolStatus::Rejected),
        "interrupted" | "cancelled" | "aborted" => Some(ToolStatus::Interrupted),
        "inProgress" | "in_progress" | "pending" | "running" | "searching" => {
            Some(ToolStatus::Pending)
        }
        _ => None,
    }
}

fn exec_status(status: Option<&str>, exit: Option<i32>) -> ToolStatus {
    let failed = exit.is_some_and(|c| c != 0);
    status.and_then(status_word).unwrap_or(if failed {
        ToolStatus::Error
    } else {
        ToolStatus::Ok
    })
}

/// Status of a tool item with a `status` word and maybe `success` / `error` / `result.is_error`.
pub(super) fn item_status(item: &Obj) -> ToolStatus {
    let failed = item.get("success") == Some(&Value::Bool(false))
        || item.get("error").is_some_and(|e| !e.is_null())
        || item.get("result").is_some_and(mcp_error);
    let fallback = if failed {
        ToolStatus::Error
    } else {
        ToolStatus::Ok
    };
    obj_str(item, "status")
        .as_deref()
        .and_then(status_word)
        .unwrap_or(fallback)
}

/// The shell clock. Three candidates, measured on 741 real commands: the event `duration`
/// equals the item envelope's `completed_at_ms - started_at_ms` and is the whole process lifetime
/// (a 7.8 s type check, a 39.7 s diff polled over three chunks), while a chunk's
/// `wall_time_seconds` is only how long the model waited for that chunk (capped by
/// `yield_time_ms`). So `duration_ms` comes from `duration`; its `{secs, nanos}` stays in `data`,
/// the envelope times are `started_at`/`ended_at`, chunk wall times stay in the cell text.
fn duration_ms(d: &Value) -> Option<u64> {
    let secs = d.get("secs")?.as_u64()?;
    let nanos = d.get("nanos").and_then(Value::as_u64).unwrap_or(0);
    secs.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}

/// A tool item between an `exec` cell's call and its output. The model saw its result only
/// through the cell's output, so `text` stays empty; the event is `data`.
pub(super) fn child_call(ty: &str, mut item: Obj, times: Times, cwd: Option<String>) -> ToolCall {
    match ty {
        "CommandExecution" => return command_execution(item, times, cwd, false),
        "FileChange" => {
            let mut call = tool_call(ty, &item, Tool::Other, times);
            call.status = exec_status(call.status_detail.as_deref(), None);
            applied_files(&mut call, &mut item);
            push_data(output_mut(&mut call), Value::Object(item));
            return call;
        }
        "ImageGeneration" => {
            if let Some(Value::String(b64)) = item.get("result") {
                let bytes = json!({ "bytes": base64_len(b64) });
                item.insert("result".into(), bytes);
            }
        }
        _ => {}
    }
    let tool = match ty {
        "Extension" | "WebSearch" => web_item(&item),
        "McpToolCall" => match (obj_str(&item, "server"), obj_str(&item, "tool")) {
            (Some(server), Some(tool)) => Tool::Mcp { server, tool },
            _ => Tool::Other,
        },
        "ImageView" => obj_str(&item, "path").map_or(Tool::Other, |path| Tool::Read {
            path,
            offset: None,
            limit: None,
        }),
        _ => Tool::Other,
    };
    let mut call = tool_call(ty, &item, tool, times);
    call.status = item_status(&item);
    if ty == "McpToolCall" {
        mcp_fields(&mut call, &mut item);
    }
    push_data(output_mut(&mut call), stripped(Value::Object(item)));
    call
}

/// An MCP item's `arguments` are the call's input and the text of its `result` the output text;
/// both leave the data they were read from.
fn mcp_fields(call: &mut ToolCall, item: &mut Obj) {
    call.input = item.remove("arguments").map_or(Value::Null, stripped);
    // A call that never ran (approval policy) has an `error` and no result.
    if let Some(message) = item.get("error").and_then(|e| str_field(e, &["message"])) {
        output_mut(call).text = message;
    }
    let Some(Value::Object(result)) = item.get_mut("result") else {
        return;
    };
    if let Some(content) = result.remove("content") {
        let (items, attachments) = body_items(content);
        let out = output_mut(call);
        out.text = join_items(&items);
        out.attachments = attachments;
    }
}

fn tool_call(name: &str, item: &Obj, tool: Tool, times: Times) -> ToolCall {
    let id = obj_str(item, "id").unwrap_or_default();
    ToolCall {
        status: ToolStatus::Ok,
        status_detail: obj_str(item, "status"),
        output: Some(ToolOutput::default()),
        started_at: times.started,
        ended_at: times.ended,
        ..ToolCall::new(id, name.into(), Value::Null, tool)
    }
}

/// A `CommandExecution` item: argv, file URL cwd, one interleaved output stream, exit code.
pub(super) fn command_execution(
    mut item: Obj,
    times: Times,
    cwd: Option<String>,
    model_saw: bool,
) -> ToolCall {
    let argv: Vec<String> = item.get("command").map(texts).unwrap_or_default();
    let cwd = obj_str(&item, "cwd").map(|c| file_url_path(&c)).or(cwd);
    let tool = if obj_str(&item, "source").as_deref() == Some("unified_exec_interaction") {
        let input = obj_str(&item, "interaction_input");
        Tool::Process {
            id: obj_str(&item, "process_id"),
            action: ProcessAction::Write,
            input,
        }
    } else {
        Tool::Shell(argv_shell(argv, cwd, None))
    };
    let exit = item
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|c| i32::try_from(c).ok());
    let stdout = take_str(&mut item, "aggregated_output");
    let mut call = tool_call("CommandExecution", &item, tool, times);
    call.status = exec_status(call.status_detail.as_deref(), exit);
    let out = output_mut(&mut call);
    out.truncated = stdout.as_deref().is_some_and(|s| s.contains(PERSIST_CUT));
    if model_saw {
        out.text = stdout.clone().unwrap_or_default();
    }
    out.shell = Some(ShellResult {
        exit_code: exit,
        stdout,
        duration_ms: item.get("duration").and_then(duration_ms),
        ..Default::default()
    });
    out.data = Some(Value::Object(item));
    call
}

fn file_changes(changes: &Value) -> Vec<FileChange> {
    let Some(map) = changes.as_object() else {
        return Vec::new();
    };
    map.iter()
        .map(|(path, change)| file_change(path, change))
        .collect()
}

/// `{"type": "update", "unified_diff", "move_path"}` (or the older `{"update": {...}}`).
fn file_change(path: &str, change: &Value) -> FileChange {
    let (op, body) = match change.get("type").and_then(Value::as_str) {
        Some(op) => (op.to_owned(), change),
        None => match change.as_object().and_then(|o| o.iter().next()) {
            Some((op, body)) => (op.clone(), body),
            None => (String::new(), change),
        },
    };
    let lines = |prefix: char| {
        let content = str_field(body, &["content"]).unwrap_or_default();
        let diff: Vec<String> = content.lines().map(|l| format!("{prefix}{l}")).collect();
        diff.join("\n")
    };
    let (op, diff, move_to) = match op.as_str() {
        "add" => (FileOp::Add, lines('+'), None),
        "delete" => (FileOp::Delete, lines('-'), None),
        _ => {
            let move_to = str_field(body, &["move_path"]);
            let op = if move_to.is_some() {
                FileOp::Move
            } else {
                FileOp::Update
            };
            (
                op,
                str_field(body, &["unified_diff"]).unwrap_or_default(),
                move_to,
            )
        }
    };
    let (adds, dels) = diff_stat(&diff);
    let path = path.to_owned();
    let (diff, additions, deletions) = (Some(diff), Some(adds), Some(dels));
    FileChange {
        path,
        op,
        move_to,
        diff,
        additions,
        deletions,
    }
}

/// Text items and media of a tool output (a string or an array of content items).
pub(super) fn body_items(body: Value) -> (Vec<String>, Vec<Media>) {
    match body {
        Value::String(s) => (vec![s], Vec::new()),
        Value::Array(items) => {
            let (mut texts, mut media) = (Vec::new(), Vec::new());
            for item in items {
                match content_part(item) {
                    Part::Text { text } => texts.push(text),
                    Part::Attachment(m) => media.push(m),
                    _ => {}
                }
            }
            (texts, media)
        }
        Value::Object(o) => o
            .get("content")
            .cloned()
            .map(body_items)
            .unwrap_or_default(),
        _ => (Vec::new(), Vec::new()),
    }
}

pub(super) fn join_items(items: &[String]) -> String {
    let mut out = String::new();
    for item in items {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printed_headers() {
        let p = printed(
            "Chunk ID: a1\nWall time: 0.5012 seconds\nProcess running with session ID 64391\nOriginal token count: 0\nOutput:\n",
        );
        assert_eq!(
            (p.exit, p.session.as_deref(), p.body.as_deref()),
            (None, Some("64391"), Some(""))
        );
        let p =
            printed("Exit code: 1\nWall time: 0.2 seconds\nTotal output lines: 3\nOutput:\nboom");
        assert_eq!(
            (p.exit, p.wall_ms, p.truncated, p.body.as_deref()),
            (Some(1), Some(200), true, Some("boom"))
        );
        let p = printed(
            "{\"output\":\"x\\n\",\"metadata\":{\"exit_code\":2,\"duration_seconds\":1.5}}",
        );
        assert_eq!(
            (p.exit, p.wall_ms, p.body.as_deref()),
            (Some(2), Some(1500), Some("x\n"))
        );
        let p = printed("Plan updated");
        assert_eq!((p.exit, p.body), (None, None));
    }

    #[test]
    fn cell_headers() {
        let items = |h: &str| vec![format!("{h}\nWall time 2.5 seconds\nOutput:\n")];
        assert_eq!(
            read_cell(&items("Script failed")).status.map(|s| s.0),
            Some(ToolStatus::Error)
        );
        let c = read_cell(&items("Script running with cell ID 7"));
        assert_eq!(
            (c.status.map(|s| s.0), c.cell_id.as_deref(), c.wall_ms),
            (Some(ToolStatus::Pending), Some("7"), Some(2500))
        );
        let mut error = items("Script completed");
        error.push("Script error:\nboom".into());
        assert!(read_cell(&error).error);
        assert!(read_cell(&["notify text".to_owned()]).status.is_none());
    }
}
