//! Codex `/review` rollouts and commands that end after their code cell, on inline records shaped
//! like the real rollouts named in each test (values are fake).

use agentparse::{Agent, EntryKind, Notice, Origin, Part, Role, Session, Source, ToolCall};
use serde_json::{Value, json};

fn rec(ty: &str, payload: Value) -> Value {
    json!({"timestamp": "2026-11-07T03:17:56.100Z", "type": ty, "payload": payload})
}

fn item(thread: &str, item: Value) -> Value {
    rec(
        "event_msg",
        json!({"type": "item_completed", "thread_id": thread, "turn_id": "t1", "item": item,
            "started_at_ms": 1, "completed_at_ms": 2}),
    )
}

fn command(thread: &str, id: &str, process: &str, cmd: &str) -> Value {
    item(
        thread,
        json!({"type": "CommandExecution", "id": id, "process_id": process,
            "command": ["/bin/zsh", "-lc", cmd], "cwd": "file:///work/demo", "parsed_cmd": [],
            "source": "unified_exec_startup", "status": "completed", "exit_code": 0,
            "aggregated_output": "ok\n", "duration": {"secs": 1, "nanos": 0}}),
    )
}

fn parse(path: &str, meta: Value, records: &[Value]) -> Session {
    let mut lines = vec![rec("session_meta", meta).to_string()];
    lines.extend(records.iter().map(Value::to_string));
    let source = Source::File { path: path.into() };
    agentparse::parse_reader(Agent::Codex, lines.join("\n").as_bytes(), source).unwrap()
}

const PROMPT: &str = "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings.";
const VERDICT: &str = "No actionable regressions found.";

/// The `codex exec review` parent, rollout 2026-11-07T06-17-56 `01a1b403-4697-...`: lines 3-35
/// are the review sub-agent's items, tagged with its thread id `01a1b403-4830-...`.
fn review_parent() -> Session {
    let child = "01a1b403-4830";
    parse(
        "rollout-2026-11-07T06-17-56-01a1b403-4697.jsonl",
        json!({"id": "01a1b403-4697", "cwd": "/work/demo", "source": "exec", "history_mode": "paginated"}),
        &[
            item(
                "01a1b403-4697",
                json!({"type": "EnteredReviewMode", "id": "r1", "target": {"type": "uncommittedChanges"},
                    "user_facing_hint": "current changes"}),
            ),
            rec(
                "event_msg",
                json!({"type": "task_started", "turn_id": "t2", "model_context_window": 475000}),
            ),
            item(
                child,
                json!({"type": "UserMessage", "id": "u1", "content": [{"type": "text", "text": PROMPT, "text_elements": []}]}),
            ),
            command(child, "exec-1", "11", "git status --short"),
            item(
                child,
                json!({"type": "Reasoning", "id": "rs1", "summary_text": [], "raw_content": []}),
            ),
            command(child, "exec-2", "12", "git diff"),
            item(
                child,
                json!({"type": "Reasoning", "id": "rs2", "summary_text": [], "raw_content": []}),
            ),
            item(
                "01a1b403-4697",
                json!({"type": "ExitedReviewMode", "id": "r2", "review_output": {"findings": [],
                    "overall_correctness": "patch is correct", "overall_explanation": VERDICT,
                    "overall_confidence_score": 0.82}}),
            ),
            rec(
                "response_item",
                json!({"type": "message", "id": "m1", "role": "assistant",
                    "content": [{"type": "output_text", "text": VERDICT}]}),
            ),
            item(
                "01a1b403-4697",
                json!({"type": "AgentMessage", "id": "m1", "content": [{"type": "Text", "text": VERDICT}]}),
            ),
            rec(
                "event_msg",
                json!({"type": "task_complete", "turn_id": "t1", "duration_ms": 122890}),
            ),
        ],
    )
}

fn notices(s: &Session) -> Vec<&Notice> {
    (s.entries.iter())
        .filter_map(|e| match &e.kind {
            EntryKind::Notice(n) => Some(n),
            _ => None,
        })
        .collect()
}

