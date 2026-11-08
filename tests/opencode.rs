//! OpenCode parser tests through the public API. Fixture-based tests build databases from the
//! synthetic fixtures (`tests/fixtures/opencode/*.sql`) and assert the facts listed in the
//! fixture README; the later tests build small databases inline with the row shapes opencode
//! writes, one per shape.

use agentparse::{
    Agent, Entry, EntryKind, Error, FileOp, Message, Origin, Part, Role, Session, Source, Tool,
    ToolCall, ToolStatus, opencode,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const V1_SQL: &str = include_str!("fixtures/opencode/v1.sql");
const V2_SQL: &str = include_str!("fixtures/opencode/v2.sql");

const MAIN: &str = "ses_ed65ba2ffffeDtcAUZWJkZu6qD";
const CHILD: &str = "ses_ed65a5c6ffccKWy8YrFgn0VuXa";
const MIGRATED: &str = "ses_ed52a75fff9cZv864nVGS5jPGC";
const MIGRATED_CHILD: &str = "ses_ed52a633ff93Y7Vatlwl1zyPKr";
const NATIVE: &str = "ses_ecd6ad17ff80AEbRxnOg8B21Mn";
const NATIVE_CHILD: &str = "ses_ecd6a835ff7fukfa0R96znWxUu";
const LEGACY: &str = "ses_79a8c3d52ffeLegacyDemoSess01";

struct Db {
    _dir: TempDir,
    path: PathBuf,
}

impl Db {
    fn from_sql(sql: &str) -> Db {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("opencode.db");
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        Db { _dir: dir, path }
    }

    fn conn(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn parse(&self, id: &str) -> Session {
        let s = opencode::parse_db(&self.path, id).unwrap();
        assert_round_trip(&s);
        s
    }
}

fn assert_round_trip(s: &Session) {
    let json = serde_json::to_string(s).unwrap();
    let back: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, s, "JSON round trip of {}", s.id);
}

fn legacy_session_file() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opencode/storage");
    root.join("session/e4d909c290d0fb1ca068ffaddf22cbd0cd8aaaaa")
        .join(format!("{LEGACY}.json"))
}

// ---- small views over a parsed session -----------------------------------------------------

fn msgs(s: &Session) -> Vec<&Message> {
    s.messages().collect()
}

fn count(s: &Session, role: Role, origin: Origin) -> usize {
    s.messages()
        .filter(|m| m.role == role && m.origin == origin)
        .count()
}

fn calls(s: &Session) -> Vec<&ToolCall> {
    s.tool_calls()
}

fn call<'a>(s: &'a Session, id: &str) -> &'a ToolCall {
    calls(s)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no call {id}"))
}

fn message<'a>(s: &'a Session, id: &str) -> &'a Message {
    s.entries
        .iter()
        .find_map(|e| match &e.kind {
            EntryKind::Message(m) if e.id.as_deref() == Some(id) => Some(m),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no message {id}"))
}

fn shell(c: &ToolCall) -> &agentparse::ShellResult {
    c.output
        .as_ref()
        .and_then(|o| o.shell.as_ref())
        .unwrap_or_else(|| panic!("no shell result on {}", c.id))
}

fn text(c: &ToolCall) -> &str {
    &c.output.as_ref().unwrap().text
}

fn kinds(s: &Session) -> Vec<&'static str> {
    s.entries
        .iter()
        .map(|e| match &e.kind {
            EntryKind::Message(m) => match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            },
            EntryKind::Compaction(_) => "compaction",
            EntryKind::Turn(_) => "turn",
            EntryKind::Settings(_) => "settings",
            EntryKind::Notice(_) => "notice",
            EntryKind::Hook(_) => "hook",
            EntryKind::Error(_) => "error",
            EntryKind::Rewind(_) => "rewind",
            EntryKind::Other(_) => "other",
        })
        .collect()
}

fn near(a: Option<f64>, b: f64) {
    assert!((a.unwrap() - b).abs() < 1e-9, "{a:?} vs {b}");
}

// ---- listing and lookup -----------------------------------------------------------------------

#[test]
fn list_reads_v1_tables_when_session_v2_is_empty() {
    let db = Db::from_sql(V1_SQL);
    let refs = opencode::list(&db.path).unwrap();
    let ids: Vec<_> = refs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(refs.len(), 2);
    assert!(ids.contains(&MAIN) && ids.contains(&CHILD));
    let child = refs.iter().find(|r| r.id == CHILD).unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(MAIN));
    assert!(child.updated_at.is_some());
    assert_eq!(child.agent, Agent::Opencode);
    assert_eq!(
        child.source,
        Source::Sqlite {
            path: db.path.clone(),
            session_id: CHILD.into()
        }
    );
    assert!(
        refs.iter()
            .find(|r| r.id == MAIN)
            .unwrap()
            .parent_id
            .is_none()
    );
}

#[test]
fn list_counts_a_migrated_session_once() {
    let db = Db::from_sql(V2_SQL);
    let refs = opencode::list(&db.path).unwrap();
    let mut ids: Vec<_> = refs.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    let mut want = vec![MIGRATED, MIGRATED_CHILD, NATIVE, NATIVE_CHILD];
    want.sort_unstable();
    assert_eq!(ids, want);
    // Newest first.
    assert!(refs.windows(2).all(|w| w[0].updated_at >= w[1].updated_at));
}

#[test]
fn unknown_session_is_not_found_and_missing_file_is_io() {
    let db = Db::from_sql(V2_SQL);
    assert!(
        matches!(opencode::parse_db(&db.path, "ses_nope"), Err(Error::NotFound(id)) if id == "ses_nope")
    );
    assert!(matches!(
        opencode::parse_db(&db.path.with_extension("missing"), MAIN),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        opencode::list(&db.path.with_extension("missing")),
        Err(Error::Io(_))
    ));
}

#[test]
fn empty_or_partial_databases_list_nothing() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, b"").unwrap();
    assert!(opencode::list(&empty).unwrap().is_empty());
    let partial = Db::from_sql("CREATE TABLE kv (key text, value text);");
    assert!(opencode::list(&partial.path).unwrap().is_empty());
    assert!(matches!(
        opencode::parse_db(&partial.path, "ses_x"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn a_file_that_is_not_a_database_is_a_sqlite_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("junk.db");
    std::fs::write(&path, b"this is not a database file at all, just text").unwrap();
    assert!(matches!(opencode::list(&path), Err(Error::Sqlite(_))));
}

// ---- v1 main session ------------------------------------------------------------------------

#[test]
fn v1_session_header_and_usage() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    assert_eq!(
        (s.agent, s.format.as_str()),
        (Agent::Opencode, "opencode-v1")
    );
    assert_eq!(s.title.as_deref(), Some("Fix the failing test"));
    assert_eq!(s.cwd.as_deref(), Some("/work/demo"));
    assert_eq!(s.cli_version.as_deref(), Some("1.18.35"));
    assert!(s.parent.is_none() && s.git.is_none());
    assert_eq!(s.models, ["demo-sonnet", "demo-gpt"]);
    // README: input 14330, output 1365 (excludes reasoning), reasoning 400, cache 14000/950, cost 0.0179.
    let u = s.usage;
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read, u.cache_write),
        (14330, 1365 + 400, 400, 14000, 950)
    );
    assert_eq!(u.total(), 31045); // README sum: input + output + reasoning + cache
    near(u.cost_usd, 0.0179);
    // Session row columns without a typed home are kept.
    let extra = s.extra.as_ref().unwrap();
    assert_eq!(extra["tokens_input"], 14330);
    assert_eq!(extra["slug"], "calm-harbor");
    assert_eq!(extra["permission"].as_array().unwrap().len(), 3);
    assert_eq!(extra["summary_files"], 3);
    assert!(s.started_at.is_some() && s.updated_at >= s.started_at);
    assert!(s.warnings.is_empty());
}

#[test]
fn v1_entry_shape_follows_the_readme_counts() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    // 8 user and 15 assistant messages in the tables. The `!cmd` pair (marker + bash message) is one
    // Command message, the compaction marker and its summary message are one Compaction entry.
    assert_eq!(count(&s, Role::User, Origin::Human), 6);
    assert_eq!(count(&s, Role::User, Origin::Command), 1);
    assert_eq!(count(&s, Role::Assistant, Origin::Model), 13);
    assert_eq!(kinds(&s).iter().filter(|k| **k == "compaction").count(), 1);
    assert_eq!(kinds(&s).iter().filter(|k| **k == "error").count(), 2);
    assert_eq!(kinds(&s).iter().filter(|k| **k == "turn").count(), 1);
    assert_eq!(kinds(&s).last(), Some(&"other"));
    assert_eq!(calls(&s).len(), 17);
    // step parts stay as Other parts, one per step
    let others = |kind: &str| {
        msgs(&s)
            .iter()
            .flat_map(|m| &m.parts)
            .filter(|p| matches!(p, Part::Other { kind: k, .. } if k == kind))
            .count()
    };
    // README counts 14/12 step parts; one pair sits in the summary message the compaction absorbed
    assert_eq!(
        (others("step-start"), others("step-finish"), others("patch")),
        (13, 11, 1)
    );
    // assistant messages point at the user message that triggered the turn
    let first = message(&s, "msg_129a464d00048HHebBUCLkpqMH");
    assert_eq!(first.stop_reason.as_deref(), Some("tool-calls"));
    let entry = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_129a464d00048HHebBUCLkpqMH"))
        .unwrap();
    assert_eq!(
        entry.turn_id.as_deref(),
        Some("msg_129a45d00002DJC3QKUfRB41eK")
    );
    assert_eq!(entry.ts, Some(1791700002000));
}

#[test]
fn v1_message_usage_comes_from_the_message_not_the_steps() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    // message #4 has two steps; its tokens and cost are the sum of both
    let m = message(&s, "msg_129a4802800eQJdWUeniQSRYCh");
    let u = m.usage.unwrap();
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read, u.cache_write),
        (350, 95, 0, 4300, 0)
    );
    near(u.cost_usd, 0.0027);
    assert_eq!(m.model.as_deref(), Some("demo-sonnet"));
    assert_eq!(m.agent.as_deref(), Some("build"));
    // reasoning and text of an assistant message
    let first = message(&s, "msg_129a464d00048HHebBUCLkpqMH");
    assert!(first.parts.iter().any(
        |p| matches!(p, Part::Reasoning { text: Some(t), encrypted: true } if t.starts_with("The test output"))
    ));
    assert_eq!(first.text(), "I will start by checking the working tree.");
    // openai reasoning: empty text, encrypted blob only
    let openai = message(&s, "msg_129a59580028l3jelVCNspMz0Z");
    assert!(openai.parts.iter().any(|p| matches!(
        p,
        Part::Reasoning {
            text: None,
            encrypted: true
        }
    )));
    assert_eq!(openai.phase.as_deref(), Some("commentary"));
    // the encrypted blob is never kept anywhere
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("fake-encrypted-reasoning-blob") && !json.contains("ZmFrZS1zaWduYXR1cmU")
    );
}

#[test]
fn v1_shell_calls_carry_exit_code_duration_and_truncation() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    let ok = call(&s, "toolu_demo0001");
    assert_eq!(ok.status, ToolStatus::Ok);
    assert_eq!(ok.status_detail.as_deref(), Some("completed"));
    let Tool::Shell(sh) = &ok.tool else {
        panic!("{:?}", ok.tool)
    };
    assert_eq!(sh.command, "git status --short");
    assert_eq!(sh.cwd.as_deref(), Some("/work/demo")); // assistant path.cwd
    assert_eq!(sh.description.as_deref(), Some("Show working tree status"));
    let r = shell(ok);
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.stderr.as_ref()),
        (Some(0), Some(" M src/math.ts\n"), None)
    );
    assert_eq!(r.duration_ms, Some(80));
    assert_eq!(
        (ok.started_at, ok.ended_at),
        (Some(1791700003200), Some(1791700003280))
    );

    // exit 2: the call is an Error, output truncated with a saved full output
    let failed = call(&s, "toolu_demo0002");
    assert_eq!(failed.status, ToolStatus::Error);
    assert!(failed.failed());
    let Tool::Shell(sh) = &failed.tool else {
        panic!()
    };
    assert_eq!(
        (sh.timeout_ms, sh.cwd.as_deref()),
        (Some(120000), Some("/work/demo"))
    );
    assert_eq!(shell(failed).exit_code, Some(2));
    let out = failed.output.as_ref().unwrap();
    assert!(out.truncated);
    assert_eq!(
        out.full_output_path.as_deref(),
        Some("/home/demo/.local/share/opencode/tool-output/tool_c0ffee000001")
    );
    // mapped metadata keys are not repeated in data
    let meta = &out.data.as_ref().unwrap()["metadata"];
    assert!(
        meta.get("exit").is_none()
            && meta.get("truncated").is_none()
            && meta.get("outputPath").is_none()
    );
}

