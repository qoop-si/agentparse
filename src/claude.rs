//! Claude Code transcripts: `~/.claude/projects/<dir>/<session>.jsonl`, plus subagent files under
//! `<session>/subagents/` (and `subagents/workflows/<run>/`).
//!
//! The CLI writes one line per content block of an API message; lines are regrouped by
//! `message.id` and the usage of the last line of an id is the message usage. Tool results ride
//! on `user` lines and fold into their call (a result written before its call waits for it).
//! Metadata lines without a typed home stay as [`EntryKind::Other`]. A line repeating the uuid of
//! an earlier line of the same file is a history copy and is skipped; copies across files (resumed
//! sessions) are the caller's to dedupe, so `Session.usage` of a resumed file includes them.

use crate::util::{
    Obj, as_extra, base64_len, bool_field, join_texts, json_lines, str_field, stripped, take_array,
    take_bool, take_obj, take_str, take_u64, take_with, ts_ms,
};
use crate::{
    Agent, Compaction, Entry, EntryKind, Error, ErrorInfo, Git, Hook, Media, Message, Notice,
    Origin, Other, ParentLink, Part, ProcessAction, Result, Role, Session, Settings, Shell,
    ShellResult, Source, Tool, ToolCall, ToolOutput, ToolResult, ToolStatus, Turn, Usage, Warning,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::BufRead;
use std::path::Path;

mod tools;

use tools::{apply_notification, launch_summary, outcome, strip_line_numbers, task_id, tool_call};

/// Keys every conversation line repeats. Their first value belongs to the session; a line keeps
/// one in its `extra` only when its value differs.
const ENVELOPE: &[&str] = &[
    "sessionId",
    "session_id",
    "version",
    "gitBranch",
    "isSidechain",
    "agentId",
    "userType",
    "entrypoint",
    "slug",
    "teamName",
    "agentName",
];

/// The one envelope key metadata lines carry.
const METADATA_ENVELOPE: &[&str] = &["sessionId"];

/// Leading tags of user text that a slash command or a `!cmd` shell escape wrote.
const COMMAND_TAGS: &[&str] = &[
    "command-name",
    "command-message",
    "command-args",
    "local-command-stdout",
    "local-command-stderr",
    "bash-input",
    "bash-stdout",
    "bash-stderr",
];

/// How far apart a typed slash command and its expanded line can be written.
const TYPED_GAP_MS: i64 = 10_000;

/// How long before its `agents_killed` record the harness queues the words of the kill.
const KILL_GAP_MS: i64 = 1_000;

const INTERRUPTED: &str = "[Request interrupted by user";
const CONTINUED: &str = "This session is being continued from a previous conversation";
/// The line Claude Code puts before a teammate's message it hands over as a prompt, with the
/// `<teammate-message>` under it.
const PEER_SENT: &str = "Another Claude session sent a message:";

/// Where an attachment keeps its text when no `rendered` copy was recorded.
const ATTACHMENT_TEXT: &[&str] = &[
    "/text",
    "/content",
    "/snippet",
    "/banner",
    "/planContent",
    "/content/file/content",
    "/content/content",
];

/// Parse a Claude Code transcript from any slice of its lines.
pub(crate) fn parse_reader(reader: impl BufRead, source: Source) -> Result<Session> {
    let mut p = Parser::default();
    for item in json_lines(reader) {
        let (line, parsed) = item?;
        match parsed {
            Ok(Value::Object(rec)) => p.record(line, rec),
            Ok(_) => p.warn(Some(line), "line is not a JSON object".into()),
            Err(e) => p.warn(Some(line), format!("invalid JSON: {e}")),
        }
    }
    p.finish(source)
}

/// The keys every line kind shares, taken off the record first.
#[derive(Clone)]
struct Head {
    line: usize,
    uuid: Option<String>,
    parent: Option<String>,
    ts: Option<i64>,
}

#[derive(Default)]
struct Parser {
    entries: Vec<Entry>,
    warnings: Vec<Warning>,
    /// First value of each [`ENVELOPE`] key.
    env: Obj,
    first_cwd: Option<String>,
    /// Settings as last seen, to emit `Settings` on change only.
    current: Settings,
    /// Last `promptId`, carried forward.
    turn: Option<String>,
    /// Line uuid to the id of the entry that absorbed that line.
    uuids: HashMap<String, String>,
    /// Every line uuid read so far, to skip history copies.
    seen: HashSet<String>,
    /// Lines skipped as history copies.
    copies: Vec<usize>,
    /// Results whose call was not read yet, by call id.
    pending: HashMap<String, Held>,
    /// Results that will never meet their call: a second result for a waiting id.
    orphans: Vec<(String, Held)>,
    /// API `message.id` to entry index.
    messages: HashMap<String, usize>,
    /// `tool_use` id to (entry index, part index).
    calls: HashMap<String, (usize, usize)>,
    /// Background task id (shell, agent, monitor, workflow) to the call that launched it; a named
    /// agent (a teammate) is here under its name too.
    tasks: HashMap<String, (usize, usize)>,
    /// Task list number to its subject, from `Task #N created successfully: <subject>`.
    todos: HashMap<String, String>,
    /// `SubagentHandback` call id to the entry its report became the text of.
    handbacks: HashMap<String, usize>,
    /// Latest title per line type (`custom-title`, `agent-name`, `ai-title`, `summary`).
    titles: Obj,
    cost: Option<Obj>,
    /// Human input queued and not taken yet: (index of its `queue-operation` entry, text).
    queue: VecDeque<(usize, String)>,
    /// A `compact_boundary` waiting for its summary line.
    compaction: Option<usize>,
    /// A `<bash-input>` command waiting for its output line.
    shell_escape: Option<usize>,
    /// Records other than workflow journal lines (`started`, `result`, `launched`).
    records: usize,
    /// The current line is a sidechain line (subagent files, or inline in old main files).
    sidechain: bool,
    /// Old `summary` lines: (leaf uuid, text). A summary titles this session only when its leaf
    /// is a line of this file; the others summarize earlier sessions.
    summaries: Vec<(String, String)>,
}

impl Parser {
    fn warn(&mut self, line: Option<usize>, message: String) {
        self.warnings.push(Warning { line, message });
    }

    fn record(&mut self, line: usize, mut rec: Obj) {
        let kind = take_str(&mut rec, "type").unwrap_or_default();
        let head = Head {
            line,
            uuid: take_str(&mut rec, "uuid"),
            parent: take_str(&mut rec, "parentUuid"),
            ts: take_with(&mut rec, "timestamp", ts_ms),
        };
        if let Some(uuid) = &head.uuid {
            if !self.seen.insert(uuid.clone()) {
                self.copies.push(line);
                return;
            }
        }
        if let Some(prompt) = take_str(&mut rec, "promptId") {
            self.turn = Some(prompt);
        }
        let conversation = matches!(
            kind.as_str(),
            "assistant" | "user" | "attachment" | "system"
        );
        self.sidechain = rec
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.envelope(&head, &mut rec, conversation);
        if !matches!(kind.as_str(), "started" | "result" | "launched") {
            self.records += 1;
        }
        match kind.as_str() {
            "assistant" => self.assistant(head, rec),
            "user" => self.user(head, rec),
            "attachment" => self.attachment(head, rec),
            "system" => self.system(head, rec),
            "custom-title" | "agent-name" | "ai-title" => self.title(&kind, rec),
            "summary" => {
                let leaf = rec.get("leafUuid").and_then(Value::as_str);
                if let (Some(leaf), Some(text)) = (leaf, rec.get("summary").and_then(Value::as_str))
                {
                    self.summaries.push((leaf.to_string(), text.to_string()));
                }
                self.other(head, kind, rec)
            }
            "permission-mode" => match take_str(&mut rec, "permissionMode") {
                Some(mode) => self.setting(&head, |s| &mut s.approval, mode),
                None => self.other(head, kind, rec),
            },
            "agent-setting" => match take_str(&mut rec, "agentSetting") {
                Some(agent) => self.setting(&head, |s| &mut s.agent, agent),
                None => self.other(head, kind, rec),
            },
            "cost-state" => self.cost = Some(rec),
            "queue-operation" => self.queue_op(head, rec),
            _ => self.other(head, kind, rec),
        }
    }

    /// Move envelope keys to the session; the first `cwd` and every change of it is a `Settings`
    /// entry.
    fn envelope(&mut self, head: &Head, rec: &mut Obj, conversation: bool) {
        if conversation {
            if let Some(cwd) = take_str(rec, "cwd") {
                self.first_cwd.get_or_insert_with(|| cwd.clone());
                self.setting(head, |s| &mut s.cwd, cwd);
            }
        }
        let keys = if conversation {
            ENVELOPE
        } else {
            METADATA_ENVELOPE
        };
        for &key in keys {
            let Some(v) = rec.remove(key) else { continue };
            if key == "session_id" && self.env.get("sessionId") == Some(&v) {
                continue;
            }
            match self.env.get(key) {
                None => {
                    self.env.insert(key.into(), v);
                }
                Some(first) if *first == v => {}
                Some(_) => {
                    rec.insert(key.into(), v);
                }
            }
        }
    }

    /// Emit `Settings` for the first value and on every change.
    fn setting(
        &mut self,
        head: &Head,
        field: fn(&mut Settings) -> &mut Option<String>,
        value: String,
    ) {
        let current = field(&mut self.current);
        if current.as_deref() == Some(value.as_str()) {
            return;
        }
        *current = Some(value.clone());
        let mut changed = Settings::default();
        *field(&mut changed) = Some(value);
        let head = Head {
            uuid: None,
            parent: None,
            ..head.clone()
        };
        self.push(&head, None, None, EntryKind::Settings(changed));
    }

    fn push(
        &mut self,
        head: &Head,
        id: Option<String>,
        extra: Option<Obj>,
        kind: EntryKind,
    ) -> usize {
        let parent_id = head.parent.as_ref().map(|p| self.resolve(p));
        if let (Some(uuid), Some(id)) = (&head.uuid, &id) {
            self.uuids.entry(uuid.clone()).or_insert_with(|| id.clone());
        }
        self.entries.push(Entry {
            id,
            parent_id,
            turn_id: self.turn.clone(),
            ts: head.ts,
            line: Some(head.line),
            extra: extra.map(Value::Object),
            kind,
        });
        self.entries.len() - 1
    }

    /// A line uuid to the entry that absorbed it; an unknown uuid is kept as recorded.
    fn resolve(&self, uuid: &str) -> String {
        self.uuids
            .get(uuid)
            .cloned()
            .unwrap_or_else(|| uuid.to_string())
    }

    fn other(&mut self, head: Head, kind: String, rec: Obj) {
        let id = head.uuid.clone();
        let data = stripped(Value::Object(rec));
        self.push(&head, id, None, EntryKind::Other(Other { kind, data }));
    }

    fn title(&mut self, kind: &str, mut rec: Obj) {
        let key = match kind {
            "custom-title" => "customTitle",
            "agent-name" => "agentName",
            _ => "aiTitle",
        };
        if let Some(title) = take_str(&mut rec, key).filter(|t| !t.trim().is_empty()) {
            self.titles.insert(kind.into(), Value::String(title));
        }
    }

    /// Replay the human input queue FIFO: what is still queued at the end was never sent.
    fn queue_op(&mut self, head: Head, rec: Obj) {
        let content = rec.get("content").and_then(Value::as_str);
        match (rec.get("operation").and_then(Value::as_str), content) {
            // Notifications (`<...>`) are queued too but delivered another way, without a
            // dequeue; only human text enters the FIFO.
            (Some("enqueue"), Some(text)) if is_human_text(text) => {
                self.queue.push_back((self.entries.len(), text.to_string()));
            }
            // A `remove` without content takes the front, like a dequeue (measured: the
            // `queued_command` that follows it carries the front text).
            (Some("dequeue"), _) | (Some("remove"), None) => {
                self.queue.pop_front();
            }
            (Some("remove"), Some(content)) => {
                if let Some(at) = self.queue.iter().position(|(_, text)| text == content) {
                    self.queue.remove(at);
                }
            }
            (Some("popAll"), _) => self.queue.clear(),
            _ => {}
        }
        self.other(head, "queue-operation".into(), rec);
    }

    fn assistant(&mut self, head: Head, mut rec: Obj) {
        let mut msg = take_obj(&mut rec, "message").unwrap_or_default();
        let api_error = take_bool(&mut rec, "isApiErrorMessage");
        if api_error || msg.get("model").and_then(Value::as_str) == Some("<synthetic>") {
            return self.synthetic(head, rec, msg, api_error);
        }
        if let Some(effort) = take_str(&mut rec, "effort") {
            self.setting(&head, |s| &mut s.effort, effort);
        }
        // The block's position in the message, which part order already says.
        rec.remove("apiBlockIndex");
        let id = take_str(&mut msg, "id")
            .or_else(|| head.uuid.clone())
            .unwrap_or_default();
        let model = take_str(&mut msg, "model");
        let stop = take_str(&mut msg, "stop_reason");
        let usage = take_obj(&mut msg, "usage").map(usage_of);
        let cwd = self.current.cwd.clone();
        let parts: Vec<(Part, Obj)> = take_array(&mut msg, "content")
            .into_iter()
            .map(|b| part(b, cwd.as_deref(), head.ts))
            .collect();
        drop_eq(&mut msg, "type", "message");
        drop_eq(&mut msg, "role", "assistant");
        if let Some(model) = &model {
            drop_eq(&mut rec, "requestedModel", model);
        }
        dedupe_wire_inputs(&mut rec, &parts);

        let idx = match self.messages.get(&id).copied() {
            Some(idx) if !self.user_line_after(idx) => {
                self.continuation(idx, &head, &parts);
                idx
            }
            earlier => {
                // A message that goes on after a user line continues as a piece of its own
                // (`<id>#2`, ...) below that line; the usage moves on with it, so it counts once.
                let (entry_id, usage) = match earlier {
                    Some(idx) => self.split_off(idx, &id),
                    None => (id.clone(), None),
                };
                let msg = Message {
                    usage,
                    ..Message::new(Role::Assistant, Origin::Model, Vec::new())
                };
                let idx = self.push(&head, Some(entry_id), None, EntryKind::Message(msg));
                self.messages.insert(id.clone(), idx);
                idx
            }
        };
        let Entry {
            id: entry_id,
            kind: EntryKind::Message(m),
            extra,
            ..
        } = &mut self.entries[idx]
        else {
            return;
        };
        let mut calls = Vec::new();
        for (mut p, rest) in parts {
            if let Part::ToolCall(c) = &mut p {
                // A subagent's report handed back to its caller is its last word, not a call.
                if let ("SubagentHandback", Some(text)) =
                    (c.name.as_str(), str_field(&c.input, &["message"]))
                {
                    self.handbacks.insert(c.id.clone(), idx);
                    m.parts.push(Part::Text { text });
                    continue;
                }
                if let (Tool::Todo { items }, "TaskUpdate") = (&mut c.tool, c.name.as_str()) {
                    for item in items {
                        let n = item.content.strip_prefix('#');
                        if let Some(subject) = n.and_then(|n| self.todos.get(n)) {
                            item.content = subject.clone();
                        }
                    }
                }
                self.calls.insert(c.id.clone(), (idx, m.parts.len()));
                calls.push((c.id.clone(), rest));
            }
            m.parts.push(p);
        }
        if m.model.is_none() {
            m.model = model;
        }
        if stop.is_some() {
            m.stop_reason = stop;
        }
        if let Some((u, rest)) = usage {
            m.usage = Some(u);
            msg.insert("usage".into(), Value::Object(rest));
        }
        let extra = extra_mut(extra);
        push_uuid(extra, &head.uuid);
        merge(extra, rec);
        merge(
            extra,
            Obj::from_iter([("message".to_string(), Value::Object(msg))]),
        );
        for (call_id, rest) in &mut calls {
            call_extra(extra, call_id, "call", std::mem::take(rest));
        }
        if let (Some(uuid), Some(entry_id)) = (head.uuid, entry_id.clone()) {
            self.uuids.entry(uuid).or_insert(entry_id);
        }
        for (call_id, _) in calls {
            if let Some(held) = self.pending.remove(&call_id) {
                self.fold(call_id, held);
            }
        }
    }

    /// A context line (a skill body, an image note: part of a call's result) was written after
    /// the message at `idx` began. Not a teammate's message: written while one response still
    /// streams, it reaches the model only with the next request (fefc1886 lines 1125-1127).
    fn user_line_after(&self, idx: usize) -> bool {
        self.entries[idx + 1..].iter().any(|e| {
            matches!(&e.kind, EntryKind::Message(m) if m.role == Role::User && m.origin == Origin::Context)
        })
    }

    /// The id of the next piece of message `id`, whose latest piece is at `idx`, and the usage
    /// taken off that piece.
    fn split_off(&mut self, idx: usize, id: &str) -> (String, Option<Usage>) {
        let entry = &mut self.entries[idx];
        let n = (entry.id.as_deref())
            .and_then(|e| e.strip_prefix(id)?.strip_prefix('#')?.parse::<usize>().ok())
            .unwrap_or(1);
        let usage = match &mut entry.kind {
            EntryKind::Message(m) => m.usage.take(),
            _ => None,
        };
        (format!("{id}#{}", n + 1), usage)
    }

    /// A later line of a merged message keeps its own parent and timestamp in
    /// `extra.lines.<uuid>` when they say something the entry does not: a parent other than the
    /// message or its parent, a timestamp other than the entry's (a `tool_use` line's timestamp is
    /// its call's `started_at` already).
    fn continuation(&mut self, idx: usize, head: &Head, parts: &[(Part, Obj)]) {
        let parent = head.parent.as_ref().map(|p| self.resolve(p));
        let Entry {
            id,
            parent_id,
            ts,
            extra,
            ..
        } = &mut self.entries[idx];
        let mut line = Obj::new();
        if parent.is_some() && parent != *id && parent != *parent_id {
            line.insert("parentUuid".into(), head.parent.clone().into());
        }
        let call_line = parts.iter().any(|(p, _)| matches!(p, Part::ToolCall(_)));
        if head.ts != *ts && !call_line {
            line.extend(head.ts.map(|t| ("ts".to_string(), Value::from(t))));
        }
        if let (false, Some(uuid)) = (line.is_empty(), &head.uuid) {
            let lines = extra_mut(extra)
                .entry("lines")
                .or_insert_with(|| Value::Object(Obj::new()));
            if let Value::Object(lines) = lines {
                lines.insert(uuid.clone(), Value::Object(line));
            }
        }
    }

    /// A message the CLI wrote in the model's place: an API error, or a filler such as
    /// `No response requested.`.
    fn synthetic(&mut self, head: Head, mut rec: Obj, mut msg: Obj, api_error: bool) {
        let text = content_text(&msg.remove("content").unwrap_or(Value::Null));
        drop_eq(&mut msg, "type", "message");
        drop_eq(&mut msg, "role", "assistant");
        if !msg.is_empty() {
            rec.insert("message".into(), Value::Object(msg));
        }
        let kind = if api_error {
            EntryKind::Error(ErrorInfo {
                kind: take_str(&mut rec, "error"),
                message: text,
                status: take_u64(&mut rec, "apiErrorStatus").and_then(|s| u16::try_from(s).ok()),
                retry: None,
            })
        } else {
            EntryKind::Notice(Notice {
                level: None,
                kind: Some("synthetic".into()),
                text,
            })
        };
        let id = head.uuid.clone();
        self.push(&head, id, Some(rec), kind);
    }

    fn user(&mut self, head: Head, mut rec: Obj) {
        let mut msg = take_obj(&mut rec, "message").unwrap_or_default();
        drop_eq(&mut msg, "role", "user");
        let content = msg.remove("content").unwrap_or(Value::Null);
        if !msg.is_empty() {
            rec.insert("message".into(), Value::Object(msg));
        }
        if let Some(mode) = take_str(&mut rec, "permissionMode") {
            self.setting(&head, |s| &mut s.approval, mode);
        }
        if take_bool(&mut rec, "isCompactSummary") {
            return self.compact_summary(head, rec, content_text(&content));
        }
        let blocks = match content {
            Value::String(text) => vec![json!({"type": "text", "text": text})],
            Value::Array(blocks) => blocks,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        let (results, blocks): (Vec<Value>, Vec<Value>) = blocks
            .into_iter()
            .partition(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"));
        if !results.is_empty() {
            let only_results = blocks.is_empty();
            self.results(&head, &mut rec, results, only_results);
            if only_results {
                return;
            }
        }
        if self.shell_escape(&head, &mut rec, &blocks) {
            return;
        }
        self.shell_escape = None;
        self.user_message(head, rec, blocks);
    }

    /// Fold results into their calls. A line made only of results goes with its first result
    /// (`absorb`); a result whose call is not read yet waits for it in `pending`.
    fn results(&mut self, head: &Head, rec: &mut Obj, results: Vec<Value>, absorb: bool) {
        // The line-level result keys describe the one result such a line carries.
        let single = results.len() == 1;
        let mut tur = if single {
            rec.remove("toolUseResult")
        } else {
            None
        };
        let mut denial = if single {
            take_str(rec, "toolDenialKind")
        } else {
            None
        };
        if rec.get("sourceToolAssistantUUID").and_then(Value::as_str) == head.parent.as_deref() {
            rec.remove("sourceToolAssistantUUID");
        }
        let mut rest = absorb.then(|| std::mem::take(rec));
        for block in results {
            let Value::Object(mut block) = block else {
                continue;
            };
            let call_id = take_str(&mut block, "tool_use_id").unwrap_or_default();
            // The ack of a hand-back says only that it was delivered.
            if let Some(&entry) = self.handbacks.get(&call_id) {
                if let Some(rest) = rest.take() {
                    self.absorb_result_line(head, rest, entry, &call_id);
                }
                continue;
            }
            let held = Held {
                head: head.clone(),
                turn: self.turn.clone(),
                block,
                tur: tur.take(),
                denial: denial.take(),
                rest: rest.take(),
            };
            if self.calls.contains_key(&call_id) {
                self.fold(call_id, held);
            } else if let Some(older) = self.pending.insert(call_id.clone(), held) {
                self.orphans.push((call_id, older));
            }
        }
    }

    /// Put a result into its read call; its line, when made only of results, is absorbed too.
    fn fold(&mut self, call_id: String, held: Held) {
        let Some(&(entry, part)) = self.calls.get(&call_id) else {
            return;
        };
        let Held {
            head,
            block,
            tur,
            denial,
            rest,
            ..
        } = held;
        let mut launched = Vec::new();
        let mut stopped = None;
        if let Some(call) = call_at(&mut self.entries[entry], part) {
            let o = outcome(&call.name, &call.input, block, tur, denial);
            call.status = o.status;
            call.status_detail = o.detail;
            call.output = Some(o.output);
            // A launched task has not ended: its notification, a stop or a status line ends it.
            call.ended_at = (o.status != ToolStatus::Pending)
                .then_some(head.ts)
                .flatten();
            if o.child.is_some() {
                call.child_session = o.child;
            }
            launch_summary(call);
            if call.status == ToolStatus::Pending {
                launched.extend(task_id(call).map(String::from));
                // A TaskStop names a teammate (or a named background agent) by its name too.
                if let Tool::Task {
                    name: Some(name), ..
                } = &call.tool
                {
                    launched.push(name.clone());
                }
            }
            if call.name == "TaskCreate" {
                let created = call.output.as_ref().and_then(|o| created_task(&o.text));
                self.todos.extend(created);
            }
            if let (
                Tool::Process {
                    id: Some(id),
                    action: ProcessAction::Kill,
                    ..
                },
                ToolStatus::Ok,
            ) = (&call.tool, call.status)
            {
                stopped = Some(id.clone());
            }
        }
        for id in launched {
            self.tasks.insert(id, (entry, part));
        }
        if let Some(id) = stopped {
            self.end_task(&id, "stopped", head.ts);
        }
        if let Some(rest) = rest {
            self.absorb_result_line(&head, rest, entry, &call_id);
        }
    }

    /// A line made only of a result: its uuid goes to the entry holding the call, its leftover
    /// keys to `extra.calls.<call_id>.output`.
    fn absorb_result_line(&mut self, head: &Head, mut rest: Obj, entry: usize, call_id: &str) {
        let parent = head.parent.as_ref().map(|p| self.resolve(p));
        let Entry { id, extra, .. } = &mut self.entries[entry];
        if parent.is_some() && parent != *id {
            rest.insert("parentUuid".into(), head.parent.clone().into());
        }
        let extra = extra_mut(extra);
        push_uuid(extra, &head.uuid);
        call_extra(extra, call_id, "output", rest);
        if let (Some(uuid), Some(id)) = (&head.uuid, id.clone()) {
            self.uuids.entry(uuid.clone()).or_insert(id);
        }
    }

    fn compact_summary(&mut self, head: Head, rec: Obj, summary: String) {
        let Some(at) = self.compaction.take() else {
            let id = head.uuid.clone();
            let kind = EntryKind::Compaction(Compaction {
                summary: Some(summary),
                ..Default::default()
            });
            self.push(&head, id, Some(rec), kind);
            return;
        };
        let Entry {
            id, extra, kind, ..
        } = &mut self.entries[at];
        if let EntryKind::Compaction(c) = kind {
            c.summary = Some(summary);
        }
        let extra = extra_mut(extra);
        push_uuid(extra, id);
        push_uuid(extra, &head.uuid);
        merge(extra, rec);
        if let (Some(uuid), Some(id)) = (head.uuid, id.clone()) {
            self.uuids.entry(uuid).or_insert(id);
        }
    }

    /// `<bash-input>` opens a Command message with one Shell call; the next
    /// `<bash-stdout>`/`<bash-stderr>` line is its result.
    fn shell_escape(&mut self, head: &Head, rec: &mut Obj, blocks: &[Value]) -> bool {
        let [block] = blocks else { return false };
        let Some(text) = block.get("text").and_then(Value::as_str) else {
            return false;
        };
        match leading_tag(text) {
            Some("bash-input") => {
                let command = tag_body(text, "bash-input").unwrap_or_default().to_string();
                let input = Value::String(command.clone());
                let cwd = self.current.cwd.clone();
                let shell = Tool::Shell(Shell {
                    command,
                    cwd,
                    ..Default::default()
                });
                let id = head.uuid.clone().unwrap_or_default();
                let call = ToolCall {
                    started_at: head.ts,
                    ..ToolCall::new(id, "bash-input".into(), input, shell)
                };
                let msg = Message {
                    kind: Some("bash-input".into()),
                    ..Message::new(Role::User, Origin::Command, vec![Part::ToolCall(call)])
                };
                let id = head.uuid.clone();
                let at = self.push(head, id, Some(std::mem::take(rec)), EntryKind::Message(msg));
                self.shell_escape = Some(at);
                true
            }
            Some("bash-stdout" | "bash-stderr") if self.shell_escape.is_some() => {
                let at = self.shell_escape.take().unwrap_or_default();
                let Entry {
                    id,
                    extra,
                    kind: EntryKind::Message(m),
                    ..
                } = &mut self.entries[at]
                else {
                    return false;
                };
                if let Some(Part::ToolCall(call)) = m.parts.first_mut() {
                    let shell = ShellResult {
                        stdout: tag_body(text, "bash-stdout").map(String::from),
                        stderr: tag_body(text, "bash-stderr")
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                        ..Default::default()
                    };
                    call.output = Some(ToolOutput {
                        text: text.to_string(),
                        shell: Some(shell),
                        ..Default::default()
                    });
                    call.status = ToolStatus::Ok;
                    call.ended_at = head.ts;
                }
                let extra = extra_mut(extra);
                push_uuid(extra, id);
                push_uuid(extra, &head.uuid);
                merge(extra, std::mem::take(rec));
                if let (Some(uuid), Some(id)) = (&head.uuid, id.clone()) {
                    self.uuids.entry(uuid.clone()).or_insert(id);
                }
                true
            }
            _ => false,
        }
    }

    /// A user line that is not a tool result: one message per run of blocks of the same kind,
    /// ids suffixed `#1`, `#2` when the line splits; interrupt markers become `Turn`.
    fn user_message(&mut self, head: Head, mut rec: Obj, blocks: Vec<Value>) {
        let ctx = self.ctx(&mut rec);
        let mut groups: Vec<(Class, Vec<Part>)> = Vec::new();
        for block in blocks {
            let class = match block.get("text").and_then(Value::as_str) {
                Some(text) if block.get("type").and_then(Value::as_str) == Some("text") => {
                    classify(text, &ctx)
                }
                _ => groups
                    .last()
                    .map_or_else(|| classify("", &ctx), |g| g.0.clone()),
            };
            let (part, _) = part(block, None, head.ts);
            match groups.last_mut() {
                Some(group) if group.0 == class => group.1.push(part),
                _ => groups.push((class, vec![part])),
            }
        }
        if groups.is_empty() {
            groups.push((classify("", &ctx), Vec::new()));
        }
        let split = groups.len() > 1;
        let mut rest = Some(rec);
        for (i, (class, parts)) in groups.into_iter().enumerate() {
            let id = head.uuid.as_ref().map(|u| {
                if split {
                    format!("{u}#{}", i + 1)
                } else {
                    u.clone()
                }
            });
            // `/compact` typed in the prompt is already a bubble: this expansion of it adds nothing.
            if let Some(typed) = self.typed_command(&class, &parts, head.ts) {
                if let (Some(uuid), Some(id)) = (&head.uuid, self.entries[typed].id.clone()) {
                    self.uuids.entry(uuid.clone()).or_insert(id);
                }
                continue;
            }
            let mut extra = rest.take();
            let kind = match class {
                Class::Interrupt => {
                    let text = join_texts(&parts, "\n");
                    extra
                        .get_or_insert_default()
                        .insert("marker".into(), Value::String(text));
                    EntryKind::Turn(Turn {
                        status: Some("interrupted".into()),
                        ..Default::default()
                    })
                }
                Class::Message(origin, kind, author) => EntryKind::Message(Message {
                    kind,
                    author,
                    ..Message::new(Role::User, origin, parts)
                }),
            };
            let at = self.push(&head, id, extra, kind);
            self.enrich(at);
        }
    }

    /// The human message just before a `<command-name>` line that holds the same text: Claude
    /// writes a typed slash command as a prompt and again expanded, the same moment (a compaction
    /// in between can start a new `promptId`).
    fn typed_command(&self, class: &Class, parts: &[Part], ts: Option<i64>) -> Option<usize> {
        let Class::Message(Origin::Command, Some(tag), _) = class else {
            return None;
        };
        let text = join_texts(parts, "\n");
        let name = tag_body(&text, "command-name").filter(|_| tag == "command-name")?;
        let args = tag_body(&text, "command-args").unwrap_or_default();
        let human =
            |e: &Entry| matches!(&e.kind, EntryKind::Message(m) if m.origin == Origin::Human);
        let (at, prev) = self
            .entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| human(e))?;
        let EntryKind::Message(m) = &prev.kind else {
            return None;
        };
        let close = ts.zip(prev.ts).is_none_or(|(a, b)| a - b <= TYPED_GAP_MS);
        (close && m.text().trim() == format!("{name} {args}").trim()).then_some(at)
    }

    fn ctx(&self, rec: &mut Obj) -> Ctx {
        let meta = take_bool(rec, "isMeta");
        let (kind, author) = take_origin(rec);
        Ctx {
            meta,
            origin: kind,
            author,
            sidechain: self.sidechain,
        }
    }

    /// A background task notification updates the call it reports on: by `<tool-use-id>`, or by
    /// `<task-id>` when the notification carries no call id (Monitor events, agent reports).
    fn enrich(&mut self, at: usize) {
        let Entry {
            kind: EntryKind::Message(m),
            ts,
            ..
        } = &self.entries[at]
        else {
            return;
        };
        let (text, ts) = (m.text(), *ts);
        if m.origin == Origin::Agent {
            self.idle_pings(&text);
            return;
        }
        if m.origin != Origin::Notification {
            return;
        }
        for note in text.split("<task-notification>").skip(1) {
            let by_call = tag_body(note, "tool-use-id").and_then(|id| self.calls.get(id));
            let by_task = tag_body(note, "task-id").and_then(|id| self.tasks.get(id));
            let Some(&(entry, part)) = by_call.or(by_task) else {
                continue;
            };
            if let Some(call) = call_at(&mut self.entries[entry], part) {
                apply_notification(call, note, ts);
            }
        }
    }

    /// A teammate that went idle still runs (the next message wakes it): its spawn says `idle`
    /// until the teammate sends anything else.
    fn idle_pings(&mut self, text: &str) {
        for note in text.split("<teammate-message").skip(1) {
            let idle = note
                .split("</teammate-message>")
                .next()
                .is_some_and(|body| body.contains("\"type\":\"idle_notification\""));
            let Some(&(entry, part)) =
                attr(note, "teammate_id").and_then(|name| self.tasks.get(&name))
            else {
                continue;
            };
            let call = call_at(&mut self.entries[entry], part);
            if let Some(call) = call.filter(|c| c.status == ToolStatus::Pending) {
                if idle {
                    call.status_detail = Some("idle".into());
                } else if call.status_detail.as_deref() == Some("idle") {
                    call.status_detail = None;
                }
            }
        }
    }

    /// End the launched task `id` that is still running: `status` is a word of
    /// [`apply_notification`].
    fn end_task(&mut self, id: &str, status: &str, ts: Option<i64>) {
        let Some(&(entry, part)) = self.tasks.get(id) else {
            return;
        };
        if let Some(call) = call_at(&mut self.entries[entry], part) {
            if call.status == ToolStatus::Pending {
                apply_notification(call, &format!("<status>{status}</status>"), ts);
            }
        }
    }

    /// End every launched agent (or workflow) still running, when the harness says none is left
    /// (`ended`) or the user stopped them all (`killed`). Teammates are not in the count, so an
    /// empty count leaves them running; a kill stops them too (6332d703 line 8301 names three).
    fn end_agents(&mut self, workflows: bool, status: &str, ts: Option<i64>) {
        for &(entry, part) in self.tasks.values() {
            let Some(call) = call_at(&mut self.entries[entry], part) else {
                continue;
            };
            let teammate = (call.output.as_ref())
                .and_then(|o| o.data.as_ref()?.get("status"))
                .is_some_and(|s| s == "teammate_spawned");
            let agent = matches!(call.tool, Tool::Task { .. }) && !(teammate && status == "ended");
            if agent && (call.name == "Workflow") == workflows && call.status == ToolStatus::Pending
            {
                apply_notification(call, &format!("<status>{status}</status>"), ts);
            }
        }
    }

    /// A Monitor outlives its `timeout_ms` without a word: once the file holds a record past
    /// that time the watch is over.
    fn expire_monitors(&mut self) {
        let Some(latest) = self.entries.iter().filter_map(|e| e.ts).max() else {
            return;
        };
        for &(entry, part) in self.tasks.values() {
            let Some(call) = call_at(&mut self.entries[entry], part) else {
                continue;
            };
            let (Tool::Shell(shell), Some(start)) = (&call.tool, call.started_at) else {
                continue;
            };
            // A persistent Monitor ignores its timeout and runs until stopped or the session ends.
            let end = shell
                .timeout_ms
                .filter(|_| !bool_field(&call.input, &["persistent"]))
                .map(|t| start.saturating_add(t as i64));
            if let (true, Some(end)) = (
                call.name == "Monitor" && call.status == ToolStatus::Pending,
                end,
            ) {
                if end <= latest {
                    apply_notification(call, "<status>expired</status>", Some(end));
                }
            }
        }
    }

    fn attachment(&mut self, head: Head, mut rec: Obj) {
        let mut a = take_obj(&mut rec, "attachment").unwrap_or_default();
        let sub = take_str(&mut a, "type").unwrap_or_default();
        if sub == "task_status" {
            let field = |k: &str| a.get(k).and_then(Value::as_str).map(String::from);
            if let (Some(id), Some(status)) = (field("taskId"), field("status")) {
                self.end_task(&id, &status, head.ts);
            }
        }
        match sub.as_str() {
            "queued_command" => self.queued(head, rec, a),
            "hook_additional_context" => self.context(head, rec, a, sub),
            s if s.starts_with("hook_") || s == "async_hook_response" => {
                self.hook(head, rec, a, sub)
            }
            _ => match attachment_line(&sub, &a) {
                Some(text) => self.line(head, rec, a, sub, text),
                None => self.context(head, rec, a, sub),
            },
        }
    }

    /// An attachment the UI shows as one line (`Read a.rs (12 lines)`): a notice with that line,
    /// the record kept as recorded. The `rendered` copy goes when it only wraps the body.
    fn line(&mut self, head: Head, mut rec: Obj, a: Obj, sub: String, text: String) {
        let a = Value::Object(a);
        let rendered = rendered_text(rec.get("rendered"));
        let wraps = |body: String| {
            rendered
                .as_deref()
                .is_some_and(|r| r.contains(&body) || strip_line_numbers(r).contains(&body))
        };
        if attachment_text(&a).is_some_and(|(_, body)| wraps(body)) {
            rec.remove("rendered");
            rec.remove("renderedRole");
        }
        rec.insert("attachment".into(), a);
        let notice = Notice {
            level: None,
            kind: Some(sub),
            text,
        };
        let id = head.uuid.clone();
        self.push(&head, id, Some(rec), EntryKind::Notice(notice));
    }

    /// Input typed (or delivered) while the agent was busy and folded into the running turn.
    fn queued(&mut self, head: Head, mut rec: Obj, mut a: Obj) {
        let (kind, author) = take_origin(&mut a);
        // Old deliveries carry no `origin`; their `commandMode` names the kind (`prompt` is plain input).
        let mode = a
            .get("commandMode")
            .and_then(Value::as_str)
            .filter(|m| *m != "prompt");
        let mode = mode.map(String::from);
        let meta = take_bool(&mut a, "isMeta");
        let prompt = a.remove("prompt").unwrap_or(Value::Null);
        let human = !meta
            && match kind.as_deref() {
                Some(k) => k == "human",
                None => {
                    let text = content_text(&prompt);
                    !text.trim_start().starts_with('<') && !is_harness_text(&text)
                }
            };
        let parts = match prompt {
            Value::String(text) => vec![Part::Text { text }],
            Value::Array(blocks) => blocks
                .into_iter()
                .map(|b| part(b, None, head.ts).0)
                .collect(),
            _ => Vec::new(),
        };
        let msg = if human {
            Message::new(Role::User, Origin::Human, parts)
        } else {
            let kind = kind.or_else(|| {
                mode.inspect(|_| {
                    a.remove("commandMode");
                })
            });
            let origin = kind.as_deref().map_or(Origin::Notification, origin_of);
            let kind = Some(kind.unwrap_or_else(|| "queued_command".into()));
            Message {
                kind,
                author,
                ..Message::new(Role::User, origin, parts)
            }
        };
        if !a.is_empty() {
            rec.insert("attachment".into(), Value::Object(a));
        }
        let id = head.uuid.clone();
        let at = self.push(&head, id, Some(rec), EntryKind::Message(msg));
        self.enrich(at);
    }

    fn hook(&mut self, head: Head, mut rec: Obj, mut a: Obj, sub: String) {
        let mut hook = Hook {
            event: take_str(&mut a, "hookEvent"),
            name: take_str(&mut a, "hookName"),
            command: take_str(&mut a, "command"),
            exit_code: take_u64(&mut a, "exitCode").and_then(|c| i32::try_from(c).ok()),
            stdout: take_str(&mut a, "stdout").filter(|s| !s.is_empty()),
            stderr: take_str(&mut a, "stderr").filter(|s| !s.is_empty()),
            duration_ms: take_u64(&mut a, "durationMs"),
            blocked: sub == "hook_blocking_error",
        };
        if hook.stdout.is_some()
            && a.get("content").and_then(Value::as_str) == hook.stdout.as_deref()
        {
            a.remove("content");
        }
        // The text the hook returned: the reason of a block, or a message for the user.
        if let Some(mut blocking) = take_obj(&mut a, "blockingError") {
            hook.stderr = hook.stderr.or(take_str(&mut blocking, "blockingError"));
            hook.command = hook.command.or(take_str(&mut blocking, "command"));
            if !blocking.is_empty() {
                a.insert("blockingError".into(), Value::Object(blocking));
            }
        }
        if sub == "hook_system_message" {
            hook.stdout = hook.stdout.or(take_str(&mut a, "content"));
        }
        a.insert("type".into(), Value::String(sub));
        rec.insert("attachment".into(), Value::Object(a));
        let id = head.uuid.clone();
        self.push(&head, id, Some(rec), EntryKind::Hook(hook));
    }

    /// Injected context: a message when it carries text the model saw, else `Other`.
    fn context(&mut self, head: Head, mut rec: Obj, a: Obj, sub: String) {
        let mut a = Value::Object(a);
        let text = match rendered_text(rec.get("rendered")) {
            Some(rendered) => {
                rec.remove("rendered");
                drop_rendered_copy(&mut a, &rendered);
                Some(rendered)
            }
            None => take_attachment_text(&mut a),
        };
        let id = head.uuid.clone();
        let Some(text) = text else {
            let kind = EntryKind::Other(Other {
                kind: format!("attachment.{sub}"),
                data: stripped(a),
            });
            self.push(&head, id, Some(rec), kind);
            return;
        };
        let role = match rec.get("renderedRole").and_then(Value::as_str) {
            Some("system") => Some(Role::System),
            Some("user") => Some(Role::User),
            _ => None,
        };
        if role.is_some() {
            rec.remove("renderedRole");
        }
        if a.as_object().is_some_and(|a| !a.is_empty()) {
            rec.insert("attachment".into(), a);
        }
        let role = role.unwrap_or(Role::User);
        let msg = Message {
            kind: Some(sub),
            ..Message::new(role, Origin::Context, vec![Part::Text { text }])
        };
        self.push(&head, id, Some(rec), EntryKind::Message(msg));
    }

    fn system(&mut self, head: Head, mut rec: Obj) {
        let sub = take_str(&mut rec, "subtype").unwrap_or_default();
        let killed = sub == "agents_killed";
        let queued = killed.then(|| self.killed_text(head.ts)).flatten();
        let kind = match sub.as_str() {
            "turn_duration" => EntryKind::Turn(Turn {
                duration_ms: take_u64(&mut rec, "durationMs"),
                pending_agents: take_u64(&mut rec, "pendingBackgroundAgentCount").map(|n| n as u32),
                pending_workflows: take_u64(&mut rec, "pendingWorkflowCount").map(|n| n as u32),
                ..Default::default()
            }),
            "api_error" => EntryKind::Error(api_error(&mut rec)),
            "compact_boundary" => EntryKind::Compaction(self.boundary(&mut rec)),
            "local_command" => {
                let text = take_str(&mut rec, "content").unwrap_or_default();
                let kind = leading_tag(&text).unwrap_or("local_command").to_string();
                let parts = vec![Part::Text { text }];
                EntryKind::Message(Message {
                    kind: Some(kind),
                    ..Message::new(Role::User, Origin::Command, parts)
                })
            }
            "stop_hook_summary" => EntryKind::Hook(stop_hooks(&mut rec)),
            // A recap of the session for a person coming back to it.
            "away_summary" => {
                let text = take_str(&mut rec, "content").unwrap_or_default();
                let parts = vec![Part::Text { text }];
                EntryKind::Message(Message {
                    kind: Some(sub.clone()),
                    ..Message::new(Role::System, Origin::Summary, parts)
                })
            }
            _ => EntryKind::Notice(Notice {
                level: take_str(&mut rec, "level"),
                kind: (!sub.is_empty()).then(|| sub.clone()),
                text: take_str(&mut rec, "content").or(queued).unwrap_or_default(),
            }),
        };
        let pending = match &kind {
            EntryKind::Turn(t) => (t.pending_agents, t.pending_workflows),
            _ => (None, None),
        };
        let id = head.uuid.clone();
        let at = self.push(&head, id, Some(rec), kind);
        if sub == "compact_boundary" {
            self.compaction = Some(at);
        }
        // A turn that ends with none left means the earlier launches are over, report or not.
        let status = if killed { "killed" } else { "ended" };
        if killed || pending.0 == Some(0) {
            self.end_agents(false, status, head.ts);
        }
        if killed || pending.1 == Some(0) {
            self.end_agents(true, status, head.ts);
        }
    }

    fn boundary(&self, rec: &mut Obj) -> Compaction {
        let mut meta = take_obj(rec, "compactMetadata").unwrap_or_default();
        let compaction = Compaction {
            summary: None,
            trigger: take_str(&mut meta, "trigger"),
            tokens_before: take_u64(&mut meta, "preTokens"),
            tokens_after: take_u64(&mut meta, "postTokens"),
            logical_parent: take_str(rec, "logicalParentUuid").map(|p| self.resolve(&p)),
            usage: None,
        };
        if !meta.is_empty() {
            rec.insert("compactMetadata".into(), Value::Object(meta));
        }
        compaction
    }

    fn finish(mut self, source: Source) -> Result<Session> {
        if self.records == 0 && !self.entries.is_empty() {
            return Err(Error::NotASession(
                "a workflow journal, not a session".into(),
            ));
        }
        self.flush_queue();
        self.flush_orphans();
        self.expire_monitors();
        self.relink();
        for e in &mut self.entries {
            if let Some(Value::Object(map)) = &mut e.extra {
                clean(map, 3);
                e.extra = as_extra(std::mem::take(map));
            }
        }
        if let Some(&first) = self.copies.first() {
            let n = self.copies.len();
            let message =
                format!("{n} lines repeat the uuid of an earlier line (history copies), skipped");
            self.warn(Some(first), message);
        }
        self.warnings.sort_by_key(|w| w.line);
        let path = match &source {
            Source::File { path } => Some(path.clone()),
            Source::Sqlite { .. } => None,
        };
        let mut extra = Obj::new();
        let mut meta = Obj::new();
        let (id, parent) = self.identity(path.as_deref(), &mut meta, &mut extra);
        let mut session = Session::new(Agent::Claude, id, source, "claude-jsonl");
        session.parent = parent;
        session.title = self.title_of(&mut meta);
        session.cwd = self.first_cwd;
        session.cli_version = take_str(&mut self.env, "version");
        session.git = take_str(&mut self.env, "gitBranch").map(|branch| Git {
            branch: Some(branch),
            ..Default::default()
        });
        extra.append(&mut self.env);
        extra.append(&mut self.titles);
        if let Some(mut state) = self.cost {
            // The recorded session cost; `Session::finish` keeps it.
            session.usage.cost_usd = state.remove("totalCostUSD").and_then(|c| c.as_f64());
            if !state.is_empty() {
                extra.insert("cost-state".into(), Value::Object(state));
            }
        }
        if !meta.is_empty() {
            extra.insert("meta".into(), Value::Object(meta));
        }
        session.extra = as_extra(extra);
        session.entries = self.entries;
        session.warnings = self.warnings;
        // After `finish`: the usage of a dropped branch was spent all the same and stays in the total.
        let mut session = session.finish();
        if let Some((n, first)) = prune_abandoned(&mut session.entries) {
            let message = format!(
                "{n} entries on branches the conversation left (a prompt sent again), skipped"
            );
            session.warnings.push(Warning {
                line: first,
                message,
            });
            session.warnings.sort_by_key(|w| w.line);
        }
        Ok(session)
    }

    /// Session id and parent. A subagent file starts with a sidechain line: its `agentId` is the
    /// id, the parent comes from the sidecar meta file (`parentAgentId`, `toolUseId`, `agentType`)
    /// or else the line `sessionId`. A `sessionId` that took no typed field goes to `extra`.
    fn identity(
        &mut self,
        path: Option<&Path>,
        meta: &mut Obj,
        extra: &mut Obj,
    ) -> (String, Option<ParentLink>) {
        let sidechain = self.env.remove("isSidechain") == Some(Value::Bool(true));
        let agent_id = sidechain
            .then(|| take_str(&mut self.env, "agentId"))
            .flatten();
        let session_id = take_str(&mut self.env, "sessionId");
        let stem = path
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned());
        let Some(agent_id) = agent_id else {
            let id = session_id.clone().or(stem).unwrap_or_default();
            if let Some(s) = session_id.filter(|s| *s != id) {
                extra.insert("sessionId".into(), Value::String(s));
            }
            return (id, None);
        };
        if let Some(path) = path {
            *meta = self.meta(path).unwrap_or_default();
        }
        // A teammate's meta names no call, and its `agentType` is its name (kept in the meta).
        let teammate = meta.get("taskKind").and_then(Value::as_str) == Some("in_process_teammate");
        let call_id = take_str(meta, "toolUseId").or_else(|| {
            let lead = path.filter(|_| teammate)?.parent()?.parent()?;
            // An older spawn names it `<name>@<teamName>` (e857d7c5 line 1110), never its id.
            let field = |k| meta.get(k).and_then(Value::as_str);
            let address = match (field("name"), field("teamName")) {
                (Some(name), Some(team)) => format!("{name}@{team}"),
                _ => agent_id.clone(),
            };
            spawn_call(&lead.with_extension("jsonl"), &[&agent_id, &address])
        });
        let agent_type = (!teammate).then(|| take_str(meta, "agentType")).flatten();
        let parent = match take_str(meta, "parentAgentId") {
            Some(parent) => {
                extra.extend(session_id.map(|s| ("sessionId".to_string(), Value::String(s))));
                Some(parent)
            }
            None => session_id,
        };
        let parent = parent.map(|session_id| ParentLink {
            session_id,
            call_id,
            agent_type,
        });
        (agent_id, parent)
    }

    /// custom-title > agent-name > subagent meta description > ai-title > old summary line whose
    /// leaf is in this file. `Session::finish` falls back to the first human prompt.
    fn title_of(&mut self, meta: &mut Obj) -> Option<String> {
        if let Some(d) = take_str(meta, "description") {
            self.titles.insert("description".into(), Value::String(d));
        }
        let summary = self
            .summaries
            .iter()
            .rev()
            .find(|(leaf, _)| self.uuids.contains_key(leaf));
        let summary = summary.map(|(_, text)| text.clone());
        let title = ["custom-title", "agent-name", "description", "ai-title"]
            .iter()
            .find_map(|k| take_str(&mut self.titles, k));
        title.or(summary)
    }

    /// The subagent sidecar `agent-<id>.meta.json`, when present.
    fn meta(&mut self, path: &Path) -> Option<Obj> {
        let text = match std::fs::read_to_string(path.with_extension("meta.json")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                self.warn(None, format!("subagent meta file unreadable: {e}"));
                return None;
            }
        };
        match serde_json::from_str(&text) {
            Ok(Value::Object(meta)) => Some(meta),
            _ => {
                self.warn(None, "subagent meta file is not a JSON object".into());
                None
            }
        }
    }

    /// The words of an `agents_killed` record, which has none of its own, read from the queue:
    /// the harness queues them the same moment (6332d703 line 8301) or a millisecond before
    /// (xheron 53b86fc1 line 66460).
    fn killed_text(&self, ts: Option<i64>) -> Option<String> {
        let ts = ts?;
        let at = (self.queue.iter()).rposition(|(at, text)| {
            let gap = self.entries[*at].ts.map(|t| ts - t);
            gap.is_some_and(|g| (0..=KILL_GAP_MS).contains(&g)) && stopped_text(text)
        })?;
        // Left queued: a later dequeue takes them, not the typed text behind them, and
        // `flush_queue` drops harness text.
        self.queue.get(at).map(|(_, text)| text.clone())
    }

    /// Human input still queued at the end, right after the record that queued it. Harness text
    /// stays in the queue until then, since it takes its dequeue like typed input.
    fn flush_queue(&mut self) {
        let queue = std::mem::take(&mut self.queue);
        let typed = queue.into_iter().filter(|(_, text)| !is_harness_text(text));
        for (at, text) in typed.rev() {
            let src = &self.entries[at];
            let msg = Message {
                phase: Some("queued".into()),
                ..Message::new(Role::User, Origin::Human, vec![Part::Text { text }])
            };
            let entry = Entry {
                id: None,
                parent_id: None,
                turn_id: src.turn_id.clone(),
                ts: src.ts,
                line: src.line,
                extra: None,
                kind: EntryKind::Message(msg),
            };
            self.entries.insert(at + 1, entry);
        }
    }

    /// Results whose call never came: a `ToolResult` message each, at its line, with a warning.
    fn flush_orphans(&mut self) {
        let mut orphans = std::mem::take(&mut self.orphans);
        orphans.extend(self.pending.drain());
        orphans.sort_by_key(|(_, held)| held.head.line);
        for (call_id, held) in orphans {
            let line = held.head.line;
            self.warn(
                Some(line),
                format!("tool result for a call not in the parsed lines: {call_id}"),
            );
            let o = outcome("", &Value::Null, held.block, held.tur, held.denial);
            let result = ToolResult {
                call_id,
                status: o.status,
                status_detail: o.detail,
                output: o.output,
            };
            let msg = Message::new(Role::User, Origin::Tool, vec![Part::ToolResult(result)]);
            if let Some(uuid) = &held.head.uuid {
                self.uuids
                    .entry(uuid.clone())
                    .or_insert_with(|| uuid.clone());
            }
            let entry = Entry {
                id: held.head.uuid,
                parent_id: held.head.parent,
                turn_id: held.turn,
                ts: held.head.ts,
                line: Some(line),
                extra: held.rest.map(Value::Object),
                kind: EntryKind::Message(msg),
            };
            let at = self
                .entries
                .partition_point(|e| e.line.is_some_and(|l| l <= line));
            self.entries.insert(at, entry);
        }
    }

    /// Resolve parents again once every line is read: a line can name a parent written after it.
    fn relink(&mut self) {
        let resolve = |p: &mut Option<String>| {
            if let Some(id) = p.as_ref().and_then(|p| self.uuids.get(p)) {
                *p = Some(id.clone());
            }
        };
        for e in &mut self.entries {
            resolve(&mut e.parent_id);
            if let EntryKind::Compaction(c) = &mut e.kind {
                resolve(&mut c.logical_parent);
            }
        }
    }
}

