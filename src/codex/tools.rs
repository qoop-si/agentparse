//! Tool calls: typed inputs, outputs, completion events and code-mode cell children.

use super::json::{calls_extra, merge_extra, part_mut, take, take_passthrough, with_meta};
use super::messages::texts;
use super::results::{
    body_items, child_call, command_execution, fill_output, item_status, join_items, mcp_error,
    output_mut, patch_end, push_data, read_cell, shell_end, status_word, text_status,
};
use super::{COLLAB_TOOLS, Parser, Ref, Times};
use crate::util::{
    Obj, base64_len, bool_field, mcp_tool, obj_str, str_field, strip_blobs, stripped, take_str,
    u64_field,
};
use crate::{
    EntryKind, Media, Message, Origin, Part, ProcessAction, Role, Shell, ShellResult, Source,
    TodoItem, Tool, ToolCall, ToolOutput, ToolResult, ToolStatus,
};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};

/// How much of a child rollout's end is read to find how its last turn ended.
const CHILD_TAIL: u64 = 256 * 1024;

impl Parser {
    // ---- tool calls ----

    pub(super) fn call(&mut self, ty: &str, mut p: Obj, meta: Option<Value>) {
        let (turn, _) = take_passthrough(&mut p);
        let name = take_str(&mut p, "name").unwrap_or_else(|| ty.to_owned());
        let item = take_str(&mut p, "id");
        let id = take_str(&mut p, "call_id")
            .or_else(|| item.clone())
            .unwrap_or_else(|| format!("line:{}", self.rec.line));
        let item = item.filter(|item| *item != id);
        let mut input = match ty {
            "function_call" => arguments(p.remove("arguments")),
            "custom_tool_call" => take(&mut p, "input"),
            "tool_search_call" => take(&mut p, "arguments"),
            _ => take(&mut p, "action"),
        };
        hide_blobs(&mut input);
        let tool = tool_for(&name, &input, self.cwd());
        let mut call = ToolCall {
            started_at: self.rec.ts,
            ..ToolCall::new(id.clone(), name, input, tool)
        };
        if matches!(ty, "web_search_call" | "image_generation_call") {
            self_contained(&mut call, &mut p);
        }
        let cell = ty == "custom_tool_call" && call.name == "exec";
        let waited = match &call.tool {
            Tool::Process {
                id: Some(cell_id), ..
            } if call.name == "wait" => (self.yielded.iter())
                .find(|(cell, _)| cell == cell_id)
                .map(|(_, exec)| exec.clone()),
            _ => None,
        };
        let r = self.add_part(
            Part::ToolCall(call),
            item,
            turn,
            None,
            calls_extra(&id, "call", with_meta(p, meta)),
        );
        self.calls.insert(id.clone(), r);
        if cell {
            self.open_cells.push(id.clone());
            self.cells.push(id.clone());
        }
        if let Some(exec) = waited {
            self.open_cells.push(exec.clone());
            self.waits.insert(id, exec);
        }
    }

    fn call_mut(&mut self, r: Ref) -> Option<&mut ToolCall> {
        match part_mut(&mut self.s.entries, r)? {
            Part::ToolCall(call) => Some(call),
            _ => None,
        }
    }

