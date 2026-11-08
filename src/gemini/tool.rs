//! Tool calls: the typed view of the input, the result of the call, its status and files.

use super::Obj;
use super::part::{P, array, media, norm, response_for, rest};
use super::shell::{ShellParse, parse_shell};
use crate::util::{as_extra, bool_field, diff_stat, str_field, stripped, u64_field};
use crate::{FileChange, FileOp, Media, Shell, ShellResult, TextEdit, TodoItem, Tool, ToolStatus};
use serde_json::Value;

/// Record keys of a `ToolCallRecord` that typed fields took; the rest goes to `ToolOutput.data`.
const MAPPED_CALL_KEYS: &[&str] = &[
    "id",
    "name",
    "args",
    "result",
    "status",
    "timestamp",
    "agentId",
];
/// Tools that run a subagent, named after the agent (older sessions have no `invoke_agent`).
const AGENT_TOOLS: &[&str] = &[
    "invoke_agent",
    "codebase_investigator",
    "cli_help",
    "generalist",
];

/// Typed view of a call; a call missing a required field stays `Other`.
pub(super) fn map_tool(name: &str, args: &Value, has_agent_id: bool) -> Tool {
    typed(name, args, has_agent_id).unwrap_or(Tool::Other)
}

fn typed(name: &str, args: &Value, has_agent_id: bool) -> Option<Tool> {
    let text = |keys: &[&str]| str_field(args, keys);
    let path = || text(&["file_path", "absolute_path", "path"]);
    let dir = || text(&["dir_path", "directory", "path"]);
    Some(match name {
        "run_shell_command" => Tool::Shell(Shell {
            command: text(&["command"])?,
            argv: Vec::new(),
            cwd: text(&["dir_path", "directory"]),
            timeout_ms: None,
            background: bool_field(args, &["is_background"]),
            description: text(&["description"]),
        }),
        "read_file" => {
            // `offset`/`limit` (v0.15 to v0.30, `offset` 0 based) or `start_line`/`end_line`
            // (later, 1 based): the end line becomes a line count. The raw input keeps the truth.
            let start = u64_field(args, &["start_line", "offset"]);
            let limit = u64_field(args, &["limit"]).or_else(|| {
                u64_field(args, &["end_line"])
                    .map(|end| end.saturating_sub(start.unwrap_or(1)).saturating_add(1))
            });
            Tool::Read {
                path: path()?,
                offset: start,
                limit,
            }
        }
        "write_file" => Tool::Write {
            path: path()?,
            content: text(&["content"]),
        },
        "replace" => {
            let replace_all = bool_field(args, &["allow_multiple"])
                || u64_field(args, &["expected_replacements"]).is_some_and(|n| n > 1);
            let edit = TextEdit {
                old: text(&["old_string"])?,
                new: text(&["new_string"])?,
                replace_all,
            };
            Tool::Edit {
                path: path()?,
                edits: vec![edit],
            }
        }
        "glob" => Tool::Glob {
            pattern: text(&["pattern"])?,
            path: dir(),
        },
        "grep_search" | "search_file_content" => Tool::Search {
            pattern: text(&["pattern"])?,
            path: dir(),
            glob: text(&["include_pattern", "include"]),
        },
        "list_directory" => Tool::List { path: dir() },
        "web_fetch" => {
            let prompt = text(&["prompt"]);
            Tool::WebFetch {
                url: text(&["url"]).or_else(|| prompt.as_deref().and_then(first_url)),
                prompt,
            }
        }
        "google_web_search" => Tool::WebSearch {
            query: text(&["query"]),
        },
        "write_todos" | "write_todos_list" => Tool::Todo {
            items: todos(args.get("todos")?)?,
        },
        "ask_user" => Tool::Question {
            questions: Vec::new(),
            declined: false,
        },
        "enter_plan_mode" | "exit_plan_mode" => Tool::Plan,
        "activate_skill" => Tool::Skill {
            name: text(&["name"]),
        },
        n if has_agent_id || AGENT_TOOLS.contains(&n) => Tool::Task {
            description: text(&["description"]),
            prompt: text(&["prompt", "query", "objective", "task", "request"]),
            agent_type: if n == "invoke_agent" {
                text(&["agent_name"])
            } else {
                Some(n.to_string())
            },
            name: None,
        },
        n => {
            // `mcp_<server>_<tool>`: server names may hold `_`, so the split at the first one is a guess.
            let (server, tool) = n.strip_prefix("mcp_")?.split_once('_')?;
            if server.is_empty() || tool.is_empty() {
                return None;
            }
            Tool::Mcp {
                server: server.to_string(),
                tool: tool.to_string(),
            }
        }
    })
}