#[test]
fn v1_file_tools_map_to_typed_inputs_and_file_changes() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    let read = call(&s, "toolu_demo0003");
    assert!(
        matches!(&read.tool, Tool::Read { path, offset: Some(1), limit: Some(40) } if path == "/work/demo/src/math.ts")
    );
    let grep = call(&s, "toolu_demo0004");
    assert!(
        matches!(&grep.tool, Tool::Search { pattern, path: Some(p), glob: Some(g) } if pattern == "add\\(" && p == "/work/demo/src" && g == "*.ts")
    );
    let glob = call(&s, "toolu_demo0005");
    assert!(
        matches!(&glob.tool, Tool::Glob { pattern, path: Some(_) } if pattern == "src/**/*.ts")
    );

    let edit = call(&s, "toolu_demo0006");
    let Tool::Edit { path, edits } = &edit.tool else {
        panic!()
    };
    assert_eq!(path, "/work/demo/src/math.ts");
    assert_eq!(
        (
            edits[0].old.as_str(),
            edits[0].new.as_str(),
            edits[0].replace_all
        ),
        ("return a - b", "return a + b", false)
    );
    let files = &edit.output.as_ref().unwrap().files;
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].op, files[0].additions, files[0].deletions),
        (FileOp::Update, Some(1), Some(1))
    );
    assert!(
        files[0]
            .diff
            .as_deref()
            .unwrap()
            .starts_with("@@ -1,3 +1,3 @@")
    );

    let write = call(&s, "toolu_demo0007");
    assert!(
        matches!(&write.tool, Tool::Write { path, content: Some(c) } if path == "/work/demo/src/sub.test.ts" && c.contains("sub"))
    );
    let out = write.output.as_ref().unwrap();
    assert_eq!(
        (out.files[0].path.as_str(), out.files[0].op),
        ("/work/demo/src/sub.test.ts", FileOp::Add)
    );
    // LSP diagnostics stay in data
    assert_eq!(
        out.data.as_ref().unwrap()["metadata"]["diagnostics"]
            .as_object()
            .unwrap()
            .len(),
        1
    );

    let patch = call(&s, "toolu_demo0008");
    assert!(matches!(&patch.tool, Tool::Patch { patch } if patch.starts_with("*** Begin Patch")));
    let files = &patch.output.as_ref().unwrap().files;
    let got: Vec<_> = files
        .iter()
        .map(|f| (f.path.as_str(), f.op, f.additions, f.deletions))
        .collect();
    assert_eq!(
        got,
        [
            ("/work/demo/src/mul.ts", FileOp::Add, Some(1), Some(0)),
            ("/work/demo/src/math.ts", FileOp::Update, Some(1), Some(1)),
            ("/work/demo/src/old.ts", FileOp::Delete, Some(0), Some(1)),
        ]
    );
}

#[test]
fn v1_web_todo_task_mcp_and_invalid_tools() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    assert!(
        matches!(&call(&s, "call_demo0009").tool, Tool::WebFetch { url: Some(u), .. } if u == "https://example.com/docs/testing")
    );
    assert!(
        matches!(&call(&s, "call_demo0010").tool, Tool::WebSearch { query: Some(q) } if q == "bun test exit code")
    );
    let Tool::Todo { items } = &call(&s, "call_demo0011").tool else {
        panic!()
    };
    let got: Vec<_> = items
        .iter()
        .map(|i| {
            (
                i.content.as_str(),
                i.status.as_deref(),
                i.priority.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("Fix add", Some("completed"), Some("high")),
            ("Add mul tests", Some("in_progress"), Some("medium")),
            ("Update docs", Some("pending"), Some("low")),
        ]
    );
    let task = call(&s, "call_demo0012");
    assert!(
        matches!(&task.tool, Tool::Task { description: Some(d), prompt: Some(_), agent_type: Some(a), name: None } if d == "Find failing assertions" && a == "general")
    );
    assert_eq!(task.child_session.as_deref(), Some(CHILD));
    // MCP and plugin tools are not typed; the raw name and input stay
    let mcp = call(&s, "call_demo0013");
    assert_eq!(
        (mcp.name.as_str(), &mcp.tool),
        ("demo-server_search", &Tool::Other)
    );
    assert_eq!(mcp.input, json!({"query": "retry backoff"}));
    let invalid = call(&s, "call_demo0014");
    assert_eq!(
        (&invalid.tool, invalid.input["tool"].as_str()),
        (&Tool::Other, Some("bsh"))
    );
}

#[test]
fn v1_interrupted_and_pending_tools() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    // error state with metadata.interrupted and the partial output kept in the stream
    let aborted = call(&s, "call_demo0015");
    assert_eq!(
        (aborted.status, aborted.status_detail.as_deref()),
        (ToolStatus::Interrupted, Some("error"))
    );
    let r = shell(aborted);
    assert!(r.interrupted && r.exit_code.is_none());
    assert_eq!(
        r.stdout.as_deref(),
        Some("formatting src/a.ts\nformatting src/b.ts\n")
    );
    assert_eq!(text(aborted), "Tool execution aborted");
    assert_eq!(r.duration_ms, Some(4000));
    // a pending call in a session with later messages was cut off; the raw partial JSON stays
    let pending = call(&s, "call_demo0016");
    assert_eq!(
        (pending.status, pending.status_detail.as_deref()),
        (ToolStatus::Interrupted, Some("pending"))
    );
    assert_eq!(pending.tool, Tool::Other);
    // the partial JSON is the input (as for a v2 streaming call); no result, so no output
    assert_eq!(pending.input, json!("{\"command\":\"bun run build"));
    assert!(pending.output.is_none());
    assert!(pending.started_at.is_none());
}

#[test]
fn v1_assistant_errors_retry_and_abort() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    let errors: Vec<(&Entry, &agentparse::ErrorInfo)> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Error(i) => Some((e, i)),
            _ => None,
        })
        .collect();
    // the retry part of message #9 is an entry right after that message
    let (retry_entry, retry) = errors[0];
    assert_eq!(
        (
            retry.kind.as_deref(),
            retry.status,
            retry.retry,
            retry.message.as_str()
        ),
        (Some("APIError"), Some(529), Some(1), "Overloaded")
    );
    assert_eq!(retry_entry.ts, Some(1791700080000));
    let holder = s
        .entries
        .iter()
        .position(|e| e.id.as_deref() == Some("msg_129a59580028l3jelVCNspMz0Z"))
        .unwrap();
    assert!(std::ptr::eq(&s.entries[holder + 1], retry_entry));
    assert_eq!(
        retry_entry.turn_id.as_deref(),
        Some("msg_129a58db0024LV2k0G2u7jBUEh")
    );
    // assistant error after the (empty) message
    let (err_entry, err) = errors[1];
    assert_eq!(
        (
            err.kind.as_deref(),
            err.status,
            err.retry,
            err.message.as_str()
        ),
        (Some("APIError"), Some(503), None, "Service unavailable")
    );
    let extra = err_entry.extra.as_ref().unwrap();
    assert_eq!(extra["isRetryable"], true);
    assert_eq!(extra["responseHeaders"]["retry-after"], "30");
    let at = s
        .entries
        .iter()
        .position(|e| e.id.as_deref() == Some("msg_129a6b0b403enp1MoOgmnnFnXk"))
        .unwrap();
    assert!(
        message(&s, "msg_129a6b0b403enp1MoOgmnnFnXk")
            .parts
            .is_empty()
    );
    assert!(matches!(&s.entries[at + 1].kind, EntryKind::Error(_)));
    assert_eq!(
        message(&s, "msg_129a6b0b403enp1MoOgmnnFnXk")
            .stop_reason
            .as_deref(),
        Some("error")
    );
    // user abort is an interrupt marker, not an error
    let turn = s.entries.iter().find_map(|e| match &e.kind {
        EntryKind::Turn(t) => Some(t),
        _ => None,
    });
    let turn = turn.unwrap();
    assert_eq!(
        (turn.status.as_deref(), turn.error.as_deref()),
        (Some("interrupted"), None)
    );
    let entry = s
        .entries
        .iter()
        .find(|e| matches!(e.kind, EntryKind::Turn(_)))
        .unwrap();
    assert_eq!(
        entry.extra.as_ref().unwrap()["marker"],
        "The operation was aborted."
    );
}

#[test]
fn v1_user_run_command_pair_is_one_command_message() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    // the entry keeps the marker's id, which the compaction's tail_start_id points at
    let cmd = message(&s, "msg_129a8912c049JK26fFvV1Qqo98");
    assert_eq!((cmd.role, cmd.origin), (Role::User, Origin::Command));
    assert_eq!(cmd.model.as_deref(), Some("demo-gpt"));
    assert_eq!(cmd.parts.len(), 1);
    let Part::ToolCall(c) = &cmd.parts[0] else {
        panic!()
    };
    assert_eq!(c.id, "01JDEMOUSERSHELL00000001A");
    assert!(matches!(&c.tool, Tool::Shell(sh) if sh.command == "ls src"));
    assert_eq!(shell(c).exit_code, Some(0));
    assert_eq!(
        shell(c).stdout.as_deref(),
        Some("math.test.ts\nmath.ts\nmul.ts\n")
    );
    // the synthetic marker text is not a message of its own
    assert!(
        msgs(&s)
            .iter()
            .all(|m| !m.text().contains("executed by the user"))
    );
    let entry = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_129a8912c049JK26fFvV1Qqo98"))
        .unwrap();
    let extra = entry.extra.as_ref().unwrap();
    assert_eq!(extra["message_id"], "msg_129a8919004ba3RwpRr87lLLNl");
    assert_eq!(extra["user"]["model"]["providerID"], "openai");
    assert!(extra["user"].get("agent").is_none());
    assert_eq!(entry.ts, Some(1791700275500));
}

#[test]
fn v1_compaction_absorbs_the_summary_message() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    let (entry, c) = s
        .entries
        .iter()
        .find_map(|e| match &e.kind {
            EntryKind::Compaction(c) => Some((e, c)),
            _ => None,
        })
        .unwrap();
    assert_eq!(c.trigger.as_deref(), Some("auto"));
    assert_eq!(
        c.summary.as_deref(),
        Some(
            "## Goal\nFix the failing test in /work/demo.\n\n## Done\n- Fixed the sign in add\n- Added mul\n\n## Next\n- Update docs"
        )
    );
    let u = c.usage.unwrap();
    assert_eq!((u.input, u.output), (4200, 380));
    let extra = entry.extra.as_ref().unwrap();
    assert_eq!(extra["overflow"], false);
    assert_eq!(extra["tail_start_id"], "msg_129a8912c049JK26fFvV1Qqo98");
    assert_eq!(
        extra["summary_message"]["id"],
        "msg_129a97de404ftZjiVmW8vWN7Fh"
    );
    // neither the marker nor the summary carrier appear again as messages
    assert!(
        s.entries
            .iter()
            .all(|e| e.id.as_deref() != Some("msg_129a97de404ftZjiVmW8vWN7Fh"))
    );
    assert!(msgs(&s).iter().all(|m| !m.text().contains("## Goal")));
}

#[test]
fn v1_user_attachments_agent_mentions_and_todo_table() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    let user = message(&s, "msg_129a58db0024LV2k0G2u7jBUEh");
    assert!(user.is_human());
    assert_eq!(
        user.text(),
        "check this screenshot and look up the docs for testing"
    );
    let Part::Attachment(media) = &user.parts[1] else {
        panic!("{:?}", user.parts)
    };
    // inline data is sized, never stored
    assert_eq!(
        (
            media.mime.as_deref(),
            media.name.as_deref(),
            media.url.as_deref(),
            media.bytes
        ),
        (Some("image/png"), Some("failure.png"), None, Some(70))
    );
    assert!(
        matches!(&user.parts[2], Part::Other { kind, data } if kind == "agent" && data["name"] == "general")
    );
    assert!(!serde_json::to_string(&s).unwrap().contains("iVBORw0KGgo"));
    // todo table rows end the session as one Other entry
    let Some(Entry {
        kind: EntryKind::Other(o),
        ..
    }) = s.entries.last()
    else {
        panic!()
    };
    assert_eq!(o.kind, "todos");
    assert_eq!(
        o.data,
        json!([
            {"content": "Fix add", "status": "completed", "priority": "high"},
            {"content": "Add mul tests", "status": "in_progress", "priority": "medium"},
            {"content": "Update docs", "status": "pending", "priority": "low"},
        ])
    );
}

