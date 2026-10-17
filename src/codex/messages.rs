//! Input messages, model responses and their usage.

use super::json::{
    add_item_id, item_extra, merge_extra, part_mut, take, take_passthrough, with_meta,
};
use super::{AgentEvent, Parser, Ref, Unmatched};
use crate::util::{
    Obj, base64_len, bool_field, join_texts, media_from_url, str_field, stripped, take_str,
    u64_field,
};
use crate::{EntryKind, Media, Message, Origin, Part, Role, Usage};
use serde_json::{Value, json};

impl Parser {
    // ---- input messages ----

    pub(super) fn message_item(&mut self, mut p: Obj, meta: Option<Value>) {
        let role = take_str(&mut p, "role").unwrap_or_default();
        let id = take_str(&mut p, "id");
        let content = take(&mut p, "content");
        let (turn, kinds) = take_passthrough(&mut p);
        if role == "assistant" {
            let phase = take_str(&mut p, "phase");
            let text = texts(&content).concat();
            return self.agent_text(id, text, phase, turn, with_meta(p, meta));
        }
        let inherited = meta
            .as_ref()
            .is_some_and(|m| bool_field(m, &["inherited_user_message"]));
        self.close_group();
        let mut runs: Vec<(Label, Vec<Part>)> = Vec::new();
        for (i, part) in content_parts(content).into_iter().enumerate() {
            let label = match (&part, runs.last()) {
                (Part::Text { text }, _) => {
                    classify(&role, kinds.get(i).cloned().flatten(), text, inherited)
                }
                (_, Some((last, _))) => last.clone(),
                (_, None) => classify(&role, kinds.get(i).cloned().flatten(), "", inherited),
            };
            match runs.last_mut() {
                Some((last, parts)) if *last == label => parts.push(part),
                _ => runs.push((label, vec![part])),
            }
        }
        let n = runs.len();
        let mut extra = Some(with_meta(p, meta));
        for (i, (label, parts)) in runs.into_iter().enumerate() {
            let id = id.as_ref().map(|id| {
                if n > 1 {
                    format!("{id}#{}", i + 1)
                } else {
                    id.clone()
                }
            });
            self.input_run(
                label,
                parts,
                id,
                turn.clone(),
                extra.take().unwrap_or_default(),
            );
        }
    }

    /// One run of same-labelled parts of an input message.
    fn input_run(
        &mut self,
        label: Label,
        parts: Vec<Part>,
        id: Option<String>,
        turn: Option<String>,
        extra: Obj,
    ) {
        let (role, origin, kind) = label;
        if matches!(
            kind.as_deref(),
            Some("generic.turn_aborted" | "turn_aborted")
        ) {
            return self.abort_marker(join_texts(&parts, "\n"), id, turn, extra);
        }
        if let Some(r) = id.as_ref().and_then(|id| self.items.get(id)).copied() {
            return self.merge_copy(r.entry, None, extra);
        }
        let human = origin == Origin::Human;
        let key = prompt_key(&parts);
        if human {
            if let Some(e) = self.take_unmatched(true, &key) {
                return self.merge_copy(e, id, extra);
            }
        }
        let message = EntryKind::Message(Message {
            kind,
            ..Message::new(role, origin, parts)
        });
        let entry = self.push(id.clone(), turn, message, extra);
        if let Some(id) = id {
            self.items.insert(id, Ref { entry, part: 0 });
        }
        if human {
            self.remember(false, key, entry);
        }
    }

    /// A user prompt as an event (`UserMessage` item, legacy `user_message`): the trusted copy.
    pub(super) fn user_event(
        &mut self,
        id: Option<String>,
        parts: Vec<Part>,
        turn: Option<String>,
        extra: Obj,
    ) {
        let key = prompt_key(&parts);
        if let Some(e) = self.take_unmatched(false, &key) {
            return self.merge_copy(e, id, extra);
        }
        self.close_group();
        let entry = self.push(
            id,
            turn,
            EntryKind::Message(Message::new(Role::User, Origin::Human, parts)),
            extra,
        );
        self.remember(true, key, entry);
    }

    /// The second record of an input recorded twice joins the entry of the first.
    fn merge_copy(&mut self, entry: usize, id: Option<String>, extra: Obj) {
        let e = &mut self.s.entries[entry];
        if let Some(id) = id {
            add_item_id(e, id);
        }
        merge_extra(e, extra);
    }

