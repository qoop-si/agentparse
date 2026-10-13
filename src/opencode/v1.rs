//! v1 `message`/`part` rows and the JSON storage tree (same shapes): one assistant message is
//! one step.

use super::tool::v1_call;
use super::{Map, db::V1Message, record::*};
use crate::model::*;
use serde_json::Value;

/// The synthetic user text OpenCode writes in front of a human-run `!command`.
const COMMAND_MARKER: &str = "The following tool was executed by the user";

pub(super) fn v1_entries(messages: Vec<V1Message>) -> Vec<Entry> {
    let total = messages.len();
    let mut slots: Vec<Option<V1Message>> = messages.into_iter().map(Some).collect();
    let mut out = Vec::new();
    for i in 0..total {
        let Some(m) = slots[i].take() else { continue };
        match m.data.get("role").and_then(Value::as_str) {
            Some("user") => out.extend(user_entries(m, &mut slots, i)),
            Some("assistant") => out.extend(assistant_entries(m)),
            _ => {
                let kind = EntryKind::Other(Other {
                    kind: "message".into(),
                    data: Value::Object(m.data),
                });
                out.push(entry(Some(m.id), m.created, None, Map::new(), kind));
            }
        }
    }
    out
}

/// Take every later message that satisfies `pred` out of its slot.
fn take_where(
    slots: &mut [Option<V1Message>],
    from: usize,
    pred: impl Fn(&V1Message) -> bool,
) -> Vec<V1Message> {
    let mut taken = Vec::new();
    for slot in slots.iter_mut().skip(from) {
        if slot.as_ref().is_some_and(&pred) {
            taken.extend(slot.take());
        }
    }
    taken
}

/// Take only the first later message that satisfies `pred`; the others stay in their slots.
fn take_first(
    slots: &mut [Option<V1Message>],
    from: usize,
    pred: impl Fn(&V1Message) -> bool,
) -> Option<V1Message> {
    let at = slots
        .iter()
        .enumerate()
        .skip(from)
        .find_map(|(i, s)| s.as_ref().is_some_and(&pred).then_some(i))?;
    slots[at].take()
}

fn replies_to<'a>(parent: &'a str, summary: Option<bool>) -> impl Fn(&V1Message) -> bool + 'a {
    move |m| {
        m.data.get("parentID").and_then(Value::as_str) == Some(parent)
            && summary.is_none_or(|s| (m.data.get("summary") == Some(&Value::Bool(true))) == s)
    }
}

fn user_entries(m: V1Message, slots: &mut [Option<V1Message>], index: usize) -> Vec<Entry> {
    let is_marker = m.parts.iter().any(|p| {
        is_part(p, "text") && p.get("text").and_then(Value::as_str) == Some(COMMAND_MARKER)
    });
    if is_marker {
        let pair = take_where(slots, index + 1, replies_to(&m.id, None));
        if !pair.is_empty() {
            return vec![command_entry(m, pair)];
        }
    }
    let V1Message {
        id,
        created,
        mut data,
        mut parts,
    } = m;
    data.remove("role");
    let ts = take_time(&mut data).or(created);
    let mut entries = Vec::new();
    if let Some(at) = parts.iter().position(|p| is_part(p, "compaction")) {
        let compaction = parts.remove(at);
        let summary = take_first(slots, index + 1, replies_to(&id, Some(true)));
        entries.extend(compaction_entry(&id, ts, compaction, summary));
        if parts.is_empty() {
            return entries;
        }
    }
    entries.extend(user_messages(&id, ts, data, parts));
    entries
}

/// How a user part is attributed: typed by the human, or text OpenCode injected or hid.
fn user_class(p: &Map) -> (Origin, Option<&'static str>) {
    let flag = |f: &str| p.get(f) == Some(&Value::Bool(true));
    match (is_part(p, "text"), flag("synthetic"), flag("ignored")) {
        (true, true, _) => (Origin::Context, Some("synthetic")),
        (true, false, true) => (Origin::Context, Some("ignored")),
        _ => (Origin::Human, None),
    }
}

