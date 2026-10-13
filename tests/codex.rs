//! Codex rollouts through the public API, against the facts in `tests/fixtures/codex/README.md`.

use agentparse::{
    Agent, Entry, EntryKind, FileOp, Message, Origin, Part, ProcessAction, Role, Session,
    ShellResult, Source, Tool, ToolCall, ToolStatus, Turn, Usage,
};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/codex")
        .join(name)
}

fn parse(name: &str) -> Session {
    agentparse::parse_file(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn messages(s: &Session, role: Role, origin: Origin) -> Vec<&Message> {
    s.messages()
        .filter(|m| m.role == role && m.origin == origin)
        .collect()
}

fn texts(ms: &[&Message]) -> Vec<String> {
    ms.iter()
        .map(|m| m.text())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Top-level tool calls, in order.
fn calls(s: &Session) -> Vec<&ToolCall> {
    s.messages()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            Part::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn shell(c: &ToolCall) -> &ShellResult {
    c.output
        .as_ref()
        .and_then(|o| o.shell.as_ref())
        .unwrap_or_else(|| panic!("{} has no shell result", c.id))
}

/// What a code cell (or a `wait` on one) reports: wall time, cell id, running process ids.
fn cell_data(c: &ToolCall) -> &Value {
    let out = c.output.as_ref().unwrap();
    assert!(out.shell.is_none(), "{} is not a shell", c.id);
    out.data
        .as_ref()
        .unwrap_or_else(|| panic!("{} has no data", c.id))
}

fn command(c: &ToolCall) -> &str {
    match &c.tool {
        Tool::Shell(s) => &s.command,
        other => panic!("{} is not a shell: {other:?}", c.id),
    }
}

fn turns(s: &Session) -> Vec<(&Entry, &Turn)> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Turn(t) => Some((e, t)),
            _ => None,
        })
        .collect()
}

fn others(s: &Session, kind: &str) -> usize {
    s.entries
        .iter()
        .filter(|e| matches!(&e.kind, EntryKind::Other(o) if o.kind == kind))
        .count()
}

fn reasoning(s: &Session) -> Vec<(Option<&str>, bool)> {
    s.messages()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            Part::Reasoning { text, encrypted } => Some((text.as_deref(), *encrypted)),
            _ => None,
        })
        .collect()
}

fn usage(input: u64, cache_read: u64, output: u64, reasoning: u64) -> Usage {
    Usage {
        input,
        cache_read,
        output,
        reasoning,
        cache_write: 0,
        cost_usd: None,
    }
}

fn model_usages(s: &Session) -> Vec<(Option<&str>, Usage)> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) => m.usage.map(|u| (e.id.as_deref(), u)),
            _ => None,
        })
        .collect()
}

const ALL: &[&str] = &[
    "paginated-code-mode.jsonl",
    "paginated-late-exec.jsonl",
    "paginated-subagent-parent.jsonl",
    "paginated-subagent-child.jsonl",
    "paginated-interrupt.jsonl",
    "legacy-direct-tools.jsonl",
    "flat-era.jsonl",
    "corrupt-tail.jsonl",
    "exit3.jsonl",
];

#[test]
fn code_mode_identity_messages_and_usage() {
    let s = parse("paginated-code-mode.jsonl");
    assert_eq!(
        (s.agent, s.id.as_str(), s.format.as_str()),
        (
            Agent::Codex,
            "019f1a10-0001-7000-8000-000000000001",
            "codex-paginated"
        )
    );
    assert_eq!(
        (s.cwd.as_deref(), s.cli_version.as_deref()),
        (Some("/work/demo"), Some("0.162.1"))
    );
    let git = s.git.as_ref().unwrap();
    assert_eq!(
        (git.branch.as_deref(), git.remote.as_deref()),
        (Some("main"), Some("https://example.com/demo.git"))
    );
    assert_eq!(
        (s.title.as_deref(), s.models.as_slice()),
        (Some("list the files"), &["gpt-6.1-sol".to_owned()][..])
    );
    assert!(s.parent.is_none() && s.warnings.is_empty());
    assert_eq!(
        s.extra.as_ref().unwrap()["session_id"],
        "019f1a10-0001-7000-8000-000000000001"
    );

    let human = messages(&s, Role::User, Origin::Human);
    assert_eq!(texts(&human), ["list the files"]);
    let prompt = s
        .entries
        .iter()
        .find(|e| matches!(&e.kind, EntryKind::Message(m) if m.is_human()))
        .unwrap();
    assert_eq!(
        prompt.id.as_deref(),
        Some("msg_019f1a01-0004-7000-8000-000000000004")
    );
    assert_eq!(
        prompt.extra.as_ref().unwrap()["item_ids"],
        json!([
            "msg_019f1a01-0004-7000-8000-000000000004",
            "019f1a02-0001-7000-8000-000000000001"
        ])
    );
    let first = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("resp_7c92cf1eee8d99cc85f8355a"));
    assert_eq!(
        first.unwrap().extra.as_ref().unwrap()["item_ids"],
        json!([
            "rs_4fc2f671f0898f76d30da6b1",
            "msg_019f1a03-0001-7000-8000-000000000001",
            "ctc_d80f7b66d83d390c4781b032"
        ])
    );

    let kinds = |ms: Vec<&Message>| {
        ms.iter()
            .map(|m| m.kind.clone().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(messages(&s, Role::System, Origin::Context)),
        [
            "host_skills.instructions",
            "permissions.instructions",
            "collaboration_mode.instructions"
        ]
    );
    assert_eq!(
        kinds(messages(&s, Role::User, Origin::Context)),
        ["agents_md.instructions", "environments.environment_context"]
    );

    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(model.len(), 3, "one message per model response");
    assert_eq!(
        texts(&model),
        [
            "I will read the README and run the checks.",
            "The demo directory has README.md and src/. One check failed."
        ]
    );
    let phases: Vec<_> = model.iter().map(|m| m.phase.as_deref()).collect();
    assert_eq!(phases, [Some("commentary"), None, Some("final_answer")]);
    assert_eq!(reasoning(&s), [(None, true), (None, true)]);

    assert_eq!(
        model_usages(&s),
        [
            (
                Some("resp_7c92cf1eee8d99cc85f8355a"),
                usage(8000, 4000, 300, 100)
            ),
            (
                Some("resp_d279186428a75016b17e4df5"),
                usage(1000, 12000, 200, 50)
            ),
            (
                Some("resp_9d3e622df914d8de7f747b7b"),
                usage(1000, 13000, 80, 0)
            ),
        ]
    );
    assert_eq!(s.usage, usage(10000, 29000, 580, 150));
    assert_eq!(s.usage.total(), 39580);

    let ts = turns(&s);
    assert_eq!(ts.len(), 1);
    assert_eq!(
        (ts[0].1.status.as_deref(), ts[0].1.duration_ms),
        (Some("completed"), Some(4614))
    );
    let settings: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Settings(st) => Some(st),
            _ => None,
        })
        .collect();
    assert_eq!(settings.len(), 1);
    assert_eq!(
        (
            settings[0].model.as_deref(),
            settings[0].sandbox.as_deref(),
            settings[0].approval.as_deref()
        ),
        (Some("gpt-6.1-sol"), Some("read-only"), Some("never"))
    );
    assert_eq!(
        (
            others(&s, "task_started"),
            others(&s, "world_state"),
            others(&s, "token_count")
        ),
        (1, 1, 4)
    );
}

