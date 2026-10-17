//! Tool parts of both generations: a normalized raw tool, the typed view, status, shell result
//! and file changes.

use super::{Map, record::*};
use crate::model::*;
use crate::patch;
use crate::util::{bool_field, str_field, strip_blobs, u64_field};
use serde_json::Value;

/// A v1 tool part with its call and result. A call that never got a result (pending or
/// running) has no output; the keys of its state come back as the second value.
/// `cwd` is the working directory recorded on the v1 assistant message.
pub(super) fn v1_call(part: Map, cwd: Option<&str>) -> (ToolCall, Option<Value>) {
    build_call(raw_v1_tool(part), cwd)
}

/// A v2 tool content block with its call and result (see [`v1_call`] for the second value).
pub(super) fn v2_call(block: Map) -> (ToolCall, Option<Value>) {
    build_call(raw_v2_tool(block), None)
}

/// A tool part of either generation, with the keys the typed mapping needs pulled out.
struct RawTool {
    id: String,
    name: String,
    input: Value,
    /// `pending`, `running`, `streaming`, `completed` or `error`.
    status: String,
    /// The result as the model saw it.
    text: String,
    /// The command's output stream, without the harness notices.
    stream: Option<String>,
    /// What the harness appended to the output (exit code, timeout, signal, abort).
    notice: Notice,
    error: Option<ApiError>,
    metadata: Map,
    attachments: Vec<Media>,
    started: Option<i64>,
    ended: Option<i64>,
    /// Remaining keys of the part and its state, kept as `ToolOutput.data`.
    leftover: Map,
}

fn raw_v1_tool(mut p: Map) -> RawTool {
    let id = take_str(&mut p, "callID").unwrap_or_default();
    let name = take_str(&mut p, "tool").unwrap_or_default();
    let provider = p.remove("metadata");
    let mut state = obj(p.remove("state"));
    let status = take_str(&mut state, "status").unwrap_or_default();
    let mut input = state.remove("input").unwrap_or(Value::Null);
    // A call cut off while its arguments were streaming has `input: {}` and the partial JSON in
    // `raw`; keep it as the input, like a v2 `streaming` call does.
    if input.as_object().is_some_and(Map::is_empty) {
        if let Some(raw) = take_with(&mut state, "raw", |v| {
            v.as_str().filter(|r| !r.is_empty()).map(str::to_string)
        }) {
            input = Value::String(raw);
        }
    }
    let output = take_str(&mut state, "output");
    let metadata = obj(state.remove("metadata"));
    let attachments = take_objects(&mut state, "attachments")
        .into_iter()
        .map(media_from_file)
        .collect();
    let (started, ended) = take_span(&mut state, &["start"], &["end"]);
    let error = state.remove("error").map(|e| match e {
        Value::String(message) => ApiError {
            kind: None,
            message,
            status: None,
            rest: Map::new(),
        },
        other => v2_error(other),
    });
    // An interrupted command keeps its partial output only in the metadata preview.
    let source = output
        .clone()
        .or_else(|| metadata.get("output")?.as_str().map(str::to_string));
    let (stream, notice) = match source.as_deref().map(split_tagged) {
        Some((body, notice)) => (Some(clean_stream(&body)), notice),
        None => (None, Notice::default()),
    };
    let text = output
        .or_else(|| error.as_ref().map(|e| e.message.clone()))
        .unwrap_or_default();
    let mut leftover = state;
    leftover.extend(p);
    if let Some(provider) = provider {
        leftover.insert("provider_metadata".into(), provider);
    }
    RawTool {
        id,
        name,
        input,
        status,
        text,
        stream,
        notice,
        error,
        metadata,
        attachments,
        started,
        ended,
        leftover,
    }
}

