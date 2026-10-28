//! Claude Code parser against the facts in `tests/fixtures/claude/README.md`.

use agentparse::{
    Agent, Entry, EntryKind, FileOp, Message, Origin, Part, ProcessAction, Role, Session,
    ShellResult, Source, TodoItem, Tool, ToolCall, ToolStatus, Usage, parse_file, parse_file_tail,
    parse_reader,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MAIN_ID: &str = "3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11";
const SUB: &str = "main-session/subagents/agent-a1b2c3d4e5f607182.jsonl";
const FIXTURES: [&str; 4] = [
    "main-session.jsonl",
    SUB,
    "old-format.jsonl",
    "fork-and-corrupt.jsonl",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude")
        .join(name)
}

fn parse(name: &str) -> Session {
    parse_file(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn usage(input: u64, output: u64, reasoning: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input,
        output,
        reasoning,
        cache_read,
        cache_write,
        cost_usd: None,
    }
}

/// The first non-`Settings` entry that starts at this line.
fn at(s: &Session, line: usize) -> &Entry {
    s.entries
        .iter()
        .find(|e| e.line == Some(line) && !matches!(e.kind, EntryKind::Settings(_)))
        .unwrap_or_else(|| panic!("no entry at line {line}"))
}

fn message(e: &Entry) -> &Message {
    match &e.kind {
        EntryKind::Message(m) => m,
        other => panic!("not a message: {other:?}"),
    }
}

fn call<'a>(s: &'a Session, id: &str) -> &'a ToolCall {
    s.tool_calls()
        .into_iter()
        .find(|c| c.id.starts_with(id))
        .unwrap_or_else(|| panic!("no call {id}"))
}

fn shell(c: &ToolCall) -> &ShellResult {
    c.output
        .as_ref()
        .and_then(|o| o.shell.as_ref())
        .unwrap_or_else(|| panic!("{} has no shell result", c.id))
}

fn assistants(s: &Session) -> Vec<(&Entry, &Message)> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) if m.role == Role::Assistant => Some((e, m)),
            _ => None,
        })
        .collect()
}

fn user_texts(s: &Session, origin: Origin) -> Vec<String> {
    s.messages()
        .filter(|m| m.role != Role::Assistant && m.origin == origin)
        .map(Message::text)
        .collect()
}

