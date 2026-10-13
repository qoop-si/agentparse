//! The record reducer: the CLI's own loader (`createJsonlRecordAccumulator`) ported, with the
//! full view on top. Every message id ever seen stays at its last version in first-seen order;
//! the live list (what a resume would load) is kept next to it.

use super::Obj;
use super::part::{P, array, media, norm, response_for};
use crate::util::json_lines;
use crate::{Media, Warning};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Marker the CLI writes over old tool results to save context.
const MASKED: &str = "<tool_output_masked>";

/// One message id at its last version.
pub(super) struct Rec {
    pub value: Obj,
    /// 1-based source line where the id first appeared (`None` for a whole-document file).
    pub line: Option<usize>,
}

pub(super) enum Slot {
    Msg(String),
    /// Turns left the live history here.
    Rewind {
        dropped: Vec<String>,
        line: Option<usize>,
    },
}

#[derive(Default)]
struct Seen {
    /// Messages inside the header record (a one-document G1 file).
    header_messages: bool,
    /// A `$set.messages` full snapshot (G2).
    snapshot: bool,
    /// A `$patch` or `$rewindTo` line (G3).
    patched: bool,
    /// A bare message record.
    bare: bool,
}

/// What earlier lines knew about tool calls that later lines lost or never repeated.
#[derive(Default)]
pub(super) struct Memory {
    /// First unmasked result of every tool call id, so a result masked later is recovered from
    /// the line that still had it.
    pub originals: HashMap<String, Value>,
    /// Binary parts that came back in a call's echo turn.
    pub echo_media: HashMap<String, Vec<Media>>,
}

#[derive(Default)]
pub(super) struct Reducer {
    pub meta: Obj,
    /// Every id ever seen, last version.
    pub msgs: HashMap<String, Rec>,
    /// First-seen order of ids, with the rewinds where they happened.
    pub slots: Vec<Slot>,
    pub memory: Memory,
    pub warnings: Vec<Warning>,
    /// The CLI's own message order (a `Map` upstream): what a resume would load.
    live: Vec<String>,
    live_set: HashSet<String>,
    seen: Seen,
}

impl Reducer {
    /// A single JSON document is the one record of a G1 file; otherwise every line is a record.
    pub(super) fn read(&mut self, text: &str) -> std::io::Result<()> {
        if let Ok(doc @ Value::Object(_)) = serde_json::from_str::<Value>(text) {
            self.apply(None, doc);
            return Ok(());
        }
        for item in json_lines(text.as_bytes()) {
            match item? {
                (line, Ok(record)) => self.apply(Some(line), record),
                (line, Err(e)) => self.warn(Some(line), format!("not json: {e}")),
            }
        }
        Ok(())
    }