fn raw_v2_tool(mut b: Map) -> RawTool {
    let id = take_str(&mut b, "id").unwrap_or_default();
    let name = take_str(&mut b, "name").unwrap_or_default();
    let mut state = obj(b.remove("state"));
    let status = take_str(&mut state, "status").unwrap_or_default();
    let input = state.remove("input").unwrap_or(Value::Null);
    let metadata = obj(state.remove("metadata"));
    let error = state.remove("error").map(v2_error);
    let (started, ended) = take_span(&mut b, &["ran", "created"], &["completed"]);
    // `created` is when the model began streaming the call; `ran` and `completed` bound the run.
    take_nested(&mut b, "time", "created", Value::as_i64);

    let (mut texts, mut attachments, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
    for item in take_objects(&mut state, "content") {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                texts.extend(item.get("text").and_then(Value::as_str).map(str::to_string))
            }
            Some("file") => attachments.push(media_from_file(item)),
            _ => unknown.push(Value::Object(item)),
        }
    }
    let mut shown = texts.clone();
    shown.extend(error.as_ref().map(|e| e.message.clone()));
    // The shell tool's content is `[output, notice]`: the notice is its own last part.
    let notice = texts.last().and_then(|t| notice_line(t));
    if notice.is_some() {
        texts.pop();
    }
    let stream = (!shown.is_empty() || notice.is_some()).then(|| clean_stream(&texts.join("\n")));

    let mut leftover = state;
    leftover.extend(b);
    if !unknown.is_empty() {
        leftover.insert("content".into(), Value::Array(unknown));
    }
    RawTool {
        id,
        name,
        input,
        status,
        text: shown.join("\n"),
        stream,
        notice: notice.unwrap_or_default(),
        error,
        metadata,
        attachments,
        started,
        ended,
        leftover,
    }
}

/// What the harness printed after a command's output, from the shell tool and its notices
/// (`Exited with code N`, `Timed out before completion`, `Killed by <signal>`, the v1
/// `<shell_metadata>` blocks).
#[derive(Default)]
pub(super) struct Notice {
    pub(super) exit: Option<i32>,
    pub(super) timed_out: bool,
    pub(super) aborted: bool,
    pub(super) signal: Option<String>,
}

/// A v2 notice line. `Exited with code unknown` is a notice without a code.
fn notice_line(line: &str) -> Option<Notice> {
    let line = line.trim();
    let mut notice = Notice::default();
    if line == "Timed out before completion" {
        notice.timed_out = true;
    } else if let Some(signal) = line.strip_prefix("Killed by ") {
        if signal.is_empty()
            || !signal
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        notice.signal = Some(signal.to_string());
    } else {
        let code = line.strip_prefix("Exited with code ")?;
        if code != "unknown" {
            notice.exit = Some(code.parse().ok()?);
        }
    }
    Some(notice)
}

/// Split a trailing notice paragraph off a report whose parts were joined with a blank line
/// (a background job's completion body).
pub(super) fn split_report(text: &str) -> (String, Notice) {
    let trimmed = text.trim_end_matches('\n');
    let (head, last) = trimmed.rsplit_once("\n\n").unwrap_or(("", trimmed));
    match notice_line(last) {
        Some(notice) => (clean_stream(head), notice),
        None => (clean_stream(text), Notice::default()),
    }
}

/// Split a trailing v1 `<shell_metadata>` / `<bash_metadata>` / `<metadata>` block off the
/// output. Only a block that ends the output counts, so output that merely contains the tag stays.
fn split_tagged(stream: &str) -> (String, Notice) {
    let trimmed = stream.trim_end();
    for tag in ["shell_metadata", "bash_metadata", "metadata"] {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let Some(inner) = trimmed.strip_suffix(close.as_str()) else {
            continue;
        };
        let Some(at) = inner.rfind(open.as_str()) else {
            continue;
        };
        let body = &trimmed[..at];
        let body = body
            .strip_suffix("\n\n")
            .or_else(|| body.strip_suffix('\n'))
            .unwrap_or(body);
        let block = &inner[at + open.len()..];
        let notice = Notice {
            timed_out: block.contains("timeout"),
            aborted: block.contains("aborted"),
            ..Default::default()
        };
        return (body.to_string(), notice);
    }
    (stream.to_string(), Notice::default())
}

