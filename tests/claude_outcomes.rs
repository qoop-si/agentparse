//! Claude call outcomes read from the result text when the line has no `toolUseResult`, task
//! notifications that carry a report, and typed questions. Shapes come from real transcripts
//! (session and line named on each test); ids are kept, prompts and paths are shortened.

use agentparse::{Agent, Session, Source, Tool, ToolCall, ToolStatus, parse_file, parse_reader};
use serde_json::{Value, json};
use std::path::Path;

const TS: &str = "2026-11-07T09:00:00.000Z";

fn line(kind: &str, uuid: &str, fields: Value) -> Value {
    let mut v = json!({"type": kind, "uuid": uuid, "parentUuid": null, "timestamp": TS,
                       "sessionId": "s1", "cwd": "/w"});
    if let (Value::Object(v), Value::Object(f)) = (&mut v, fields) {
        v.extend(f);
    }
    v
}

fn calls(uuid: &str, blocks: Value) -> Value {
    let message = json!({"id": format!("msg_{uuid}"), "model": "m", "role": "assistant", "content": blocks,
                         "stop_reason": "tool_use", "usage": {"input_tokens": 1, "output_tokens": 1}});
    line("assistant", uuid, json!({ "message": message }))
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

/// A result line: `tur` is its `toolUseResult` (`Null` for none), `more` other line keys.
fn result(call: &str, content: Value, is_error: bool, tur: Value, more: Value) -> Value {
    let block = json!({"type": "tool_result", "tool_use_id": call, "content": content, "is_error": is_error});
    let mut fields = json!({"message": {"role": "user", "content": [block]}});
    if !tur.is_null() {
        fields["toolUseResult"] = tur;
    }
    if let (Value::Object(f), Value::Object(m)) = (&mut fields, more) {
        f.extend(m);
    }
    line("user", &format!("r-{call}"), fields)
}

fn notification(uuid: &str, body: &str) -> Value {
    let fields = json!({"origin": {"kind": "task-notification"},
                        "message": {"role": "user", "content": format!("<task-notification>\n{body}\n</task-notification>")}});
    line("user", uuid, fields)
}

fn transcript(lines: &[Value]) -> Session {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let source = Source::File {
        path: "s1.jsonl".into(),
    };
    parse_reader(Agent::Claude, text.as_bytes(), source).unwrap()
}

/// One call and its result line.
fn one(name: &str, input: Value, content: Value, tur: Value) -> Session {
    transcript(&[
        calls("a1", json!([tool_use("c1", name, input)])),
        result("c1", content, false, tur, json!({})),
    ])
}

fn call<'a>(s: &'a Session, id: &str) -> &'a ToolCall {
    s.tool_calls()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no call {id}"))
}

fn data(c: &ToolCall) -> Value {
    c.output
        .as_ref()
        .and_then(|o| o.data.clone())
        .unwrap_or(Value::Null)
}

fn background_id(c: &ToolCall) -> Option<&str> {
    c.output.as_ref()?.shell.as_ref()?.background_id.as_deref()
}

fn texts(blocks: &[&str]) -> Value {
    json!(
        blocks
            .iter()
            .map(|t| json!({"type": "text", "text": t}))
            .collect::<Vec<_>>()
    )
}

const ASYNC_LAUNCH: &str = "Async agent launched successfully. (This tool result is internal metadata, never quote or paste any part of it, including the agentId below, into a user-facing reply.)\nagentId: a6072920f90923f73 (internal ID - do not mention to user. Use SendMessage with to: 'a6072920f90923f73', summary: '<5-10 word recap>' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes.\noutput_file: /private/tmp/claude-501/demo/tasks/a6072920f90923f73.output";

/// ap-claude-structures#1: 6332d703 lines 208-209, an async Agent whose result line has no
/// `toolUseResult`. The launch and the child id are in the text.
#[test]
fn async_agent_launched_without_tool_use_result_is_pending_with_its_child() {
    let input = json!({"description": "Linux fresh install e2e", "prompt": "p", "run_in_background": true, "subagent_type": "general-purpose"});
    let s = one("Agent", input, texts(&[ASYNC_LAUNCH]), Value::Null);
    let c = call(&s, "c1");
    assert_eq!(
        (c.status, c.child_session.as_deref(), c.ended_at),
        (ToolStatus::Pending, Some("a6072920f90923f73"), None)
    );
}