fn other_kinds(s: &Session) -> Vec<&str> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Other(o) => Some(o.kind.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn main_session_identity() {
    let s = parse("main-session.jsonl");
    assert_eq!(s.agent, Agent::Claude);
    assert_eq!(s.id, MAIN_ID);
    assert_eq!(s.format, "claude-jsonl");
    assert_eq!(s.parent, None);
    assert_eq!(s.cwd.as_deref(), Some("/work/demo"));
    assert_eq!(
        s.git.as_ref().and_then(|g| g.branch.as_deref()),
        Some("main")
    );
    assert_eq!(s.cli_version.as_deref(), Some("2.1.296"));
    assert_eq!(s.models, ["claude-sonnet-5-5"]);
    assert_eq!(s.title.as_deref(), Some("demo-fix-session"));
    assert_eq!(
        s.extra.as_ref().unwrap()["ai-title"],
        "Fix the failing demo test"
    );
    assert_eq!(s.started_at, Some(1_791_795_600_100)); // 2026-10-12T09:00:00.100Z
    assert_eq!(s.updated_at, Some(1_791_795_659_200)); // 2026-10-12T09:00:59.200Z
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

#[test]
fn main_session_usage_per_message_and_total() {
    let s = parse("main-session.jsonl");
    #[rustfmt::skip]
    let table: [(&str, &[usize], Option<&str>, Usage); 23] = [
        ("01", &[8, 9, 10], Some("tool_use"), usage(6, 118, 40, 9991, 1200)),
        ("02", &[14], Some("tool_use"), usage(2, 87, 0, 10191, 1431)),
        ("03", &[16, 17], Some("tool_use"), usage(2, 118, 0, 11622, 136)),
        ("04", &[19], Some("tool_use"), usage(2, 171, 0, 11758, 161)),
        ("05", &[22], Some("tool_use"), usage(2, 90, 0, 11919, 150)),
        ("06", &[24], Some("tool_use"), usage(2, 140, 0, 12069, 120)),
        ("07", &[26, 27], Some("tool_use"), usage(2, 160, 0, 12189, 210)),
        ("08", &[30], Some("end_turn"), usage(2, 69, 0, 12399, 98)),
        ("09", &[34], Some("tool_use"), usage(2, 95, 0, 12600, 300)),
        ("10", &[37], Some("tool_use"), usage(2, 72, 0, 12900, 180)),
        ("11", &[39], Some("tool_use"), usage(2, 64, 0, 13100, 90)),
        ("12", &[41], Some("end_turn"), usage(2, 55, 0, 13300, 60)),
        ("13", &[45], Some("tool_use"), usage(2, 80, 0, 13400, 120)),
        ("14", &[47], Some("tool_use"), usage(2, 60, 0, 13500, 90)),
        ("15", &[49], Some("tool_use"), usage(2, 110, 0, 13600, 140)),
        ("16", &[51], Some("tool_use"), usage(2, 98, 0, 13750, 110)),
        ("17", &[53], Some("end_turn"), usage(2, 40, 0, 13860, 50)),
        ("18", &[57], Some("tool_use"), usage(2, 66, 0, 13950, 700)),
        ("19", &[59], Some("end_turn"), usage(2, 52, 0, 14650, 60)),
        ("20", &[61, 62], Some("tool_use"), usage(2, 75, 0, 14710, 130)),
        ("21", &[65], Some("tool_use"), usage(2, 70, 0, 14850, 100)),
        ("22", &[69, 70, 71], None, usage(2, 1, 0, 15010, 90)),
        ("23", &[79], Some("end_turn"), usage(2, 45, 0, 15100, 70)),
    ];
    let got = assistants(&s);
    assert_eq!(got.len(), 23);
    for ((entry, m), (n, lines, stop, u)) in got.iter().zip(table) {
        let id = format!("msg_01DemoMain{n}");
        assert!(
            entry.id.as_deref().unwrap().starts_with(&id),
            "{:?} vs {id}",
            entry.id
        );
        assert_eq!(entry.line, Some(lines[0]), "{id}");
        // The assistant lines, then the result lines folded into the message.
        let uuids = entry.extra.as_ref().unwrap()["uuids"]
            .as_array()
            .unwrap()
            .len();
        assert!(uuids >= lines.len(), "{id}");
        assert_eq!(m.origin, Origin::Model);
        assert_eq!(m.model.as_deref(), Some("claude-sonnet-5-5"), "{id}");
        assert_eq!(m.stop_reason.as_deref(), stop, "{id}");
        assert_eq!(m.usage, Some(u), "{id}");
    }
    // The 5m/1h split stays in extra.
    let first = got[0].0.extra.as_ref().unwrap();
    assert_eq!(
        first["message"]["usage"]["cache_creation"]["ephemeral_5m_input_tokens"],
        200
    );
    assert_eq!(s.usage, usage(50, 1936, 40, 300_418, 5796));
    // Message 1: thinking (signature only), text, tool_use, in that order.
    let parts = &got[0].1.parts;
    assert!(matches!(
        parts[0],
        Part::Reasoning {
            text: None,
            encrypted: true
        }
    ));
    assert!(matches!(parts[1], Part::Text { .. }));
    assert!(matches!(parts[2], Part::ToolCall(_)));
}

#[test]
fn main_session_user_side() {
    let s = parse("main-session.jsonl");
    let human: Vec<String> = s
        .messages()
        .filter(|m| m.is_human())
        .map(Message::text)
        .collect();
    assert_eq!(
        human,
        [
            "fix the failing test in @test.sh",
            "summarize the demo docs",
            "and track it as a task",
            "clean the build directory",
            "leave it, just run cargo build",
            "run cargo test instead",
            "continue with the failing test",
        ]
    );
    assert!(
        s.messages().all(|m| m.phase.is_none()),
        "the one queued text was dequeued"
    );
    // Slash commands, their output and the `!` shell escape.
    let commands: Vec<Option<&str>> = s
        .messages()
        .filter(|m| m.origin == Origin::Command)
        .map(|m| m.kind.as_deref())
        .collect();
    assert_eq!(
        commands,
        [
            Some("command-name"),
            Some("local-command-stdout"),
            Some("command-name"),
            Some("bash-input")
        ]
    );
    // Meta lines and injected attachments the model saw.
    let context: Vec<(Role, Option<&str>)> = s
        .messages()
        .filter(|m| m.origin == Origin::Context)
        .map(|m| (m.role, m.kind.as_deref()))
        .collect();
    assert_eq!(
        context,
        [
            (Role::System, Some("total_tokens_reminder")),
            (Role::User, Some("local-command-caveat")),
            (Role::User, Some("meta")),
            (Role::User, Some("system-reminder")),
        ]
    );
    let notes = user_texts(&s, Origin::Notification);
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().all(|t| t.starts_with("<task-notification>")));
    // The file mention and the compaction's file reference are one line each; the record keeps
    // its metadata and the body once (the rendered copy that wraps it goes).
    let EntryKind::Notice(n) = &at(&s, 4).kind else {
        panic!()
    };
    assert_eq!(
        (n.kind.as_deref(), n.text.as_str()),
        (Some("file"), "Read test.sh (4 lines)")
    );
    let file = at(&s, 4).extra.as_ref().unwrap();
    assert_eq!(file["attachment"]["displayPath"], "test.sh");
    assert!(file["attachment"]["content"]["file"]["content"].is_string());
    assert!(file.get("rendered").is_none());
    let EntryKind::Notice(n) = &at(&s, 85).kind else {
        panic!()
    };
    assert_eq!(
        (n.kind.as_deref(), n.text.as_str()),
        (Some("compact_file_reference"), "Referenced file a.txt")
    );
    // No user message holds a tool result: all 18 folded into their calls.
    assert!(s.messages().all(|m| m.origin != Origin::Tool));
}

#[test]
fn main_session_tool_calls() {
    let s = parse("main-session.jsonl");
    use ToolStatus::*;
    #[rustfmt::skip]
    let table = [
        // A plain success object means exit 0 (the CLI turns any other code into an error).
        ("toolu_01DemoBash01", "Bash", Ok, None, Some(0)),
        ("toolu_01DemoBash02", "Bash", Error, None, Some(3)),
        ("toolu_01DemoRead03", "Read", Ok, None, None),
        ("toolu_01DemoEdit04", "Edit", Ok, None, None),
        ("toolu_01DemoBash05", "Bash", Error, None, Some(143)),
        ("toolu_01DemoWrite06", "Write", Ok, None, None),
        ("toolu_01DemoGrep07", "Grep", Ok, None, None),
        ("toolu_01DemoGlob08", "Glob", Ok, None, None),
        ("toolu_01DemoWebFetch09", "WebFetch", Ok, None, None),
        ("toolu_01DemoWebSearch10", "WebSearch", Ok, None, None),
        ("toolu_01Demomcpdemosearch11", "mcp__demo__search", Ok, None, None),
        ("toolu_01DemoTaskCreate12", "TaskCreate", Ok, None, None),
        ("toolu_01DemoTaskUpdate13", "TaskUpdate", Ok, None, None),
        ("toolu_01DemoAgent14", "Agent", Ok, Some("completed"), None),
        ("toolu_01DemoBash15", "Bash", Ok, Some("completed"), Some(0)),
        ("toolu_01DemoTaskOutput16", "TaskOutput", Ok, None, Some(0)),
        ("toolu_01DemoBash17", "Bash", Rejected, Some("user-rejected"), None),
        ("toolu_01DemoBash18", "Bash", Interrupted, Some("interrupted"), None),
        // No result, but the conversation went on after it.
        ("toolu_01DemoBash19", "Bash", Interrupted, None, None),
    ];
    let calls: Vec<&ToolCall> = s
        .tool_calls()
        .into_iter()
        .filter(|c| c.id.starts_with("toolu_"))
        .collect();
    assert_eq!(calls.len(), table.len());
    for (c, (id, name, status, detail, exit)) in calls.iter().zip(table) {
        assert!(c.id.starts_with(id), "{} vs {id}", c.id);
        assert_eq!(c.name, name, "{id}");
        assert_eq!(c.status, status, "{id}");
        assert_eq!(c.status_detail.as_deref(), detail, "{id}");
        let code = c
            .output
            .as_ref()
            .and_then(|o| o.shell.as_ref())
            .and_then(|s| s.exit_code);
        assert_eq!(code, exit, "{id}");
        assert!(c.started_at.is_some(), "{id}");
        assert_eq!(c.ended_at.is_some(), c.output.is_some(), "{id}");
    }

    let ls = call(&s, "toolu_01DemoBash01");
    let Tool::Shell(sh) = &ls.tool else { panic!() };
    assert_eq!(
        (sh.command.as_str(), sh.cwd.as_deref(), sh.background),
        ("ls -la", Some("/work/demo"), false)
    );
    assert_eq!(
        shell(ls).stdout.as_deref().map(|o| o.lines().count()),
        Some(3)
    );
    assert_eq!(shell(ls).stderr, None);

    let failed = call(&s, "toolu_01DemoBash02");
    assert_eq!(shell(failed).stdout.as_deref(), Some("out-line\nerr-line"));
    assert_eq!(shell(failed).stderr, None, "one merged stream");
    assert_eq!(
        failed.output.as_ref().unwrap().data,
        None,
        "the string result repeats the text"
    );

    let read = call(&s, "toolu_01DemoRead03");
    assert_eq!(
        read.tool,
        Tool::Read {
            path: "/work/demo/a.txt".into(),
            offset: None,
            limit: None
        }
    );
    let read_out = read.output.as_ref().unwrap();
    assert_eq!(read_out.text, "1\tone\n2\ttwo\n3\tthree\n4\t");
    assert_eq!(read_out.data.as_ref().unwrap()["file"]["totalLines"], 4);
    assert!(
        read_out.data.as_ref().unwrap()["file"]
            .get("content")
            .is_none()
    );

    let edit = call(&s, "toolu_01DemoEdit04");
    let Tool::Edit { path, edits } = &edit.tool else {
        panic!()
    };
    assert_eq!(
        (path.as_str(), edits[0].old.as_str(), edits[0].new.as_str()),
        ("/work/demo/a.txt", "two\nthree", "two edited\nthree")
    );
    let change = &edit.output.as_ref().unwrap().files[0];
    assert_eq!(
        (change.op, change.additions, change.deletions),
        (FileOp::Update, Some(1), Some(1))
    );
    assert_eq!(
        change.diff.as_deref(),
        Some("@@ -1,3 +1,3 @@\n one\n-two\n+two edited\n three\n")
    );

    let timeout = call(&s, "toolu_01DemoBash05");
    assert!(shell(timeout).timed_out);
    assert!(matches!(&timeout.tool, Tool::Shell(sh) if sh.timeout_ms == Some(120_000)));

    let write = call(&s, "toolu_01DemoWrite06");
    let created = &write.output.as_ref().unwrap().files[0];
    assert_eq!(
        (created.path.as_str(), created.op, created.diff.as_deref()),
        ("/work/demo/notes.md", FileOp::Add, None)
    );

    assert!(
        matches!(&call(&s, "toolu_01DemoGrep07").tool, Tool::Search { pattern, .. } if pattern == "exit")
    );
    assert!(
        matches!(&call(&s, "toolu_01DemoGlob08").tool, Tool::Glob { pattern, .. } if pattern == "**/*.sh")
    );
    assert!(
        matches!(&call(&s, "toolu_01DemoWebFetch09").tool, Tool::WebFetch { url: Some(u), .. } if u == "https://example.com/docs")
    );
    assert!(matches!(
        &call(&s, "toolu_01DemoWebSearch10").tool,
        Tool::WebSearch { query: Some(_) }
    ));

    let mcp = call(&s, "toolu_01Demomcpdemosearch11");
    assert_eq!(
        mcp.tool,
        Tool::Mcp {
            server: "demo".into(),
            tool: "search".into()
        }
    );
    assert_eq!(
        mcp.output.as_ref().unwrap().data,
        None,
        "the list result equals the content"
    );
    assert_eq!(
        call(&s, "toolu_01DemoTaskCreate12").tool,
        Tool::Todo {
            items: vec![TodoItem {
                content: "Check demo docs".into(),
                status: Some("pending".into()),
                priority: None
            }]
        }
    );

    let agent = call(&s, "toolu_01DemoAgent14");
    assert!(
        matches!(&agent.tool, Tool::Task { agent_type: Some(t), .. } if t == "general-purpose")
    );
    assert_eq!(agent.child_session.as_deref(), Some("a1b2c3d4e5f607182"));

    let bg = call(&s, "toolu_01DemoBash15");
    assert!(matches!(&bg.tool, Tool::Shell(sh) if sh.background));
    assert_eq!(shell(bg).background_id.as_deref(), Some("bk7d2m9qa"));
    assert!(
        bg.output
            .as_ref()
            .unwrap()
            .full_output_path
            .as_deref()
            .unwrap()
            .ends_with("tasks/bk7d2m9qa.output")
    );
    assert_eq!(
        bg.ended_at,
        at(&s, 56).ts,
        "finished when the notification came"
    );

    let poll = call(&s, "toolu_01DemoTaskOutput16");
    assert_eq!(
        poll.tool,
        Tool::Process {
            id: Some("bk7d2m9qa".into()),
            action: ProcessAction::Read,
            input: None
        }
    );
    assert_eq!(
        call(&s, "toolu_01DemoBash17")
            .output
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        "User rejected tool use"
    );
    assert!(shell(call(&s, "toolu_01DemoBash18")).interrupted);
    assert_eq!(call(&s, "toolu_01DemoBash19").output, None);

    // The polled background shell reports its own output and exit code.
    let polled = shell(call(&s, "toolu_01DemoTaskOutput16"));
    assert_eq!(polled.stdout.as_deref(), Some("bg-done\n"));

    // Result lines are absorbed into the message holding the call; their own keys go with them.
    let first = &assistants(&s)[0].0;
    let calls = &first.extra.as_ref().unwrap()["calls"]["toolu_01DemoBash01xxxxxxxxxxxx"];
    assert_eq!(calls["output"]["permissionDecision"]["decision"], "accept");
    assert!(
        calls.get("call").is_none(),
        "`caller: direct` is no leftover"
    );
}

#[test]
fn main_session_events() {
    let s = parse("main-session.jsonl");
    let msg1 = "msg_01DemoMain01xxxxxxxxxxxxxx";
    // Graph: the hook (L11) and the reminder after the result (L13) both hang off message 1.
    assert_eq!(at(&s, 11).parent_id.as_deref(), Some(msg1));
    assert_eq!(at(&s, 13).parent_id.as_deref(), Some(msg1));
    let hooks: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Hook(h) => Some((
                e.line,
                h.event.as_deref(),
                h.exit_code,
                h.duration_ms,
                h.blocked,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        hooks,
        [
            (Some(2), Some("SessionStart"), Some(0), Some(120), false),
            (Some(11), Some("PreToolUse"), Some(0), Some(49), false),
            (Some(31), Some("Stop"), None, Some(404), false),
        ]
    );
    let EntryKind::Hook(pre) = &at(&s, 11).kind else {
        panic!()
    };
    assert_eq!(
        pre.command.as_deref(),
        Some("/work/demo/.claude/hooks/pre-bash.sh")
    );
    assert_eq!(
        at(&s, 11).extra.as_ref().unwrap()["attachment"]["toolUseID"],
        "toolu_01DemoBash01xxxxxxxxxxxx"
    );

    let turns: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Turn(t) => Some((e.line.unwrap(), t.status.as_deref(), t.duration_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(
        turns,
        [
            (32, None, Some(41_250)),
            (43, None, Some(18_800)),
            (54, None, Some(9100)),
            (67, Some("interrupted"), None),
            (72, Some("interrupted"), None)
        ]
    );
    let marker = at(&s, 67).extra.as_ref().unwrap();
    assert_eq!(
        marker["interruptedMessageId"],
        "msg_01DemoMain21xxxxxxxxxxxxxx"
    );
    assert_eq!(
        marker["marker"],
        "[Request interrupted by user for tool use]"
    );

    let EntryKind::Compaction(c) = &at(&s, 83).kind else {
        panic!()
    };
    assert_eq!(c.trigger.as_deref(), Some("auto"));
    assert_eq!(
        (c.tokens_before, c.tokens_after),
        (Some(177_627), Some(3771))
    );
    assert_eq!(
        c.logical_parent.as_deref(),
        Some("c0de0000-0a0a-4b0b-8c0c-000000000058")
    );
    assert!(
        c.summary
            .as_deref()
            .unwrap()
            .starts_with("This session is being continued from a previous conversation")
    );
    assert_eq!(at(&s, 83).parent_id, None);
    assert!(
        s.entries.iter().all(|e| e.line != Some(84)),
        "the summary line is absorbed"
    );
    assert_eq!(
        at(&s, 85).parent_id,
        at(&s, 83).id,
        "children of the summary resolve to the compaction"
    );

    let EntryKind::Notice(n) = &at(&s, 82).kind else {
        panic!()
    };
    assert_eq!(
        (n.level.as_deref(), n.kind.as_deref()),
        (Some("notice"), Some("informational"))
    );
    let EntryKind::Error(api) = &at(&s, 87).kind else {
        panic!()
    };
    assert_eq!(
        (api.message.as_str(), api.status, api.retry),
        ("529 Overloaded", Some(529), Some(1))
    );
    let EntryKind::Error(limit) = &at(&s, 88).kind else {
        panic!()
    };
    assert_eq!(
        (limit.kind.as_deref(), limit.status),
        (Some("rate_limit"), Some(429))
    );
    assert!(limit.message.contains("session limit"));

    let settings: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Settings(x) => {
                Some((x.approval.as_deref(), x.effort.as_deref(), x.cwd.as_deref()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        settings,
        [
            (Some("default"), None, None),
            (None, None, Some("/work/demo")),
            (None, Some("medium"), None)
        ]
    );

    let mut others = other_kinds(&s);
    others.sort_unstable();
    assert_eq!(
        others,
        [
            "atis-latch",
            "file-history-snapshot",
            "last-prompt",
            "last-prompt",
            "queue-operation",
            "queue-operation"
        ]
    );

    // `!echo hi`: one Command message holding a Shell call and its output.
    let escape = message(at(&s, 80));
    let Part::ToolCall(c) = &escape.parts[0] else {
        panic!()
    };
    assert_eq!(
        Some(c.id.as_str()),
        at(&s, 80).id.as_deref(),
        "the <bash-input> line uuid"
    );
    assert_eq!(
        c.tool,
        Tool::Shell(agentparse::Shell {
            command: "echo hi".into(),
            cwd: Some("/work/demo".into()),
            ..Default::default()
        })
    );
    assert_eq!(
        (
            c.status,
            shell(c).stdout.as_deref(),
            shell(c).stderr.as_deref()
        ),
        (ToolStatus::Ok, Some("hi\n"), None)
    );
    assert!(
        s.entries.iter().all(|e| e.line != Some(81)),
        "the output line is absorbed"
    );
}

#[test]
fn subagent_file() {
    let s = parse(SUB);
    assert_eq!(s.id, "a1b2c3d4e5f607182");
    let parent = s.parent.as_ref().unwrap();
    assert_eq!(parent.session_id, MAIN_ID);
    assert_eq!(
        parent.call_id.as_deref(),
        Some("toolu_01DemoAgent14xxxxxxxxxxx")
    );
    assert_eq!(parent.agent_type.as_deref(), Some("general-purpose"));
    assert_eq!(s.title.as_deref(), Some("Survey demo tests"));
    assert_eq!(s.extra.as_ref().unwrap()["meta"]["spawnDepth"], 1);
    assert_eq!(
        user_texts(&s, Origin::Agent),
        ["List the test files in /work/demo and report how many there are."]
    );
    assert_eq!(
        s.messages().filter(|m| m.is_human()).count(),
        0,
        "a subagent prompt is no human"
    );
    let got = assistants(&s);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].1.usage, Some(usage(4, 95, 20, 5000, 300)));
    assert_eq!(got[1].1.usage, Some(usage(2, 30, 0, 5300, 60)));
    assert_eq!(got[1].1.text(), "The demo project has 3 test files.");
    assert_eq!(s.usage, usage(6, 125, 20, 10_300, 360));
    let c = call(&s, "toolu_01DemoSubBash01");
    assert_eq!((c.status, shell(c).exit_code), (ToolStatus::Ok, Some(0)));
    assert_eq!(
        shell(c).stdout.as_deref(),
        Some("a_test.rs\nb_test.rs\nc_test.rs")
    );
    assert!(s.warnings.is_empty());
}

#[test]
fn old_format() {
    let s = parse("old-format.jsonl");
    assert_eq!(s.id, "9d41c7a2-5e3b-4f60-8a17-2c9b6d0e4f35");
    assert_eq!(s.cli_version.as_deref(), Some("2.0.31"));
    // Both summaries point at leaves of earlier sessions, so the first prompt titles this one.
    assert_eq!(s.title.as_deref(), Some("fix the failing test"));
    let mut others = other_kinds(&s);
    others.sort_unstable();
    assert_eq!(others, ["progress", "progress", "summary", "summary"]);
    assert_eq!(
        s.messages()
            .filter(|m| m.is_human())
            .map(Message::text)
            .collect::<Vec<_>>(),
        ["fix the failing test"]
    );
    assert_eq!(
        user_texts(&s, Origin::Agent),
        ["Count the test files in /work/demo."],
        "inline sidechain prompt"
    );
    let got = assistants(&s);
    assert_eq!(got.len(), 6);
    assert_eq!(
        got.iter().map(|(e, _)| e.line.unwrap()).collect::<Vec<_>>(),
        [4, 7, 12, 14, 17, 20]
    );
    assert_eq!(
        got[1].1.usage,
        Some(usage(6, 55, 0, 5000, 300)),
        "last line of the two"
    );
    assert_eq!(got[4].0.extra.as_ref().unwrap()["isSidechain"], true);
    assert_eq!(s.usage, usage(25, 417, 0, 21_700, 6850));

    let todo = call(&s, "toolu_01OldDemoTodo");
    let Tool::Todo { items } = &todo.tool else {
        panic!()
    };
    assert_eq!(
        (
            items[0].content.as_str(),
            items[0].status.as_deref(),
            items[0].priority.as_deref()
        ),
        ("Fix the failing test", Some("in_progress"), Some("high"))
    );
    let ls = call(&s, "toolu_01OldDemoLS");
    assert_eq!(
        (ls.tool.clone(), ls.output.as_ref().unwrap().data.clone()),
        (
            Tool::List {
                path: Some("/work/demo".into())
            },
            None
        )
    );
    let bash = call(&s, "toolu_01OldDemoBash");
    assert_eq!(bash.status, ToolStatus::Error);
    assert_eq!(
        (
            shell(bash).exit_code,
            shell(bash).stdout.as_deref(),
            shell(bash).stderr.as_deref()
        ),
        (Some(3), Some("out-line"), Some("err-line"))
    );
    assert_eq!(call(&s, "toolu_01OldDemoRead").status, ToolStatus::Ok);
    let multi = call(&s, "toolu_01OldDemoMulti");
    assert!(matches!(&multi.tool, Tool::Edit { edits, .. } if edits.len() == 2));
    assert_eq!(multi.output.as_ref().unwrap().files[0].op, FileOp::Update);
    let task = call(&s, "toolu_01OldDemoTask");
    assert_eq!(
        task.output.as_ref().unwrap().text,
        "There are 3 test files."
    );
    assert_eq!(task.child_session, None, "no agent id in this era");
    let EntryKind::Notice(n) = &at(&s, 19).kind else {
        panic!()
    };
    assert_eq!(
        (n.kind.as_deref(), n.level.as_deref()),
        (None, Some("info"))
    );
    assert!(s.warnings.is_empty());
}

#[test]
fn fork_and_corrupt_lines() {
    let s = parse("fork-and-corrupt.jsonl");
    let lines: Vec<Option<usize>> = s.warnings.iter().map(|w| w.line).collect();
    assert_eq!(lines, [Some(10), Some(12)]);
    assert_eq!(s.messages().filter(|m| m.is_human()).count(), 4);
    let got = assistants(&s);
    assert_eq!(got.len(), 5);
    // Both branches stay, in file order; L5 forks off L2 like L3 does.
    assert_eq!(at(&s, 3).parent_id, at(&s, 2).id);
    assert_eq!(at(&s, 5).parent_id, at(&s, 2).id);
    assert!(
        at(&s, 7).ts < at(&s, 6).ts,
        "timestamps go backwards, order does not"
    );
    assert_eq!(
        at(&s, 8).parent_id.as_deref(),
        Some("f0f00000-0a0a-4b0b-8c0c-00000000dead"),
        "dangling parent kept"
    );
    assert_eq!(at(&s, 11).parent_id, at(&s, 8).id);
    assert_eq!(s.usage, usage(15, 95, 0, 10_000, 500));
}

#[test]
fn json_round_trip() {
    for name in FIXTURES {
        let s = parse(name);
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s, "{name}");
    }
}

#[test]
fn no_base64_or_signature_survives() {
    for name in FIXTURES {
        let json = serde_json::to_string(&parse(name)).unwrap();
        assert!(!json.contains("\"signature\":\""), "{name}");
        assert!(!json.contains(";base64,"), "{name}");
    }
}

#[test]
fn tail_read_drops_the_torn_first_line() {
    let path = fixture("main-session.jsonl");
    let len = std::fs::metadata(&path).unwrap().len();
    for max in [500, 4000, len / 2, len - 1] {
        let s = parse_file_tail(&path, max).unwrap();
        assert_eq!(s.id, MAIN_ID);
        assert!(
            s.warnings
                .iter()
                .all(|w| !w.message.starts_with("invalid JSON")),
            "{max}: {:?}",
            s.warnings
        );
        assert!(s.entries.iter().all(|e| e.line.is_some_and(|l| l >= 1)));
    }
    // A window that starts after the calls keeps their results as orphans, with a warning.
    let s = parse_file_tail(&path, len / 2).unwrap();
    let orphans = s.messages().filter(|m| m.origin == Origin::Tool).count();
    assert_eq!(orphans, s.warnings.len());
}

#[test]
fn bad_bytes_become_warnings() {
    let good =
        r#"{"type":"user","uuid":"u1","sessionId":"s1","message":{"role":"user","content":"hi"}}"#;
    let mut input = Vec::new();
    input.extend_from_slice(good.as_bytes());
    input.extend_from_slice(b"\n\xff\xfe not json\n");
    input.extend_from_slice(
        br#"{"type":"user","uuid":"u2","message":{"role":"user","content":"caf"#,
    );
    input.extend_from_slice(b"\xe9\"}}\n");
    input.extend_from_slice(
        br#"{"type":"user","uuid":"u3","message":{"role":"user","content":"torn"#,
    );
    let source = Source::File {
        path: "s1.jsonl".into(),
    };
    let s = parse_reader(Agent::Claude, &input[..], source).unwrap();
    assert_eq!(s.id, "s1");
    assert_eq!(
        s.warnings.iter().map(|w| w.line).collect::<Vec<_>>(),
        [Some(2), Some(4)]
    );
    let texts: Vec<String> = s.messages().map(Message::text).collect();
    assert_eq!(texts, ["hi", "caf\u{fffd}"]);
}

#[test]
fn queued_input_left_in_the_queue_is_a_human_message() {
    let lines = [
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-12T09:00:00.000Z","sessionId":"s1","content":"folded into the turn"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-12T09:00:01.000Z","sessionId":"s1","content":"also check the docs"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-12T09:00:02.000Z","sessionId":"s1","content":"<task-notification>x</task-notification>"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-12T09:00:03.000Z","sessionId":"s1","content":"dropped"}"#,
        r#"{"type":"queue-operation","operation":"remove","timestamp":"2026-10-12T09:00:04.000Z","sessionId":"s1","content":"dropped"}"#,
        // No content: the front of the queue.
        r#"{"type":"queue-operation","operation":"remove","timestamp":"2026-10-12T09:00:05.000Z","sessionId":"s1"}"#,
    ];
    let s = parse_reader(
        Agent::Claude,
        lines.join("\n").as_bytes(),
        Source::File {
            path: "s1.jsonl".into(),
        },
    )
    .unwrap();
    let queued: Vec<(&Entry, &Message)> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) => Some((e, m)),
            _ => None,
        })
        .collect();
    assert_eq!(queued.len(), 1);
    let (entry, m) = queued[0];
    assert!(m.is_human());
    assert_eq!(
        (m.phase.as_deref(), m.text().as_str(), entry.line),
        (Some("queued"), "also check the docs", Some(2))
    );
    assert!(
        matches!(s.entries[2].kind, EntryKind::Message(_)),
        "right after the enqueue record"
    );
}