// ---- v1 child session -----------------------------------------------------------------------

#[test]
fn v1_child_session_links_to_the_spawning_call() {
    let s = Db::from_sql(V1_SQL).parse(CHILD);
    let parent = s.parent.as_ref().unwrap();
    assert_eq!(parent.session_id, MAIN);
    assert_eq!(parent.call_id.as_deref(), Some("call_demo0012"));
    assert_eq!(parent.agent_type.as_deref(), Some("general"));
    assert_eq!(
        s.title.as_deref(),
        Some("Find failing assertions (@general subagent)")
    );
    // a child's first prompt is the parent's task prompt: Origin::Agent
    assert_eq!(count(&s, Role::User, Origin::Human), 0);
    assert_eq!(count(&s, Role::User, Origin::Agent), 1);
    assert_eq!(calls(&s).len(), 1);
    assert!(matches!(&calls(&s)[0].tool, Tool::Search { .. }));
    assert_eq!(
        (s.usage.input, s.usage.output, s.usage.cache_read),
        (1100, 55, 0)
    );
    near(s.usage.cost_usd, 0.0);
}

// ---- migrated sessions: v1 rows win ----------------------------------------------------------

#[test]
fn migrated_session_reads_v1_rows_then_the_v2_only_rows() {
    let s = Db::from_sql(V2_SQL).parse(MIGRATED);
    assert_eq!(s.format, "opencode-v1+v2");
    assert_eq!(s.title.as_deref(), Some("Update the readme"));
    assert_eq!(
        kinds(&s),
        ["user", "assistant", "assistant", "assistant", "system"]
    );
    // v1 wins: the original time.completed (v2 would say +2500), tool title, step parts, mode
    let first = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_12ad58fdc066C0DqNrUXU3WNwV"))
        .unwrap();
    assert_eq!(
        first.extra.as_ref().unwrap()["time"]["completed"],
        1791720002600u64
    );
    let EntryKind::Message(m) = &first.kind else {
        panic!()
    };
    assert!(
        m.parts
            .iter()
            .any(|p| matches!(p, Part::Other { kind, .. } if kind == "step-finish"))
    );
    let Part::ToolCall(bash) = m
        .parts
        .iter()
        .find(|p| matches!(p, Part::ToolCall(_)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        bash.output.as_ref().unwrap().data.as_ref().unwrap()["title"],
        "cat README.md"
    );
    assert_eq!(first.extra.as_ref().unwrap().get("mode"), None);
    // the migration notice is a v2-only row, appended last as a system message
    let EntryKind::Message(notice) = &s.entries.last().unwrap().kind else {
        panic!()
    };
    assert_eq!(
        (notice.role, notice.origin, notice.kind.as_deref()),
        (Role::System, Origin::Context, Some("system"))
    );
    assert!(
        notice
            .text()
            .contains("The `bash` tool is now named `shell`")
    );
    // usage: sum of the v1 messages
    let u = s.usage;
    assert_eq!((u.input, u.output, u.reasoning), (2400, 130 + 20, 20));
    near(u.cost_usd, 0.0033);
    // tools keep v1 names and typed views; the task links to the migrated child
    let ids: Vec<_> = calls(&s)
        .iter()
        .map(|c| (c.name.as_str(), c.child_session.as_deref()))
        .collect();
    assert_eq!(ids, [("bash", None), ("task", Some(MIGRATED_CHILD))]);
}

#[test]
fn migrated_child_is_v1_plus_v2_and_keeps_its_parent() {
    let s = Db::from_sql(V2_SQL).parse(MIGRATED_CHILD);
    assert_eq!(s.format, "opencode-v1+v2");
    assert_eq!(
        s.parent.as_ref().map(|p| p.session_id.as_str()),
        Some(MIGRATED)
    );
    assert_eq!(
        s.parent.as_ref().unwrap().call_id.as_deref(),
        Some("call_demom002")
    );
    assert_eq!(kinds(&s), ["user", "assistant", "system"]);
    assert_eq!((s.usage.input, s.usage.output), (300, 10));
}

#[test]
fn a_v2_row_written_after_the_migration_is_appended() {
    let db = Db::from_sql(V2_SQL);
    let data =
        json!({"text": "one more thing", "files": [], "time": {"created": 1791720100000u64}});
    db.conn()
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) VALUES (?1, ?2, 'user', 5, 1791720100000, 1791720100000, ?3)",
            params!["msg_zz_after_migration", MIGRATED, data.to_string()],
        )
        .unwrap();
    let s = db.parse(MIGRATED);
    assert_eq!(
        kinds(&s),
        [
            "user",
            "assistant",
            "assistant",
            "assistant",
            "system",
            "user"
        ]
    );
    let last = s.entries.last().unwrap();
    assert_eq!(last.id.as_deref(), Some("msg_zz_after_migration"));
    let EntryKind::Message(m) = &last.kind else {
        panic!()
    };
    assert!(m.is_human());
    assert_eq!(m.text(), "one more thing");
    // v1 rows are not duplicated by their v2 copies
    assert_eq!(count(&s, Role::Assistant, Origin::Model), 3);
}

// ---- native v2 session ------------------------------------------------------------------------

#[test]
fn v2_native_session_header_usage_and_order() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    assert_eq!(s.format, "opencode-v2");
    assert_eq!(s.title.as_deref(), Some("Add a retry helper"));
    assert_eq!(
        (s.cwd.as_deref(), s.cli_version.as_deref()),
        (Some("/work/demo"), Some("2.0.27"))
    );
    assert_eq!(s.models, ["demo-gpt", "demo-sonnet"]);
    assert_eq!(
        kinds(&s),
        [
            "user",
            "user",
            "system",
            "user",
            "user",
            "assistant",
            "assistant",
            "assistant",
            "user",
            "assistant",
            "turn",
            "user",
            "settings",
            "settings",
            "assistant",
            "compaction",
            "assistant",
            "error",
            "turn",
        ]
    );
    // README: input 9350, output 1060 (excludes reasoning), reasoning 150, cache 5700/600, cost 0.0134
    // (sum of the assistant rows plus the compaction row)
    let u = s.usage;
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read, u.cache_write),
        (9350, 1060 + 150, 150, 5700, 600)
    );
    near(u.cost_usd, 0.0134);
    let extra = s.extra.as_ref().unwrap();
    assert_eq!(
        (extra["agent"].as_str(), extra["idle_outcome"].as_str()),
        (Some("plan"), Some("failed"))
    );
    assert_eq!(extra["model"]["id"], "demo-sonnet");
    assert_eq!(extra["tokens_input"], 9350);
    assert!(s.parent.is_none() && s.warnings.is_empty());
}

#[test]
fn v2_user_synthetic_system_skill_rows() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let user = message(&s, "msg_132952e80081LAJnrjto27uBcD");
    assert!(user.is_human());
    assert_eq!(user.text(), "add a retry helper to src/net.ts");
    let Part::Attachment(m) = &user.parts[1] else {
        panic!()
    };
    assert_eq!(
        (
            m.mime.as_deref(),
            m.name.as_deref(),
            m.url.as_deref(),
            m.bytes
        ),
        (Some("image/png"), Some("spec.png"), None, Some(70))
    );
    assert!(
        matches!(&user.parts[2], Part::Other { kind, data } if kind == "agent" && data["name"] == "general")
    );

    let synthetic = message(&s, "msg_132953074082pZ06he4X4t7KTZ");
    assert_eq!(
        (synthetic.role, synthetic.origin, synthetic.kind.as_deref()),
        (Role::User, Origin::Context, Some("synthetic"))
    );
    assert_eq!(synthetic.text(), "Reminder: keep changes small.");

    let system = message(&s, "msg_132953268083Zt75C5RC0iG6Yx");
    assert_eq!(
        (system.role, system.origin),
        (Role::System, Origin::Context)
    );
    assert!(
        system
            .text()
            .starts_with("The available tools have changed.")
    );

    let skill = message(&s, "msg_13295345c084Lsdg2dIf0UlhkU");
    assert_eq!(
        (skill.role, skill.kind.as_deref(), skill.text().as_str()),
        (Role::User, Some("skill"), "Use small commits.")
    );
    let entry = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_13295345c084Lsdg2dIf0UlhkU"))
        .unwrap();
    assert_eq!(entry.extra.as_ref().unwrap()["skill"], "demo-skill");
}

#[test]
fn v2_shell_row_is_a_command_message_with_the_full_result() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let cmd = message(&s, "msg_132953844085kB2oAf0R52kjj2");
    assert_eq!((cmd.role, cmd.origin), (Role::User, Origin::Command));
    let Part::ToolCall(c) = &cmd.parts[0] else {
        panic!()
    };
    assert_eq!(
        (c.id.as_str(), c.name.as_str(), c.status),
        ("sh_user000001", "shell", ToolStatus::Ok)
    );
    assert_eq!(c.status_detail.as_deref(), Some("exited"));
    assert!(matches!(&c.tool, Tool::Shell(sh) if sh.command == "git status --short"));
    // the row recorded a command string, not an input object
    assert_eq!(c.input, json!("git status --short"));
    let r = shell(c);
    assert_eq!(
        (
            r.exit_code,
            r.stdout.as_deref(),
            r.stderr.as_ref(),
            r.duration_ms
        ),
        (Some(0), Some(" M src/net.ts\n"), None, Some(120))
    );
    // output.size and output.cursor stay as data
    assert_eq!(
        c.output.as_ref().unwrap().data.as_ref().unwrap(),
        &json!({"cursor": 14, "size": 14})
    );
}

#[test]
fn v2_tool_calls_in_assistant_rows() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let row = message(&s, "msg_132954014086GxRGZlQM7hWhEa");
    assert_eq!(
        (
            row.model.as_deref(),
            row.agent.as_deref(),
            row.stop_reason.as_deref()
        ),
        (Some("demo-gpt"), Some("build"), Some("tool-calls"))
    );
    assert_eq!(row.phase.as_deref(), Some("commentary"));
    assert!(matches!(
        &row.parts[0],
        Part::Reasoning {
            text: None,
            encrypted: true
        }
    ));
    assert_eq!(
        row.parts[1],
        Part::Text {
            text: "I will inspect the module first.".into()
        }
    );
    let u = row.usage.unwrap();
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read),
        (1500, 120 + 60, 60, 0)
    );

    let shell_call = call(&s, "call_demov201");
    let Tool::Shell(sh) = &shell_call.tool else {
        panic!()
    };
    assert_eq!(
        (
            sh.command.as_str(),
            sh.cwd.as_deref(),
            sh.timeout_ms,
            sh.background
        ),
        ("bun test", Some("/work/demo"), Some(120000), false)
    );
    let r = shell(shell_call);
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.duration_ms),
        (Some(0), Some("1 pass\n0 fail\n"), Some(780))
    );
    assert_eq!(
        (shell_call.started_at, shell_call.ended_at),
        (Some(1791850005420), Some(1791850006200))
    );

    let read = call(&s, "call_demov202");
    assert!(
        matches!(&read.tool, Tool::Read { path, offset: Some(1), limit: Some(50) } if path == "/work/demo/src/net.ts")
    );

    let edit = call(&s, "call_demov203");
    assert!(
        matches!(&edit.tool, Tool::Edit { path, edits } if path == "/work/demo/src/net.ts" && edits.len() == 1)
    );
    let files = &edit.output.as_ref().unwrap().files;
    assert_eq!(
        (
            files[0].path.as_str(),
            files[0].op,
            files[0].additions,
            files[0].deletions
        ),
        ("/work/demo/src/net.ts", FileOp::Update, Some(5), Some(0))
    );

    let patch = call(&s, "call_demov204");
    assert!(matches!(&patch.tool, Tool::Patch { .. }));
    let got: Vec<_> = patch
        .output
        .as_ref()
        .unwrap()
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.op, f.additions, f.deletions))
        .collect();
    assert_eq!(
        got,
        [
            ("/work/demo/src/index.ts", FileOp::Update, Some(1), Some(0)),
            ("/work/demo/src/old.ts", FileOp::Delete, Some(0), Some(1))
        ]
    );
}