/// A prompt sent again after Esc or a rewind leaves its first try as a branch the conversation
/// never went on from: a human prompt whose sibling prompt is on the live chain, and everything
/// below it, is dropped. Only prompt heads count, so results that share a parent stay. A
/// compaction hangs below its `logical_parent` like a child. Returns how many entries went and the
/// first of their lines.
fn prune_abandoned(entries: &mut Vec<Entry>) -> Option<(usize, Option<usize>)> {
    let ids: HashMap<&str, usize> = (entries.iter().enumerate())
        .filter_map(|(i, e)| Some((e.id.as_deref()?, i)))
        .collect();
    // A compaction has no `parent_id`: it links to the line it follows by `logical_parent`.
    let parent_of = |e: &Entry| match (&e.kind, &e.parent_id) {
        (EntryKind::Compaction(c), None) => c.logical_parent.clone(),
        (_, parent) => parent.clone(),
    };
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(&p) = parent_of(e).as_deref().and_then(|p| ids.get(p)) {
            children.entry(p).or_default().push(i);
        }
    }
    // The live chain: from the last line that has a parent back to the root.
    let mut live = HashSet::new();
    let mut at = entries.iter().rposition(|e| parent_of(e).is_some());
    while let Some(i) = at.filter(|i| live.insert(*i)) {
        at = parent_of(&entries[i])
            .as_deref()
            .and_then(|p| ids.get(p))
            .copied();
    }
    let prompt = |i: usize| matches!(&entries[i].kind, EntryKind::Message(m) if m.origin == Origin::Human && m.role == Role::User);
    let mut dead = HashSet::new();
    for siblings in children.values() {
        if !siblings.iter().any(|&s| live.contains(&s) && prompt(s)) {
            continue;
        }
        for &head in siblings
            .iter()
            .filter(|&&h| !live.contains(&h) && prompt(h))
        {
            let mut stack = vec![head];
            while let Some(i) = stack.pop() {
                if dead.insert(i) {
                    stack.extend(children.get(&i).into_iter().flatten());
                }
            }
        }
    }
    if dead.is_empty() {
        return None;
    }
    let first = dead.iter().filter_map(|&i| entries[i].line).min();
    let n = dead.len();
    let mut i = 0;
    entries.retain(|_| {
        i += 1;
        !dead.contains(&(i - 1))
    });
    Some((n, first))
}

