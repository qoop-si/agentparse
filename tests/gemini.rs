//! Gemini CLI parser, through the public API only.

use agentparse::{
    Agent, EntryKind, Error, FileOp, Origin, Part, Role, Session, Source, Tool, ToolCall,
    ToolStatus,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gemini")
        .join(name)
}

fn parse(name: &str) -> Session {
    agentparse::parse_file(&fixture(name)).unwrap()
}

fn parse_text(text: &str) -> Session {
    try_parse_text(text).unwrap()
}

fn try_parse_text(text: &str) -> agentparse::Result<Session> {
    let source = Source::File {
        path: "inline.jsonl".into(),
    };
    agentparse::parse_reader(Agent::Gemini, text.as_bytes(), source)
}

fn jsonl(records: &[Value]) -> Session {
    let lines: Vec<String> = records.iter().map(Value::to_string).collect();
    parse_text(&lines.join("\n"))
}

fn header() -> Value {
    json!({"sessionId": "s1", "projectHash": "h", "startTime": "2026-01-01T00:00:00.000Z",
           "lastUpdated": "2026-01-01T00:00:00.000Z", "kind": "main"})
}

fn user(id: &str, text: &str) -> Value {
    json!({"id": id, "timestamp": "2026-01-01T00:00:01.000Z", "type": "user", "content": [{"text": text}]})
}

fn gem(id: &str, content: Value, calls: Vec<Value>) -> Value {
    json!({"id": id, "timestamp": "2026-01-01T00:00:02.000Z", "type": "gemini", "content": content, "toolCalls": calls})
}

fn call(id: &str, name: &str, args: Value, status: &str, response: Value) -> Value {
    json!({"id": id, "name": name, "args": args, "status": status, "timestamp": "2026-01-01T00:00:03.000Z",
           "result": [{"functionResponse": {"id": id, "name": name, "response": response}}]})
}

/// One gemini message holding one shell call with the given result text and status.
fn shell(output: &str, status: &str) -> ToolCall {
    let c = call(
        "c1",
        "run_shell_command",
        json!({"command": "x"}),
        status,
        json!({"output": output}),
    );
    only_call(&jsonl(&[header(), gem("g1", json!(""), vec![c])]))
}

fn only_call(s: &Session) -> ToolCall {
    let calls = s.tool_calls();
    assert_eq!(calls.len(), 1);
    calls[0].clone()
}

fn ids(s: &Session) -> Vec<&str> {
    s.entries.iter().filter_map(|e| e.id.as_deref()).collect()
}

fn message(s: &Session, id: &str) -> agentparse::Message {
    s.entries
        .iter()
        .find_map(|e| match &e.kind {
            EntryKind::Message(m) if e.id.as_deref() == Some(id) => Some(m.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no message {id}"))
}

fn entry<'a>(s: &'a Session, id: &str) -> &'a agentparse::Entry {
    s.entries
        .iter()
        .find(|e| e.id.as_deref() == Some(id))
        .unwrap()
}

fn tool_calls_of(s: &Session, id: &str) -> Vec<ToolCall> {
    message(s, id)
        .parts
        .into_iter()
        .filter_map(|p| match p {
            Part::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn round_trips(s: &Session) {
    let back: Session = serde_json::from_value(serde_json::to_value(s).unwrap()).unwrap();
    assert_eq!(&back, s);
}

// ---------------------------------------------------------------------------------------------
// G1: one JSON document

#[test]
fn g1_document() {
    let s = parse("gemini-old-session.json");
    assert_eq!((s.agent, s.format.as_str()), (Agent::Gemini, "gemini-g1"));
    assert_eq!(s.id, "11111111-2222-4333-8444-555555555555");
    assert_eq!(s.title.as_deref(), Some("Fix failing test in parser"));
    assert_eq!(s.models, ["gemini-2.5-pro"]);
    assert_eq!(
        (s.started_at, s.updated_at),
        (Some(1_772_359_200_000), Some(1_772_359_350_000))
    );
    assert_eq!(s.cwd, None);
    assert_eq!(
        s.extra.as_ref().unwrap()["projectHash"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    assert!(s.warnings.is_empty() && s.parent.is_none());
    assert_eq!(
        ids(&s),
        [
            "m-user-1", "m-gem-1", "m-gem-2", "m-gem-3", "m-gem-4", "m-info-1", "m-err-1"
        ]
    );
    assert!(s.entries.iter().all(|e| e.line.is_none()));
    round_trips(&s);
}

#[test]
fn g1_messages_and_usage() {
    let s = parse("gemini-old-session.json");
    let human = message(&s, "m-user-1");
    assert!(
        human.is_human() && human.text() == "Run the tests and fix the failure in src/parser.ts"
    );

    let first = message(&s, "m-gem-1");
    assert_eq!(
        (first.role, first.origin, first.model.as_deref()),
        (Role::Assistant, Origin::Model, Some("gemini-2.5-pro"))
    );
    assert!(
        matches!(&first.parts[0], Part::Reasoning { text: Some(t), encrypted: false }
        if t == "**Planning** Run the tests to see which one fails.")
    );
    assert!(
        matches!(&first.parts[1], Part::Text { text } if text == "I'll run the test suite first.")
    );

    // input = input - cached + tool, output = output + thoughts.
    let u = first.usage.unwrap();
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read, u.cache_write),
        (400, 65, 25, 800, 0)
    );
    assert_eq!(u.total(), 1265);
    assert_eq!(message(&s, "m-gem-4").usage.unwrap().total(), 1808);
    assert_eq!(s.usage.total(), 1265 + 1560 + 1715 + 1808);
    assert_eq!(
        (
            s.usage.input,
            s.usage.output,
            s.usage.reasoning,
            s.usage.cache_read
        ),
        (1000, 148, 25, 5200)
    );
    assert!(s.usage.cost_usd.is_none());

    let EntryKind::Notice(n) = &entry(&s, "m-info-1").kind else {
        panic!()
    };
    assert_eq!(
        (n.level.as_deref(), n.text.as_str()),
        (Some("info"), "Example info notice.")
    );
    let EntryKind::Error(e) = &entry(&s, "m-err-1").kind else {
        panic!()
    };
    assert_eq!(
        (e.kind.as_deref(), e.message.as_str()),
        (Some("api_error"), "[API Error: example error text]")
    );
}

#[test]
fn g1_tool_calls() {
    let s = parse("gemini-old-session.json");
    let calls = s.tool_calls();
    assert_eq!(calls.len(), 4);

    let failing = &calls[0];
    let Tool::Shell(sh) = &failing.tool else {
        panic!("{:?}", failing.tool)
    };
    assert_eq!(sh.command, "bun test");
    assert_eq!(sh.cwd.as_deref(), Some("/home/user/proj"));
    assert_eq!(sh.description.as_deref(), Some("Run the test suite"));
    assert_eq!(
        (failing.status, failing.status_detail.as_deref()),
        (ToolStatus::Error, Some("success"))
    );
    assert_eq!(failing.ended_at, Some(1_772_359_209_000));
    let out = failing.output.as_ref().unwrap();
    let r = out.shell.as_ref().unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.stderr.as_deref()),
        (Some(1), Some("3 pass\n1 fail"), None)
    );
    assert!(out.text.starts_with("Output: 3 pass"));
    assert_eq!(out.data.as_ref().unwrap()["process_group"], 4242);
    assert_eq!(
        out.data.as_ref().unwrap()["resultDisplay"],
        "3 pass\n1 fail"
    );

    let edit = &calls[1];
    let Tool::Edit { path, edits } = &edit.tool else {
        panic!()
    };
    assert_eq!(path, "/home/user/proj/src/parser.ts");
    assert_eq!(
        (
            edits[0].old.as_str(),
            edits[0].new.as_str(),
            edits[0].replace_all
        ),
        ("i <= n", "i < n", false)
    );
    assert_eq!(edit.status, ToolStatus::Ok);
    let files = &edit.output.as_ref().unwrap().files;
    assert_eq!(files.len(), 1);
    assert_eq!(
        (
            files[0].path.as_str(),
            files[0].op,
            files[0].additions,
            files[0].deletions
        ),
        (
            "/home/user/proj/src/parser.ts",
            FileOp::Update,
            Some(1),
            Some(1)
        )
    );
    assert!(
        files[0]
            .diff
            .as_deref()
            .unwrap()
            .contains("@@ -1,3 +1,3 @@")
    );
    // The diff lives in `files`, not again in `data`.
    let display = &edit.output.as_ref().unwrap().data.as_ref().unwrap()["resultDisplay"];
    assert!(display.get("fileDiff").is_none() && display["diffStat"]["model_added_lines"] == 1);

    let missing = &calls[2];
    assert!(
        matches!(&missing.tool, Tool::Read { path, offset: None, limit: None } if path.ends_with("missing.ts"))
    );
    assert_eq!(
        (missing.status, missing.status_detail.as_deref()),
        (ToolStatus::Error, Some("error"))
    );
    assert_eq!(
        missing.output.as_ref().unwrap().text,
        "File not found: /home/user/proj/src/missing.ts"
    );
    assert!(missing.failed());

    // Modern success without an `Exit Code` line exited 0.
    let passing = &calls[3];
    let r = passing.output.as_ref().unwrap().shell.as_ref().unwrap();
    assert_eq!(
        (passing.status, r.exit_code, r.stdout.as_deref()),
        (ToolStatus::Ok, Some(0), Some("4 pass"))
    );
}

// ---------------------------------------------------------------------------------------------
// G2: snapshot JSONL

#[test]
fn g2_snapshot() {
    let s = parse("gemini-snapshot-session.jsonl");
    assert_eq!(s.format, "gemini-g2");
    assert_eq!(s.id, "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff");
    assert_eq!(s.title.as_deref(), Some("List the files"));
    assert_eq!(s.cwd.as_deref(), Some("/home/user/proj"));
    assert_eq!(s.models, ["gemini-2.5-flash"]);
    // Header lastUpdated lags; the last `$set.messages` carries 21:19:05.200.
    assert_eq!(
        (s.started_at, s.updated_at),
        (Some(1_791_667_126_901), Some(1_791_667_145_200))
    );
    // The echo turn u-tr-1 folded into the call; nothing was dropped.
    assert_eq!(ids(&s), ["ctx00000000000000000000000000001", "u-1", "g-1"]);
    assert_eq!(
        s.entries.iter().map(|e| e.line).collect::<Vec<_>>(),
        [Some(2), Some(3), Some(5)]
    );
    assert!(s.warnings.is_empty());

    let ctx = message(&s, "ctx00000000000000000000000000001");
    assert_eq!(
        (ctx.role, ctx.origin, ctx.kind.as_deref()),
        (Role::User, Origin::Context, Some("session_context"))
    );
    assert!(message(&s, "u-1").is_human());

    let g = message(&s, "g-1");
    assert_eq!(g.parts.len(), 2);
    assert!(matches!(&g.parts[0], Part::Text { text } if text == "Listing now."));
    let Part::ToolCall(c) = &g.parts[1] else {
        panic!()
    };
    assert_eq!(c.id, "list_directory-1791667144000-123abc");
    assert!(matches!(&c.tool, Tool::List { path } if path.as_deref() == Some("/home/user/proj")));
    assert_eq!(c.status, ToolStatus::Ok);
    assert_eq!(
        c.output.as_ref().unwrap().text,
        "Directory listing for /home/user/proj:\n[DIR] src\nREADME.md"
    );
    assert_eq!(
        c.output.as_ref().unwrap().data.as_ref().unwrap()["resultDisplay"],
        "Listed 2 item(s)."
    );
    assert_eq!(c.ended_at, Some(1_791_667_145_000));
    assert_eq!(g.usage.unwrap().total(), 910);
    assert_eq!(s.usage.total(), 910);
    round_trips(&s);
}

// ---------------------------------------------------------------------------------------------
// G3: append-only patches and rewinds

#[test]
fn g3_full_view_and_rewind() {
    let s = parse("gemini-new-session.jsonl");
    assert_eq!(s.format, "gemini-g3");
    assert_eq!(s.title.as_deref(), Some("Fix failing parser test"));
    assert_eq!(s.cwd.as_deref(), Some("/home/user/proj"));
    assert_eq!(s.updated_at, Some(1_791_795_664_000));
    let extra = s.extra.as_ref().unwrap();
    assert_eq!(extra["kind"], "main");
    assert!(extra.get("summary").is_none() && extra.get("sessionId").is_none());

    // Every id ever seen, first-seen order; echo turns folded; the rewind where it happened.
    let expected = [
        "ctx00000000000000000000000000001",
        "u-1",
        "g-1",
        "g-2",
        "g-3",
        "e-1",
        "u-2",
        "g-4",
    ];
    assert_eq!(ids(&s), expected);
    assert_eq!(s.entries.len(), expected.len() + 1);
    let EntryKind::Rewind(r) = &s.entries.last().unwrap().kind else {
        panic!()
    };
    assert_eq!(r.dropped, ["u-2", "g-4"]);
    assert_eq!(s.entries.last().unwrap().line, Some(17));
    // Dropped messages are still there, with their text and usage.
    assert_eq!(message(&s, "u-2").text(), "Wrong prompt, ignore");
    assert_eq!(message(&s, "g-4").usage.unwrap().total(), 2603);
    round_trips(&s);
}

#[test]
fn g3_messages_calls_usage() {
    let s = parse("gemini-new-session.jsonl");
    // g-1 was appended twice and patched: first position and first line kept, last content wins.
    assert_eq!(entry(&s, "g-1").line, Some(5));
    let g1 = message(&s, "g-1");
    assert!(
        matches!(&g1.parts[0], Part::Reasoning { text: Some(t), encrypted: true } if t == "Planning: Run tests first.")
    );
    assert!(matches!(&g1.parts[1], Part::Text { text } if text == "I'll run the tests."));
    assert_eq!(g1.parts.len(), 3);

    // The result was masked by a later patch; the first line that had it is used.
    let Part::ToolCall(shell) = &g1.parts[2] else {
        panic!()
    };
    let out = shell.output.as_ref().unwrap();
    assert!(!out.text.contains("masked") && !out.truncated);
    let r = out.shell.as_ref().unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.stderr.as_deref()),
        (Some(1), Some("3 pass\n1 fail"), None)
    );
    assert_eq!(shell.status, ToolStatus::Error);
    assert_eq!(out.data.as_ref().unwrap()["process_group"], 5150);

    let g2 = tool_calls_of(&s, "g-2");
    assert_eq!(g2.len(), 2);
    assert!(matches!(&g2[0].tool, Tool::Edit { edits, .. } if !edits[0].replace_all));
    assert_eq!(g2[0].output.as_ref().unwrap().files.len(), 1);
    assert_eq!(g2[1].status, ToolStatus::Error);

    let g3 = tool_calls_of(&s, "g-3");
    assert_eq!(g3[0].status, ToolStatus::Interrupted);
    assert_eq!(
        g3[0].status_detail.as_deref(),
        Some("cancelled: User cancelled tool execution.")
    );
    let r = g3[0].output.as_ref().unwrap().shell.as_ref().unwrap();
    assert!(r.interrupted && r.stdout.is_none() && r.exit_code.is_none());
    assert!(message(&s, "g-3").usage.is_none());

    let u = message(&s, "g-1").usage.unwrap();
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read),
        (500, 50, 20, 1500)
    );
    assert_eq!(s.usage.total(), 2050 + 2570 + 2603);
    assert_eq!(s.models, ["gemini-3-pro-preview"]);

    let EntryKind::Error(e) = &entry(&s, "e-1").kind else {
        panic!()
    };
    assert_eq!(e.message, "[API Error: example error text]");
}

