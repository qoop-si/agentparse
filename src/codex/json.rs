//! JSON record helpers.

use super::{PASSTHROUGH, Ref};
use crate::util::{Obj, as_extra, take_str};
use crate::{Entry, EntryKind, Part};
use serde_json::Value;

pub(super) fn take(p: &mut Obj, key: &str) -> Value {
    p.remove(key).unwrap_or(Value::Null)
}

/// A policy as a word: `"never"`, `{"type": "read-only", ...}` or `{"granular": {...}}`.
pub(super) fn label(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => match o.get("type") {
            Some(Value::String(t)) => Some(t.clone()),
            _ => (o.len() == 1).then(|| o.keys().next().cloned()).flatten(),
        },
        _ => None,
    }
}

/// Take a policy that is a plain word; a structured one stays in `extra` as recorded.
pub(super) fn take_label(p: &mut Obj, key: &str) -> Option<String> {
    match p.get(key) {
        Some(Value::String(_)) => take_str(p, key),
        other => label(other),
    }
}

/// The turn id and per-part kinds from a response item's passthrough metadata.
pub(super) fn take_passthrough(p: &mut Obj) -> (Option<String>, Vec<Option<String>>) {
    let Some(Value::Object(pt)) = p.get_mut(PASSTHROUGH) else {
        return (None, Vec::new());
    };
    let turn = take_str(pt, "turn_id");
    let kinds = match pt.remove("content_item_kinds") {
        Some(Value::Array(kinds)) => kinds
            .into_iter()
            .map(|k| k.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    if pt.is_empty() {
        p.remove(PASSTHROUGH);
    }
    (turn, kinds)
}

pub(super) fn with_meta(mut p: Obj, meta: Option<Value>) -> Obj {
    p.extend(meta.map(|m| ("metadata".to_owned(), m)));
    p
}

pub(super) fn with_type(mut item: Obj, ty: &str) -> Obj {
    item.insert("type".into(), Value::String(ty.into()));
    item
}

/// Leftovers of one item of a merged response, under `items.<item id>` so that several items
/// (reasoning, text) keep their own times and metadata.
pub(super) fn item_extra(id: Option<&str>, rest: Obj) -> Obj {
    match id {
        Some(id) if !rest.is_empty() => {
            let item = Obj::from_iter([(id.to_owned(), Value::Object(rest))]);
            Obj::from_iter([("items".to_owned(), Value::Object(item))])
        }
        _ => rest,
    }
}

pub(super) fn calls_extra(call_id: &str, key: &str, rest: Obj) -> Obj {
    if rest.is_empty() {
        return rest;
    }
    let record = Obj::from_iter([(key.to_owned(), Value::Object(rest))]);
    let call = Obj::from_iter([(call_id.to_owned(), Value::Object(record))]);
    Obj::from_iter([("calls".to_owned(), Value::Object(call))])
}

pub(super) fn part_mut(entries: &mut [Entry], r: Ref) -> Option<&mut Part> {
    match &mut entries.get_mut(r.entry)?.kind {
        EntryKind::Message(m) => m.parts.get_mut(r.part),
        _ => None,
    }
}

/// Note the id of a source item merged into an entry: `extra.item_ids` lists every merged item,
/// the one whose id became `Entry.id` included.
pub(super) fn add_item_id(e: &mut Entry, id: String) {
    let own =
        e.id.clone()
            .filter(|own| *own != id && !own.starts_with("ord:"));
    let Value::Object(extra) = e.extra.get_or_insert_with(|| Value::Object(Obj::new())) else {
        return;
    };
    let Value::Array(ids) = extra
        .entry("item_ids")
        .or_insert_with(|| Value::Array(Vec::new()))
    else {
        return;
    };
    for id in own.filter(|_| ids.is_empty()).into_iter().chain([id]) {
        if !ids.iter().any(|v| v.as_str() == Some(&id)) {
            ids.push(Value::String(id));
        }
    }
}

/// Merge keys into an entry's `extra`; on a clash the value already there wins.
pub(super) fn merge_extra(e: &mut Entry, add: Obj) {
    let Some(Value::Object(add)) = as_extra(add) else {
        return;
    };
    match &mut e.extra {
        Some(Value::Object(dst)) => deep_merge(dst, add),
        _ => e.extra = Some(Value::Object(add)),
    }
}

fn deep_merge(dst: &mut Obj, src: Obj) {
    for (k, v) in src {
        match (dst.get_mut(&k), v) {
            (Some(Value::Object(d)), Value::Object(s)) => deep_merge(d, s),
            (Some(_), _) => {}
            (None, v) => {
                dst.insert(k, v);
            }
        }
    }
}
