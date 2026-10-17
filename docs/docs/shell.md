# Shell Commands

Shell commands get the most detail of any tool. A command has two halves: the call, which says what was run, and the result, which says what happened.

| Half | Where | Type |
|---|---|---|
| What was run | `ToolCall.tool`, when it is `Tool::Shell` | `Shell` |
| What happened | `ToolCall.output.shell` | `ShellResult` |
| What the model saw | `ToolCall.output.text` | `String` |

Commands a human ran themselves (`!cmd` in Claude Code or OpenCode, a Codex user shell) are not tool calls of the model. They become a `Message` with `role: User` and `origin: Command` that holds one `Shell` call with its result.

## Shell (the call)

| Field | Type | Meaning |
|---|---|---|
| `command` | `String` | The command line as the model wrote it |
| `argv` | `Vec<String>` | The exact argument vector, when the product recorded it |
| `cwd` | `Option<String>` | Working directory |
| `timeout_ms` | `Option<u64>` | Requested timeout in milliseconds |
| `background` | `bool` | The model asked to run it in the background |
| `description` | `Option<String>` | The model's own description of the command |

## ShellResult (the outcome)

| Field | Type | Meaning |
|---|---|---|
| `exit_code` | `Option<i32>` | A recorded or printed code, or 0 where the product reports plain success only for a zero exit (Claude Code and Gemini success results). `None` means it is truly unknown: backgrounded, interrupted, or a soft failure the product reinterpreted |
| `stdout` | `Option<String>` | Standard output. When `stderr` is `None`, this is the single interleaved stream |
| `stderr` | `Option<String>` | Set only where the product records stderr separately |
| `duration_ms` | `Option<u64>` | How long it ran, where the product records it |
| `interrupted` | `bool` | The command was stopped by the user or the run ending |
| `timed_out` | `bool` | The command hit its timeout |
| `signal` | `Option<String>` | The signal that ended it, when recorded |
| `background_id` | `Option<String>` | The shell, task or process id when the command kept running in the background |

Three rules to remember:

- A non-zero `exit_code`, or `timed_out`, makes the call's `status` `Error`. A zero exit code is `Ok`.
- `stderr` being `None` does not mean the command wrote nothing to stderr. It means the product did not separate the streams. Look in `stdout`.
- `output.text` and `shell.stdout` can differ. The text is what the model saw, including headers such as `Exit code 3`. `stdout` is what the process wrote.

Related fields on `ToolOutput`: `truncated` is true when the product cut the output, and `full_output_path` points to the untruncated copy the product saved.

## Where each product records it

| Field | Claude Code | Codex | OpenCode | Gemini CLI |
|---|---|---|---|---|
| command and argv | `input.command` | The command item's argv, or the call arguments (`cmd` or `command`). The command is the third element only for `sh -c` and `sh -lc` forms, otherwise the shell-quoted join of the argv | `input.command` | `args.command` |
| cwd | `cwd` of the tool call line | The command item's cwd (decoded from a `file://` URL), else `workdir`, else the turn's cwd | `input.workdir`, else the assistant message's cwd | `args.dir_path` or `directory` |
| timeout | `input.timeout` in ms | `timeout_ms` | `input.timeout` in ms | not recorded |
| background | `run_in_background`. The result has `backgroundTaskId` | The output JSON has a `session_id` while the process still runs | `input.background` and `metadata.shellID` | `is_background`, and the text "moved to background (PID: n)" |
| exit code | The text `Exit code N` on error results. A timeout is 143 | The event `exit_code`, the output JSON, or the text headers | `metadata.exit` | The text `Exit Code: N`. A 0 is only set for a modern success with no timeout or background text |
| stdout and stderr | `toolUseResult.stdout` and `.stderr` are separate. Error results are text only, so the streams are merged | `aggregated_output`, one stream | The output text, one stream | `Output:` is one stream. `Stdout:` and `Stderr:` sections exist up to v0.26 |
| duration | not recorded | One of the product's clocks (see the Codex page) | `state.time.end` minus `start` | not recorded |
| interrupted or timed out | The denial kind, or marker text. The text "Command timed out after" | `Script terminated`, or a turn that was aborted | `metadata.interrupted`, or the `<shell_metadata>` timeout text | A `cancelled` status, or the timeout text |
| truncated and full path | The persisted-output marker | `Warning: truncated output`, and the persistence marker | `metadata.truncated` and `outputPath` | `Output too large`, and the saved path |