// ---------------------------------------------------------------------------------------------
// Real local sessions under ~/.gemini/tmp, opt in: `cargo test -- --ignored`

#[test]
#[ignore = "reads the Gemini CLI sessions in your home directory"]
fn real_local_sessions_parse_and_classify_context() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let tmp = Path::new(&home).join(".gemini/tmp");
    let Ok(projects) = std::fs::read_dir(tmp) else {
        return;
    };
    for chats in projects.flatten().map(|p| p.path().join("chats")) {
        for file in std::fs::read_dir(chats)
            .into_iter()
            .flatten()
            .flatten()
            .map(|f| f.path())
        {
            if file.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let s = agentparse::parse_file(&file).unwrap();
            assert!(s.format.starts_with("gemini-g"), "{}", s.format);
            assert!(s.warnings.is_empty(), "{:?}", s.warnings);
            for m in s
                .messages()
                .filter(|m| m.text().starts_with("<session_context>"))
            {
                assert_eq!(
                    (m.role, m.origin, m.kind.as_deref()),
                    (Role::User, Origin::Context, Some("session_context"))
                );
            }
            round_trips(&s);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reducer edge cases

#[test]
fn snapshot_replacement_keeps_dropped_ids_and_marks_the_cut() {
    let s = jsonl(&[
        header(),
        json!({"$set": {"messages": [user("a", "one"), user("b", "two"), user("c", "three")]}}),
        json!({"$set": {"messages": [user("c", "three"), user("d", "four")]}}),
    ]);
    assert_eq!(ids(&s), ["a", "b", "c", "d"]);
    assert_eq!(s.format, "gemini-g2");
    let EntryKind::Rewind(r) = &s.entries[3].kind else {
        panic!("{:?}", s.entries[3].kind)
    };
    assert_eq!(r.dropped, ["a", "b"]);
    assert_eq!(s.entries[3].line, Some(3));
}

#[test]
fn duplicate_ids_keep_first_position_and_last_content() {
    let s = jsonl(&[
        header(),
        user("a", "first"),
        user("b", "other"),
        user("a", "second"),
    ]);
    assert_eq!(ids(&s), ["a", "b"]);
    assert_eq!(message(&s, "a").text(), "second");
    assert_eq!(entry(&s, "a").line, Some(2));
    assert_eq!(s.format, "gemini-g3");
}

#[test]
fn patch_updates_content_results_and_ignores_unknown_ids() {
    let c = json!({"id": "t1", "name": "glob", "args": {"pattern": "*.rs"}, "status": "success", "timestamp": "2026-01-01T00:00:03.000Z"});
    let patch = json!({"$patch": {"updates": [
        {"id": "g", "content": [{"text": "patched"}], "toolCalls": [{"id": "t1", "result": [{"functionResponse": {"id": "t1", "name": "glob", "response": {"output": "Found 1 file"}}}]}]},
        {"id": "nope", "content": "ignored"}
    ]}});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c]), patch]);
    assert_eq!(ids(&s), ["g"]);
    let m = message(&s, "g");
    assert!(matches!(&m.parts[0], Part::Text { text } if text == "patched"));
    let Part::ToolCall(call) = &m.parts[1] else {
        panic!()
    };
    assert_eq!(call.output.as_ref().unwrap().text, "Found 1 file");
    assert!(matches!(&call.tool, Tool::Glob { pattern, path: None } if pattern == "*.rs"));
}