#[test]
fn code_mode_cells_and_children() {
    let s = parse("paginated-code-mode.jsonl");
    let cells = calls(&s);
    assert_eq!(cells.len(), 2);
    for c in &cells {
        assert_eq!(
            (c.name.as_str(), c.status, c.status_detail.as_deref()),
            ("exec", ToolStatus::Ok, Some("Script completed"))
        );
        assert!(matches!(&c.tool, Tool::Code { source } if source == c.input.as_str().unwrap()));
    }
    let (one, two) = (cells[0], cells[1]);
    assert_eq!(one.id, "call_a647ae9e7f808525a472d74deedcc965");
    let out = one.output.as_ref().unwrap();
    assert!(
        out.text
            .starts_with("Script completed\nWall time 1.2 seconds\nOutput:\n")
    );
    assert!(
        out.text
            .contains("Warning: truncated output (original token count: 6120)")
            && out.truncated
    );
    assert_eq!(cell_data(one)["wall_time_ms"], 1200);

    assert_eq!(one.children.len(), 2);
    let (a, b) = (&one.children[0], &one.children[1]);
    assert_eq!(
        (command(a), a.status, shell(a).exit_code),
        ("cat README.md; rg TODO src", ToolStatus::Ok, Some(0))
    );
    assert_eq!(
        (command(b), b.status, shell(b).exit_code),
        ("./run-checks.sh", ToolStatus::Error, Some(1))
    );
    assert_eq!(
        (a.status_detail.as_deref(), b.status_detail.as_deref()),
        (Some("completed"), Some("failed"))
    );
    let Tool::Shell(sh) = &a.tool else {
        unreachable!()
    };
    assert_eq!(
        (sh.argv.as_slice(), sh.cwd.as_deref()),
        (
            &[
                "/bin/zsh".to_owned(),
                "-lc".into(),
                "cat README.md; rg TODO src".into()
            ][..],
            Some("/work/demo")
        )
    );
    assert_eq!(
        (shell(a).duration_ms, shell(b).duration_ms),
        (Some(0), Some(2))
    );
    assert_eq!(
        shell(b).stdout.as_deref(),
        Some("check 1 ok\ncheck 2 FAILED: synthetic failure\n")
    );
    assert_eq!(
        (
            shell(b).stderr.as_deref(),
            b.output.as_ref().unwrap().text.as_str()
        ),
        (None, "")
    );
    let data = a.output.as_ref().unwrap().data.as_ref().unwrap();
    assert_eq!(
        (
            data["process_id"].as_str(),
            data["parsed_cmd"][1]["query"].as_str()
        ),
        (Some("40101"), Some("TODO"))
    );
    assert!(data.get("aggregated_output").is_none());
    assert_eq!(
        (a.name.as_str(), &a.input, a.started_at, a.ended_at),
        (
            "CommandExecution",
            &Value::Null,
            Some(1791795601245),
            Some(1791795601365)
        )
    );

    assert_eq!(two.children.len(), 1);
    let web = &two.children[0];
    assert!(matches!(&web.tool, Tool::WebSearch { query: Some(q) } if q == "example query"));
    assert_eq!(
        web.output.as_ref().unwrap().data.as_ref().unwrap()["results"][0]["domain"],
        "example.com"
    );
    assert_eq!(cell_data(two)["wall_time_ms"], 2300);
    assert_eq!(s.tool_calls().len(), 5);
}

#[test]
fn late_exec_joins_its_cell_by_process_id() {
    let s = parse("paginated-late-exec.jsonl");
    let cells = calls(&s);
    assert_eq!(cells.len(), 2);
    let (a, b) = (cells[0], cells[1]);
    assert_eq!(cell_data(a)["running_session_ids"], json!(["64391"]));
    assert!(b.children.is_empty());
    assert_eq!(a.children.len(), 1);
    let late = &a.children[0];
    assert_eq!(
        (command(late), late.status, shell(late).exit_code),
        ("bun run dev", ToolStatus::Ok, Some(0))
    );
    assert_eq!(
        (
            shell(late).duration_ms,
            shell(late).background_id.as_deref()
        ),
        (Some(12500), Some("64391"))
    );
    let known: Vec<_> = s
        .tool_calls()
        .iter()
        .filter_map(|c| c.output.as_ref()?.shell.as_ref()?.exit_code)
        .collect();
    assert_eq!(known, [0]);
    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["start the dev server and check it"]
    );
    assert_eq!(
        texts(&messages(&s, Role::Assistant, Origin::Model)),
        ["The dev server started and answered GET / with 200."]
    );
    assert_eq!(reasoning(&s).len(), 2);
    assert_eq!(s.usage, usage(6700, 21400, 250, 60));
    assert_eq!(turns(&s).len(), 1);
}

#[test]
fn subagent_parent_links_its_child() {
    let s = parse("paginated-subagent-parent.jsonl");
    let child = parse("paginated-subagent-child.jsonl");
    let cs = calls(&s);
    let names: Vec<_> = cs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["spawn_agent", "send_message", "wait_agent"]);
    assert!(
        cs.iter()
            .all(|c| matches!(c.tool, Tool::Task { .. }) && c.status == ToolStatus::Ok)
    );
    let spawn = cs[0];
    assert!(
        matches!(&spawn.tool, Tool::Task { description: Some(d), prompt: None, .. } if d == "review")
    );
    assert_eq!(
        spawn.input,
        json!({"task_name": "review", "message": {"bytes": 10}})
    );
    assert_eq!(
        spawn.output.as_ref().unwrap().text,
        r#"{"task_name":"/root/review"}"#
    );
    assert_eq!(spawn.child_session.as_deref(), Some(child.id.as_str()));
    assert_eq!(cs[1].child_session.as_deref(), Some(child.id.as_str()));
    assert_eq!(cs[1].output.as_ref().unwrap().text, "");
    assert_eq!(cs[2].status_detail.as_deref(), Some("completed"));
    assert_eq!(
        cs[2].output.as_ref().unwrap().data.as_ref().unwrap()["tool"],
        "wait"
    );
    assert_eq!(child.parent.as_ref().unwrap().session_id, s.id);

    let agents = messages(&s, Role::User, Origin::Agent);
    assert_eq!(agents.len(), 2);
    assert_eq!(
        (agents[0].kind.as_deref(), agents[1].kind.as_deref()),
        (Some("MESSAGE"), Some("FINAL_ANSWER"))
    );
    assert!(
        agents
            .iter()
            .all(|m| m.author.as_deref() == Some("/root/review"))
    );
    assert!(
        matches!(&agents[0].parts[1], Part::Other { kind, data } if kind == "encrypted_content" && data["encrypted_content"] == json!({"bytes": 10}))
    );
    assert!(agents[1].text().ends_with("VERDICT: APPROVE"));
    let first = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("amsg_019f1a05-0001-7000-8000-000000000001"))
        .unwrap();
    assert_eq!(first.extra.as_ref().unwrap()["recipient"], "/root");

    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["review the demo project with a helper"]
    );
    assert_eq!(
        texts(&messages(&s, Role::Assistant, Origin::Model)),
        [
            "I will ask a helper to review it.",
            "The helper approved the demo project."
        ]
    );
    assert_eq!(
        (
            others(&s, "SubAgentActivity"),
            others(&s, "inter_agent_communication_metadata")
        ),
        (1, 2)
    );
    assert_eq!(s.usage, usage(7000, 26800, 260, 40));
    assert!(s.parent.is_none());
}

