//! Gemini CLI sessions (`~/.gemini/tmp/<project>/chats/`).
//!
//! Three on-disk generations share one record grammar: G1 is one JSON document, G2 and G3 are
//! JSONL. The file is reduced the way the CLI's own loader does it (header, `$set`, `$patch`,
//! `$rewindTo`, bare records that upsert by id and keep their first position), but nothing is
//! forgotten: every message id ever seen stays in the session at its last version in
//! first-seen order, and a [`Rewind`] entry marks where turns left the live history. The
//! tool-result echo turns (`user` messages made of `functionResponse` parts) fold into the
//! calls they answer.
//!
//! - `reduce`: the record reducer (no knowledge of entries).
//! - `part`: content parts, media, thoughts.
//! - `tool`: typed tool input, results, status, file changes.
//! - `shell`: the text of shell results per CLI era.

mod part;
mod reduce;
mod shell;
mod tool;

use crate::util::{Obj, join_texts, str_field, stripped, ts_ms, u64_field};
use crate::{
    Agent, Compaction, Entry, EntryKind, Error, ErrorInfo, Media, Message, Notice, Origin, Other,
    ParentLink, Part, Result, Rewind, Role, Session, Source, Tool, ToolCall, ToolOutput,
    ToolResult, ToolStatus, Usage, Warning,
};
use part::{any_thought, array, convert, join_text, media, norm, thought_parts, unmapped};
use reduce::{Rec, Reducer, Slot, is_masked};
use serde_json::Value;
use shell::truncation;
use std::collections::HashSet;
use std::io::BufRead;
use tool::{call_data, call_status, file_change, is_cancelled, map_tool, read_resp, shell_output};

/// Keys of every message record that typed fields take.
const RECORD_KEYS: &[&str] = &["id", "timestamp", "type", "content"];
/// Keys only a `gemini` record has.
const MODEL_KEYS: &[&str] = &["thoughts", "tokens", "model", "toolCalls"];
const MAPPED_META_KEYS: &[&str] = &["sessionId", "startTime", "lastUpdated", "summary"];

/// Parse a Gemini transcript: a G1 `.json` document or a G2/G3 `.jsonl` record stream.
pub(crate) fn parse_reader(mut reader: impl BufRead, source: Source) -> Result<Session> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut red = Reducer::default();
    red.read(&text)?;
    let Some(id) = red
        .meta
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Err(Error::NotASession(
            "no gemini header with a sessionId".into(),
        ));
    };
    let subagent = red.meta.get("kind").and_then(Value::as_str) == Some("subagent");
    let mut build = Build::new(&red, subagent);
    build.run();
    let Build {
        mut entries,
        warnings,
        ..
    } = build;
    link_compaction_tokens(&mut entries);

    let mut session = Session::new(Agent::Gemini, id, source.clone(), red.format());
    session.title = red
        .meta
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_string);
    session.started_at = red.meta.get("startTime").and_then(ts_ms);
    session.updated_at = red.meta.get("lastUpdated").and_then(ts_ms);
    session.parent = subagent.then(|| parent_link(&source)).flatten();
    session.cwd = project_root(&source).or_else(|| context_workspace(&entries));
    default_shell_cwd(&mut entries, session.cwd.as_deref());
    // `projectHash`, `kind`, `directories`, `memoryScratchpad`, anything newer.
    session.extra = unmapped(&red.meta, |k| MAPPED_META_KEYS.contains(&k));
    session.entries = entries;
    session.warnings = red.warnings;
    session.warnings.extend(warnings);
    Ok(session.finish())
}

// ---------------------------------------------------------------------------------------------
// User records

/// What one text part of a user record is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Human,
    Slash,
    SessionContext,
    HookContext,
    /// The `<state_snapshot>` summary the compression writes as a user turn.
    Snapshot,
}