#[test]
fn v2_subagent_error_and_background_shell() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let sub = call(&s, "call_demov207");
    assert!(
        matches!(&sub.tool, Tool::Task { description: Some(d), agent_type: Some(a), .. } if d == "Locate callers" && a == "general")
    );
    assert_eq!(sub.child_session.as_deref(), Some(NATIVE_CHILD));
    assert_eq!(sub.status, ToolStatus::Ok);

    let failed = call(&s, "call_demov206");
    assert_eq!(
        (failed.status, failed.status_detail.as_deref()),
        (ToolStatus::Error, Some("tool.execution"))
    );
    assert_eq!(text(failed), "File not found: /work/demo/src/missing.ts");

    // the background job: moved to the background at call time, completed by a later synthetic row
    let bg = call(&s, "call_demov205");
    assert!(matches!(&bg.tool, Tool::Shell(sh) if sh.background && sh.command == "bun run build"));
    assert!(text(bg).starts_with("Command moved to the background"));
    let r = shell(bg);
    assert_eq!(r.background_id.as_deref(), Some("sh_bg00000001"));
    assert_eq!(
        (r.exit_code, r.timed_out, r.interrupted),
        (Some(0), false, false)
    );
    assert_eq!(r.stdout.as_deref(), Some("build ok"));
    assert_eq!(bg.status_detail.as_deref(), Some("completed"));
    assert_eq!(r.duration_ms, Some(1791850018500 - 1791850011420));
    assert_eq!(bg.status, ToolStatus::Ok);
    // the completion row stays as its own notification message
    let done = message(&s, "msg_1329576c4089JvHdBCssNXOYqk");
    assert_eq!(
        (done.role, done.origin, done.kind.as_deref()),
        (Role::User, Origin::Notification, Some("shell"))
    );
}

#[test]
fn v2_switches_compaction_idle_and_errors() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let get = |id: &str| {
        s.entries
            .iter()
            .find(|e| e.id.as_deref() == Some(id))
            .unwrap()
    };

    let EntryKind::Turn(t) = &get("msg_132957ca008bc6no0pJ0etCoxk").kind else {
        panic!()
    };
    assert_eq!(
        (t.status.as_deref(), t.error.as_deref()),
        (Some("completed"), None)
    );

    let EntryKind::Settings(a) = &get("msg_13295f2fc08dLrb4vSV4MK3x55").kind else {
        panic!()
    };
    assert_eq!(a.agent.as_deref(), Some("plan"));
    assert_eq!(
        get("msg_13295f2fc08dLrb4vSV4MK3x55")
            .extra
            .as_ref()
            .unwrap()["previous"],
        "build"
    );
    let EntryKind::Settings(m) = &get("msg_13295f36008eCMYgzA6XQYcLyb").kind else {
        panic!()
    };
    assert_eq!(m.model.as_deref(), Some("demo-sonnet"));
    assert_eq!(
        get("msg_13295f36008eCMYgzA6XQYcLyb")
            .extra
            .as_ref()
            .unwrap()["previous"]["id"],
        "demo-gpt"
    );

    let compaction = get("msg_13296010c090AcQhkl1JxGuOko");
    let EntryKind::Compaction(c) = &compaction.kind else {
        panic!()
    };
    assert_eq!(c.trigger.as_deref(), Some("auto"));
    assert_eq!(
        c.summary.as_deref(),
        Some(
            "## Goal\nAdd a retry helper to /work/demo.\n\n## Done\n- retry in src/net.ts\n\n## Next\n- README section"
        )
    );
    let u = c.usage.unwrap();
    assert_eq!((u.input, u.output, u.reasoning), (5000, 400, 0));
    near(u.cost_usd, 0.005);
    assert_eq!(
        compaction.extra.as_ref().unwrap()["recent"],
        "user: now plan the docs\nassistant: Plan: update README.md with a retry section."
    );
    assert_eq!(compaction.extra.as_ref().unwrap()["status"], "completed");

    // the failed assistant row: empty message, then an Error entry carrying the retry attempt
    let failed = message(&s, "msg_132961494091oktXzWAHtDI2HY");
    assert!(failed.parts.is_empty() && failed.usage.is_none());
    assert_eq!(failed.stop_reason.as_deref(), Some("error"));
    let row = s
        .entries
        .iter()
        .position(|e| e.id.as_deref() == Some("msg_132961494091oktXzWAHtDI2HY"))
        .unwrap();
    let EntryKind::Error(err) = &s.entries[row + 1].kind else {
        panic!()
    };
    assert_eq!(
        (
            err.kind.as_deref(),
            err.message.as_str(),
            err.status,
            err.retry
        ),
        (
            Some("provider.error"),
            "Service unavailable",
            Some(503),
            Some(2)
        )
    );
    // the retry object repeats the error; only its timestamp is left
    assert_eq!(
        s.entries[row].extra.as_ref().unwrap()["retry"],
        json!({"at": 1791850063900u64})
    );

    let idle = get("msg_13296187c0926bF9hO1IP2Sy1l");
    let EntryKind::Turn(t) = &idle.kind else {
        panic!()
    };
    assert_eq!(
        (t.status.as_deref(), t.error.as_deref()),
        (
            Some("failed"),
            Some("Model demo-sonnet has been deprecated.")
        )
    );
    let err = &idle.extra.as_ref().unwrap()["error"];
    assert_eq!(
        (err["type"].as_str(), err["status"].as_u64()),
        (Some("provider.invalid-request"), Some(410))
    );
    assert!(
        err["response"]["body"]
            .as_str()
            .unwrap()
            .contains("ModelDeprecated")
    );
}

#[test]
fn v2_assistant_rows_point_at_the_triggering_user_row() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE);
    let turn = |id: &str| {
        s.entries
            .iter()
            .find(|e| e.id.as_deref() == Some(id))
            .unwrap()
            .turn_id
            .clone()
    };
    assert_eq!(
        turn("msg_132954014086GxRGZlQM7hWhEa").as_deref(),
        Some("msg_132952e80081LAJnrjto27uBcD")
    );
    assert_eq!(
        turn("msg_1329578b808ar49ahhCfPruU2Q").as_deref(),
        Some("msg_132952e80081LAJnrjto27uBcD")
    );
    assert_eq!(
        turn("msg_13295f93c08fzhxzLKbtBbOHrx").as_deref(),
        Some("msg_13295f1d008cnJ4pQpINkE28Xz")
    );
}

#[test]
fn v2_native_child_links_to_the_subagent_call() {
    let s = Db::from_sql(V2_SQL).parse(NATIVE_CHILD);
    let parent = s.parent.as_ref().unwrap();
    assert_eq!(
        (
            parent.session_id.as_str(),
            parent.call_id.as_deref(),
            parent.agent_type.as_deref()
        ),
        (NATIVE, Some("call_demov207"), Some("general"))
    );
    assert_eq!(kinds(&s), ["user", "assistant", "turn"]);
    assert!(matches!(&calls(&s)[0].tool, Tool::Search { .. }));
    assert_eq!(
        (s.usage.input, s.usage.output, s.usage.cache_read),
        (500, 40, 0)
    );
}

// ---- legacy JSON tree -------------------------------------------------------------------------

#[test]
fn json_tree_session() {
    let path = legacy_session_file();
    let s = agentparse::parse_file(&path).unwrap();
    assert_round_trip(&s);
    assert_eq!(
        (s.agent, s.format.as_str(), s.id.as_str()),
        (Agent::Opencode, "opencode-json", LEGACY)
    );
    assert_eq!(s.source, Source::File { path: path.clone() });
    assert_eq!(s.title.as_deref(), Some("Legacy fix the greeting"));
    assert_eq!(
        (s.cwd.as_deref(), s.cli_version.as_deref()),
        (Some("/work/demo"), Some("0.15.31"))
    );
    assert!(s.parent.is_none() && s.warnings.is_empty());
    assert_eq!(s.models, ["demo-sonnet"]);
    // README: step-finish equals assistant tokens, no `total` key: input 900, output 140, reasoning 30, cache 1000/200
    let u = s.usage;
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read, u.cache_write),
        (900, 140 + 30, 30, 1000, 200)
    );
    near(u.cost_usd, 0.0123);
    assert_eq!(kinds(&s), ["user", "assistant", "other"]);
    assert_eq!(count(&s, Role::User, Origin::Human), 1);

    let bash = call(&s, "toolu_legacy0001");
    assert!(matches!(&bash.tool, Tool::Shell(sh) if sh.command == "cat hello.txt"));
    assert_eq!(
        (shell(bash).exit_code, bash.status),
        (Some(0), ToolStatus::Ok)
    );
    let edit = call(&s, "toolu_legacy0002");
    assert!(matches!(&edit.tool, Tool::Edit { path, .. } if path == "/work/demo/hello.txt"));
    assert_eq!(edit.status, ToolStatus::Ok);
    // parts come in id order: step-start, reasoning, text, tools, step-finish
    let EntryKind::Message(m) = &s.entries[1].kind else {
        panic!()
    };
    let order: Vec<&str> = m
        .parts
        .iter()
        .map(|p| match p {
            Part::Other { kind, .. } => kind.as_str(),
            Part::Reasoning { .. } => "reasoning",
            Part::Text { .. } => "text",
            Part::ToolCall(c) => c.name.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(
        order,
        [
            "step-start",
            "reasoning",
            "text",
            "bash",
            "edit",
            "step-finish"
        ]
    );
    // the todo file and the session diff file ride along
    let Some(Entry {
        kind: EntryKind::Other(todos),
        ..
    }) = s.entries.last()
    else {
        panic!()
    };
    assert_eq!(
        (todos.kind.as_str(), todos.data.as_array().map(Vec::len)),
        ("todos", Some(1))
    );
    assert_eq!(
        s.extra.as_ref().unwrap()["session_diff"][0]["file"],
        "/work/demo/hello.txt"
    );
    assert_eq!(
        s.extra.as_ref().unwrap()["projectID"],
        "e4d909c290d0fb1ca068ffaddf22cbd0cd8aaaaa"
    );
}

#[test]
fn json_tree_rejects_other_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("c.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[1, 2]").unwrap();
    assert!(matches!(
        agentparse::parse_file(&path),
        Err(Error::NotASession(_))
    ));
    assert!(matches!(
        agentparse::parse_file(&dir.path().join("nope.json")),
        Err(Error::Io(_))
    ));
}

#[test]
fn json_tree_broken_message_file_is_a_warning() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let session = root.join("session/p1/ses_x.json");
    std::fs::create_dir_all(session.parent().unwrap()).unwrap();
    std::fs::write(&session, r#"{"id":"ses_x","projectID":"p1","directory":"/w","title":"t","version":"0.1","time":{"created":1,"updated":2}}"#).unwrap();
    let messages = root.join("message/ses_x");
    std::fs::create_dir_all(&messages).unwrap();
    std::fs::write(
        messages.join("msg_1.json"),
        r#"{"id":"msg_1","sessionID":"ses_x","role":"user","time":{"created":5}}"#,
    )
    .unwrap();
    std::fs::write(messages.join("msg_2.json"), "{ not json").unwrap();
    let s = agentparse::parse_file(&session).unwrap();
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.warnings.len(), 1);
    assert!(s.warnings[0].message.contains("msg_2.json"));
    assert_eq!((s.started_at, s.updated_at), (Some(1), Some(5)));
}

// ---- tolerance: old schemas, bad rows, status rules -------------------------------------------

/// A v1 database of an old release: no parent_id, usage, agent or model columns.
fn old_v1_db() -> Db {
    Db::from_sql(
        "CREATE TABLE session (id text PRIMARY KEY, directory text NOT NULL, title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
         CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer, data text);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer, data text);",
    )
}

fn insert_v1(db: &Db, sid: &str, id: &str, created: i64, data: Value, parts: &[(&str, Value)]) {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
        params![id, sid, created, data.to_string()],
    )
    .unwrap();
    for (pid, part) in parts {
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pid, id, sid, created, part.to_string()],
        )
        .unwrap();
    }
}