#[test]
fn subagent_child_drops_its_inherited_prefix() {
    let s = parse("paginated-subagent-child.jsonl");
    assert_eq!(s.id, "019f1a10-0004-7000-8000-000000000004");
    let parent = s.parent.as_ref().unwrap();
    assert_eq!(
        (parent.session_id.as_str(), parent.agent_type.as_deref()),
        ("019f1a10-0003-7000-8000-000000000003", Some("/root/review"))
    );
    let extra = s.extra.as_ref().unwrap();
    assert_eq!(
        (
            extra["subagent_history_start_ordinal"].as_u64(),
            extra["session_id"].as_str()
        ),
        (Some(9), Some("019f1a10-0003-7000-8000-000000000003"))
    );
    assert!(
        s.entries.iter().all(|e| e.line.unwrap() >= 10),
        "records below ordinal 9 are the parent's"
    );
    assert!(messages(&s, Role::User, Origin::Human).is_empty());
    assert_eq!(s.title, None);
    assert_eq!(others(&s, "task_started"), 1);

    let first = &s.entries[0];
    let EntryKind::Settings(st) = &first.kind else {
        panic!("{first:?}")
    };
    assert_eq!(
        (st.model.as_deref(), st.effort.as_deref(), st.cwd.as_deref()),
        (Some("gpt-6.1-sol"), Some("low"), Some("/work/demo"))
    );

    let incoming = messages(&s, Role::User, Origin::Agent);
    assert_eq!(
        (
            incoming.len(),
            incoming[0].kind.as_deref(),
            incoming[0].author.as_deref()
        ),
        (1, Some("NEW_TASK"), Some("/root"))
    );
    // Its own message to the parent is model output, told apart by its author.
    let model = messages(&s, Role::Assistant, Origin::Model);
    let (own, plain): (Vec<_>, Vec<_>) = model.into_iter().partition(|m| m.author.is_some());
    assert_eq!(
        (own.len(), own[0].author.as_deref(), own[0].kind.as_deref()),
        (1, Some("/root/review"), None)
    );
    assert!(own[0].text().starts_with("Message Type: FINAL_ANSWER"));
    assert!(messages(&s, Role::Assistant, Origin::Agent).is_empty());
    assert_eq!(
        texts(&plain),
        ["Reviewing the demo files.", "VERDICT: APPROVE"]
    );

    let cells = calls(&s);
    assert_eq!((cells.len(), cells[0].children.len()), (1, 1));
    let cat = &cells[0].children[0];
    assert_eq!(
        (command(cat), shell(cat).exit_code, cat.status),
        ("cat README.md", Some(0), ToolStatus::Ok)
    );
    assert_eq!(s.usage, usage(6300, 8000, 70, 10));
    let ts = turns(&s);
    assert_eq!(
        (ts.len(), ts[0].0.turn_id.as_deref()),
        (1, Some("019f1a11-0004-7000-8000-000000000004"))
    );
}

#[test]
fn interrupt_marker_and_event_make_one_turn() {
    let s = parse("paginated-interrupt.jsonl");
    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["run the long build", "never mind, just list the files"]
    );
    assert_eq!(
        texts(&messages(&s, Role::Assistant, Origin::Model)),
        [
            "Starting the build.",
            "The directory has README.md and src/."
        ]
    );
    assert!(s.messages().all(|m| !m.text().contains("<turn_aborted>")));
    let ts = turns(&s);
    assert_eq!(ts.len(), 2);
    let (entry, aborted) = ts[0];
    assert_eq!(
        (aborted.status.as_deref(), aborted.duration_ms),
        (Some("interrupted"), Some(10146))
    );
    assert_eq!(
        (entry.line, entry.turn_id.as_deref()),
        (Some(17), Some("019f1a11-0005-7000-8000-000000000005"))
    );
    assert!(
        entry.extra.as_ref().unwrap()["marker"]
            .as_str()
            .unwrap()
            .starts_with("<turn_aborted>")
    );
    assert_eq!(ts[1].1.status.as_deref(), Some("completed"));
    let cell = calls(&s)[0];
    assert_eq!(
        (
            cell.status,
            cell.children.len(),
            &cell_data(cell)["running_session_ids"]
        ),
        (ToolStatus::Ok, 0, &json!(["70001"]))
    );
    assert_eq!(s.usage, usage(5200, 7000, 110, 20));
}

#[test]
fn legacy_messages_are_deduped_by_text() {
    let s = parse("legacy-direct-tools.jsonl");
    assert_eq!(
        (s.format.as_str(), s.cli_version.as_deref(), s.id.as_str()),
        (
            "codex-legacy",
            Some("0.120.0"),
            "019f1a20-0001-7000-8000-000000000001"
        )
    );
    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["list the files", "run the slow tests"]
    );
    let context: Vec<_> = messages(&s, Role::User, Origin::Context)
        .iter()
        .map(|m| m.kind.clone().unwrap())
        .collect();
    assert_eq!(context, ["user_instructions", "environment_context"]);
    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(
        texts(&model),
        [
            "I will list the files and edit the code.",
            "Done: the files are listed and the code is edited."
        ]
    );
    assert_eq!(
        model
            .iter()
            .filter_map(|m| m.phase.as_deref())
            .collect::<Vec<_>>(),
        ["commentary", "final_answer"]
    );
    assert_eq!(
        reasoning(&s),
        [(
            Some("Plan the file listing.\n\nRaw thought about listing files."),
            true
        )]
    );
    assert_eq!(model_usages(&s), [(None, usage(1200, 5200, 120, 20))]);
    assert_eq!(s.usage, usage(1200, 5200, 120, 20));
    assert_eq!(s.models, ["gpt-5-codex"]);

    let compactions: Vec<_> = s
        .entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::Compaction(_)))
        .collect();
    assert_eq!(compactions.len(), 1);
    let EntryKind::Compaction(c) = &compactions[0].kind else {
        unreachable!()
    };
    assert_eq!(
        c.summary.as_deref(),
        Some("Synthetic summary: listed files, edited code, searched the web.")
    );
    let extra = compactions[0].extra.as_ref().unwrap();
    assert_eq!(
        (
            extra["replacement_history"].as_array().map(Vec::len),
            extra.get("context_compacted").is_some()
        ),
        (Some(2), true)
    );

    let ts = turns(&s);
    let statuses: Vec<_> = ts.iter().map(|t| t.1.status.as_deref()).collect();
    assert_eq!(statuses, [Some("completed"), Some("interrupted")]);
    assert_eq!(
        ts[1].0.turn_id.as_deref(),
        Some("019f1a21-0002-7000-8000-000000000002")
    );
    assert!(ts[1].0.extra.as_ref().unwrap().get("marker").is_some());
    assert_eq!(
        (others(&s, "task_started"), others(&s, "token_count")),
        (2, 2)
    );
}

