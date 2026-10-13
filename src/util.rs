//! Small shared helpers: timestamps, paths, text, JSON fields, media, diffs.

use crate::{Media, Part, Tool};
use serde_json::{Map, Value};
use std::io::BufRead;

pub type Obj = Map<String, Value>;

/// `2026-06-10T14:32:00.000Z` (or a numeric offset) to epoch millis. `None` when unparseable.
///
/// Hand-written instead of a date crate: the four products write only RFC3339 with `Z` or a
/// numeric offset and an optional fraction, and that is all this accepts.
pub fn rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if b[10] != b'T' && b[10] != b't' && b[10] != b' ' {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || sec > 60
    {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        if end == 0 {
            return None;
        }
        let digits = &frac[..end.min(3)];
        millis = digits.parse::<i64>().ok()? * 10_i64.pow(3 - digits.len() as u32);
        rest = &frac[end..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let (sign, digits) = match rest.strip_prefix('+') {
                Some(d) => (1, d),
                None => (-1, rest.strip_prefix('-')?),
            };
            let digits = digits.replace(':', "");
            if digits.len() != 2 && digits.len() != 4 || !digits.bytes().all(|c| c.is_ascii_digit())
            {
                return None;
            }
            let oh: i64 = digits[..2].parse().ok()?;
            let om: i64 = digits
                .get(2..)
                .filter(|m| !m.is_empty())
                .map_or(Some(0), |m| m.parse().ok())?;
            sign * (oh * 3600 + om * 60)
        }
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset;
    Some(secs * 1000 + millis)
}

/// A timestamp field that is either an RFC3339 string or epoch millis.
pub fn ts_ms(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => rfc3339_ms(s),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `file:///a/b%20c` to `/a/b c`; anything else is returned unchanged.
pub fn file_url_path(s: &str) -> String {
    let Some(rest) = s.strip_prefix("file://") else {
        return s.to_string();
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// First non-empty line, trimmed and cut to `max` chars, for titles.
pub fn one_line(text: &str, max: usize) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_string(),
    })
}

// JSON fields. Typed inputs are read leniently: a number may be written as a string and a
// string field may hold a number. The `take_*` helpers remove a key as it is mapped, so what a
// record has left is its unmapped keys; a value of the wrong type stays in place.

/// A string, or a number as written.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A non-negative number (a float is cut to an integer), or one written as a string.
fn unsigned(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn signed(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok().filter(|f: &f64| f.is_finite()),
        _ => None,
    }
}

/// A bool, or `"true"` / `"false"`.
fn boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// String field lookup over several key spellings (`file_path`, `filePath`, `path`).
pub fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| text(v.get(k)?))
}

/// Unsigned field lookup over several key spellings.
pub fn u64_field(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| unsigned(v.get(k)?))
}

/// Bool field lookup over several key spellings.
pub fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| v.get(k).and_then(boolean) == Some(true))
}

/// A string (or number) field of a record, left in place.
pub fn obj_str(m: &Obj, key: &str) -> Option<String> {
    m.get(key).and_then(text)
}

/// Remove `key` and convert its value; a value `f` hands back is put back.
fn take_as<T>(m: &mut Obj, key: &str, f: impl FnOnce(Value) -> Result<T, Value>) -> Option<T> {
    f(m.remove(key)?).map_err(|v| m.insert(key.into(), v)).ok()
}

/// Remove `key` when `f` reads its value.
pub fn take_with<T>(m: &mut Obj, key: &str, f: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
    take_as(m, key, |v| f(&v).ok_or(v))
}

pub fn take_str(m: &mut Obj, key: &str) -> Option<String> {
    take_as(m, key, |v| match v {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        v => Err(v),
    })
}

pub fn take_u64(m: &mut Obj, key: &str) -> Option<u64> {
    take_with(m, key, unsigned)
}

pub fn take_i64(m: &mut Obj, key: &str) -> Option<i64> {
    take_with(m, key, signed)
}

pub fn take_f64(m: &mut Obj, key: &str) -> Option<f64> {
    take_with(m, key, float)
}

/// A bool, or the strings `"true"`/`"false"`; `false` when absent or anything else.
pub fn take_bool(m: &mut Obj, key: &str) -> bool {
    take_with(m, key, boolean).unwrap_or(false)
}

pub fn take_obj(m: &mut Obj, key: &str) -> Option<Obj> {
    take_as(m, key, |v| match v {
        Value::Object(o) => Ok(o),
        v => Err(v),
    })
}

pub fn take_array(m: &mut Obj, key: &str) -> Vec<Value> {
    let array = take_as(m, key, |v| match v {
        Value::Array(a) => Ok(a),
        v => Err(v),
    });
    array.unwrap_or_default()
}

/// Unmapped keys as an `extra` value: blobs sized, `None` when nothing is left.
pub fn as_extra(m: Obj) -> Option<Value> {
    (!m.is_empty()).then(|| stripped(Value::Object(m)))
}

