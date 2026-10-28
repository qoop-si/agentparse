//! Claude tool calls: the typed view of a `tool_use` input, and what a `tool_result` plus the
//! line's `toolUseResult` say about the outcome (status, shell detail, file changes, `data`).

use super::{INTERRUPTED, media, tag_body};
use crate::util::{
    Obj, bool_field, diff_stat, mcp_tool, str_field, stripped, take_bool, take_str, u64_field,
};
use crate::{
    FileChange, FileOp, Media, ProcessAction, Shell, ShellResult, TextEdit, TodoItem, Tool,
    ToolCall, ToolOutput, ToolStatus,
};
use serde_json::{Value, json};
use std::fmt::Write as _;

const PATH_KEYS: &[&str] = &[
    "file_path",
    "filePath",
    "path",
    "absolute_path",
    "notebook_path",
];

/// A `tool_use` block as a call, plus its unmapped keys. `caller: {type: direct}`, the value of
/// every call in the measured corpus, is dropped as noise.
pub(super) fn tool_call(mut b: Obj, cwd: Option<&str>, ts: Option<i64>) -> (ToolCall, Obj) {
    let name = take_str(&mut b, "name").unwrap_or_default();
    let input = stripped(b.remove("input").unwrap_or(Value::Null));
    if b.get("caller") == Some(&json!({"type": "direct"})) {
        b.remove("caller");
    }
    let id = take_str(&mut b, "id").unwrap_or_default();
    let tool = tool_view(&name, &input, cwd);
    let call = ToolCall {
        started_at: ts,
        ..ToolCall::new(id, name, input, tool)
    };
    (call, b)
}

/// Typed view of a call's input. An input the CLI could not parse (`__unparsedToolInput`) is
/// read from its raw text when that is JSON.
fn tool_view(name: &str, input: &Value, cwd: Option<&str>) -> Tool {
    let reparsed;
    let input = match input
        .pointer("/__unparsedToolInput/raw")
        .and_then(Value::as_str)
    {
        Some(raw) => {
            reparsed = serde_json::from_str(raw).unwrap_or(Value::Null);
            &reparsed
        }
        None => input,
    };
    typed(name, input, cwd).unwrap_or(Tool::Other)
}

fn typed(name: &str, i: &Value, cwd: Option<&str>) -> Option<Tool> {
    let s = |keys: &[&str]| str_field(i, keys);
    Some(match name {
        "Bash" | "Monitor" => Tool::Shell(Shell {
            command: s(&["command"])?,
            argv: Vec::new(),
            cwd: cwd.map(String::from),
            timeout_ms: u64_field(i, &["timeout", "timeout_ms"]),
            background: name == "Monitor" || bool_field(i, &["run_in_background"]),
            description: s(&["description"]),
        }),
        "BashOutput" | "TaskOutput" => Tool::Process {
            id: s(&["task_id", "bash_id", "shell_id"]),
            action: ProcessAction::Read,
            input: None,
        },
        "KillShell" | "KillBash" | "TaskStop" => Tool::Process {
            id: s(&["task_id", "shell_id", "bash_id"]),
            action: ProcessAction::Kill,
            input: None,
        },
        "Read" => Tool::Read {
            path: s(PATH_KEYS)?,
            offset: u64_field(i, &["offset"]),
            limit: u64_field(i, &["limit"]),
        },
        "Write" => Tool::Write {
            path: s(PATH_KEYS)?,
            content: s(&["content"]),
        },
        "Edit" => Tool::Edit {
            path: s(PATH_KEYS)?,
            edits: vec![text_edit(i)?],
        },
        "MultiEdit" => Tool::Edit {
            path: s(PATH_KEYS)?,
            edits: i
                .get("edits")?
                .as_array()?
                .iter()
                .map(text_edit)
                .collect::<Option<_>>()?,
        },
        "Grep" => Tool::Search {
            pattern: s(&["pattern"])?,
            path: s(PATH_KEYS),
            glob: s(&["glob"]),
        },
        "Glob" => Tool::Glob {
            pattern: s(&["pattern"])?,
            path: s(PATH_KEYS),
        },
        "LS" => Tool::List { path: s(PATH_KEYS) },
        "WebFetch" => Tool::WebFetch {
            url: s(&["url"]),
            prompt: s(&["prompt"]),
        },
        "WebSearch" => Tool::WebSearch {
            query: s(&["query"]),
        },
        // A task list entry by entry: the call names one task, not the whole list.
        "TaskCreate" => Tool::Todo {
            items: vec![TodoItem {
                content: s(&["subject"])?,
                status: Some("pending".into()),
                priority: None,
            }],
        },
        "TaskUpdate" => Tool::Todo {
            items: vec![TodoItem {
                content: s(&["subject"]).or_else(|| s(&["taskId"]).map(|id| format!("#{id}")))?,
                status: s(&["status"]),
                priority: None,
            }],
        },
        "TodoWrite" => Tool::Todo {
            items: i
                .get("todos")?
                .as_array()?
                .iter()
                .map(todo_item)
                .collect::<Option<_>>()?,
        },
        "Task" | "Agent" => Tool::Task {
            description: s(&["description"]),
            prompt: s(&["prompt"]),
            agent_type: s(&["subagent_type"]),
            name: s(&["name"]),
        },
        // The script is long; the row's text is the launch result's `summary` (`launched`).
        "Workflow" => Tool::Task {
            description: None,
            prompt: None,
            agent_type: Some("workflow".into()),
            name: None,
        },
        "SendMessage" => Tool::Task {
            description: s(&["summary"]),
            prompt: i
                .get("message")
                .map(|m| m.as_str().map_or_else(|| m.to_string(), String::from)),
            agent_type: None,
            name: s(&["to"]),
        },
        "AskUserQuestion" => Tool::Question,
        "EnterPlanMode" | "ExitPlanMode" => Tool::Plan,
        "Skill" => Tool::Skill {
            name: s(&["skill", "name"]),
        },
        "SlashCommand" => Tool::Skill {
            name: s(&["command"]),
        },
        _ => mcp_tool(name)?,
    })
}