#[test]
fn workflow_journal_is_not_a_session() {
    let journal = r#"{"type":"started","key":"v2:abc","agentId":"a1"}
{"type":"result","key":"v2:abc","agentId":"a1","result":{"ok":true}}"#;
    let err = parse_reader(
        Agent::Claude,
        journal.as_bytes(),
        Source::File {
            path: "journal.jsonl".into(),
        },
    );
    assert!(matches!(err, Err(agentparse::Error::NotASession(_))));
}

/// Plan v2.1 rules that the fixtures do not reach.
#[test]
fn v2_1_rules() {
    let lines = [
        r#"{"type":"user","uuid":"u1","parentUuid":null,"timestamp":"2026-10-12T09:00:00.000Z","sessionId":"s1","cwd":"/w","message":{"role":"user","content":"start the server"}}"#,
        r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-10-12T09:00:01.000Z","sessionId":"s1","cwd":"/w","message":{"id":"msg_1","model":"m","role":"assistant","content":[{"type":"tool_use","id":"toolu_bg","name":"Bash","input":{"command":"serve","run_in_background":true}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":2}}}"#,
        r#"{"type":"user","uuid":"r1","parentUuid":"a1","timestamp":"2026-10-12T09:00:02.000Z","sessionId":"s1","cwd":"/w","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_bg","content":"Command running in background with ID: b1.","is_error":false}]},"toolUseResult":{"stdout":"","stderr":"","interrupted":false,"backgroundTaskId":"b1"}}"#,
        r#"{"type":"assistant","uuid":"a2","parentUuid":"r1","timestamp":"2026-10-12T09:00:03.000Z","sessionId":"s1","cwd":"/w","message":{"id":"msg_2","model":"m","role":"assistant","content":[{"type":"text","text":"started"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        r#"{"type":"assistant","uuid":"x1","parentUuid":"a2","timestamp":"2026-10-12T09:00:04.000Z","sessionId":"s1","cwd":"/w","isApiErrorMessage":true,"error":"rate_limit","apiErrorStatus":429,"message":{"id":"x","model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"API Error: limit"}]}}"#,
        r#"{"type":"assistant","uuid":"x2","parentUuid":"x1","timestamp":"2026-10-12T09:00:05.000Z","sessionId":"s1","cwd":"/w","message":{"id":"y","model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"No response requested."}]}}"#,
        r#"{"type":"system","subtype":"away_summary","uuid":"w1","parentUuid":"x2","timestamp":"2026-10-12T09:00:06.000Z","sessionId":"s1","cwd":"/w","content":"You started the server.","isMeta":false}"#,
        r#"{"type":"system","subtype":"compact_boundary","uuid":"c1","parentUuid":null,"logicalParentUuid":"a2","timestamp":"2026-10-12T09:00:07.000Z","sessionId":"s1","cwd":"/w","content":"Conversation compacted","compactMetadata":{"trigger":"manual","preTokens":10,"postTokens":2}}"#,
        r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.25,"totalDuration":7000}"#,
    ];
    let path = std::env::temp_dir().join(format!("agentparse-claude-{}.jsonl", std::process::id()));
    std::fs::write(&path, lines.join("\n")).unwrap();
    let s = parse_file(&path).unwrap();
    assert_eq!(
        s.usage.cost_usd,
        Some(0.25),
        "the recorded cost survives finish"
    );
    assert_eq!(
        s.extra.as_ref().unwrap()["cost-state"]["totalDuration"],
        7000
    );
    let bg = call(&s, "toolu_bg");
    assert_eq!(
        (bg.status, bg.status_detail.as_deref()),
        (ToolStatus::Pending, None),
        "still running, never inferred"
    );
    assert_eq!(shell(bg).background_id.as_deref(), Some("b1"));
    let EntryKind::Error(e) = &at(&s, 5).kind else {
        panic!()
    };
    assert_eq!(
        (e.kind.as_deref(), e.status),
        (Some("rate_limit"), Some(429))
    );
    let EntryKind::Notice(n) = &at(&s, 6).kind else {
        panic!()
    };
    assert_eq!(
        (n.kind.as_deref(), n.text.as_str()),
        (Some("synthetic"), "No response requested.")
    );
    assert_eq!(
        assistants(&s).len(),
        2,
        "placeholders are no model messages"
    );
    let away = message(at(&s, 7));
    assert_eq!(
        (away.role, away.origin, away.kind.as_deref()),
        (Role::System, Origin::Summary, Some("away_summary"))
    );
    let EntryKind::Compaction(c) = &at(&s, 8).kind else {
        panic!()
    };
    assert_eq!(
        c.logical_parent.as_deref(),
        Some("msg_2"),
        "a line uuid resolves to its entry"
    );

    // A tail window that starts exactly at a line start keeps that line.
    let len = std::fs::metadata(&path).unwrap().len();
    let tail = lines[6..].join("\n").len() as u64;
    let t = parse_file_tail(&path, tail).unwrap();
    let first = t
        .entries
        .iter()
        .find(|e| !matches!(e.kind, EntryKind::Settings(_)));
    assert_eq!(
        (len > tail, first.and_then(|e| e.id.as_deref())),
        (true, Some("w1"))
    );
    std::fs::remove_file(&path).unwrap();
}