impl Class {
    /// `ack_follows`: the next record is the model's "Got it" answer to a compression.
    fn of(text: &str, ack_follows: bool) -> Class {
        let t = text.trim_start();
        let tagged = t.contains("<state_snapshot>") && t.contains("</state_snapshot>");
        if t.starts_with("<session_context>") {
            Class::SessionContext
        } else if t.starts_with("<hook_context>") {
            Class::HookContext
        } else if tagged && (t.starts_with("<state_snapshot>") || ack_follows) {
            Class::Snapshot
        } else if is_slash_command(t) {
            Class::Slash
        } else {
            Class::Human
        }
    }

    /// Origin and kind of the message; `None` for a snapshot, which becomes a `Compaction`.
    fn label(self, subagent: bool) -> Option<(Origin, Option<&'static str>)> {
        Some(match self {
            Class::Human if subagent => (Origin::Agent, None),
            Class::Human => (Origin::Human, None),
            Class::Slash => (Origin::Command, Some("slash_command")),
            Class::SessionContext => (Origin::Context, Some("session_context")),
            Class::HookContext => (Origin::Context, Some("hook_context")),
            Class::Snapshot => return None,
        })
    }
}

/// `/help`, `/dir add x`: a slash and a word. A path such as `/usr/bin/x` is a prompt.
fn is_slash_command(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('/') else {
        return false;
    };
    let word = rest.split_whitespace().next().unwrap_or("");
    word.starts_with(|c: char| c.is_ascii_alphabetic()) && !word.contains('/')
}

/// One entry's worth of a user record.
enum Group {
    /// Results whose call is not in the file.
    Orphans(Vec<Part>),
    Text(Class, Vec<Part>),
}

// ---------------------------------------------------------------------------------------------
// Entries

struct Build<'a> {
    red: &'a Reducer,
    subagent: bool,
    /// Ids of every tool call in the file, to tell echo turns from orphan results.
    call_ids: HashSet<&'a str>,
    entries: Vec<Entry>,
    warnings: Vec<Warning>,
}

/// A call to build: the `toolCalls[]` record when there is one, else the bare `functionCall` part.
struct CallSrc<'a> {
    id: String,
    name: String,
    args: Value,
    rec: Option<&'a Value>,
}

impl<'a> CallSrc<'a> {
    fn record(rec: &'a Value) -> Self {
        let args = rec.get("args").cloned().unwrap_or(Value::Null);
        let text = |key| str_field(rec, &[key]).unwrap_or_default();
        CallSrc {
            id: text("id"),
            name: text("name"),
            args,
            rec: Some(rec),
        }
    }

    /// A call that only exists as a `functionCall` content part (the record was never written).
    fn part(call: &'a Value, fallback_id: String) -> Self {
        let args = call.get("args").cloned().unwrap_or(Value::Null);
        let id = str_field(call, &["id"]).unwrap_or(fallback_id);
        CallSrc {
            id,
            name: str_field(call, &["name"]).unwrap_or_default(),
            args,
            rec: None,
        }
    }
}

fn record_type(rec: &Rec) -> &str {
    rec.value.get("type").and_then(Value::as_str).unwrap_or("")
}