/// A `tool_result` block with what its line said about it, kept until its call is read.
struct Held {
    head: Head,
    turn: Option<String>,
    block: Obj,
    tur: Option<Value>,
    denial: Option<String>,
    /// The line's leftover keys, when the line holds nothing but results.
    rest: Option<Obj>,
}

/// Line-level facts that decide who a user text came from.
struct Ctx {
    meta: bool,
    origin: Option<String>,
    author: Option<String>,
    sidechain: bool,
}

#[derive(Clone, PartialEq)]
enum Class {
    Interrupt,
    /// origin, kind, author
    Message(Origin, Option<String>, Option<String>),
}

fn classify(text: &str, ctx: &Ctx) -> Class {
    if text.starts_with(INTERRUPTED) {
        return Class::Interrupt;
    }
    // A teammate's message handed over as a prompt: the tag is under Claude Code's own line.
    let tag = leading_tag(text.strip_prefix(PEER_SENT).unwrap_or(text));
    let msg = |origin, kind: Option<&str>| Class::Message(origin, kind.map(String::from), None);
    if let Some(tag) = tag.filter(|t| COMMAND_TAGS.contains(t)) {
        return msg(Origin::Command, Some(tag));
    }
    if let Some(kind) = ctx.origin.as_deref().filter(|k| *k != "human") {
        return Class::Message(origin_of(kind), Some(kind.into()), ctx.author.clone());
    }
    match tag {
        Some("task-notification") => msg(Origin::Notification, tag),
        Some("teammate-message") => Class::Message(
            Origin::Agent,
            Some("teammate-message".into()),
            attr(text, "teammate_id"),
        ),
        _ if ctx.meta => msg(Origin::Context, Some(tag.unwrap_or("meta"))),
        Some("system-reminder" | "local-command-caveat") => msg(Origin::Context, tag),
        _ if text.starts_with(CONTINUED) => msg(Origin::Summary, None),
        _ if ctx.sidechain => msg(Origin::Agent, None),
        _ => msg(Origin::Human, None),
    }
}