#[test]
fn patch_result_null_leaves_no_output() {
    let c = json!({"id": "t1", "name": "glob", "args": {"pattern": "*"}, "status": "success"});
    let patch = json!({"$patch": {"id": "g", "toolCalls": [{"id": "t1", "result": null}]}});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c]), patch]);
    let call = only_call(&s);
    assert!(call.output.is_none());
    assert_eq!(call.status, ToolStatus::Ok);
}

#[test]
fn patch_remove_and_order_ids() {
    let s = jsonl(&[
        header(),
        user("a", "1"),
        user("b", "2"),
        user("c", "3"),
        json!({"$patch": {"removeIds": ["b", "zzz"]}}),
        // c keeps its place; a moves behind it: live order becomes c, a (a repeated id counts once).
        json!({"$patch": {"orderIds": ["a", "a"]}}),
        json!({"$rewindTo": "c"}),
    ]);
    assert_eq!(ids(&s), ["a", "b", "c"]);
    assert_eq!(rewinds(&s), vec![vec!["b"], vec!["c", "a"]]);
}

fn rewinds(s: &Session) -> Vec<Vec<&str>> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Rewind(r) => Some(r.dropped.iter().map(String::as_str).collect()),
            _ => None,
        })
        .collect()
}

#[test]
fn rewind_to_unknown_id_clears_everything() {
    let s = jsonl(&[
        header(),
        user("a", "1"),
        user("b", "2"),
        json!({"$rewindTo": "missing"}),
    ]);
    let EntryKind::Rewind(r) = &s.entries.last().unwrap().kind else {
        panic!()
    };
    assert_eq!(r.dropped, ["a", "b"]);
    assert_eq!(s.entries.len(), 3);
}

#[test]
fn rewind_with_nothing_live_emits_nothing() {
    let s = jsonl(&[header(), json!({"$rewindTo": "x"})]);
    assert!(s.entries.is_empty());
}

#[test]
fn generations_by_record_shape() {
    assert_eq!(jsonl(&[header()]).format, "gemini-g2");
    assert_eq!(jsonl(&[header(), user("a", "x")]).format, "gemini-g3");
    let snapshot = json!({"$set": {"messages": [user("a", "x")]}});
    assert_eq!(
        jsonl(&[header(), snapshot.clone(), user("b", "y")]).format,
        "gemini-g2"
    );
    assert_eq!(
        jsonl(&[header(), snapshot, json!({"$patch": {"removeIds": []}})]).format,
        "gemini-g3"
    );
}

#[test]
fn set_merges_metadata_and_keeps_unmapped_keys() {
    let s = jsonl(&[
        header(),
        json!({"$set": {"summary": "Tidy up", "directories": ["/a"], "memoryScratchpad": {"version": 1}, "lastUpdated": "2026-01-01T00:05:00.000Z"}}),
    ]);
    assert_eq!(s.title.as_deref(), Some("Tidy up"));
    assert_eq!(s.updated_at, Some(1_767_225_900_000));
    let extra = s.extra.unwrap();
    assert_eq!(
        (
            extra["directories"][0].as_str(),
            extra["memoryScratchpad"]["version"].as_u64()
        ),
        (Some("/a"), Some(1))
    );
    assert_eq!(extra["projectHash"], "h");
}

#[test]
fn torn_and_bad_lines_become_warnings() {
    let mut text = header().to_string();
    text.push_str("\r\n");
    text.push_str(&user("a", "kept").to_string());
    text.push_str("\nnot json at all\n");
    text.push_str(
        &user("b", "caf\u{e9}")
            .to_string()
            .replace("caf\u{e9}", "caf\u{fffd}"),
    );
    text.push_str("\n{\"id\":\"torn\",\"type\":\"us");
    let s = parse_text(&text);
    assert_eq!(ids(&s), ["a", "b"]);
    assert_eq!(
        s.warnings.iter().map(|w| w.line).collect::<Vec<_>>(),
        [Some(3), Some(5)]
    );
}

#[test]
fn invalid_utf8_is_replaced() {
    let mut bytes = header().to_string().into_bytes();
    bytes.push(b'\n');
    let mut line = user("a", "Q").to_string().into_bytes();
    let at = line.iter().position(|b| *b == b'Q').unwrap();
    line[at] = 0xff;
    bytes.extend(line);
    let source = Source::File {
        path: "inline.jsonl".into(),
    };
    let s = agentparse::parse_reader(Agent::Gemini, bytes.as_slice(), source).unwrap();
    assert_eq!(message(&s, "a").text(), "\u{fffd}");
}

#[test]
fn not_a_session_without_a_session_id() {
    let err = try_parse_text(&user("a", "x").to_string()).unwrap_err();
    assert!(matches!(err, Error::NotASession(_)));
    let err =
        try_parse_text("{\"projectHash\":\"h\"}\n{\"id\":\"a\",\"type\":\"user\"}").unwrap_err();
    assert!(matches!(err, Error::NotASession(_)));
}

#[test]
fn header_without_kind_and_a_single_line_header_parse() {
    let s = parse_text(
        r#"{"sessionId":"s9","projectHash":"h","startTime":"2026-01-01T00:00:00.000Z"}"#,
    );
    assert_eq!(
        (s.id.as_str(), s.entries.len(), s.parent.is_none()),
        ("s9", 0, true)
    );
    assert!(s.extra.unwrap().get("kind").is_none());
}

// ---------------------------------------------------------------------------------------------
// Messages

#[test]
fn content_shapes() {
    let user_obj = json!({"id": "u", "type": "user", "content": {"text": "single part"}});
    let user_str = json!({"id": "v", "type": "user", "content": "plain string"});
    let mixed = json!({"id": "w", "type": "user", "content": ["bare", {"text": "obj"}]});
    let g_empty = gem(
        "g",
        json!(""),
        vec![call(
            "t",
            "list_directory",
            json!({"dir_path": "/x"}),
            "success",
            json!({"output": "ok"}),
        )],
    );
    let s = jsonl(&[header(), user_obj, user_str, mixed, g_empty]);
    assert_eq!(message(&s, "u").text(), "single part");
    assert_eq!(message(&s, "v").text(), "plain string");
    assert_eq!(message(&s, "w").text(), "bare\n\nobj");
    // An empty `content` with tool calls only: just the call.
    let g = message(&s, "g");
    assert!(matches!(g.parts.as_slice(), [Part::ToolCall(_)]));
}

#[test]
fn thoughts_tokens_and_model_variants() {
    let with_null = json!({"id": "a", "type": "gemini", "content": "x", "thoughts": [], "tokens": null, "model": "m1"});
    let bare_tokens = json!({"id": "b", "type": "gemini", "content": "y", "tokens": {"input": 10, "output": 2, "cached": 4, "total": 12}});
    let thoughts = json!({"id": "c", "type": "gemini", "content": "z",
        "thoughts": [{"subject": "", "description": "no subject"}, {"subject": "Only", "description": ""}]});
    let s = jsonl(&[header(), with_null, bare_tokens, thoughts]);
    assert!(message(&s, "a").usage.is_none());
    let u = message(&s, "b").usage.unwrap();
    assert_eq!((u.input, u.output, u.reasoning, u.cache_read), (6, 2, 0, 4));
    assert_eq!(u.total(), 12);
    let c = message(&s, "c");
    assert!(matches!(&c.parts[0], Part::Reasoning { text: Some(t), .. } if t == "no subject"));
    assert!(matches!(&c.parts[1], Part::Reasoning { text: Some(t), .. } if t == "**Only**"));
    assert_eq!(s.models, ["m1"]);
}

#[test]
fn tool_prompt_tokens_count_as_input_and_cached_above_input_warns() {
    let tool = json!({"id": "a", "type": "gemini", "content": "x", "tokens": {"input": 100, "output": 5, "cached": 60, "thoughts": 7, "tool": 9, "total": 121}});
    let odd = json!({"id": "b", "type": "gemini", "content": "y", "tokens": {"input": 10, "output": 1, "cached": 30, "total": 11}});
    let s = jsonl(&[header(), tool, odd]);
    let u = message(&s, "a").usage.unwrap();
    assert_eq!(
        (u.input, u.output, u.reasoning, u.cache_read),
        (49, 12, 7, 60)
    );
    assert_eq!(u.total(), 121);
    assert_eq!(message(&s, "b").usage.unwrap().input, 0);
    assert_eq!(s.warnings.len(), 1);
    assert!(s.warnings[0].message.contains("exceed"));
}

#[test]
fn user_record_classes() {
    let slash = user("s", "/dir add /tmp/x");
    let path = user("p", "/usr/bin/env is missing, why?");
    let hook = user("h", "<hook_context>extra</hook_context>");
    let s = jsonl(&[header(), slash, path, hook]);
    let m = message(&s, "s");
    assert_eq!(
        (m.origin, m.kind.as_deref()),
        (Origin::Command, Some("slash_command"))
    );
    assert!(message(&s, "p").is_human());
    let m = message(&s, "h");
    assert_eq!(
        (m.origin, m.kind.as_deref(), m.text().as_str()),
        (
            Origin::Context,
            Some("hook_context"),
            "<hook_context>extra</hook_context>"
        )
    );
}

