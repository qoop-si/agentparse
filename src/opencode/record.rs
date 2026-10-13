//! Helpers over OpenCode records, on top of the shared `take_*` (re-exported here): what a
//! record has left after its keys are taken is its unmapped keys, which land in `Entry.extra`.

use super::Map;
use crate::model::*;
use crate::util::{as_extra, media_from_url, stripped, u64_field};
pub(super) use crate::util::{take_bool, take_f64, take_i64, take_str, take_u64, take_with};
use serde_json::Value;

/// Keys that hold an opaque provider blob (encrypted reasoning, signatures).
const OPAQUE_KEYS: &[&str] = &[
    "reasoningEncryptedContent",
    "signature",
    "encrypted_content",
];

pub(super) fn obj(v: Option<Value>) -> Map {
    match v {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// `m[outer][key]`, removing the inner key and the outer object when it becomes empty.
pub(super) fn take_nested<T>(
    m: &mut Map,
    outer: &str,
    key: &str,
    f: impl Fn(&Value) -> Option<T>,
) -> Option<T> {
    let inner = m.get_mut(outer)?.as_object_mut()?;
    let v = take_with(inner, key, f);
    if inner.is_empty() {
        m.remove(outer);
    }
    v
}

pub(super) fn take_nested_str(m: &mut Map, outer: &str, key: &str) -> Option<String> {
    take_nested(m, outer, key, |v| v.as_str().map(str::to_string))
}

/// `time.created`; other `time` keys (`completed`, `streamed`) stay as unmapped.
pub(super) fn take_time(m: &mut Map) -> Option<i64> {
    take_nested(m, "time", "created", Value::as_i64)
}

/// Start and end of a tool call: the first present key of each list is taken from `time`.
pub(super) fn take_span(m: &mut Map, starts: &[&str], ends: &[&str]) -> (Option<i64>, Option<i64>) {
    let mut pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| take_nested(m, "time", k, Value::as_i64))
    };
    (pick(starts), pick(ends))
}

/// `time.<key>` of a record that has not had its `time` taken apart yet.
pub(super) fn time_of(m: &Map, key: &str) -> Option<i64> {
    m.get("time")?.get(key)?.as_i64()
}

/// Milliseconds from `start` to `end`; `None` when either is missing or the span is not a
/// non-negative number that fits (timestamps are source data and can be anything).
pub(super) fn span_ms(start: Option<i64>, end: Option<i64>) -> Option<u64> {
    u64::try_from(end?.checked_sub(start?)?).ok()
}

/// The unmapped keys of a record, blobs sized instead of kept; `None` when nothing is left.
pub(super) fn extra_of(mut m: Map) -> Option<Value> {
    for key in ["id", "sessionID", "messageID"] {
        m.remove(key);
    }
    as_extra(m)
}

pub(super) fn entry(
    id: Option<String>,
    ts: Option<i64>,
    turn_id: Option<String>,
    extra: Map,
    kind: EntryKind,
) -> Entry {
    Entry {
        id,
        parent_id: None,
        turn_id,
        ts,
        line: None,
        extra: extra_of(extra),
        kind,
    }
}

pub(super) fn other_part(kind: impl Into<String>, data: Map) -> Part {
    Part::Other {
        kind: kind.into(),
        data: stripped(Value::Object(data)),
    }
}

/// Provider bookkeeping with no information of its own: item ids, the `phase` that
/// `Message.phase` takes, and the opaque blobs that `Part::Reasoning.encrypted` stands for.
const BOOKKEEPING: &[&str] = &["itemId", "phase"];

fn prune_bookkeeping(m: &mut Map) {
    m.retain(|k, _| !BOOKKEEPING.contains(&k.as_str()) && !OPAQUE_KEYS.contains(&k.as_str()));
    for v in m.values_mut() {
        if let Value::Object(inner) = v {
            prune_bookkeeping(inner);
        }
    }
    m.retain(|_, v| !matches!(v, Value::Object(inner) if inner.is_empty()));
}

/// What a text or reasoning part has left after its text, flags and provider bookkeeping
/// (timings, other provider state). It goes to `Entry.extra.parts`, keyed by part position.
pub(super) fn part_leftover(mut part: Map) -> Option<Value> {
    part.remove("synthetic");
    part.remove("ignored");
    prune_bookkeeping(&mut part);
    as_extra(part)
}

/// Record the leftover of the part at `position` for `Entry.extra.parts`.
pub(super) fn note_leftover(extras: &mut Map, position: usize, leftover: Option<Value>) {
    if let Some(v) = leftover {
        extras.insert(position.to_string(), v);
    }
}

