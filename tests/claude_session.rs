//! Claude session state across records: the human input queue, teammates and their lifecycle,
//! the order of a merged message, task list ids, and what a subagent hands back. Shapes come from
//! real transcripts (session and line named on each test); ids are kept, texts are shortened.

use agentparse::{
    Agent, EntryKind, Message, Origin, Part, Role, Session, Source, Tool, ToolCall, ToolStatus,
    parse_file, parse_reader,
};
use serde_json::{Value, json};

const TS: &str = "2026-11-07T09:00:00.000Z";

fn line(kind: &str, uuid: &str, fields: Value) -> Value {
    let mut v = json!({"type": kind, "uuid": uuid, "parentUuid": null, "timestamp": TS,
                       "sessionId": "s1", "cwd": "/w"});
    if let (Value::Object(v), Value::Object(f)) = (&mut v, fields) {
        v.extend(f);
    }
    v
}

/// One line of the API message `msg`, holding `blocks`.
fn assistant(uuid: &str, msg: &str, blocks: Value, output_tokens: u64) -> Value {
    let message = json!({"id": msg, "model": "m", "role": "assistant", "content": blocks,
                         "stop_reason": "tool_use", "usage": {"input_tokens": 1, "output_tokens": output_tokens}});
    line("assistant", uuid, json!({ "message": message }))
}

fn calls(uuid: &str, blocks: Value) -> Value {
    assistant(uuid, &format!("msg_{uuid}"), blocks, 1)
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn result(call: &str, content: Value, tur: Value) -> Value {
    let block = json!({"type": "tool_result", "tool_use_id": call, "content": content});
    let mut fields = json!({"message": {"role": "user", "content": [block]}});
    if !tur.is_null() {
        fields["toolUseResult"] = tur;
    }
    line("user", &format!("r-{call}"), fields)
}

fn user(uuid: &str, text: &str, more: Value) -> Value {
    let mut fields = json!({"message": {"role": "user", "content": text}});
    if let (Value::Object(f), Value::Object(m)) = (&mut fields, more) {
        f.extend(m);
    }
    line("user", uuid, fields)
}

fn system(uuid: &str, subtype: &str, more: Value) -> Value {
    let mut fields = json!({ "subtype": subtype });
    if let (Value::Object(f), Value::Object(m)) = (&mut fields, more) {
        f.extend(m);
    }
    line("system", uuid, fields)
}

fn enqueue(text: &str) -> Value {
    json!({"type": "queue-operation", "operation": "enqueue", "timestamp": TS, "sessionId": "s1", "content": text})
}

fn dequeue() -> Value {
    json!({"type": "queue-operation", "operation": "dequeue", "timestamp": TS, "sessionId": "s1"})
}

fn text_of(lines: &[Value]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn transcript(lines: &[Value]) -> Session {
    let source = Source::File {
        path: "s1.jsonl".into(),
    };
    parse_reader(Agent::Claude, text_of(lines).as_bytes(), source).unwrap()
}

fn call<'a>(s: &'a Session, id: &str) -> &'a ToolCall {
    s.tool_calls()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no call {id}"))
}

fn queued(s: &Session) -> Vec<String> {
    s.messages()
        .filter(|m| m.phase.as_deref() == Some("queued"))
        .map(Message::text)
        .collect()
}

const KILLED: &str = "3 background agents were stopped by the user: \"Verify blog posts batch A\", \"Verify blog posts batch B\", \"Build home page round 10\".";

/// ap-claude-main#1 and #2: 6332d703 lines 8301-8303. The harness enqueues the words of an
/// `agents_killed` record, which has no text of its own, the same moment it writes the record
/// (xheron 53b86fc1 lines 66460-66462: a millisecond before). Read up to there (a live tail),
/// that enqueue is no typed input and is the notice's text.
#[test]
fn agents_killed_takes_its_queued_text_and_queues_nothing() {
    for at in [TS, "2026-11-07T08:59:59.999Z"] {
        let mut queue = enqueue(KILLED);
        queue["timestamp"] = json!(at);
        let s = transcript(&[
            user("u1", "verify the posts", json!({})),
            queue,
            system(
                "t1",
                "turn_duration",
                json!({"durationMs": 1, "messageCount": 2}),
            ),
            system("k1", "agents_killed", json!({})),
        ]);
        assert_eq!(queued(&s), Vec::<String>::new(), "queued at {at}");
        let notice = s.entries.iter().find_map(|e| match &e.kind {
            EntryKind::Notice(n) if n.kind.as_deref() == Some("agents_killed") => {
                Some(n.text.clone())
            }
            _ => None,
        });
        assert_eq!(notice.as_deref(), Some(KILLED), "queued at {at}");
    }
}

