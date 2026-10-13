//! Content parts of a record: strings, objects, media, thoughts.

use super::Obj;
use crate::util::{as_extra, base64_len, str_field, stripped};
use crate::{Media, Part};
use serde_json::Value;

pub(super) fn array(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// A content part: bare strings are legal in `PartListUnion`.
#[derive(Clone, Copy)]
pub(super) enum P<'a> {
    Str(&'a str),
    Obj(&'a Value),
}

impl<'a> P<'a> {
    pub(super) fn text(self) -> Option<&'a str> {
        match self {
            P::Str(s) => Some(s),
            P::Obj(v) => v.get("text").and_then(Value::as_str),
        }
    }

    pub(super) fn field(self, key: &str) -> Option<&'a Value> {
        match self {
            P::Obj(v) => v.get(key),
            P::Str(_) => None,
        }
    }

    pub(super) fn obj(self) -> Option<&'a Value> {
        match self {
            P::Obj(v) => Some(v),
            P::Str(_) => None,
        }
    }

    fn is_thought(self) -> bool {
        self.field("thought").and_then(Value::as_bool) == Some(true)
    }

    pub(super) fn function_response(self) -> Option<&'a Value> {
        self.field("functionResponse").filter(|v| v.is_object())
    }
}

/// `content` as parts: a string, one part, or an array mixing both.
pub(super) fn norm(content: Option<&Value>) -> Vec<P<'_>> {
    match content {
        Some(Value::String(s)) => vec![P::Str(s)],
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => P::Str(s),
                other => P::Obj(other),
            })
            .collect(),
        Some(v @ Value::Object(_)) => vec![P::Obj(v)],
        _ => Vec::new(),
    }
}

pub(super) fn any_thought(parts: &[P<'_>]) -> bool {
    parts.iter().any(|p| p.is_thought())
}

/// The `functionResponse` that answers a call: the one with its id, else the first.
pub(super) fn response_for<'a>(parts: &[P<'a>], call_id: &str) -> Option<&'a Value> {
    let mut responses = parts.iter().filter_map(|p| p.function_response());
    let first = responses.next()?;
    let by_id = |fr: &&Value| fr.get("id").and_then(Value::as_str) == Some(call_id);
    Some(
        std::iter::once(first)
            .chain(responses)
            .find(by_id)
            .unwrap_or(first),
    )
}

/// An inline or referenced file part. Inline data is kept only as its size.
pub(super) fn media(part: P<'_>) -> Option<Media> {
    if let Some(d) = part.field("inlineData") {
        let bytes = d.get("data").and_then(Value::as_str).map(base64_len);
        return Some(Media {
            mime: str_field(d, &["mimeType"]),
            name: str_field(d, &["displayName"]),
            url: None,
            bytes,
        });
    }
    let f = part.field("fileData")?;
    Some(Media {
        mime: str_field(f, &["mimeType"]),
        name: str_field(f, &["displayName"]),
        url: str_field(f, &["fileUri"]),
        bytes: None,
    })
}

fn text_part(text: &str) -> Option<Part> {
    (!text.is_empty()).then(|| Part::Text {
        text: text.to_string(),
    })
}

/// Part keys of `@google/genai` this crate does not model; the first present one names a `Part::Other`.
const OTHER_PART_KEYS: &[&str] = &[
    "functionCall",
    "functionResponse",
    "executableCode",
    "codeExecutionResult",
    "videoMetadata",
];

/// A part that is not a function call as a model part.
pub(super) fn convert(part: P<'_>) -> Option<Part> {
    if let Some(m) = media(part) {
        return Some(Part::Attachment(m));
    }
    if part.is_thought() {
        let signed = part
            .field("thoughtSignature")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        let text = part.text().filter(|t| !t.is_empty()).map(str::to_string);
        return Some(Part::Reasoning {
            text,
            encrypted: signed,
        });
    }
    if let Some(text) = part.text() {
        return text_part(text);
    }
    let P::Obj(value) = part else { return None };
    let kind = OTHER_PART_KEYS.iter().find(|k| value.get(**k).is_some());
    Some(Part::Other {
        kind: kind.copied().unwrap_or("unknown").to_string(),
        data: stripped(value.clone()),
    })
}

/// `thoughts[]` as reasoning, written like the CLI replays it: `**subject** description`.
/// Each thought's own timestamp has no home in the model and is dropped.
pub(super) fn thought_parts(thoughts: Option<&Value>) -> impl Iterator<Item = Part> + '_ {
    array(thoughts).iter().map(|t| {
        let subject = str_field(t, &["subject"]).unwrap_or_default();
        let description = str_field(t, &["description"]).unwrap_or_default();
        let text = match (subject.is_empty(), description.is_empty()) {
            (true, true) => None,
            (true, false) => Some(description),
            (false, true) => Some(format!("**{subject}**")),
            (false, false) => Some(format!("**{subject}** {description}")),
        };
        Part::Reasoning {
            text,
            encrypted: false,
        }
    })
}

pub(super) fn join_text(content: Option<&Value>) -> String {
    let texts: Vec<&str> = norm(content).into_iter().filter_map(P::text).collect();
    texts.join("\n")
}

/// Record keys no typed field took, as recorded.
pub(super) fn rest(record: &Obj, mapped: impl Fn(&str) -> bool) -> Obj {
    (record.iter())
        .filter(|(k, _)| !mapped(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// [`rest`] as an `extra` value, `None` when nothing is left.
pub(super) fn unmapped(record: &Obj, mapped: impl Fn(&str) -> bool) -> Option<Value> {
    as_extra(rest(record, mapped))
}