The product pages ([Claude Code](#claude), [Codex](#codex), [OpenCode](#opencode), [Gemini CLI](#gemini)) cover the rest of each format.

## Background jobs

A command started in the background has a `background_id`. Its first result only says the job started. The finished state arrives later and is attached to the same call.

| Product | How the end of the job is attached |
|---|---|
| Claude Code | A later task notification (an `Origin::Notification` message) names the call by its tool use id. Its status and exit code enrich the call. `BashOutput` and `TaskOutput` calls are `Tool::Process` reads of the same id |
| Codex | `write_stdin` and `wait` calls are `Tool::Process` calls. A late command event is joined to the cell whose output names the same `session_id` |
| OpenCode | A background completion record is matched to the call by `shellID` |
| Gemini CLI | The call records `is_background` and the process id in its text |

A call whose result says the job is still running is never marked `Interrupted` just because the log ends. It stays `Pending` until its own completion record arrives.

## Real examples

All of these come from the synthetic fixtures in `tests/fixtures/`.

### Codex: exit code 3, one stream

From `tests/fixtures/codex/exit3.jsonl`, shown with `agentparse show`:

```
> $ sh test.sh   (in /work/demo)  [error: failed]
  exit 3 | 0.0s
  output:
    err-line
    out-line
```

The same call as JSON, trimmed:

```json
{
  "name": "CommandExecution",
  "tool": {
    "kind": "shell",
    "command": "sh test.sh",
    "argv": ["/bin/zsh", "-lc", "sh test.sh"],
    "cwd": "/work/demo"
  },
  "status": "error",
  "status_detail": "failed",
  "output": {
    "text": "",
    "shell": { "exit_code": 3, "stdout": "err-line\nout-line\n", "duration_ms": 0 }
  }
}
```

The command ran for a few microseconds, so `duration_ms` is 0. The order of lines in a merged stream is the order the product captured them. Do not rely on it to tell stdout from stderr. This call ran inside a Codex code cell, so it is a child of a `Tool::Code` call. `Session::tool_calls()` returns it anyway.

### Claude Code: separate stdout and stderr

From `tests/fixtures/claude/old-format.jsonl`, where the result object has both streams:

```json
{
  "tool": { "kind": "shell", "command": "sh test.sh", "cwd": "/work/demo" },
  "status": "error",
  "output": {
    "text": "Exit code 3\nout-line\nerr-line",
    "shell": { "exit_code": 3, "stdout": "out-line", "stderr": "err-line" }
  }
}
```

In `tests/fixtures/claude/main-session.jsonl` the same command failed with an error result that is text only. There the streams are merged and `stderr` is `None`:

```json
{ "exit_code": 3, "stdout": "out-line\nerr-line" }
```

### Claude Code: timeout

```json
{
  "tool": { "kind": "shell", "command": "sleep 300", "cwd": "/work/demo", "timeout_ms": 120000 },
  "status": "error",
  "output": {
    "text": "Exit code 143\nCommand timed out after 2m 0s",
    "shell": { "exit_code": 143, "stdout": "", "timed_out": true }
  }
}
```

### Claude Code: background job

```json
{
  "tool": { "kind": "shell", "command": "sleep 2; echo bg-done", "cwd": "/work/demo", "background": true },
  "status": "ok",
  "status_detail": "completed",
  "output": {
    "shell": { "exit_code": 0, "stdout": "", "background_id": "bk7d2m9qa" },
    "full_output_path": "/tmp/claude-1000/-work-demo/3f2a.../tasks/bk7d2m9qa.output"
  }
}
```

The launch result carries the id. The exit code and the `completed` label come from the task notification that arrives later. In `agentparse show` this prints as:

```
> $ sleep 2; echo bg-done   (in /work/demo)  &
  exit 0 | background bk7d2m9qa
```

A `TaskOutput` call that polls the same id is a process read:

```
> read process bk7d2m9qa
```

### Claude Code: rejected and interrupted

```
> $ rm -rf build   (in /work/demo)  [rejected: user-rejected]
> $ cargo build   (in /work/demo)  [interrupted: interrupted]
```

A rejected command never ran, so it has no `shell` result at all, only the text the model saw. `status_detail` keeps the product's reason.

### OpenCode: truncated output

From the v1 database fixture:

```json
{
  "name": "bash",
  "tool": { "kind": "shell", "command": "bun test", "cwd": "/work/demo", "timeout_ms": 120000 },
  "status": "error",
  "status_detail": "completed",
  "output": {
    "shell": { "exit_code": 2, "duration_ms": 2100 },
    "truncated": true,
    "full_output_path": "/home/demo/.local/share/opencode/tool-output/tool_c0ffee000001"
  }
}
```

OpenCode called the tool `completed`, but the exit code was 2, so the normalized status is `error`. The product's word survives in `status_detail`. (The `shell.stdout` and `text` fields hold the truncation notice and are left out above.)

### Gemini CLI: a failed command the product called a success

```json
{
  "name": "run_shell_command",
  "tool": { "kind": "shell", "command": "bun test", "cwd": "/home/user/proj" },
  "status": "error",
  "status_detail": "success",
  "output": {
    "shell": { "exit_code": 1, "stdout": "3 pass\n1 fail" }
  }
}
```

Gemini CLI reports a non-zero exit under a `success` status. Because the exit code is 1, the normalized status is `error`.

## Example: read the streams correctly

```rust
use agentparse::{Tool, parse_file};

fn main() -> Result<(), agentparse::Error> {
    let path = std::env::args().nth(1).expect("usage: shell_streams <session file>");
    for call in parse_file(path.as_ref())?.tool_calls() {
        let (Tool::Shell(shell), Some(out)) = (&call.tool, &call.output) else { continue };
        let Some(result) = &out.shell else { continue };
        match (&result.stdout, &result.stderr) {
            // The product recorded both streams.
            (stdout, Some(stderr)) => println!("{}: stdout {:?}, stderr {stderr:?}", shell.command, stdout),
            // One interleaved stream: it is in `stdout`, and `stderr` is None.
            (Some(both), None) => println!("{}: combined {both:?}", shell.command),
            (None, None) => println!("{}: no output recorded", shell.command),
        }
        if result.timed_out || result.interrupted {
            println!("  timed out: {}, interrupted: {}", result.timed_out, result.interrupted);
        }
        if let Some(id) = &result.background_id {
            println!("  still running in the background as {id}");
        }
    }
    Ok(())
}
```

On `tests/fixtures/claude/old-format.jsonl`:

```
sh test.sh: stdout Some("out-line"), stderr "err-line"
```

On `tests/fixtures/codex/exit3.jsonl`:

```
ls -la: combined "total 0\n"
sh test.sh: combined "err-line\nout-line\n"
cat a.txt: combined "one\ntwo\nthree\n"
```

On `tests/fixtures/claude/main-session.jsonl`, trimmed:

```
sleep 300: combined ""
  timed out: true, interrupted: false
sleep 2; echo bg-done: combined ""
  still running in the background as bk7d2m9qa
cargo build: no output recorded
  timed out: false, interrupted: true
```

## Next

- [Usage and Status](#conventions)
- [Tool Calls](#tools)