#[test]
fn mixed_user_record_splits_by_kind() {
    let rec = json!({"id": "m", "type": "user", "content": [
        {"text": "<session_context>ctx</session_context>"}, {"text": "do the thing"}, {"inlineData": {"mimeType": "image/png", "data": "aGVsbG8="}}]});
    let s = jsonl(&[header(), rec]);
    assert_eq!(ids(&s), ["m#1", "m#2"]);
    assert_eq!(message(&s, "m#1").kind.as_deref(), Some("session_context"));
    let human = message(&s, "m#2");
    assert!(human.is_human());
    let Part::Attachment(media) = &human.parts[1] else {
        panic!()
    };
    assert_eq!(
        (media.mime.as_deref(), media.bytes),
        (Some("image/png"), Some(5))
    );
}

/// A prompt with `@file` mentions is the text as typed; the expansion stays in `extra`.
#[test]
fn display_content_is_the_human_text() {
    let rec = json!({"id": "u", "type": "user", "content": [{"text": "explain @a.ts\n<file a.ts>body</file>"}], "displayContent": [{"text": "explain @a.ts"}]});
    let s = jsonl(&[header(), rec]);
    let m = message(&s, "u");
    assert_eq!((m.is_human(), m.text().as_str()), (true, "explain @a.ts"));
    let extra = entry(&s, "u").extra.as_ref().unwrap();
    assert_eq!(
        extra["content"][0]["text"],
        "explain @a.ts\n<file a.ts>body</file>"
    );
    assert!(extra.get("displayContent").is_none());
    // An empty `displayContent` is no text: the content is the prompt.
    let rec =
        json!({"id": "v", "type": "user", "content": [{"text": "plain"}], "displayContent": ""});
    let s = jsonl(&[header(), rec]);
    assert_eq!(message(&s, "v").text(), "plain");
}

/// The compression marker is an `info` with no text: no notice.
#[test]
fn an_empty_info_is_no_notice() {
    let empty = json!({"id": "i1", "type": "info", "content": ""});
    let full = json!({"id": "i2", "type": "info", "content": "Chat history compressed."});
    let s = jsonl(&[header(), empty, full]);
    assert_eq!(ids(&s), ["i2"]);
}

#[test]
fn echo_turns_fold_and_orphans_stay_visible() {
    let c = call(
        "t1",
        "glob",
        json!({"pattern": "*"}),
        "success",
        json!({"output": "Found 0 files"}),
    );
    let echo = |id: &str| json!({"id": format!("e-{id}"), "type": "user", "content": [{"functionResponse": {"id": id, "name": "glob", "response": {"output": "x"}}}]});
    let s = jsonl(&[
        header(),
        gem("g", json!(""), vec![c]),
        echo("t1"),
        echo("ghost"),
    ]);
    assert_eq!(ids(&s), ["g", "e-ghost"]);
    let orphan = message(&s, "e-ghost");
    assert_eq!((orphan.role, orphan.origin), (Role::User, Origin::Tool));
    let Part::ToolResult(r) = &orphan.parts[0] else {
        panic!()
    };
    assert_eq!(
        (r.call_id.as_str(), r.status, r.output.text.as_str()),
        ("ghost", ToolStatus::Ok, "x")
    );
    assert!(s.warnings.iter().any(|w| w.message.contains("ghost")));
}

#[test]
fn echo_turn_supplies_a_result_the_record_lacks() {
    let c = json!({"id": "t1", "name": "list_directory", "args": {"dir_path": "/x"}, "status": "success"});
    let echo = json!({"id": "e", "type": "user", "content": [{"functionResponse": {"id": "t1", "name": "list_directory", "response": {"output": "[DIR] a"}}}]});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c]), echo]);
    assert_eq!(only_call(&s).output.unwrap().text, "[DIR] a");
    assert_eq!(ids(&s), ["g"]);
}

#[test]
fn function_call_part_without_a_record_becomes_a_call() {
    let content = json!([{"text": "going"}, {"functionCall": {"id": "f1", "name": "write_file", "args": {"file_path": "/a", "content": "x"}}}]);
    let s = jsonl(&[
        header(),
        gem("g", content, vec![]),
        gem("g2", json!("next"), vec![]),
    ]);
    let call = only_call(&s);
    assert_eq!(call.id, "f1");
    assert!(
        matches!(&call.tool, Tool::Write { path, content: Some(c) } if path == "/a" && c == "x")
    );
    // No result and a later model message exists: the call never finished.
    assert_eq!(
        (call.status, call.output.is_none()),
        (ToolStatus::Interrupted, true)
    );
}

#[test]
fn pending_when_nothing_follows() {
    let c = json!({"id": "t", "name": "glob", "args": {"pattern": "*"}, "status": "executing"});
    let last = jsonl(&[header(), gem("g", json!(""), vec![c.clone()])]);
    assert_eq!(only_call(&last).status, ToolStatus::Pending);
    assert_eq!(only_call(&last).status_detail.as_deref(), Some("executing"));
    // A later user message alone proves nothing; a later model message does.
    let user_after = jsonl(&[
        header(),
        gem("g", json!(""), vec![c.clone()]),
        user("u", "hi"),
    ]);
    assert_eq!(only_call(&user_after).status, ToolStatus::Pending);
    let later = jsonl(&[
        header(),
        gem("g", json!(""), vec![c]),
        gem("g2", json!("ok"), vec![]),
    ]);
    assert_eq!(only_call(&later).status, ToolStatus::Interrupted);
}

#[test]
fn result_as_plain_string() {
    let c = json!({"id": "t", "name": "glob", "args": {"pattern": "*"}, "status": "success", "result": "raw-output"});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c])]);
    assert_eq!(only_call(&s).output.unwrap().text, "raw-output");
}

#[test]
fn binary_results_keep_only_sizes() {
    let c = json!({"id": "t", "name": "read_file", "args": {"file_path": "/a.png"}, "status": "success", "result": [
        {"functionResponse": {"id": "t", "name": "read_file", "response": {}, "parts": [{"inlineData": {"mimeType": "image/png", "data": "aGVsbG8h"}}]}},
        {"inlineData": {"mimeType": "application/pdf", "data": "aGVsbG8=", "displayName": "a.pdf"}}]});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c])]);
    let out = only_call(&s).output.unwrap();
    assert_eq!(out.text, "");
    assert_eq!(out.attachments.len(), 2);
    assert_eq!(
        (
            out.attachments[0].bytes,
            out.attachments[1].name.as_deref(),
            out.attachments[1].bytes
        ),
        (Some(6), Some("a.pdf"), Some(5))
    );
    let dumped = serde_json::to_string(&s).unwrap();
    assert!(!dumped.contains("aGVsbG8"));
}

#[test]
fn signatures_never_stored() {
    let content = json!([{"text": "thinking", "thought": true, "thoughtSignature": "c2VjcmV0c2VjcmV0"}, {"thought": true, "thoughtSignature": "YQ=="}]);
    let s = jsonl(&[header(), gem("g", content, vec![])]);
    let parts = message(&s, "g").parts;
    assert!(
        matches!(&parts[0], Part::Reasoning { text: Some(t), encrypted: true } if t == "thinking")
    );
    assert!(matches!(
        &parts[1],
        Part::Reasoning {
            text: None,
            encrypted: true
        }
    ));
    assert!(!serde_json::to_string(&s).unwrap().contains("c2VjcmV0"));
}

// ---------------------------------------------------------------------------------------------
// Compaction

#[test]
fn state_snapshot_becomes_a_compaction_and_the_ack_is_labeled() {
    let snapshot = user(
        "snap",
        "<state_snapshot>\n<overall_goal>x</overall_goal>\n</state_snapshot>",
    );
    let ack = gem(
        "ack",
        json!("Got it. Thanks for the additional context!"),
        vec![],
    );
    let info = json!({"id": "i", "type": "info", "content": "Chat history compressed from 1,200 to 300 tokens."});
    let later_ack = gem("late", json!("Got it, doing that"), vec![]);
    let s = jsonl(&[
        header(),
        user("old", "long talk"),
        snapshot,
        ack,
        info,
        later_ack,
    ]);
    let EntryKind::Compaction(c) = &entry(&s, "snap").kind else {
        panic!()
    };
    assert!(
        c.summary
            .as_deref()
            .unwrap()
            .contains("<overall_goal>x</overall_goal>")
    );
    assert_eq!(
        (c.tokens_before, c.tokens_after, c.trigger.as_deref()),
        (Some(1200), Some(300), None)
    );
    assert_eq!(message(&s, "ack").kind.as_deref(), Some("compaction_ack"));
    assert_eq!(message(&s, "late").kind, None);
    // The summary is not also a user message.
    assert_eq!(
        s.messages()
            .filter(|m| m.text().contains("state_snapshot"))
            .count(),
        0
    );
}

#[test]
fn compaction_in_a_snapshot_rewrite_marks_the_dropped_history() {
    let s = jsonl(&[
        header(),
        json!({"$set": {"messages": [user("a", "one"), gem("b", json!("two"), vec![])]}}),
        json!({"$set": {"messages": [user("snap", "<state_snapshot>s</state_snapshot>"), gem("ack", json!("Got it. Thanks for the additional context!"), vec![])]}}),
    ]);
    assert_eq!(ids(&s), ["a", "b", "snap", "ack"]);
    assert!(matches!(&s.entries[2].kind, EntryKind::Rewind(r) if r.dropped == ["a", "b"]));
    assert!(matches!(&s.entries[3].kind, EntryKind::Compaction(_)));
    assert_eq!(message(&s, "ack").kind.as_deref(), Some("compaction_ack"));
}