fn origin_of(kind: &str) -> Origin {
    match kind {
        "human" => Origin::Human,
        "task-notification" => Origin::Notification,
        "coordinator" | "peer" => Origin::Agent,
        _ => Origin::Context,
    }
}

/// `origin.kind` and the sender (`origin.name` or `origin.from`); the rest of `origin` stays.
fn take_origin(m: &mut Obj) -> (Option<String>, Option<String>) {
    let Some(mut origin) = take_obj(m, "origin") else {
        return (None, None);
    };
    let kind = take_str(&mut origin, "kind");
    let author = take_str(&mut origin, "name").or_else(|| take_str(&mut origin, "from"));
    if !origin.is_empty() {
        m.insert("origin".into(), Value::Object(origin));
    }
    (kind, author)
}

/// One content block as a part, plus the unmapped keys of a `tool_use` block.
fn part(block: Value, cwd: Option<&str>, ts: Option<i64>) -> (Part, Obj) {
    let mut b = match block {
        Value::Object(b) => b,
        other => {
            let part = Part::Other {
                kind: "unknown".into(),
                data: other,
            };
            return (part, Obj::new());
        }
    };
    let kind = take_str(&mut b, "type").unwrap_or_default();
    if kind == "tool_use" {
        let (call, rest) = tool_call(b, cwd, ts);
        return (Part::ToolCall(call), rest);
    }
    let part = match kind.as_str() {
        "text" => Part::Text {
            text: take_str(&mut b, "text").unwrap_or_default(),
        },
        "thinking" => Part::Reasoning {
            text: take_str(&mut b, "thinking").filter(|t| !t.is_empty()),
            encrypted: b
                .get("signature")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty()),
        },
        "redacted_thinking" => Part::Reasoning {
            text: None,
            encrypted: true,
        },
        "image" | "document" => Part::Attachment(media(&b)),
        _ => Part::Other {
            kind,
            data: stripped(Value::Object(b)),
        },
    };
    (part, Obj::new())
}