fn others(s: &Session) -> Vec<&str> {
    (s.entries.iter())
        .filter_map(|e| match &e.kind {
            EntryKind::Other(o) => Some(o.kind.as_str()),
            _ => None,
        })
        .collect()
}

/// ap-other-backends#1: the sub-agent's items are its own file's, not orphan results here.
#[test]
fn a_review_parent_drops_the_sub_agent_items() {
    let s = review_parent();
    let warnings: Vec<_> = s.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(warnings, Vec::<&str>::new());
    let users: Vec<_> = (s.messages())
        .filter(|m| m.role == Role::User)
        .map(|m| (m.origin, m.text()))
        .collect();
    assert_eq!(users, []);
    let texts: Vec<_> = (s.messages())
        .filter(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .collect();
    assert_eq!(texts, [VERDICT]);
    // No prompt of its own: titled by the review, as Codex names the thread.
    assert_eq!(s.title.as_deref(), Some("Review: current changes"));
}

/// An event carrying the id `session_meta` gave is kept even when the file name names another
/// thread (a forked or renamed rollout), and with no `session_meta` (a tail read) nothing is
/// dropped.
#[test]
fn only_a_foreign_thread_id_drops_an_event() {
    let said = |s: &Session| -> Vec<String> { s.messages().map(|m| m.text()).collect() };
    let msg = |thread: &str| {
        item(
            thread,
            json!({"type": "AgentMessage", "id": "m1", "content": [{"type": "Text", "text": "hi"}]}),
        )
    };
    let renamed = parse(
        "rollout-2026-11-07T06-17-56-01a1b403-9999.jsonl",
        json!({"id": "01a1b403-4697", "cwd": "/w", "history_mode": "paginated"}),
        &[msg("01a1b403-4697")],
    );
    assert_eq!(said(&renamed), ["hi"]);
    let window = msg("01a1b403-4830").to_string();
    let source = Source::File {
        path: "rollout-2026-11-07T06-17-56-01a1b403-4697.jsonl".into(),
    };
    let tail = agentparse::parse_reader(Agent::Codex, window.as_bytes(), source).unwrap();
    assert_eq!(said(&tail), ["hi"]);
}

/// ref-paperclip-parse#1, ap-other-backends#2: review mode records are notices with the verdict.
#[test]
fn review_mode_records_are_notices() {
    let s = review_parent();
    let got: Vec<_> = (notices(&s).iter())
        .map(|n| (n.kind.as_deref(), n.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (Some("EnteredReviewMode"), "Review: current changes"),
            (
                Some("ExitedReviewMode"),
                "Review done, patch is correct, 0 findings: No actionable regressions found."
            ),
        ]
    );
    let left: Vec<_> = (others(&s).into_iter())
        .filter(|k| k.contains("Review"))
        .collect();
    assert_eq!(left, Vec::<&str>::new());
}

/// ap-other-backends#8: the review child (rollout `01a1b403-4830-...`, `source.subagent` "review")
/// names its type, and its built-in prompt is the harness's, not typed by a human.
#[test]
fn a_review_child_has_its_type_and_a_command_prompt() {
    let s = parse(
        "rollout-2026-11-07T06-17-56-01a1b403-4830.jsonl",
        json!({"id": "01a1b403-4830", "parent_thread_id": "01a1b403-4697", "cwd": "/work/demo",
            "source": {"subagent": "review"}, "history_mode": "paginated"}),
        &[
            rec(
                "response_item",
                json!({"type": "message", "id": "p1", "role": "user",
                    "content": [{"type": "input_text", "text": PROMPT}],
                    "internal_chat_message_metadata_passthrough": {"turn_id": "t2", "content_item_kinds": ["user.text"]}}),
            ),
            item(
                "01a1b403-4830",
                json!({"type": "UserMessage", "id": "u1", "content": [{"type": "text", "text": PROMPT, "text_elements": []}]}),
            ),
        ],
    );
    let parent = s.parent.as_ref().expect("parent link");
    assert_eq!(
        (parent.session_id.as_str(), parent.agent_type.as_deref()),
        ("01a1b403-4697", Some("review"))
    );
    let users: Vec<_> = (s.messages())
        .filter(|m| m.role == Role::User)
        .map(|m| (m.origin, m.text()))
        .collect();
    assert_eq!(users, [(Origin::Command, PROMPT.to_owned())]);
    assert_eq!(s.title.as_deref(), Some(PROMPT));
}

/// An interrupted review leaves `review_output` null (Codex's `Option`), and the findings count
/// says one or many.
#[test]
fn a_review_end_names_its_findings_or_no_result() {
    let end = |out: Value| {
        let s = parse(
            "rollout-2026-11-07T06-17-56-01a1b403-4697.jsonl",
            json!({"id": "01a1b403-4697", "cwd": "/w", "history_mode": "paginated"}),
            &[item(
                "01a1b403-4697",
                json!({"type": "ExitedReviewMode", "id": "r2", "review_output": out}),
            )],
        );
        notices(&s)
            .iter()
            .map(|n| n.text.clone())
            .collect::<Vec<_>>()
    };
    let verdict = |n: usize| {
        json!({"findings": vec![json!({"title": "x"}); n], "overall_correctness": "patch is incorrect",
            "overall_explanation": "See findings."})
    };
    assert_eq!(end(Value::Null), ["Review ended without a result"]);
    assert_eq!(
        end(verdict(1)),
        ["Review done, patch is incorrect, 1 finding: See findings."]
    );
    assert_eq!(
        end(verdict(2)),
        ["Review done, patch is incorrect, 2 findings: See findings."]
    );
}

/// ap-other-backends#4: rollout 2026-11-06T03-34-36 ordinals 65-73. A cell prints each
/// `Promise.allSettled` result as `{i, status, value}`; `bun run typecheck` was still running
/// (`value.session_id` 40311) when the cell ended, and its item (process 40311) lands later.
#[test]
fn a_settled_command_that_ends_after_its_cell_joins_it() {
    let program = "const rs=await Promise.allSettled([\ntools.exec_command({cmd:\"bun run typecheck\",\"yield_time_ms\":1000}),\ntools.exec_command({cmd:\"git diff\"})\n]);rs.forEach((r,i)=>text({i,...r}));\n";
    let printed =
        |i: u32, value: Value| json!({"i": i, "status": "fulfilled", "value": value}).to_string();
    let output: Vec<_> = [
        "Script completed\nWall time 1.2 seconds\nOutput:\n".to_owned(),
        printed(
            0,
            json!({"chunk_id": "7bf02b", "wall_time_seconds": 1.0, "session_id": 40311, "output": "$ tsc --noEmit\n"}),
        ),
        printed(
            1,
            json!({"chunk_id": "419466", "wall_time_seconds": 0.8, "exit_code": 0, "output": "ok\n"}),
        ),
    ]
    .iter()
    .map(|t| json!({"type": "input_text", "text": t}))
    .collect();
    let s = parse(
        "rollout-2026-11-06T03-34-36-01a1ae47.jsonl",
        json!({"id": "01a1ae47", "cwd": "/work/demo", "history_mode": "paginated"}),
        &[
            rec(
                "response_item",
                json!({"type": "custom_tool_call", "call_id": "cell", "name": "exec", "input": program}),
            ),
            command("01a1ae47", "exec-2", "14204", "git diff"),
            rec(
                "response_item",
                json!({"type": "custom_tool_call_output", "call_id": "cell", "output": output}),
            ),
            command("01a1ae47", "exec-1", "40311", "bun run typecheck"),
        ],
    );
    let warnings: Vec<_> = s.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(warnings, Vec::<&str>::new());
    let cells: Vec<&ToolCall> = (s.messages())
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            Part::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 1);
    let children: Vec<_> = (cells[0].children.iter())
        .map(|c| {
            let shell = c.output.as_ref().and_then(|o| o.shell.as_ref());
            (
                c.id.as_str(),
                shell.and_then(|s| s.background_id.as_deref()),
            )
        })
        .collect();
    assert_eq!(children, [("exec-2", None), ("exec-1", Some("40311"))]);
}