    pub(super) fn output(&mut self, mut p: Obj, meta: Option<Value>) {
        self.close_group();
        let call_id = take_str(&mut p, "call_id").unwrap_or_default();
        take_passthrough(&mut p);
        let body = take(&mut p, "output");
        let extra = with_meta(p, meta);
        let Some(r) = self.calls.get(&call_id).copied() else {
            return self.orphan_output(call_id, body, extra);
        };
        let (ts, cwd) = (self.rec.ts, self.cwd());
        let Some(call) = self.call_mut(r) else { return };
        let answer = fill_output(call, body, ts, cwd);
        let polled = match &call.tool {
            Tool::Process {
                id: Some(process), ..
            } if call.name == "write_stdin" => Some(process.clone()),
            _ => None,
        };
        let finished =
            (call.status != ToolStatus::Pending).then(|| (call.status, call.status_detail.clone()));
        merge_extra(
            &mut self.s.entries[r.entry],
            calls_extra(&call_id, "output", extra),
        );
        if answer.cell_done {
            self.open_cells.retain(|c| *c != call_id);
        }
        if let Some(cell_id) = answer.cell_id {
            // A wait that yields again still belongs to the exec that started the cell.
            let exec = self
                .waits
                .get(&call_id)
                .cloned()
                .unwrap_or_else(|| call_id.clone());
            self.yielded.retain(|(cell, _)| *cell != cell_id);
            self.yielded.push((cell_id, exec));
        }
        for process in answer.running {
            self.procs.entry(process).or_insert(r);
        }
        if let (Some(process), Some(code)) = (polled, answer.exit) {
            self.process_exit(&process, code);
        }
        if let Some(exec) = self.waits.remove(&call_id) {
            self.open_cells.retain(|c| *c != exec);
            if let Some((status, detail)) = finished {
                self.yielded.retain(|(_, e)| *e != exec);
                if let Some(cell) = self
                    .calls
                    .get(&exec)
                    .copied()
                    .and_then(|r| self.call_mut(r))
                {
                    (cell.status, cell.status_detail) = (status, detail);
                }
            }
        }
    }

    /// A `write_stdin` poll saw the process exit: the call that started it gets the code.
    fn process_exit(&mut self, process: &str, code: i32) {
        let Some(r) = self.procs.get(process).copied() else {
            return;
        };
        let ts = self.rec.ts;
        let Some(call) = self.call_mut(r) else { return };
        if !matches!(call.tool, Tool::Shell(_)) {
            return;
        }
        let shell = output_mut(call)
            .shell
            .get_or_insert_with(ShellResult::default);
        if shell.exit_code.is_none() {
            shell.exit_code = Some(code);
            if code != 0 {
                call.status = ToolStatus::Error;
            } else if call.status == ToolStatus::Pending {
                call.status = ToolStatus::Ok;
            }
            if call.ended_at.is_none() {
                call.ended_at = ts;
            }
        }
    }

    fn orphan_output(&mut self, call_id: String, body: Value, extra: Obj) {
        let (items, attachments) = body_items(body);
        let text = join_items(&items);
        let status = read_cell(&items)
            .status
            .map_or_else(|| text_status(&text), |s| s.0);
        let output = ToolOutput {
            text,
            attachments,
            ..Default::default()
        };
        self.orphan(
            ToolResult {
                call_id,
                status,
                status_detail: None,
                output,
            },
            extra,
        );
    }

    /// A tool item event: enriches the call with its id, else becomes a child of the open cell
    /// (or of the cell whose process it reports), else a human shell command or an orphan.
    pub(super) fn tool_item(&mut self, ty: &str, item: Obj, times: Times) {
        let id = obj_str(&item, "id").unwrap_or_default();
        if let Some(r) = self.calls.get(&id).copied() {
            return self.enrich(r, ty, item);
        }
        let source = obj_str(&item, "source").unwrap_or_default();
        if source == "user_shell" {
            return self.user_shell(item, times);
        }
        let process = obj_str(&item, "process_id")
            .filter(|_| ty == "CommandExecution" && source != "unified_exec_interaction");
        if let Some((process, r)) = process.and_then(|p| self.procs.remove(&p).map(|r| (p, r))) {
            return self.late_process(r, process, item, times);
        }
        let child = child_call(ty, item, times, self.cwd());
        // A yielded cell keeps running (`yield_control()`), so with no cell open an item comes
        // from the latest yielded cell still running.
        let owner = (self.open_cells.last())
            .or_else(|| self.yielded.last().map(|(_, exec)| exec))
            .and_then(|id| self.calls.get(id))
            .copied();
        match owner.or_else(|| self.cell_running(&child)) {
            Some(cell) => self.push_child(cell, child),
            None => {
                let mut output = child.output.unwrap_or_default();
                // No cell carried the item to the model: what the command printed is its text.
                if let (true, Some(shell)) = (output.text.is_empty(), &output.shell) {
                    output.text = shell.stdout.clone().unwrap_or_default();
                }
                let result = ToolResult {
                    call_id: child.id,
                    status: child.status,
                    status_detail: child.status_detail,
                    output,
                };
                self.orphan(result, Obj::new())
            }
        }
    }