fn todos(list: &Value) -> Option<Vec<TodoItem>> {
    list.as_array()?
        .iter()
        .map(|t| {
            Some(TodoItem {
                content: str_field(t, &["description", "content"])?,
                status: str_field(t, &["status"]),
                priority: None,
            })
        })
        .collect()
}

/// The first URL of a prompt; `web_fetch` carries its URLs inside the prompt text.
fn first_url(text: &str) -> Option<String> {
    let start = text
        .find("http://")
        .into_iter()
        .chain(text.find("https://"))
        .min()?;
    let end = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']');
    let url = text[start..].split(end).next()?;
    Some(url.trim_end_matches(['.', ',', ';', ':']).to_string())
}

// ---------------------------------------------------------------------------------------------
// Results

/// A `functionResponse` read into what the model saw.
#[derive(Default)]
pub(super) struct Resp {
    pub output: Option<String>,
    pub error: Option<String>,
    pub attachments: Vec<Media>,
}

impl Resp {
    /// Output first; the error text only when it is all there is. A cancelled call keeps its
    /// reason in `status_detail`, so the error is not repeated next to partial output.
    pub(super) fn text(&self, cancelled: bool) -> String {
        match (&self.output, &self.error) {
            (Some(o), Some(e)) if !cancelled => format!("{o}\n{e}"),
            (Some(o), _) => o.clone(),
            (None, Some(e)) => e.clone(),
            (None, None) => String::new(),
        }
    }

    /// Add binary parts the echo turn carried, unless the result already has them.
    pub(super) fn add_media(&mut self, more: &[Media]) {
        for m in more {
            if !self
                .attachments
                .iter()
                .any(|a| a.mime == m.mime && a.bytes == m.bytes)
            {
                self.attachments.push(m.clone());
            }
        }
    }
}

fn append(target: &mut Option<String>, text: &str) {
    match target {
        Some(t) => {
            t.push('\n');
            t.push_str(text);
        }
        None => *target = Some(text.to_string()),
    }
}

/// `result` is normally `[{functionResponse: {id, name, response: {output | error}}}]` plus sibling
/// binary parts; plain strings and single parts also occur.
pub(super) fn read_resp(result: &Value, call_id: &str) -> Resp {
    let parts = norm(Some(result));
    let chosen = response_for(&parts, call_id);
    let mut resp = Resp::default();
    for part in &parts {
        match part.function_response() {
            Some(fr) if chosen.is_some_and(|c| std::ptr::eq(c, fr)) => read_response(fr, &mut resp),
            Some(_) => {}
            None => match (media(*part), part.text()) {
                (Some(m), _) => resp.attachments.push(m),
                (None, Some(text)) => append(&mut resp.output, text),
                (None, None) => {}
            },
        }
    }
    resp
}

fn read_response(fr: &Value, resp: &mut Resp) {
    resp.attachments.extend(
        array(fr.get("parts"))
            .iter()
            .filter_map(|p| media(P::Obj(p))),
    );
    let Some(response) = fr.get("response") else {
        return;
    };
    let string = |key: &str| match response.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    };
    let (output, error) = (string("output"), string("error"));
    if output.is_none() && error.is_none() && response.as_object().is_some_and(|o| !o.is_empty()) {
        append(&mut resp.output, &stripped(response.clone()).to_string());
    }
    if let Some(output) = output {
        append(&mut resp.output, &output);
    }
    resp.error = error;
}

/// The shell view of a result: parsed text, the directory the old text names, extra values for `data`.
pub(super) fn shell_output(
    tool: &mut Tool,
    resp: &Resp,
    raw_status: Option<&str>,
) -> Option<(ShellResult, Obj)> {
    let Tool::Shell(shell) = tool else {
        return None;
    };
    let ok = matches!(raw_status, Some("success" | "completed"));
    let ShellParse { result, cwd, data } = parse_shell(resp.output.as_deref(), ok);
    if shell.cwd.is_none() {
        shell.cwd = cwd;
    }
    let interrupted = result.interrupted || is_cancelled(raw_status);
    Some((
        ShellResult {
            interrupted,
            ..result
        },
        data,
    ))
}