/// Text a person typed into the queue; `<...>` texts are notifications.
fn is_human_text(text: &str) -> bool {
    !text.trim().is_empty() && !text.trim_start().starts_with('<')
}

/// Untagged text the harness queues as if typed: `3 background agents were stopped by the
/// user: ...` (with `agents_killed`) and the usage limit reset prompt.
fn is_harness_text(text: &str) -> bool {
    stopped_text(text) || text.starts_with("Your claude.ai usage limit has reset.")
}

/// `3 background agents were stopped by the user: ...`, the words of an `agents_killed`.
fn stopped_text(text: &str) -> bool {
    let count = text.trim_start_matches(|c: char| c.is_ascii_digit());
    count.len() < text.len()
        && count.starts_with(" background ")
        && count.contains(" stopped by the user")
}

/// The spawn call of a teammate in the lead's transcript: the first result that names the agent
/// (by any of `names`) is its spawn acknowledgement.
fn spawn_call(lead: &Path, names: &[&str]) -> Option<String> {
    // ponytail: one scan of the lead file per teammate file parse; index it if that shows up.
    let file = std::io::BufReader::new(std::fs::File::open(lead).ok()?);
    file.lines()
        .map_while(std::io::Result::ok)
        .find_map(|line| {
            if !line.contains("\"tool_use_id\"") || !names.iter().any(|n| line.contains(n)) {
                return None;
            }
            let rec: Value = serde_json::from_str(&line).ok()?;
            let blocks = rec.pointer("/message/content")?.as_array()?;
            blocks.iter().find_map(|b| {
                let content = b.get("content")?.to_string();
                let named = names.iter().any(|n| content.contains(n));
                named.then(|| b.get("tool_use_id")?.as_str().map(String::from))?
            })
        })
}