// ---------------------------------------------------------------------------------------------
// Masking

#[test]
fn masked_result_without_an_original_is_marked_truncated() {
    let masked = "<tool_output_masked>\n[output masked to save context]\n</tool_output_masked>";
    let c = call(
        "t",
        "run_shell_command",
        json!({"command": "ls"}),
        "success",
        json!({"output": masked}),
    );
    let out = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]))
        .output
        .unwrap();
    assert!(out.truncated && out.text.contains("masked"));
}

// ---------------------------------------------------------------------------------------------
// Shell text per era

#[test]
fn shell_exit_zero_only_for_modern_success() {
    let r = shell("Output: hi\nProcess Group PGID: 9", "success")
        .output
        .unwrap()
        .shell
        .unwrap();
    assert_eq!((r.exit_code, r.stdout.as_deref()), (Some(0), Some("hi")));
    let r = shell("Output: (empty)", "success")
        .output
        .unwrap()
        .shell
        .unwrap();
    assert_eq!((r.exit_code, r.stdout.as_deref()), (Some(0), Some("")));
}

#[test]
fn shell_exit_code_signal_and_pids() {
    let c = shell(
        "Output: a\nb\nExit Code: 137\nSignal: SIGKILL\nBackground PIDs: 12, 34\nProcess Group PGID: 7",
        "success",
    );
    let out = c.output.unwrap();
    let r = out.shell.unwrap();
    assert_eq!(
        (r.exit_code, r.signal.as_deref(), r.stdout.as_deref()),
        (Some(137), Some("SIGKILL"), Some("a\nb"))
    );
    assert_eq!(r.background_id.as_deref(), Some("12"));
    assert_eq!(
        out.data.as_ref().unwrap()["background_pids"],
        json!([12, 34])
    );
    assert_eq!(out.data.as_ref().unwrap()["process_group"], 7);
    assert_eq!(c.status, ToolStatus::Error);
}

#[test]
fn shell_stdout_that_looks_like_a_tail_line_is_not_misread() {
    let r = shell("Output: Exit Code: 5 in the text\nExit Code: 1", "success")
        .output
        .unwrap()
        .shell
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref()),
        (Some(1), Some("Exit Code: 5 in the text"))
    );
}

#[test]
fn shell_old_era_splits_stdout_and_stderr() {
    let text = "Command: sh t.sh\nDirectory: sub\nStdout: out-line\nmore\nStderr: err-line\nError: (none)\nExit Code: 3\nSignal: (none)\nBackground PIDs: (none)\nProcess Group PGID: 4242";
    let c = shell(text, "success");
    let r = c.output.as_ref().unwrap().shell.clone().unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.stderr.as_deref()),
        (Some(3), Some("out-line\nmore"), Some("err-line"))
    );
    assert_eq!((r.signal, r.background_id), (None, None));
    let Tool::Shell(sh) = &c.tool else { panic!() };
    assert_eq!(sh.cwd.as_deref(), Some("sub"));
    assert_eq!(c.status, ToolStatus::Error);
}

#[test]
fn shell_old_era_placeholders() {
    let text = "Command: true\nDirectory: (root)\nStdout: (empty)\nStderr: (empty)\nError: (none)\nExit Code: (none)\nSignal: (none)\nBackground PIDs: (none)\nProcess Group PGID: (none)";
    let c = shell(text, "success");
    let r = c.output.as_ref().unwrap().shell.clone().unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref(), r.stderr.as_deref()),
        (None, Some(""), Some(""))
    );
    let Tool::Shell(sh) = &c.tool else { panic!() };
    assert_eq!(sh.cwd, None);
    assert_eq!(c.status, ToolStatus::Ok);
}

#[test]
fn shell_untrusted_wrapper_is_unwrapped_and_the_escaped_tag_restored() {
    let text = "<untrusted_context>\nOutput: has &lt;/untrusted_context&gt; inside\nExit Code: 2\n</untrusted_context>";
    let c = shell(text, "success");
    let out = c.output.unwrap();
    let r = out.shell.unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_deref()),
        (Some(2), Some("has </untrusted_context> inside"))
    );
    assert!(out.text.starts_with("<untrusted_context>"));
}

#[test]
fn shell_truncated_output_keeps_its_exit_code_and_saved_path() {
    // `formatTruncatedToolOutput` cuts the whole result: a banner line, then the head of
    // `Output: ...`, an omission marker and the tail that still ends with the tail lines.
    let text = "Output too large. Showing first 8,000 and last 32,000 characters. For full output see: /tmp/proj/tool-outputs/session-1/out.txt\nOutput: head line\n\n... [5 characters omitted] ...\n\ntail line\nExit Code: 2\nProcess Group PGID: 9";
    let c = shell(text, "success");
    let out = c.output.unwrap();
    assert!(out.truncated);
    assert_eq!(
        out.full_output_path.as_deref(),
        Some("/tmp/proj/tool-outputs/session-1/out.txt")
    );
    let r = out.shell.unwrap();
    assert_eq!(r.exit_code, Some(2));
    assert!(
        r.stdout.as_deref().unwrap().starts_with("head line")
            && r.stdout.unwrap().ends_with("tail line")
    );
    assert_eq!(c.status, ToolStatus::Error);
    assert_eq!(out.data.unwrap()["process_group"], 9);

    // Since v0.49 the banner sits outside the `<untrusted_context>` wrapper.
    let wrapped = "Output too large. Showing first 8,000 and last 32,000 characters. For full output see: /tmp/o.txt\n<untrusted_context>\nOutput: head\n\n... [5 characters omitted] ...\n\ntail\nExit Code: 2\n</untrusted_context>";
    let c = shell(wrapped, "success");
    let r = c.output.unwrap().shell.unwrap();
    assert_eq!((r.exit_code, c.status), (Some(2), ToolStatus::Error));
    assert!(!r.stdout.unwrap().contains("untrusted_context"));
}

#[test]
fn shell_background() {
    let text = "Command moved to background (PID: 4711). Output hidden. Press Ctrl+B to view.";
    let r = shell(text, "success").output.unwrap().shell.unwrap();
    assert_eq!(
        (r.background_id.as_deref(), r.stdout, r.exit_code),
        (Some("4711"), None, None)
    );
}

#[test]
fn shell_cancelled_with_partial_output_and_timeout() {
    let c = call(
        "c1",
        "run_shell_command",
        json!({"command": "x"}),
        "cancelled",
        json!({"output": "Command was cancelled by user before it could complete. Below is the output before it was cancelled:\nhalf", "error": "[Operation Cancelled] Reason: Operation cancelled by user"}),
    );
    let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    let out = call.output.as_ref().unwrap();
    let r = out.shell.as_ref().unwrap();
    assert!(r.interrupted && !r.timed_out);
    assert_eq!(r.stdout.as_deref(), Some("half"));
    assert!(!out.text.contains("Operation Cancelled"));
    assert_eq!(call.status, ToolStatus::Interrupted);

    let timeout = "Command was automatically cancelled because it exceeded the timeout of 5.0 minutes without output. Below is the output before it was cancelled:\nslow";
    let c = shell(timeout, "success");
    let r = c.output.as_ref().unwrap().shell.clone().unwrap();
    assert!(r.timed_out && !r.interrupted);
    assert_eq!(r.stdout.as_deref(), Some("slow"));
    // Gemini records `success`; the timeout still makes it an error.
    assert_eq!(
        (c.status, c.status_detail.as_deref()),
        (ToolStatus::Error, Some("success"))
    );
}

#[test]
fn denied_calls_are_rejected_with_the_reason() {
    // Scheduler: `[Operation Cancelled] Reason: User denied execution.`
    let c = call(
        "c1",
        "write_file",
        json!({"file_path": "/a", "content": "x"}),
        "cancelled",
        json!({"error": "[Operation Cancelled] Reason: User denied execution."}),
    );
    let refused = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    assert_eq!(refused.status, ToolStatus::Rejected);
    assert_eq!(
        refused.status_detail.as_deref(),
        Some("cancelled: User denied execution.")
    );
    assert_eq!(
        refused.output.unwrap().text,
        "[Operation Cancelled] Reason: User denied execution."
    );

    // Policy engine: recorded as an error, still a refusal.
    let c = call(
        "c2",
        "run_shell_command",
        json!({"command": "rm -rf /"}),
        "error",
        json!({"error": "Tool execution denied by policy."}),
    );
    let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    assert_eq!(call.status, ToolStatus::Rejected);
    assert_eq!(
        call.status_detail.as_deref(),
        Some("error: Tool execution denied by policy.")
    );
}

#[test]
fn user_aborts_are_interrupted_not_rejected() {
    for reason in [
        "Operation cancelled by user",
        "User cancelled operation",
        "User cancelled tool execution.",
    ] {
        let error = format!("[Operation Cancelled] Reason: {reason}");
        let c = call(
            "c1",
            "glob",
            json!({"pattern": "*"}),
            "cancelled",
            json!({ "error": error }),
        );
        let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
        assert_eq!(call.status, ToolStatus::Interrupted, "{reason}");
        assert_eq!(call.status_detail, Some(format!("cancelled: {reason}")));
    }
}

