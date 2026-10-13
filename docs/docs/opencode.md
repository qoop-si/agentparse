# OpenCode

OpenCode keeps sessions in one SQLite database. Two generations of tables live in that file, and an older version kept sessions as a tree of JSON files. agentparse reads all three.

## Where the data is

| Kind | Path under the data directory (`~/.local/share/opencode`, or `$XDG_DATA_HOME/opencode`) |
|---|---|
| Database | `opencode.db`, with `-wal` and `-shm` files next to it while OpenCode runs |
| Other release channels | `opencode-<channel>.db`. Discovery reads every `opencode*.db` in the directory |
| Legacy JSON tree | `storage/session/<project>/<session id>.json`, with messages in `storage/message/<session id>/` and parts in `storage/part/<message id>/` |

`OPENCODE_DB` points discovery at one database. An absolute path is used as is, and a bare name is looked up inside the data directory.

A legacy JSON session is listed only when no database already holds the same id.

The database is opened read-only and read through SQLite, so the write-ahead log is included and the file is never copied. A busy database is retried for 2 seconds. The credential and account tables are never read.

## Table generations

| `Session.format` | Where the session lives | Rows used, in order |
|---|---|---|
| `opencode-v1` | Only in the `session` table | `message` and `part` rows by `(time_created, id)`, parts by id |
| `opencode-v2` | Only in the `session_v2` table (OpenCode 2.x) | `session_message` rows by `seq` |
| `opencode-v1+v2` | In both: a migrated session | The v1 rows first, since they are the lossless copy. Then the v2 rows whose id is not a v1 message id, by `seq`. The migration notice and everything written after the upgrade are such rows |
| `opencode-json` | The legacy JSON tree | Message and part files |

A database from a 1.x version has an empty `session_v2` table. Such a session is read from the v1 tables.

## How rows become entries

### v1 rows

| Source | Becomes |
|---|---|
| Assistant message | One `Message`. It is one step, and its parts map one to one |
| `text`, `reasoning`, `file` parts | `Part::Text`, `Part::Reasoning`, `Part::Attachment` |
| `tool` part | `Part::ToolCall`. The tool `state` holds input, output and metadata |
| `step-start`, `step-finish`, `snapshot`, `patch` parts | `Part::Other`. The `step-finish` token counts are never added again |
| `retry` part, assistant `error` | An `Error` entry right after the message |
| User message with a `compaction` part, plus the paired assistant message with `summary: true` | One `Compaction`. The paired summary message is absorbed |
| A `!cmd` user and assistant pair | A `Command` message holding a `Shell` call. The call id is the tool `callID` |

### v2 rows

| `session_message` type | Becomes |
|---|---|
| `user` | A user `Message` |
| `assistant` | A `Message` whose `content[]` of text, reasoning and tool blocks become parts |
| `synthetic` | A `Message` with origin `Notification` or `Context`, by `metadata.source`. A background shell completion also enriches the call with the same `shellID` |
| `system` | A `Message` with role `System` |
| `skill` | A user `Message` with `kind: "skill"` |
| `shell` | A `Command` message holding a `Shell` call. The call id is the `shellID` |
| `compaction` | `Compaction` |
| `idle` | `Turn` |
| `agent-switched`, `model-switched`, `location-switched` | `Settings` |

### Both generations

| Source | Becomes |
|---|---|
| `todo` table rows | One final `Other` entry with `kind: "todos"` |
| Session row columns a field takes (`directory`, `title`, `version`, times) | `Session.cwd`, `title`, `cli_version`, `started_at`, `updated_at` |
| Other session columns | `Session.extra`. JSON columns are parsed |
| Tool state `pending` or `running` while later messages exist | `Interrupted` |

Tool names from both generations are understood: `bash` and `shell`, `task` and `subagent`, `apply_patch` and `patch`, and the input keys `filePath` and `path`. See [Tool Calls](#tools).

## Usage

One `Message.usage` per assistant message. OpenCode counts reasoning separately from output, so agentparse adds it to `output` and also reports it in `reasoning`. `cost_usd` is the recorded cost, and the session cost is the sum of recorded costs. See [Usage and Status](#conventions).

## Subagents

A subagent is a separate session linked by the `parent_id` column. The parent's `task` (v1) or `subagent` (v2) call carries the child id, which becomes `ToolCall.child_session`. `ParentLink.call_id` holds that call's id.

## Using the database

OpenCode sessions have no file of their own, so use the dedicated functions:

```rust
use agentparse::opencode;
use std::path::Path;

fn main() -> Result<(), agentparse::Error> {
    let db = std::env::args().nth(1).expect("usage: opencode_db <opencode.db>");
    for r in opencode::list(Path::new(&db))? {
        let session = opencode::parse_db(Path::new(&db), &r.id)?;
        println!("{} {} {} entries", session.id, session.format, session.entries.len());
    }
    Ok(())
}
```

On a database built from `tests/fixtures/opencode/v2.sql` (`sqlite3 opencode.db < v2.sql`):

```
ses_ecd6ad17ff80AEbRxnOg8B21Mn opencode-v2 19 entries
ses_ecd6a835ff7fukfa0R96znWxUu opencode-v2 3 entries
ses_ed52a75fff9cZv864nVGS5jPGC opencode-v1+v2 5 entries
ses_ed52a633ff93Y7Vatlwl1zyPKr opencode-v1+v2 3 entries
```

From the command line, pass the session id. The CLI finds the database through `OPENCODE_DB` or the default location:

```bash
OPENCODE_DB=opencode.db agentparse show ses_ed52a75fff9cZv864nVGS5jPGC
```

```
opencode session ses_ed52a75fff9cZv864nVGS5jPGC  [opencode-v1+v2]
  title: Update the readme
  cwd: /work/demo
  models: demo-gpt
  usage: in 2400 out 150 reasoning 20 $0.0033

-- user  2026-10-11 12:00:00
  update the readme

-- assistant demo-gpt  2026-10-11 12:00:01  [in 700 out 70 reasoning 20 $0.0011]
  (step-start)
  [reasoning, not readable]
  Checking the readme.
  > $ cat README.md   (in /work/demo)
    exit 0 | 0.0s
    output:
      # demo
  (step-finish)

-- assistant demo-gpt  2026-10-11 12:00:04  [in 800 out 60 $0.0013]
  (step-start)
  > task general: Summarize the changelog
      <task id="ses_ed52a633ff93Y7Vatlwl1zyPKr" state="completed">
      <task_result>
      One bugfix release.
      </task_result>
      </task>
    -> session ses_ed52a633ff93Y7Vatlwl1zyPKr
  (step-finish)
```

A legacy JSON session can be parsed from its session file with `parse_file`, or with `opencode::parse_storage_file`.

## Known limits

| Limit | Detail |
|---|---|
| A database path is not a session file | `agentparse show <path to opencode.db>` fails with `not a session`. Show a session by id, or use `opencode::parse_db` |
| Reasoning is often unreadable | OpenAI reasoning is stored only as an encrypted blob. Such a part is `Reasoning { text: None, encrypted: true }` |
| One output stream | OpenCode records no separate stderr. `stderr` stays `None` |
| MCP tools are not typed | Their names are ambiguous with plugin tools, so they stay `Tool::Other`. The raw name is in `ToolCall.name` |
| Full output files expire | `full_output_path` points at a file OpenCode may have deleted already, since it keeps truncated output for about a week |
| No per-message system prompt | v1 stores none, and v2 stores prompts separately. They are not part of the model |
| v2 is the lossy copy | For a migrated session the v1 rows win. The v2 copy has no step records |