/// The output without the placeholder and timeout sentence the shell tool adds around it.
fn clean_stream(output: &str) -> String {
    let marker = "\n\nCommand exceeded timeout of ";
    let output = match output.rfind(marker) {
        Some(at) if output[at..].contains(" ms. Retry with a larger timeout") => &output[..at],
        _ => output,
    };
    if output.trim() == "(no output)" {
        String::new()
    } else {
        output.to_string()
    }
}

fn build_call(mut raw: RawTool, cwd: Option<&str>) -> (ToolCall, Option<Value>) {
    strip_blobs(&mut raw.input);
    let tool = classify(&raw.name, &raw.input, cwd);
    let finished = matches!(raw.status.as_str(), "completed" | "error");
    let interrupted = take_bool(&mut raw.metadata, "interrupted");
    let shell =
        (finished && matches!(tool, Tool::Shell(_))).then(|| shell_result(&mut raw, interrupted));
    let files = if raw.status == "completed" {
        file_changes(&mut raw.metadata, &tool, &raw.text)
    } else {
        Vec::new()
    };
    let status = tool_status(&raw, shell.as_ref(), interrupted);
    let running = finished && status == ToolStatus::Pending;
    let shell = shell.map(|s| ShellResult {
        duration_ms: s.duration_ms.filter(|_| !running),
        ..s
    });
    let child_session = matches!(tool, Tool::Task { .. })
        .then(|| {
            take_str(&mut raw.metadata, "sessionId")
                .or_else(|| take_str(&mut raw.metadata, "sessionID"))
                .or_else(|| str_field(&raw.input, &["sessionID", "task_id"]))
        })
        .flatten();
    let truncated = take_bool(&mut raw.metadata, "truncated");
    let full_output_path = take_str(&mut raw.metadata, "outputPath");

    let status_detail = raw
        .error
        .as_ref()
        .and_then(|e| e.kind.clone())
        .or_else(|| Some(raw.status.clone()).filter(|s| !s.is_empty()));
    let mut data = raw.leftover;
    if !raw.metadata.is_empty() {
        data.insert("metadata".into(), Value::Object(raw.metadata));
    }
    if let Some(e) = raw.error.filter(|e| !e.rest.is_empty()) {
        data.insert("error".into(), Value::Object(e.rest));
    }
    let data = extra_of(data);
    // A call without a result has no output (`Session::finish` marks it Interrupted once the
    // run moved on); whatever its state recorded goes back to the caller.
    let (output, unfinished) = if finished {
        let output = ToolOutput {
            text: raw.text,
            shell,
            files,
            attachments: raw.attachments,
            data,
            truncated,
            full_output_path,
        };
        (Some(output), None)
    } else {
        (None, data)
    };
    let call = ToolCall {
        id: raw.id,
        name: raw.name,
        input: raw.input,
        tool,
        status,
        status_detail,
        output,
        started_at: raw.started,
        ended_at: raw.ended.filter(|_| !running),
        child_session,
        children: Vec::new(),
    };
    (call, unfinished)
}

/// What the command did, from the metadata and the notices the harness appended.
fn shell_result(raw: &mut RawTool, interrupted: bool) -> ShellResult {
    let m = &mut raw.metadata;
    let exit_code = m
        .remove("exit")
        .and_then(|e| e.as_i64())
        .and_then(|e| i32::try_from(e).ok())
        .or(raw.notice.exit);
    let running = m.get("status").and_then(Value::as_str) == Some("running");
    let background_id = if running {
        take_str(m, "shellID")
    } else {
        None
    };
    // v1 `metadata.output` is a preview of the output, already the stream.
    m.remove("output");
    ShellResult {
        exit_code,
        stdout: raw.stream.clone(),
        stderr: None,
        duration_ms: span_ms(raw.started, raw.ended),
        interrupted: interrupted || raw.notice.aborted,
        timed_out: take_bool(m, "timeout") || raw.notice.timed_out,
        signal: take_str(m, "signal").or_else(|| raw.notice.signal.clone()),
        background_id,
    }
}