#[test]
fn legacy_direct_tools() {
    let s = parse("legacy-direct-tools.jsonl");
    let cs = calls(&s);
    let names: Vec<_> = cs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "shell",
            "shell_command",
            "exec_command",
            "write_stdin",
            "local_shell_call",
            "apply_patch",
            "update_plan",
            "view_image",
            "mcp__demo__search",
            "web_search_call",
            "shell"
        ]
    );
    let [
        sh,
        sh_cmd,
        exec,
        stdin,
        local,
        patch,
        plan,
        image,
        mcp,
        web,
        slow,
    ] = cs[..]
    else {
        unreachable!()
    };

    let Tool::Shell(t) = &sh.tool else { panic!() };
    assert_eq!(
        (
            t.argv.len(),
            t.command.as_str(),
            t.cwd.as_deref(),
            t.timeout_ms
        ),
        (3, "ls", Some("/work/demo"), Some(10000))
    );
    assert_eq!(
        (
            sh.status,
            shell(sh).exit_code,
            shell(sh).duration_ms,
            shell(sh).stdout.as_deref()
        ),
        (ToolStatus::Ok, Some(0), Some(100), Some("README.md\nsrc\n"))
    );

    assert_eq!(
        (
            command(sh_cmd),
            sh_cmd.status,
            shell(sh_cmd).exit_code,
            shell(sh_cmd).duration_ms
        ),
        ("rg missing src", ToolStatus::Error, Some(1), Some(200))
    );

    assert_eq!(command(exec), "sleep 1; echo ok");
    let r = shell(exec);
    assert_eq!(
        (
            r.exit_code,
            r.duration_ms,
            r.stdout.as_deref(),
            r.stderr.as_deref(),
            r.background_id.as_deref()
        ),
        (Some(0), Some(1105), Some("ok\n"), Some(""), Some("64391"))
    );
    assert_eq!(
        (exec.status, exec.status_detail.as_deref()),
        (ToolStatus::Ok, Some("completed"))
    );
    assert_eq!(
        exec.output.as_ref().unwrap().data.as_ref().unwrap()["formatted_output"],
        "ok\n"
    );

    assert!(
        matches!(&stdin.tool, Tool::Process { id: Some(id), action: ProcessAction::Read, input: None } if id == "64391")
    );
    assert_eq!(
        (shell(stdin).exit_code, shell(stdin).stdout.as_deref()),
        (Some(0), Some("ok\n"))
    );

    let Tool::Shell(t) = &local.tool else {
        panic!()
    };
    assert_eq!(
        (
            t.command.as_str(),
            t.argv.len(),
            t.timeout_ms,
            shell(local).exit_code
        ),
        ("git status --short", 3, Some(5000), Some(0))
    );

    assert!(matches!(&patch.tool, Tool::Patch { patch: p } if p.starts_with("*** Begin Patch")));
    let files = &patch.output.as_ref().unwrap().files;
    let ops: Vec<_> = files
        .iter()
        .map(|f| (f.path.as_str(), f.op, f.additions, f.deletions))
        .collect();
    assert_eq!(
        ops,
        [
            ("/work/demo/docs/new.txt", FileOp::Add, Some(2), Some(0)),
            ("/work/demo/old.txt", FileOp::Delete, Some(0), Some(1)),
            ("/work/demo/src/lib.rs", FileOp::Move, Some(1), Some(1)),
        ]
    );
    assert_eq!(files[2].move_to.as_deref(), Some("/work/demo/src/core.rs"));
    assert_eq!(
        (patch.status, patch.status_detail.as_deref()),
        (ToolStatus::Ok, Some("completed"))
    );

    let Tool::Todo { items } = &plan.tool else {
        panic!()
    };
    let steps: Vec<_> = items
        .iter()
        .map(|i| (i.content.as_str(), i.status.as_deref()))
        .collect();
    assert_eq!(
        steps,
        [
            ("Edit the code", Some("completed")),
            ("Run the tests", Some("in_progress"))
        ]
    );
    assert_eq!(plan.output.as_ref().unwrap().text, "Plan updated");

    assert!(matches!(&image.tool, Tool::Read { path, .. } if path == "/work/demo/shot.png"));
    let media = &image.output.as_ref().unwrap().attachments;
    assert_eq!(
        (
            media.len(),
            media[0].mime.as_deref(),
            media[0].bytes,
            media[0].url.as_deref()
        ),
        (1, Some("image/png"), Some(8), None)
    );

    assert!(
        matches!(&mcp.tool, Tool::Mcp { server, tool } if server == "demo" && tool == "search")
    );
    assert!(
        mcp.output
            .as_ref()
            .unwrap()
            .text
            .ends_with("3 results for demo")
    );

    assert!(matches!(&web.tool, Tool::WebSearch { query: Some(q) } if q == "example query"));
    assert_eq!(web.status, ToolStatus::Ok);

    assert_eq!(
        (command(slow), slow.status, slow.output.is_none()),
        ("sleep 600", ToolStatus::Interrupted, true)
    );
}

#[test]
fn flat_era() {
    let s = parse("flat-era.jsonl");
    assert_eq!(
        (s.format.as_str(), s.id.as_str()),
        ("codex-flat", "019f1a30-0001-7000-8000-000000000001")
    );
    assert_eq!(s.started_at, Some(1755684000000));
    assert_eq!(s.git.as_ref().unwrap().branch.as_deref(), Some("main"));
    assert_eq!(
        s.extra.as_ref().unwrap()["instructions"],
        "Synthetic user instructions."
    );
    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["list the files"]
    );
    assert_eq!(
        texts(&messages(&s, Role::Assistant, Origin::Model)),
        [
            "I will run ls.",
            "The directory has README.md and src. missing.txt does not exist."
        ]
    );
    assert_eq!(reasoning(&s), [(Some("Plan the listing."), true)]);
    let cs = calls(&s);
    let results: Vec<_> = cs
        .iter()
        .map(|c| (command(c), c.status, shell(c).exit_code))
        .collect();
    assert_eq!(
        results,
        [
            ("ls", ToolStatus::Ok, Some(0)),
            ("cat missing.txt", ToolStatus::Error, Some(1))
        ]
    );
    assert_eq!(
        shell(cs[1]).stdout.as_deref(),
        Some("cat: missing.txt: No such file or directory\n")
    );
    assert_eq!((s.usage, s.models.len()), (Usage::default(), 0));
    assert!(
        s.entries
            .iter()
            .all(|e| e.ts.is_none() && !matches!(e.kind, EntryKind::Other(_)))
    );
    assert!(s.warnings.is_empty());
}

#[test]
fn corrupt_lines_become_warnings() {
    let s = parse("corrupt-tail.jsonl");
    let lines: Vec<_> = s.warnings.iter().map(|w| w.line).collect();
    assert_eq!(lines, [Some(6), Some(12)]);
    assert_eq!(
        texts(&messages(&s, Role::User, Origin::Human)),
        ["say hello"]
    );
    assert_eq!(
        texts(&messages(&s, Role::Assistant, Origin::Model)),
        ["Hello."]
    );
    assert_eq!(s.usage, usage(2000, 1000, 5, 0));
    assert_eq!((turns(&s).len(), others(&s, "task_started")), (1, 1));
}

