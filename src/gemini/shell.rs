//! The text of a `run_shell_command` result, by era:
//! - up to v0.26: `Command:`, `Directory:`, `Stdout:`, `Stderr:` and the tail lines (`(none)` placeholders);
//! - v0.28 on: `Output:` (one merged stream, so `stderr` stays `None`) and a tail with only the
//!   lines that apply (`shell.ts`);
//! - v0.49 on: the same inside `<untrusted_context>`;
//! - any era: the whole text can be cut to `Output too large. ...` (`formatTruncatedToolOutput`),
//!   a user abort and the inactivity timeout have their own sentences, and a background start
//!   is one sentence.

use super::Obj;
use crate::ShellResult;
use serde_json::Value;
use std::borrow::Cow;

const CANCELLED: &str = "Command was cancelled by user before it could";
const TIMED_OUT: &str = "Command was automatically cancelled because it exceeded the timeout";
const OUTPUT_BEFORE: &str = "Below is the output before it was cancelled:";
const MOVED_TO_BACKGROUND: &str = "Command moved to background (PID: ";
const TRUNCATED: &str = "Output too large. Showing first";
/// Tail lines in print order; matched from the end, each key once.
const TAIL_KEYS: [&str; 5] = [
    "Error: ",
    "Exit Code: ",
    "Signal: ",
    "Background PIDs: ",
    "Process Group PGID: ",
];
const ERROR: usize = 0;
const EXIT_CODE: usize = 1;
const SIGNAL: usize = 2;
const BACKGROUND_PIDS: usize = 3;
const PGID: usize = 4;

#[derive(Default)]
pub(super) struct ShellParse {
    pub result: ShellResult,
    /// The `Directory:` line of the v0.9 to v0.26 text.
    pub cwd: Option<String>,
    /// `process_group`, `background_pids` and `error` (spawn error), which have no typed home.
    pub data: Obj,
}

/// `ok` says the call was recorded as a success: a modern success without an `Exit Code` line
/// or a signal exited 0 (upstream prints the code only when it is non-zero).
pub(super) fn parse_shell(text: Option<&str>, ok: bool) -> ShellParse {
    let mut out = ShellParse::default();
    let Some(text) = text else { return out };
    // Since v0.49 the truncation banner sits outside the `<untrusted_context>` wrapper; older
    // text has no wrapper. Strip it on both sides of the unwrap.
    let unwrapped = unwrap_untrusted(strip_truncation_banner(text));
    let body = strip_truncation_banner(&unwrapped);
    if body.starts_with(CANCELLED) || body.starts_with(TIMED_OUT) {
        aborted(body, &mut out.result);
    } else if let Some(rest) = body.strip_prefix(MOVED_TO_BACKGROUND) {
        let pid: String = rest.chars().take_while(char::is_ascii_digit).collect();
        out.result.background_id = Some(pid).filter(|p| !p.is_empty());
    } else if body.starts_with("Command: ") || body.starts_with("Output: ") {
        structured(body, ok, &mut out);
    } else {
        out.result.stdout = Some(body.to_string());
    }
    out
}

/// Where truncation applies: the saved path, and whether the read-file banner cut the text.
pub(super) fn truncation(text: &str) -> (bool, Option<String>) {
    let shell = text.contains(TRUNCATED);
    let path = text
        .split_once("For full output see: ")
        .and_then(|(_, rest)| rest.lines().next())
        .map(|p| p.trim().to_string());
    let read = text.starts_with("IMPORTANT: The file content has been truncated");
    (shell || read, path.filter(|p| shell && !p.is_empty()))
}

/// The banner line is not part of the shell text: after it comes `Output: <head>...<tail>`.
fn strip_truncation_banner(body: &str) -> &str {
    match body.strip_prefix(TRUNCATED) {
        Some(rest) => rest.split_once('\n').map_or(body, |(_, rest)| rest),
        None => body,
    }
}