/// The phrases opencode's own UI treats as a denied tool call.
fn is_rejection(message: &str) -> bool {
    [
        "rejected permission",
        "specified a rule",
        "QuestionRejectedError",
        "user dismissed",
    ]
    .iter()
    .any(|phrase| message.contains(phrase))
}

fn tool_status(raw: &RawTool, shell: Option<&ShellResult>, interrupted: bool) -> ToolStatus {
    let shell_interrupted = shell.is_some_and(|s| s.interrupted);
    match raw.status.as_str() {
        "completed" if shell_interrupted => ToolStatus::Interrupted,
        "completed" if shell.is_some_and(ShellResult::failed) => ToolStatus::Error,
        // A background job: the call only started it, a later completion row ends it.
        "completed" if shell.is_some_and(|s| s.background_id.is_some()) => ToolStatus::Pending,
        "completed" => ToolStatus::Ok,
        "error" => {
            let kind = raw.error.as_ref().and_then(|e| e.kind.as_deref());
            let message = raw.error.as_ref().map_or("", |e| e.message.as_str());
            if interrupted || shell_interrupted || kind == Some("tool.interrupted") {
                ToolStatus::Interrupted
            } else if is_rejection(message) {
                ToolStatus::Rejected
            } else {
                ToolStatus::Error
            }
        }
        _ => ToolStatus::Pending,
    }
}

const PATH_KEYS: &[&str] = &["filePath", "path", "file_path"];

/// The typed view of a call. Both name generations are accepted (`bash`/`shell`,
/// `task`/`subagent`, `apply_patch`/`patch`, `filePath`/`path`). MCP and plugin tools stay
/// `Other` (their names are ambiguous), except the MCP resource tools.
fn classify(name: &str, input: &Value, cwd: Option<&str>) -> Tool {
    let s = |keys: &[&str]| str_field(input, keys);
    let typed = match name {
        "bash" | "shell" => s(&["command"]).map(|command| {
            Tool::Shell(Shell {
                command,
                argv: Vec::new(),
                cwd: s(&["workdir"]).or_else(|| cwd.map(str::to_string)),
                timeout_ms: u64_field(input, &["timeout"]),
                background: bool_field(input, &["background"]),
                description: s(&["description"]),
            })
        }),
        "read" => s(PATH_KEYS).map(|path| Tool::Read {
            path,
            offset: u64_field(input, &["offset"]),
            limit: u64_field(input, &["limit"]),
        }),
        "write" => s(PATH_KEYS).map(|path| Tool::Write {
            path,
            content: s(&["content"]),
        }),
        "edit" => s(PATH_KEYS).map(|path| {
            let (old, new) = (
                s(&["oldString", "old_string"]),
                s(&["newString", "new_string"]),
            );
            let edit = TextEdit {
                old: old.clone().unwrap_or_default(),
                new: new.clone().unwrap_or_default(),
                replace_all: bool_field(input, &["replaceAll", "replace_all"]),
            };
            Tool::Edit {
                path,
                edits: if old.is_some() || new.is_some() {
                    vec![edit]
                } else {
                    Vec::new()
                },
            }
        }),
        "apply_patch" | "patch" => {
            s(&["patchText", "patch_text", "patch"]).map(|patch| Tool::Patch { patch })
        }
        "grep" => s(&["pattern"]).map(|pattern| Tool::Search {
            pattern,
            path: s(&["path"]),
            glob: s(&["include", "glob"]),
        }),
        "glob" => s(&["pattern"]).map(|pattern| Tool::Glob {
            pattern,
            path: s(&["path"]),
        }),
        "list" => Some(Tool::List { path: s(&["path"]) }),
        "webfetch" => Some(Tool::WebFetch {
            url: s(&["url"]),
            prompt: None,
        }),
        "websearch" => Some(Tool::WebSearch {
            query: s(&["query"]),
        }),
        "todowrite" => todo_items(input).map(|items| Tool::Todo { items }),
        "task" | "subagent" => Some(Tool::Task {
            description: s(&["description"]),
            prompt: s(&["prompt"]),
            agent_type: s(&["subagent_type", "agent"]),
        }),
        "question" => Some(Tool::Question),
        "execute" => s(&["code"]).map(|source| Tool::Code { source }),
        "update_goal_plan" => Some(Tool::Todo {
            items: goal_phases(input),
        }),
        "plan_enter" | "plan_exit" => Some(Tool::Plan),
        "skill" => Some(Tool::Skill {
            name: s(&["name", "id"]),
        }),
        "list_mcp_resources" | "list_mcp_resource_templates" | "read_mcp_resource" => {
            Some(Tool::Mcp {
                server: s(&["server"]).unwrap_or_default(),
                tool: name.to_string(),
            })
        }
        _ => None,
    };
    typed.unwrap_or(Tool::Other)
}