fn text_edit(i: &Value) -> Option<TextEdit> {
    Some(TextEdit {
        old: str_field(i, &["old_string", "oldString", "old_str"])?,
        new: str_field(i, &["new_string", "newString", "new_str"])?,
        replace_all: bool_field(i, &["replace_all", "replaceAll"]),
    })
}

fn todo_item(i: &Value) -> Option<TodoItem> {
    Some(TodoItem {
        content: str_field(i, &["content", "subject"])?,
        status: str_field(i, &["status"]),
        priority: str_field(i, &["priority"]),
    })
}

pub(super) struct Outcome {
    pub(super) output: ToolOutput,
    pub(super) status: ToolStatus,
    pub(super) detail: Option<String>,
    pub(super) child: Option<String>,
}

/// A `tool_result` block plus the line's `toolUseResult` and `toolDenialKind`.
pub(super) fn outcome(
    name: &str,
    input: &Value,
    mut block: Obj,
    tur: Option<Value>,
    denial: Option<String>,
) -> Outcome {
    let is_error = take_bool(&mut block, "is_error");
    let content = block.remove("content").unwrap_or(Value::Null);
    let (text, attachments) = result_content(&content);
    let mut out = ToolOutput {
        text,
        attachments,
        ..Default::default()
    };
    let status = denial_status(denial.as_deref()).unwrap_or_else(|| {
        if is_error {
            error_status(&out.text)
        } else {
            ToolStatus::Ok
        }
    });
    let mut child = None;
    let mut async_launch = false;
    match tur {
        Some(Value::Object(map)) => {
            async_launch = matches!(name, "Agent" | "Task" | "Workflow")
                && map.get("status").and_then(Value::as_str) == Some("async_launched");
            child = structured(name, input, map, &mut out);
        }
        // A string result is the error text again, with an `Error: ` prefix.
        Some(Value::String(s)) => {
            if s != out.text && s.strip_prefix("Error: ") != Some(out.text.as_str()) {
                out.data = Some(Value::String(s));
            }
        }
        Some(v @ Value::Array(_)) if v != content => out.data = Some(stripped(v)),
        _ => {}
    }
    let status = if matches!(name, "Bash" | "Monitor") {
        shell_detail(&mut out, status)
    } else {
        status
    };
    persisted(&mut out);
    // A launch is not an outcome: the task reports back through `apply_notification`.
    let background = out
        .shell
        .as_ref()
        .is_some_and(|s| s.background_id.is_some());
    let status = match status {
        ToolStatus::Ok if async_launch || background => ToolStatus::Pending,
        status => status,
    };
    Outcome {
        output: out,
        status,
        detail: denial,
        child,
    }
}