fn aborted(body: &str, r: &mut ShellResult) {
    r.interrupted = body.starts_with(CANCELLED);
    r.timed_out = !r.interrupted;
    r.stdout = body
        .split_once(OUTPUT_BEFORE)
        .map(|(_, rest)| rest.strip_prefix('\n').unwrap_or(rest).to_string());
}

fn structured(body: &str, ok: bool, out: &mut ShellParse) {
    let old = body.starts_with("Command: ");
    let (rest, tail) = split_tail(body, old);
    let value = |i: usize| tail[i].filter(|v| *v != "(none)");
    let r = &mut out.result;
    r.exit_code = value(EXIT_CODE).and_then(|v| v.parse().ok());
    r.signal = value(SIGNAL).map(str::to_string);
    let pids: Vec<u64> = value(BACKGROUND_PIDS)
        .iter()
        .flat_map(|v| v.split(','))
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    r.background_id = pids.first().map(u64::to_string);
    if !pids.is_empty() {
        out.data.insert("background_pids".into(), Value::from(pids));
    }
    if let Some(pgid) = value(PGID).and_then(|v| v.parse::<u64>().ok()) {
        out.data.insert("process_group".into(), Value::from(pgid));
    }
    if let Some(error) = value(ERROR) {
        out.data.insert("error".into(), Value::from(error));
    }

    let placeholder = |s: &str| {
        if s == "(empty)" {
            String::new()
        } else {
            s.to_string()
        }
    };
    if let Some(old) = rest.strip_prefix("Command: ") {
        // Stdout can hold anything, so the last `Stderr:` line is the split.
        let (head, stderr) = old.rsplit_once("\nStderr: ").unwrap_or((old, ""));
        let (before, stdout) = head.split_once("\nStdout: ").unwrap_or((head, ""));
        let directory = before.split_once("\nDirectory: ").map(|(_, d)| d.trim());
        out.cwd = directory
            .filter(|d| !matches!(*d, "(root)" | "(none)" | ""))
            .map(str::to_string);
        r.stdout = Some(placeholder(stdout));
        r.stderr = Some(placeholder(stderr));
    } else {
        r.stdout = Some(placeholder(rest.strip_prefix("Output: ").unwrap_or(rest)));
        // A spawn error means no process ever exited.
        if ok && tail[EXIT_CODE].is_none() && tail[SIGNAL].is_none() && tail[ERROR].is_none() {
            r.exit_code = Some(0);
        }
    }
}

/// Peel the tail lines (`Exit Code: 1`, ...) off the end, each key once, in print order. In the
/// modern text an `Error:` line only exists when the spawn failed, and then nothing else was
/// printed (`Output: (empty)`); any other `Error: ` line is the command's own output.
fn split_tail(body: &str, old_era: bool) -> (&str, [Option<&str>; 5]) {
    let mut tail = [None; 5];
    let mut rest = body.trim_end_matches('\n');
    let mut limit = TAIL_KEYS.len();
    loop {
        let (head, last) = rest.rsplit_once('\n').unwrap_or(("", rest));
        let found = TAIL_KEYS[..limit]
            .iter()
            .enumerate()
            .find_map(|(i, k)| last.strip_prefix(k).map(|v| (i, v)));
        let Some((i, value)) = found else { break };
        if i == ERROR && !old_era && head != "Output: (empty)" {
            break;
        }
        tail[i] = Some(value.trim());
        limit = i;
        rest = head;
    }
    (rest, tail)
}

/// `<untrusted_context>\n...\n</untrusted_context>` (v0.49+) to its body, with the escaped closing tag restored.
fn unwrap_untrusted(text: &str) -> Cow<'_, str> {
    let inner = text
        .trim()
        .strip_prefix("<untrusted_context>")
        .and_then(|t| t.strip_suffix("</untrusted_context>"));
    match inner {
        Some(inner) => {
            let inner = inner.strip_prefix('\n').unwrap_or(inner);
            let inner = inner.strip_suffix('\n').unwrap_or(inner);
            inner
                .replace("&lt;/untrusted_context&gt;", "</untrusted_context>")
                .into()
        }
        None => text.into(),
    }
}