    /// The `spawn_agent` call of the child a `completed` activity reports on: its record has an
    /// id of its own (`subagent-completed-<thread>`), so the call is found by its child thread.
    pub(super) fn spawn_of(&mut self, ty: &str, item: &Obj) -> Option<Ref> {
        let thread = obj_str(item, "agent_thread_id").filter(|_| {
            ty == "SubAgentActivity" && obj_str(item, "kind").as_deref() == Some("completed")
        })?;
        let refs: Vec<Ref> = self.calls.values().copied().collect();
        refs.into_iter().find(|&r| {
            let call = self.call_mut(r);
            call.is_some_and(|c| {
                c.name == "spawn_agent" && c.child_session.as_deref() == Some(&thread)
            })
        })
    }

    /// A spawned agent runs until its `completed` record: a spawn call whose child started and
    /// never completed is still running, unless the child's own rollout (next to this file) shows
    /// its last turn ended: `Interrupted` after a `turn_aborted`, `Ok` after a `task_complete`.
    pub(super) fn running_children(&mut self) {
        let refs: Vec<Ref> = self.calls.values().copied().collect();
        let source = self.s.source.clone();
        for r in refs {
            let Some(call) = self.call_mut(r) else {
                continue;
            };
            let spawned = call.name == "spawn_agent" && call.child_session.is_some();
            if !(spawned && call.status == ToolStatus::Ok && call.status_detail.is_none()) {
                continue;
            }
            let ended = call
                .child_session
                .as_deref()
                .and_then(|t| child_end(&source, t));
            match ended {
                Some(status) => {
                    call.status = status;
                    call.status_detail = Some("ended".into());
                }
                None => {
                    call.status = ToolStatus::Pending;
                    call.ended_at = None;
                }
            }
        }
    }

    /// A command that outlived its call completes: a code cell gets it as a child, a direct
    /// `exec_command` call gets its result.
    fn late_process(&mut self, r: Ref, process: String, item: Obj, times: Times) {
        if !matches!(self.call_mut(r).map(|c| &c.tool), Some(Tool::Code { .. })) {
            self.enrich(r, "CommandExecution", item);
            // The call returned when the process went to the background; it ends with the item.
            if let Some(call) = self.call_mut(r) {
                call.ended_at = times.ended.or(call.ended_at);
            }
            return;
        }
        let mut child = child_call("CommandExecution", item, times, self.cwd());
        if let Some(shell) = child.output.as_mut().and_then(|o| o.shell.as_mut()) {
            shell.background_id = Some(process);
        }
        self.push_child(r, child);
    }

    /// A command item recorded after its cell closed (an interrupted cell, a command that ended
    /// with the cell) belongs to the latest recent cell whose script runs that command.
    fn cell_running(&mut self, child: &ToolCall) -> Option<Ref> {
        if !matches!(child.tool, Tool::Shell(_)) {
            return None;
        }
        let recent: Vec<Ref> = (self.cells.iter().rev().take(8))
            .filter_map(|id| self.calls.get(id).copied())
            .collect();
        recent.into_iter().find(|&r| {
            let Some(cell) = self.call_mut(r) else {
                return false;
            };
            let Tool::Code { source } = &cell.tool else {
                return false;
            };
            let prefix = format!("{}#", cell.id);
            let runs = super::js::invocations(source).map_or(0, |calls| {
                (calls.iter())
                    .filter(|(n, a)| {
                        n == "exec_command" && same_command(&tool_for(n, a, None), &child.tool)
                    })
                    .count()
            });
            let recorded = (cell.children.iter())
                .filter(|c| !c.id.starts_with(&prefix) && same_command(&c.tool, &child.tool))
                .count();
            recorded < runs
        })
    }

    /// A recorded command replaces the child that was read from the cell's printed chunk.
    fn push_child(&mut self, cell: Ref, child: ToolCall) {
        let Some(cell) = self.call_mut(cell) else {
            return;
        };
        let prefix = format!("{}#", cell.id);
        let read = (cell.children.iter()).position(|c| {
            let ended = exit_of(c).zip(exit_of(&child));
            c.id.starts_with(&prefix)
                && same_command(&c.tool, &child.tool)
                && ended.is_none_or(|(a, b)| a == b)
        });
        match read {
            Some(i) => cell.children[i] = child,
            None => cell.children.push(child),
        }
    }