/// Review r2: `agents_killed` reads its words but leaves them queued. When the harness dequeues
/// them after the record, that dequeue takes the kill words, not the owner's text behind them.
#[test]
fn a_kill_dequeued_after_its_record_keeps_typed_text_queued() {
    let s = transcript(&[
        user("u1", "verify the posts", json!({})),
        enqueue(KILLED),
        system("k1", "agents_killed", json!({})),
        enqueue("and check the logs"),
        dequeue(),
    ]);
    assert_eq!(queued(&s), vec!["and check the logs".to_string()]);
}

const RESET: &str = "Your claude.ai usage limit has reset. Continue the task you were working on when the limit was reached; do not repeat work that is already complete.";

/// ap-claude-main#1: einzel b0915df3 line 3165. The usage limit reset prompt is enqueued and
/// dequeued like typed input; read before its dequeue it is no queued bubble, and it still takes
/// its dequeue, so the owner's own text behind it stays queued.
#[test]
fn harness_queue_texts_are_not_typed_input() {
    let s = transcript(&[user("u1", "go", json!({})), enqueue(RESET)]);
    assert_eq!(queued(&s), Vec::<String>::new());
    let s = transcript(&[
        user("u1", "go", json!({})),
        enqueue(RESET),
        enqueue("and check the logs"),
        dequeue(),
    ]);
    assert_eq!(queued(&s), vec!["and check the logs".to_string()]);
}

/// ap-claude-main#6: crw ae032c21 lines 5993-5997. One API message goes on after a user line (a
/// skill body): the later call comes after that line, and the message's usage counts once.
#[test]
fn a_merged_message_splits_at_an_interleaved_user_line() {
    let msg = "msg_011CgeZxFKTwHpJVD721VDCJ";
    let s = transcript(&[
        assistant(
            "a1",
            msg,
            json!([tool_use(
                "c1",
                "Skill",
                json!({"skill": "workflow-authoring"})
            )]),
            10,
        ),
        result(
            "c1",
            json!("Launching skill: workflow-authoring"),
            Value::Null,
        ),
        user(
            "u1",
            "# Workflow authoring reference",
            json!({"isMeta": true}),
        ),
        assistant(
            "a2",
            msg,
            json!([tool_use("c2", "Bash", json!({"command": "ls"}))]),
            20,
        ),
        result("c2", json!("a.txt"), Value::Null),
        user("u2", "# Second skill body", json!({"isMeta": true})),
        assistant(
            "a3",
            msg,
            json!([tool_use("c3", "Bash", json!({"command": "pwd"}))]),
            30,
        ),
    ]);
    let order: Vec<String> = s
        .messages()
        .map(|m| match m.parts.first() {
            Some(Part::ToolCall(c)) => c.id.clone(),
            _ => m.text(),
        })
        .collect();
    assert_eq!(
        order,
        [
            "c1",
            "# Workflow authoring reference",
            "c2",
            "# Second skill body",
            "c3"
        ]
    );
    // The first piece keeps the bare id; later pieces count on from `#2`.
    let ids: Vec<Option<&str>> = (s.entries.iter())
        .filter(|e| matches!(&e.kind, EntryKind::Message(m) if m.role == Role::Assistant))
        .map(|e| e.id.as_deref())
        .collect();
    let (two, three) = (format!("{msg}#2"), format!("{msg}#3"));
    assert_eq!(ids, [Some(msg), Some(two.as_str()), Some(three.as_str())]);
    assert_eq!(s.usage.output, 30);
}

const SPAWNED: &str = "Spawned successfully. (This tool result is internal metadata, never quote or paste any part of it, including the ID below, into a user-facing reply.)\nagent_id: aprovider-research-561dfe84cf55cbd8\nname: provider-research\nThe agent is now running and will receive instructions via mailbox.";

/// esim 51749b4d line 239: a teammate spawn with no `toolUseResult`.
fn spawn() -> [Value; 2] {
    let input = json!({"description": "eSIM provider API research", "prompt": "p", "name": "provider-research", "run_in_background": "true"});
    [
        calls("a1", json!([tool_use("c1", "Agent", input)])),
        result(
            "c1",
            json!([{"type": "text", "text": SPAWNED}]),
            Value::Null,
        ),
    ]
}

