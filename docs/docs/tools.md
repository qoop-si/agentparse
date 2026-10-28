# Tool Calls

Every product names its tools differently and shapes their inputs differently. agentparse keeps the input exactly as recorded and adds a typed view next to it.

| Field of `ToolCall` | What it holds |
|---|---|
| `name` | The tool name as recorded, for example `Bash`, `exec_command`, `run_shell_command` |
| `input` | The input as recorded: parsed JSON arguments, or a string for freeform input |
| `tool` | The typed view, a `Tool`. The same enum for every product |
| `status` | `Pending`, `Ok`, `Error`, `Rejected` or `Interrupted` |
| `output` | The result, a `ToolOutput`, already attached to the call |

Use `tool` to write code that works for every product. Use `name` and `input` when you need something the typed view does not cover.

## The Tool enum

| Variant | Fields | Used for |
|---|---|---|
| `Shell` | a `Shell` (see [Shell Commands](#shell)) | Running a command |
| `Process` | `id`, `action`, `input` | Polling, writing to, or killing a running shell. `action` is `Read`, `Write` or `Kill` |
| `Read` | `path`, `offset`, `limit` | Reading a file. `offset` and `limit` are as recorded, so their base (line or byte, 0 or 1) is the product's |
| `Write` | `path`, `content` | Writing a whole file |
| `Edit` | `path`, `edits` | Replacing text. `edits` is a list of `TextEdit` with `old`, `new`, `replace_all` |
| `Patch` | `patch` | An `apply_patch` envelope. Per-file changes are in `output.files` |
| `Search` | `pattern`, `path`, `glob` | Searching file contents |
| `Glob` | `pattern`, `path` | Finding files by pattern |
| `List` | `path` | Listing a directory |
| `WebFetch` | `url`, `prompt` | Fetching a page |
| `WebSearch` | `query` | Searching the web |
| `Todo` | `items` | Writing a task list. Each `TodoItem` has `content`, `status`, `priority` |
| `Task` | `description`, `prompt`, `agent_type`, `name` | Spawning, or sending a message to, a subagent. `name` is the agent the call names: Claude `Agent` `name` and `SendMessage` `to`, Codex `target` |
| `Mcp` | `server`, `tool` | A call to an MCP server tool |
| `Code` | `source` | A JavaScript program the model wrote for the harness to run (Codex code mode) |
| `Question` | none | Asking the user a question |
| `Plan` | none | Entering or leaving plan mode |
| `Skill` | `name` | Loading a skill |
| `Other` | none | Everything else. The raw `input` is still there |

All optional fields are `Option` values and are left out of the JSON when absent.

### How inputs are read

| Rule | Detail |
|---|---|
| Aliases | Field names that differ between products or versions are accepted: `file_path`, `filePath`, `path`, `absolute_path` for a path, `dir_path` and `directory` for a directory, `old_string` and `oldString` for the text to replace |
| Numbers as strings | A number recorded as a string is still read as a number |
| Unparsed input | If the product could not parse the model's JSON (Claude Code `__unparsedToolInput`), the raw text is parsed again |
| Missing required field | If a tool lacks a field its typed variant needs, the call becomes `Tool::Other`. The raw `input` is still there |
| Unknown fields | Ignored, never an error |

## Name mapping

Which recorded tool name becomes which `Tool`. "none" means the product has no such tool, or its tool is not typed.

| Tool | Claude Code | Codex | OpenCode (v1 / v2) | Gemini CLI |
|---|---|---|---|---|
| `Shell` | `Bash`, `Monitor` (background) | `shell`, `shell_command`, `exec_command`, `local_shell_call`, and the `CommandExecution` item inside a code cell | `bash` / `shell` | `run_shell_command` |
| `Process` | `BashOutput`, `TaskOutput` (read). `KillShell`, `KillBash`, `TaskStop` (kill) | `write_stdin` (write). `wait` (read, or kill when called with `terminate`). `unified_exec_interaction` item (write) | none | none |
| `Read` | `Read` | `view_image` | `read` | `read_file` |
| `Write` | `Write` | none | `write` | `write_file` |
| `Edit` | `Edit`, `MultiEdit` | none | `edit` | `replace` |
| `Patch` | none | `apply_patch` | `apply_patch` / `patch` | none |
| `Search` | `Grep` | none | `grep` | `grep_search`, `search_file_content` |
| `Glob` | `Glob` | none | `glob` | `glob` |
| `List` | `LS` | none | `list` | `list_directory` |
| `WebFetch` | `WebFetch` | `web__run` open and find, `web_search_call` open_page and find_in_page | `webfetch` | `web_fetch` |
| `WebSearch` | `WebSearch` | `web__run` search, `Extension web.search`, `web_search_call` search | `websearch` | `google_web_search` |
| `Todo` | `TodoWrite`, and task list tools that write the list | `update_plan` | `todowrite` | `write_todos`, `write_todos_list` |
| `Task` | `Task`, `Agent`, `SendMessage` | `spawn_agent`, `send_message`, `followup_task`, `wait_agent` | `task` / `subagent` | `invoke_agent`, and agent tools called with an `agentId` |
| `Mcp` | `mcp__<server>__<tool>` | `mcp__<server>__<tool>`, and the `McpToolCall` item | not typed (the names are ambiguous with plugin tools) | `mcp_<server>_<tool>`, split at the first `_` after `mcp_`. The raw name is kept |
| `Code` | none | the custom `exec` tool (code mode) | none | none |
| `Question` | `AskUserQuestion` | `request_user_input` | `question` | `ask_user` |
| `Plan` | `EnterPlanMode`, `ExitPlanMode` | none (a Codex plan item is assistant text) | `plan_enter`, `plan_exit` | `enter_plan_mode`, `exit_plan_mode` |
| `Skill` | `Skill`, `SlashCommand` | none | `skill` | `activate_skill` |
| `Other` | `NotebookEdit`, `ToolSearch`, and everything else. | everything else | `invalid`, plugin tools, MCP tools | `read_many_files`, `save_memory`, tracker tools, and everything else |

Gemini CLI MCP names have a known limit: server names may contain `_`, so the split at the first `_` is a best guess. The raw name is always in `ToolCall.name`.

## Subagents and code cells

| Case | How it shows up |
|---|---|
| Subagent | The spawning call is a `Tool::Task`. Its `child_session` holds the child session id. The child is a separate `Session` with `parent` set |
| Code cell | Codex code mode runs JavaScript. The cell is a `Tool::Code` call. The tool calls it made are its `children`, in order. `Session::tool_calls()` returns the parent, then its children |

A child call inside a Codex cell has `name` set to the item type (such as `CommandExecution`) and `input` set to null. The completion event is in `output.data`.

A cell status is the cell's own: a script that ran a failing command still has status `Ok` (`Script completed`). Look at the children for exit codes. A command the sandbox refused leaves no item, so its child is read from the cell's printed chunk and has `name` `exec_command` (or `write_stdin`), the call's argument as `input`, and an id of `<cell id>#<n>`. Known limit: a `write_stdin` poll is always read from its chunk, even when the command it polls also has a recorded `Shell` child, so the same run can show as a `Process` and a `Shell` child. See [Codex](#codex).

## Changed files

Calls that change files fill `output.files` with one `FileChange` per file.

| Product | Where the changes come from |
|---|---|
| Claude Code | `Edit` and `MultiEdit` hunks (`structuredPatch`) become one `Update` with a unified diff. `Write` becomes `Add` or `Update`. A `Bash` result can carry changed files too (`bashEditDiff.files`) |
| Codex | The `apply_patch` input is parsed. It is replaced by the `FileChange` items or the patch end event when those are present |
| OpenCode | `edit` file diff, `apply_patch` and v2 file lists |
| Gemini CLI | The result display file diff, with a flag for new files |

Each `FileChange` has `op` (`Add`, `Update`, `Delete`, `Move`), an optional `move_to`, an optional `diff`, and optional `additions` and `deletions`. A real Gemini CLI edit, from `tests/fixtures/gemini/gemini-new-session.jsonl`:

```json
{
  "tool": {
    "kind": "edit",
    "path": "/home/user/proj/src/parser.ts",
    "edits": [{ "old": "i <= n", "new": "i < n" }]
  },
  "output": {
    "files": [{
      "path": "/home/user/proj/src/parser.ts",
      "op": "update",
      "diff": "Index: parser.ts\n...",
      "additions": 1,
      "deletions": 1
    }]
  }
}
```

## The structured result in `data`

`ToolOutput.data` holds the structured result as the product recorded it, minus the values that already have a typed home:

| Product | `data` holds |
|---|---|
| Claude Code | `toolUseResult`, minus `stdout`, `stderr`, and file content that equals the text |
| Codex | The completion event or events, minus the aggregated output |
| OpenCode | The tool `state`, minus `input` and `output` |
| Gemini CLI | The tool call record, minus `args` and `result` |

Inline base64 becomes `{"bytes": n}`. Nothing is stored twice, except two deliberate overlaps: the raw `input` next to the typed `tool`, and `output.text` (what the model saw) next to `shell.stdout` and `shell.stderr` (what the process wrote), which are often different texts.

## Example: find failed calls

```rust
use agentparse::{ToolStatus, parse_file};

fn main() -> Result<(), agentparse::Error> {
    let path = std::env::args().nth(1).expect("usage: failures <file>");
    let session = parse_file(path.as_ref())?;
    for call in session.tool_calls().into_iter().filter(|c| c.failed()) {
        let why = match call.status {
            ToolStatus::Rejected => "rejected",
            ToolStatus::Interrupted => "interrupted",
            _ => "error",
        };
        println!("{} {why} {:?}", call.name, call.status_detail);
    }
    Ok(())
}
```

On `tests/fixtures/claude/main-session.jsonl`:

```
Bash error None
Bash error None
Bash rejected Some("user-rejected")
Bash interrupted Some("interrupted")
Bash interrupted None
```

`status_detail` keeps the product's own label (`user-rejected` here), so you can tell a user denial from a permission rule without re-reading the source.

## Next

- [Shell Commands](#shell)
- [Usage and Status](#conventions)