#[test]
fn real_session_exit_code_and_one_output_stream() {
    let s = parse("exit3.jsonl");
    let cells = calls(&s);
    assert_eq!(cells.len(), 2);
    let cmds: Vec<_> = cells[0]
        .children
        .iter()
        .map(|c| (command(c), shell(c).exit_code, c.status))
        .collect();
    assert_eq!(
        cmds,
        [
            ("ls -la", Some(0), ToolStatus::Ok),
            ("sh test.sh", Some(3), ToolStatus::Error),
            ("cat a.txt", Some(0), ToolStatus::Ok)
        ]
    );
    let test = shell(&cells[0].children[1]);
    let out = test.stdout.as_deref().unwrap();
    assert!(
        out.contains("out-line") && out.contains("err-line"),
        "{out:?}"
    );
    assert_eq!(test.stderr, None);

    let edit = &cells[1].children;
    assert_eq!(
        (edit.len(), edit[0].name.as_str(), edit[0].status),
        (1, "FileChange", ToolStatus::Ok)
    );
    let files = &edit[0].output.as_ref().unwrap().files;
    assert_eq!(
        (files.len(), files[0].path.as_str(), files[0].op),
        (1, "/work/demo/a.txt", FileOp::Update)
    );
    assert!(files[0].diff.as_deref().unwrap().contains("+two edited"));
    assert_eq!(s.usage, usage(69103 - 38656, 38656, 215, 0));
    assert_eq!(s.title.as_deref(), Some("run the demo checks"));
    assert_eq!(turns(&s).len(), 1);
}

#[test]
fn every_fixture_round_trips_through_json() {
    for name in ALL {
        let s = parse(name);
        let back: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s, "{name}");
    }
}

#[test]
fn zst_parses_like_plain() {
    let mut zst = parse("paginated-code-mode.jsonl.zst");
    let plain = parse("paginated-code-mode.jsonl");
    zst.source = plain.source.clone();
    assert_eq!(zst, plain);
}

#[test]
fn parse_reader_matches_parse_file() {
    let path = fixture("legacy-direct-tools.jsonl");
    let reader = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let s = agentparse::parse_reader(Agent::Codex, reader, Source::File { path }).unwrap();
    assert_eq!(s, parse("legacy-direct-tools.jsonl"));
}

#[test]
fn empty_input_is_not_a_session() {
    let src = Source::File {
        path: "empty.jsonl".into(),
    };
    let err = agentparse::parse_reader(Agent::Codex, &b"\n\n"[..], src).unwrap_err();
    assert!(matches!(err, agentparse::Error::NotASession(_)), "{err}");
}

// ---- shapes no local file has: built from the upstream types ----

fn rec(ty: &str, payload: Value) -> Value {
    json!({"timestamp": "2026-10-12T09:00:00.000Z", "type": ty, "payload": payload})
}

fn item(item: Value) -> Value {
    rec(
        "event_msg",
        json!({"type": "item_completed", "turn_id": "t1", "item": item, "started_at_ms": 1, "completed_at_ms": 2}),
    )
}

fn exec_call(id: &str) -> Value {
    rec(
        "response_item",
        json!({"type": "custom_tool_call", "call_id": id, "name": "exec", "input": "text(1)"}),
    )
}

fn exec_output(id: &str, items: &[&str]) -> Value {
    let output: Vec<_> = items
        .iter()
        .map(|t| json!({"type": "input_text", "text": t}))
        .collect();
    rec(
        "response_item",
        json!({"type": "custom_tool_call_output", "call_id": id, "output": output}),
    )
}

fn command_item(id: &str, source: &str, extra: Value) -> Value {
    let mut it = json!({"type": "CommandExecution", "id": id, "process_id": "9", "command": ["bash", "-c", "make"],
        "cwd": "file:///work/a%20b", "parsed_cmd": [], "source": source, "status": "completed",
        "aggregated_output": "done\n", "exit_code": 0, "duration": {"secs": 3, "nanos": 0}});
    it.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    item(it)
}

fn session(records: &[Value]) -> Session {
    let meta = json!({"id": "t1", "cwd": "/work/demo", "history_mode": "paginated"});
    parse_lines("inline.jsonl", meta, records)
}

/// An envelope file without `history_mode`, as every pre-0.143 build writes.
fn legacy(records: &[Value]) -> Session {
    let meta = json!({"id": "t1", "cwd": "/work/demo", "cli_version": "0.120.0"});
    parse_lines("inline.jsonl", meta, records)
}

fn parse_lines(path: &str, meta: Value, records: &[Value]) -> Session {
    let mut lines = vec![rec("session_meta", meta)];
    lines.extend(records.iter().cloned());
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    let source = Source::File { path: path.into() };
    agentparse::parse_reader(Agent::Codex, text.join("\n").as_bytes(), source).unwrap()
}

#[test]
fn cell_failures_yield_wait_and_notify() {
    let s = session(&[
        exec_call("c1"),
        exec_output(
            "c1",
            &[
                "Script failed\nWall time 0.5 seconds\nOutput:\n",
                "Script error:\nboom",
            ],
        ),
        exec_call("c2"),
        exec_output(
            "c2",
            &["Script running with cell ID 7\nWall time 10.0 seconds\nOutput:\n"],
        ),
        rec(
            "response_item",
            json!({"type": "function_call", "call_id": "w1", "name": "wait", "arguments": "{\"cell_id\":\"7\"}"}),
        ),
        command_item("e1", "unified_exec_startup", json!({"process_id": "11"})),
        rec(
            "response_item",
            json!({"type": "function_call_output", "call_id": "w1",
            "output": [{"type": "input_text", "text": "Script completed\nWall time 2.0 seconds\nOutput:\n"}]}),
        ),
        exec_call("c3"),
        exec_output("c3", &["notified"]),
        command_item(
            "e2",
            "unified_exec_interaction",
            json!({"interaction_input": "y\n"}),
        ),
        exec_output(
            "c3",
            &["Script terminated\nWall time 1.0 seconds\nOutput:\n"],
        ),
        rec(
            "response_item",
            json!({"type": "custom_tool_call_output", "call_id": "nope", "output": "lost"}),
        ),
    ]);
    let cs = calls(&s);
    let [failed, yielded, wait, notified] = cs[..] else {
        panic!("{cs:?}")
    };
    assert_eq!(
        (failed.status, failed.status_detail.as_deref()),
        (ToolStatus::Error, Some("Script failed"))
    );
    assert_eq!(
        (yielded.status, &cell_data(yielded)["cell_id"]),
        (ToolStatus::Ok, &json!("7"))
    );
    assert_eq!(
        yielded.children.len(),
        1,
        "items during the wait belong to the waited cell"
    );
    assert_eq!(shell(&yielded.children[0]).duration_ms, Some(3000));
    assert!(
        matches!(&wait.tool, Tool::Process { id: Some(id), action: ProcessAction::Read, .. } if id == "7")
    );
    assert_eq!(
        (wait.status, &cell_data(wait)["wall_time_ms"]),
        (ToolStatus::Ok, &json!(2000))
    );
    assert_eq!(notified.status, ToolStatus::Interrupted);
    assert!(
        notified
            .output
            .as_ref()
            .unwrap()
            .text
            .starts_with("notified\nScript terminated")
    );
    let typed = &notified.children[0];
    assert!(
        matches!(&typed.tool, Tool::Process { action: ProcessAction::Write, input: Some(i), .. } if i == "y\n")
    );
    let orphans: Vec<_> = s.messages().filter(|m| m.origin == Origin::Tool).collect();
    assert!(
        matches!(&orphans[0].parts[0], Part::ToolResult(r) if r.call_id == "nope" && r.output.text == "lost")
    );
    assert_eq!(s.warnings.len(), 1);
}