#[test]
fn shell_spawn_failure_is_an_error_without_a_shell_result_stream() {
    let c = call(
        "c1",
        "run_shell_command",
        json!({"command": "x"}),
        "error",
        json!({"error": "spawn failed"}),
    );
    let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    let out = call.output.unwrap();
    assert_eq!(
        (call.status, out.text.as_str()),
        (ToolStatus::Error, "spawn failed")
    );
    let r = out.shell.unwrap();
    assert_eq!((r.stdout, r.exit_code), (None, None));
}

// ---------------------------------------------------------------------------------------------
// Typed tools

fn typed(name: &str, args: Value) -> Tool {
    let c = call("t", name, args, "success", json!({"output": "ok"}));
    only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])])).tool
}

#[test]
fn tool_names_and_argument_renames_across_versions() {
    let read = |args| typed("read_file", args);
    assert!(
        matches!(read(json!({"absolute_path": "/a"})), Tool::Read { path, offset: None, limit: None } if path == "/a")
    );
    assert!(matches!(
        read(json!({"file_path": "/a", "offset": 5, "limit": 10})),
        Tool::Read {
            offset: Some(5),
            limit: Some(10),
            ..
        }
    ));
    assert!(matches!(
        read(json!({"file_path": "/a", "start_line": 5, "end_line": 14})),
        Tool::Read {
            offset: Some(5),
            limit: Some(10),
            ..
        }
    ));
    assert!(matches!(
        read(json!({"file_path": "/a", "end_line": "3"})),
        Tool::Read {
            offset: None,
            limit: Some(3),
            ..
        }
    ));

    assert!(
        matches!(typed("search_file_content", json!({"pattern": "x", "path": "src", "include": "*.rs"})),
        Tool::Search { pattern, path: Some(p), glob: Some(g) } if (pattern.as_str(), p.as_str(), g.as_str()) == ("x", "src", "*.rs"))
    );
    assert!(
        matches!(typed("grep_search", json!({"pattern": "x", "dir_path": "d", "include_pattern": "*.ts"})),
        Tool::Search { path: Some(p), glob: Some(g), .. } if (p.as_str(), g.as_str()) == ("d", "*.ts"))
    );
    assert!(
        matches!(typed("glob", json!({"pattern": "*.rs", "path": "d"})), Tool::Glob { path: Some(p), .. } if p == "d")
    );
    assert!(
        matches!(typed("list_directory", json!({"path": "old"})), Tool::List { path: Some(p) } if p == "old")
    );
    assert!(matches!(
        typed("write_file", json!({"file_path": "/a", "content": "c"})),
        Tool::Write { .. }
    ));

    let Tool::Edit { edits, .. } = typed(
        "replace",
        json!({"file_path": "/a", "old_string": "a", "new_string": "b", "expected_replacements": 3}),
    ) else {
        panic!()
    };
    assert!(edits[0].replace_all);
    let Tool::Edit { edits, .. } = typed(
        "replace",
        json!({"file_path": "/a", "old_string": "", "new_string": "new file", "allow_multiple": true}),
    ) else {
        panic!()
    };
    assert_eq!((edits[0].old.as_str(), edits[0].replace_all), ("", true));

    let Tool::Shell(sh) = typed(
        "run_shell_command",
        json!({"command": "ls", "directory": "d", "is_background": true}),
    ) else {
        panic!()
    };
    assert_eq!(
        (sh.cwd.as_deref(), sh.background, sh.timeout_ms),
        (Some("d"), true, None)
    );
}

#[test]
fn tool_names_beyond_files_and_shell() {
    assert!(
        matches!(typed("google_web_search", json!({"query": "rust"})), Tool::WebSearch { query: Some(q) } if q == "rust")
    );
    let Tool::WebFetch { url, prompt } = typed(
        "web_fetch",
        json!({"prompt": "Summarize https://example.com/a?b=1, then compare."}),
    ) else {
        panic!()
    };
    assert_eq!(url.as_deref(), Some("https://example.com/a?b=1"));
    assert!(prompt.is_some());
    for name in ["write_todos", "write_todos_list"] {
        let Tool::Todo { items } = typed(
            name,
            json!({"todos": [{"description": "a", "status": "pending"}, {"description": "b", "status": "completed"}]}),
        ) else {
            panic!()
        };
        assert_eq!(
            (
                items.len(),
                items[1].content.as_str(),
                items[1].status.as_deref()
            ),
            (2, "b", Some("completed"))
        );
    }
    assert!(matches!(
        typed("ask_user", json!({"questions": []})),
        Tool::Question
    ));
    assert!(matches!(typed("enter_plan_mode", json!({})), Tool::Plan));
    assert!(matches!(
        typed("exit_plan_mode", json!({"plan_filename": "p.md"})),
        Tool::Plan
    ));
    assert!(
        matches!(typed("activate_skill", json!({"name": "pdf"})), Tool::Skill { name: Some(n) } if n == "pdf")
    );
    assert!(
        matches!(typed("mcp_my_server_do_thing", json!({"a": 1})), Tool::Mcp { server, tool } if (server.as_str(), tool.as_str()) == ("my", "server_do_thing"))
    );
    for name in [
        "read_many_files",
        "save_memory",
        "tracker_create_task",
        "get_internal_docs",
        "mcp_x",
    ] {
        assert!(matches!(typed(name, json!({})), Tool::Other), "{name}");
    }
}

#[test]
fn a_tool_missing_a_required_field_stays_other_with_its_raw_input() {
    let c = call(
        "t",
        "run_shell_command",
        json!({"description": "no command"}),
        "success",
        json!({"output": "Output: x"}),
    );
    let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    assert!(matches!(call.tool, Tool::Other));
    assert_eq!(call.input["description"], "no command");
    assert_eq!(call.name, "run_shell_command");
}

#[test]
fn write_file_new_file_diff_is_an_add() {
    let mut c = call(
        "t",
        "write_file",
        json!({"file_path": "/n.txt", "content": "a\nb\n"}),
        "success",
        json!({"output": "Successfully created and wrote to new file: /n.txt."}),
    );
    c["resultDisplay"] = json!({"fileDiff": "--- n.txt\tOriginal\n+++ n.txt\tWritten\n@@ -0,0 +1,2 @@\n+a\n+b\n", "fileName": "n.txt", "filePath": "/n.txt", "originalContent": null, "newContent": "a\nb\n", "isNewFile": true});
    let out = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]))
        .output
        .unwrap();
    let f = &out.files[0];
    assert_eq!(
        (f.op, f.additions, f.deletions, f.path.as_str()),
        (FileOp::Add, Some(2), Some(0), "/n.txt")
    );
    assert!(
        out.data.unwrap()["resultDisplay"]
            .get("isNewFile")
            .is_none()
    );
}

#[test]
fn todo_and_listing_displays_stay_in_data() {
    let mut c = call(
        "t",
        "write_todos",
        json!({"todos": [{"description": "a", "status": "pending"}]}),
        "success",
        json!({"output": "Successfully updated the todo list."}),
    );
    c["resultDisplay"] = json!({"todos": [{"description": "a", "status": "pending"}]});
    let out = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]))
        .output
        .unwrap();
    assert_eq!(
        out.data.unwrap()["resultDisplay"]["todos"][0]["status"],
        "pending"
    );
}

// ---------------------------------------------------------------------------------------------
// Subagents and the project directory

#[test]
fn subagent_file_links_to_its_parent_and_reads_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let chats = dir.path().join("proj/chats");
    std::fs::create_dir_all(chats.join("parent-session")).unwrap();
    std::fs::write(dir.path().join("proj/.project_root"), "/work/proj").unwrap();
    let child = chats.join("parent-session/agent-1.jsonl");
    let mut head = header();
    head["sessionId"] = json!("agent-1");
    head["kind"] = json!("subagent");
    let lines = [
        head,
        user("p", "Investigate the parser"),
        gem("g", json!("done"), vec![]),
    ];
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(&child, text.join("\n")).unwrap();

    let s = agentparse::parse_file(&child).unwrap();
    assert_eq!(s.id, "agent-1");
    let parent = s.parent.as_ref().unwrap();
    assert_eq!(
        (parent.session_id.as_str(), parent.call_id.as_deref()),
        ("parent-session", None)
    );
    assert_eq!(s.cwd.as_deref(), Some("/work/proj"));
    assert_eq!(message(&s, "p").origin, Origin::Agent);
    assert_eq!(s.extra.unwrap()["kind"], "subagent");

    // A main session next to its chats directory gets the cwd and no parent.
    let main = chats.join("session-2026-01-01T00-00-s1.jsonl");
    std::fs::write(
        &main,
        [header(), user("u", "hi")]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let s = agentparse::parse_file(&main).unwrap();
    assert_eq!(
        (s.cwd.as_deref(), s.parent.is_none()),
        (Some("/work/proj"), true)
    );
    assert!(message(&s, "u").is_human());
}

#[test]
fn subagent_tool_call_links_the_child_session() {
    let mut c = call(
        "a1",
        "invoke_agent",
        json!({"agent_name": "codebase_investigator", "prompt": "Map the parser"}),
        "success",
        json!({"output": "Found it"}),
    );
    c["agentId"] = json!("child-7");
    let legacy = call(
        "a2",
        "codebase_investigator",
        json!({"objective": "Look around"}),
        "success",
        json!({"output": "ok"}),
    );
    let s = jsonl(&[header(), gem("g", json!(""), vec![c, legacy])]);
    let calls = tool_calls_of(&s, "g");
    assert_eq!(calls[0].child_session.as_deref(), Some("child-7"));
    assert!(
        matches!(&calls[0].tool, Tool::Task { prompt: Some(p), agent_type: Some(a), .. } if (p.as_str(), a.as_str()) == ("Map the parser", "codebase_investigator"))
    );
    assert!(
        matches!(&calls[1].tool, Tool::Task { prompt: Some(p), agent_type: Some(a), .. } if (p.as_str(), a.as_str()) == ("Look around", "codebase_investigator"))
    );
    // `agentId` is the link, not data.
    assert!(calls[0].output.as_ref().unwrap().data.is_none());
}