/// `Task #5 created successfully: Phase 0` to (`5`, `Phase 0`).
fn created_task(text: &str) -> Option<(String, String)> {
    let (n, subject) = text
        .strip_prefix("Task #")?
        .split_once(" created successfully: ")?;
    Some((n.to_string(), subject.trim().to_string()))
}

/// Leftover keys of one call's source record in `extra.calls.<call_id>.<key>` (`call` for the
/// `tool_use` block, `output` for the result line).
fn call_extra(extra: &mut Obj, call_id: &str, key: &str, rest: Obj) {
    if rest.is_empty() {
        return;
    }
    let calls = extra
        .entry("calls")
        .or_insert_with(|| Value::Object(Obj::new()));
    if let Value::Object(calls) = calls {
        let call = calls
            .entry(call_id)
            .or_insert_with(|| Value::Object(Obj::new()));
        if let Value::Object(call) = call {
            call.insert(key.into(), Value::Object(rest));
        }
    }
}

fn media(block: &Obj) -> Media {
    let source = block.get("source");
    let field = |k: &str| source.and_then(|s| s.get(k)).and_then(Value::as_str);
    Media {
        mime: field("media_type").map(String::from),
        name: block.get("title").and_then(Value::as_str).map(String::from),
        url: field("url").map(String::from),
        // A `text` document source holds plain text, every other inline source base64.
        bytes: field("data").map(|d| match field("type") {
            Some("text") => d.len() as u64,
            _ => base64_len(d),
        }),
    }
}