#[test]
fn user_shell_items_rollback_and_errors() {
    let s = session(&[
        command_item(
            "u1",
            "user_shell",
            json!({"exit_code": 2, "status": "failed"}),
        ),
        exec_call("c1"),
        item(
            json!({"type": "McpToolCall", "id": "m1", "server": "docs", "tool": "find", "arguments": {},
            "status": "completed", "result": {"content": [], "isError": true}}),
        ),
        item(
            json!({"type": "FileChange", "id": "f1", "status": "completed",
            "changes": {"/work/n.txt": {"type": "add", "content": "a\nb\n"}}}),
        ),
        item(
            json!({"type": "FileChange", "id": "f2", "status": "declined",
            "changes": {"/work/m.txt": {"type": "delete", "content": "x\n"}}}),
        ),
        exec_output(
            "c1",
            &["Script completed\nWall time 0.1 seconds\nOutput:\n"],
        ),
        rec(
            "event_msg",
            json!({"type": "thread_rolled_back", "num_turns": 2}),
        ),
        rec(
            "event_msg",
            json!({"type": "error", "message": "stream closed", "codex_error_info": "context_window_exceeded"}),
        ),
    ]);
    let human = messages(&s, Role::User, Origin::Command);
    assert_eq!(
        (human.len(), human[0].kind.as_deref()),
        (1, Some("user_shell"))
    );
    let Part::ToolCall(cmd) = &human[0].parts[0] else {
        panic!()
    };
    let Tool::Shell(t) = &cmd.tool else { panic!() };
    assert_eq!(
        (t.command.as_str(), t.cwd.as_deref()),
        ("make", Some("/work/a b"))
    );
    assert_eq!(
        (
            cmd.status,
            shell(cmd).exit_code,
            cmd.output.as_ref().unwrap().text.as_str()
        ),
        (ToolStatus::Error, Some(2), "done\n")
    );

    let children = &calls(&s)
        .into_iter()
        .find(|c| c.name == "exec")
        .unwrap()
        .children;
    assert!(
        matches!(&children[0].tool, Tool::Mcp { server, tool } if server == "docs" && tool == "find")
    );
    assert_eq!(children[0].status, ToolStatus::Error);
    let add = &children[1].output.as_ref().unwrap().files[0];
    assert_eq!(
        (add.op, add.additions, add.diff.as_deref()),
        (FileOp::Add, Some(2), Some("+a\n+b"))
    );
    let declined = &children[2];
    let out = declined.output.as_ref().unwrap();
    assert_eq!(
        (declined.status, out.files.len()),
        (ToolStatus::Rejected, 0)
    );
    assert_eq!(
        out.data.as_ref().unwrap()["changes"]["/work/m.txt"]["type"],
        "delete"
    );

    let rewind = s.entries.iter().find_map(|e| match &e.kind {
        EntryKind::Rewind(r) => Some(r.turns),
        _ => None,
    });
    assert_eq!(rewind, Some(Some(2)));
    let error = s.entries.iter().find_map(|e| match &e.kind {
        EntryKind::Error(e) => Some((e.kind.as_deref(), e.message.as_str())),
        _ => None,
    });
    assert_eq!(
        error,
        Some((Some("context_window_exceeded"), "stream closed"))
    );
}

#[test]
fn paginated_direct_exec_is_enriched_by_its_item() {
    let s = session(&[
        rec(
            "response_item",
            json!({"type": "function_call", "call_id": "x1", "name": "exec_command", "arguments": "{\"cmd\":\"make\"}"}),
        ),
        rec(
            "response_item",
            json!({"type": "function_call_output", "call_id": "x1",
            "output": "Chunk ID: a\nWall time: 3.0 seconds\nProcess exited with code 4\nOriginal token count: 1\nOutput:\nbad\n"}),
        ),
        command_item(
            "x1",
            "agent",
            json!({"exit_code": 4, "status": "failed", "aggregated_output": "bad\n"}),
        ),
    ]);
    let c = calls(&s)[0];
    assert!(c.children.is_empty());
    assert_eq!(
        (
            c.status,
            shell(c).exit_code,
            shell(c).duration_ms,
            shell(c).stdout.as_deref()
        ),
        (ToolStatus::Error, Some(4), Some(3000), Some("bad\n"))
    );
    assert_eq!(c.status_detail.as_deref(), Some("failed"));
    let Tool::Shell(t) = &c.tool else { panic!() };
    assert_eq!(
        t.cwd.as_deref(),
        Some("/work/demo"),
        "falls back to the session cwd"
    );
}

fn usage_record(response: &str, thread_total: u64) -> Value {
    let usage = json!({"input_tokens": 100, "cached_input_tokens": 40, "output_tokens": 10,
        "reasoning_output_tokens": 0, "total_tokens": 110});
    rec(
        "token_usage_record",
        json!({"response_id": response, "usage": usage,
        "thread_token_usage": {"total_tokens": thread_total}}),
    )
}

fn compactions(s: &Session) -> Vec<(&Entry, &agentparse::Compaction)> {
    s.entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Compaction(c) => Some((e, c)),
            _ => None,
        })
        .collect()
}

#[test]
fn compaction_call_usage_goes_to_the_compaction() {
    let compacted = rec(
        "compacted",
        json!({"message": "summary", "compaction_response_id": "resp_c"}),
    );
    let event = rec("event_msg", json!({"type": "context_compacted"}));
    for records in [
        vec![
            usage_record("resp_c", 110),
            event.clone(),
            compacted.clone(),
        ],
        vec![
            compacted.clone(),
            event.clone(),
            usage_record("resp_c", 110),
        ],
        vec![
            usage_record("resp_c", 110),
            compacted.clone(),
            event.clone(),
        ],
    ] {
        let s = session(&records);
        let cs = compactions(&s);
        assert_eq!(cs.len(), 1, "{records:?}");
        let (entry, c) = cs[0];
        assert_eq!(
            (c.summary.as_deref(), c.usage),
            (Some("summary"), Some(usage(60, 40, 10, 0)))
        );
        assert!(
            entry
                .extra
                .as_ref()
                .unwrap()
                .get("context_compacted")
                .is_some()
        );
        assert_eq!((s.messages().count(), s.usage), (0, usage(60, 40, 10, 0)));
    }
}