pub fn stripped(mut v: Value) -> Value {
    strip_blobs(&mut v);
    v
}

/// `mcp__<server>__<tool>` as an MCP tool.
pub fn mcp_tool(name: &str) -> Option<Tool> {
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    Some(Tool::Mcp {
        server: server.into(),
        tool: tool.into(),
    })
}

/// Text parts joined with `sep`.
pub fn join_texts(parts: &[Part], sep: &str) -> String {
    let texts: Vec<&str> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join(sep)
}

/// A media URL: a `data:` URL keeps only its type and decoded size, any other URL is kept.
pub fn media_from_url(url: &str, mime: Option<String>, name: Option<String>) -> Media {
    let Some(rest) = url.strip_prefix("data:") else {
        return Media {
            mime,
            name,
            url: Some(url.to_string()),
            bytes: None,
        };
    };
    let (head, payload) = rest.split_once(',').unwrap_or(("", rest));
    let bytes = if head.ends_with(";base64") {
        base64_len(payload)
    } else {
        payload.len() as u64
    };
    let mime = mime.or_else(|| Some(head.split(';').next()?.to_string()).filter(|m| !m.is_empty()));
    Media {
        mime,
        name,
        url: None,
        bytes: Some(bytes),
    }
}

/// Added and deleted lines of a diff. A `---` line directly followed by a `+++` line before the
/// first `@@` is a file header; every other line starting with `+` or `-` counts, a hunk line
/// whose own text starts with `++` or `--` included.
pub fn diff_stat(diff: &str) -> (u64, u64) {
    let (mut adds, mut dels, mut hunks) = (0, 0, false);
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        hunks |= line.starts_with("@@");
        if !hunks && line.starts_with("---") && lines.peek().is_some_and(|l| l.starts_with("+++")) {
            lines.next();
        } else if line.starts_with('+') {
            adds += 1;
        } else if line.starts_with('-') {
            dels += 1;
        }
    }
    (adds, dels)
}

/// Decoded byte length of a base64 payload, without decoding it.
pub fn base64_len(data: &str) -> u64 {
    let data = data.trim_end();
    let pad = data.bytes().rev().take_while(|b| *b == b'=').count() as u64;
    (data.len() as u64 / 4 * 3).saturating_sub(pad)
}

/// JSON records of a line stream: `(1-based line, parsed value or parse error)`.
/// Blank lines are skipped, invalid UTF-8 is replaced, `\r\n` is accepted, and a line that is
/// not JSON comes back as `Err` so the caller can record a warning and go on.
pub fn json_lines<R: BufRead>(reader: R) -> JsonLines<R> {
    JsonLines {
        reader,
        buf: Vec::new(),
        line: 0,
    }
}

pub struct JsonLines<R> {
    reader: R,
    buf: Vec<u8>,
    line: usize,
}

impl<R: BufRead> Iterator for JsonLines<R> {
    type Item = std::io::Result<(usize, Result<Value, String>)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.buf.clear();
            match self.reader.read_until(b'\n', &mut self.buf) {
                Ok(0) => return None,
                Ok(_) => self.line += 1,
                Err(e) => return Some(Err(e)),
            }
            let raw = self.buf.trim_ascii();
            if raw.is_empty() {
                continue;
            }
            let parsed = serde_json::from_slice(raw).or_else(|e| match std::str::from_utf8(raw) {
                Ok(_) => Err(e.to_string()),
                Err(_) => {
                    serde_json::from_str(&String::from_utf8_lossy(raw)).map_err(|e| e.to_string())
                }
            });
            return Some(Ok((self.line, parsed)));
        }
    }
}

/// Keys whose string values are opaque blobs: kept only as their size.
const BLOB_KEYS: &[&str] = &[
    "base64",
    "encrypted_content",
    "encrypted_function_args",
    "reasoningEncryptedContent",
    "signature",
    "thoughtSignature",
];

/// Replace inline base64 payloads, `data:` URLs and encrypted blobs with `{"bytes": n}`,
/// recursively. Text bodies are left alone.
pub fn strip_blobs(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Some(idx) = s
                .strip_prefix("data:")
                .and_then(|r| r.find(";base64,").map(|i| i + 13))
            {
                *v = blob(base64_len(&s[idx..]));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_blobs),
        Value::Object(map) => {
            // Anthropic image/document sources `{type: base64, data}` and Gemini `inlineData {mimeType, data}`.
            let inline = map.get("type").and_then(Value::as_str) == Some("base64")
                || map.contains_key("mimeType")
                || map.contains_key("media_type");
            for (key, val) in map.iter_mut() {
                let is_blob = BLOB_KEYS.contains(&key.as_str()) || inline && key == "data";
                match val {
                    Value::String(s) if is_blob && looks_base64(s) => *val = blob(base64_len(s)),
                    _ => strip_blobs(val),
                }
            }
        }
        _ => {}
    }
}

/// Base64 or base64url text of some length: no spaces, nothing a prose or code field would have.
fn looks_base64(s: &str) -> bool {
    s.len() >= 8
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=-_".contains(&b))
}