    /// `gemini-g3` once a `$patch`/`$rewindTo` line exists or messages are appended without any
    /// `$set.messages` snapshot; `gemini-g1` for one document; `gemini-g2` otherwise (also
    /// for a header-only file). A resumed or mixed file counts as its newest generation.
    pub(super) fn format(&self) -> &'static str {
        let s = &self.seen;
        if s.patched || (s.bare && !s.snapshot) {
            "gemini-g3"
        } else if s.header_messages && !s.snapshot {
            "gemini-g1"
        } else {
            "gemini-g2"
        }
    }

    fn warn(&mut self, line: Option<usize>, message: String) {
        self.warnings.push(Warning { line, message });
    }

    /// Classification order of the upstream loader: first match wins, unknown records are ignored.
    fn apply(&mut self, line: Option<usize>, record: Value) {
        let Value::Object(mut o) = record else {
            return self.warn(line, "record is not an object".into());
        };
        if let Some(Value::String(id)) = o.get("$rewindTo") {
            self.seen.patched = true;
            let id = id.clone();
            self.rewind_to(line, &id);
        } else if let Some(Value::Object(patch)) = o.get("$patch") {
            self.seen.patched = true;
            self.patch(line, patch);
        } else if let Some(Value::String(id)) = o.get("id") {
            self.seen.bare = true;
            let id = id.clone();
            self.upsert(line, id, o);
        } else if let Some(Value::Object(set)) = o.get_mut("$set") {
            self.set(line, std::mem::take(set));
        } else if o.get("sessionId").is_some_and(Value::is_string) {
            if let Some(Value::Array(messages)) = o.remove("messages") {
                self.seen.header_messages = true;
                self.replace_all(line, messages);
            }
            self.meta.extend(o);
        }
    }

    fn set(&mut self, line: Option<usize>, mut set: Obj) {
        if let Some(Value::Array(messages)) = set.remove("messages") {
            self.seen.snapshot = true;
            self.replace_all(line, messages);
        }
        self.meta.extend(set);
    }

    /// Store a message at its last version; a known id keeps its first position and line.
    fn store(&mut self, line: Option<usize>, id: String, value: Obj) {
        self.memory.harvest(&value);
        match self.msgs.get_mut(&id) {
            Some(rec) => rec.value = value,
            None => {
                self.slots.push(Slot::Msg(id.clone()));
                self.msgs.insert(id, Rec { value, line });
            }
        }
    }

    fn upsert(&mut self, line: Option<usize>, id: String, value: Obj) {
        if self.live_set.insert(id.clone()) {
            self.live.push(id.clone());
        }
        self.store(line, id, value);
    }

    /// `$set.messages` / header messages: the live list becomes exactly these (a repeated id
    /// keeps its first position and last value, an entry without an id is skipped like
    /// upstream does); live ids that are missing leave history.
    fn replace_all(&mut self, line: Option<usize>, messages: Vec<Value>) {
        let mut order: Vec<String> = Vec::new();
        let mut latest: HashMap<String, Obj> = HashMap::new();
        for (i, message) in messages.into_iter().enumerate() {
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let (Value::Object(value), Some(id)) = (message, id) else {
                self.warn(line, format!("snapshot entry {i} is not a message record"));
                continue;
            };
            if latest.insert(id.clone(), value).is_none() {
                order.push(id);
            }
        }
        let keep: HashSet<&String> = order.iter().collect();
        let dropped: Vec<String> = self
            .live
            .iter()
            .filter(|id| !keep.contains(id))
            .cloned()
            .collect();
        self.drop_ids(line, dropped);
        self.live_set = order.iter().cloned().collect();
        self.live = order.clone();
        for id in order {
            if let Some(value) = latest.remove(&id) {
                self.store(line, id, value);
            }
        }
    }

    /// Take ids out of the live list. Only user and model turns are history that was cut:
    /// the CLI also removes its own `info`/`warning`/`error` notices on the next sync, and
    /// those get no `Rewind`.
    fn drop_ids(&mut self, line: Option<usize>, dropped: Vec<String>) {
        if dropped.is_empty() {
            return;
        }
        let gone: HashSet<&String> = dropped.iter().collect();
        self.live.retain(|id| !gone.contains(id));
        dropped.iter().for_each(|id| {
            self.live_set.remove(id);
        });
        let is_turn = |id: &String| {
            let kind = self
                .msgs
                .get(id)
                .and_then(|r| r.value.get("type"))
                .and_then(Value::as_str);
            !matches!(kind, Some("info" | "warning" | "error"))
        };
        let cut: Vec<String> = dropped.iter().filter(|id| is_turn(id)).cloned().collect();
        if !cut.is_empty() {
            self.slots.push(Slot::Rewind { dropped: cut, line });
        }
    }

    /// Drop that message and everything after it; an unknown id clears the whole history.
    fn rewind_to(&mut self, line: Option<usize>, id: &str) {
        let at = self.live.iter().position(|l| l == id).unwrap_or(0);
        let dropped: Vec<String> = self.live[at..].to_vec();
        self.drop_ids(line, dropped);
    }

    /// The patch itself, then its `updates`, then `removeIds` and `orderIds` (upstream order).
    fn patch(&mut self, line: Option<usize>, patch: &Obj) {
        self.patch_one(patch);
        for update in array(patch.get("updates")) {
            if let Value::Object(update) = update {
                self.patch_one(update);
            }
        }
        let ids = |key: &str| -> Vec<String> {
            array(patch.get(key))
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        let remove: HashSet<String> = ids("removeIds").into_iter().collect();
        let dropped: Vec<String> = self
            .live
            .iter()
            .filter(|id| remove.contains(*id))
            .cloned()
            .collect();
        self.drop_ids(line, dropped);
        let order = ids("orderIds");
        if !order.is_empty() {
            let listed: HashSet<&String> = order.iter().collect();
            let mut next: Vec<String> = self
                .live
                .iter()
                .filter(|id| !listed.contains(id))
                .cloned()
                .collect();
            let mut placed: HashSet<&String> = HashSet::new();
            next.extend(
                order
                    .iter()
                    .filter(|id| self.live_set.contains(*id) && placed.insert(id))
                    .cloned(),
            );
            self.live = next;
        }
    }

    /// `{id, content?, toolCalls?: [{id, result?}]}`: replace the content, replace tool results.
    /// An unknown id is ignored; a known id that already left the live list is still updated.
    fn patch_one(&mut self, update: &Obj) {
        let Some(rec) = update
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| self.msgs.get_mut(id))
        else {
            return;
        };
        if let Some(content) = update.get("content") {
            rec.value.insert("content".into(), content.clone());
        }
        let calls = rec.value.get_mut("toolCalls").and_then(Value::as_array_mut);
        if let Some(calls) = calls {
            for patch in array(update.get("toolCalls")) {
                let id = patch.get("id").and_then(Value::as_str);
                let call = calls
                    .iter_mut()
                    .find(|c| c.get("id").and_then(Value::as_str) == id);
                if let (Some(Value::Object(call)), Some(result)) = (call, patch.get("result")) {
                    call.insert("result".into(), result.clone());
                }
            }
        }
        self.memory.harvest(&rec.value);
    }
}

