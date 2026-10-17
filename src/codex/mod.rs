//! Codex CLI rollouts: `~/.codex/sessions/YYYY/MM/DD/rollout-<local time>-<thread id>.jsonl[.zst]`.
//!
//! Every line is handled by its own shape, so one parser reads every generation:
//! - flat (Rust CLI up to about 0.32): a first line `{id, timestamp, instructions}`, then bare
//!   response items and `{"record_type": "state"}` lines;
//! - envelope `{timestamp, [ordinal], type, payload}` with `history_mode` absent or `legacy`:
//!   user and assistant text arrive twice (response item and `event_msg`) with no shared id;
//! - envelope with `history_mode: paginated` (0.143+): `item_completed` events carry every item.
//!
//! Mapping:
//! - One assistant [`Message`] per model response: the assistant items up to the response's
//!   `token_usage_record` (before 0.153: a `token_count` with a new cumulative total), a tool
//!   output, or the next input. An item recorded both as an event and as a response item is
//!   emitted once.
//! - Tool outputs and completion events fold into their call. A code-mode `exec` cell gets the
//!   tool items recorded between the call and its output as `children`; a command that outlives
//!   its cell joins that cell later by process id, and a `Shell` item recorded after its cell
//!   closed joins one of the last 8 cells that runs the same command text. A command the sandbox
//!   refused has no item: when `js` can read the cell's program, each finished printed chunk
//!   with no recorded item (same command text and exit code) becomes a child call. Headerless
//!   (aborted) cells and chunks whose exit code is unreadable get none.
//! - Unmapped keys: the `session_meta` rest goes to `Session.extra`; a record absorbed into an
//!   entry goes to that entry's `extra` (the first record wins a key clash); call and output
//!   records go to `extra.calls.<call_id>.{call,output}` of the message holding the call;
//!   completion events go to `ToolOutput.data`.
//!
//! Layout: `messages` (input messages, model responses, usage), `tools` (calls and their typed
//! inputs), `results` (what outputs, completion events and tool items say), `js` (the cell
//! program reader), `json` (helpers).

use crate::util::{Obj, as_extra, json_lines, obj_str, str_field, take_str, ts_ms};
use crate::{
    Agent, Compaction, Entry, EntryKind, Error, ErrorInfo, Other, ParentLink, Part, Result, Rewind,
    Role, Session, Settings, Source, Turn,
};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::BufRead;

mod js;
mod json;
mod messages;
mod results;
mod tools;

use json::{item_extra, label, merge_extra, take, take_label, with_meta, with_type};
use messages::{content_parts, texts};

const PASSTHROUGH: &str = "internal_chat_message_metadata_passthrough";
/// Marker Codex puts in the middle of a command output longer than 64 KiB.
const PERSIST_CUT: &str = "command output truncated for persistence";
const TRUNCATED: &str = "Warning: truncated output";
const COLLAB_TOOLS: &[&str] = &[
    "spawn_agent",
    "send_message",
    "followup_task",
    "wait_agent",
    "send_input",
    "resume_agent",
    "close_agent",
    "interrupt_agent",
    "list_agents",
];

/// Parse a Codex rollout from any slice of its lines.
pub(crate) fn parse_reader(reader: impl BufRead, source: Source) -> Result<Session> {
    let mut parser = Parser::new(source);
    for record in json_lines(reader) {
        let (line, parsed) = record?;
        match parsed {
            Ok(v) => parser.record(line, v),
            Err(e) => parser.warn(line, format!("invalid JSON: {e}")),
        }
    }
    parser.finish()
}

/// A part inside `Session.entries`. Entries and parts are only appended (an entry changes kind
/// only when it has no parts: a usage-only response becoming a compaction), so it stays valid.
#[derive(Clone, Copy, PartialEq)]
struct Ref {
    entry: usize,
    part: usize,
}

/// The source record being handled.
#[derive(Clone, Default)]
struct Rec {
    line: usize,
    ts: Option<i64>,
    ordinal: Option<u64>,
}

/// `item_completed` envelope times.
#[derive(Clone, Copy, Default)]
struct Times {
    started: Option<i64>,
    ended: Option<i64>,
}

/// A user prompt seen once so far: Codex records it twice (response item, event) without a
/// shared id, so the copies meet by `key` (`prompt_key`).
struct Unmatched {
    event: bool,
    key: String,
    entry: usize,
}

/// A legacy `agent_message` event waiting for the response item it repeats, with the record
/// and turn it came from in case no item claims it.
struct AgentEvent {
    text: String,
    phase: Option<String>,
    extra: Obj,
    rec: Rec,
    turn: Option<String>,
}