    pub(super) fn legacy_user(&mut self, mut p: Obj) {
        let mut parts: Vec<Part> = take_str(&mut p, "message")
            .map(|text| Part::Text { text })
            .into_iter()
            .collect();
        for key in ["images", "local_images"] {
            let Some(Value::Array(list)) = p.remove(key) else {
                continue;
            };
            let urls = list.iter().map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| str_field(v, &["path"]))
            });
            parts.extend(urls.map(|url| Part::Attachment(media(url))));
        }
        self.user_event(None, parts, None, p)
    }

    /// A `HookPrompt` item. Codex also records its prompt as a user message with the same id;
    /// whichever comes second joins the first.
    pub(super) fn hook_prompt(&mut self, mut item: Obj) {
        let turn = take_str(&mut item, "turn_id");
        let id = take_str(&mut item, "id");
        if let Some(r) = id.as_ref().and_then(|id| self.items.get(id)).copied() {
            return self.merge_copy(r.entry, None, item);
        }
        self.close_group();
        let text = texts(&take(&mut item, "fragments")).join("\n");
        let m = Message {
            kind: Some("hook_prompt".into()),
            ..Message::new(Role::User, Origin::Notification, vec![Part::Text { text }])
        };
        let entry = self.push(id.clone(), turn, EntryKind::Message(m), item);
        if let Some(id) = id {
            self.items.insert(id, Ref { entry, part: 0 });
        }
    }

    /// `agent_message` (multi-agent v2): the session's own when its author is this agent's path.
    pub(super) fn agent_message(&mut self, mut p: Obj, meta: Option<Value>) {
        let id = take_str(&mut p, "id");
        let (turn, _) = take_passthrough(&mut p);
        let author = take_str(&mut p, "author");
        let mut parts = content_parts(take(&mut p, "content"));
        if let Some(Value::String(blob)) = p.remove("encrypted_content") {
            parts.push(Part::Other {
                kind: "encrypted_content".into(),
                data: json!({ "bytes": base64_len(&blob) }),
            });
        }
        let own = author.as_deref() == Some(self.own_path.as_str());
        if !own {
            self.close_group();
            // A header with nothing after `Payload:` and no encrypted body says nothing.
            if matches!(parts.as_slice(), [Part::Text { text }] if bare_header(text)) {
                return;
            }
        }
        // This session's own message to another agent is model output; a received one is input.
        let mut m = if own {
            Message::new(Role::Assistant, Origin::Model, parts)
        } else {
            let kind = message_type(&parts);
            Message {
                kind,
                ..Message::new(Role::User, Origin::Agent, parts)
            }
        };
        m.author = author;
        self.push(id, turn, EntryKind::Message(m), with_meta(p, meta));
    }

    fn take_unmatched(&mut self, event: bool, key: &str) -> Option<usize> {
        let i = (self.unmatched.iter()).position(|u| u.event == event && u.key == key)?;
        self.unmatched.remove(i).map(|u| u.entry)
    }

    fn remember(&mut self, event: bool, key: String, entry: usize) {
        if self.unmatched.len() == 32 {
            self.unmatched.pop_front();
        }
        self.unmatched.push_back(Unmatched { event, key, entry });
    }

    /// A legacy `agent_message` event: it repeats the response item that follows it (one event per
    /// text part), so it waits to be matched and is emitted only when no item claims it.
    pub(super) fn agent_event(&mut self, text: String, phase: Option<String>, extra: Obj) {
        let (rec, turn) = (self.rec.clone(), self.turn.clone());
        self.agent_events.push(AgentEvent {
            text,
            phase,
            extra,
            rec,
            turn,
        });
    }

    /// The buffered events whose texts, in order, make up `text`: their phase and leftovers.
    fn take_agent_events(&mut self, text: &str) -> Option<(Option<String>, Obj)> {
        for start in 0..self.agent_events.len() {
            let mut joined = String::new();
            for end in start..self.agent_events.len() {
                joined.push_str(&self.agent_events[end].text);
                if joined == text {
                    let (mut phase, mut extra) = (None, Obj::new());
                    for ev in self.agent_events.drain(start..=end) {
                        phase = phase.or(ev.phase);
                        extra.extend(ev.extra);
                    }
                    return Some((phase, extra));
                }
                if !text.starts_with(&joined) {
                    break;
                }
            }
        }
        None
    }

    // ---- model responses ----

    /// The message of the open response that a part with `phase` goes to; a second phase in
    /// one response starts a second message.
    fn response_message(
        &mut self,
        id: Option<String>,
        turn: Option<String>,
        phase: Option<&str>,
    ) -> usize {
        if let Some(&last) = self.group.last() {
            if let EntryKind::Message(m) = &mut self.s.entries[last].kind {
                match (m.phase.as_deref(), phase) {
                    (Some(a), Some(b)) if a != b => {}
                    _ => {
                        m.phase = m.phase.take().or(phase.map(str::to_owned));
                        return last;
                    }
                }
            }
        }
        let mut m = Message::new(Role::Assistant, Origin::Model, Vec::new());
        m.model = self.model.clone();
        m.phase = phase.map(str::to_owned);
        let entry = self.push(id, turn, EntryKind::Message(m), Obj::new());
        self.group.push(entry);
        entry
    }

    pub(super) fn add_part(
        &mut self,
        part: Part,
        id: Option<String>,
        turn: Option<String>,
        phase: Option<&str>,
        extra: Obj,
    ) -> Ref {
        self.flush_reasoning();
        let entry = self.response_message(id.clone(), turn, phase);
        let e = &mut self.s.entries[entry];
        if let Some(id) = id {
            add_item_id(e, id);
        }
        merge_extra(e, extra);
        let part = match &mut e.kind {
            EntryKind::Message(m) => {
                m.parts.push(part);
                m.parts.len() - 1
            }
            _ => usize::MAX,
        };
        Ref { entry, part }
    }

    fn flush_reasoning(&mut self) {
        for text in std::mem::take(&mut self.reasoning) {
            self.add_part(
                Part::Reasoning {
                    text: Some(text),
                    encrypted: false,
                },
                None,
                None,
                None,
                Obj::new(),
            );
        }
    }

    pub(super) fn close_group(&mut self) {
        self.flush_reasoning();
        // Legacy assistant events no response item claimed: their text must not be lost.
        for ev in std::mem::take(&mut self.agent_events) {
            let current = std::mem::replace(&mut self.rec, ev.rec);
            let text = Part::Text { text: ev.text };
            self.add_part(text, None, ev.turn, ev.phase.as_deref(), ev.extra);
            self.rec = current;
        }
        if !self.group.is_empty() {
            self.last_group = std::mem::take(&mut self.group);
        }
    }

    /// Assistant text from a response item or an `AgentMessage` item. Both carry the item id, so
    /// the second one merges into the first. A legacy event copy without an id was buffered and
    /// joins here by its text.
    pub(super) fn agent_text(
        &mut self,
        id: Option<String>,
        text: String,
        phase: Option<String>,
        turn: Option<String>,
        mut extra: Obj,
    ) {
        if let Some(r) = id.as_ref().and_then(|id| self.items.get(id)).copied() {
            let e = &mut self.s.entries[r.entry];
            if let EntryKind::Message(m) = &mut e.kind {
                m.phase = m.phase.take().or(phase);
            }
            return merge_extra(e, item_extra(id.as_deref(), extra));
        }
        let mut phase = phase;
        if let Some((event_phase, event_extra)) = self.take_agent_events(&text) {
            phase = phase.or(event_phase);
            extra.extend(event_extra);
        }
        let extra = item_extra(id.as_deref(), extra);
        let r = self.add_part(
            Part::Text { text },
            id.clone(),
            turn,
            phase.as_deref(),
            extra,
        );
        if let Some(id) = id {
            self.items.insert(id, r);
        }
    }

    pub(super) fn reasoning_item(&mut self, mut p: Obj, meta: Option<Value>) {
        let id = take_str(&mut p, "id");
        let (turn, _) = take_passthrough(&mut p);
        let mut text = texts(&take(&mut p, "summary"));
        text.extend(texts(&take(&mut p, "content")));
        let encrypted =
            matches!(p.remove("encrypted_content"), Some(Value::String(s)) if !s.is_empty());
        self.reasoning.retain(|t| !text.contains(t));
        self.add_reasoning(id, text, encrypted, turn, with_meta(p, meta));
    }

    pub(super) fn add_reasoning(
        &mut self,
        id: Option<String>,
        text: Vec<String>,
        encrypted: bool,
        turn: Option<String>,
        extra: Obj,
    ) {
        let text = Some(text.join("\n\n")).filter(|t| !t.is_empty());
        if let Some(r) = id.as_ref().and_then(|id| self.items.get(id)).copied() {
            if let Some(Part::Reasoning {
                text: t,
                encrypted: e,
            }) = part_mut(&mut self.s.entries, r)
            {
                *e |= encrypted;
                *t = t.take().or(text);
            }
            let extra = item_extra(id.as_deref(), extra);
            return merge_extra(&mut self.s.entries[r.entry], extra);
        }
        let extra = item_extra(id.as_deref(), extra);
        let r = self.add_part(
            Part::Reasoning { text, encrypted },
            id.clone(),
            turn,
            None,
            extra,
        );
        if let Some(id) = id {
            self.items.insert(id, r);
        }
    }

    // ---- usage ----

    fn usage(&mut self, u: &Value) -> Usage {
        let n = |k: &str| u64_field(u, &[k]).unwrap_or(0);
        let (input, cached, write) = (
            n("input_tokens"),
            n("cached_input_tokens"),
            n("cache_write_input_tokens"),
        );
        if cached.saturating_add(write) > input {
            self.warn(
                self.rec.line,
                format!("usage: cached tokens {cached}+{write} exceed input tokens {input}"),
            );
        }
        Usage {
            input: input.saturating_sub(cached).saturating_sub(write),
            output: n("output_tokens"),
            reasoning: n("reasoning_output_tokens"),
            cache_read: cached,
            cache_write: write,
            cost_usd: None,
        }
    }

    /// The authoritative per-response usage (0.153+). Every record counts; a thread total that
    /// goes down is kept and reported.
    pub(super) fn usage_record(&mut self, mut p: Obj) {
        let raw = take(&mut p, "usage");
        let own = u64_field(&raw, &["total_tokens"]).unwrap_or(0);
        let thread = p
            .get("thread_token_usage")
            .and_then(|t| u64_field(t, &["total_tokens"]));
        if let Some(total) = thread.filter(|t| *t < self.cum_total) {
            let line = self.rec.line;
            let message = format!(
                "usage: thread total fell from {} to {total}",
                self.cum_total
            );
            self.warn(line, message);
        }
        self.cum_total = thread.unwrap_or_else(|| self.cum_total.saturating_add(own));
        self.records_seen = true;
        let usage = self.usage(&raw);
        let response = take_str(&mut p, "response_id");
        for k in ["thread_id", "session_id", "turn_id"] {
            p.remove(k);
        }
        self.apply_usage(usage, response, p);
    }

    /// Before `token_usage_record` existed, a `token_count` with a new cumulative total is the
    /// only per-response usage. It is re-emitted with identical totals, so dedupe by total.
    pub(super) fn token_count(&mut self, p: Obj) {
        let info = p.get("info").cloned().unwrap_or_default();
        let total = info
            .get("total_token_usage")
            .and_then(|t| u64_field(t, &["total_tokens"]));
        let last = info.get("last_token_usage");
        if let (Some(total), Some(last), false) = (total, last, self.records_seen) {
            if total > self.cum_total {
                self.cum_total = total;
                let usage = self.usage(last);
                self.apply_usage(usage, None, Obj::new());
            }
        }
        self.other("token_count", p);
    }

    /// Usage of one model response: the compaction it belongs to, else the open response, else
    /// the one closed before it when it has none yet, else an open compaction without usage, else
    /// an empty assistant message. A response split by phase keeps its usage on the first piece.
    fn apply_usage(&mut self, usage: Usage, response: Option<String>, extra: Obj) {
        self.flush_reasoning();
        if let Some(&c) = response.as_ref().and_then(|r| self.compactions.get(r)) {
            return self.compaction_usage(c, usage, extra);
        }
        let mut group = std::mem::take(&mut self.group);
        let last_unused = self.last_group.first().is_some_and(
            |&i| matches!(&self.s.entries[i].kind, EntryKind::Message(m) if m.usage.is_none()),
        );
        if group.is_empty() && last_unused {
            group = std::mem::take(&mut self.last_group);
        }
        if group.is_empty() {
            let open = self.last_entry().filter(|&c| {
                matches!(&self.s.entries[c].kind, EntryKind::Compaction(c) if c.usage.is_none())
            });
            if let Some(c) = open {
                return self.compaction_usage(c, usage, extra);
            }
            group.push(self.response_message(None, None, None));
            self.group.clear();
        }
        let n = group.len();
        for (i, &entry) in group.iter().enumerate() {
            if let Some(id) = &response {
                self.s.entries[entry].id = Some(if n == 1 {
                    id.clone()
                } else {
                    format!("{id}#{}", i + 1)
                });
            }
        }
        let first = &mut self.s.entries[group[0]];
        if let EntryKind::Message(m) = &mut first.kind {
            m.usage = Some(usage);
        }
        merge_extra(first, extra);
        if let Some(id) = response {
            self.responses.insert(id, group[0]);
        }
        self.last_group = group;
    }

    pub(super) fn compaction_usage(&mut self, c: usize, usage: Usage, extra: Obj) {
        let e = &mut self.s.entries[c];
        if let EntryKind::Compaction(compaction) = &mut e.kind {
            compaction.usage = Some(usage);
        }
        merge_extra(e, extra);
    }
}