/// ap-claude-structures#3: esim 51749b4d line 2664. The lead stops a teammate by its name; the
/// result names an internal task id, not the agent.
#[test]
fn a_teammate_ends_on_a_task_stop_that_names_it() {
    let tur = json!({"message": "Successfully stopped task: tyjgy1u3f (eSIM provider API research)",
        "task_id": "tyjgy1u3f", "task_type": "in_process_teammate", "command": "eSIM provider API research"});
    let content = json!(tur.to_string());
    for name in ["provider-research", "aprovider-research-561dfe84cf55cbd8"] {
        let [a, r] = spawn();
        let s = transcript(&[
            a,
            r,
            calls(
                "a2",
                json!([tool_use("c2", "TaskStop", json!({"task_id": name}))]),
            ),
            result("c2", content.clone(), tur.clone()),
        ]);
        let c = call(&s, "c1");
        assert_eq!(
            (c.status, c.status_detail.as_deref()),
            (ToolStatus::Interrupted, Some("stopped")),
            "stopped by {name}"
        );
    }
}

/// ap-claude-structures#3: teammates are not in `pendingBackgroundAgentCount` (crw ae032c21: 10
/// spawns at line 855, later counts 1 to 4), so an empty count leaves one running.
#[test]
fn a_teammate_outlives_an_empty_agent_count() {
    let [a, r] = spawn();
    let s = transcript(&[
        a,
        r,
        system(
            "t1",
            "turn_duration",
            json!({"durationMs": 1, "pendingBackgroundAgentCount": 0}),
        ),
    ]);
    assert_eq!(call(&s, "c1").status, ToolStatus::Pending);
}

/// 6332d703 lines 8301-8303: the user's kill names the teammates `blog-verify-a`, `blog-verify-b`
/// and `round10` by their descriptions, so `agents_killed` ends a teammate too.
#[test]
fn agents_killed_ends_a_teammate() {
    let [a, r] = spawn();
    let s = transcript(&[a, r, system("k1", "agents_killed", json!({}))]);
    let c = call(&s, "c1");
    assert_eq!(
        (c.status, c.status_detail.as_deref()),
        (ToolStatus::Interrupted, Some("killed"))
    );
}

/// ap-claude-structures#3: esim 51749b4d line 781. A teammate's idle ping marks its spawn idle;
/// it keeps running (it wakes on the next message).
#[test]
fn an_idle_ping_marks_the_teammate_idle() {
    let ping = "Another Claude session sent a message:\n<teammate-message teammate_id=\"provider-research\" color=\"blue\">\n{\"type\":\"idle_notification\",\"from\":\"provider-research\",\"timestamp\":\"2026-11-07T01:37:01.186Z\",\"result\":\"Done.\"}\n</teammate-message>";
    let [a, r] = spawn();
    let s = transcript(&[a, r, user("p1", ping, json!({}))]);
    let c = call(&s, "c1");
    assert_eq!(
        (c.status, c.status_detail.as_deref()),
        (ToolStatus::Pending, Some("idle"))
    );
}

/// ap-claude-structures#9: esim 51749b4d `subagents/agent-a*.meta.json`. A teammate's meta has
/// its name as `agentType` and no `toolUseId`; the spawn call is found in the lead's file.
#[test]
fn a_teammate_file_links_to_its_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let id = "aprovider-research-561dfe84cf55cbd8";
    std::fs::write(dir.path().join("s1.jsonl"), text_of(&spawn())).unwrap();
    let subagents = dir.path().join("s1/subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    let meta = json!({"agentType": "provider-research", "isBuiltIn": false, "description": "eSIM provider API research",
        "name": "provider-research", "taskKind": "in_process_teammate", "teamName": "session-51749b4d"});
    std::fs::write(
        subagents.join(format!("agent-{id}.meta.json")),
        meta.to_string(),
    )
    .unwrap();
    let first = line(
        "user",
        "t1",
        json!({"isSidechain": true, "agentId": id, "message": {"role": "user", "content": "research"}}),
    );
    let path = subagents.join(format!("agent-{id}.jsonl"));
    std::fs::write(&path, text_of(&[first])).unwrap();
    let s = parse_file(&path).unwrap();
    let parent = s.parent.expect("a parent");
    assert_eq!(
        (
            parent.session_id.as_str(),
            parent.call_id.as_deref(),
            parent.agent_type.as_deref()
        ),
        ("s1", Some("c1"), None)
    );
    assert_eq!(s.extra.unwrap()["meta"]["name"], json!("provider-research"));
}