struct Parser {
    s: Session,
    rec: Rec,
    records: usize,
    /// An envelope record was seen; files without any are the flat era.
    enveloped: bool,
    paginated: bool,
    meta_seen: bool,
    /// A child thread's file starts with a copy of its parent's history; records below this
    /// ordinal are that copy.
    start_ordinal: u64,
    /// This session's multi-agent path; inter-agent messages from it are its own.
    own_path: String,
    model: Option<String>,
    turn: Option<String>,
    settings: Option<Settings>,
    /// Message entries of the open model response, and of the response closed before it.
    group: Vec<usize>,
    last_group: Vec<usize>,
    /// Assistant text and reasoning parts by item id, to merge the second copy.
    items: HashMap<String, Ref>,
    calls: HashMap<String, Ref>,
    /// Code-mode cells whose output has not arrived: tool items recorded now are theirs.
    open_cells: Vec<String>,
    /// Every `exec` cell call id, oldest first.
    cells: Vec<String>,
    /// Yielded cells still running, oldest first: (cell id, the `exec` call that started it). A
    /// `wait` call id maps to that same `exec` call id.
    yielded: Vec<(String, String)>,
    waits: HashMap<String, String>,
    /// A process still running when its call returned, by process (session) id.
    procs: HashMap<String, Ref>,
    unmatched: VecDeque<Unmatched>,
    agent_events: Vec<AgentEvent>,
    /// A `token_usage_record` was seen: `token_count` usage no longer counts.
    records_seen: bool,
    /// Legacy `agent_reasoning` texts waiting for the reasoning item that repeats them.
    reasoning: Vec<String>,
    /// Cumulative thread token total already attributed, to skip re-emitted counts.
    cum_total: u64,
    /// An interrupt seen as one of its two records (marker message, `turn_aborted` event).
    abort: Option<usize>,
    /// Entry holding each model response's usage, and the compaction entry by its model call's response id.
    responses: HashMap<String, usize>,
    compactions: HashMap<String, usize>,
}

impl Parser {
    fn new(source: Source) -> Parser {
        Parser {
            s: Session::new(Agent::Codex, "", source, "codex-legacy"),
            rec: Rec::default(),
            records: 0,
            enveloped: false,
            paginated: false,
            meta_seen: false,
            start_ordinal: 0,
            own_path: "/root".into(),
            model: None,
            turn: None,
            settings: None,
            group: Vec::new(),
            last_group: Vec::new(),
            items: HashMap::new(),
            calls: HashMap::new(),
            open_cells: Vec::new(),
            cells: Vec::new(),
            yielded: Vec::new(),
            waits: HashMap::new(),
            procs: HashMap::new(),
            unmatched: VecDeque::new(),
            agent_events: Vec::new(),
            records_seen: false,
            reasoning: Vec::new(),
            cum_total: 0,
            abort: None,
            responses: HashMap::new(),
            compactions: HashMap::new(),
        }
    }

    fn warn(&mut self, line: usize, message: String) {
        self.s.warnings.push(crate::Warning {
            line: Some(line),
            message,
        });
    }

    fn record(&mut self, line: usize, v: Value) {
        let Value::Object(mut o) = v else {
            return self.warn(line, "record is not a JSON object".into());
        };
        let ordinal = o.get("ordinal").and_then(Value::as_u64);
        if ordinal.is_some_and(|n| n < self.start_ordinal) {
            return;
        }
        self.records += 1;
        self.rec = Rec {
            line,
            ts: o.get("timestamp").and_then(ts_ms),
            ordinal,
        };
        let ty = o.get("type").and_then(Value::as_str).map(str::to_owned);
        match (ty, o.remove("payload")) {
            (Some(ty), Some(Value::Object(p))) => {
                self.enveloped = true;
                let meta = o.remove("metadata");
                self.envelope(&ty, p, meta)
            }
            // The flat era: bare response items, state lines and a first line without a type.
            (Some(_), _) => self.response_item(o, None),
            (None, _) if o.contains_key("record_type") => {
                o.remove("record_type");
                if !o.is_empty() {
                    self.other("state", o);
                }
            }
            (None, _) => self.session_meta(o),
        }
    }