fn result_content(content: &Value) -> (String, Vec<Media>) {
    match content {
        Value::String(s) => (s.clone(), Vec::new()),
        Value::Array(items) => {
            let mut texts = Vec::new();
            let mut media_items = Vec::new();
            for item in items {
                match (item.get("type").and_then(Value::as_str), item) {
                    (Some("text"), _) => {
                        texts.push(item.get("text").and_then(Value::as_str).unwrap_or_default())
                    }
                    (Some("image" | "document"), Value::Object(b)) => media_items.push(media(b)),
                    // `tool_reference` blocks repeat the `matches` of the structured result.
                    _ => {}
                }
            }
            (texts.join("\n"), media_items)
        }
        _ => (String::new(), Vec::new()),
    }
}

fn denial_status(kind: Option<&str>) -> Option<ToolStatus> {
    match kind? {
        "interrupted" | "cancelled" => Some(ToolStatus::Interrupted),
        k if k == "user-rejected" || k == "permission-rule" || k.starts_with("automode") => {
            Some(ToolStatus::Rejected)
        }
        _ => None,
    }
}

/// Error results without a denial kind (older CLIs) still say what happened in their text.
fn error_status(text: &str) -> ToolStatus {
    if text.starts_with("The user doesn't want to proceed")
        || text.contains("Permission to use ") && text.contains(" has been denied")
    {
        ToolStatus::Rejected
    } else if text.starts_with(INTERRUPTED)
        || text.starts_with("[Tool call interrupted")
        || text.contains("Cancelled: parallel tool call")
    {
        ToolStatus::Interrupted
    } else {
        ToolStatus::Error
    }
}

/// The structured `toolUseResult` object: typed fields out, the rest stays as `data`.
fn structured(name: &str, input: &Value, mut map: Obj, out: &mut ToolOutput) -> Option<String> {
    let child = take_str(&mut map, "agentId").or_else(|| take_str(&mut map, "agent_id"));
    match name {
        "Bash" | "Monitor" => shell_fields(&mut map, out),
        "Edit" | "MultiEdit" | "Write" => out.files.extend(edit_change(&mut map, input)),
        "Read" => read_fields(&mut map, &out.text),
        "TaskOutput" => task_output(&mut map, out),
        _ => {}
    }
    if let Some(path) = take_str(&mut map, "persistedOutputPath") {
        out.full_output_path = Some(path);
        out.truncated = true;
    }
    drop_input_echo(&mut map, input);
    if !map.is_empty() {
        out.data = Some(stripped(Value::Object(map)));
    }
    child
}

/// Result keys that repeat the call's input (`oldString` for `old_string`, `content`, `prompt`)
/// are dropped; the recorded input keeps them.
fn drop_input_echo(map: &mut Obj, input: &Value) {
    let Some(input) = input.as_object() else {
        return;
    };
    let norm = |k: &str| k.replace('_', "").to_ascii_lowercase();
    map.retain(|k, v| !input.iter().any(|(ik, iv)| iv == v && norm(ik) == norm(k)));
}

fn shell_fields(map: &mut Obj, out: &mut ToolOutput) {
    let shell = out.shell.get_or_insert_default();
    shell.stdout = take_str(map, "stdout");
    // Claude folds stderr into stdout for most commands, so an empty stderr says nothing.
    shell.stderr = take_str(map, "stderr").filter(|s| !s.is_empty());
    shell.interrupted = take_bool(map, "interrupted");
    shell.background_id = take_str(map, "backgroundTaskId").or_else(|| take_str(map, "taskId"));
    if let Some(Value::Object(diff)) = map.get_mut("bashEditDiff") {
        if let Some(Value::Array(files)) = diff.remove("files") {
            out.files
                .extend(files.into_iter().filter_map(bash_edit_change));
        }
        if diff.is_empty() {
            map.remove("bashEditDiff");
        }
    }
}

fn edit_change(map: &mut Obj, input: &Value) -> Option<FileChange> {
    let path = take_str(map, "filePath").or_else(|| str_field(input, PATH_KEYS))?;
    let op = match map.get("type").and_then(Value::as_str) {
        Some("create") => FileOp::Add,
        _ => FileOp::Update,
    };
    if matches!(
        map.get("type").and_then(Value::as_str),
        Some("create" | "update")
    ) {
        map.remove("type");
    }
    let hunks = map.remove("structuredPatch");
    Some(file_change(path, op, hunks.as_ref()))
}