#[test]
fn old_v1_schema_without_optional_columns() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_old', '/w', 'Old session', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_old",
        "msg_1",
        10,
        json!({"role": "user", "time": {"created": 10}}),
        &[("prt_1", json!({"type": "text", "text": "hello"}))],
    );
    // the last message has a pending tool call: nothing follows, so it is Pending, not Interrupted
    insert_v1(
        &db,
        "ses_old",
        "msg_2",
        11,
        json!({"role": "assistant", "time": {"created": 11}, "parentID": "msg_1", "modelID": "m", "providerID": "p"}),
        &[(
            "prt_2",
            json!({"type": "tool", "tool": "bash", "callID": "c1", "state": {"status": "running", "input": {"command": "sleep 9"}, "time": {"start": 12}}}),
        )],
    );
    let refs = opencode::list(&db.path).unwrap();
    assert_eq!(
        (refs.len(), refs[0].parent_id.clone(), refs[0].updated_at),
        (1, None, Some(20))
    );
    let s = db.parse("ses_old");
    assert_eq!(
        (
            s.format.as_str(),
            s.title.as_deref(),
            s.cli_version.as_deref()
        ),
        ("opencode-v1", Some("Old session"), Some("1.2.0"))
    );
    assert!(s.extra.is_none() && s.usage.cost_usd.is_none());
    let c = call(&s, "c1");
    assert_eq!((c.status, c.started_at), (ToolStatus::Pending, Some(12)));
    assert!(matches!(&c.tool, Tool::Shell(sh) if sh.command == "sleep 9"));
    // todo table is absent: no todos entry
    assert!(
        s.entries
            .iter()
            .all(|e| !matches!(e.kind, EntryKind::Other(_)))
    );
}

#[test]
fn bad_rows_become_warnings_and_do_not_stop_the_parse() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_bad', '/w', 'Bad rows', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_bad",
        "msg_1",
        10,
        json!({"role": "user", "time": {"created": 10}}),
        &[("prt_1", json!({"type": "text", "text": "ok"}))],
    );
    let conn = db.conn();
    conn.execute(
        "INSERT INTO message VALUES ('msg_2', 'ses_bad', 11, '{broken')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO part VALUES ('prt_9', 'msg_1', 'ses_bad', 11, 'nope')",
        [],
    )
    .unwrap();
    let s = db.parse("ses_bad");
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.warnings.len(), 2);
    assert!(
        s.warnings.iter().any(|w| w.message.contains("msg_2"))
            && s.warnings.iter().any(|w| w.message.contains("prt_9"))
    );
}

fn v2_db() -> Db {
    Db::from_sql(
        "CREATE TABLE session_v2 (id text PRIMARY KEY, directory text NOT NULL, version text NOT NULL, title text, time_created integer NOT NULL, time_updated integer NOT NULL);
         CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer, time_updated integer, data text);
         INSERT INTO session_v2 VALUES ('ses_v2', '/w', '2.0.27', NULL, 100, 200);",
    )
}

fn insert_v2(db: &Db, seq: i64, ty: &str, data: Value) {
    db.conn()
        .execute(
            "INSERT INTO session_message VALUES (?1, 'ses_v2', ?2, ?3, ?4, ?4, ?5)",
            params![format!("row_{seq}"), ty, seq, 100 + seq, data.to_string()],
        )
        .unwrap();
}

fn tool_block(id: &str, name: &str, state: Value) -> Value {
    json!({"type": "tool", "id": id, "name": name, "state": state, "time": {"created": 1000, "ran": 1100, "completed": 1350}})
}

#[test]
fn v2_status_rules() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "user",
        json!({"text": "go", "time": {"created": 100}}),
    );
    insert_v2(
        &db,
        1,
        "assistant",
        json!({"agent": "build", "model": {"id": "m", "providerID": "p"}, "time": {"created": 101}, "content": [
            // non-zero exit with the printed trailer; the model-visible text keeps it, stdout does not
            tool_block("exit2", "shell", json!({"status": "completed", "input": {"command": "false"},
                "content": [{"type": "text", "text": "boom\n"}, {"type": "text", "text": "Exited with code 2"}],
                "metadata": {"status": "completed", "exit": 2, "truncated": false}})),
            // exit only from the trailer when metadata has none
            tool_block("trailer", "shell", json!({"status": "completed", "input": {"command": "x"},
                "content": [{"type": "text", "text": "Exited with code 7"}]})),
            tool_block("timeout", "shell", json!({"status": "completed", "input": {"command": "sleep 99", "timeout": 1000},
                "content": [{"type": "text", "text": "partial"}], "metadata": {"timeout": true, "signal": "SIGTERM"}})),
            tool_block("rejected", "edit", json!({"status": "error", "input": {"path": "/a", "oldString": "x", "newString": "y"},
                "error": {"type": "tool.execution", "message": "The user rejected permission to use this specific tool call."}})),
            tool_block("denied", "write", json!({"status": "error", "input": {"path": "/a", "content": "x"},
                "error": {"type": "tool.execution", "message": "The user has specified a rule which prevents you from using this specific tool call."}})),
            tool_block("migrated", "bash", json!({"status": "error", "input": {"command": "x"},
                "error": {"type": "tool.interrupted", "message": "Tool execution was interrupted"}})),
            tool_block("streaming", "shell", json!({"status": "streaming", "input": "{\"command\": \"ls"})),
        ]}),
    );
    insert_v2(
        &db,
        2,
        "assistant",
        json!({"time": {"created": 102}, "content": [tool_block("last", "shell", json!({"status": "running", "input": {"command": "tail -f x"}, "metadata": {}}))]}),
    );
    let s = db.parse("ses_v2");
    let status = |id: &str| call(&s, id).status;
    assert_eq!(status("exit2"), ToolStatus::Error);
    let c = call(&s, "exit2");
    assert_eq!(
        (
            shell(c).exit_code,
            shell(c).stdout.as_deref(),
            shell(c).duration_ms
        ),
        (Some(2), Some("boom\n"), Some(250))
    );
    assert_eq!(text(c), "boom\n\nExited with code 2");
    assert_eq!(
        (shell(call(&s, "trailer")).exit_code, status("trailer")),
        (Some(7), ToolStatus::Error)
    );
    assert_eq!(shell(call(&s, "trailer")).stdout.as_deref(), Some(""));
    let t = call(&s, "timeout");
    assert!(shell(t).timed_out && t.status == ToolStatus::Error);
    assert_eq!(shell(t).signal.as_deref(), Some("SIGTERM"));
    assert_eq!(status("rejected"), ToolStatus::Rejected);
    assert_eq!(status("denied"), ToolStatus::Rejected);
    assert_eq!(status("migrated"), ToolStatus::Interrupted);
    // a streaming call with a later row is cut off; its partial input is a string
    assert_eq!(status("streaming"), ToolStatus::Interrupted);
    assert_eq!(call(&s, "streaming").input, json!("{\"command\": \"ls"));
    assert_eq!(call(&s, "streaming").tool, Tool::Other);
    // the same state in the very last row is still pending
    assert_eq!(status("last"), ToolStatus::Pending);
    // the model was never recorded on the last row, so it adds no model
    assert_eq!(s.models, ["m"]);
    assert!(
        s.title.as_deref() == Some("go"),
        "title falls back to the first human prompt"
    );
}

#[test]
fn v2_shell_rows_timeout_killed_running() {
    let db = v2_db();
    let shell_row = |status: &str, exit: Value| {
        json!({"shellID": format!("sh_{status}"), "command": "sleep 9", "status": status, "exit": exit,
               "output": {"output": "out", "cursor": 3, "size": 3, "truncated": true}, "time": {"created": 100, "completed": 160}})
    };
    insert_v2(&db, 0, "shell", shell_row("timeout", Value::Null));
    insert_v2(&db, 1, "shell", shell_row("killed", Value::Null));
    insert_v2(&db, 2, "shell", shell_row("exited", json!(4)));
    insert_v2(
        &db,
        3,
        "shell",
        json!({"shellID": "sh_running", "command": "tail -f x", "status": "running", "time": {"created": 100}}),
    );
    let s = db.parse("ses_v2");
    let (t, k, e, r) = (
        call(&s, "sh_timeout"),
        call(&s, "sh_killed"),
        call(&s, "sh_exited"),
        call(&s, "sh_running"),
    );
    assert_eq!(
        (t.status, shell(t).timed_out, shell(t).exit_code),
        (ToolStatus::Error, true, None)
    );
    assert_eq!(
        (k.status, shell(k).interrupted),
        (ToolStatus::Interrupted, true)
    );
    assert_eq!(
        (e.status, shell(e).exit_code, shell(e).duration_ms),
        (ToolStatus::Error, Some(4), Some(60))
    );
    assert!(e.output.as_ref().unwrap().truncated);
    // a shell row that is still running and is the last row: pending, running in the background
    assert_eq!(
        (
            r.status,
            shell(r).background_id.as_deref(),
            shell(r).exit_code
        ),
        (ToolStatus::Pending, Some("sh_running"), None)
    );
}

#[test]
fn v2_background_completion_failure_and_unknown_rows() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [tool_block("bg", "shell", json!({"status": "completed", "input": {"command": "make", "background": true},
            "content": [{"type": "text", "text": "Command moved to the background (shell ID: sh_9)"}],
            "metadata": {"status": "running", "shellID": "sh_9", "truncated": false}}))]}),
    );
    insert_v2(
        &db,
        1,
        "synthetic",
        json!({"time": {"created": 5000}, "text": "<shell id=\"sh_9\" state=\"completed\" command=\"make\">\nerror: no rule\n\n</shell>",
               "metadata": {"source": "shell", "shellID": "sh_9", "state": "completed", "exit": 2}}),
    );
    insert_v2(&db, 2, "future-row-type", json!({"hello": "world"}));
    insert_v2(
        &db,
        3,
        "assistant",
        json!({"time": {"created": 6000}, "content": [{"type": "image-gen", "prompt": "x"}]}),
    );
    let s = db.parse("ses_v2");
    let bg = call(&s, "bg");
    assert_eq!(
        (shell(bg).exit_code, shell(bg).stdout.as_deref()),
        (Some(2), Some("error: no rule"))
    );
    assert_eq!(bg.status, ToolStatus::Error);
    assert_eq!(shell(bg).background_id.as_deref(), Some("sh_9"));
    // unknown row and unknown content block types stay in place
    let Some(Entry {
        kind: EntryKind::Other(o),
        id,
        ..
    }) = s
        .entries
        .iter()
        .find(|e| matches!(e.kind, EntryKind::Other(_)))
    else {
        panic!()
    };
    assert_eq!(
        (o.kind.as_str(), id.as_deref(), &o.data),
        (
            "session_message.future-row-type",
            Some("row_2"),
            &json!({"hello": "world"})
        )
    );
    let EntryKind::Message(m) = &s.entries.last().unwrap().kind else {
        panic!()
    };
    assert!(
        matches!(&m.parts[0], Part::Other { kind, data } if kind == "image-gen" && data["prompt"] == "x")
    );
}

#[test]
fn v1_shell_notices_set_timed_out_and_interrupted() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_n', '/w', 'n', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    let bash = |id: &str, output: &str| {
        (
            format!("prt_{id}"),
            json!({"type": "tool", "tool": "bash", "callID": id, "state": {"status": "completed", "input": {"command": "sleep 9"}, "output": output,
                "title": "sleep 9", "metadata": {"exit": null, "truncated": false}, "time": {"start": 1, "end": 2}}}),
        )
    };
    let timed = bash(
        "timed",
        "partial\n\n\n<shell_metadata>\nbash tool terminated command after exceeding timeout 1000 ms\n</shell_metadata>",
    );
    let aborted = bash(
        "aborted",
        "x\n\n<bash_metadata>\nUser aborted the command\n</bash_metadata>",
    );
    let empty = bash("empty", "(no output)");
    let parts: Vec<(&str, Value)> = [&timed, &aborted, &empty]
        .into_iter()
        .map(|(i, v)| (i.as_str(), v.clone()))
        .collect();
    insert_v1(
        &db,
        "ses_n",
        "msg_1",
        11,
        json!({"role": "assistant", "time": {"created": 11}}),
        &parts,
    );
    let s = db.parse("ses_n");
    let t = call(&s, "timed");
    assert_eq!(
        (t.status, shell(t).timed_out, shell(t).exit_code),
        (ToolStatus::Error, true, None)
    );
    assert_eq!(shell(t).stdout.as_deref(), Some("partial\n"));
    assert!(
        text(t).contains("<shell_metadata>"),
        "the model-visible text keeps the notice"
    );
    let a = call(&s, "aborted");
    assert_eq!(
        (a.status, shell(a).interrupted, shell(a).stdout.as_deref()),
        (ToolStatus::Interrupted, true, Some("x"))
    );
    assert_eq!(shell(call(&s, "empty")).stdout.as_deref(), Some(""));
    assert_eq!(call(&s, "empty").status, ToolStatus::Ok);
}