/// Put the collected part leftovers into a record's unmapped keys.
pub(super) fn attach_part_extras(data: &mut Map, extras: Map) {
    if !extras.is_empty() {
        data.insert("parts".into(), Value::Object(extras));
    }
}

/// An opaque provider blob (encrypted reasoning, signature) exists somewhere in `v`.
pub(super) fn has_opaque(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, v)| {
            (OPAQUE_KEYS.contains(&k.as_str()) && v.as_str().is_some_and(|s| !s.is_empty()))
                || has_opaque(v)
        }),
        Value::Array(items) => items.iter().any(has_opaque),
        _ => false,
    }
}

/// OpenAI-style `phase` (`commentary`, `final_answer`) kept inside a provider object.
pub(super) fn phase_of(v: Option<&Value>) -> Option<String> {
    v?.as_object()?
        .values()
        .find_map(|provider| provider.get("phase")?.as_str().map(str::to_string))
}

pub(super) fn usage(tokens: Option<Value>, cost: Option<f64>) -> Option<Usage> {
    if tokens.is_none() && cost.is_none() {
        return None;
    }
    let t = tokens.unwrap_or(Value::Null);
    let n = |keys: &[&str]| u64_field(&t, keys).unwrap_or(0);
    let cache = |key: &str| {
        t.get("cache")
            .and_then(|c| u64_field(c, &[key]))
            .unwrap_or(0)
    };
    let reasoning = n(&["reasoning"]);
    Some(Usage {
        input: n(&["input"]),
        output: n(&["output"]).saturating_add(reasoning),
        reasoning,
        cache_read: cache("read"),
        cache_write: cache("write"),
        cost_usd: cost,
    })
}

pub(super) fn media_from_file(mut f: Map) -> Media {
    let (mime, name) = (
        take_str(&mut f, "mime"),
        take_str(&mut f, "filename").or_else(|| take_str(&mut f, "name")),
    );
    match take_str(&mut f, "url").or_else(|| take_str(&mut f, "uri")) {
        Some(url) => media_from_url(&url, mime, name),
        None => Media {
            mime,
            name,
            url: None,
            bytes: None,
        },
    }
}

pub(super) struct ApiError {
    pub(super) kind: Option<String>,
    pub(super) message: String,
    pub(super) status: Option<u16>,
    /// Keys beyond kind, message and status (response bodies, retry hints).
    pub(super) rest: Map,
}

/// v1 `{name, data: {message, statusCode, ...}}`.
pub(super) fn v1_error(v: Value) -> ApiError {
    let mut e = obj(Some(v));
    let kind = take_str(&mut e, "name");
    let mut data = obj(e.remove("data"));
    let message = take_str(&mut data, "message")
        .or_else(|| kind.clone())
        .unwrap_or_default();
    let status = take_u64(&mut data, "statusCode").and_then(|s| u16::try_from(s).ok());
    ApiError {
        kind,
        message,
        status,
        rest: data,
    }
}

/// v2 `{type, message, status?, response?}`.
pub(super) fn v2_error(v: Value) -> ApiError {
    let mut e = obj(Some(v));
    let kind = take_str(&mut e, "type");
    let message = take_str(&mut e, "message").unwrap_or_default();
    let status = take_u64(&mut e, "status").and_then(|s| u16::try_from(s).ok());
    ApiError {
        kind,
        message,
        status,
        rest: e,
    }
}

fn is_abort(kind: Option<&str>) -> bool {
    matches!(kind, Some("MessageAbortedError" | "aborted"))
}

/// A failed step: an interrupt marker for a user abort, an error entry for everything else.
pub(super) fn failure_entry(
    e: ApiError,
    retry: Option<u32>,
    ts: Option<i64>,
    turn_id: Option<String>,
) -> Entry {
    if is_abort(e.kind.as_deref()) {
        let turn = Turn {
            status: Some("interrupted".into()),
            duration_ms: None,
            error: None,
        };
        // The interrupt text is the marker, not an error.
        let mut extra = e.rest;
        extra.insert("marker".into(), Value::String(e.message));
        return entry(None, ts, turn_id, extra, EntryKind::Turn(turn));
    }
    let info = ErrorInfo {
        kind: e.kind,
        message: e.message,
        status: e.status,
        retry,
    };
    entry(None, ts, turn_id, e.rest, EntryKind::Error(info))
}

pub(super) fn is_part(p: &Map, ty: &str) -> bool {
    p.get("type").and_then(Value::as_str) == Some(ty)
}

pub(super) fn take_objects(m: &mut Map, key: &str) -> Vec<Map> {
    match m.remove(key) {
        Some(Value::Array(items)) => items.into_iter().map(|i| obj(Some(i))).collect(),
        _ => Vec::new(),
    }
}