    fn user_shell(&mut self, item: Obj, times: Times) {
        self.close_group();
        let call = command_execution(item, times, self.cwd(), true);
        let id = Some(call.id.clone());
        let m = Message {
            kind: Some("user_shell".into()),
            ..Message::new(Role::User, Origin::Command, vec![Part::ToolCall(call)])
        };
        self.push(id, None, EntryKind::Message(m), Obj::new());
    }

    /// A result whose call is not in the parsed records.
    fn orphan(&mut self, result: ToolResult, extra: Obj) {
        self.close_group();
        self.warn(
            self.rec.line,
            format!(
                "result of call {} outside the parsed records",
                result.call_id
            ),
        );
        let m = Message::new(Role::User, Origin::Tool, vec![Part::ToolResult(result)]);
        self.push(None, None, EntryKind::Message(m), extra);
    }

    /// A completion event for a call (legacy `*_end` events, items whose id is the call id).
    pub(super) fn enrich(&mut self, r: Ref, kind: &str, mut ev: Obj) {
        let Some(call) = self.call_mut(r) else { return };
        match kind {
            "exec_command_end" | "CommandExecution" => shell_end(call, &mut ev),
            "patch_apply_end" | "FileChange" => patch_end(call, &mut ev),
            // The child finished: the call that spawned it ends with it.
            "SubAgentActivity" if obj_str(&ev, "kind").as_deref() == Some("completed") => {
                call.status = ToolStatus::Ok;
                call.status_detail = Some("completed".into());
                call.ended_at = ev
                    .get("completed_at_ms")
                    .and_then(Value::as_i64)
                    .or(call.ended_at);
            }
            "SubAgentActivity" => call.child_session = obj_str(&ev, "agent_thread_id"),
            "CollabAgentToolCall" => {
                call.status = item_status(&ev);
                call.status_detail = obj_str(&ev, "status");
                if let Some(Value::Array(ids)) = ev.get("receiver_thread_ids") {
                    if let [Value::String(id)] = ids.as_slice() {
                        call.child_session = Some(id.clone());
                    }
                }
            }
            "mcp_tool_call_end" | "McpToolCall" => {
                let failed = ev
                    .get("result")
                    .is_some_and(|r| r.get("Err").is_some() || mcp_error(r.get("Ok").unwrap_or(r)));
                call.status = if failed {
                    ToolStatus::Error
                } else {
                    item_status(&ev)
                };
            }
            _ => {}
        }
        if !ev.contains_key("type") {
            ev.insert("type".into(), Value::String(kind.into()));
        }
        push_data(output_mut(call), stripped(Value::Object(ev)));
    }
}

/// How a child thread's rollout ends, read from the tail of its file in the parent's directory:
/// `Interrupted` when the last turn record is `turn_aborted`, `Ok` for `task_complete`, `None` when
/// it is still in a turn or the file is not there.
fn child_end(source: &Source, thread: &str) -> Option<ToolStatus> {
    let Source::File { path } = source else {
        return None;
    };
    let name = |p: &std::path::Path| p.file_name()?.to_str().map(str::to_owned);
    let file = std::fs::read_dir(path.parent()?)
        .ok()?
        .flatten()
        .find(|e| {
            let n = name(&e.path()).unwrap_or_default();
            n.ends_with(".jsonl") && super::thread_id(&n) == Some(thread)
        })?;
    let mut f = std::fs::File::open(file.path()).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(CHILD_TAIL)))
        .ok()?;
    let mut tail = String::new();
    f.take(CHILD_TAIL).read_to_string(&mut tail).ok()?;
    let last = tail.lines().rev().find_map(|l| {
        ["turn_aborted", "task_complete", "task_started"]
            .into_iter()
            .find(|t| l.contains(&format!("\"type\":\"{t}\"")))
    });
    match last? {
        "turn_aborted" => Some(ToolStatus::Interrupted),
        "task_complete" => Some(ToolStatus::Ok),
        _ => None,
    }
}