/// A user record becomes one message per run of parts with the same origin and kind.
fn user_messages(id: &str, ts: Option<i64>, mut data: Map, parts: Vec<Map>) -> Vec<Entry> {
    let agent = take_str(&mut data, "agent");
    let model = take_nested_str(&mut data, "model", "modelID");
    // Each group: origin, kind, parts, and the leftover keys of its text parts by position.
    type Group = (Origin, Option<&'static str>, Vec<Part>, Map);
    let mut groups: Vec<Group> = Vec::new();
    for p in parts {
        let (origin, kind) = user_class(&p);
        let (part, leftover) = v1_part(p, None, true);
        match groups.last_mut() {
            Some(g) if (g.0, g.1) == (origin, kind) => {
                note_leftover(&mut g.3, g.2.len(), leftover);
                g.2.push(part);
            }
            _ => {
                let mut extras = Map::new();
                note_leftover(&mut extras, 0, leftover);
                groups.push((origin, kind, vec![part], extras));
            }
        }
    }
    if groups.is_empty() {
        groups.push((Origin::Human, None, Vec::new(), Map::new()));
    }
    let split = groups.len() > 1;
    let mut entries = Vec::new();
    for (i, (origin, kind, parts, extras)) in groups.into_iter().enumerate() {
        let entry_id = if split {
            format!("{id}#{}", i + 1)
        } else {
            id.to_string()
        };
        let msg = Message {
            kind: kind.map(str::to_string),
            model: model.clone(),
            agent: agent.clone(),
            ..Message::new(Role::User, origin, parts)
        };
        // The record's unmapped keys belong to its first message only.
        let mut extra = if i == 0 {
            std::mem::take(&mut data)
        } else {
            Map::new()
        };
        attach_part_extras(&mut extra, extras);
        // A prompt starts its turn, so its turn id is its own (unsplit) id.
        entries.push(entry(
            Some(entry_id),
            ts,
            Some(id.to_string()),
            extra,
            EntryKind::Message(msg),
        ));
    }
    entries
}

/// A human-run `!command`: the marker user message plus the assistant message holding the
/// shell call become one `Command` message. The entry keeps the marker's id (other records,
/// such as `tail_start_id`, point at it); the assistant id is `extra.message_id` and the
/// marker's own unmapped keys are `extra.user`.
fn command_entry(user: V1Message, pair: Vec<V1Message>) -> Entry {
    let V1Message {
        id: user_id,
        created,
        data: mut user_data,
        ..
    } = user;
    user_data.remove("role");
    let ts = take_time(&mut user_data).or(created);
    let mut agent = take_str(&mut user_data, "agent");
    let model = take_nested_str(&mut user_data, "model", "modelID");
    let mut parts = Vec::new();
    let (mut message_id, mut extra, mut part_extras) = (None, Map::new(), Map::new());
    for m in pair {
        let V1Message {
            id,
            mut data,
            parts: raw_parts,
            ..
        } = m;
        data.remove("role");
        take_time(&mut data);
        take_str(&mut data, "parentID");
        let cwd = path_cwd(&data);
        message_id.get_or_insert(id);
        let assistant_agent = take_agent(&mut data);
        agent = agent.or(assistant_agent);
        extra.extend(data);
        for p in raw_parts {
            let (part, leftover) = v1_part(p, cwd.as_deref(), false);
            note_leftover(&mut part_extras, parts.len(), leftover);
            parts.push(part);
        }
    }
    attach_part_extras(&mut extra, part_extras);
    if let Some(id) = message_id {
        extra.insert("message_id".into(), Value::String(id));
    }
    if !user_data.is_empty() {
        extra.insert("user".into(), Value::Object(user_data));
    }
    let msg = Message {
        agent,
        model,
        ..Message::new(Role::User, Origin::Command, parts)
    };
    entry(Some(user_id), ts, None, extra, EntryKind::Message(msg))
}

/// The compaction marker (user side) and the summary the paired assistant message wrote. A
/// failed summary also yields its error as an entry.
fn compaction_entry(
    user_id: &str,
    ts: Option<i64>,
    mut marker: Map,
    summary: Option<V1Message>,
) -> Vec<Entry> {
    // `auto` is recorded as a flag; no flag, no trigger.
    let trigger = take_with(&mut marker, "auto", Value::as_bool)
        .map(|auto| if auto { "auto" } else { "manual" }.to_string());
    marker.remove("type");
    let mut compaction = Compaction {
        trigger,
        ..Default::default()
    };
    let mut error = None;
    if let Some(V1Message {
        id,
        mut data,
        parts,
        ..
    }) = summary
    {
        compaction.summary = Some(
            parts
                .iter()
                .filter(|p| is_part(p, "text"))
                .filter_map(|p| p.get("text")?.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        compaction.usage = usage(data.remove("tokens"), take_f64(&mut data, "cost"));
        error = data.remove("error");
        for key in ["role", "parentID", "summary", "finish"] {
            data.remove(key);
        }
        data.insert("id".into(), Value::String(id));
        marker.insert("summary_message".into(), Value::Object(data));
    }
    let mut entries = vec![entry(
        Some(user_id.to_string()),
        ts,
        None,
        marker,
        EntryKind::Compaction(compaction),
    )];
    entries.extend(error.map(|e| failure_entry(v1_error(e), None, ts, None)));
    entries
}

fn path_cwd(data: &Map) -> Option<String> {
    data.get("path")?.get("cwd")?.as_str().map(str::to_string)
}

/// `agent`, with the legacy `mode` as fallback; `mode` stays as unmapped when it differs.
fn take_agent(data: &mut Map) -> Option<String> {
    let agent = take_str(data, "agent");
    let mode = data.get("mode").and_then(Value::as_str);
    match (&agent, mode) {
        (_, Some(m)) if agent.as_deref() == Some(m) => {
            data.remove("mode");
            agent
        }
        (None, Some(_)) => take_str(data, "mode"),
        _ => agent,
    }
}

fn assistant_entries(m: V1Message) -> Vec<Entry> {
    let V1Message {
        id,
        created,
        mut data,
        parts,
    } = m;
    data.remove("role");
    let done = time_of(&data, "completed");
    let ts = take_time(&mut data).or(created);
    let turn_id = take_str(&mut data, "parentID");
    let cwd = path_cwd(&data);
    let phase = parts
        .iter()
        .rev()
        .filter(|p| is_part(p, "text"))
        .find_map(|p| phase_of(p.get("metadata")));
    let model = take_str(&mut data, "modelID");
    let agent = take_agent(&mut data);
    let usage = usage(data.remove("tokens"), take_f64(&mut data, "cost"));
    let stop_reason = take_str(&mut data, "finish");
    let error = data.remove("error");

    // Retry notices and the final error become entries right after the message.
    let mut follow_ups = Vec::new();
    let mut msg_parts = Vec::new();
    // Leftover keys of text and reasoning parts, by position in the message.
    let mut extras = Map::new();
    for p in parts {
        if is_part(&p, "retry") {
            let mut retry = retry_entry(p);
            retry.turn_id = turn_id.clone();
            follow_ups.push(retry);
        } else {
            let (part, leftover) = v1_part(p, cwd.as_deref(), false);
            note_leftover(&mut extras, msg_parts.len(), leftover);
            msg_parts.push(part);
        }
    }
    attach_part_extras(&mut data, extras);
    let msg = Message {
        model,
        agent,
        phase,
        usage,
        stop_reason,
        ..Message::new(Role::Assistant, Origin::Model, msg_parts)
    };
    let mut entries = vec![entry(
        Some(id),
        ts,
        turn_id.clone(),
        data,
        EntryKind::Message(msg),
    )];
    entries.extend(follow_ups);
    entries.extend(error.map(|e| failure_entry(v1_error(e), None, done.or(ts), turn_id)));
    entries
}

/// One v1 part (`retry` parts are entries, see `retry_entry`) and, for text and reasoning,
/// the keys no field took. `user` parts keep flagged text as text (the message carries the
/// flag); on an assistant message a synthetic or ignored text stays visible as its own
/// `text.<flag>` part.
fn v1_part(mut p: Map, cwd: Option<&str>, user: bool) -> (Part, Option<Value>) {
    let kind = take_str(&mut p, "type").unwrap_or_default();
    // JSON storage part files carry these ids inline (database rows have them as columns).
    for key in ["id", "sessionID", "messageID"] {
        p.remove(key);
    }
    match kind.as_str() {
        "text" => {
            let flagged = ["synthetic", "ignored"]
                .into_iter()
                .find(|f| p.get(*f) == Some(&Value::Bool(true)));
            match flagged {
                Some(flag) if !user => (other_part(format!("text.{flag}"), p), None),
                _ => {
                    let text = take_str(&mut p, "text").unwrap_or_default();
                    (Part::Text { text }, part_leftover(p))
                }
            }
        }
        "reasoning" => {
            let text = take_str(&mut p, "text").filter(|t| !t.is_empty());
            let encrypted = p.get("metadata").is_some_and(has_opaque);
            (Part::Reasoning { text, encrypted }, part_leftover(p))
        }
        "file" => (Part::Attachment(media_from_file(p)), None),
        "tool" => {
            let (call, leftover) = v1_call(p, cwd);
            (Part::ToolCall(call), leftover)
        }
        _ => (other_part(kind, p), None),
    }
}

fn retry_entry(mut p: Map) -> Entry {
    let ts = take_time(&mut p);
    let attempt = take_u64(&mut p, "attempt").and_then(|a| u32::try_from(a).ok());
    let error = v1_error(p.remove("error").unwrap_or(Value::Null));
    let info = ErrorInfo {
        kind: error.kind,
        message: error.message,
        status: error.status,
        retry: attempt,
    };
    let mut extra = error.rest;
    extra.extend(p);
    entry(None, ts, None, extra, EntryKind::Error(info))
}