fn bash_edit_change(file: Value) -> Option<FileChange> {
    let Value::Object(mut f) = file else {
        return None;
    };
    let path = take_str(&mut f, "filePath")?;
    let op = if f.get("created").and_then(Value::as_bool) == Some(true) {
        FileOp::Add
    } else if f.get("deleted").and_then(Value::as_bool) == Some(true) {
        FileOp::Delete
    } else {
        FileOp::Update
    };
    Some(file_change(path, op, f.get("hunks")))
}

/// `structuredPatch` hunks rendered as a unified diff body.
fn file_change(path: String, op: FileOp, hunks: Option<&Value>) -> FileChange {
    let change = FileChange::new(path, op);
    let Some(hunks) = hunks.and_then(Value::as_array).filter(|h| !h.is_empty()) else {
        return change;
    };
    let mut diff = String::new();
    for h in hunks {
        let n = |k: &str| h.get(k).and_then(Value::as_u64).unwrap_or(0);
        let _ = writeln!(
            diff,
            "@@ -{},{} +{},{} @@",
            n("oldStart"),
            n("oldLines"),
            n("newStart"),
            n("newLines")
        );
        for line in h
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            diff.push_str(line);
            diff.push('\n');
        }
    }
    let (adds, dels) = diff_stat(&diff);
    FileChange {
        diff: Some(diff),
        additions: Some(adds),
        deletions: Some(dels),
        ..change
    }
}

/// A text read is stored twice: numbered in the result text, raw in `file.content`. Keep one.
fn read_fields(map: &mut Obj, text: &str) {
    let Some(Value::Object(file)) = map.get_mut("file") else {
        return;
    };
    let same = file
        .get("content")
        .and_then(Value::as_str)
        .is_some_and(|c| c == text || c == strip_line_numbers(text));
    if same {
        file.remove("content");
    }
}

/// A polled background shell (`task_type` `local_bash`) reports its output and exit code.
fn task_output(map: &mut Obj, out: &mut ToolOutput) {
    let Some(Value::Object(task)) = map.get_mut("task") else {
        return;
    };
    if task.get("task_type").and_then(Value::as_str) != Some("local_bash") {
        return;
    }
    let exit_code = task
        .get("exitCode")
        .and_then(Value::as_i64)
        .and_then(|c| i32::try_from(c).ok());
    if exit_code.is_some() {
        task.remove("exitCode");
    }
    out.shell = Some(ShellResult {
        exit_code,
        stdout: take_str(task, "output"),
        ..Default::default()
    });
}

/// `     1\tline` (or `1→line`) to `line`.
pub(super) fn strip_line_numbers(text: &str) -> String {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| {
            let rest = line.trim_start_matches(' ');
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            match digits {
                0 => line,
                _ => rest[digits..]
                    .strip_prefix('\t')
                    .or_else(|| rest[digits..].strip_prefix('→'))
                    .unwrap_or(line),
            }
        })
        .collect();
    lines.join("\n")
}

/// Exit code and timeout from the error text; a non-zero exit or a timeout is an Error.
fn shell_detail(out: &mut ToolOutput, status: ToolStatus) -> ToolStatus {
    let mut shell = out.shell.take().unwrap_or_default();
    if status == ToolStatus::Error {
        if let Some((code, rest)) = exit_code(&out.text) {
            shell.exit_code = Some(code);
            // `Exit code 143\nCommand timed out after 2m 0s\n<partial output>`
            let output = match rest.strip_prefix("Command timed out after") {
                Some(after) => {
                    shell.timed_out = true;
                    after.split_once('\n').map_or("", |(_, output)| output)
                }
                None => rest,
            };
            shell.stdout.get_or_insert_with(|| output.to_string());
        }
    }
    shell.interrupted |= status == ToolStatus::Interrupted;
    // The CLI reports a non-zero exit as an error unless it reinterpreted the code
    // (`returnCodeInterpretation`, code 1). A plain success object therefore means exit 0;
    // a backgrounded or interrupted command has no code yet.
    let reinterpreted = out
        .data
        .as_ref()
        .is_some_and(|d| d.get("returnCodeInterpretation").is_some());
    if status == ToolStatus::Ok
        && shell.stdout.is_some()
        && shell.exit_code.is_none()
        && shell.background_id.is_none()
        && !shell.interrupted
        && !reinterpreted
    {
        shell.exit_code = Some(0);
    }
    let failed = shell.failed();
    if shell != ShellResult::default() {
        out.shell = Some(shell);
    }
    if status == ToolStatus::Ok && failed {
        ToolStatus::Error
    } else {
        status
    }
}