// ---- message helpers ----

/// Role, origin and product kind of a run of input parts.
type Label = (Role, Origin, Option<String>);

/// `content_item_kinds` classify each input part; older files have none, so leading tags do.
fn classify(role: &str, kind: Option<String>, text: &str, inherited: bool) -> Label {
    let kind = kind.filter(|k| k != "unknown").or_else(|| sniff(text));
    let system = matches!(role, "developer" | "system");
    match kind.as_deref() {
        None | Some("user.text") if !system => {
            let origin = if inherited {
                Origin::Agent
            } else {
                Origin::Human
            };
            (Role::User, origin, kind.filter(|_| inherited))
        }
        Some(k) if k.contains("hook") => (Role::User, Origin::Notification, kind),
        Some(k) if k.contains("shell_command") => (Role::User, Origin::Command, kind),
        _ if system => (Role::System, Origin::Context, kind),
        _ => (Role::User, Origin::Context, kind),
    }
}

/// The wrapper tag of injected context, for files without `content_item_kinds`.
fn sniff(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("# AGENTS.md instructions for") {
        return Some("agents_md".into());
    }
    let tag = t.strip_prefix('<')?.split(['>', ' ']).next()?;
    let known = tag.contains('_') || tag == "permissions";
    let wrapped =
        tag.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') && t.contains(&format!("</{tag}"));
    (known && wrapped).then(|| tag.to_owned())
}