/// ap-claude-structures#7 (parser half): xheron 53b86fc1 lines 3061 and 4851. A TaskUpdate names
/// its task by number; the number is the one the TaskCreate result gave the subject.
#[test]
fn a_task_update_names_its_task_by_subject() {
    let subject = "Phase 0: infra repo, tracking, branch safety";
    let s = transcript(&[
        calls(
            "a1",
            json!([tool_use(
                "c1",
                "TaskCreate",
                json!({"subject": subject, "description": "d"})
            )]),
        ),
        result(
            "c1",
            json!(format!("Task #5 created successfully: {subject}")),
            Value::Null,
        ),
        calls(
            "a2",
            json!([tool_use(
                "c2",
                "TaskUpdate",
                json!({"taskId": "5", "status": "in_progress"})
            )]),
        ),
    ]);
    match &call(&s, "c2").tool {
        Tool::Todo { items } => assert_eq!(
            (items[0].content.as_str(), items[0].status.as_deref()),
            (subject, Some("in_progress"))
        ),
        other => panic!("not a todo: {other:?}"),
    }
}

const HANDBACK: &str = "<agent-message from=\"a73ea1c1d3db1ada1\">\n[Subagent hand-back] The text below is the final report of a subagent this session delegated to. The report follows:\n  Nothing was modified.\n</agent-message>";

fn queued_command(prompt: &str, origin: Value) -> Value {
    let mut a = json!({"type": "queued_command", "prompt": prompt, "commandMode": "prompt", "isMeta": true});
    if !origin.is_null() {
        a["origin"] = origin;
    }
    line("attachment", "q1", json!({ "attachment": a }))
}

fn first_message(origin: Value) -> (Origin, Option<String>, String) {
    let s = transcript(&[queued_command(HANDBACK, origin)]);
    let m = s.messages().next().unwrap();
    (m.origin, m.author.clone(), m.text())
}

/// real-claude-2#1 (queued shape): 048236c5 line 307. A subagent's report delivered mid-turn as a
/// queued command, with its `origin` record: an agent's message, its sender's name the author.
#[test]
fn a_queued_agent_message_with_its_origin_is_from_an_agent() {
    let origin = json!({"kind": "peer", "from": "a73ea1c1d3db1ada1", "senderTaskId": "a73ea1c1d3db1ada1",
        "name": "general-purpose", "body": "[Subagent hand-back] ...", "handback": true});
    assert_eq!(
        first_message(origin),
        (
            Origin::Agent,
            Some("general-purpose".into()),
            HANDBACK.into()
        )
    );
}

/// gap 5 (hand-back): 048236c5 `subagents/agent-aa09474a10fff30b4.jsonl` lines 19-23. A
/// subagent ends by handing its report back through a tool; the report is its last word.
#[test]
fn a_subagent_hand_back_is_its_final_text() {
    let side = json!({"isSidechain": true, "agentId": "aa09474a10fff30b4"});
    let first = user("u1", "Which functions does calc.py define?", side);
    let mut hand = calls(
        "a1",
        json!([tool_use(
            "c1",
            "SubagentHandback",
            json!({"message": "add, sub"})
        )]),
    );
    let mut done = result(
        "c1",
        json!([{"type": "text", "text": "{\"success\":true,\"message\":\"Report delivered to your caller.\"}"}]),
        json!({"success": true, "message": "Report delivered to your caller."}),
    );
    for l in [&mut hand, &mut done] {
        l["isSidechain"] = json!(true);
        l["agentId"] = json!("aa09474a10fff30b4");
    }
    let s = transcript(&[first, hand, done]);
    let last = s.messages().last().unwrap();
    assert_eq!(
        (last.role, last.origin, last.text()),
        (Role::Assistant, Origin::Model, "add, sub".to_string())
    );
    assert!(s.tool_calls().is_empty(), "{:?}", s.tool_calls());
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

/// ap-claude-main#6, review r1: ownstartup-shits fefc1886 lines 1125-1127. A teammate's message
/// written while one API response still streams reaches the model only with the next request, so
/// the response is not split around it: the call stays with its own text.
#[test]
fn a_teammate_message_mid_response_does_not_split_it() {
    let msg = "msg_011CcncBz8";
    let note = "Another Claude session sent a message:\n<teammate-message teammate_id=\"provider-research\" color=\"blue\">\nfound two APIs\n</teammate-message>";
    let s = transcript(&[
        assistant(
            "a1",
            msg,
            json!([{"type": "text", "text": "Writing the plan."}]),
            10,
        ),
        user("p1", note, json!({})),
        assistant(
            "a2",
            msg,
            json!([tool_use(
                "c1",
                "Write",
                json!({"file_path": "/w/plan.md", "content": "x"})
            )]),
            20,
        ),
    ]);
    let ids: Vec<&str> = (s.entries.iter())
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) if m.role == Role::Assistant => e.id.as_deref(),
            _ => None,
        })
        .collect();
    assert_eq!(ids, [msg]);
}