#[test]
fn phase_split_keeps_usage_on_the_first_piece() {
    let text = |id: &str, phase: &str| {
        rec(
            "response_item",
            json!({"type": "message", "id": id, "role": "assistant", "phase": phase,
            "content": [{"type": "output_text", "text": id}]}),
        )
    };
    let s = session(&[
        text("m1", "commentary"),
        text("m2", "final_answer"),
        usage_record("resp_x", 110),
    ]);
    let pieces: Vec<_> = s
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            EntryKind::Message(m) => Some((e.id.as_deref(), m.phase.as_deref(), m.usage.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(
        pieces,
        [
            (Some("resp_x#1"), Some("commentary"), true),
            (Some("resp_x#2"), Some("final_answer"), false)
        ]
    );
    assert_eq!(
        s.entries[1].extra.as_ref().unwrap()["item_ids"],
        json!(["m2"])
    );
}

#[test]
fn history_base_is_kept_and_warned() {
    let meta = rec(
        "session_meta",
        json!({"id": "t2", "history_mode": "paginated",
        "history_base": {"thread_id": "t0", "end_ordinal_exclusive": 5, "end_byte_offset": 900}}),
    );
    let source = Source::File {
        path: "inline.jsonl".into(),
    };
    let s = agentparse::parse_reader(Agent::Codex, meta.to_string().as_bytes(), source).unwrap();
    assert_eq!(s.extra.as_ref().unwrap()["history_base"]["thread_id"], "t0");
    assert_eq!(s.warnings.len(), 1);
    assert!(s.warnings[0].message.contains("another rollout"));
}

// ---- regression tests, one per shape ----

fn assistant_item(id: Option<&str>, phase: Option<&str>, parts: &[&str]) -> Value {
    let content: Vec<_> = parts
        .iter()
        .map(|t| json!({"type": "output_text", "text": t}))
        .collect();
    let mut p = json!({"type": "message", "role": "assistant", "content": content});
    if let Some(id) = id {
        p["id"] = json!(id);
    }
    if let Some(phase) = phase {
        p["phase"] = json!(phase);
    }
    rec("response_item", p)
}

fn agent_event(text: &str) -> Value {
    rec(
        "event_msg",
        json!({"type": "agent_message", "message": text}),
    )
}

fn entry_of<'a>(s: &'a Session, m: &Message) -> &'a Entry {
    s.entries
        .iter()
        .find(|e| matches!(&e.kind, EntryKind::Message(x) if std::ptr::eq(x, m)))
        .unwrap()
}

#[test]
fn failed_apply_patch_is_an_error_without_file_changes() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch";
    let header = "Exit code: 0\nWall time: 0.1 seconds\nOutput:\n";
    let cases = [
        (
            "apply_patch verification failed: Failed to find expected lines in /w/a.txt:\nold",
            ToolStatus::Error,
        ),
        (
            "failed to parse apply_patch: invalid hunk at line 2",
            ToolStatus::Error,
        ),
        (
            "Exit code: 1\nWall time: 0.1 seconds\nOutput:\nerror: no such file",
            ToolStatus::Error,
        ),
        ("patch rejected by user", ToolStatus::Rejected),
        (
            "Success. Updated the following files:\nM a.txt\n",
            ToolStatus::Ok,
        ),
        (
            &format!("{header}Success. Updated the following files:\nM a.txt\n"),
            ToolStatus::Ok,
        ),
    ];
    for (output, status) in cases {
        let s = legacy(&[
            rec(
                "response_item",
                json!({"type": "custom_tool_call", "call_id": "p1", "name": "apply_patch", "input": patch}),
            ),
            rec(
                "response_item",
                json!({"type": "custom_tool_call_output", "call_id": "p1", "output": output}),
            ),
        ]);
        let c = calls(&s)[0];
        let files = c.output.as_ref().unwrap().files.len();
        assert_eq!(
            (c.status, files),
            (status, usize::from(status == ToolStatus::Ok)),
            "{output}"
        );
    }
    let s = legacy(&[
        rec(
            "response_item",
            json!({"type": "custom_tool_call", "call_id": "p1", "name": "apply_patch", "input": patch}),
        ),
        rec(
            "response_item",
            json!({"type": "custom_tool_call_output", "call_id": "p1", "output": cases[0].0}),
        ),
    ]);
    let detail = calls(&s)[0].status_detail.clone();
    assert_eq!(
        detail.as_deref(),
        Some("apply_patch verification failed: Failed to find expected lines in /w/a.txt:")
    );
}

#[test]
fn legacy_assistant_text_comes_from_the_response_item() {
    let s = legacy(&[
        agent_event("Hello there."),
        assistant_item(Some("msg_a1"), Some("final_answer"), &["Hello there."]),
    ]);
    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(model.len(), 1);
    assert_eq!(
        (
            model[0].parts.len(),
            model[0].text(),
            model[0].phase.as_deref()
        ),
        (1, "Hello there.".into(), Some("final_answer"))
    );
    assert_eq!(
        entry_of(&s, model[0]).extra.as_ref().unwrap()["item_ids"],
        json!(["msg_a1"])
    );

    // Codex writes one event per content part.
    let s = legacy(&[
        agent_event("Part one. "),
        agent_event("Part two."),
        assistant_item(Some("msg_b1"), None, &["Part one. ", "Part two."]),
    ]);
    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(
        (model.len(), model[0].parts.len(), model[0].text()),
        (1, 1, "Part one. Part two.".into())
    );

    // The event's phase fills in; an event no item repeats is kept as text.
    let s = legacy(&[
        rec(
            "event_msg",
            json!({"type": "agent_message", "message": "Done.", "phase": "final_answer"}),
        ),
        assistant_item(None, None, &["Done."]),
        agent_event("Only in the event."),
        rec(
            "event_msg",
            json!({"type": "task_complete", "turn_id": "u1"}),
        ),
    ]);
    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(texts(&model), ["Done.\n\nOnly in the event."]);
    assert_eq!(model[0].phase.as_deref(), Some("final_answer"));
}

#[test]
fn image_prompt_copies_are_one_human_message() {
    let item_content = json!([
        {"type": "input_text", "text": "what is in this screenshot"},
        {"type": "input_text", "text": "<image name=[Image #1] path=\"/w/s.png\">"},
        {"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8="},
        {"type": "input_text", "text": "</image>"},
    ]);
    let s = legacy(&[
        rec(
            "response_item",
            json!({"type": "message", "role": "user", "content": item_content.clone()}),
        ),
        rec(
            "event_msg",
            json!({"type": "user_message", "message": "what is in this screenshot", "images": [], "local_images": ["/w/s.png"]}),
        ),
    ]);
    let human = messages(&s, Role::User, Origin::Human);
    assert_eq!(human.len(), 1);
    assert!(matches!(&human[0].parts[2], Part::Attachment(m) if m.bytes == Some(5)));

    let s = session(&[
        rec(
            "response_item",
            json!({"type": "message", "id": "msg_u1", "role": "user", "content": item_content}),
        ),
        item(json!({"type": "UserMessage", "id": "u-1", "content": [
            {"type": "text", "text": "what is in this screenshot", "text_elements": []},
            {"type": "local_image", "path": "/w/s.png"}]})),
    ]);
    assert_eq!(messages(&s, Role::User, Origin::Human).len(), 1);
}

#[test]
fn hook_prompt_message_and_item_are_one_entry() {
    let message = rec(
        "response_item",
        json!({"type": "message", "id": "msg_h1", "role": "user",
        "content": [{"type": "input_text", "text": "<hook_prompt hook_run_id=\"run1\">keep going</hook_prompt>"}]}),
    );
    let hook = item(
        json!({"type": "HookPrompt", "id": "msg_h1", "fragments": [{"text": "keep going", "hookRunId": "run1"}]}),
    );
    for records in [vec![message.clone(), hook.clone()], vec![hook, message]] {
        let s = session(&records);
        let hooks = messages(&s, Role::User, Origin::Notification);
        assert_eq!((s.messages().count(), hooks.len()), (1, 1));
        assert_eq!(hooks[0].kind.as_deref(), Some("hook_prompt"));
        let extra = entry_of(&s, hooks[0]).extra.as_ref().unwrap();
        assert_eq!(extra["started_at_ms"], 1);
    }
}