fn bare_header(text: &str) -> bool {
    text.starts_with("Message Type: ")
        && text
            .split_once("Payload:")
            .is_some_and(|(_, payload)| payload.trim().is_empty())
}

/// `Message Type: NEW_TASK` header of an inter-agent message.
fn message_type(parts: &[Part]) -> Option<String> {
    parts.iter().find_map(|p| match p {
        Part::Text { text } => text
            .lines()
            .next()?
            .strip_prefix("Message Type: ")
            .map(str::to_owned),
        _ => None,
    })
}

/// What the two copies of a user prompt share: the text inputs joined as Codex joins them for
/// the event, without the tags it writes around an image or audio input in the response item.
fn prompt_key(parts: &[Part]) -> String {
    let tag = |t: &str| {
        matches!(t, "<image>" | "</image>" | "<audio>" | "</audio>")
            || (t.starts_with("<image name=") || t.starts_with("<audio name=")) && t.ends_with('>')
    };
    let texts = parts.iter().filter_map(|p| match p {
        Part::Text { text } if !tag(text) => Some(text.as_str()),
        _ => None,
    });
    texts.collect()
}

/// One part per content item, so parts stay aligned with `content_item_kinds`.
pub(super) fn content_parts(content: Value) -> Vec<Part> {
    match content {
        Value::String(text) => vec![Part::Text { text }],
        Value::Array(items) => items.into_iter().map(content_part).collect(),
        _ => Vec::new(),
    }
}

