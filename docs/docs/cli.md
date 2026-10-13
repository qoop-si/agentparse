# CLI

The `agentparse` binary lists sessions, prints one as a readable transcript or as JSON, and measures how well every session on the machine parses. It uses the same library as your code and the same default locations.

```bash
agentparse list  [--agent claude|codex|opencode|gemini] [--json]
agentparse show  <file | session id> [--json] [--full]
agentparse stats [--agent claude|codex|opencode|gemini] [--json]
```

Install it with `cargo install --path .` (see [Installation](#installation)).

## Global behavior

| Topic | Behavior |
|---|---|
| Locations | The default locations and environment overrides from `Roots::from_env()`: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_DB` and `XDG_DATA_HOME`, `GEMINI_CLI_HOME` |
| `--agent` | Limits `list` and `stats` to one product. The name is case insensitive. An unknown name prints `unknown agent "<name>" (claude, codex, opencode, gemini)` and exits with 1 |
| Warnings | Discovery warnings (an unreadable directory) go to stderr as `warning: ...` |
| Exit status | 0 on success. 1 for usage errors, an unknown session id, or a file that cannot be parsed. The message goes to stderr |
| Closed pipes | `agentparse list | head` exits quietly when the reader closes |
| No arguments | Prints the usage text and exits with 1 |

All examples below come from the synthetic fixtures in `tests/fixtures/` of the repository, laid out the way each product lays out its own files. Paths in the output are shortened.

## list

Lists every session found, newest first by last write time. Subagent and child sessions are listed too, with their parent.

```bash
agentparse list
agentparse list --agent opencode
agentparse list --json
```

Text output has one line per session: the product, the last write time (UTC), the session id, where it is, and the parent when there is one.

```
gemini   2026-10-13 06:33:32  aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee  /fixtures/gemini/.gemini/tmp/proj/chats/session-2026-10-12T09-00-aaaaaaaa.jsonl
codex    2026-10-13 06:33:32  0190a000-0000-7000-8000-000000000001  /fixtures/codex/sessions/2026/10/13/rollout-2026-10-13T01-34-18-0190a000-0000-7000-8000-000000000001.jsonl
claude   2026-10-13 06:33:32  a1b2c3d4e5f607182  /fixtures/claude/projects/-work-demo/3f2a9c10-.../subagents/agent-a1b2c3d4e5f607182.jsonl  (child of 3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11)
claude   2026-10-13 06:33:32  3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11  /fixtures/claude/projects/-work-demo/3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11.jsonl
opencode 2026-10-13 00:07:39  ses_ecd6ad17ff80AEbRxnOg8B21Mn  /fixtures/oc/opencode.db#ses_ecd6ad17ff80AEbRxnOg8B21Mn
opencode 2026-10-13 00:07:02  ses_ecd6a835ff7fukfa0R96znWxUu  /fixtures/oc/opencode.db#ses_ecd6a835ff7fukfa0R96znWxUu  (child of ses_ecd6ad17ff80AEbRxnOg8B21Mn)
```

An OpenCode session is shown as `<database path>#<session id>`.

With `--json` the output is a JSON array of `SessionRef` objects:

```json
[
  {
    "agent": "codex",
    "id": "0190a000-0000-7000-8000-000000000001",
    "source": {
      "kind": "file",
      "path": "/fixtures/codex/sessions/2026/10/13/rollout-2026-10-13T01-34-18-0190a000-0000-7000-8000-000000000001.jsonl"
    },
    "updated_at": 1791873212592
  }
]
```

## show

Parses one session and prints it. The target is either a file path or a session id.

```bash
agentparse show tests/fixtures/codex/exit3.jsonl
agentparse show 3f2a9c10-7b4e-4d58-9a61-0c5e8d2b7a11
agentparse show ses_ed52a75fff9cZv864nVGS5jPGC --json
```

| Target | How it is resolved |
|---|---|
| A path to a file | Parsed directly with `parse_file`. The product is detected |
| Anything else | Looked up as a session id among everything `list` finds. The first match is parsed. If none matches, the output is `no session with id <id>` |

An OpenCode database file is not a session file. Show its sessions by id.

| Flag | Effect |
|---|---|
| `--json` | Print the whole `Session` as pretty JSON. See [Sessions and Entries](#model) for the shape |
| `--full` | Print without the line limits described below |

### Reading the transcript

The first lines are the session header: the product, id and format, then the title, cwd, branch, models, parent and summed usage when known. Then come the entries in order.

| Line | Meaning |
|---|---|
| `-- user  <time>` | A human message |
| `-- assistant <model>  <time>  [in .. out .. cache r.. w.. reasoning ..]` | A model message with its usage. `cache r` is cache read and `w` is cache write. A cost is shown as `$0.0033` |
| `-- system (<kind>)`, `-- user (<kind>)` | A message that is not human. The product's label is in parentheses, and the origin is used when there is no label |
| `[reasoning]`, `[reasoning, not readable]` | A reasoning part, with text or encrypted |
| `> $ <command>   (in <cwd>)` | A shell command. A trailing `&` means it ran in the background |
| `exit 3 | 0.1s | timed out | interrupted | background <id>` | The facts of a shell result |
| `output:` or `stdout:` and `stderr:` | The command's streams. `output:` means one combined stream |
| `[error: failed]`, `[rejected: user-rejected]` | A status other than `ok`, with the product's own label after the colon |
| `update /path +1 -1` | A file change: the operation, the path, and added and removed lines |
| `-> session <id>` | The call spawned this child session |
| `== compaction (auto) 177627 -> 3771 tokens` | A compaction with its trigger and token counts |
| `== turn completed 14.7s` | The end of a turn |
| `== settings model=... cwd=...` | A settings change |
| `== notice`, `== hook`, `== error`, `== rewind: 2 entries dropped` | The other event entries |
| `(token_count)` | A record the model does not cover, listed by its kind |

Without `--full`, long blocks are cut with `... N more lines`:

| Block | Lines shown |
|---|---|
| Message text | 40 |
| Reasoning | 4 |
| Compaction summary | 6 |
| Shell streams, and the result of an orphan tool result | 12 |
| Tool result text of calls that are not shell commands | 6 |

`--full` lifts these limits, except that the result text of non-shell tools still stops at 6 lines. Use `--json` to get a complete tool result.

### Example: Codex, exit code 3

```bash
agentparse show tests/fixtures/codex/exit3.jsonl
```

```
codex session 0190a000-0000-7000-8000-000000000001  [codex-paginated]
  title: run the demo checks
  cwd: /work/demo
  branch: main
  models: gpt-6.1-sol
  usage: in 30447 out 215 cache r38656 w0
   (task_started)

-- system (host_skills.instructions)  2026-10-13 01:34:18
  Synthetic context block 0.

-- user  2026-10-13 01:34:18
  run the demo checks

-- assistant gpt-6.1-sol  2026-10-13 01:34:21  [in 9882 out 94 cache r12288 w0]
  Synthetic assistant text 79.
  > code cell (3 lines)
    0.8s
    > $ ls -la   (in /work/demo)
      exit 0 | 0.1s
      output:
        total 0
    > $ sh test.sh   (in /work/demo)  [error: failed]
      exit 3 | 0.0s
      output:
        err-line
        out-line
    > $ cat a.txt   (in /work/demo)
      exit 0 | 0.0s
      output:
        one
        two
        three
   (token_count)

-- assistant gpt-6.1-sol  2026-10-13 01:34:27  [in 10227 out 87 cache r13184 w0]
  > code cell (1 lines)
    0.2s
    > FileChange
      update /work/demo/a.txt +1 -1
   (token_count)

-- assistant gpt-6.1-sol  2026-10-13 01:34:30  [in 10338 out 34 cache r13184 w0]
  Synthetic assistant text 102.
   (token_count)
== turn completed 14.7s
```

### Example: Claude Code, background and rejected commands

```bash
agentparse show tests/fixtures/claude/main-session.jsonl
```

Trimmed:

```
-- assistant claude-sonnet-5-5  2026-10-12 09:00:09  [in 2 out 90 cache r12069 w120]
  > $ sleep 300   (in /work/demo)  [error]
    exit 143 | timed out

-- assistant claude-sonnet-5-5  2026-10-12 09:00:29  [in 2 out 98 cache r13750 w110]
  > $ sleep 2; echo bg-done   (in /work/demo)  &
    exit 0 | background bk7d2m9qa

-- assistant claude-sonnet-5-5  2026-10-12 09:00:41  [in 2 out 75 cache r14710 w130]
  I will remove the build directory.
  > $ rm -rf build   (in /work/demo)  [rejected: user-rejected]
      The user doesn't want to proceed with this tool use. ...

-- assistant claude-sonnet-5-5  2026-10-12 09:00:45  [in 2 out 70 cache r14850 w100]
  > $ cargo build   (in /work/demo)  [interrupted: interrupted]
    interrupted
== turn interrupted

== compaction (auto) 177627 -> 3771 tokens  2026-10-12 09:00:54
  This session is being continued from a previous conversation that ran out of context. ...
```

### Example: Gemini CLI

```bash
agentparse show tests/fixtures/gemini/gemini-new-session.jsonl
```

See the full output on the [Gemini CLI](#gemini) page. A failed test run, a cancelled command and a rewind look like this:

```
  > $ bun test   (in /home/user/proj)  [error: success]
    exit 1
    output:
      3 pass
      1 fail
...
  > $ sleep 600  [interrupted: cancelled: User cancelled the operation.]
    interrupted
== error api_error: [API Error: example error text]
...
== rewind: 2 entries dropped
```

### Example: JSON

```bash
agentparse show tests/fixtures/codex/exit3.jsonl --json
```

The output is the whole session. The header fields come first, then `entries`. The header of this one, with `entries` left out:

```json
{
  "agent": "codex",
  "id": "0190a000-0000-7000-8000-000000000001",
  "format": "codex-paginated",
  "title": "run the demo checks",
  "cwd": "/work/demo",
  "git": { "branch": "main", "commit": "0123456789abcdef0123456789abcdef01234567" },
  "cli_version": "0.162.1",
  "models": ["gpt-6.1-sol"],
  "usage": { "input": 30447, "output": 215, "reasoning": 0, "cache_read": 38656, "cache_write": 0 }
}
```

Pipe it to `jq` to query it:

```bash
agentparse show session.jsonl --json | jq '[.entries[] | select(.type == "message") | .parts[] | select(.type == "tool_call")] | length'
```

## stats

Parses every session it finds and prints counts and timings, per product. It is a way to check how well agentparse reads your own history, and how fast.

```bash
agentparse stats
agentparse stats --agent claude
agentparse stats --json
```

Text output, for the OpenCode fixture database:

```
== opencode
  sessions 4  failures {}  warnings 0  entries 30  orphan results 0
  parse ms p50 0.98 p90 1.82 p99 1.82 max 1.8  total 0.0s  0 MB/s
  entries: message 23, turn 3, settings 2, compaction 1, error 1
  messages: assistant/model 11, user/human 5, system/context 3, user/context 2, user/command 1, user/notification 1
  tools: shell 4, read 2, task 2, edit 1, patch 1, search 1
  status: ok 10, error 1
  exit codes: 0 4
  other kinds: part:step-finish 4, part:step-start 4, part:agent 1
```

| Line | Meaning |
|---|---|
| `sessions` | Sessions parsed |
| `failures` | Sessions that did not parse, counted by error kind: `io`, `sqlite`, `not_a_session`, `not_found` |
| `warnings` | Total warnings across sessions |
| `entries` | Total entries |
| `orphan results` | Tool results whose call was not in the parsed records |
| `parse ms` | Parse time per session: p50, p90, p99 and max, in milliseconds |
| `total`, `MB/s` | Total parse time and throughput over file bytes. A database reports 0 MB/s because its size is not counted |
| `entries:` | Entries by type |
| `messages:` | Messages by role and origin |
| `tools:` | Tool calls by kind, nested calls included |
| `status:` | Tool calls by status |
| `exit codes:` | Shell commands by exit code. `none` means the product recorded no code |
| `other kinds:` | Records and blocks that stayed as `Other`, by kind |

Each list shows up to 25 items, most frequent first. Anything unexpected in `failures`, `orphan results` or `other kinds` is worth a look.

With `--json` the output is an object keyed by product, with the same figures as numbers and maps. For the Gemini CLI fixture, without the timing fields:

```json
{
  "sessions": 1,
  "failures": {},
  "bytes": 6538,
  "entries": 9,
  "warnings": 0,
  "messages_by_origin": { "assistant/model": 4, "user/context": 1, "user/human": 2 },
  "entries_by_type": { "error": 1, "message": 7, "rewind": 1 },
  "tool_calls_by_kind": { "edit": 1, "read": 1, "shell": 2 },
  "tool_calls_by_status": { "error": 2, "interrupted": 1, "ok": 1 },
  "shell_exit_codes": { "1": 1, "none": 1 },
  "other_kinds": {},
  "orphan_results": 0
}
```

The timing fields (`parse_ms_p50`, `parse_ms_p90`, `parse_ms_p99`, `parse_ms_max`, `total_s`, `mb_per_s`) are in the same object.

## Privacy

`list`, `show` and `stats` print what is in your sessions: prompts, commands and their output. Check the output before you paste it anywhere.