/// ap-claude-structures#2: mocker 63ab4334 line 16, a foreground Agent with no `toolUseResult`.
/// It finished, so it stays Ok, but its child and step count are in the trailer block.
#[test]
fn foreground_agent_text_gives_its_child_and_step_count() {
    let report = "**Bottom line**: voice input is available.\n\nagentId: in a quoted log line";
    let trailer = "agentId: a053fa4a031effa2b (use SendMessage with to: 'a053fa4a031effa2b' to continue this agent)\n<usage>subagent_tokens: 29931\ntool_uses: 5\nduration_ms: 28232</usage>";
    let input = json!({"description": "Claude Code terminal voice input", "prompt": "p", "subagent_type": "general-purpose"});
    let s = one("Agent", input, texts(&[report, trailer]), Value::Null);
    let c = call(&s, "c1");
    assert_eq!(
        (c.status, c.child_session.as_deref()),
        (ToolStatus::Ok, Some("a053fa4a031effa2b"))
    );
    assert_eq!(data(c)["totalToolUseCount"], json!(5));
}

/// gap 1 (launch half): the real text-only Workflow launch of crw-opencore issue-89 session
/// 1c97d622 line 60 (its tool_use line 59 with the script cut short). The run id appears a
/// second time inside the resume hint.
#[test]
fn workflow_text_launch_gives_task_run_and_summary() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude/outcomes/workflow-text-launch.jsonl");
    let s = parse_file(&path).unwrap();
    let c = call(&s, "toolu_01AbndhJ97v8oSesFwqnvJDH");
    let d = data(c);
    assert_eq!(
        (
            c.status,
            d["taskId"].as_str(),
            d["runId"].as_str(),
            c.child_session.as_deref()
        ),
        (
            ToolStatus::Pending,
            Some("wtbva73ov"),
            Some("wf_2a5b55a4-5fc"),
            None
        )
    );
    let summary =
        "Plan + senior multi-agent review loop for crw MCP outputSchema mismatch (issue #89)";
    assert_eq!(d["summary"], json!(summary));
    match &c.tool {
        Tool::Task { description, .. } => assert_eq!(description.as_deref(), Some(summary)),
        other => panic!("not a task: {other:?}"),
    }
}

const SPAWNED: &str = "Spawned successfully. (This tool result is internal metadata, never quote or paste any part of it, including the ID below, into a user-facing reply.)\nagent_id: aprovider-research-561dfe84cf55cbd8\nname: provider-research\nThe agent is now running and will receive instructions via mailbox.";

/// ap-claude-structures#3 (spawn half): esim 51749b4d, a teammate spawn keeps running after its
/// acknowledgement, with or without a `toolUseResult`.
#[test]
fn a_teammate_spawn_is_pending() {
    let input = json!({"description": "provider research", "prompt": "p", "name": "provider-research", "team_name": "session-51749b4d"});
    let tur = json!({"status": "teammate_spawned", "agentId": "aprovider-research-561dfe84cf55cbd8",
        "teammate_id": "provider-research@session-51749b4d", "agent_id": "aprovider-research-561dfe84cf55cbd8",
        "name": "provider-research", "team_name": "session-51749b4d"});
    for tur in [tur, Value::Null] {
        let s = one("Agent", input.clone(), texts(&[SPAWNED]), tur);
        let c = call(&s, "c1");
        assert_eq!(
            (c.status, c.child_session.as_deref()),
            (
                ToolStatus::Pending,
                Some("aprovider-research-561dfe84cf55cbd8")
            )
        );
        assert_eq!(data(c)["status"], json!("teammate_spawned"));
    }
}