pub(super) fn content_part(c: Value) -> Part {
    let ty = c
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match ty.as_str() {
        "input_text" | "output_text" | "text" | "Text" => Part::Text {
            text: str_field(&c, &["text"]).unwrap_or_default(),
        },
        "input_image" | "image" => {
            Part::Attachment(media(str_field(&c, &["image_url", "file_id"])))
        }
        "local_image" => Part::Attachment(Media {
            url: str_field(&c, &["path"]),
            ..Default::default()
        }),
        _ => Part::Other {
            kind: ty,
            data: stripped(c),
        },
    }
}

fn media(url: Option<String>) -> Media {
    url.map_or_else(Media::default, |url| media_from_url(&url, None, None))
}

/// Texts of a list of strings or `{text}` objects.
pub(super) fn texts(v: &Value) -> Vec<String> {
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|i| {
            i.as_str()
                .map(str::to_owned)
                .or_else(|| str_field(i, &["text"]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_tags() {
        let env = "<environment_context>\n<cwd>/x</cwd>\n</environment_context>";
        assert_eq!(sniff(env).as_deref(), Some("environment_context"));
        assert_eq!(
            sniff("# AGENTS.md instructions for /x\n\nrules").as_deref(),
            Some("agents_md")
        );
        assert_eq!(sniff("<div>hello</div>"), None);
        assert_eq!(sniff("plain prompt"), None);
    }
}