// Inline transcripts for the rules the fixtures do not reach: one small test per rule group.

const TS: &str = "2026-10-12T09:00:00.000Z";
const TS_MS: i64 = 1_791_795_600_000;

/// One line: `fields` over a conversation envelope.
fn line(kind: &str, uuid: &str, parent: Option<&str>, fields: Value) -> Value {
    let mut v = json!({"type": kind, "uuid": uuid, "parentUuid": parent, "timestamp": TS,
                       "sessionId": "s1", "cwd": "/w"});
    if let (Value::Object(v), Value::Object(f)) = (&mut v, fields) {
        v.extend(f);
    }
    v
}

fn prompt(uuid: &str, parent: Option<&str>, content: Value) -> Value {
    line(
        "user",
        uuid,
        parent,
        json!({"message": {"role": "user", "content": content}}),
    )
}

fn reply(uuid: &str, parent: Option<&str>, id: &str, blocks: Value) -> Value {
    let message = json!({"id": id, "model": "m", "role": "assistant", "content": blocks,
                         "stop_reason": "tool_use", "usage": {"input_tokens": 1, "output_tokens": 1}});
    line("assistant", uuid, parent, json!({ "message": message }))
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn bash(id: &str, command: &str) -> Value {
    tool_use(id, "Bash", json!({ "command": command }))
}

/// A result line: `tur` is its `toolUseResult` (`Null` for none), `more` other line keys.
fn result(
    uuid: &str,
    call: &str,
    content: Value,
    is_error: bool,
    tur: Value,
    more: Value,
) -> Value {
    let block = json!({"type": "tool_result", "tool_use_id": call, "content": content, "is_error": is_error});
    let mut fields = json!({"message": {"role": "user", "content": [block]}});
    if !tur.is_null() {
        fields["toolUseResult"] = tur;
    }
    if let (Value::Object(f), Value::Object(m)) = (&mut fields, more) {
        f.extend(m);
    }
    line("user", uuid, Some("a1"), fields)
}

fn shell_ok(stdout: &str) -> Value {
    json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false})
}

fn notification(uuid: &str, body: &str) -> Value {
    let fields = json!({"origin": {"kind": "task-notification"},
                        "message": {"role": "user", "content": format!("<task-notification>\n{body}\n</task-notification>")}});
    line("user", uuid, None, fields)
}

fn transcript(lines: &[Value]) -> Session {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let source = Source::File {
        path: "s1.jsonl".into(),
    };
    parse_reader(Agent::Claude, text.as_bytes(), source).unwrap()
}

fn hooks(s: &Session) -> Vec<&agentparse::Hook> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Hook(h) => Some(h),
            _ => None,
        })
        .collect()
}

/// CLI 2.1.156 to 2.1.177 sometimes wrote a result line before the line
/// holding its call.
#[test]
fn a_result_written_before_its_call_waits_for_it() {
    let hook = json!({"attachment": {"type": "hook_success", "hookEvent": "PostToolUse", "hookName": "PostToolUse:Bash",
                                     "command": "h", "stdout": "", "stderr": "", "exitCode": 0, "durationMs": 1}});
    let s = transcript(&[
        prompt("u1", None, json!("go")),
        result(
            "r1",
            "toolu_1",
            json!("done"),
            false,
            shell_ok("done"),
            json!({"timestamp": "2026-10-12T09:00:02.000Z"}),
        ),
        line("attachment", "h1", Some("r1"), hook),
        reply(
            "a1",
            Some("u1"),
            "msg_1",
            json!([bash("toolu_1", "echo done")]),
        ),
        reply(
            "a2",
            Some("h1"),
            "msg_2",
            json!([{"type": "text", "text": "ok"}]),
        ),
        line(
            "user",
            "r9",
            Some("a2"),
            json!({"message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_gone", "content": "lost"}]}}),
        ),
    ]);
    let c = call(&s, "toolu_1");
    assert_eq!((c.status, shell(c).exit_code), (ToolStatus::Ok, Some(0)));
    assert_eq!(c.ended_at, Some(TS_MS + 2000), "the result line's time");
    let msg_1 = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_1"))
        .unwrap();
    assert!(
        msg_1.extra.as_ref().unwrap()["uuids"]
            .as_array()
            .unwrap()
            .contains(&json!("r1"))
    );
    assert_eq!(
        at(&s, 3).parent_id.as_deref(),
        Some("msg_1"),
        "a parent written later resolves"
    );
    // Only the result whose call never came is an orphan, at its own line.
    let orphans: Vec<&Entry> = s
        .entries
        .iter()
        .filter(|e| matches!(&e.kind, EntryKind::Message(m) if m.origin == Origin::Tool))
        .collect();
    assert_eq!(orphans.len(), 1);
    assert_eq!(
        (orphans[0].line, orphans[0].parent_id.as_deref()),
        (Some(6), Some("msg_2"))
    );
    assert_eq!(
        s.warnings.iter().map(|w| w.line).collect::<Vec<_>>(),
        [Some(6)]
    );
}

/// History copies inside one file repeat a line uuid.
#[test]
fn history_copies_inside_a_file_are_skipped() {
    let hook = line(
        "attachment",
        "h1",
        None,
        json!({"attachment": {"type": "hook_success", "hookEvent": "PreToolUse",
        "hookName": "PreToolUse:Bash", "command": "h", "stdout": "", "stderr": "", "exitCode": 0, "durationMs": 1}}),
    );
    let ask = reply("a1", Some("h1"), "msg_1", json!([bash("toolu_1", "ls")]));
    let done = result("r1", "toolu_1", json!("x"), false, shell_ok("x"), json!({}));
    let copy = |mut v: Value| {
        v["parentUuid"] = json!("elsewhere");
        v["timestamp"] = json!("2026-10-12T10:00:00.000Z");
        v
    };
    let s = transcript(&[
        hook.clone(),
        ask.clone(),
        done.clone(),
        copy(hook),
        copy(ask),
        copy(done),
    ]);
    assert_eq!(hooks(&s).len(), 1);
    assert_eq!(s.tool_calls().len(), 1);
    assert_eq!(
        call(&s, "toolu_1").ended_at,
        Some(TS_MS),
        "the original result, not the copy"
    );
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.warnings[0].line, Some(4));
    assert!(s.warnings[0].message.starts_with("3 lines repeat the uuid"));
}

/// A queued notification never gets a dequeue of its own.
#[test]
fn notifications_never_enter_the_human_queue() {
    let op = |op: &str, content: Option<&str>| {
        let mut v =
            json!({"type": "queue-operation", "operation": op, "timestamp": TS, "sessionId": "s1"});
        if let Some(c) = content {
            v["content"] = json!(c);
        }
        v
    };
    let s = transcript(&[
        op("enqueue", Some("<task-notification>x</task-notification>")),
        op("enqueue", Some("and the docs")),
        op("dequeue", None),
        prompt("u1", None, json!("and the docs")),
    ]);
    assert!(s.messages().all(|m| m.phase.is_none()));
    assert_eq!(s.messages().filter(|m| m.is_human()).count(), 1);
}