/// The exit code of a call that ran a command.
pub(super) fn exit_of(call: &ToolCall) -> Option<i32> {
    call.output.as_ref()?.shell.as_ref()?.exit_code
}

/// Both are shell commands with the same command text.
pub(super) fn same_command(a: &Tool, b: &Tool) -> bool {
    matches!((a, b), (Tool::Shell(a), Tool::Shell(b)) if a.command == b.command)
}

/// The typed view of a direct tool call's input.
pub(super) fn tool_for(name: &str, input: &Value, cwd: Option<String>) -> Tool {
    let typed = match name {
        "exec" => input.as_str().map(|source| Tool::Code {
            source: source.to_owned(),
        }),
        "shell" | "container.exec" | "local_shell_call" => shell_input(input, cwd),
        "shell_command" | "exec_command" => str_field(input, &["command", "cmd"]).map(|command| {
            let cwd = str_field(input, &["workdir"]).or(cwd);
            let timeout_ms = u64_field(input, &["timeout_ms"]);
            Tool::Shell(Shell {
                command,
                cwd,
                timeout_ms,
                ..Default::default()
            })
        }),
        "write_stdin" => {
            let chars = str_field(input, &["chars"]).filter(|c| !c.is_empty());
            let action = if chars.is_some() {
                ProcessAction::Write
            } else {
                ProcessAction::Read
            };
            Some(Tool::Process {
                id: str_field(input, &["session_id"]),
                action,
                input: chars,
            })
        }
        "wait" if input.get("cell_id").is_some() => {
            let action = if bool_field(input, &["terminate"]) {
                ProcessAction::Kill
            } else {
                ProcessAction::Read
            };
            Some(Tool::Process {
                id: str_field(input, &["cell_id"]),
                action,
                input: None,
            })
        }
        "apply_patch" => input
            .as_str()
            .map(str::to_owned)
            .or_else(|| str_field(input, &["input", "patch"]))
            .map(|patch| Tool::Patch { patch }),
        "update_plan" => Some(todo(input)),
        "view_image" => str_field(input, &["path"]).map(|path| Tool::Read {
            path,
            offset: None,
            limit: None,
        }),
        "request_user_input" => Some(Tool::Question),
        "web_search_call" | "web_search" => Some(web_action(Some(input), None, None)),
        n if COLLAB_TOOLS.contains(&n) || n == "wait" => Some(Tool::Task {
            description: str_field(input, &["task_name", "target", "path_prefix"]),
            prompt: str_field(input, &["message", "prompt"]),
            agent_type: str_field(input, &["agent_type"]),
        }),
        n => mcp_tool(n),
    };
    typed.unwrap_or(Tool::Other)
}

/// `shell` and `local_shell_call`: an argv `command` (sometimes a plain string).
fn shell_input(input: &Value, cwd: Option<String>) -> Option<Tool> {
    let cwd = str_field(input, &["workdir", "working_directory"]).or(cwd);
    let timeout = u64_field(input, &["timeout_ms"]);
    let shell = match input.get("command")? {
        Value::String(command) => Shell {
            command: command.clone(),
            cwd,
            timeout_ms: timeout,
            ..Default::default()
        },
        argv => argv_shell(texts(argv), cwd, timeout),
    };
    (!shell.command.is_empty()).then_some(Tool::Shell(shell))
}

/// `command` is the script of `[sh, -c|-lc, script]`, else the argv joined and shell-quoted.
pub(super) fn argv_shell(argv: Vec<String>, cwd: Option<String>, timeout_ms: Option<u64>) -> Shell {
    let command = match argv.as_slice() {
        [sh, flag, script] if is_shell(sh) && matches!(flag.as_str(), "-c" | "-lc") => {
            script.clone()
        }
        _ => argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    };
    Shell {
        command,
        argv,
        cwd,
        timeout_ms,
        background: false,
        description: None,
    }
}

/// A POSIX shell word: bare when it is plain, else in single quotes.
fn shell_quote(arg: &str) -> String {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b);
    if !arg.is_empty() && arg.bytes().all(plain) {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn is_shell(program: &str) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish")
}