/// Review r1: esim 51749b4d, teammate `ios-teardown` pings idle (lines 2874, 2896), then the lead
/// wakes it and it reports its work (line 3025). A message that is no idle ping clears `idle`.
#[test]
fn a_teammate_message_after_an_idle_ping_clears_idle() {
    let ping = "Another Claude session sent a message:\n<teammate-message teammate_id=\"provider-research\" color=\"blue\">\n{\"type\":\"idle_notification\",\"from\":\"provider-research\"}\n</teammate-message>";
    let work = "Another Claude session sent a message:\n<teammate-message teammate_id=\"provider-research\" color=\"blue\" summary=\"Walkthrough done\">\nWalkthrough finished, notes in teardown/notes.md.\n</teammate-message>";
    let [a, r] = spawn();
    let s = transcript(&[
        a,
        r,
        user("p1", ping, json!({})),
        user("p2", work, json!({})),
    ]);
    let c = call(&s, "c1");
    assert_eq!(
        (c.status, c.status_detail.as_deref()),
        (ToolStatus::Pending, None)
    );
}

/// Review r1: stremio-alt db3427fd lines 24702-24709. After `agents_killed` took the words, the
/// harness delivers them as a `queued_command` with no `origin` (`commandMode`
/// `task-notification`); that is no typed input, so the kill does not read as the owner's words.
#[test]
fn a_delivered_kill_text_is_no_typed_input() {
    let a = json!({"type": "queued_command", "prompt": KILLED, "commandMode": "task-notification"});
    let s = transcript(&[line("attachment", "q1", json!({ "attachment": a }))]);
    let m = s.messages().next().unwrap();
    assert_eq!(m.origin, Origin::Notification);
}

/// Review r1 guard, the common kill shape (19 of 23 files): pet-ios 2e91890a lines 1853-1858.
/// The harness dequeues the words before `agents_killed`, then delivers them as a user line with
/// `origin` `task-notification`. Nothing stays queued, the words read as a notification.
#[test]
fn a_kill_dequeued_before_its_record_queues_nothing() {
    let s = transcript(&[
        user("u1", "review the plan", json!({})),
        enqueue(KILLED),
        dequeue(),
        system("k1", "agents_killed", json!({})),
        user(
            "n1",
            KILLED,
            json!({"origin": {"kind": "task-notification"}}),
        ),
    ]);
    assert_eq!(queued(&s), Vec::<String>::new());
    let last = s.messages().last().unwrap();
    assert_eq!(
        (last.origin, last.text()),
        (Origin::Notification, KILLED.to_string())
    );
}

/// Review r1: -Users-us-coding e857d7c5 line 1110. An older teammate spawn names the agent as
/// `<name>@<teamName>`, never its file id; the teammate file links to it by that address.
#[test]
fn an_older_teammate_file_links_by_its_team_address() {
    let dir = tempfile::tempdir().unwrap();
    let id = "adocument-p1-fa2c1e6561eff98e";
    let spawned = "Spawned successfully.\nagent_id: document-p1@session-e857d7c5\nname: document-p1\nThe agent is now running and will receive instructions via mailbox.";
    let input = json!({"description": "Build document model and undo", "prompt": "p", "name": "document-p1"});
    let lead = [
        calls("a1", json!([tool_use("c1", "Agent", input)])),
        result(
            "c1",
            json!([{"type": "text", "text": spawned}]),
            Value::Null,
        ),
    ];
    std::fs::write(dir.path().join("s1.jsonl"), text_of(&lead)).unwrap();
    let subagents = dir.path().join("s1/subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    let meta = json!({"agentType": "document-p1", "name": "document-p1",
        "taskKind": "in_process_teammate", "teamName": "session-e857d7c5"});
    std::fs::write(
        subagents.join(format!("agent-{id}.meta.json")),
        meta.to_string(),
    )
    .unwrap();
    let first = line(
        "user",
        "t1",
        json!({"isSidechain": true, "agentId": id, "message": {"role": "user", "content": "build"}}),
    );
    let path = subagents.join(format!("agent-{id}.jsonl"));
    std::fs::write(&path, text_of(&[first])).unwrap();
    let s = parse_file(&path).unwrap();
    assert_eq!(s.parent.expect("a parent").call_id.as_deref(), Some("c1"));
}