fn api_error(rec: &mut Obj) -> ErrorInfo {
    let mut error = rec.remove("error").unwrap_or(Value::Null);
    let mut info = ErrorInfo {
        retry: take_u64(rec, "retryAttempt").and_then(|r| u32::try_from(r).ok()),
        ..Default::default()
    };
    if let Value::Object(e) = &mut error {
        info.message = take_str(e, "formatted")
            .or_else(|| take_str(e, "message"))
            .unwrap_or_default();
        info.status = take_u64(e, "status").and_then(|s| u16::try_from(s).ok());
        info.kind = take_str(e, "type");
    }
    if info.message.is_empty() {
        // Older shapes nest the message; 2.1.143 lines put a `cause` next to `error`.
        let cause = rec.get("cause");
        let nested = ["/error/error/message", "/error/message"]
            .iter()
            .find_map(|p| error.pointer(p).and_then(Value::as_str));
        let from_cause = ["/message", "/code"]
            .iter()
            .find_map(|p| cause?.pointer(p).and_then(Value::as_str));
        info.message = nested.or(from_cause).unwrap_or("API error").to_string();
    }
    if !error.is_null() && error.as_object().is_none_or(|e| !e.is_empty()) {
        rec.insert("error".into(), error);
    }
    info
}

fn stop_hooks(rec: &mut Obj) -> Hook {
    let mut hook = Hook {
        event: Some("Stop".into()),
        blocked: take_bool(rec, "preventedContinuation"),
        ..Default::default()
    };
    let errors: Vec<String> = take_array(rec, "hookErrors")
        .into_iter()
        .filter_map(|e| e.as_str().map(String::from))
        .collect();
    hook.stderr = (!errors.is_empty()).then(|| errors.join("\n"));
    if let Some(Value::Array(infos)) = rec.get_mut("hookInfos") {
        if let [Value::Object(info)] = infos.as_mut_slice() {
            hook.command = take_str(info, "command");
            hook.duration_ms = take_u64(info, "durationMs");
            if info.is_empty() {
                rec.remove("hookInfos");
            }
        }
    }
    hook
}