impl<'a> Build<'a> {
    fn new(red: &'a Reducer, subagent: bool) -> Self {
        let mut call_ids = HashSet::new();
        for rec in red.msgs.values() {
            let calls = array(rec.value.get("toolCalls")).iter();
            let parts = norm(rec.value.get("content"))
                .into_iter()
                .filter_map(|p| p.field("functionCall"));
            call_ids.extend(
                calls
                    .chain(parts)
                    .filter_map(|c| c.get("id").and_then(Value::as_str)),
            );
        }
        Build {
            red,
            subagent,
            call_ids,
            entries: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn run(&mut self) {
        let red = self.red;
        for (i, slot) in red.slots.iter().enumerate() {
            match slot {
                Slot::Msg(id) => self.message(i, id, &red.msgs[id]),
                Slot::Rewind { dropped, line } => {
                    let kind = EntryKind::Rewind(Rewind {
                        dropped: dropped.clone(),
                        turns: None,
                    });
                    self.entries.push(Entry {
                        id: None,
                        parent_id: None,
                        turn_id: None,
                        ts: None,
                        line: *line,
                        extra: None,
                        kind,
                    });
                }
            }
        }
    }

    fn push(&mut self, id: &str, rec: &Rec, extra: Option<Value>, kind: EntryKind) {
        let ts = rec.value.get("timestamp").and_then(ts_ms);
        self.entries.push(Entry {
            id: Some(id.to_string()),
            parent_id: None,
            turn_id: None,
            ts,
            line: rec.line,
            extra,
            kind,
        });
    }

    fn message(&mut self, slot: usize, id: &str, rec: &'a Rec) {
        let kind = record_type(rec);
        let model = kind == "gemini";
        let extra = unmapped(&rec.value, |k| {
            RECORD_KEYS.contains(&k) || (model && MODEL_KEYS.contains(&k))
        });
        match kind {
            "user" => self.user(slot, id, rec, extra),
            "gemini" => self.gemini(id, rec, extra),
            "info" | "warning" => self.notice(id, rec, extra),
            "error" => {
                let message = join_text(rec.value.get("content"));
                let api = message.starts_with("[API Error");
                let error = ErrorInfo {
                    kind: api.then(|| "api_error".to_string()),
                    message,
                    ..Default::default()
                };
                self.push(id, rec, extra, EntryKind::Error(error));
            }
            other => {
                let data = stripped(Value::Object(rec.value.clone()));
                let kind = if other.is_empty() { "unknown" } else { other };
                self.push(
                    id,
                    rec,
                    None,
                    EntryKind::Other(Other {
                        kind: kind.to_string(),
                        data,
                    }),
                );
            }
        }
    }

    /// `info`/`warning`; binary parts in one (the CLI's binary injection) follow as their own message.
    fn notice(&mut self, id: &str, rec: &Rec, extra: Option<Value>) {
        let level = Some(record_type(rec).to_string());
        let text = join_text(rec.value.get("content"));
        self.push(
            id,
            rec,
            extra,
            EntryKind::Notice(Notice {
                level,
                kind: None,
                text,
            }),
        );
        let files: Vec<Part> = norm(rec.value.get("content"))
            .into_iter()
            .filter_map(media)
            .map(Part::Attachment)
            .collect();
        if !files.is_empty() {
            let kind = EntryKind::Message(Message {
                kind: Some("binary_injection".into()),
                ..Message::new(Role::User, Origin::Context, files)
            });
            self.push(&format!("{id}#media"), rec, None, kind);
        }
    }

    /// A user record becomes one entry per run of same-class text parts; echo turns fold away.
    fn user(&mut self, slot: usize, id: &str, rec: &Rec, mut extra: Option<Value>) {
        let groups = self.user_groups(slot, rec);
        let total = groups.len();
        for (n, group) in groups.into_iter().enumerate() {
            let eid = if total == 1 {
                id.to_string()
            } else {
                format!("{id}#{}", n + 1)
            };
            let kind = match group {
                Group::Orphans(parts) => {
                    EntryKind::Message(Message::new(Role::User, Origin::Tool, parts))
                }
                Group::Text(class, parts) => match class.label(self.subagent) {
                    Some((origin, label)) => EntryKind::Message(Message {
                        kind: label.map(str::to_string),
                        ..Message::new(Role::User, origin, parts)
                    }),
                    None => EntryKind::Compaction(Compaction {
                        summary: Some(join_texts(&parts, "\n")),
                        ..Default::default()
                    }),
                },
            };
            self.push(&eid, rec, extra.take(), kind);
        }
    }

    fn user_groups(&mut self, slot: usize, rec: &Rec) -> Vec<Group> {
        let content = norm(rec.value.get("content"));
        let has_response = content.iter().any(|p| p.function_response().is_some());
        let mut orphans: Vec<Part> = content
            .iter()
            .filter_map(|p| p.function_response())
            .filter_map(|fr| self.orphan(fr, rec))
            .collect();
        // Binary parts of an echo turn belong to the call (kept in `echo_media`), or to the orphan result.
        let turn_media: Vec<Media> = content
            .iter()
            .filter(|p| p.function_response().is_none())
            .filter_map(|p| media(*p))
            .collect();
        if let (Some(Part::ToolResult(first)), true) = (orphans.first_mut(), has_response) {
            first.output.attachments.extend(turn_media);
        }
        let ack = self.ack_follows(slot);
        let mut groups = Vec::new();
        if !orphans.is_empty() {
            groups.push(Group::Orphans(orphans));
        }
        for part in content.iter().filter(|p| p.function_response().is_none()) {
            if has_response && media(*part).is_some() {
                continue;
            }
            let Some(converted) = convert(*part) else {
                continue;
            };
            let previous = match groups.last() {
                Some(Group::Text(class, _)) => Some(*class),
                _ => None,
            };
            let class = part
                .text()
                .map(|t| Class::of(t, ack))
                .or(previous)
                .unwrap_or(Class::Human);
            match groups.last_mut() {
                Some(Group::Text(last, parts)) if *last == class => parts.push(converted),
                _ => groups.push(Group::Text(class, vec![converted])),
            }
        }
        if groups.is_empty() && !has_response {
            groups.push(Group::Text(Class::Human, Vec::new()));
        }
        groups
    }

    /// The next record is the model's "Got it" answer to a compression.
    fn ack_follows(&self, slot: usize) -> bool {
        let Some(Slot::Msg(next)) = self.red.slots.get(slot + 1) else {
            return false;
        };
        let next = &self.red.msgs[next];
        record_type(next) == "gemini"
            && join_text(next.value.get("content"))
                .trim_start()
                .starts_with("Got it")
    }

    /// A `functionResponse` whose call is not in the file stays visible as a result part.
    fn orphan(&mut self, response: &Value, rec: &Rec) -> Option<Part> {
        let id = response.get("id").and_then(Value::as_str);
        if id.is_some_and(|id| self.call_ids.contains(id)) {
            return None;
        }
        let call_id = id.unwrap_or_default().to_string();
        self.warnings.push(Warning {
            line: rec.line,
            message: format!("tool result without a call: {call_id}"),
        });
        let result = Value::Array(vec![serde_json::json!({ "functionResponse": response })]);
        let resp = read_resp(&result, &call_id);
        let status = if resp.error.is_some() {
            ToolStatus::Error
        } else {
            ToolStatus::Ok
        };
        let output = ToolOutput {
            text: resp.text(false),
            attachments: resp.attachments,
            ..Default::default()
        };
        Some(Part::ToolResult(ToolResult {
            call_id,
            status,
            status_detail: None,
            output,
        }))
    }

    fn gemini(&mut self, id: &str, rec: &'a Rec, extra: Option<Value>) {
        let m = &rec.value;
        let content = norm(m.get("content"));
        let mut pool: Vec<Option<&'a Value>> = array(m.get("toolCalls")).iter().map(Some).collect();
        let mut parts = Vec::new();
        if !any_thought(&content) {
            parts.extend(thought_parts(m.get("thoughts")));
        }
        let mut orphans = 0;
        for part in content {
            let Some(call) = part.field("functionCall") else {
                parts.extend(convert(part));
                continue;
            };
            let src = match take_call(&mut pool, call) {
                Some(rec) => CallSrc::record(rec),
                None => {
                    orphans += 1;
                    CallSrc::part(call, format!("{id}#call{orphans}"))
                }
            };
            parts.push(Part::ToolCall(self.call(src)));
        }
        parts.extend(
            pool.into_iter()
                .flatten()
                .map(|r| Part::ToolCall(self.call(CallSrc::record(r)))),
        );
        let acknowledged = matches!(
            self.entries.last(),
            Some(Entry {
                kind: EntryKind::Compaction(_),
                ..
            })
        ) && join_text(m.get("content"))
            .trim_start()
            .starts_with("Got it");
        let usage = m
            .get("tokens")
            .filter(|t| t.is_object())
            .map(|t| self.usage(t, rec.line));
        let message = Message {
            kind: acknowledged.then(|| "compaction_ack".into()),
            model: m.get("model").and_then(Value::as_str).map(str::to_string),
            usage,
            ..Message::new(Role::Assistant, Origin::Model, parts)
        };
        self.push(id, rec, extra, EntryKind::Message(message));
    }

    /// Usage convention: `input` is uncached prompt plus built-in tool prompt tokens, `output` includes thoughts.
    fn usage(&mut self, tokens: &Value, line: Option<usize>) -> Usage {
        let get = |key: &str| u64_field(tokens, &[key]).unwrap_or(0);
        let (input, cached, thoughts) = (get("input"), get("cached"), get("thoughts"));
        if cached > input {
            self.warnings.push(Warning {
                line,
                message: format!("cached tokens {cached} exceed input tokens {input}"),
            });
        }
        Usage {
            input: input.saturating_sub(cached).saturating_add(get("tool")),
            output: get("output").saturating_add(thoughts),
            reasoning: thoughts,
            cache_read: cached,
            cache_write: 0,
            cost_usd: None,
        }
    }

    /// The result to read for a call: its own, else the echo turn's; a masked one is replaced by
    /// the original from an earlier line. The flag says a masked result could not be recovered.
    fn result_for(&self, rec: Option<&'a Value>, id: &str) -> (Option<&'a Value>, bool) {
        let own = rec.and_then(|r| r.get("result")).filter(|v| !v.is_null());
        let original = self.red.memory.originals.get(id);
        match own {
            Some(r) if is_masked(r, id) => original.map_or((Some(r), true), |o| (Some(o), false)),
            Some(r) => (Some(r), false),
            None => (original, false),
        }
    }

    fn call(&self, src: CallSrc<'a>) -> ToolCall {
        let rec = src.rec;
        let field = |key: &str| rec.and_then(|r| r.get(key));
        let child = field("agentId").and_then(Value::as_str).map(str::to_string);
        let raw_status = field("status").and_then(Value::as_str);
        let mut tool = map_tool(&src.name, &src.args, child.is_some());
        let input = stripped(src.args);

        let (result, masked) = self.result_for(rec, &src.id);
        let mut resp = result.map(|r| read_resp(r, &src.id));
        if let (Some(resp), Some(echo)) = (resp.as_mut(), self.red.memory.echo_media.get(&src.id)) {
            resp.add_media(echo);
        }
        let (shell, extras) = resp
            .as_ref()
            .and_then(|r| shell_output(&mut tool, r, raw_status))
            .unzip();
        let (status, status_detail) = call_status(raw_status, resp.as_ref(), shell.as_ref());
        let cancelled = is_cancelled(raw_status);
        let display = field("resultDisplay").filter(|_| status == ToolStatus::Ok);
        let files: Vec<_> = display
            .and_then(|d| file_change(d, &input))
            .into_iter()
            .collect();
        let data = call_data(rec, extras.unwrap_or_default(), &input, !files.is_empty());
        // A call without a result still keeps its UI metadata (`displayName`, `description`, ...).
        let output = match resp {
            Some(resp) => {
                let text = resp.text(cancelled);
                let (cut, full_output_path) = truncation(&text);
                Some(ToolOutput {
                    shell,
                    files,
                    attachments: resp.attachments,
                    data,
                    truncated: cut || masked,
                    full_output_path,
                    text,
                })
            }
            None => data.map(|data| ToolOutput {
                data: Some(data),
                ..Default::default()
            }),
        };
        ToolCall {
            status,
            status_detail,
            output,
            ended_at: field("timestamp").and_then(ts_ms),
            child_session: child,
            ..ToolCall::new(src.id, src.name, input, tool)
        }
    }
}

/// The `toolCalls[]` record answering this `functionCall` part: by id, else by name.
fn take_call<'a>(pool: &mut [Option<&'a Value>], call: &Value) -> Option<&'a Value> {
    fn key<'v>(rec: &'v Value, field: &str) -> Option<&'v str> {
        rec.get(field).and_then(Value::as_str)
    }
    let id = key(call, "id");
    let name = key(call, "name");
    let at = match id {
        Some(id) => pool
            .iter()
            .position(|r| r.is_some_and(|r| key(r, "id") == Some(id))),
        None => pool
            .iter()
            .position(|r| r.is_some_and(|r| key(r, "name") == name)),
    }?;
    pool[at].take()
}