    fn envelope(&mut self, ty: &str, p: Obj, meta: Option<Value>) {
        match ty {
            "session_meta" => self.session_meta(p),
            "turn_context" => self.turn_context(p),
            "response_item" => self.response_item(p, meta),
            "event_msg" => self.event(p),
            "token_usage_record" => self.usage_record(p),
            "compacted" => self.compacted(p),
            "inter_agent_communication" => self.agent_message(p, meta),
            _ => self.other(ty, with_meta(p, meta)),
        }
    }

    fn response_item(&mut self, mut p: Obj, meta: Option<Value>) {
        let ty = take_str(&mut p, "type").unwrap_or_default();
        match ty.as_str() {
            "message" => self.message_item(p, meta),
            "reasoning" => self.reasoning_item(p, meta),
            "agent_message" => self.agent_message(p, meta),
            "function_call"
            | "custom_tool_call"
            | "local_shell_call"
            | "web_search_call"
            | "tool_search_call"
            | "image_generation_call" => self.call(&ty, p, meta),
            "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
                self.output(p, meta)
            }
            "compaction" | "compaction_summary" | "context_compaction" => {
                self.fold_compaction(&ty, with_meta(p, meta))
            }
            _ => self.other(&ty, with_meta(p, meta)),
        }
    }

    fn event(&mut self, mut p: Obj) {
        let ty = take_str(&mut p, "type").unwrap_or_default();
        p.remove("thread_id");
        match ty.as_str() {
            "item_completed" => self.item_completed(p),
            "user_message" => self.legacy_user(p),
            "agent_message" => {
                let text = take_str(&mut p, "message").unwrap_or_default();
                let phase = take_str(&mut p, "phase");
                self.agent_event(text, phase, p)
            }
            "agent_reasoning" | "agent_reasoning_raw_content" => {
                let text = take_str(&mut p, "text").filter(|t| !t.is_empty());
                self.reasoning.extend(text)
            }
            "token_count" => self.token_count(p),
            "task_started" | "turn_started" => self.turn_start(&ty, p),
            "task_complete" | "turn_complete" => self.turn_end(p, false),
            "turn_aborted" => self.turn_end(p, true),
            "thread_settings_applied" => self.thread_settings(p),
            "thread_rolled_back" => self.rewind(p),
            "context_compacted" => self.fold_compaction(&ty, p),
            "error" | "stream_error" => self.error(&ty, p),
            _ => match p
                .get("call_id")
                .and_then(Value::as_str)
                .and_then(|id| self.calls.get(id))
                .copied()
            {
                // exec_command_end, patch_apply_end, mcp_tool_call_end, web_search_end, ...
                Some(r) => self.enrich(r, &ty, p),
                None => self.other(&ty, p),
            },
        }
    }

    /// An `item_completed` event. Message items keep the envelope (times, turn) in `extra`, tool
    /// items map the times to `started_at`/`ended_at`, the rest carry the envelope in their data.
    fn item_completed(&mut self, mut p: Obj) {
        let Some(Value::Object(mut item)) = p.remove("item") else {
            return self.other("item_completed", p);
        };
        let ty = take_str(&mut item, "type").unwrap_or_default();
        let id = obj_str(&item, "id");
        match ty.as_str() {
            "UserMessage" | "AgentMessage" | "Reasoning" | "Plan" => {
                let turn = take_str(&mut p, "turn_id");
                self.message_item_completed(&ty, item, turn, p)
            }
            "CommandExecution" | "Extension" | "McpToolCall" | "DynamicToolCall" | "FileChange"
            | "ImageView" | "ImageGeneration" | "WebSearch" => {
                let mut time = |k: &str| p.remove(k).as_ref().and_then(Value::as_i64);
                let times = Times {
                    started: time("started_at_ms"),
                    ended: time("completed_at_ms"),
                };
                item.extend(p);
                self.tool_item(&ty, with_type(item, &ty), times)
            }
            _ => {
                item.extend(p);
                match ty.as_str() {
                    "HookPrompt" => self.hook_prompt(item),
                    "ContextCompaction" => self.fold_compaction(&ty, item),
                    "SubAgentActivity" | "CollabAgentToolCall" | "FunctionCallOutput" => {
                        match id.and_then(|id| self.calls.get(&id)).copied() {
                            Some(r) => self.enrich(r, &ty, with_type(item, &ty)),
                            None => self.other(&ty, item),
                        }
                    }
                    _ => self.other(&ty, item),
                }
            }
        }
    }

    fn message_item_completed(
        &mut self,
        ty: &str,
        mut item: Obj,
        turn: Option<String>,
        mut p: Obj,
    ) {
        let id = take_str(&mut item, "id");
        match ty {
            "UserMessage" => {
                let parts = content_parts(take(&mut item, "content"));
                p.extend(item);
                self.user_event(id, parts, turn, p)
            }
            "AgentMessage" => {
                let text = texts(&take(&mut item, "content")).concat();
                let phase = take_str(&mut item, "phase");
                p.extend(item);
                self.agent_text(id, text, phase, turn, p)
            }
            "Reasoning" => {
                let mut text = texts(&take(&mut item, "summary_text"));
                text.extend(texts(&take(&mut item, "raw_content")));
                p.extend(item);
                self.add_reasoning(id, text, false, turn, p)
            }
            _ => {
                // A plan-mode plan: the raw assistant text may hold the same block, so it is not
                // a text part (it would read twice).
                let plan = Part::Other {
                    kind: "plan".into(),
                    data: Value::Object(item),
                };
                let extra = item_extra(id.as_deref(), p);
                self.add_part(plan, id, turn, None, extra);
            }
        }
    }

    // ---- identity and settings ----

    /// The first `session_meta` is the identity. In a child file ordinal 1 is a copy of the root
    /// meta, so a window that starts there (a tail read) keeps the id from the file name.
    fn session_meta(&mut self, mut p: Obj) {
        if self.meta_seen || self.rec.ordinal.is_some_and(|n| n > 0) {
            return self.other("session_meta", p);
        }
        self.meta_seen = true;
        if let Some(id) = take_str(&mut p, "id") {
            self.s.id = id;
        }
        self.s.cwd = take_str(&mut p, "cwd");
        self.s.cli_version = take_str(&mut p, "cli_version");
        self.s.started_at = p.remove("timestamp").as_ref().and_then(ts_ms);
        if let Some(git) = p.remove("git") {
            let field = |k: &str| str_field(&git, &[k]);
            self.s.git = Some(crate::Git {
                branch: field("branch"),
                commit: field("commit_hash"),
                remote: field("repository_url"),
            });
        }
        self.paginated = p.get("history_mode").and_then(Value::as_str) == Some("paginated");
        self.start_ordinal = p
            .get("subagent_history_start_ordinal")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let spawn = p
            .get("source")
            .and_then(|s| s.pointer("/subagent/thread_spawn"))
            .cloned()
            .unwrap_or_default();
        if let Some(path) = obj_str(&p, "agent_path").or_else(|| str_field(&spawn, &["agent_path"]))
        {
            self.own_path = path;
        }
        let parent = take_str(&mut p, "parent_thread_id")
            .or_else(|| str_field(&spawn, &["parent_thread_id"]));
        if let Some(session_id) = parent {
            let agent_type =
                take_str(&mut p, "agent_path").or_else(|| str_field(&spawn, &["agent_path"]));
            self.s.parent = Some(ParentLink {
                session_id,
                call_id: None,
                agent_type,
            });
        }
        if p.contains_key("history_base") {
            let line = self.rec.line;
            self.warn(
                line,
                "history_base: earlier history lives in another rollout".into(),
            );
        }
        self.s.extra = as_extra(p);
    }

    fn turn_context(&mut self, mut p: Obj) {
        self.close_group();
        if let Some(turn) = take_str(&mut p, "turn_id") {
            self.turn = Some(turn);
        }
        let collab = p.get("collaboration_mode").cloned().unwrap_or_default();
        let next = Settings {
            model: take_str(&mut p, "model"),
            agent: str_field(&collab, &["mode"]),
            cwd: take_str(&mut p, "cwd"),
            effort: take_str(&mut p, "effort")
                .or_else(|| str_field(&collab["settings"], &["reasoning_effort"])),
            approval: take_label(&mut p, "approval_policy"),
            sandbox: label(p.get("sandbox_policy")),
        };
        self.apply_settings(next, p);
    }

    /// `thread_settings_applied` states only some settings; the rest carry over.
    fn thread_settings(&mut self, mut p: Obj) {
        let Some(Value::Object(mut t)) = p.remove("thread_settings") else {
            return self.other("thread_settings_applied", p);
        };
        let prev = self.settings.clone().unwrap_or_default();
        let collab = t.get("collaboration_mode").cloned().unwrap_or_default();
        let next = Settings {
            model: take_str(&mut t, "model").or(prev.model),
            agent: str_field(&collab, &["mode"]).or(prev.agent),
            cwd: take_str(&mut t, "cwd").or(prev.cwd),
            effort: take_str(&mut t, "reasoning_effort").or(prev.effort),
            approval: take_label(&mut t, "approval_policy").or(prev.approval),
            sandbox: label(t.get("sandbox_policy")).or(prev.sandbox),
        };
        p.insert("thread_settings".into(), Value::Object(t));
        self.apply_settings(next, p);
    }

    fn apply_settings(&mut self, next: Settings, rest: Obj) {
        if next.model.is_some() {
            self.model = next.model.clone();
        }
        if self.settings.as_ref() == Some(&next) {
            return;
        }
        self.settings = Some(next.clone());
        self.push(None, None, EntryKind::Settings(next), rest);
    }

    fn cwd(&self) -> Option<String> {
        self.settings
            .as_ref()
            .and_then(|s| s.cwd.clone())
            .or_else(|| self.s.cwd.clone())
    }

    // ---- entries ----

    fn push(
        &mut self,
        id: Option<String>,
        turn: Option<String>,
        kind: EntryKind,
        extra: Obj,
    ) -> usize {
        self.s.entries.push(Entry {
            id: id.or_else(|| self.rec.ordinal.map(|n| format!("ord:{n}"))),
            parent_id: None,
            turn_id: turn.or_else(|| self.turn.clone()),
            ts: self.rec.ts,
            line: Some(self.rec.line),
            extra: as_extra(extra),
            kind,
        });
        self.s.entries.len() - 1
    }

    fn other(&mut self, kind: &str, p: Obj) {
        let data = as_extra(p).unwrap_or_else(|| json!({}));
        self.push(
            None,
            None,
            EntryKind::Other(Other {
                kind: kind.into(),
                data,
            }),
            Obj::new(),
        );
    }

    fn error(&mut self, kind: &str, mut p: Obj) {
        let message = take_str(&mut p, "message").unwrap_or_default();
        let kind =
            label(p.get("codex_error_info")).or_else(|| (kind != "error").then(|| kind.to_owned()));
        let info = ErrorInfo {
            kind,
            message,
            status: None,
            retry: None,
        };
        self.push(None, None, EntryKind::Error(info), p);
    }

    fn rewind(&mut self, mut p: Obj) {
        self.close_group();
        let turns = p
            .remove("num_turns")
            .and_then(|n| n.as_u64())
            .and_then(|n| u32::try_from(n).ok());
        self.push(
            None,
            None,
            EntryKind::Rewind(Rewind {
                dropped: Vec::new(),
                turns,
            }),
            p,
        );
    }

    fn turn_start(&mut self, kind: &str, p: Obj) {
        self.close_group();
        self.abort = None;
        self.turn = obj_str(&p, "turn_id");
        self.other(kind, p);
    }

    fn turn_end(&mut self, mut p: Obj, aborted: bool) {
        self.close_group();
        self.open_cells.clear();
        let turn = take_str(&mut p, "turn_id");
        let duration_ms = p.remove("duration_ms").and_then(|d| d.as_u64());
        let error = p.remove("error").filter(|e| !e.is_null());
        let error = error.map(|e| str_field(&e, &["message"]).unwrap_or_else(|| e.to_string()));
        let status = match (aborted, &error) {
            (true, _) => "interrupted".into(),
            (false, Some(_)) => "failed".into(),
            (false, None) => "completed".into(),
        };
        let done = EntryKind::Turn(Turn {
            status: Some(status),
            duration_ms,
            error,
        });
        match (aborted, self.abort.take()) {
            (true, Some(i)) => {
                let e = &mut self.s.entries[i];
                e.kind = done;
                e.turn_id = e.turn_id.take().or(turn);
                merge_extra(e, p);
            }
            _ => {
                let i = self.push(None, turn, done, p);
                self.abort = aborted.then_some(i);
            }
        }
        self.turn = None;
    }

    /// The `<turn_aborted>` message pairs with the `turn_aborted` event into one `Turn`.
    fn abort_marker(
        &mut self,
        text: String,
        id: Option<String>,
        turn: Option<String>,
        mut extra: Obj,
    ) {
        extra.insert("marker".into(), Value::String(text));
        match self.abort.take() {
            Some(i) => merge_extra(&mut self.s.entries[i], extra),
            None => {
                let turn_kind = EntryKind::Turn(Turn {
                    status: Some("interrupted".into()),
                    ..Default::default()
                });
                self.abort = Some(self.push(id, turn, turn_kind, extra));
            }
        }
    }

    /// The entry compaction records arriving now belong to. Interleaved `Other` records are
    /// skipped; a model response that wrote nothing but its usage is the compaction's own call
    /// and becomes the compaction.
    fn compaction_entry(&mut self) -> usize {
        self.close_group();
        match self.last_entry() {
            Some(i) if matches!(self.s.entries[i].kind, EntryKind::Compaction(_)) => i,
            Some(i) if self.usage_only(i) => self.make_compaction(i),
            _ => self.push(
                None,
                None,
                EntryKind::Compaction(Compaction::default()),
                Obj::new(),
            ),
        }
    }

    /// The latest entry other than `Other` records.
    pub(super) fn last_entry(&self) -> Option<usize> {
        (self.s.entries.iter()).rposition(|e| !matches!(e.kind, EntryKind::Other(_)))
    }

    fn usage_only(&self, i: usize) -> bool {
        matches!(&self.s.entries[i].kind, EntryKind::Message(m) if m.role == Role::Assistant && m.parts.is_empty())
    }

    fn make_compaction(&mut self, i: usize) -> usize {
        let e = &mut self.s.entries[i];
        let usage = match &e.kind {
            EntryKind::Message(m) => m.usage,
            _ => None,
        };
        e.kind = EntryKind::Compaction(Compaction {
            usage,
            ..Default::default()
        });
        i
    }

    /// The `compacted` line carries the summary; `replacement_history` stays in `extra`. The
    /// usage record of the compaction's own model call (`compaction_response_id`) is its usage.
    fn compacted(&mut self, mut p: Obj) {
        let summary = take_str(&mut p, "message").filter(|s| !s.is_empty());
        let response = obj_str(&p, "compaction_response_id");
        let i = match response
            .as_ref()
            .and_then(|r| self.responses.get(r))
            .copied()
        {
            Some(e) => self.compaction_from_response(e),
            None => self.compaction_entry(),
        };
        if let Some(response) = response {
            self.compactions.insert(response, i);
        }
        let e = &mut self.s.entries[i];
        if let EntryKind::Compaction(c) = &mut e.kind {
            c.summary = c.summary.take().or(summary);
        }
        merge_extra(e, p);
    }

    /// The compaction call's usage record came first and went to entry `e` as a model response.
    fn compaction_from_response(&mut self, e: usize) -> usize {
        self.close_group();
        if self.usage_only(e) {
            return self.make_compaction(e);
        }
        let usage = match &mut self.s.entries[e].kind {
            EntryKind::Message(m) => m.usage.take(),
            _ => None,
        };
        let c = self.compaction_entry();
        if let Some(usage) = usage {
            self.compaction_usage(c, usage, Obj::new());
        }
        c
    }

    /// The other records of one compaction (item, event, response item) fold into it.
    fn fold_compaction(&mut self, kind: &str, p: Obj) {
        let i = self.compaction_entry();
        let mut add = Obj::new();
        add.insert(kind.into(), Value::Object(p));
        merge_extra(&mut self.s.entries[i], add);
    }

    // ---- finish ----

    fn finish(mut self) -> Result<Session> {
        if self.records == 0 {
            return Err(Error::NotASession("no Codex rollout records".into()));
        }
        self.close_group();
        let format = if !self.enveloped {
            "codex-flat"
        } else if self.paginated {
            "codex-paginated"
        } else {
            "codex-legacy"
        };
        self.s.format = format.into();
        if let (true, Source::File { path }) = (self.s.id.is_empty(), &self.s.source) {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let stem = name.split(".jsonl").next().unwrap_or(name);
            self.s.id = thread_id(name).unwrap_or(stem).to_owned();
        }
        Ok(self.s.finish())
    }
}

/// The thread id of a rollout file name,
/// `rollout-<YYYY-MM-DDThh-mm-ss>-<thread id>[_<rollout id>].jsonl[.zst]`; `None` for other names.
pub(crate) fn thread_id(file_name: &str) -> Option<&str> {
    let rest = file_name.strip_prefix("rollout-")?;
    let stem = rest
        .strip_suffix(".jsonl")
        .or_else(|| rest.strip_suffix(".jsonl.zst"))?;
    stem.get(20..).unwrap_or(stem).split('_').next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_file_names() {
        let id = "0190a000-0000-7000-8000-000000000001";
        let name = format!("rollout-2026-10-13T04-34-15-{id}.jsonl");
        assert_eq!(thread_id(&name), Some(id));
        assert_eq!(
            thread_id("rollout-2026-10-13T04-34-15-aaaa_bbbb.jsonl.zst"),
            Some("aaaa")
        );
        assert_eq!(thread_id("inline.jsonl"), None);
    }
}