fn todo(input: &Value) -> Tool {
    let items = input
        .get("plan")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let items = items
        .filter_map(|step| {
            let content = str_field(step, &["step", "content"])?;
            Some(TodoItem {
                content,
                status: str_field(step, &["status"]),
                priority: None,
            })
        })
        .collect();
    Tool::Todo { items }
}

/// Web search actions: `search` (query or queries), `open_page` / `openPage` (url),
/// `find_in_page` / `findInPage` (url, pattern in `prompt`). An action that names no page (a
/// `findInPage` with a null url, `other`) reads the page from the first of the item's `results`.
fn web_action(action: Option<&Value>, query: Option<String>, results: Option<&Value>) -> Tool {
    let ty = action
        .and_then(|a| str_field(a, &["type"]))
        .unwrap_or_default();
    let url = action.and_then(|a| str_field(a, &["url"]));
    let page = || results?.get(0).and_then(|r| str_field(r, &["url"]));
    match ty.as_str() {
        "open_page" | "openPage" | "find_in_page" | "findInPage" => Tool::WebFetch {
            url: url.or_else(page),
            prompt: action.and_then(|a| str_field(a, &["pattern"])),
        },
        _ => {
            let first = action
                .and_then(|a| a.get("queries"))
                .map(texts)
                .and_then(|q| q.into_iter().next());
            let query = action
                .and_then(|a| str_field(a, &["query"]))
                .or(first)
                .or(query)
                .filter(|q| !q.trim().is_empty());
            match (query, page()) {
                (None, Some(url)) => Tool::WebFetch {
                    url: Some(url),
                    prompt: None,
                },
                (query, _) => Tool::WebSearch { query },
            }
        }
    }
}

pub(super) fn web_item(item: &Obj) -> Tool {
    match obj_str(item, "kind") {
        Some(kind) if kind != "web.search" => Tool::Other,
        _ => web_action(
            item.get("action"),
            obj_str(item, "query"),
            item.get("results"),
        ),
    }
}

/// `web_search_call` and `image_generation_call` carry their own result.
fn self_contained(call: &mut ToolCall, p: &mut Obj) {
    call.status_detail = obj_str(p, "status");
    call.status = call
        .status_detail
        .as_deref()
        .and_then(status_word)
        .unwrap_or(ToolStatus::Ok);
    if let Some(Value::String(b64)) = p.remove("result") {
        let image = Media {
            bytes: Some(base64_len(&b64)),
            ..Default::default()
        };
        output_mut(call).attachments.push(image);
    }
}

fn arguments(v: Option<Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        Some(v) => v,
        None => Value::Null,
    }
}

/// Inline blobs and encrypted tokens become `{"bytes": n}`. Codex encrypts agent messages and
/// some tool arguments as Fernet tokens, which always start with `gAAAAA`.
fn hide_blobs(v: &mut Value) {
    strip_blobs(v);
    hide_tokens(v);
}

fn hide_tokens(v: &mut Value) {
    match v {
        Value::String(s) if is_token(s) => *v = json!({ "bytes": base64_len(s) }),
        Value::Array(items) => items.iter_mut().for_each(hide_tokens),
        Value::Object(map) => map.values_mut().for_each(hide_tokens),
        _ => {}
    }
}

fn is_token(s: &str) -> bool {
    s.len() >= 16
        && s.starts_with("gAAAAA")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_=+/".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_from_argv() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            argv_shell(argv(&["/bin/zsh", "-lc", "ls -a"]), None, None).command,
            "ls -a"
        );
        assert_eq!(
            argv_shell(argv(&["git", "status"]), None, None).command,
            "git status"
        );
        let quoted = argv_shell(argv(&["git", "commit", "-m", "it's done"]), None, None);
        assert_eq!(quoted.command, "git commit -m 'it'\\''s done'");
        assert_eq!(
            argv_shell(argv(&["printf", ""]), None, None).command,
            "printf ''"
        );
    }

    #[test]
    fn encrypted_tokens() {
        assert!(is_token("gAAAAAB0ZmFrZQ==") && !is_token("gAAAAA plain words"));
        let mut v = json!({"task_name": "x", "message": "gAAAAAB0ZmFrZQ=="});
        hide_blobs(&mut v);
        assert_eq!(v, json!({"task_name": "x", "message": {"bytes": 10}}));
    }
}