/// The phases of a goal plan (`plan.phases[]`) as checklist items.
fn goal_phases(input: &Value) -> Vec<TodoItem> {
    let phases = input.pointer("/plan/phases").and_then(Value::as_array);
    let item = |p: &Value| {
        Some(TodoItem {
            content: str_field(p, &["objective", "id"])?,
            status: str_field(p, &["status"]),
            priority: None,
        })
    };
    phases.into_iter().flatten().filter_map(item).collect()
}

fn todo_items(input: &Value) -> Option<Vec<TodoItem>> {
    let todos = input.get("todos")?.as_array()?;
    Some(
        todos
            .iter()
            .filter_map(|t| {
                Some(TodoItem {
                    content: str_field(t, &["content"])?,
                    status: str_field(t, &["status"]),
                    priority: str_field(t, &["priority"]),
                })
            })
            .collect(),
    )
}

/// A per-file record of v1 `filediff`, v1 `apply_patch` `files[]` and v2 `files[]`.
fn file_change(v: &Value) -> Option<FileChange> {
    let op = match str_field(v, &["type", "status"]).as_deref() {
        Some("add" | "added") => FileOp::Add,
        Some("delete" | "deleted") => FileOp::Delete,
        Some("move" | "moved" | "renamed") => FileOp::Move,
        _ => FileOp::Update,
    };
    Some(FileChange {
        path: str_field(v, &["filePath", "file", "path"])?,
        op,
        move_to: str_field(v, &["movePath"]),
        diff: str_field(v, &["patch", "diff"]),
        additions: u64_field(v, &["additions"]),
        deletions: u64_field(v, &["deletions"]),
    })
}

/// Files a completed edit, write or patch call changed: from `metadata.files[]` /
/// `metadata.filediff`, else the patch input, else the written path.
fn file_changes(meta: &mut Map, tool: &Tool, result: &str) -> Vec<FileChange> {
    if !matches!(
        tool,
        Tool::Edit { .. } | Tool::Write { .. } | Tool::Patch { .. }
    ) {
        return Vec::new();
    }
    if let Some(Value::Array(files)) = meta.get("files") {
        let changes: Vec<_> = files.iter().filter_map(file_change).collect();
        if !changes.is_empty() || files.is_empty() {
            meta.remove("files");
            return changes;
        }
    }
    if let Some(change) = meta.get("filediff").and_then(file_change) {
        meta.remove("filediff");
        return vec![change];
    }
    match tool {
        Tool::Patch { patch: text } => patch::parse(text),
        Tool::Write { path, .. } => {
            // v1 says so in `metadata.exists`; v2 only in the result text.
            let exists = take_with(meta, "exists", Value::as_bool).or_else(|| {
                result
                    .starts_with("Created file successfully")
                    .then_some(false)
            });
            let path = take_str(meta, "filepath").unwrap_or_else(|| path.clone());
            let op = if exists == Some(false) {
                FileOp::Add
            } else {
                FileOp::Update
            };
            vec![FileChange::new(path, op)]
        }
        _ => Vec::new(),
    }
}