/// `Chat history compressed from X to Y tokens.` info notices give the compaction's token counts.
fn link_compaction_tokens(entries: &mut [Entry]) {
    fn counts(text: &str) -> Option<(u64, u64)> {
        let (_, rest) = text.split_once("compressed from ")?;
        let (before, rest) = rest.split_once(" to ")?;
        let num = |s: &str| s.trim().replace(',', "").parse().ok();
        Some((num(before)?, num(rest.split_whitespace().next()?)?))
    }
    let is_open =
        |e: &Entry| matches!(&e.kind, EntryKind::Compaction(c) if c.tokens_before.is_none());
    for i in 0..entries.len() {
        let EntryKind::Notice(notice) = &entries[i].kind else {
            continue;
        };
        let Some((before, after)) = counts(&notice.text) else {
            continue;
        };
        let target = entries[..i]
            .iter()
            .rposition(is_open)
            .or_else(|| entries[i..].iter().position(is_open).map(|p| p + i));
        if let Some(EntryKind::Compaction(c)) = target.map(|t| &mut entries[t].kind) {
            c.tokens_before = Some(before);
            c.tokens_after = Some(after);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Session fields

/// A shell call that names no directory ran in the session directory.
fn default_shell_cwd(entries: &mut [Entry], cwd: Option<&str>) {
    let Some(cwd) = cwd else { return };
    for entry in entries {
        let EntryKind::Message(message) = &mut entry.kind else {
            continue;
        };
        for part in &mut message.parts {
            if let Part::ToolCall(ToolCall {
                tool: Tool::Shell(shell),
                ..
            }) = part
            {
                shell.cwd.get_or_insert_with(|| cwd.to_string());
            }
        }
    }
}

/// Subagent files live at `chats/<parentSessionId>/<agentId>.jsonl`.
fn parent_link(source: &Source) -> Option<ParentLink> {
    let Source::File { path } = source else {
        return None;
    };
    let dir = path.parent()?;
    (dir.parent()?.file_name()? == "chats").then(|| ParentLink {
        session_id: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        call_id: None,
        agent_type: None,
    })
}

/// The cwd: `.project_root` beside the `chats` directory.
fn project_root(source: &Source) -> Option<String> {
    let Source::File { path } = source else {
        return None;
    };
    let chats = path
        .ancestors()
        .skip(1)
        .find(|d| d.file_name().is_some_and(|n| n == "chats"))?;
    let raw = std::fs::read_to_string(chats.parent()?.join(".project_root")).ok()?;
    let root = raw.trim();
    (!root.is_empty()).then(|| root.to_string())
}

/// Fallback cwd: the first workspace directory the session context lists.
fn context_workspace(entries: &[Entry]) -> Option<String> {
    let context = entries.iter().find_map(|e| match &e.kind {
        EntryKind::Message(m) if m.kind.as_deref() == Some("session_context") => Some(m.text()),
        _ => None,
    })?;
    let after = context.split_once("Workspace Directories:")?.1;
    let dir = after
        .lines()
        .find_map(|l| l.trim().strip_prefix("- "))?
        .trim();
    (dir.starts_with('/') || dir.get(1..3) == Some(":\\")).then(|| dir.to_string())
}