#[test]
fn v1_user_message_parts_split_by_origin() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_u', '/w', 'u', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_u",
        "msg_1",
        11,
        json!({"role": "user", "time": {"created": 11}, "agent": "build", "model": {"providerID": "p", "modelID": "m"}}),
        &[
            ("prt_1", json!({"type": "text", "text": "do it"})),
            (
                "prt_2",
                json!({"type": "text", "text": "Called the Read tool", "synthetic": true}),
            ),
            (
                "prt_3",
                json!({"type": "text", "text": "ui only", "ignored": true}),
            ),
            ("prt_4", json!({"type": "text", "text": "and this"})),
        ],
    );
    insert_v1(
        &db,
        "ses_u",
        "msg_2",
        12,
        json!({"role": "assistant", "time": {"created": 12}}),
        &[(
            "prt_5",
            json!({"type": "text", "text": "hidden note", "synthetic": true, "time": {"start": 1}}),
        )],
    );
    let s = db.parse("ses_u");
    let ids: Vec<_> = s.entries.iter().map(|e| e.id.as_deref().unwrap()).collect();
    assert_eq!(ids, ["msg_1#1", "msg_1#2", "msg_1#3", "msg_1#4", "msg_2"]);
    let got: Vec<_> = msgs(&s)
        .iter()
        .take(4)
        .map(|m| (m.origin, m.kind.as_deref(), m.text()))
        .collect();
    assert_eq!(
        got,
        [
            (Origin::Human, None, "do it".to_string()),
            (
                Origin::Context,
                Some("synthetic"),
                "Called the Read tool".to_string()
            ),
            (Origin::Context, Some("ignored"), "ui only".to_string()),
            (Origin::Human, None, "and this".to_string()),
        ]
    );
    assert!(
        msgs(&s)[0].agent.as_deref() == Some("build") && msgs(&s)[0].model.as_deref() == Some("m")
    );
    // flagged text on an assistant message stays visible as its own part
    let EntryKind::Message(a) = &s.entries[4].kind else {
        panic!()
    };
    assert!(
        matches!(&a.parts[0], Part::Other { kind, data } if kind == "text.synthetic" && data["text"] == "hidden note")
    );
}

#[test]
fn every_fixture_session_round_trips_through_json() {
    for sql in [V1_SQL, V2_SQL] {
        let db = Db::from_sql(sql);
        for r in opencode::list(&db.path).unwrap() {
            let s = db.parse(&r.id);
            assert_eq!(s.id, r.id);
            assert_eq!(
                agentparse::parse(&r).unwrap(),
                s,
                "parse(&SessionRef) matches parse_db"
            );
        }
    }
    assert!(agentparse::parse_file(&legacy_session_file()).is_ok());
}

#[test]
fn rows_still_in_the_wal_are_read_through_sqlite() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opencode.db");
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
        .unwrap();
    writer.execute_batch(V2_SQL).unwrap();
    // the writer stays open, so nothing has been checkpointed into the main file
    let wal = std::fs::metadata(path.with_extension("db-wal"))
        .unwrap()
        .len();
    assert!(wal > 0);
    let s = opencode::parse_db(&path, NATIVE).unwrap();
    assert_eq!(s.entries.len(), 19);
    assert_eq!(opencode::list(&path).unwrap().len(), 4);
    drop(writer);
}

#[test]
fn credential_tables_are_never_read() {
    let db = Db::from_sql(V2_SQL);
    db.conn()
        .execute_batch("CREATE TABLE credential (id text, secret text); INSERT INTO credential VALUES ('c', 'sk-live-should-never-appear');")
        .unwrap();
    for r in opencode::list(&db.path).unwrap() {
        let json = serde_json::to_string(&db.parse(&r.id)).unwrap();
        assert!(!json.contains("sk-live-should-never-appear"));
    }
}

// ---- review fixes: shapes opencode really writes ------------------------------------------------

/// A database with both generations and one session `ses_m` present in both.
fn both_db() -> Db {
    Db::from_sql(
        "CREATE TABLE session (id text PRIMARY KEY, directory text NOT NULL, title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, cost real DEFAULT 0 NOT NULL);
         CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer, data text);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer, data text);
         CREATE TABLE session_v2 (id text PRIMARY KEY, directory text NOT NULL, version text NOT NULL, title text, time_created integer NOT NULL, time_updated integer NOT NULL, cost real DEFAULT 0 NOT NULL);
         CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer, time_updated integer, data text);
         INSERT INTO session VALUES ('ses_m', '/w', 'm', '1.18.35', 10, 20, 0);
         INSERT INTO session_v2 VALUES ('ses_m', '/w', '1.18.35', 'm', 10, 20, 0);",
    )
}

fn insert_row(db: &Db, sid: &str, id: &str, seq: i64, ty: &str, data: Value) {
    db.conn()
        .execute(
            "INSERT INTO session_message VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
            params![id, sid, ty, seq, 100 + seq, data.to_string()],
        )
        .unwrap();
}

#[test]
fn migrated_synthetic_split_is_not_repeated() {
    let db = both_db();
    // v1: one user message with ordinary and synthetic text, one assistant reply
    insert_v1(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaa0001",
        11,
        json!({"role": "user", "time": {"created": 11}}),
        &[
            ("prt_1", json!({"type": "text", "text": "read @README.md"})),
            (
                "prt_2",
                json!({"type": "text", "text": "Called the Read tool", "synthetic": true}),
            ),
        ],
    );
    insert_v1(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaa0002",
        12,
        json!({"role": "assistant", "parentID": "msg_aaaaaaaaaaaa0001", "time": {"created": 12}}),
        &[("prt_3", json!({"type": "text", "text": "done"}))],
    );
    // v2 copies: the user row, the synthetic row the migration split out (new id, same 16
    // character prefix), the assistant row, and the migration notice
    insert_row(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaa0001",
        0,
        "user",
        json!({"text": "read @README.md"}),
    );
    insert_row(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaaZZZZ",
        1,
        "synthetic",
        json!({"text": "Called the Read tool"}),
    );
    insert_row(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaa0002",
        2,
        "assistant",
        json!({"content": []}),
    );
    insert_row(
        &db,
        "ses_m",
        "msg_aaaaaaaaaaaaNOTE",
        3,
        "system",
        json!({"text": "tools changed"}),
    );
    // a synthetic row written after the upgrade has its own prefix and stays
    insert_row(
        &db,
        "ses_m",
        "msg_bbbbbbbbbbbb0001",
        4,
        "synthetic",
        json!({"text": "new reminder"}),
    );
    let s = db.parse("ses_m");
    let texts: Vec<String> = msgs(&s).iter().map(|m| m.text()).collect();
    assert_eq!(
        texts,
        [
            "read @README.md",
            "Called the Read tool",
            "done",
            "tools changed",
            "new reminder"
        ]
    );
    assert_eq!(kinds(&s), ["user", "user", "assistant", "system", "user"]);
}

fn background_db(completion: Value) -> Db {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [tool_block("bg", "shell", json!({"status": "completed",
            "input": {"command": "make", "background": true},
            "content": [{"type": "text", "text": "Command moved to the background (shell ID: sh_9)"}],
            "metadata": {"status": "running", "shellID": "sh_9", "truncated": false}}))]}),
    );
    insert_v2(&db, 1, "synthetic", completion);
    db
}

fn completion(state: &str, text: &str, extra_meta: Value) -> Value {
    let mut meta = json!({"source": "shell", "shellID": "sh_9", "jobID": "sh_9", "state": state, "truncated": false});
    meta.as_object_mut()
        .unwrap()
        .extend(extra_meta.as_object().unwrap().clone());
    json!({"time": {"created": 5000}, "description": "make",
           "text": format!("<shell id=\"sh_9\" state=\"{state}\" command=\"make\">\n{text}\n</shell>"),
           "metadata": meta})
}

#[test]
fn background_completions_cancelled_error_timeout_and_signal() {
    let cases = [
        ("cancelled", "Cancelled", json!({}), ToolStatus::Interrupted),
        ("error", "spawn failed", json!({}), ToolStatus::Error),
        (
            "completed",
            "partial\n\nTimed out before completion",
            json!({"timeout": true}),
            ToolStatus::Error,
        ),
        (
            "completed",
            "x\n\nKilled by SIGKILL",
            json!({"signal": "SIGKILL"}),
            ToolStatus::Error,
        ),
        (
            "completed",
            "boom\n\nExited with code 2",
            json!({"exit": 2}),
            ToolStatus::Error,
        ),
        ("completed", "fine", json!({"exit": 0}), ToolStatus::Ok),
    ];
    for (state, text, meta, want) in cases {
        let s = background_db(completion(state, text, meta)).parse("ses_v2");
        let bg = call(&s, "bg");
        assert_eq!(bg.status, want, "{state} {text}");
        assert_eq!(bg.status_detail.as_deref(), Some(state));
        let r = shell(bg);
        assert_eq!(r.interrupted, state == "cancelled", "{text}");
        assert!(
            !r.stdout.as_deref().unwrap().contains("Exited with code"),
            "{text}"
        );
        assert!(
            !r.stdout.as_deref().unwrap().contains("Killed by"),
            "{text}"
        );
        assert!(
            !r.stdout.as_deref().unwrap().contains("Timed out before"),
            "{text}"
        );
    }
    let s = background_db(completion(
        "completed",
        "boom\n\nExited with code 2",
        json!({"exit": 2}),
    ))
    .parse("ses_v2");
    assert_eq!(
        (
            shell(call(&s, "bg")).exit_code,
            shell(call(&s, "bg")).stdout.as_deref()
        ),
        (Some(2), Some("boom"))
    );
    let s = background_db(completion(
        "completed",
        "partial\n\nTimed out before completion",
        json!({"timeout": true}),
    ))
    .parse("ses_v2");
    assert!(shell(call(&s, "bg")).timed_out);
    let s = background_db(completion(
        "completed",
        "x\n\nKilled by SIGKILL",
        json!({"signal": "SIGKILL"}),
    ))
    .parse("ses_v2");
    assert_eq!(shell(call(&s, "bg")).signal.as_deref(), Some("SIGKILL"));
    let mut truncated = completion("completed", "ok", json!({"exit": 0}));
    truncated["metadata"]["truncated"] = json!(true);
    let s = background_db(truncated).parse("ses_v2");
    assert!(call(&s, "bg").output.as_ref().unwrap().truncated);
}

#[test]
fn background_completion_of_a_command_containing_a_quote_bracket() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [tool_block("bg", "shell", json!({"status": "completed",
            "input": {"command": "echo \"hi\">out.txt", "background": true},
            "content": [{"type": "text", "text": "Command moved to the background (shell ID: sh_9)"}],
            "metadata": {"status": "running", "shellID": "sh_9"}}))]}),
    );
    insert_v2(
        &db,
        1,
        "synthetic",
        json!({"time": {"created": 5000}, "description": "echo \"hi\">out.txt",
               "text": "<shell id=\"sh_9\" state=\"completed\" command=\"echo \"hi\">out.txt\">\n\n</shell>",
               "metadata": {"source": "shell", "shellID": "sh_9", "jobID": "sh_9", "state": "completed", "exit": 0}}),
    );
    let s = db.parse("ses_v2");
    assert_eq!(shell(call(&s, "bg")).stdout.as_deref(), Some(""));
}