/// The shapes of a Bash result object.
#[test]
fn bash_result_objects() {
    let edit_diff = json!({"files": [
        {"filePath": "/w/x.txt", "hunks": [{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": ["-a", "+b"]}]},
        {"filePath": "/w/new.txt", "created": true, "hunks": [{"oldStart": 0, "oldLines": 0, "newStart": 1, "newLines": 1, "lines": ["+n"]}]}],
        "changedFiles": ["/w/x.txt", "/w/new.txt"], "moreFiles": false});
    let with = |extra: Value| {
        let mut v = shell_ok("");
        if let (Value::Object(v), Value::Object(e)) = (&mut v, extra) {
            v.extend(e);
        }
        v
    };
    let persisted = "<persisted-output>\nOutput too large (40.5KB). Full output saved to: /t/r/b3.txt\n\nhead\n</persisted-output>";
    let calls = [
        bash("b1", "sed -i s/a/b/ x.txt"),
        bash("b2", "grep zzz x"),
        bash("b3", "cat big"),
        bash("b4", "sleep 999"),
        bash("b5", "true"),
    ];
    let s = transcript(&[
        prompt("u1", None, json!("go")),
        reply("a1", Some("u1"), "msg_1", json!(calls)),
        result(
            "r1",
            "b1",
            json!(""),
            false,
            with(json!({ "bashEditDiff": edit_diff })),
            json!({}),
        ),
        result(
            "r2",
            "b2",
            json!(""),
            false,
            with(json!({"returnCodeInterpretation": "No matches found"})),
            json!({}),
        ),
        result(
            "r3",
            "b3",
            json!(persisted),
            false,
            with(
                json!({"stdout": "<big>", "persistedOutputPath": "/t/r/b3.txt", "persistedOutputSize": 41472}),
            ),
            json!({}),
        ),
        result(
            "r4",
            "b4",
            json!("Command running in background with ID: bg4."),
            false,
            with(json!({"backgroundTaskId": "bg4", "timedOutAfterMs": 120000})),
            json!({}),
        ),
        result("r5", "b5", json!(""), false, shell_ok(""), json!({})),
    ]);
    let b1 = call(&s, "b1");
    let files = &b1.output.as_ref().unwrap().files;
    assert_eq!(
        (
            files[0].op,
            files[0].diff.as_deref(),
            files[0].additions,
            files[0].deletions
        ),
        (
            FileOp::Update,
            Some("@@ -1,1 +1,1 @@\n-a\n+b\n"),
            Some(1),
            Some(1)
        )
    );
    assert_eq!(
        (files[1].path.as_str(), files[1].op),
        ("/w/new.txt", FileOp::Add)
    );
    let data = b1.output.as_ref().unwrap().data.as_ref().unwrap();
    assert!(
        data["bashEditDiff"].get("files").is_none()
            && data["bashEditDiff"]["changedFiles"].is_array()
    );
    // A soft failure (grep found nothing, code 1) stays Ok with no code.
    let b2 = call(&s, "b2");
    assert_eq!((b2.status, shell(b2).exit_code), (ToolStatus::Ok, None));
    let b3 = call(&s, "b3");
    let out = b3.output.as_ref().unwrap();
    assert_eq!(
        (out.truncated, out.full_output_path.as_deref()),
        (true, Some("/t/r/b3.txt"))
    );
    // Moved to the background after a foreground timeout: still running, not timed out.
    let b4 = call(&s, "b4");
    assert_eq!(
        (
            b4.status,
            shell(b4).background_id.as_deref(),
            shell(b4).timed_out,
            shell(b4).exit_code
        ),
        (ToolStatus::Pending, Some("bg4"), false, None)
    );
    assert_eq!(
        b4.output.as_ref().unwrap().data.as_ref().unwrap()["timedOutAfterMs"],
        120000
    );
    assert_eq!(shell(call(&s, "b5")).exit_code, Some(0));
}

/// Denial kinds, and the texts older CLIs wrote without one.
#[test]
fn denials_and_interrupts() {
    let denied = "Permission to use Bash with command rm has been denied.";
    let cancelled =
        "<tool_use_error>Cancelled: parallel tool call Bash(x) errored</tool_use_error>";
    let ended = "[Tool call interrupted: the session ended before this call's result was recorded, so its outcome is unknown.]";
    let cases: [(&str, &str, Option<&str>, ToolStatus); 7] = [
        ("d1", denied, Some("permission-rule"), ToolStatus::Rejected),
        ("d2", denied, Some("automode-blocked"), ToolStatus::Rejected),
        ("d3", cancelled, Some("cancelled"), ToolStatus::Interrupted),
        ("d4", cancelled, None, ToolStatus::Interrupted),
        ("d5", ended, None, ToolStatus::Interrupted),
        ("d6", denied, None, ToolStatus::Rejected),
        (
            "d7",
            "PreToolUse:Bash hook error: [blocked]",
            None,
            ToolStatus::Error,
        ),
    ];
    let mut lines = vec![reply(
        "a1",
        None,
        "msg_1",
        json!(cases.map(|(id, ..)| bash(id, "rm x"))),
    )];
    for (id, text, kind, _) in cases {
        let more = kind.map_or(json!({}), |k| json!({ "toolDenialKind": k }));
        lines.push(result(
            &format!("r{id}"),
            id,
            json!(text),
            true,
            json!(format!("Error: {text}")),
            more,
        ));
    }
    let s = transcript(&lines);
    for (id, _, kind, status) in cases {
        let c = call(&s, id);
        assert_eq!(
            (c.status, c.status_detail.as_deref()),
            (status, kind),
            "{id}"
        );
        assert_eq!(
            c.output.as_ref().unwrap().data,
            None,
            "{id}: the string result repeats the text"
        );
        let interrupted = c
            .output
            .as_ref()
            .and_then(|o| o.shell.as_ref())
            .is_some_and(|s| s.interrupted);
        assert_eq!(interrupted, status == ToolStatus::Interrupted, "{id}");
    }
}

/// A task notification settles the background shell it names.
#[test]
fn task_notifications_settle_background_shells() {
    let ids = ["n1", "n2", "n3", "n4"];
    let mut lines = vec![reply(
        "a1",
        None,
        "msg_1",
        json!(ids.map(|id| bash(id, "make"))),
    )];
    for id in ids {
        let started = json!({"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": format!("bg-{id}")});
        lines.push(result(
            &format!("r{id}"),
            id,
            json!("Command running in background"),
            false,
            started,
            json!({}),
        ));
    }
    let notes = [
        (
            "n1",
            "failed",
            "Background command \"make\" failed (exit code 2)",
        ),
        ("n2", "killed", "Background command \"make\" was stopped"),
        (
            "n3",
            "completed",
            "Background command \"make\" completed (exit code 1: No matches found)",
        ),
        (
            "n4",
            "completed",
            "Background command \"make\" completed (exit code 2)",
        ),
    ];
    for (id, status, summary) in notes {
        let body = format!(
            "<task-id>bg-{id}</task-id>\n<tool-use-id>{id}</tool-use-id>\n<output-file>/t/bg-{id}.output</output-file>\n<status>{status}</status>\n<summary>{summary}</summary>"
        );
        lines.push(notification(&format!("t{id}"), &body));
    }
    let s = transcript(&lines);
    let got: Vec<_> = ids
        .iter()
        .map(|id| {
            let c = call(&s, id);
            (
                c.status,
                c.status_detail.as_deref(),
                shell(c).exit_code,
                shell(c).interrupted,
            )
        })
        .collect();
    use ToolStatus::*;
    assert_eq!(
        got,
        [
            (Error, Some("failed"), Some(2), false),
            (Interrupted, Some("killed"), None, true),
            (Ok, Some("completed"), Some(1), false),
            (Error, Some("completed"), Some(2), false),
        ]
    );
    let out = call(&s, "n1").output.as_ref().unwrap();
    assert_eq!(out.full_output_path.as_deref(), Some("/t/bg-n1.output"));
}

/// A nested subagent's parent is the agent that spawned it, from the meta file.
#[test]
fn nested_subagent_links_to_its_parent_agent() {
    let dir = std::env::temp_dir().join(format!("agentparse-claude-nested-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("agent-anested.jsonl");
    let mut first = prompt("u1", None, json!("look around"));
    first["isSidechain"] = json!(true);
    first["agentId"] = json!("anested");
    first["sessionId"] = json!("root");
    std::fs::write(&path, format!("{first}\n")).unwrap();
    let meta = json!({"agentType": "Explore", "description": "Look around", "toolUseId": "toolu_p",
                      "parentAgentId": "aparent", "spawnDepth": 2});
    std::fs::write(dir.join("agent-anested.meta.json"), meta.to_string()).unwrap();
    let s = parse_file(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(s.id, "anested");
    let parent = s.parent.unwrap();
    assert_eq!(
        (
            parent.session_id.as_str(),
            parent.call_id.as_deref(),
            parent.agent_type.as_deref()
        ),
        ("aparent", Some("toolu_p"), Some("Explore"))
    );
    assert_eq!(s.title.as_deref(), Some("Look around"));
    let extra = s.extra.unwrap();
    assert_eq!(
        (&extra["sessionId"], &extra["meta"]["spawnDepth"]),
        (&json!("root"), &json!(2))
    );
}

/// Messages from other agents carry origin Agent and their sender.
#[test]
fn messages_from_other_agents() {
    let with = |uuid: &str, fields: Value, content: &str| {
        let mut v = prompt(uuid, None, json!(content));
        if let (Value::Object(v), Value::Object(f)) = (&mut v, fields) {
            v.extend(f);
        }
        v
    };
    let s = transcript(&[
        prompt(
            "u1",
            None,
            json!("<teammate-message teammate_id=\"lead\" summary=\"s\">do x</teammate-message>"),
        ),
        prompt(
            "u0",
            None,
            json!(
                "Another Claude session sent a message:\n<teammate-message teammate_id=\"install-login\" color=\"green\" summary=\"s\">\nready</teammate-message>"
            ),
        ),
        with(
            "u2",
            json!({"isMeta": true, "origin": {"kind": "coordinator"}}),
            "The coordinator sent a message",
        ),
        with(
            "u3",
            json!({"isMeta": true, "origin": {"kind": "peer", "from": "session-b", "name": "worker"}}),
            "hello",
        ),
    ]);
    let got: Vec<_> = s
        .messages()
        .map(|m| (m.origin, m.kind.as_deref(), m.author.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            (Origin::Agent, Some("teammate-message"), Some("lead")),
            (
                Origin::Agent,
                Some("teammate-message"),
                Some("install-login")
            ),
            (Origin::Agent, Some("coordinator"), None),
            (Origin::Agent, Some("peer"), Some("worker")),
        ]
    );
    assert_eq!(message(at(&s, 4)).text(), "hello");
    assert_eq!(
        at(&s, 4).extra.as_ref().unwrap()["origin"]["from"],
        "session-b"
    );
}

/// Images keep their type and size; `file.base64` stays as its size.
#[test]
fn images_keep_only_their_size() {
    let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="}});
    let read = json!({"type": "image", "file": {"type": "image/png", "base64": "aGVsbG8=", "originalSize": 5}});
    let s = transcript(&[
        prompt("u1", None, json!([{"type": "text", "text": "see"}, image])),
        reply(
            "a1",
            Some("u1"),
            "msg_1",
            json!([tool_use("i1", "Read", json!({"file_path": "/w/p.png"}))]),
        ),
        result("r1", "i1", json!([image]), false, read, json!({})),
    ]);
    let png = agentparse::Media {
        mime: Some("image/png".into()),
        bytes: Some(5),
        ..Default::default()
    };
    assert_eq!(message(at(&s, 1)).parts[1], Part::Attachment(png.clone()));
    let out = call(&s, "i1").output.as_ref().unwrap();
    assert_eq!(out.attachments, [png]);
    assert_eq!(
        out.data.as_ref().unwrap()["file"]["base64"],
        json!({"bytes": 5})
    );
    assert!(!serde_json::to_string(&s).unwrap().contains("aGVsbG8"));
}

/// A user line whose blocks have different kinds splits into `#1`, `#2`.
#[test]
fn a_mixed_user_line_splits_by_kind() {
    let reminder = "<system-reminder>be careful</system-reminder>";
    let s = transcript(&[
        prompt(
            "u1",
            None,
            json!([{"type": "text", "text": "fix it"}, {"type": "text", "text": reminder}]),
        ),
        reply("a1", Some("u1"), "msg_1", json!([bash("c1", "ls")])),
        prompt(
            "u2",
            Some("a1"),
            json!([{"type": "tool_result", "tool_use_id": "c1", "content": "x"},
                                        {"type": "text", "text": "and also this"}]),
        ),
    ]);
    let got: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) if m.role == Role::User => {
                Some((e.id.as_deref(), m.origin, m.kind.as_deref()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        got,
        [
            (Some("u1#1"), Origin::Human, None),
            (Some("u1#2"), Origin::Context, Some("system-reminder")),
            (Some("u2"), Origin::Human, None),
        ]
    );
    assert_eq!(call(&s, "c1").output.as_ref().unwrap().text, "x");
}

/// `system/local_command` and the hook outcomes.
#[test]
fn local_commands_and_hook_outcomes() {
    let hook = |uuid: &str, kind: &str| {
        line(
            "attachment",
            uuid,
            None,
            json!({"attachment": {"type": kind, "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash", "command": "guard", "stdout": "", "stderr": "no", "exitCode": 2, "durationMs": 3}}),
        )
    };
    let s = transcript(&[
        line(
            "system",
            "l1",
            None,
            json!({"subtype": "local_command", "level": "info",
             "content": "<local-command-stdout>Done</local-command-stdout>"}),
        ),
        hook("h1", "hook_blocking_error"),
        hook("h2", "hook_cancelled"),
    ]);
    let m = message(at(&s, 1));
    assert_eq!(
        (m.role, m.origin, m.kind.as_deref()),
        (Role::User, Origin::Command, Some("local-command-stdout"))
    );
    let got: Vec<_> = hooks(&s)
        .iter()
        .map(|h| (h.blocked, h.exit_code, h.stderr.as_deref()))
        .collect();
    assert_eq!(
        got,
        [(true, Some(2), Some("no")), (false, Some(2), Some("no"))]
    );
}

/// An input the CLI could not parse is read from its raw text when that is JSON.
#[test]
fn unparsed_tool_input_is_read_from_its_raw_text() {
    let raw = |text: &str| json!({"__unparsedToolInput": {"raw": text, "len": text.len()}});
    let s = transcript(&[reply(
        "a1",
        None,
        "msg_1",
        json!([
            tool_use("p1", "Read", raw(r#"{"file_path":"/w/a"}"#)),
            tool_use("p2", "Edit", raw("not json")),
        ]),
    )]);
    assert_eq!(
        call(&s, "p1").tool,
        Tool::Read {
            path: "/w/a".into(),
            offset: None,
            limit: None
        }
    );
    assert_eq!(call(&s, "p2").tool, Tool::Other);
}

/// 2.1.143 `api_error` lines put a `cause` next to `error`.
#[test]
fn api_error_message_from_the_line_cause() {
    let s = transcript(&[line(
        "system",
        "e1",
        None,
        json!({"subtype": "api_error", "level": "error",
        "error": {"status": 503}, "cause": {"code": "ECONNRESET"}, "retryAttempt": 2, "maxRetries": 10}),
    )]);
    let EntryKind::Error(e) = &at(&s, 1).kind else {
        panic!()
    };
    assert_eq!(
        (e.message.as_str(), e.status, e.retry),
        ("ECONNRESET", Some(503), Some(2))
    );
}

/// What a merged line or a call block says beyond the entry.
#[test]
fn merged_lines_and_call_blocks_keep_their_own_keys() {
    let at_time = |mut v: Value, ts: &str| {
        v["timestamp"] = json!(ts);
        v
    };
    let mut block = bash("c1", "ls");
    block["caller"] = json!({"type": "code_execution_20250825"});
    let s = transcript(&[
        reply(
            "a1",
            None,
            "msg_1",
            json!([{"type": "thinking", "thinking": "", "signature": "c2lnbmF0dXJl"}]),
        ),
        at_time(
            line(
                "attachment",
                "x1",
                Some("a1"),
                json!({"attachment": {"type": "date_change", "newDate": "d"}}),
            ),
            "2026-10-12T09:00:01.000Z",
        ),
        at_time(
            reply(
                "a2",
                Some("x1"),
                "msg_1",
                json!([{"type": "text", "text": "hi"}]),
            ),
            "2026-10-12T09:00:02.000Z",
        ),
        at_time(
            reply("a3", Some("a2"), "msg_1", json!([block])),
            "2026-10-12T09:00:03.000Z",
        ),
    ]);
    let entry = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("msg_1"))
        .unwrap();
    let extra = entry.extra.as_ref().unwrap();
    assert_eq!(
        extra["lines"]["a2"],
        json!({"parentUuid": "x1", "ts": TS_MS + 2000})
    );
    assert!(
        extra["lines"].get("a3").is_none(),
        "chained, and its time is the call's started_at"
    );
    assert_eq!(call(&s, "c1").started_at, Some(TS_MS + 3000));
    assert_eq!(
        extra["calls"]["c1"]["call"]["caller"]["type"],
        "code_execution_20250825"
    );
}

// Chat parity (background work, hooks, attachments, queued input).

/// A prompt, one assistant message with these calls, and a launch result line for each:
/// `(call id, tool name, input, toolUseResult)`.
fn launched(calls: &[(&str, &str, Value, Value)]) -> Vec<Value> {
    let blocks: Vec<Value> = calls
        .iter()
        .map(|(id, name, input, _)| tool_use(id, name, input.clone()))
        .collect();
    let mut lines = vec![
        prompt("u0", None, json!("go")),
        reply("a1", Some("u0"), "msg_1", json!(blocks)),
    ];
    for (id, _, _, tur) in calls {
        let content = json!("launched");
        let uuid = format!("r-{id}");
        lines.push(result(&uuid, id, content, false, tur.clone(), json!({})));
    }
    lines
}

fn async_agent(id: &'static str, agent_id: &str) -> (&'static str, &'static str, Value, Value) {
    let input = json!({"description": "scan", "prompt": "p"});
    let tur =
        json!({"isAsync": true, "status": "async_launched", "agentId": agent_id, "prompt": "p"});
    (id, "Agent", input, tur)
}

fn background_bash(id: &'static str, task: &str) -> (&'static str, &'static str, Value, Value) {
    let input = json!({"command": "sleep 9", "run_in_background": true});
    let tur = json!({"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": task});
    (id, "Bash", input, tur)
}

fn monitor(
    id: &'static str,
    input: Value,
    task: &str,
) -> (&'static str, &'static str, Value, Value) {
    let mut base = json!({"command": "tail -f x", "description": "watch"});
    base.as_object_mut()
        .unwrap()
        .extend(input.as_object().unwrap().clone());
    (
        id,
        "Monitor",
        base,
        json!({"taskId": task, "timeoutMs": 600000, "persistent": false}),
    )
}

fn workflow(id: &'static str, task: &str) -> (&'static str, &'static str, Value, Value) {
    let tur =
        json!({"status": "async_launched", "taskId": task, "runId": "wf_1", "summary": "Plan it"});
    (
        id,
        "Workflow",
        json!({"script": "export const meta = {}"}),
        tur,
    )
}

fn status_of(s: &Session, id: &str) -> (ToolStatus, Option<String>) {
    let c = call(s, id);
    (c.status, c.status_detail.clone())
}

fn at_time(mut v: Value, ts: &str) -> Value {
    v["timestamp"] = json!(ts);
    v
}

/// A launch is not an outcome: the call stays Pending, without an end time, until its task
/// reports. The report finds its call by `<tool-use-id>` or, when it has none, by `<task-id>`.
#[test]
fn launches_stay_pending_until_a_notification_ends_them() {
    let calls = [
        background_bash("sh", "bg1"),
        async_agent("ag", "agid"),
        monitor("mon", json!({"timeout_ms": 600000}), "mid"),
        workflow("wf", "wid"),
        background_bash("st", "bg2"),
        background_bash("dn", "bg3"),
    ];
    let launches = launched(&calls);
    let s = transcript(&launches);
    for id in ["sh", "ag", "mon", "wf", "st", "dn"] {
        let c = call(&s, id);
        assert_eq!((id, c.status, c.ended_at), (id, ToolStatus::Pending, None));
    }
    let wf = call(&s, "wf");
    assert!(
        matches!(&wf.tool, Tool::Task { description: Some(d), agent_type: Some(t), .. }
        if d == "Plan it" && t == "workflow")
    );
    assert_eq!(wf.child_session, None);
    assert_eq!(call(&s, "ag").child_session.as_deref(), Some("agid"));

    // Notes that are not an end: a running status, a Monitor event, an agent that waits on its
    // own background work.
    let interim =
        "This agent stopped with background work of its own still running. It may resume.";
    let mut lines = launches.clone();
    lines.extend([
        notification("n1", "<task-id>bg2</task-id>\n<status>running</status>"),
        notification("n2", "<task-id>mid</task-id>\n<summary>Monitor event: \"watch\"</summary>\n<event>tick</event>"),
        notification(
            "n3",
            &format!("<task-id>agid</task-id>\n<status>completed</status>\n<summary>Agent \"scan\" is waiting on its own background work</summary>\n<note>{interim}</note>"),
        ),
    ]);
    let s = transcript(&lines);
    for id in ["ag", "mon", "st"] {
        assert_eq!((id, call(&s, id).status), (id, ToolStatus::Pending));
    }

    // The ends: by call id, by task id alone, and the statuses `done` and `stopped`.
    lines.extend([
        notification("n4", "<task-id>bg1</task-id>\n<tool-use-id>sh</tool-use-id>\n<status>completed</status>\n<summary>Background command \"make\" completed (exit code 0)</summary>"),
        notification("n5", "<task-id>agid</task-id>\n<status>completed</status>\n<summary>Agent \"scan\" completed</summary>"),
        notification("n6", "<task-id>wid</task-id>\n<status>failed</status>\n<summary>Dynamic workflow failed</summary>"),
        notification("n7", "<task-id>bg2</task-id>\n<tool-use-id>st</tool-use-id>\n<status>stopped</status>"),
        notification("n8", "<task-id>bg3</task-id>\n<status>done</status>"),
        notification("n9", "<task-id>mid</task-id>\n<summary>Monitor event: \"watch\"</summary>\n<event>[Monitor expired after 10m with no events delivered. Re-arm it.]</event>"),
    ]);
    let s = transcript(&lines);
    use ToolStatus::*;
    let got: Vec<_> = ["sh", "ag", "wf", "st", "dn", "mon"]
        .iter()
        .map(|id| {
            (
                call(&s, id).status,
                call(&s, id).status_detail.clone().unwrap(),
            )
        })
        .collect();
    let want = [
        (Ok, "completed"),
        (Ok, "completed"),
        (Error, "failed"),
        (Interrupted, "stopped"),
        (Ok, "done"),
        (Ok, "expired"),
    ];
    assert_eq!(got, want.map(|(s, d)| (s, d.to_string())));
    assert!(shell(call(&s, "st")).interrupted);
    assert!(call(&s, "ag").ended_at.is_some());
}

/// What never reported still ends: a stop result, `agents_killed`, a turn that ends with none
/// left, a Monitor that ran out its `timeout_ms`.
#[test]
fn records_of_the_harness_end_what_never_reported() {
    let calls = [
        async_agent("ag", "agid"),
        workflow("wf", "wid"),
        background_bash("sh", "bg1"),
        monitor("m10", json!({"timeout_ms": 600000}), "m10id"),
        monitor("mp", json!({"persistent": true}), "mpid"),
    ];
    let with = |extra: Vec<Value>| {
        let mut lines = launched(&calls);
        lines.extend(extra);
        transcript(&lines)
    };
    let turn = |agents: u64, workflows: u64| {
        line(
            "system",
            "t1",
            None,
            json!({"subtype": "turn_duration", "durationMs": 5,
            "pendingBackgroundAgentCount": agents, "pendingWorkflowCount": workflows}),
        )
    };
    let pending = |s: &Session| -> Vec<&str> {
        ["ag", "wf", "sh", "m10", "mp"]
            .into_iter()
            .filter(|id| call(s, id).status == ToolStatus::Pending)
            .collect()
    };
    // Something is still running: nothing ends.
    assert_eq!(
        pending(&with(vec![turn(1, 1)])),
        ["ag", "wf", "sh", "m10", "mp"]
    );
    // None left: the agents end, the workflows only when their own count is zero.
    let s = with(vec![turn(0, 2)]);
    assert_eq!(pending(&s), ["wf", "sh", "m10", "mp"]);
    assert_eq!(
        status_of(&s, "ag"),
        (ToolStatus::Interrupted, Some("ended".into()))
    );
    assert_eq!(pending(&with(vec![turn(0, 0)])), ["sh", "m10", "mp"]);
    // All agents killed: agents and workflows, no shell or monitor.
    let killed = line("system", "k1", None, json!({"subtype": "agents_killed"}));
    let s = with(vec![killed]);
    assert_eq!(pending(&s), ["sh", "m10", "mp"]);
    assert_eq!(
        status_of(&s, "wf"),
        (ToolStatus::Interrupted, Some("killed".into()))
    );
    // A successful TaskStop ends the shell it names; a failed one does not.
    let stop = |id: &str, task: &str, is_error: bool| {
        let uuid = format!("s-{id}");
        let call = tool_use(id, "TaskStop", json!({"task_id": task}));
        let tur = json!({"message": "Successfully stopped task", "task_id": task});
        [
            reply(
                &format!("a-{id}"),
                None,
                &format!("msg-{id}"),
                json!([call]),
            ),
            result(&uuid, id, json!("{}"), is_error, tur, json!({})),
        ]
    };
    let s = with(stop("stop1", "bg1", false).to_vec());
    assert_eq!(
        status_of(&s, "sh"),
        (ToolStatus::Interrupted, Some("stopped".into()))
    );
    assert_eq!(call(&s, "sh").ended_at, Some(TS_MS));
    let s = with(stop("stop2", "bg1", true).to_vec());
    assert_eq!(call(&s, "sh").status, ToolStatus::Pending);
    // A record 11 minutes later is past the 10 minute Monitor; 5 minutes later is not. A
    // persistent Monitor has no end.
    let later = |ts: &str| {
        at_time(
            line(
                "system",
                "i1",
                None,
                json!({"subtype": "informational", "content": "x"}),
            ),
            ts,
        )
    };
    let s = with(vec![later("2026-10-12T09:11:00.000Z")]);
    assert_eq!(pending(&s), ["ag", "wf", "sh", "mp"]);
    assert_eq!(
        status_of(&s, "m10"),
        (ToolStatus::Ok, Some("expired".into()))
    );
    assert_eq!(call(&s, "m10").ended_at, Some(TS_MS + 600_000));
    assert_eq!(
        pending(&with(vec![later("2026-10-12T09:05:00.000Z")])).len(),
        5
    );
}

/// Claude writes a typed `/compact` as a prompt and again as the expanded command line.
#[test]
fn a_typed_slash_command_is_not_shown_twice() {
    let expanded = |name: &str, args: &str| {
        format!(
            "<command-name>{name}</command-name>\n<command-message>x</command-message>\n<command-args>{args}</command-args>"
        )
    };
    let user = |uuid: &str, parent: Option<&str>, prompt: &str, text: String| {
        line(
            "user",
            uuid,
            parent,
            json!({"promptId": prompt, "message": {"role": "user", "content": text}}),
        )
    };
    let s = transcript(&[
        user("c0", None, "p1", "/compact".into()),
        line(
            "system",
            "b1",
            None,
            json!({"subtype": "compact_boundary", "content": "Conversation compacted",
            "compactMetadata": {"trigger": "manual", "preTokens": 10, "postTokens": 2}}),
        ),
        // A new promptId in between (a compaction starts one) still pairs.
        user("c2", Some("b1"), "p9", expanded("/compact", "")),
        user(
            "c3",
            Some("c2"),
            "p9",
            "<local-command-stdout>Compacted</local-command-stdout>".into(),
        ),
        // These do not: no typed prompt behind the /model line, and the last expansion comes a
        // minute after the prompt.
        user("m1", None, "p2", expanded("/model", "opus")),
        user("c4", None, "p3", "/compact".into()),
        at_time(
            user("c5", Some("c4"), "p4", expanded("/compact", "")),
            "2026-10-12T09:01:00.000Z",
        ),
    ]);
    let commands: Vec<_> = s
        .messages()
        .filter(|m| m.origin == Origin::Command)
        .map(|m| m.kind.clone().unwrap())
        .collect();
    assert_eq!(
        commands,
        ["local-command-stdout", "command-name", "command-name"]
    );
    assert_eq!(user_texts(&s, Origin::Human), ["/compact", "/compact"]);
    let stdout = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("c3"))
        .unwrap();
    assert_eq!(
        stdout.parent_id.as_deref(),
        Some("c0"),
        "the dropped line's uuid is the prompt's"
    );
}

/// Hook text is read: a block's reason, a message, the errors of a Stop run.
#[test]
fn hook_errors_and_messages_are_read() {
    let hook = |uuid: &str, a: Value| line("attachment", uuid, None, json!({"attachment": a}));
    let s = transcript(&[
        hook(
            "h1",
            json!({"type": "hook_blocking_error", "hookEvent": "Stop", "hookName": "Stop",
            "blockingError": {"blockingError": "read the skill first", "command": "check.sh"}}),
        ),
        hook(
            "h2",
            json!({"type": "hook_system_message", "hookEvent": "PostToolUse", "content": "Tip: push soon"}),
        ),
        hook(
            "h3",
            json!({"type": "hook_non_blocking_error", "hookEvent": "UserPromptSubmit", "stderr": "bun: not found", "exitCode": 127}),
        ),
        line(
            "system",
            "h4",
            None,
            json!({"subtype": "stop_hook_summary", "hookInfos": [{"command": "check.sh"}],
            "hookErrors": ["one", "two"], "preventedContinuation": false}),
        ),
    ]);
    let got: Vec<_> = hooks(&s)
        .iter()
        .map(|h| {
            (
                h.blocked,
                h.command.as_deref(),
                h.stdout.as_deref(),
                h.stderr.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (true, Some("check.sh"), None, Some("read the skill first")),
            (false, None, Some("Tip: push soon"), None),
            (false, None, None, Some("bun: not found")),
            (false, Some("check.sh"), None, Some("one\ntwo")),
        ]
    );
}

/// Attachments the UI shows as one line become a notice with that line; the record stays.
#[test]
fn shown_attachments_are_one_line_notices() {
    let att = |uuid: &str, a: Value| line("attachment", uuid, None, json!({"attachment": a}));
    let s = transcript(&[
        att(
            "f",
            json!({"type": "file", "filename": "/w/a.rs", "displayPath": "a.rs",
            "content": {"type": "text", "file": {"filePath": "/w/a.rs", "content": "x\ny", "numLines": 2, "totalLines": 2}}}),
        ),
        att(
            "d",
            json!({"type": "directory", "path": "/w/src", "displayPath": "src", "content": "a.rs\nb.rs"}),
        ),
        att(
            "m",
            json!({"type": "nested_memory", "path": "/w/CLAUDE.md", "displayPath": "CLAUDE.md",
            "content": {"path": "/w/CLAUDE.md", "type": "Project", "content": "rules"}}),
        ),
        att(
            "c",
            json!({"type": "compact_file_reference", "filename": "/w/b.rs", "displayPath": "b.rs"}),
        ),
        att(
            "s",
            json!({"type": "invoked_skills", "skills": [{"name": "commit", "content": "body"}, {"name": "review"}]}),
        ),
        att(
            "g",
            json!({"type": "diagnostics", "files": [{"uri": "/w/a.rs", "diagnostics": [{"message": "a"}, {"message": "b"}]}]}),
        ),
        att(
            "k",
            json!({"type": "goal_status", "met": true, "condition": "tests pass", "iterations": 2}),
        ),
        att(
            "n",
            json!({"type": "goal_status", "met": false, "reason": "still red", "condition": "tests pass"}),
        ),
        att(
            "e",
            json!({"type": "goal_status", "met": false, "sentinel": true, "condition": "tests pass"}),
        ),
        att(
            "t",
            json!({"type": "task_status", "taskId": "t1", "description": "Refine map", "status": "completed"}),
        ),
    ]);
    let notices: Vec<(String, String)> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Notice(n) => Some((n.kind.clone().unwrap(), n.text.clone())),
            _ => None,
        })
        .collect();
    let want = [
        ("file", "Read a.rs (2 lines)"),
        ("directory", "Listed directory src/"),
        ("nested_memory", "Loaded CLAUDE.md"),
        ("compact_file_reference", "Referenced file b.rs"),
        ("invoked_skills", "Skills restored: commit, review"),
        ("diagnostics", "Found 2 new diagnostic issues in 1 file"),
        ("goal_status", "Goal met: tests pass"),
        ("goal_status", "Goal not met: still red"),
        ("task_status", "Refine map completed in background"),
    ];
    assert_eq!(notices, want.map(|(k, t)| (k.to_string(), t.to_string())));
    // The re-armed goal check shows nothing new and stays a plain record.
    assert!(other_kinds(&s).contains(&"attachment.goal_status"));
    let file = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("f"))
        .unwrap();
    assert_eq!(
        file.extra.as_ref().unwrap()["attachment"]["content"]["file"]["content"],
        "x\ny"
    );
}

/// A `task_status` attachment ends the task it names.
#[test]
fn a_task_status_attachment_ends_its_agent() {
    let mut lines = launched(&[async_agent("ag", "agid")]);
    lines.push(line(
        "attachment",
        "ts1",
        None,
        json!({"attachment": {"type": "task_status", "taskId": "agid",
        "description": "scan", "status": "running"}}),
    ));
    assert_eq!(call(&transcript(&lines), "ag").status, ToolStatus::Pending);
    lines.push(line(
        "attachment",
        "ts2",
        None,
        json!({"attachment": {"type": "task_status", "taskId": "agid",
        "description": "scan", "status": "completed"}}),
    ));
    assert_eq!(
        status_of(&transcript(&lines), "ag"),
        (ToolStatus::Ok, Some("completed".into()))
    );
}

/// Old deliveries have no `origin`: `commandMode` is their kind.
#[test]
fn queued_input_takes_its_kind_from_the_command_mode() {
    let queued = |uuid: &str, mode: &str, prompt: &str| {
        line(
            "attachment",
            uuid,
            None,
            json!({"attachment": {"type": "queued_command", "commandMode": mode, "prompt": prompt}}),
        )
    };
    let s = transcript(&[
        queued(
            "q1",
            "task-notification",
            "<task-notification>\n<task-id>x</task-id>\n</task-notification>",
        ),
        queued("q2", "prompt", "keep going"),
    ]);
    let notice = message(at(&s, 1));
    assert_eq!(
        (notice.origin, notice.kind.as_deref()),
        (Origin::Notification, Some("task-notification"))
    );
    let human = message(at(&s, 2));
    assert_eq!((human.origin, human.kind.as_deref()), (Origin::Human, None));
}

/// Task tools name one task each.
#[test]
fn task_create_and_update_are_typed() {
    let s = transcript(&[reply(
        "a1",
        None,
        "msg_1",
        json!([
            tool_use(
                "c",
                "TaskCreate",
                json!({"subject": "Write it", "description": "d"})
            ),
            tool_use(
                "u",
                "TaskUpdate",
                json!({"taskId": "3", "status": "completed"})
            ),
            tool_use(
                "v",
                "TaskUpdate",
                json!({"taskId": "3", "subject": "Renamed"})
            ),
        ]),
    )]);
    let item = |id: &str| match &call(&s, id).tool {
        Tool::Todo { items } => (items[0].content.clone(), items[0].status.clone()),
        other => panic!("{other:?}"),
    };
    assert_eq!(item("c"), ("Write it".into(), Some("pending".into())));
    assert_eq!(item("u"), ("#3".into(), Some("completed".into())));
    assert_eq!(item("v"), ("Renamed".into(), None));
}

// Abandoned branches.

fn human(uuid: &str, parent: Option<&str>, text: &str) -> Value {
    prompt(uuid, parent, json!(text))
}

fn note(uuid: &str, parent: &str) -> Value {
    let a = json!({"attachment": {"type": "date_change", "newDate": "d"}});
    line("attachment", uuid, Some(parent), a)
}

/// A prompt sent again after Esc or a rewind: its first try and what hangs off it are not part
/// of the conversation that went on.
#[test]
fn a_prompt_sent_again_drops_its_first_try() {
    let s = transcript(&[
        human("u0", None, "start"),
        reply(
            "a0",
            Some("u0"),
            "msg_0",
            json!([{"type": "text", "text": "ok"}]),
        ),
        // The first try, with an attachment and a reply of its own.
        human("p1", Some("a0"), "do it"),
        note("n1", "p1"),
        reply(
            "a1",
            Some("n1"),
            "msg_1",
            json!([{"type": "text", "text": "first answer"}]),
        ),
        // The prompt again from the same point, and the conversation goes on from here.
        human("p2", Some("a0"), "do it"),
        reply(
            "a2",
            Some("p2"),
            "msg_2",
            json!([{"type": "text", "text": "second answer"}]),
        ),
        human("u3", Some("a2"), "thanks"),
    ]);
    assert_eq!(user_texts(&s, Origin::Human), ["start", "do it", "thanks"]);
    let said: Vec<_> = assistants(&s).iter().map(|(_, m)| m.text()).collect();
    assert_eq!(said, ["ok", "second answer"]);
    assert!(!s.entries.iter().any(|e| e.id.as_deref() == Some("n1")));
    assert!(
        s.warnings
            .iter()
            .any(|w| w.message.starts_with("3 entries on branches"))
    );
}

/// Only a prompt head is a branch: results that share a parent, a queued input beside the
/// reply that went on, and a side record beside the next prompt stay.
#[test]
fn only_a_prompt_head_is_pruned() {
    let queued = line(
        "attachment",
        "q1",
        Some("a2"),
        json!({"attachment": {"type": "queued_command", "prompt": "also this"}}),
    );
    let s = transcript(&[
        human("u0", None, "go"),
        reply(
            "a1",
            Some("u0"),
            "msg_1",
            json!([tool_use("A", "Bash", json!({"command": "a"}))]),
        ),
        reply(
            "a2",
            Some("a1"),
            "msg_1",
            json!([tool_use("B", "Bash", json!({"command": "b"}))]),
        ),
        queued,
        result("rA", "A", json!("ra"), false, shell_ok("ra"), json!({})),
        result("rB", "B", json!("rb"), false, shell_ok("rb"), json!({})),
        reply(
            "a3",
            Some("a2"),
            "msg_3",
            json!([{"type": "text", "text": "both done"}]),
        ),
        // A side record beside the prompt that went on is no branch either.
        note("x1", "a3"),
        human("u4", Some("a3"), "next"),
    ]);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert!(s.entries.iter().any(|e| e.id.as_deref() == Some("x1")));
    assert_eq!(call(&s, "A").status, ToolStatus::Ok);
    assert_eq!(call(&s, "B").status, ToolStatus::Ok);
    assert!(s.messages().any(|m| m.text() == "also this"));
}

/// A rewind before a compaction still prunes: the live chain goes through the boundary.
#[test]
fn the_live_chain_crosses_a_compaction() {
    let boundary = line(
        "system",
        "b1",
        None,
        json!({"subtype": "compact_boundary", "content": "Conversation compacted",
        "logicalParentUuid": "a2", "compactMetadata": {"trigger": "manual", "preTokens": 10, "postTokens": 2}}),
    );
    let s = transcript(&[
        human("u0", None, "start"),
        human("p1", Some("u0"), "first try"),
        human("p2", Some("u0"), "second try"),
        reply(
            "a2",
            Some("p2"),
            "msg_2",
            json!([{"type": "text", "text": "answer"}]),
        ),
        boundary,
        human("u9", Some("b1"), "after"),
    ]);
    assert_eq!(
        user_texts(&s, Origin::Human),
        ["start", "second try", "after"]
    );
}

/// A persistent Monitor ignores its `timeout_ms` (real inputs carry both, `persistent` as a
/// boolean or as a string).
#[test]
fn a_persistent_monitor_does_not_expire() {
    let calls = [
        monitor(
            "mb",
            json!({"timeout_ms": 600000, "persistent": true}),
            "mbid",
        ),
        monitor(
            "ms",
            json!({"timeout_ms": 600000, "persistent": "true"}),
            "msid",
        ),
        monitor(
            "mf",
            json!({"timeout_ms": 600000, "persistent": "false"}),
            "mfid",
        ),
    ];
    let mut lines = launched(&calls);
    let info = json!({"subtype": "informational", "content": "x"});
    lines.push(at_time(
        line("system", "i1", None, info),
        "2026-10-12T09:11:00.000Z",
    ));
    let s = transcript(&lines);
    let got: Vec<_> = ["mb", "ms", "mf"]
        .iter()
        .map(|id| call(&s, id).status)
        .collect();
    assert_eq!(
        got,
        [ToolStatus::Pending, ToolStatus::Pending, ToolStatus::Ok]
    );
}

/// The tokens of an abandoned branch were spent: they stay in the session total.
#[test]
fn a_dropped_branch_keeps_its_usage_in_the_total() {
    let spent = |mut r: Value, input: u64, output: u64| {
        r["message"]["usage"] = json!({"input_tokens": input, "output_tokens": output});
        r
    };
    let text = json!([{"type": "text", "text": "ok"}]);
    let s = transcript(&[
        human("u0", None, "start"),
        human("p1", Some("u0"), "do it"),
        spent(reply("a1", Some("p1"), "msg_1", text.clone()), 100, 5000),
        human("p2", Some("u0"), "do it"),
        spent(reply("a2", Some("p2"), "msg_2", text), 10, 20),
    ]);
    assert_eq!(assistants(&s).len(), 1);
    assert_eq!((s.usage.input, s.usage.output), (110, 5020));
}

/// A compaction written inside a dead branch goes with it, so no parent or logical parent
/// points at a dropped entry.
#[test]
fn a_compaction_in_a_dead_branch_is_dropped() {
    let boundary = line(
        "system",
        "b1",
        None,
        json!({"subtype": "compact_boundary", "content": "Conversation compacted",
        "logicalParentUuid": "a1", "compactMetadata": {"trigger": "manual", "preTokens": 10, "postTokens": 2}}),
    );
    let summary = line(
        "user",
        "s1",
        Some("b1"),
        json!({"isCompactSummary": true,
        "message": {"role": "user", "content": "This session is being continued from a previous conversation"}}),
    );
    let text = json!([{"type": "text", "text": "ok"}]);
    let s = transcript(&[
        human("u0", None, "start"),
        human("p1", Some("u0"), "first try"),
        reply("a1", Some("p1"), "msg_1", text.clone()),
        boundary,
        summary,
        human("p2", Some("u0"), "second try"),
        reply("a2", Some("p2"), "msg_2", text),
    ]);
    assert!(
        !s.entries
            .iter()
            .any(|e| matches!(e.kind, EntryKind::Compaction(_)))
    );
    let ids: Vec<_> = s.entries.iter().filter_map(|e| e.id.as_deref()).collect();
    for e in &s.entries {
        if let Some(p) = &e.parent_id {
            assert!(ids.contains(&p.as_str()), "dangling parent {p}");
        }
    }
    assert!(
        s.warnings
            .iter()
            .any(|w| w.message.starts_with("3 entries on branches"))
    );
}

/// A named agent and a message to it carry the agent's name: `Agent.name`, `SendMessage.to`.
#[test]
fn a_named_agent_and_a_message_to_it_carry_its_name() {
    let calls = [
        (
            "sp",
            "Agent",
            json!({"description": "Build E1", "prompt": "p", "name": "e1-langfuse", "run_in_background": true}),
            json!({"status": "async_launched", "agentId": "ae1-langfuse-14ff", "prompt": "p"}),
        ),
        (
            "sm",
            "SendMessage",
            json!({"to": "e1-langfuse", "summary": "E1 accepted", "message": "Merge it."}),
            json!({"message": "Message sent to e1-langfuse's inbox"}),
        ),
    ];
    let s = transcript(&launched(&calls));
    let name = |id| match &call(&s, id).tool {
        Tool::Task { name, .. } => name.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(name("sp").as_deref(), Some("e1-langfuse"));
    assert_eq!(name("sm").as_deref(), Some("e1-langfuse"));
    assert!(matches!(
        &call(&s, "sm").tool,
        Tool::Task { description: Some(d), .. } if d == "E1 accepted"
    ));
}