#[test]
fn item_envelopes_keep_their_times() {
    let s = parse("paginated-code-mode.jsonl");
    let first = s
        .entries
        .iter()
        .find(|e| e.id.as_deref() == Some("resp_7c92cf1eee8d99cc85f8355a"))
        .unwrap();
    let items = &first.extra.as_ref().unwrap()["items"];
    assert_eq!(
        items["rs_4fc2f671f0898f76d30da6b1"]["started_at_ms"],
        1791795600310_i64
    );
    assert_eq!(
        items["msg_019f1a03-0001-7000-8000-000000000001"]["started_at_ms"],
        1791795600342_i64
    );
    let prompt = s
        .entries
        .iter()
        .find(|e| matches!(&e.kind, EntryKind::Message(m) if m.is_human()))
        .unwrap();
    assert_eq!(
        prompt.extra.as_ref().unwrap()["completed_at_ms"],
        1791795600280_i64
    );

    let parent = parse("paginated-subagent-parent.jsonl");
    let data = calls(&parent)[0]
        .output
        .as_ref()
        .unwrap()
        .data
        .clone()
        .unwrap();
    assert_eq!(
        (data["started_at_ms"].as_i64(), data["turn_id"].as_str()),
        (
            Some(1791802801145),
            Some("019f1a11-0003-7000-8000-000000000003")
        )
    );
    let done = parent.entries.iter().find_map(|e| match &e.kind {
        EntryKind::Other(o) if o.kind == "SubAgentActivity" => Some(o.data.clone()),
        _ => None,
    });
    assert_eq!(done.unwrap()["completed_at_ms"], 1791802810994_i64);
}

#[test]
fn every_usage_record_counts() {
    let text = |id: &str| assistant_item(Some(id), None, &[id]);
    let record = |id: &str, thread: u64| {
        rec(
            "token_usage_record",
            json!({"response_id": id, "thread_token_usage": {"total_tokens": thread},
            "usage": {"input_tokens": 100, "cached_input_tokens": 0, "output_tokens": 10, "reasoning_output_tokens": 0, "total_tokens": 110}}),
        )
    };
    let s = session(&[text("m1"), record("r1", 500), text("m2"), record("r2", 110)]);
    assert_eq!(s.usage, usage(200, 0, 20, 0));
    assert_eq!(model_usages(&s).len(), 2);
    assert_eq!(s.warnings.len(), 1);
    assert!(s.warnings[0].message.contains("fell from 500 to 110"));
}

#[test]
fn child_tail_window_keeps_the_child_identity() {
    let text = std::fs::read_to_string(fixture("paginated-subagent-child.jsonl")).unwrap();
    let window = text.split_once('\n').unwrap().1;
    let path = "rollout-2026-10-12T11-00-01-019f1a10-0004-7000-8000-000000000004.jsonl";
    let source = Source::File { path: path.into() };
    let s = agentparse::parse_reader(Agent::Codex, window.as_bytes(), source).unwrap();
    assert_eq!(s.id, "019f1a10-0004-7000-8000-000000000004");
    assert!(
        s.entries
            .iter()
            .any(|e| matches!(&e.kind, EntryKind::Other(o) if o.kind == "session_meta"))
    );
}

#[test]
fn huge_recorded_numbers_do_not_overflow() {
    let huge = json!({"secs": u64::MAX, "nanos": 999_999_999});
    let record = || {
        rec(
            "token_usage_record",
            json!({"response_id": "r", "usage": {"input_tokens": 1, "total_tokens": u64::MAX}}),
        )
    };
    let s = session(&[
        exec_call("c1"),
        command_item("e1", "unified_exec_startup", json!({"duration": huge})),
        exec_output(
            "c1",
            &["Script completed\nWall time 0.1 seconds\nOutput:\n"],
        ),
        record(),
        record(),
    ]);
    assert_eq!(shell(&calls(&s)[0].children[0]).duration_ms, None);
}

#[test]
fn repeated_waits_stay_on_the_cell_that_yielded() {
    let running = "Script running with cell ID 7\nWall time 10.0 seconds\nOutput:\n";
    let wait = |id: &str| {
        rec(
            "response_item",
            json!({"type": "function_call", "call_id": id, "name": "wait", "arguments": "{\"cell_id\":\"7\"}"}),
        )
    };
    let wait_output = |id: &str, header: &str| {
        rec(
            "response_item",
            json!({"type": "function_call_output", "call_id": id, "output": [{"type": "input_text", "text": header}]}),
        )
    };
    let s = session(&[
        exec_call("c1"),
        exec_output("c1", &[running]),
        wait("w1"),
        wait_output("w1", running),
        wait("w2"),
        command_item("e1", "unified_exec_startup", json!({"process_id": "21"})),
        wait_output("w2", "Script completed\nWall time 1.0 seconds\nOutput:\n"),
    ]);
    let [cell, w1, w2] = calls(&s)[..] else {
        panic!()
    };
    assert_eq!(
        (
            cell.status,
            cell.status_detail.as_deref(),
            cell.children.len()
        ),
        (ToolStatus::Ok, Some("Script completed"), 1)
    );
    assert!(w1.children.is_empty() && w2.children.is_empty());
    assert_eq!(
        (w1.status, w2.status),
        (ToolStatus::Pending, ToolStatus::Ok)
    );
}

#[test]
fn yielded_cell_keeps_its_background_commands() {
    let s = session(&[
        exec_call("c1"),
        exec_output(
            "c1",
            &["Script running with cell ID 9\nWall time 10.0 seconds\nOutput:\n"],
        ),
        command_item("e9", "unified_exec_startup", json!({"process_id": "31"})),
    ]);
    assert_eq!(calls(&s)[0].children.len(), 1);
    assert!(s.warnings.is_empty());
}

#[test]
fn plan_item_is_not_read_twice() {
    let plan = item(json!({"type": "Plan", "id": "plan1", "text": "1. edit\n2. test"}));
    let answer = assistant_item(
        Some("m1"),
        None,
        &["<proposed_plan>\n1. edit\n2. test\n</proposed_plan>"],
    );
    let s = session(&[plan, answer]);
    let model = messages(&s, Role::Assistant, Origin::Model);
    assert_eq!(model.len(), 1);
    assert!(
        matches!(&model[0].parts[0], Part::Other { kind, data } if kind == "plan" && data["text"] == "1. edit\n2. test")
    );
    assert_eq!(model[0].text().matches("1. edit").count(), 1);
}

#[test]
fn unmatched_legacy_event_keeps_its_own_time_and_line() {
    let mut late = rec(
        "event_msg",
        json!({"type": "task_complete", "turn_id": "u1"}),
    );
    late["timestamp"] = json!("2026-10-12T09:05:00.000Z");
    let s = legacy(&[agent_event("Only in the event."), late]);
    let message = s
        .entries
        .iter()
        .find(|e| matches!(&e.kind, EntryKind::Message(m) if m.role == Role::Assistant))
        .unwrap();
    let complete_ts = s
        .entries
        .iter()
        .find_map(|e| matches!(e.kind, EntryKind::Turn(_)).then_some(e.ts))
        .flatten();
    assert_ne!(message.ts, complete_ts);
    assert!(message.line < s.entries.last().unwrap().line);
}