#[test]
fn v2_shell_notices_stay_out_of_stdout() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [
            tool_block("fgtimeout", "shell", json!({"status": "completed", "input": {"command": "sleep 99", "timeout": 1000},
                "content": [
                    {"type": "text", "text": "partial\n\nCommand exceeded timeout of 1000 ms. Retry with a larger timeout if the command is expected to take longer."},
                    {"type": "text", "text": "Timed out before completion"}],
                "metadata": {"timeout": true, "truncated": false}})),
            tool_block("killed", "shell", json!({"status": "completed", "input": {"command": "x"},
                "content": [{"type": "text", "text": "out\n"}, {"type": "text", "text": "Killed by SIGKILL"}],
                "metadata": {"signal": "SIGKILL", "truncated": false}})),
            tool_block("unknown", "shell", json!({"status": "completed", "input": {"command": "x"},
                "content": [{"type": "text", "text": "Exited with code unknown"}], "metadata": {}})),
        ]}),
    );
    insert_v2(&db, 1, "idle", json!({"outcome": "succeeded"}));
    let s = db.parse("ses_v2");
    let t = call(&s, "fgtimeout");
    assert_eq!(
        (shell(t).stdout.as_deref(), shell(t).timed_out, t.status),
        (Some("partial"), true, ToolStatus::Error)
    );
    let k = call(&s, "killed");
    assert_eq!(
        (
            shell(k).stdout.as_deref(),
            shell(k).signal.as_deref(),
            k.status
        ),
        (Some("out\n"), Some("SIGKILL"), ToolStatus::Error)
    );
    let u = call(&s, "unknown");
    assert_eq!(
        (shell(u).stdout.as_deref(), shell(u).exit_code, u.status),
        (Some(""), None, ToolStatus::Ok)
    );
}

#[test]
fn v1_tag_handling_is_anchored_to_the_end_of_the_output() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_t', '/w', 't', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    let part = |id: &str, output: &str| {
        (
            format!("prt_{id}"),
            json!({"type": "tool", "tool": "bash", "callID": id, "state": {"status": "completed", "input": {"command": "x"}, "output": output,
                "metadata": {"exit": 0}, "time": {"start": 1, "end": 2}}}),
        )
    };
    let parts = [
        part(
            "midtag",
            "log: <shell_metadata>\nnot a notice\n</shell_metadata> and more\nlast line",
        ),
        part(
            "humanabort",
            "partial\n\n<metadata>\nUser aborted the command\n</metadata>",
        ),
    ];
    let refs: Vec<(&str, Value)> = parts.iter().map(|(i, v)| (i.as_str(), v.clone())).collect();
    insert_v1(
        &db,
        "ses_t",
        "msg_1",
        11,
        json!({"role": "assistant", "time": {"created": 11}}),
        &refs,
    );
    let s = db.parse("ses_t");
    let mid = call(&s, "midtag");
    assert!(shell(mid).stdout.as_deref().unwrap().ends_with("last line"));
    assert_eq!(mid.status, ToolStatus::Ok);
    let abort = call(&s, "humanabort");
    assert_eq!(
        (
            shell(abort).stdout.as_deref(),
            shell(abort).interrupted,
            abort.status
        ),
        (Some("partial"), true, ToolStatus::Interrupted)
    );
}

#[test]
fn location_switched_carries_the_directory() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "location-switched",
        json!({"location": {"directory": "/w/other", "workspaceID": "wrk_1"}, "previous": {"directory": "/w"}, "time": {"created": 100}}),
    );
    let s = db.parse("ses_v2");
    let EntryKind::Settings(settings) = &s.entries[0].kind else {
        panic!()
    };
    assert_eq!(settings.cwd.as_deref(), Some("/w/other"));
    assert_eq!(
        s.entries[0].extra.as_ref().unwrap()["location"]["workspaceID"],
        "wrk_1"
    );
}

#[test]
fn denial_phrases_follow_opencodes_own_classification() {
    let db = v2_db();
    let err = |id: &str, message: &str| {
        tool_block(
            id,
            "question",
            json!({"status": "error", "input": {}, "error": {"type": "tool.execution", "message": message}}),
        )
    };
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [
            err("dismissed", "The user dismissed this question"),
            err("rejq", "QuestionRejectedError: nope"),
            err("plain", "boom"),
        ]}),
    );
    let s = db.parse("ses_v2");
    assert_eq!(call(&s, "dismissed").status, ToolStatus::Rejected);
    assert_eq!(call(&s, "rejq").status, ToolStatus::Rejected);
    assert_eq!(call(&s, "plain").status, ToolStatus::Error);
}

#[test]
fn a_running_user_shell_row_is_pending_even_with_later_rows() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "shell",
        json!({"shellID": "sh_u", "command": "tail -f x", "status": "running", "metadata": {"sessionID": "ses_v2", "background": true}, "time": {"created": 100}}),
    );
    insert_v2(
        &db,
        1,
        "user",
        json!({"text": "meanwhile", "time": {"created": 101}}),
    );
    // A later assistant row must not turn the still-running shell into an interrupted one.
    insert_v2(
        &db,
        2,
        "assistant",
        json!({"agent": "build", "model": {"id": "m", "providerID": "p"}, "content": [{"type": "text", "text": "ok"}], "time": {"created": 102}}),
    );
    let s = db.parse("ses_v2");
    let c = call(&s, "sh_u");
    assert_eq!(c.status, ToolStatus::Pending);
    assert_eq!(shell(c).background_id.as_deref(), Some("sh_u"));
    assert!(matches!(&c.tool, Tool::Shell(sh) if sh.background));
    assert_eq!(
        s.entries[0].extra.as_ref().unwrap()["metadata"]["sessionID"],
        "ses_v2"
    );
}

#[test]
fn recorded_session_cost_wins_unless_it_is_the_zero_default() {
    let db = both_db();
    let conn = db.conn();
    conn.execute_batch(
        "INSERT INTO session_v2 VALUES ('ses_paid', '/w', '2.0.27', 'p', 10, 20, 0.5);
         INSERT INTO session_v2 VALUES ('ses_zero', '/w', '2.0.27', 'z', 10, 20, 0);",
    )
    .unwrap();
    for sid in ["ses_paid", "ses_zero"] {
        insert_row(
            &db,
            sid,
            &format!("{sid}_a"),
            0,
            "assistant",
            json!({"time": {"created": 100}, "content": [{"type": "text", "text": "x"}], "cost": 0.1,
                   "tokens": {"input": 1, "output": 1, "reasoning": 0, "cache": {"read": 0, "write": 0}}}),
        );
    }
    // the column covers calls no row carries (title generation), so it wins when recorded
    let paid = db.parse("ses_paid");
    near(paid.usage.cost_usd, 0.5);
    assert!(paid.extra.as_ref().is_none_or(|e| e.get("cost").is_none()));
    // a column still at its default 0 while the rows carry a cost: the rows are the truth
    near(db.parse("ses_zero").usage.cost_usd, 0.1);
}

#[test]
fn extreme_timestamps_do_not_overflow_durations() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [{"type": "tool", "id": "t", "name": "shell",
            "state": {"status": "completed", "input": {"command": "x"}, "content": [{"type": "text", "text": "ok"}], "metadata": {"exit": 0}},
            "time": {"ran": i64::MIN, "completed": i64::MAX}}]}),
    );
    insert_v2(
        &db,
        1,
        "shell",
        json!({"shellID": "sh_x", "command": "x", "status": "exited", "exit": 0, "output": {"output": "", "cursor": 0, "size": 0, "truncated": false},
               "time": {"created": i64::MIN, "completed": i64::MAX}}),
    );
    let s = db.parse("ses_v2");
    assert_eq!(shell(call(&s, "t")).duration_ms, None);
    assert_eq!(shell(call(&s, "sh_x")).duration_ms, None);
}

#[test]
fn storage_message_and_part_files_are_not_sessions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opencode/storage");
    let message = root.join(format!(
        "message/{LEGACY}/msg_79a8c3e10001LegacyUserMsg001.json"
    ));
    let part =
        root.join("part/msg_79a8c3e10001LegacyUserMsg001/prt_79a8c4010001LegacyPart001.json");
    for path in [message, part] {
        assert!(
            matches!(agentparse::parse_file(&path), Err(Error::NotASession(_))),
            "{path:?}"
        );
        assert!(
            matches!(agentparse::parse_file(&path), Err(Error::NotASession(_))),
            "{path:?}"
        );
    }
    assert!(agentparse::parse_file(&legacy_session_file()).is_ok());
}

#[test]
fn json_tree_session_without_a_message_directory_warns() {
    let dir = TempDir::new().unwrap();
    let session = dir.path().join("session/p1/ses_x.json");
    std::fs::create_dir_all(session.parent().unwrap()).unwrap();
    std::fs::write(&session, r#"{"id":"ses_x","projectID":"p1","directory":"/w","title":"t","version":"0.1","time":{"created":1,"updated":2}}"#).unwrap();
    let s = agentparse::parse_file(&session).unwrap();
    assert!(s.entries.is_empty());
    assert!(
        s.warnings
            .iter()
            .any(|w| w.message.contains("no message directory"))
    );
}

#[test]
fn compaction_keeps_only_the_first_summary_and_reports_its_error() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_c', '/w', 'c', '1.18.35', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_c",
        "msg_u",
        11,
        json!({"role": "user", "time": {"created": 11}}),
        &[("prt_1", json!({"type": "compaction", "auto": false}))],
    );
    let failed = json!({"role": "assistant", "parentID": "msg_u", "summary": true, "mode": "compaction", "agent": "compaction",
        "time": {"created": 12}, "error": {"name": "APIError", "data": {"message": "Service unavailable", "statusCode": 503}}});
    insert_v1(&db, "ses_c", "msg_s1", 12, failed, &[]);
    insert_v1(
        &db,
        "ses_c",
        "msg_s2",
        13,
        json!({"role": "assistant", "parentID": "msg_u", "summary": true, "time": {"created": 13}, "finish": "stop"}),
        &[(
            "prt_2",
            json!({"type": "text", "text": "second try summary"}),
        )],
    );
    let s = db.parse("ses_c");
    assert_eq!(kinds(&s), ["compaction", "error", "assistant"]);
    let EntryKind::Compaction(c) = &s.entries[0].kind else {
        panic!()
    };
    assert_eq!(
        (c.trigger.as_deref(), c.summary.as_deref()),
        (Some("manual"), Some(""))
    );
    let EntryKind::Error(e) = &s.entries[1].kind else {
        panic!()
    };
    assert_eq!(
        (e.status, e.message.as_str()),
        (Some(503), "Service unavailable")
    );
    assert_eq!(message(&s, "msg_s2").text(), "second try summary");
}

#[test]
fn v2_user_file_description_and_mention_are_kept() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "user",
        json!({"text": "see @spec", "time": {"created": 100}, "files": [
            {"mime": "text/plain", "source": {"type": "uri", "uri": "file:///w/spec.md"}, "name": "spec.md",
             "description": "the spec", "mention": {"start": 4, "end": 9, "text": "@spec"}},
            {"data": "aGVsbG8=", "mime": "image/png", "source": {"type": "inline"}}]}),
    );
    let s = db.parse("ses_v2");
    let extra = s.entries[0].extra.as_ref().unwrap();
    assert_eq!(extra["files"][0]["description"], "the spec");
    assert_eq!(extra["files"][0]["mention"]["text"], "@spec");
    assert!(extra["files"][1].is_null());
    let EntryKind::Message(m) = &s.entries[0].kind else {
        panic!()
    };
    assert!(
        matches!(&m.parts[1], Part::Attachment(a) if a.url.as_deref() == Some("file:///w/spec.md"))
    );
    assert!(matches!(&m.parts[2], Part::Attachment(a) if a.bytes == Some(5)));
}

#[test]
fn tool_input_blobs_are_sized_before_the_typed_view() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_b', '/w', 'b', '1.2.0', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_b",
        "msg_1",
        11,
        json!({"role": "assistant", "time": {"created": 11}}),
        &[(
            "prt_1",
            json!({"type": "tool", "tool": "write", "callID": "c", "state": {"status": "completed",
            "input": {"filePath": "/w/a.png", "content": "data:image/png;base64,aGVsbG8h"}, "output": "ok", "time": {"start": 1, "end": 2}}}),
        )],
    );
    let s = db.parse("ses_b");
    let c = call(&s, "c");
    assert_eq!(c.input["content"], json!({"bytes": 6}));
    assert!(!serde_json::to_string(&s).unwrap().contains("aGVsbG8h"));
}

#[test]
fn text_and_reasoning_leftovers_are_kept_by_part_position() {
    let s = Db::from_sql(V1_SQL).parse(MAIN);
    // message #9: step-start (0), reasoning (1), text (2), webfetch (3), websearch (4), step-finish (5)
    let entry = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_129a59580028l3jelVCNspMz0Z"))
        .unwrap();
    let parts = &entry.extra.as_ref().unwrap()["parts"];
    assert_eq!(parts["1"]["time"]["start"], 1791700080200u64);
    assert_eq!(parts["2"]["time"]["end"], 1791700081000u64);
    // provider item ids, the phase (Message.phase) and encrypted blobs are bookkeeping, not kept
    let json = serde_json::to_string(&s).unwrap();
    assert!(!json.contains("rs_demo0001") && !json.contains("msg_demo0001"));
    // v2 blocks work the same way
    let v2 = Db::from_sql(V2_SQL).parse(NATIVE);
    let row = v2
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_13295f93c08fzhxzLKbtBbOHrx"))
        .unwrap();
    assert_eq!(
        row.extra.as_ref().unwrap()["parts"]["0"]["time"]["created"],
        1791850051950u64
    );
}