/// `Exit code 3\n<output>` to `(3, "<output>")`.
fn exit_code(text: &str) -> Option<(i32, &str)> {
    let rest = text.strip_prefix("Exit code ")?;
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    let code = rest[..end].parse().ok()?;
    let tail = &rest[end..];
    match tail.strip_prefix('\n') {
        Some(output) => Some((code, output)),
        None => tail.is_empty().then_some((code, tail)),
    }
}

/// `<persisted-output>` means the model saw a preview; the full output was saved to a file.
fn persisted(out: &mut ToolOutput) {
    if !out.text.starts_with("<persisted-output>") {
        return;
    }
    out.truncated = true;
    if out.full_output_path.is_none() {
        out.full_output_path = out
            .text
            .split("Full output saved to: ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .map(|p| p.trim().to_string());
    }
}

/// The id a later notification, stop or status line names this call by.
pub(super) fn task_id(call: &ToolCall) -> Option<&str> {
    let out = call.output.as_ref()?;
    let shell = out.shell.as_ref().and_then(|s| s.background_id.as_deref());
    let data = || out.data.as_ref()?.get("taskId")?.as_str();
    shell.or(call.child_session.as_deref()).or_else(data)
}

/// A launched workflow's one line text is the `summary` of its launch result.
pub(super) fn launch_summary(call: &mut ToolCall) {
    let summary = call
        .output
        .as_ref()
        .and_then(|o| o.data.as_ref()?.get("summary")?.as_str());
    if let (Tool::Task { description, .. }, Some(summary)) = (&mut call.tool, summary) {
        description.get_or_insert_with(|| summary.to_string());
    }
}

/// `<task-notification>` fields, or a bare `<status>` word from another record that ends the task:
/// completed, failed or killed, plus `done`, `stopped`, `ended` (no report arrived) and `expired`
/// (a Monitor ran out its timeout). `running` and an interim report change nothing.
pub(super) fn apply_notification(call: &mut ToolCall, note: &str, ts: Option<i64>) {
    let task = tag_body(note, "task-id").map(String::from);
    match &call.tool {
        Tool::Task { .. } if call.child_session.is_none() && call.name != "Workflow" => {
            call.child_session = task.clone()
        }
        _ => {}
    }
    let status = match tag_body(note, "status") {
        Some(status) => status,
        // A Monitor that delivered no event says so in an event line without a status.
        None if tag_body(note, "event").is_some_and(|e| e.starts_with("[Monitor expired")) => {
            "expired"
        }
        None => return,
    };
    let interim = tag_body(note, "note").is_some_and(|n| n.contains("background work of its own"));
    if interim
        || !matches!(
            status,
            "completed" | "done" | "failed" | "killed" | "stopped" | "ended" | "expired"
        )
    {
        return;
    }
    let ended = matches!(status, "killed" | "stopped" | "ended");
    let (code, soft) = summary_exit(tag_body(note, "summary").unwrap_or_default());
    let out = call.output.get_or_insert_default();
    if matches!(call.tool, Tool::Shell(_)) {
        if out.full_output_path.is_none() {
            out.full_output_path = tag_body(note, "output-file").map(String::from);
        }
        let shell = out.shell.get_or_insert_default();
        shell.exit_code = code.or(shell.exit_code);
        shell.interrupted |= ended;
        if shell.background_id.is_none() {
            shell.background_id = task;
        }
    }
    call.status = match status {
        "failed" => ToolStatus::Error,
        _ if ended => ToolStatus::Interrupted,
        // `(exit code 1: No matches found)` is a soft failure, like `returnCodeInterpretation`.
        _ if code.is_some_and(|c| c != 0) && !soft => ToolStatus::Error,
        _ => ToolStatus::Ok,
    };
    call.status_detail = Some(status.to_string());
    call.ended_at = ts;
}

/// `... (exit code 1)` or `... (exit code 1: No matches found)` to (code, has interpretation).
fn summary_exit(summary: &str) -> (Option<i32>, bool) {
    let Some(rest) = summary
        .rsplit("(exit code ")
        .next()
        .filter(|_| summary.contains("(exit code "))
    else {
        return (None, false);
    };
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    (rest[..end].parse().ok(), rest[end..].starts_with(':'))
}