fn blob(bytes: u64) -> Value {
    serde_json::json!({ "bytes": bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_shapes() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_ms("2026-06-10T14:32:00.5Z"),
            rfc3339_ms("2026-06-10T14:32:00.500Z")
        );
        assert_eq!(
            rfc3339_ms("2026-06-10T14:32:00.123456789Z"),
            rfc3339_ms("2026-06-10T14:32:00.123Z")
        );
        assert_eq!(
            rfc3339_ms("2026-06-10T16:32:00+02:00"),
            rfc3339_ms("2026-06-10T14:32:00Z")
        );
        assert_eq!(
            rfc3339_ms("2026-06-10T12:32:00-0200"),
            rfc3339_ms("2026-06-10T14:32:00Z")
        );
        assert_eq!(
            rfc3339_ms("2026-10-11T14:50:06.579Z"),
            Some(1_791_730_206_579)
        );
        for bad in [
            "",
            "nightly",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-06-10T14:32:00",
        ] {
            assert_eq!(rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn file_urls_decode() {
        assert_eq!(file_url_path("file:///work/a%20b"), "/work/a b");
        assert_eq!(file_url_path("/plain/path"), "/plain/path");
        assert_eq!(file_url_path("file:///trailing%2"), "/trailing%2");
    }

    #[test]
    fn one_line_cuts_on_chars() {
        assert_eq!(
            one_line("\n  hello world \nnext", 5).as_deref(),
            Some("hello...")
        );
        assert_eq!(one_line("über café", 20).as_deref(), Some("über café"));
        assert_eq!(one_line("  \n ", 5), None);
    }

    #[test]
    fn json_lines_tolerates_bad_input() {
        let input = b"{\"a\":1}\r\n\n  \nnot json\n{\"b\":\"\xff\"}\n{\"torn\":";
        let got: Vec<_> = json_lines(&input[..]).map(Result::unwrap).collect();
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], (1, Ok(serde_json::json!({"a": 1}))));
        assert!(got[1].1.is_err() && got[1].0 == 4);
        assert_eq!(got[2].1.as_ref().unwrap()["b"], "\u{fffd}");
        assert!(got[3].1.is_err() && got[3].0 == 6);
    }

    #[test]
    fn strip_blobs_keeps_text_and_sizes_blobs() {
        let mut v = serde_json::json!({
            "text": "keep me",
            "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="},
            "url": "data:image/png;base64,aGVsbG8h",
            "inlineData": {"mimeType": "image/png", "data": "aGVsbG8="},
            "nested": [{"encrypted_content": "gAAAAAB0"}],
            "signature": "",
            "lsp": {"signature": "fn parse(s: &str) -> Value"}
        });
        strip_blobs(&mut v);
        assert_eq!(v["text"], "keep me");
        assert_eq!(v["source"]["data"], serde_json::json!({"bytes": 5}));
        assert_eq!(v["url"], serde_json::json!({"bytes": 6}));
        assert_eq!(v["inlineData"]["data"], serde_json::json!({"bytes": 5}));
        assert_eq!(
            v["nested"][0]["encrypted_content"],
            serde_json::json!({"bytes": 6})
        );
        assert_eq!(v["signature"], "");
        assert_eq!(v["lsp"]["signature"], "fn parse(s: &str) -> Value");
    }

    #[test]
    fn take_fields_leniently() {
        let v = serde_json::json!({"n": "120000", "f": 1.5, "b": "true", "s": 7, "o": [1]});
        let mut m = v.as_object().unwrap().clone();
        assert_eq!(
            (take_u64(&mut m, "n"), take_u64(&mut m, "f")),
            (Some(120_000), Some(1))
        );
        assert!(take_bool(&mut m, "b"));
        assert_eq!(take_str(&mut m, "s").as_deref(), Some("7"));
        assert_eq!(take_obj(&mut m, "o"), None);
        assert_eq!(m.len(), 1, "a value of the wrong type stays");
    }

    #[test]
    fn diff_stat_skips_only_file_headers() {
        assert_eq!(
            diff_stat("--- a\n+++ b\n@@ -1 +1 @@\n-old\n+++new\n---gone"),
            (1, 2)
        );
        assert_eq!(diff_stat("Index: a\n===\n--- a\n+++ a\n"), (0, 0));
        assert_eq!(diff_stat("+one\n+++two"), (2, 0));
    }

    #[test]
    fn data_url_media() {
        let m = media_from_url("data:image/png;base64,aGVsbG8=", None, None);
        assert_eq!(
            (m.mime.as_deref(), m.bytes, m.url),
            (Some("image/png"), Some(5), None)
        );
        let m = media_from_url("https://x/y.png", None, None);
        assert_eq!((m.url.as_deref(), m.bytes), (Some("https://x/y.png"), None));
    }

    #[test]
    fn base64_length() {
        assert_eq!(base64_len("aGVsbG8="), 5);
        assert_eq!(base64_len("aGVsbG8h"), 6);
    }
}