// ---------------------------------------------------------------------------------------------
// Status

pub(super) fn is_cancelled(status: Option<&str>) -> bool {
    matches!(status, Some("cancelled" | "canceled"))
}

/// `[Operation Cancelled] Reason: User denied execution.` to the reason alone (the abort path
/// of the tool executor writes the marker without `Reason:`).
fn cancel_reason(error: &str) -> &str {
    let error = error.trim();
    match error.strip_prefix("[Operation Cancelled]") {
        Some(rest) => {
            let rest = rest.trim_start();
            rest.strip_prefix("Reason:").unwrap_or(rest).trim()
        }
        None => error,
    }
}

/// The scheduler's refusals: `User denied execution.` (cancelled) and the policy engine's
/// `Tool execution denied by policy.` (recorded as an error).
fn is_denial(raw: Option<&str>, reason: &str) -> bool {
    match raw {
        Some("error") => reason.starts_with("Tool execution denied by policy"),
        _ => reason.to_ascii_lowercase().contains("denied"),
    }
}

/// Status convention: a shell that exited non-zero, was killed by a signal or timed out is an error
/// although Gemini records `success`.
pub(super) fn call_status(
    raw: Option<&str>,
    resp: Option<&Resp>,
    shell: Option<&ShellResult>,
) -> (ToolStatus, Option<String>) {
    let failed = shell.is_some_and(ShellResult::failed);
    let reason = resp
        .and_then(|r| r.error.as_deref())
        .map(cancel_reason)
        .unwrap_or_default();
    let denied = matches!(raw, Some("cancelled" | "canceled" | "error")) && is_denial(raw, reason);
    let status = match raw {
        _ if denied => ToolStatus::Rejected,
        Some("success" | "completed") if failed => ToolStatus::Error,
        Some("success" | "completed") => ToolStatus::Ok,
        Some("error" | "failed") => ToolStatus::Error,
        Some("cancelled" | "canceled") => ToolStatus::Interrupted,
        _ if resp.is_some_and(|r| r.error.is_some()) || failed => ToolStatus::Error,
        _ if resp.is_some() => ToolStatus::Ok,
        _ => ToolStatus::Pending,
    };
    let detail = match raw {
        Some(s @ ("cancelled" | "canceled" | "error")) if denied || is_cancelled(Some(s)) => {
            Some(if reason.is_empty() {
                s.to_string()
            } else {
                format!("{s}: {reason}")
            })
        }
        other => other.map(str::to_string),
    };
    (status, detail)
}

// ---------------------------------------------------------------------------------------------
// Output fields

/// The record minus what typed fields took, plus values parsed from shell text. When the diff
/// became a `FileChange`, it and the path leave `resultDisplay`; a write's `newContent` equal to
/// the written content leaves it too.
pub(super) fn call_data(
    rec: Option<&Value>,
    extras: Obj,
    input: &Value,
    diff_taken: bool,
) -> Option<Value> {
    let mut data = (rec.and_then(Value::as_object))
        .map(|r| rest(r, |k| MAPPED_CALL_KEYS.contains(&k)))
        .unwrap_or_default();
    if let Some(display) = data.get_mut("resultDisplay").and_then(Value::as_object_mut) {
        if diff_taken {
            display.retain(|k, _| !["fileDiff", "filePath", "isNewFile"].contains(&k.as_str()));
        }
        let written = input.get("content").and_then(Value::as_str);
        if written.is_some() && display.get("newContent").and_then(Value::as_str) == written {
            display.remove("newContent");
        }
    }
    data.extend(extras);
    as_extra(data)
}

/// `resultDisplay.fileDiff` of `replace` and `write_file`. Cancelled calls carry the diff of the
/// confirmation prompt too, so the caller only asks for completed calls.
pub(super) fn file_change(display: &Value, args: &Value) -> Option<FileChange> {
    let diff = display.get("fileDiff")?.as_str()?;
    let path = str_field(display, &["filePath", "fileName"])
        .or_else(|| str_field(args, &["file_path", "absolute_path", "path"]))?;
    let op = if bool_field(display, &["isNewFile"]) {
        FileOp::Add
    } else {
        FileOp::Update
    };
    let (adds, dels) = diff_stat(diff);
    Some(FileChange {
        diff: Some(diff.to_string()),
        additions: Some(adds),
        deletions: Some(dels),
        ..FileChange::new(path, op)
    })
}