#[test]
fn cwd_falls_back_to_the_session_context_workspace() {
    let ctx = user(
        "c",
        "<session_context>\n- **Workspace Directories:**\n  - /w/proj\n- **Directory Structure:**\n</session_context>",
    );
    assert_eq!(jsonl(&[header(), ctx]).cwd.as_deref(), Some("/w/proj"));
    assert_eq!(
        jsonl(&[
            header(),
            user("c", "<session_context>\nno dirs\n</session_context>")
        ])
        .cwd,
        None
    );
}

// ---------------------------------------------------------------------------------------------
// Unknown records

#[test]
fn unknown_message_type_stays_as_other() {
    let rec = json!({"id": "x", "type": "mystery", "content": "?", "extra": 1});
    let s = jsonl(&[header(), rec]);
    let EntryKind::Other(o) = &entry(&s, "x").kind else {
        panic!()
    };
    assert_eq!(
        (o.kind.as_str(), o.data["extra"].as_u64()),
        ("mystery", Some(1))
    );
}

#[test]
fn non_object_lines_warn_and_unknown_records_are_ignored() {
    let s = parse_text(&format!("{}\n[1,2]\n{{\"nothing\":true}}\n", header()));
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.warnings[0].line, Some(2));
}

#[test]
fn truncated_files_never_panic() {
    for name in [
        "gemini-old-session.json",
        "gemini-snapshot-session.jsonl",
        "gemini-new-session.jsonl",
    ] {
        let text = std::fs::read_to_string(fixture(name)).unwrap();
        for end in (0..text.len())
            .filter(|i| text.is_char_boundary(*i))
            .step_by(23)
        {
            let _ = try_parse_text(&text[..end]);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes upstream really writes

const FILE_DIFF: &str = "Index: f.txt\n===================================================================\n--- f.txt\tCurrent\n+++ f.txt\tProposed\n@@ -1,2 +1,3 @@\n-- old sql comment\n context\n++counter\n+-- new\n";

fn with_display(mut call: Value, display: Value) -> Value {
    call["resultDisplay"] = display;
    call
}

#[test]
fn shell_stdout_ending_in_an_error_line_keeps_it() {
    // A failing tool printing `Error: ...` last: shell.ts only prints its own `Error:` on spawn failure.
    let c = shell(
        "Output: tests ran\nError: Process completed with exit code 1.\nExit Code: 1\nProcess Group PGID: 9",
        "success",
    );
    let out = c.output.unwrap();
    let r = out.shell.unwrap();
    assert_eq!(r.exit_code, Some(1));
    assert_eq!(
        r.stdout.as_deref(),
        Some("tests ran\nError: Process completed with exit code 1.")
    );
    assert!(out.data.unwrap().get("error").is_none());
}

#[test]
fn shell_spawn_error_line_is_stored() {
    let c = shell(
        "Output: (empty)\nError: spawn nope ENOENT\nProcess Group PGID: 4",
        "success",
    );
    let out = c.output.unwrap();
    let r = out.shell.unwrap();
    // No process ever exited: the exit code stays unknown.
    assert_eq!((r.stdout.as_deref(), r.exit_code), (Some(""), None));
    assert_eq!(out.data.unwrap()["error"], "spawn nope ENOENT");
}

#[test]
fn shell_output_mentioning_the_background_sentence_is_not_a_background_start() {
    let text = "Output: docs say: Command moved to background (PID: 99) means ...\nExit Code: 3";
    let r = shell(text, "success").output.unwrap().shell.unwrap();
    assert_eq!((r.background_id, r.exit_code), (None, Some(3)));
    assert!(r.stdout.unwrap().starts_with("docs say"));
}

#[test]
fn shell_cancelled_before_start_and_signal_kills() {
    let c = shell(
        "Command was cancelled by user before it could start.",
        "cancelled",
    );
    let r = c.output.as_ref().unwrap().shell.clone().unwrap();
    assert!(r.interrupted && r.stdout.is_none());
    assert_eq!(c.status, ToolStatus::Interrupted);

    // Killed by a signal: no exit code line, only `Signal:`; success is recorded, the call is not one.
    let c = shell(
        "Output: partial\nSignal: SIGTERM\nProcess Group PGID: 5",
        "success",
    );
    let r = c.output.as_ref().unwrap().shell.clone().unwrap();
    assert_eq!((r.signal.as_deref(), r.exit_code), (Some("SIGTERM"), None));
    assert_eq!(c.status, ToolStatus::Error);
}

#[test]
fn cancelled_edit_reports_no_file_change_but_keeps_the_diff_in_data() {
    // `toCancelled` copies the confirmation diff into resultDisplay of a call that never ran.
    let c = call(
        "c1",
        "replace",
        json!({"file_path": "/f.txt", "old_string": "a", "new_string": "b"}),
        "cancelled",
        json!({"error": "[Operation Cancelled] Reason: User denied execution."}),
    );
    let c = with_display(
        c,
        json!({"fileDiff": FILE_DIFF, "fileName": "f.txt", "filePath": "/f.txt", "originalContent": "a", "newContent": "b"}),
    );
    let call = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]));
    let out = call.output.unwrap();
    assert!(out.files.is_empty());
    assert_eq!(out.data.unwrap()["resultDisplay"]["fileDiff"], FILE_DIFF);
    assert_eq!(call.status, ToolStatus::Rejected);
}

#[test]
fn file_diff_counts_hunk_lines_only() {
    let c = call(
        "c1",
        "replace",
        json!({"file_path": "/f.txt", "old_string": "a", "new_string": "b"}),
        "success",
        json!({"output": "Successfully modified file: /f.txt (1 replacements)."}),
    );
    // CRLF diff text, as written on Windows.
    let crlf = FILE_DIFF.replace('\n', "\r\n");
    let c = with_display(
        c,
        json!({"fileDiff": crlf, "fileName": "f.txt", "filePath": "/f.txt"}),
    );
    let out = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]))
        .output
        .unwrap();
    let f = &out.files[0];
    assert_eq!((f.additions, f.deletions), (Some(2), Some(1)));
}

#[test]
fn write_file_content_is_not_kept_twice_in_data() {
    let c = call(
        "c1",
        "write_file",
        json!({"file_path": "/n.txt", "content": "body\n"}),
        "success",
        json!({"output": "Successfully created and wrote to new file: /n.txt."}),
    );
    let c = with_display(
        c,
        json!({"fileDiff": "@@ -0,0 +1 @@\n+body\n", "fileName": "n.txt", "filePath": "/n.txt", "originalContent": null, "newContent": "body\n", "isNewFile": true}),
    );
    let out = only_call(&jsonl(&[header(), gem("g", json!(""), vec![c])]))
        .output
        .unwrap();
    let display = &out.data.unwrap()["resultDisplay"];
    assert!(display.get("newContent").is_none() && display.get("originalContent").is_some());
}

#[test]
fn notices_removed_by_the_sync_leave_no_rewind() {
    let notice = json!({"id": "w1", "type": "warning", "content": "careful"});
    let s = jsonl(&[
        header(),
        user("u", "go"),
        notice.clone(),
        gem("g", json!("next"), vec![]),
        json!({"$patch": {"removeIds": ["w1"]}}),
    ]);
    assert!(rewinds(&s).is_empty());
    assert_eq!(ids(&s), ["u", "w1", "g"]);

    // A rewind to a notice that is the tail drops only the notice: nothing was cut.
    let s = jsonl(&[
        header(),
        user("u", "go"),
        notice.clone(),
        json!({"$rewindTo": "w1"}),
    ]);
    assert!(rewinds(&s).is_empty());
    // A rewind that cuts a real turn lists the turn but not the notice after it.
    let s = jsonl(&[
        header(),
        user("u", "go"),
        gem("g", json!("x"), vec![]),
        notice,
        json!({"$rewindTo": "g"}),
    ]);
    assert_eq!(rewinds(&s), vec![vec!["g"]]);
}

#[test]
fn binary_echo_parts_are_not_a_user_message_and_not_duplicated() {
    let image = json!({"inlineData": {"mimeType": "image/png", "data": "aGVsbG8gd29ybGQ="}});
    let c = json!({"id": "t1", "name": "read_file", "args": {"file_path": "/a.png"}, "status": "success", "result": [
        {"functionResponse": {"id": "t1", "name": "read_file", "response": {"output": "Binary content read"}}}, image.clone()]});
    let echo = json!({"id": "e", "type": "user", "content": [{"functionResponse": {"id": "t1", "name": "read_file", "response": {"output": "Binary content read"}}}, image.clone()]});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c]), echo]);
    assert_eq!(ids(&s), ["g"]);
    let out = only_call(&s).output.unwrap();
    assert_eq!(out.attachments.len(), 1);
    assert_eq!(
        (out.attachments[0].mime.as_deref(), out.attachments[0].bytes),
        (Some("image/png"), Some(11))
    );
}