/// ap-claude-structures#1 and #5: Bash, Monitor and MCP launches named only in the text are
/// Pending under their task id, so the notification that names that id ends them.
#[test]
fn background_launches_named_in_the_text_are_pending_until_reported() {
    let launches = [
        (
            "bash",
            "Bash",
            json!({"command": "make", "run_in_background": true}),
            "Command running in background with ID: bnxheai18. Output is being written to: /private/tmp/demo/tasks/bnxheai18.output. You will be notified when it completes.",
            "bnxheai18",
        ),
        (
            "moved",
            "Bash",
            json!({"command": "make test"}),
            "1\nCommand did not complete within its 120s timeout and was moved to the background (ID: bhvzoino5). Output is being written to: /private/tmp/demo/tasks/bhvzoino5.output. You will be notified when it completes.",
            "bhvzoino5",
        ),
        (
            "mon",
            "Monitor",
            json!({"command": "tail -f x", "description": "watch"}),
            "Monitor started (task bas7urk1x, expires in 30m unless the source ends first; you get one notice at expiry, re-arm if you still need the watch). You will be notified on each event.",
            "bas7urk1x",
        ),
        (
            "mcp",
            "mcp__playwright__browser_run_code_unsafe",
            json!({"code": "1"}),
            "MCP tool \"playwright/browser_run_code_unsafe\" is still running after 120s. It was moved to the background as task kgt8jt9hy and keeps running; you'll receive a notification with the result when it completes. To stop it, use TaskStop with task_id \"kgt8jt9hy\".",
            "kgt8jt9hy",
        ),
    ];
    let blocks: Vec<Value> = launches
        .iter()
        .map(|(id, name, input, _, _)| tool_use(id, name, input.clone()))
        .collect();
    let mut lines = vec![calls("a1", json!(blocks))];
    for (id, _, _, text, _) in &launches {
        lines.push(result(id, json!(text), false, Value::Null, json!({})));
    }
    let launched = transcript(&lines);
    let got: Vec<_> = launches
        .iter()
        .map(|(id, ..)| {
            let c = call(&launched, id);
            (c.status, background_id(c))
        })
        .collect();
    let want: Vec<_> = launches
        .iter()
        .map(|(.., task)| (ToolStatus::Pending, Some(*task)))
        .collect();
    assert_eq!(got, want);
    // esim 51749b4d line 295: the MCP report names only the task.
    lines.push(notification(
        "n1",
        "<task-id>kgt8jt9hy</task-id>\n<status>completed</status>\n<summary>MCP tool finished</summary>",
    ));
    let s = transcript(&lines);
    assert_eq!(call(&s, "mcp").status, ToolStatus::Ok);
}

/// ap-claude-main#4: 6332d703 line 222, a ToolSearch result made only of `tool_reference` blocks.
#[test]
fn tool_search_names_come_from_its_references() {
    let content = json!([
        {"type": "tool_reference", "tool_name": "mcp__playwright__browser_navigate"},
        {"type": "tool_reference", "tool_name": "mcp__playwright__browser_snapshot"}
    ]);
    let input = json!({"query": "select:mcp__playwright__browser_navigate,mcp__playwright__browser_snapshot", "max_results": 2});
    let s = one("ToolSearch", input, content, Value::Null);
    assert_eq!(
        data(call(&s, "c1"))["matches"],
        json!([
            "mcp__playwright__browser_navigate",
            "mcp__playwright__browser_snapshot"
        ])
    );
}

/// gap 5 (result half) and subagents-ui#9: 6332d703 call toolu_0145uj8W, launched at line 209
/// and reported at line 502. The report lands on the call, the launch text moves to `data`, and
/// the step count comes from `<usage>`.
#[test]
fn a_completed_report_lands_on_its_call() {
    let input =
        json!({"description": "Linux fresh install e2e", "prompt": "p", "run_in_background": true});
    let report = "Report: `/w/reviews/2026-10-30-launch-linux-install.md`\n\n1. Install via `curl | sh`: PASS.";
    let body = format!(
        "<task-id>a6072920f90923f73</task-id>\n<tool-use-id>c1</tool-use-id>\n<output-file>/t/a6072920f90923f73.output</output-file>\n<status>completed</status>\n<summary>Agent \"Linux fresh install e2e\" finished</summary>\n<note>A task-notification fires each time this agent stops with no live background children of its own.</note>\n<result>{report}</result>\n<usage><subagent_tokens>111497</subagent_tokens><tool_uses>49</tool_uses><duration_ms>613132</duration_ms></usage>"
    );
    let s = transcript(&[
        calls("a1", json!([tool_use("c1", "Agent", input)])),
        result("c1", texts(&[ASYNC_LAUNCH]), false, Value::Null, json!({})),
        notification("n1", &body),
    ]);
    let c = call(&s, "c1");
    let out = c.output.as_ref().unwrap();
    assert_eq!(c.status, ToolStatus::Ok);
    assert!(out.text.starts_with("Report: "), "text: {}", out.text);
    let d = data(c);
    assert!(
        d["launch"]
            .as_str()
            .is_some_and(|l| l.starts_with("Async agent launched")),
        "data: {d}"
    );
    assert_eq!(d["totalToolUseCount"], json!(49));
}