#[test]
fn subtask_parts_stay_in_place() {
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_st', '/w', 's', '1.18.35', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_st",
        "msg_1",
        11,
        json!({"role": "user", "time": {"created": 11}}),
        &[(
            "prt_1",
            json!({"type": "subtask", "prompt": "dig", "description": "dig deep", "agent": "general"}),
        )],
    );
    let s = db.parse("ses_st");
    let EntryKind::Message(m) = &s.entries[0].kind else {
        panic!()
    };
    assert!(
        matches!(&m.parts[0], Part::Other { kind, data } if kind == "subtask" && data["agent"] == "general")
    );
}

#[test]
fn cross_review_origin_turn_cwd_and_trigger() {
    let v2 = Db::from_sql(V2_SQL);
    // a child's first prompt is Origin::Agent (v1 and v2 children)
    for id in [NATIVE_CHILD, MIGRATED_CHILD] {
        let s = v2.parse(id);
        let first = msgs(&s).into_iter().find(|m| m.role == Role::User).unwrap();
        assert_eq!(first.origin, Origin::Agent, "{id}");
    }
    // a prompt starts its turn: turn_id is its own id (v1 and v2); split groups share the base id
    let s = v2.parse(NATIVE);
    let user = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_132952e80081LAJnrjto27uBcD"))
        .unwrap();
    assert_eq!(
        user.turn_id.as_deref(),
        Some("msg_132952e80081LAJnrjto27uBcD")
    );
    let v1 = Db::from_sql(V1_SQL).parse(MAIN);
    let user = v1
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_129a45d00002DJC3QKUfRB41eK"))
        .unwrap();
    assert_eq!(
        user.turn_id.as_deref(),
        Some("msg_129a45d00002DJC3QKUfRB41eK")
    );
    // a v2 shell call that names no workdir ran in the session directory
    let bg = call(&s, "call_demov205");
    assert!(matches!(&bg.tool, Tool::Shell(sh) if sh.cwd.as_deref() == Some("/work/demo")));
    let human = call(&s, "sh_user000001");
    assert!(matches!(&human.tool, Tool::Shell(sh) if sh.cwd.as_deref() == Some("/work/demo")));
    // a compaction without a recorded `auto` flag has no trigger
    let db = old_v1_db();
    db.conn()
        .execute(
            "INSERT INTO session VALUES ('ses_t', '/w', 't', '1.18.35', 10, 20)",
            [],
        )
        .unwrap();
    insert_v1(
        &db,
        "ses_t",
        "msg_u",
        11,
        json!({"role": "user", "time": {"created": 11}}),
        &[("prt_1", json!({"type": "compaction"}))],
    );
    let s = db.parse("ses_t");
    let EntryKind::Compaction(c) = &s.entries[0].kind else {
        panic!()
    };
    assert_eq!(c.trigger, None);
}

#[test]
fn a_call_without_a_result_has_no_output_and_keeps_its_state_keys() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [
            tool_block("run", "shell", json!({"status": "running", "input": {"command": "tail -f x"}, "metadata": {"preview": "so far"}}))]}),
    );
    insert_v2(
        &db,
        1,
        "assistant",
        json!({"time": {"created": 101}, "content": [{"type": "text", "text": "next"}]}),
    );
    let s = db.parse("ses_v2");
    let c = call(&s, "run");
    // Session::finish decides Interrupted; the parser only reports what was recorded
    assert_eq!(c.status, ToolStatus::Interrupted);
    assert!(c.output.is_none());
    assert_eq!(
        s.entries[0].extra.as_ref().unwrap()["parts"]["0"]["metadata"]["preview"],
        "so far"
    );
}

#[test]
fn json_tree_part_ids_do_not_leak_into_extra_or_data() {
    let s = agentparse::parse_file(&legacy_session_file()).unwrap();
    let json = serde_json::to_string(&s.entries).unwrap();
    assert!(
        !json.contains("\"messageID\"") && !json.contains("\"sessionID\""),
        "{json}"
    );
}

#[test]
fn v2_write_tells_a_created_file_from_an_overwrite() {
    let db = v2_db();
    let write = |id: &str, result: &str| {
        tool_block(
            id,
            "write",
            json!({"status": "completed", "input": {"path": "/w/a.md", "content": "x"},
                "content": [{"type": "text", "text": result}], "metadata": {"truncated": false}}),
        )
    };
    insert_v2(
        &db,
        1,
        "assistant",
        json!({"time": {"created": 101}, "content": [
            write("new", "Created file successfully: a.md"),
            write("old", "Wrote file successfully: a.md"),
        ]}),
    );
    let s = db.parse("ses_v2");
    let op = |id: &str| call(&s, id).output.as_ref().unwrap().files[0].op;
    assert_eq!(op("new"), FileOp::Add);
    assert_eq!(op("old"), FileOp::Update);
}

// Chat parity (running work, queued input, goal and execute tools).

fn bg_block() -> Value {
    tool_block(
        "bg",
        "shell",
        json!({"status": "completed", "input": {"command": "make", "background": true},
            "content": [{"type": "text", "text": "Command moved to the background (shell ID: sh_9)"}],
            "metadata": {"status": "running", "shellID": "sh_9", "truncated": false}}),
    )
}

/// The call that started a background job only started it: it runs until the completion row.
#[test]
fn a_background_shell_is_pending_until_its_completion_row() {
    let db = v2_db();
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [bg_block()]}),
    );
    let s = db.parse("ses_v2");
    let bg = call(&s, "bg");
    assert_eq!(
        (
            bg.status,
            bg.ended_at,
            shell(bg).duration_ms,
            shell(bg).exit_code
        ),
        (ToolStatus::Pending, None, None, None)
    );
    assert_eq!(shell(bg).background_id.as_deref(), Some("sh_9"));
    // The completion row ends it, at its own time.
    let db = background_db(completion("completed", "done", json!({"exit": 0})));
    let bg = call(&db.parse("ses_v2"), "bg").clone();
    assert_eq!((bg.status, bg.ended_at), (ToolStatus::Ok, Some(5000)));
}

fn v2_db_with_children() -> Db {
    Db::from_sql(
        "CREATE TABLE session_v2 (id text PRIMARY KEY, parent_id text, directory text NOT NULL, version text NOT NULL, title text, time_created integer NOT NULL, time_updated integer NOT NULL);
         CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer, time_updated integer, data text);
         CREATE TABLE session_pending (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, data text NOT NULL, delivery text, admitted_seq integer NOT NULL, time_created integer NOT NULL);
         INSERT INTO session_v2 VALUES ('ses_v2', NULL, '/w', '2.0.32', NULL, 100, 200);
         INSERT INTO session_v2 VALUES ('ses_kid_a', 'ses_v2', '/w', '2.0.32', 'scan repo', 110, 200);
         INSERT INTO session_v2 VALUES ('ses_kid_b', 'ses_v2', '/w', '2.0.32', 'scan repo', 120, 200);
         INSERT INTO session_v2 VALUES ('ses_kid_c', 'ses_v2', '/w', '2.0.32', 'other', 130, 200);",
    )
}

/// A running subagent call has no `sessionID` yet: its child is the parent's session titled with
/// its description, and a child another call already names is not taken twice.
#[test]
fn a_running_subagent_is_linked_to_its_child_by_the_parent_id() {
    let db = v2_db_with_children();
    let running = |id: &str, description: &str| {
        tool_block(
            id,
            "subagent",
            json!({"status": "running", "input": {"agent": "general", "description": description, "prompt": "p"}}),
        )
    };
    let done = tool_block(
        "done",
        "subagent",
        json!({"status": "completed", "input": {"agent": "general", "description": "scan repo", "prompt": "p"},
            "content": [{"type": "text", "text": "ok"}], "metadata": {"sessionID": "ses_kid_a", "status": "completed"}}),
    );
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [done, running("r1", "scan repo"), running("r2", "scan repo"), running("r3", "nothing like it")]}),
    );
    let s = db.parse("ses_v2");
    let got: Vec<_> = ["done", "r1", "r2", "r3"]
        .iter()
        .map(|id| (call(&s, id).status, call(&s, id).child_session.clone()))
        .collect();
    use ToolStatus::*;
    assert_eq!(
        got,
        [
            (Ok, Some("ses_kid_a".into())),
            (Pending, Some("ses_kid_b".into())),
            (Pending, None),
            (Pending, None)
        ]
    );
}

/// Input typed while the session was busy waits in `session_pending`.
#[test]
fn pending_input_is_a_queued_human_message() {
    let db = v2_db_with_children();
    insert_v2(
        &db,
        0,
        "user",
        json!({"time": {"created": 100}, "text": "first"}),
    );
    let conn = db.conn();
    let add = |id: &str, ty: &str, seq: i64, data: Value| {
        conn.execute(
            "INSERT INTO session_pending VALUES (?1, 'ses_v2', ?2, ?3, 'deferred', ?4, 300)",
            params![id, ty, data.to_string(), seq],
        )
        .unwrap();
    };
    add(
        "pend_1",
        "user",
        1,
        json!({"text": "and also this", "time": {"created": 300}}),
    );
    add("pend_2", "compaction", 2, json!({"reason": "manual"}));
    let s = db.parse("ses_v2");
    let queued: Vec<_> = s
        .messages()
        .filter(|m| m.phase.as_deref() == Some("queued"))
        .collect();
    let [m] = queued.as_slice() else {
        panic!("{queued:?}")
    };
    assert_eq!(
        (m.origin, m.text().as_str()),
        (Origin::Human, "and also this")
    );
    let e = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("pend_1"))
        .unwrap();
    assert_eq!(
        (
            e.turn_id.as_deref(),
            e.extra.as_ref().unwrap()["delivery"].as_str()
        ),
        (None, Some("deferred"))
    );
    assert!(
        s.entries.iter().any(
            |e| matches!(&e.kind, EntryKind::Other(o) if o.kind == "session_pending.compaction")
        )
    );
    assert!(
        s.messages()
            .filter(|m| m.is_human())
            .all(|m| m.text() != "and also this" || m.phase.is_some())
    );
}

/// Goal plans, `execute` and `question`: typed where the input says enough, input kept.
#[test]
fn goal_execute_and_question_tools_are_typed() {
    let db = v2_db();
    let block = |id: &str, name: &str, input: Value| {
        tool_block(
            id,
            name,
            json!({"status": "completed", "input": input, "content": [{"type": "text", "text": "ok"}]}),
        )
    };
    let plan = json!({"goal_id": "g1", "plan": {"phases": [
        {"id": "p1", "objective": "Greet", "status": "completed"},
        {"id": "p2", "objective": "Scope", "status": "blocked"}]}});
    insert_v2(
        &db,
        0,
        "assistant",
        json!({"time": {"created": 100}, "content": [
            block("ex", "execute", json!({"code": "return 1"})),
            block("gp", "update_goal_plan", plan),
            block("sg", "set_goal", json!({"objective": "ship it"})),
            block("q", "question", json!({"questions": [{"question": "Which target?", "options": []}]})),
        ]}),
    );
    let s = db.parse("ses_v2");
    assert_eq!(
        call(&s, "ex").tool,
        Tool::Code {
            source: "return 1".into()
        }
    );
    let Tool::Todo { items } = &call(&s, "gp").tool else {
        panic!()
    };
    let got: Vec<_> = items
        .iter()
        .map(|i| (i.content.as_str(), i.status.as_deref()))
        .collect();
    assert_eq!(
        got,
        [("Greet", Some("completed")), ("Scope", Some("blocked"))]
    );
    assert_eq!(call(&s, "sg").tool, Tool::Other);
    assert_eq!(call(&s, "sg").input["objective"], "ship it");
    assert!(matches!(call(&s, "q").tool, Tool::Question { .. }));
    assert_eq!(
        call(&s, "q").input["questions"][0]["question"],
        "Which target?"
    );
}