#[test]
fn binary_echo_parts_reach_a_call_without_a_result() {
    let image = json!({"inlineData": {"mimeType": "image/png", "data": "aGVsbG8gd29ybGQ="}});
    let c = json!({"id": "t1", "name": "read_file", "args": {"file_path": "/a.png"}, "status": "success"});
    let echo = json!({"id": "e", "type": "user", "content": [{"functionResponse": {"id": "t1", "name": "read_file", "response": {"output": "Binary content read"}}}, image]});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c]), echo]);
    assert_eq!(ids(&s), ["g"]);
    let out = only_call(&s).output.unwrap();
    assert_eq!(
        (out.text.as_str(), out.attachments.len()),
        ("Binary content read", 1)
    );
}

#[test]
fn binary_parts_of_an_orphan_echo_stay_on_its_result() {
    let image = json!({"inlineData": {"mimeType": "image/png", "data": "aGVsbG8gd29ybGQ="}});
    let echo = json!({"id": "e", "type": "user", "content": [{"functionResponse": {"id": "ghost", "name": "read_file", "response": {"output": "x"}}}, image]});
    let s = jsonl(&[header(), echo]);
    let Part::ToolResult(r) = &message(&s, "e").parts[0] else {
        panic!()
    };
    assert_eq!(r.output.attachments.len(), 1);
}

#[test]
fn info_message_with_binary_parts_keeps_them() {
    let info = json!({"id": "i1", "type": "info", "content": [{"inlineData": {"mimeType": "image/png", "data": "aGVsbG8gd29ybGQ="}}]});
    let s = jsonl(&[header(), info]);
    assert_eq!(ids(&s), ["i1", "i1#media"]);
    let m = message(&s, "i1#media");
    assert_eq!(
        (m.role, m.origin, m.kind.as_deref()),
        (Role::User, Origin::Context, Some("binary_injection"))
    );
    let Part::Attachment(a) = &m.parts[0] else {
        panic!()
    };
    assert_eq!((a.mime.as_deref(), a.bytes), (Some("image/png"), Some(11)));
}

#[test]
fn a_prompt_mentioning_the_snapshot_tag_stays_a_prompt() {
    let open_only = user("p1", "please explain what <state_snapshot> tags are for");
    let both = user(
        "p2",
        "does <state_snapshot> ... </state_snapshot> get sent back to you?",
    );
    let s = jsonl(&[header(), open_only, both]);
    assert!(message(&s, "p1").is_human() && message(&s, "p2").is_human());
    assert!(
        !s.entries
            .iter()
            .any(|e| matches!(e.kind, EntryKind::Compaction(_)))
    );
    assert_eq!(
        s.title.as_deref(),
        Some("please explain what <state_snapshot> tags are for")
    );
}

#[test]
fn a_snapshot_not_at_the_start_counts_when_the_ack_follows() {
    let snap = user(
        "snap",
        "Summary follows.\n<state_snapshot>x</state_snapshot>",
    );
    let ack = gem(
        "ack",
        json!("Got it. Thanks for the additional context!"),
        vec![],
    );
    let s = jsonl(&[header(), snap, ack]);
    assert!(matches!(entry(&s, "snap").kind, EntryKind::Compaction(_)));
}

#[test]
fn a_masked_sibling_does_not_mark_a_call_truncated() {
    // The masking sync writes the whole echo turn into the `result` of every call of a batch.
    let masked = "<tool_output_masked>\n[masked]\n</tool_output_masked>";
    let echo_turn = json!([
        {"functionResponse": {"id": "cA", "name": "read_file", "response": {"output": "AAA full output"}}},
        {"functionResponse": {"id": "cB", "name": "read_file", "response": {"output": masked}}}
    ]);
    let rec = |id: &str, file: &str| json!({"id": id, "name": "read_file", "args": {"file_path": file}, "status": "success", "result": echo_turn});
    let s = jsonl(&[
        header(),
        gem("g", json!(""), vec![rec("cA", "/a"), rec("cB", "/b")]),
    ]);
    let calls = tool_calls_of(&s, "g");
    let a = calls[0].output.as_ref().unwrap();
    assert_eq!((a.text.as_str(), a.truncated), ("AAA full output", false));
    let b = calls[1].output.as_ref().unwrap();
    assert!(b.truncated && b.text.contains("masked"));
}

#[test]
fn parallel_calls_without_ids_pair_by_name_in_order() {
    let content = json!([
        {"functionCall": {"name": "glob", "args": {"pattern": "*.rs"}}},
        {"functionCall": {"name": "glob", "args": {"pattern": "*.ts"}}}
    ]);
    let rec = |id: &str, pattern: &str| {
        call(
            id,
            "glob",
            json!({"pattern": pattern}),
            "success",
            json!({"output": pattern}),
        )
    };
    let s = jsonl(&[
        header(),
        gem("g", content, vec![rec("t1", "*.rs"), rec("t2", "*.ts")]),
    ]);
    let calls = tool_calls_of(&s, "g");
    assert_eq!((calls[0].id.as_str(), calls[1].id.as_str()), ("t1", "t2"));
    assert_eq!(calls[1].output.as_ref().unwrap().text, "*.ts");
}

#[test]
fn echo_turn_with_several_responses_folds_every_call() {
    let rec =
        |id: &str| json!({"id": id, "name": "glob", "args": {"pattern": "*"}, "status": "success"});
    let fr = |id: &str| json!({"functionResponse": {"id": id, "name": "glob", "response": {"output": id}}});
    let echo = json!({"id": "e", "type": "user", "content": [fr("a"), fr("b")]});
    let s = jsonl(&[
        header(),
        gem("g", json!(""), vec![rec("a"), rec("b")]),
        echo,
    ]);
    assert_eq!(ids(&s), ["g"]);
    let calls = tool_calls_of(&s, "g");
    assert_eq!(
        (
            calls[0].output.as_ref().unwrap().text.as_str(),
            calls[1].output.as_ref().unwrap().text.as_str()
        ),
        ("a", "b")
    );
}

#[test]
fn snapshot_entries_without_an_id_are_skipped_with_a_warning() {
    let anon = json!({"type": "user", "content": [{"text": "no id"}]});
    let s = jsonl(&[
        header(),
        json!({"$set": {"messages": [anon.clone(), user("a", "kept")]}}),
        json!({"$set": {"messages": [anon, user("a", "kept")]}}),
    ]);
    assert_eq!(ids(&s), ["a"]);
    assert_eq!(s.warnings.len(), 2);
}

#[test]
fn unmodeled_parts_are_named_by_their_real_key() {
    let content = json!([{"codeExecutionResult": {"outcome": "OK", "output": "3"}}, {"zzz": 1}]);
    let s = jsonl(&[header(), gem("g", content, vec![])]);
    let parts = message(&s, "g").parts;
    assert!(matches!(&parts[0], Part::Other { kind, .. } if kind == "codeExecutionResult"));
    assert!(matches!(&parts[1], Part::Other { kind, .. } if kind == "unknown"));
}

#[test]
fn sibling_text_before_the_function_response_is_kept() {
    let c = json!({"id": "t", "name": "glob", "args": {"pattern": "*"}, "status": "success", "result": [
        "note", {"functionResponse": {"id": "t", "name": "glob", "response": {"output": "found"}}}]});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c])]);
    assert_eq!(only_call(&s).output.unwrap().text, "note\nfound");
}

#[test]
fn many_bare_records_reduce_in_linear_time() {
    // The live list is a mirror set: 40k records (each written twice) must not take seconds.
    let mut lines = vec![header().to_string()];
    for i in 0..40_000 {
        let line = user(&format!("m{i}"), "x").to_string();
        lines.push(line.clone());
        lines.push(line);
    }
    let started = std::time::Instant::now();
    let s = parse_text(&lines.join("\n"));
    assert_eq!(s.entries.len(), 40_000);
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

#[test]
fn pending_call_keeps_its_metadata_without_a_result() {
    let c = json!({"id": "t", "name": "glob", "args": {"pattern": "*"}, "status": "executing", "result": null,
        "displayName": "FindFiles", "description": "*"});
    let s = jsonl(&[header(), gem("g", json!(""), vec![c])]);
    let call = only_call(&s);
    assert_eq!(call.status, ToolStatus::Pending);
    let out = call.output.unwrap();
    assert_eq!(out.text, "");
    assert_eq!(out.data.unwrap()["displayName"], "FindFiles");
    // Nothing to keep: still no output at all.
    let bare = json!({"id": "t", "name": "glob", "args": {"pattern": "*"}, "status": "executing"});
    assert!(
        only_call(&jsonl(&[header(), gem("g", json!(""), vec![bare])]))
            .output
            .is_none()
    );
}

#[test]
fn shell_without_a_directory_runs_in_the_session_directory() {
    let ctx = user(
        "c",
        "<session_context>\n- **Workspace Directories:**\n  - /w/proj\n</session_context>",
    );
    let named = call(
        "t1",
        "run_shell_command",
        json!({"command": "ls", "dir_path": "/w/other"}),
        "success",
        json!({"output": "Output: x"}),
    );
    let bare = call(
        "t2",
        "run_shell_command",
        json!({"command": "pwd"}),
        "success",
        json!({"output": "Output: x"}),
    );
    let s = jsonl(&[header(), ctx, gem("g", json!(""), vec![named, bare])]);
    let cwds: Vec<_> = s
        .tool_calls()
        .iter()
        .map(|c| match &c.tool {
            Tool::Shell(sh) => sh.cwd.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(
        cwds,
        [Some("/w/other".to_string()), Some("/w/proj".to_string())]
    );
}
