//! `agentparse`: list, show and measure agent sessions.

use agentparse::{
    Agent, EntryKind, Origin, Part, Role, Roots, Session, SessionRef, Source, Tool, ToolCall,
    ToolStatus,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// `println!` that exits quietly when stdout is closed (`agentparse list | head`).
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write;
        if writeln!(std::io::stdout().lock(), $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

const USAGE: &str = "usage:
  agentparse list  [--agent claude|codex|opencode|gemini] [--json]
  agentparse show  <file | session id> [--json] [--full]
  agentparse stats [--agent ...] [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    if flag("--help") || flag("-h") {
        say!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let agent = match args
        .iter()
        .position(|a| a == "--agent")
        .map(|i| args.get(i + 1))
    {
        Some(Some(a)) => match a.parse::<Agent>() {
            Ok(a) => Some(a),
            Err(e) => return fail(&e),
        },
        Some(None) => return fail(USAGE),
        None => None,
    };
    let positional: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--agent"))
        .map(|(_, a)| a)
        .collect();
    let result = match positional.first().map(|s| s.as_str()) {
        Some("list") => list(agent, flag("--json")),
        Some("show") => match positional.get(1) {
            Some(target) => show(target, flag("--json"), flag("--full")),
            None => Err(USAGE.to_string()),
        },
        Some("stats") => stats(agent, flag("--json")),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::FAILURE
}

fn refs(agent: Option<Agent>) -> Vec<SessionRef> {
    let (mut refs, warnings) = agentparse::discover(&Roots::from_env());
    for w in warnings {
        eprintln!("warning: {}", w.message);
    }
    refs.retain(|r| agent.is_none_or(|a| r.agent == a));
    refs
}

fn list(agent: Option<Agent>, json: bool) -> Result<(), String> {
    let mut refs = refs(agent);
    refs.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    if json {
        return print_json(&refs);
    }
    for r in refs {
        let when = r.updated_at.map(datetime).unwrap_or_default();
        let parent = r
            .parent_id
            .map(|p| format!("  (child of {p})"))
            .unwrap_or_default();
        say!(
            "{:<8} {when}  {}  {}{parent}",
            r.agent.as_str(),
            r.id,
            location(&r.source)
        );
    }
    Ok(())
}

fn show(target: &str, json: bool, full: bool) -> Result<(), String> {
    let session = if Path::new(target).is_file() {
        agentparse::parse_file(Path::new(target))
    } else {
        // Ids can repeat across files (resumed or migrated copies): take the newest.
        let found = refs(None)
            .into_iter()
            .filter(|r| r.id == target)
            .max_by_key(|r| r.updated_at);
        let found = found.ok_or_else(|| format!("no session with id {target}"))?;
        agentparse::parse(&found)
    }
    .map_err(|e| e.to_string())?;
    if json {
        return print_json(&session);
    }
    render(&session, full);
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    say!("{text}");
    Ok(())
}

fn location(source: &Source) -> String {
    match source {
        Source::File { path } => path.display().to_string(),
        Source::Sqlite { path, session_id } => format!("{}#{session_id}", path.display()),
    }
}

// ---------------------------------------------------------------- show

fn render(s: &Session, full: bool) {
    say!("{} session {}  [{}]", s.agent.as_str(), s.id, s.format);
    let mut facts = Vec::new();
    if let Some(t) = &s.title {
        facts.push(format!("title: {t}"));
    }
    if let Some(c) = &s.cwd {
        facts.push(format!("cwd: {c}"));
    }
    if let Some(b) = s.git.as_ref().and_then(|g| g.branch.as_ref()) {
        facts.push(format!("branch: {b}"));
    }
    if !s.models.is_empty() {
        facts.push(format!("models: {}", s.models.join(", ")));
    }
    if let Some(p) = &s.parent {
        facts.push(format!("parent: {}", p.session_id));
    }
    facts.push(format!("usage: {}", usage_line(&s.usage)));
    for f in facts {
        say!("  {f}");
    }
    for e in &s.entries {
        let when = e.ts.map(datetime).unwrap_or_default();
        match &e.kind {
            EntryKind::Message(m) => {
                let who = match (m.role, m.origin) {
                    (Role::User, Origin::Human) => "user".to_string(),
                    (Role::Assistant, _) => format!(
                        "assistant{}",
                        m.model
                            .as_ref()
                            .map(|x| format!(" {x}"))
                            .unwrap_or_default()
                    ),
                    (role, origin) => format!(
                        "{} ({})",
                        lower(role),
                        m.kind.clone().unwrap_or_else(|| lower(origin))
                    ),
                };
                let usage = m
                    .usage
                    .as_ref()
                    .map(|u| format!("  [{}]", usage_line(u)))
                    .unwrap_or_default();
                say!("\n-- {who}  {when}{usage}");
                for p in &m.parts {
                    part(p, full);
                }
            }
            EntryKind::Compaction(c) => {
                let tokens = match (c.tokens_before, c.tokens_after) {
                    (Some(b), Some(a)) => format!(" {b} -> {a} tokens"),
                    (Some(b), None) => format!(" from {b} tokens"),
                    _ => String::new(),
                };
                say!(
                    "\n== compaction{}{tokens}  {when}",
                    c.trigger
                        .as_ref()
                        .map(|t| format!(" ({t})"))
                        .unwrap_or_default()
                );
                if let Some(summary) = &c.summary {
                    block(summary, if full { usize::MAX } else { 6 });
                }
            }
            EntryKind::Turn(t) => {
                let dur = t
                    .duration_ms
                    .map(|d| format!(" {}", secs(d)))
                    .unwrap_or_default();
                say!("== turn {}{dur}", t.status.as_deref().unwrap_or("end"));
            }
            EntryKind::Settings(x) => {
                let fields = [
                    ("model", &x.model),
                    ("agent", &x.agent),
                    ("cwd", &x.cwd),
                    ("effort", &x.effort),
                    ("approval", &x.approval),
                    ("sandbox", &x.sandbox),
                ];
                let set: Vec<String> = fields
                    .iter()
                    .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}")))
                    .collect();
                say!("== settings {}", set.join(" "));
            }
            EntryKind::Notice(n) => say!(
                "== notice {}: {}",
                n.kind.as_deref().unwrap_or("-"),
                first_line(&n.text)
            ),
            EntryKind::Hook(h) => {
                let exit = h
                    .exit_code
                    .map(|c| format!(" exit {c}"))
                    .unwrap_or_default();
                say!(
                    "== hook {} {}{exit}",
                    h.event.as_deref().unwrap_or("-"),
                    h.command.as_deref().or(h.name.as_deref()).unwrap_or("")
                );
            }
            EntryKind::Error(x) => {
                let status = x.status.map(|s| format!(" {s}")).unwrap_or_default();
                say!(
                    "== error{status} {}: {}",
                    x.kind.as_deref().unwrap_or("-"),
                    first_line(&x.message)
                );
            }
            EntryKind::Rewind(r) => match r.turns {
                Some(turns) => say!("== rewind: {turns} turns rolled back"),
                None => say!("== rewind: {} entries left the context", r.dropped.len()),
            },
            EntryKind::Other(o) => say!("   ({})", o.kind),
        }
    }
}

fn part(p: &Part, full: bool) {
    let max = if full { usize::MAX } else { 12 };
    match p {
        Part::Text { text } => block(text, if full { usize::MAX } else { 40 }),
        Part::Reasoning { text: Some(t), .. } if !t.trim().is_empty() => {
            say!("  [reasoning]");
            block(t, if full { usize::MAX } else { 4 });
        }
        Part::Reasoning { .. } => say!("  [reasoning, not readable]"),
        Part::Attachment(m) => say!(
            "  [attachment {} {}]",
            m.mime.as_deref().unwrap_or("-"),
            m.name.as_deref().or(m.url.as_deref()).unwrap_or("")
        ),
        Part::ToolCall(c) => call(c, 1, max),
        Part::ToolResult(r) => {
            say!("  [result for {} ({})]", r.call_id, lower(r.status));
            block(&r.output.text, max);
        }
        Part::Other { kind, .. } => say!("  ({kind})"),
    }
}

fn call(c: &ToolCall, depth: usize, max: usize) {
    let pad = "  ".repeat(depth);
    let status = match c.status {
        ToolStatus::Ok => String::new(),
        s => format!(
            "  [{}{}]",
            lower(s),
            c.status_detail
                .as_ref()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ),
    };
    say!("{pad}> {}{status}", describe(c));
    if let Some(out) = &c.output {
        if let Some(sh) = &out.shell {
            let mut facts = Vec::new();
            if let Some(code) = sh.exit_code {
                facts.push(format!("exit {code}"));
            }
            if let Some(d) = sh.duration_ms {
                facts.push(secs(d));
            }
            if sh.timed_out {
                facts.push("timed out".into());
            }
            if sh.interrupted {
                facts.push("interrupted".into());
            }
            if let Some(b) = &sh.background_id {
                facts.push(format!("background {b}"));
            }
            if !facts.is_empty() {
                say!("{pad}  {}", facts.join(" | "));
            }
            let label = if sh.stderr.is_some() {
                "stdout"
            } else {
                "output"
            };
            for (name, stream) in [(label, &sh.stdout), ("stderr", &sh.stderr)] {
                if let Some(text) = stream.as_ref().filter(|t| !t.trim().is_empty()) {
                    say!("{pad}  {name}:");
                    block_pad(text, max, &format!("{pad}    "));
                }
            }
        } else if !out.text.trim().is_empty() {
            // Non-shell results are often whole files: a short preview unless --full.
            let preview = if max == usize::MAX { max } else { 6 };
            block_pad(&out.text, preview, &format!("{pad}    "));
        }
        for f in &out.files {
            let stat = match (f.additions, f.deletions) {
                (Some(a), Some(d)) => format!(" +{a} -{d}"),
                _ => String::new(),
            };
            say!("{pad}  {} {}{stat}", lower(f.op), f.path);
        }
        if out.truncated {
            say!(
                "{pad}  (output truncated{})",
                out.full_output_path
                    .as_ref()
                    .map(|p| format!(", full: {p}"))
                    .unwrap_or_default()
            );
        }
    }
    if let Some(child) = &c.child_session {
        say!("{pad}  -> session {child}");
    }
    for child in &c.children {
        call(child, depth + 1, max);
    }
}

fn describe(c: &ToolCall) -> String {
    match &c.tool {
        Tool::Shell(sh) => {
            let mut s = format!("$ {}", first_line(&sh.command));
            if let Some(cwd) = &sh.cwd {
                s.push_str(&format!("   (in {cwd})"));
            }
            if sh.background {
                s.push_str("  &");
            }
            s
        }
        Tool::Process { id, action, input } => format!(
            "{} process {}{}",
            lower(*action),
            id.as_deref().unwrap_or("?"),
            input
                .as_ref()
                .map(|i| format!(" <- {i:?}"))
                .unwrap_or_default()
        ),
        Tool::Read { path, .. } => format!("read {path}"),
        Tool::Write { path, .. } => format!("write {path}"),
        Tool::Edit { path, edits } => format!(
            "edit {path} ({} change{})",
            edits.len(),
            if edits.len() == 1 { "" } else { "s" }
        ),
        Tool::Patch { .. } => "apply patch".into(),
        Tool::Search { pattern, path, .. } => format!(
            "search {pattern:?}{}",
            path.as_ref()
                .map(|p| format!(" in {p}"))
                .unwrap_or_default()
        ),
        Tool::Glob { pattern, .. } => format!("glob {pattern}"),
        Tool::List { path } => format!("list {}", path.as_deref().unwrap_or(".")),
        Tool::WebFetch { url, .. } => format!("fetch {}", url.as_deref().unwrap_or("?")),
        Tool::WebSearch { query } => format!("web search {:?}", query.as_deref().unwrap_or("")),
        Tool::Todo { items } => format!("todo list ({} items)", items.len()),
        Tool::Task {
            description,
            agent_type,
            ..
        } => format!(
            "task {}{}",
            agent_type.as_deref().unwrap_or("agent"),
            description
                .as_ref()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ),
        Tool::Mcp { server, tool } => format!("mcp {server}/{tool}"),
        Tool::Code { source } => format!("code cell ({} lines)", source.lines().count()),
        Tool::Skill { name } => format!("skill {}", name.as_deref().unwrap_or("?")),
        Tool::Question | Tool::Plan | Tool::Other => c.name.clone(),
    }
}

fn block(text: &str, max: usize) {
    block_pad(text, max, "  ");
}

fn block_pad(text: &str, max: usize, pad: &str) {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    for line in lines.iter().take(max) {
        say!("{pad}{line}");
    }
    if lines.len() > max {
        say!("{pad}... {} more lines", lines.len() - max);
    }
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

fn usage_line(u: &agentparse::Usage) -> String {
    let mut s = format!("in {} out {}", u.input, u.output);
    if u.cache_read + u.cache_write > 0 {
        s.push_str(&format!(" cache r{} w{}", u.cache_read, u.cache_write));
    }
    if u.reasoning > 0 {
        s.push_str(&format!(" reasoning {}", u.reasoning));
    }
    if let Some(c) = u.cost_usd {
        s.push_str(&format!(" ${c:.4}"));
    }
    s
}

/// The serde name of a unit-like enum value (`ToolStatus::Ok` -> "ok").
fn lower(v: impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn secs(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Epoch ms as `YYYY-MM-DD HH:MM:SS` UTC.
fn datetime(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000) / 1000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

// ---------------------------------------------------------------- stats

#[derive(Default, serde::Serialize)]
struct Stats {
    sessions: usize,
    failures: BTreeMap<String, usize>,
    bytes: u64,
    entries: usize,
    warnings: usize,
    messages_by_origin: BTreeMap<String, usize>,
    entries_by_type: BTreeMap<String, usize>,
    tool_calls_by_kind: BTreeMap<String, usize>,
    tool_calls_by_status: BTreeMap<String, usize>,
    shell_exit_codes: BTreeMap<String, usize>,
    other_kinds: BTreeMap<String, usize>,
    orphan_results: usize,
    parse_ms_p50: f64,
    parse_ms_p90: f64,
    parse_ms_p99: f64,
    parse_ms_max: f64,
    total_s: f64,
    mb_per_s: f64,
}

fn stats(agent: Option<Agent>, json: bool) -> Result<(), String> {
    let mut by_agent: BTreeMap<&'static str, (Stats, Vec<f64>)> = BTreeMap::new();
    for r in refs(agent) {
        let (st, times) = by_agent.entry(r.agent.as_str()).or_default();
        st.bytes += match &r.source {
            Source::File { path } => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            Source::Sqlite { .. } => 0,
        };
        let start = Instant::now();
        let parsed = agentparse::parse(&r);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        match parsed {
            Ok(s) => count(st, &s),
            Err(e) => *st.failures.entry(error_kind(&e)).or_default() += 1,
        }
    }
    let mut out = BTreeMap::new();
    for (name, (mut st, mut times)) in by_agent {
        times.sort_by(f64::total_cmp);
        let pct = |p: f64| {
            times
                .get(((times.len() as f64 - 1.0) * p).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        st.parse_ms_p50 = pct(0.5);
        st.parse_ms_p90 = pct(0.9);
        st.parse_ms_p99 = pct(0.99);
        st.parse_ms_max = times.last().copied().unwrap_or(0.0);
        st.total_s = times.iter().sum::<f64>() / 1000.0;
        st.mb_per_s = if st.total_s > 0.0 {
            st.bytes as f64 / 1e6 / st.total_s
        } else {
            0.0
        };
        out.insert(name, st);
    }
    if json {
        return print_json(&out);
    }
    for (name, st) in &out {
        say!("== {name}");
        say!(
            "  sessions {}  failures {:?}  warnings {}  entries {}  orphan results {}",
            st.sessions,
            st.failures,
            st.warnings,
            st.entries,
            st.orphan_results
        );
        say!(
            "  parse ms p50 {:.2} p90 {:.2} p99 {:.2} max {:.1}  total {:.1}s  {:.0} MB/s",
            st.parse_ms_p50,
            st.parse_ms_p90,
            st.parse_ms_p99,
            st.parse_ms_max,
            st.total_s,
            st.mb_per_s
        );
        for (label, map) in [
            ("entries", &st.entries_by_type),
            ("messages", &st.messages_by_origin),
            ("tools", &st.tool_calls_by_kind),
            ("status", &st.tool_calls_by_status),
            ("exit codes", &st.shell_exit_codes),
            ("other kinds", &st.other_kinds),
        ] {
            let mut items: Vec<_> = map.iter().collect();
            items.sort_by(|a, b| b.1.cmp(a.1));
            let shown: Vec<String> = items
                .iter()
                .take(25)
                .map(|(k, v)| format!("{k} {v}"))
                .collect();
            say!("  {label}: {}", shown.join(", "));
        }
    }
    Ok(())
}

fn count(st: &mut Stats, s: &Session) {
    st.sessions += 1;
    st.entries += s.entries.len();
    st.warnings += s.warnings.len();
    for e in &s.entries {
        let kind = serde_json::to_value(&e.kind).ok();
        let ty = kind
            .as_ref()
            .and_then(|k| k["type"].as_str())
            .unwrap_or("?")
            .to_string();
        *st.entries_by_type.entry(ty).or_default() += 1;
        match &e.kind {
            EntryKind::Message(m) => {
                *st.messages_by_origin
                    .entry(format!("{}/{}", lower(m.role), lower(m.origin)))
                    .or_default() += 1;
                for p in &m.parts {
                    match p {
                        Part::ToolResult(_) => st.orphan_results += 1,
                        Part::Other { kind, .. } => {
                            *st.other_kinds.entry(format!("part:{kind}")).or_default() += 1
                        }
                        _ => {}
                    }
                }
            }
            EntryKind::Other(o) => *st.other_kinds.entry(o.kind.clone()).or_default() += 1,
            _ => {}
        }
    }
    for c in s.tool_calls() {
        let kind = serde_json::to_value(&c.tool)
            .ok()
            .and_then(|v| v["kind"].as_str().map(str::to_string))
            .unwrap_or_default();
        *st.tool_calls_by_kind.entry(kind).or_default() += 1;
        *st.tool_calls_by_status.entry(lower(c.status)).or_default() += 1;
        if let Tool::Shell(_) = c.tool {
            let code = c
                .output
                .as_ref()
                .and_then(|o| o.shell.as_ref())
                .and_then(|s| s.exit_code);
            *st.shell_exit_codes
                .entry(code.map_or("none".into(), |c| c.to_string()))
                .or_default() += 1;
        }
    }
}

fn error_kind(e: &agentparse::Error) -> String {
    match e {
        agentparse::Error::Io(_) => "io",
        agentparse::Error::Sqlite(_) => "sqlite",
        agentparse::Error::NotASession(_) => "not_a_session",
        agentparse::Error::NotFound(_) => "not_found",
    }
    .to_string()
}