/// The one line the UI shows for an attachment; `None` for a kind it does not show that way
/// (and for a goal check that only re-armed the goal).
fn attachment_line(sub: &str, a: &Obj) -> Option<String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str);
    let path = || {
        s("displayPath")
            .or_else(|| s("filename"))
            .or_else(|| s("path"))
    };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    Some(match sub {
        "file" => match a.get("content").and_then(|c| c.pointer("/file/numLines")) {
            Some(n) => format!("Read {} ({n} lines)", path()?),
            None => format!("Read {}", path()?),
        },
        "directory" => format!("Listed directory {}/", path()?.trim_end_matches('/')),
        "nested_memory" => format!("Loaded {}", path()?),
        "compact_file_reference" => format!("Referenced file {}", path()?),
        "invoked_skills" => {
            let skills = a.get("skills")?.as_array()?;
            let names: Vec<&str> = skills
                .iter()
                .filter_map(|k| k.get("name")?.as_str())
                .collect();
            format!("Skills restored: {}", names.join(", "))
        }
        "diagnostics" => {
            let files = a.get("files")?.as_array()?;
            let count = |f: &Value| {
                f.get("diagnostics")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)
            };
            let issues: usize = files.iter().map(count).sum();
            let (i, f) = (issues, files.len());
            format!(
                "Found {i} new diagnostic issue{} in {f} file{}",
                plural(i),
                plural(f)
            )
        }
        "goal_status" if a.get("sentinel").and_then(Value::as_bool) != Some(true) => {
            match (
                a.get("met").and_then(Value::as_bool)?,
                s("reason"),
                s("condition"),
            ) {
                (true, _, Some(goal)) => format!("Goal met: {goal}"),
                (false, Some(why), _) => format!("Goal not met: {why}"),
                (met, _, _) => format!("Goal {}", if met { "met" } else { "not met" }),
            }
        }
        "task_status" => {
            let what = s("description").unwrap_or("Background task");
            match s("status")? {
                "completed" => format!("{what} completed in background"),
                "running" => format!("{what} still running in background"),
                "stopped" => format!("{what} stopped"),
                other => format!("{what}: {other}"),
            }
        }
        _ => return None,
    })
}

fn rendered_text(rendered: Option<&Value>) -> Option<String> {
    let texts: Vec<&str> = rendered?
        .as_array()?
        .iter()
        .filter_map(|r| r.get("content").and_then(Value::as_str))
        .collect();
    let text = texts.join("\n");
    (!text.is_empty()).then_some(text)
}

fn text_at(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_string) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// The attachment's own text: a field from [`ATTACHMENT_TEXT`], or the bodies of re-injected
/// skills (`None` as the pointer).
fn attachment_text(a: &Value) -> Option<(Option<&'static str>, String)> {
    if let Some(found) = ATTACHMENT_TEXT
        .iter()
        .find_map(|p| Some((Some(*p), text_at(a.pointer(p))?)))
    {
        return Some(found);
    }
    let skills = a.get("skills")?.as_array()?;
    let bodies: Vec<&str> = skills
        .iter()
        .filter_map(|s| s.get("content")?.as_str())
        .collect();
    (!bodies.is_empty()).then(|| (None, bodies.join("\n")))
}

fn remove_attachment_text(a: &mut Value, pointer: Option<&str>) {
    match pointer {
        Some(pointer) => {
            remove_at(a, pointer);
        }
        None => {
            for skill in a
                .get_mut("skills")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                skill.as_object_mut().map(|s| s.remove("content"));
            }
        }
    }
}

fn take_attachment_text(a: &mut Value) -> Option<String> {
    let (pointer, text) = attachment_text(a)?;
    remove_attachment_text(a, pointer);
    Some(text)
}

/// The rendered text usually wraps the attachment's own text field; keep only the rendered one.
fn drop_rendered_copy(a: &mut Value, rendered: &str) {
    if let Some((pointer, text)) = attachment_text(a) {
        // A file body is rendered `cat -n` style.
        if rendered.contains(&text) || strip_line_numbers(rendered).contains(&text) {
            remove_attachment_text(a, pointer);
        }
    }
}

fn remove_at(v: &mut Value, pointer: &str) -> Option<Value> {
    let (parent, key) = pointer.rsplit_once('/')?;
    v.pointer_mut(parent)?.as_object_mut()?.remove(key)
}

fn usage_of(mut u: Obj) -> (Usage, Obj) {
    let mut usage = Usage {
        input: take_u64(&mut u, "input_tokens").unwrap_or(0),
        output: take_u64(&mut u, "output_tokens").unwrap_or(0),
        cache_read: take_u64(&mut u, "cache_read_input_tokens").unwrap_or(0),
        cache_write: take_u64(&mut u, "cache_creation_input_tokens").unwrap_or(0),
        ..Default::default()
    };
    if let Some(Value::Object(details)) = u.get_mut("output_tokens_details") {
        usage.reasoning = take_u64(details, "thinking_tokens").unwrap_or(0);
        if details.is_empty() {
            u.remove("output_tokens_details");
        }
    }
    (usage, u)
}

/// `wireToolInputs` echoes each call's input as sent; drop the copies equal to the input.
fn dedupe_wire_inputs(rec: &mut Obj, parts: &[(Part, Obj)]) {
    let Some(Value::Object(wire)) = rec.get_mut("wireToolInputs") else {
        return;
    };
    wire.retain(|id, input| {
        !parts
            .iter()
            .any(|(p, _)| matches!(p, Part::ToolCall(c) if c.id == *id && c.input == *input))
    });
    if wire.is_empty() {
        rec.remove("wireToolInputs");
    }
}

fn call_at(e: &mut Entry, part: usize) -> Option<&mut ToolCall> {
    match &mut e.kind {
        EntryKind::Message(m) => match m.parts.get_mut(part)? {
            Part::ToolCall(c) => Some(c),
            _ => None,
        },
        _ => None,
    }
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// `<tag ...>` at the start of the text.
fn leading_tag(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix('<')?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    let next = rest[end..].chars().next();
    (!name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && matches!(next, Some('>' | ' ' | '\n')))
    .then_some(name)
}

/// The text between `<name>` and `</name>`.
fn tag_body<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{name}>"))? + name.len() + 2;
    let len = text[start..].find(&format!("</{name}>"))?;
    Some(&text[start..start + len])
}

/// `name="value"` inside the first tag.
fn attr(text: &str, name: &str) -> Option<String> {
    let tag = &text[..text.find('>')?];
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let len = tag[start..].find('"')?;
    Some(tag[start..start + len].to_string())
}

fn drop_eq(m: &mut Obj, key: &str, value: &str) {
    if m.get(key).and_then(Value::as_str) == Some(value) {
        m.remove(key);
    }
}

fn extra_mut(extra: &mut Option<Value>) -> &mut Obj {
    if !matches!(extra, Some(Value::Object(_))) {
        *extra = Some(Value::Object(Obj::new()));
    }
    match extra {
        Some(Value::Object(m)) => m,
        _ => unreachable!("set just above"),
    }
}

fn push_uuid(extra: &mut Obj, uuid: &Option<String>) {
    let Some(uuid) = uuid else { return };
    if let Value::Array(uuids) = extra
        .entry("uuids")
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        if !uuids.iter().any(|u| u.as_str() == Some(uuid)) {
            uuids.push(Value::String(uuid.clone()));
        }
    }
}

/// Later keys win; nested objects merge one level deep.
fn merge(dst: &mut Obj, src: Obj) {
    for (k, v) in src {
        match (dst.get_mut(&k), v) {
            (Some(Value::Object(d)), Value::Object(s)) => d.extend(s),
            (_, v) => {
                dst.insert(k, v);
            }
        }
    }
}

/// Drop null and empty values, `depth` levels deep.
fn clean(map: &mut Obj, depth: u8) {
    map.retain(|_, v| {
        if let (Value::Object(m), true) = (&mut *v, depth > 1) {
            clean(m, depth - 1);
        }
        match v {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            _ => true,
        }
    })
}