impl Memory {
    /// Remember the first unmasked result of each call, from the call record or its echo turn,
    /// and the binary parts an echo turn carries.
    fn harvest(&mut self, message: &Obj) {
        match message.get("type").and_then(Value::as_str) {
            Some("gemini") => self.harvest_calls(message),
            Some("user") => self.harvest_echo(message),
            _ => {}
        }
    }

    fn harvest_calls(&mut self, message: &Obj) {
        for call in array(message.get("toolCalls")) {
            let (Some(id), Some(result)) =
                (call.get("id").and_then(Value::as_str), call.get("result"))
            else {
                continue;
            };
            if !self.originals.contains_key(id) && !result.is_null() && !is_masked(result, id) {
                self.originals.insert(id.to_string(), result.clone());
            }
        }
    }

    /// Sibling binary parts of an echo turn belong to its first `functionResponse`.
    fn harvest_echo(&mut self, message: &Obj) {
        let content = norm(message.get("content"));
        let mut first = None;
        for part in content.iter().filter(|p| p.function_response().is_some()) {
            let id = part
                .function_response()
                .and_then(|fr| fr.get("id"))
                .and_then(Value::as_str);
            let Some(id) = id else { continue };
            first.get_or_insert(id);
            let echo = Value::Array(part.obj().cloned().into_iter().collect());
            if !self.originals.contains_key(id) && !is_masked(&echo, id) {
                self.originals.insert(id.to_string(), echo);
            }
        }
        let attached: Vec<Media> = content
            .iter()
            .filter(|p| p.function_response().is_none())
            .filter_map(|p| media(*p))
            .collect();
        if let (Some(id), false) = (first, attached.is_empty()) {
            self.echo_media.entry(id.to_string()).or_insert(attached);
        }
    }
}

/// A result whose answer for `call_id` was replaced by the masking stub. A masked sibling
/// (the sync writes the whole echo turn into every call of a parallel batch) does not count.
pub(super) fn is_masked(result: &Value, call_id: &str) -> bool {
    let parts = norm(Some(result));
    match response_for(&parts, call_id) {
        Some(fr) => fr
            .get("response")
            .and_then(|r| r.get("output"))
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains(MASKED)),
        None => parts
            .iter()
            .any(|p| matches!(p, P::Str(s) if s.contains(MASKED))),
    }
}