fn ask(questions: &[(&str, &str)]) -> Value {
    let qs: Vec<Value> = questions
        .iter()
        .map(|(header, q)| {
            json!({"question": q, "header": header, "multiSelect": false, "options": [
                {"label": "Yes (Recommended)", "description": "Do it now."},
                {"label": "No", "description": "Leave it."}
            ]})
        })
        .collect();
    json!({ "questions": qs })
}

fn tool_json(c: &ToolCall) -> Value {
    serde_json::to_value(&c.tool).unwrap()
}

const Q3: [(&str, &str); 3] = [
    ("Blog", "How should publishing start?"),
    ("License", "Which license, and when?"),
    ("CLA", "Should outside contributors sign a CLA?"),
];

/// gap 4: a 3-question call answered with no `toolUseResult` (874 of 963 answered results in
/// the corpus). An answer may hold quotes; the pairs are found by their known questions.
#[test]
fn a_question_takes_every_answer_from_the_text() {
    let text = "Your questions have been answered: \"How should publishing start?\"=\"14 posts, 1 a day (Recommended)\", \"Which license, and when?\"=\"AGPL, \"for now\", ok\", \"Should outside contributors sign a CLA?\"=\"No\". You can now continue with these answers in mind.";
    let s = one("AskUserQuestion", ask(&Q3), json!(text), Value::Null);
    let c = call(&s, "c1");
    let t = tool_json(c);
    let got: Vec<_> = t["questions"]
        .as_array()
        .map(|qs| {
            qs.iter()
                .map(|q| {
                    (
                        q["header"].clone(),
                        q["question"].clone(),
                        q["answer"].clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        got,
        [
            (
                json!("Blog"),
                json!("How should publishing start?"),
                json!("14 posts, 1 a day (Recommended)")
            ),
            (
                json!("License"),
                json!("Which license, and when?"),
                json!("AGPL, \"for now\", ok")
            ),
            (
                json!("CLA"),
                json!("Should outside contributors sign a CLA?"),
                json!("No")
            ),
        ]
    );
    assert_eq!(
        t["questions"][0]["options"][0],
        json!({"label": "Yes (Recommended)", "description": "Do it now."})
    );
    assert_eq!((c.status, t.get("declined")), (ToolStatus::Ok, None));
}

/// gap 4: answers from `toolUseResult.answers` when the line has one ("The user answered:" wording).
#[test]
fn a_question_takes_its_answers_from_the_tool_use_result() {
    let input = ask(&Q3[..1]);
    let tur = json!({"questions": input["questions"], "answers": {"How should publishing start?": "Not yet"}, "annotations": {}});
    let text = "The user answered: \"How should publishing start?\"=\"Not yet\". Read the answers carefully.";
    let s = one("AskUserQuestion", input, json!(text), tur);
    assert_eq!(
        tool_json(call(&s, "c1"))["questions"][0]["answer"],
        json!("Not yet")
    );
}

/// gap 4: a rejected question is declined; the clarify choice (crw ae032c21 line 4195) is not a
/// rejection.
#[test]
fn declined_and_clarify_questions() {
    let input = ask(&Q3[..1]);
    let rejected = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.";
    let clarify = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). To tell you how to proceed, the user said:\nThe user wants to clarify these questions.\n    This means they may have additional information, context or questions for you.\n\n    Questions asked:\n- \"How should publishing start?\"\n  (No answer provided)";
    let denied = json!({"toolDenialKind": "user-rejected"});
    let run = |text: &str| {
        let s = transcript(&[
            calls(
                "a1",
                json!([tool_use("c1", "AskUserQuestion", input.clone())]),
            ),
            result(
                "c1",
                json!(text),
                true,
                json!(format!("Error: {text}")),
                denied.clone(),
            ),
        ]);
        let c = call(&s, "c1");
        (
            c.status,
            c.status_detail.clone(),
            tool_json(c)["declined"].clone(),
        )
    };
    assert_eq!(
        run(rejected),
        (
            ToolStatus::Rejected,
            Some("user-rejected".into()),
            json!(true)
        )
    );
    assert_eq!(
        run(clarify),
        (ToolStatus::Ok, Some("clarify".into()), Value::Null)
    );
}

/// A stored question from before the fields existed still reads, and an empty one writes the same.
#[test]
fn an_empty_question_keeps_its_old_shape() {
    let old = json!({"kind": "question"});
    let tool: Tool = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&tool).unwrap(), old);
}

/// Review AP-1 r1: Claude appends ` selected preview:` (3611033b, no `toolUseResult`) or
/// ` user notes:` after an answer's closing quote; the answer ends at that marker.
#[test]
fn an_annotated_answer_ends_at_its_marker() {
    let answers = |qs: &[(&str, &str)], text: &str| {
        let s = one("AskUserQuestion", ask(qs), json!(text), Value::Null);
        let t = tool_json(call(&s, "c1"));
        (0..qs.len())
            .map(|i| t["questions"][i]["answer"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        answers(
            &Q3[..1],
            "Your questions have been answered: \"How should publishing start?\"=\"Mix\" selected preview:\n#3 -> shots\n\n= each ad. You can now continue with these answers in mind."
        ),
        [json!("Mix")]
    );
    assert_eq!(
        answers(
            &Q3[..2],
            "Your questions have been answered: \"How should publishing start?\"=\"Mix\" user notes: soon, \"Which license, and when?\"=\"AGPL\". You can now continue with these answers in mind."
        ),
        [json!("Mix"), json!("AGPL")]
    );
    assert_eq!(
        answers(
            &Q3[..2],
            "Your questions have been answered: \"How should publishing start?\"=\"Mix\" selected preview:\n\"a\" b\n, \"Which license, and when?\"=\"Wide\" user notes: keep \"cards\". later. You can now continue with these answers in mind."
        ),
        [json!("Mix"), json!("Wide")]
    );
}

/// Review AP-1 r1: a foreground command whose output prints a launch line (a grep of
/// transcripts) is no launch. The harness frames real launches with a leading `Shell cwd was
/// reset to` line or a trailing `Session cwd remains` line (both in the corpus).
#[test]
fn only_the_harness_launch_text_starts_a_background_bash() {
    let runs = [
        (
            "grep",
            "x.jsonl:1:Command running in background with ID: bnxheai18. Output is being written to: /t/bnxheai18.output.\nCommand running in background with ID: bnxheai18. Output is being written to: /t/bnxheai18.output.\nCommand did not complete within its 120s timeout and was moved to the background (ID: bhvzoino5). Output is being written to: /t/b.output.\ndone",
            None,
        ),
        (
            "reset",
            "Shell cwd was reset to /Users/us/coding/qoop/qoop\nCommand running in background with ID: bk1. Output is being written to: /t/bk1.output. You will be notified when it completes.",
            Some("bk1"),
        ),
        (
            "remains",
            "1\nCommand did not complete within its 300s timeout and was moved to the background (ID: bk2). Output is being written to: /t/bk2.output. You will be notified when it completes.\nSession cwd remains /w; directory changes made by the backgrounded command do not apply to subsequent commands.",
            Some("bk2"),
        ),
    ];
    let got: Vec<_> = runs
        .iter()
        .map(|(_, text, _)| {
            let s = one("Bash", json!({"command": "x"}), json!(text), Value::Null);
            let c = call(&s, "c1");
            (c.status, background_id(c).map(String::from))
        })
        .collect();
    let want: Vec<_> = runs
        .iter()
        .map(|(_, _, id)| match id {
            Some(id) => (ToolStatus::Pending, Some(id.to_string())),
            None => (ToolStatus::Ok, None),
        })
        .collect();
    assert_eq!(got, want);
}
