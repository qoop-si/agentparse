# Review 2: API design, minimalism, real consumers

Reviewer lens: minimal clean API, and whether `qoop-history` and `wend-core` can really sit on top of it.
Plan reviewed: `plans/agentparse.md` (v1, 2026-10-13). Consumers read: `qoop-tui/crates/qoop-history/src/*`,
`wend/crates/wend-core/src/{parser,index,recover,model.rs}`.

## Experiments run (scratch dir under the session scratchpad)

- serde: Entry with `#[serde(flatten)] kind: EntryKind` where `EntryKind` is `#[serde(tag = "type")]` and the variants are
  newtype structs (Message, Compaction, Turn, Other). Also `Part` (tag "type") holding `ToolCall(ToolCall)` whose field `tool: Tool`
  is tagged `kind`, children nested, `Other { kind, data: Value }` inside the flattened enum. Result: compiles, serializes,
  deserializes, round-trips equal (`==`) in every case, including `u64::MAX` and a `data` object that itself has a `type` key.
  The flatten pitfalls (u128, arbitrary_precision, untagged inside flatten) do not hit this shape. Two real gotchas found, see F9.
- rusqlite range: a crate with `rusqlite = ">=0.32, <0.41"` (bundled) plus a consumer pinning `0.32`, then `0.37`, then each of
  `=0.33 .. =0.40`: all resolve to ONE rusqlite and ONE libsqlite3-sys (0.32.1/0.30.1, 0.37.0/0.35.0) and build and run
  (open_with_flags READ_ONLY, busy_timeout, prepare, query_map all compile across the whole range). The idea works.
- ruzstd: `ruzstd = "0.9"` (0.9.1, one dep: twox-hash) `StreamingDecoder` over a `zstd -3` file wrapped in `BufReader::lines()`
  read all 200000 lines. Upstream Codex confirmed: `codex-rs/rollout/src/compression.rs` writes standard single-frame zstd
  (level 3) and `thread-store` handles `.jsonl.zst` paths. Sound.

## Findings

1. **BLOCKER** (section 3 `Entry`, section 6 Claude). The Claude `parentUuid` graph breaks when one API message is merged from
   several lines. Children point at the uuid of a specific line, but `Entry` has one `id` and one `line`. The research file
   (research-claude.md line 426) already says to keep `uuids`/`source_lines`; the plan dropped it. wend's recovery DFS and
   boundary `logical_parent_uuid` lookups need every line uuid resolvable.
   Change: add `Entry.lines: Vec<LineRef>` where `LineRef { line: usize, id: Option<String>, ts: Option<i64> }` (replaces
   `Entry.line`, one entry for non-merged records), and state the rule: every source line that has a uuid ends up in exactly one
   entry's `lines`, and a line with a uuid that maps to no model type still becomes an `Other` entry carrying `id` + `parent_id`
   (wend keeps `progress`, `attachment`, non-boundary `system` lines as graph nodes precisely because real turns hang off them).
   Section 6 currently says "or to `Other`" without this guarantee.

2. **MAJOR** (section 3 `Compaction`, `Message`). Missing wend fields for compaction recovery:
   `Compaction.logical_parent_id: Option<String>` (Claude `logicalParentUuid`; wend counts cross-file boundaries with it) and a
   `Message.compact_summary: bool` (Claude `isCompactSummary`, top level). `synthetic` is not enough: qoop treats "This session
   is being continued" as not typed, wend treats the summary line specially. Also `Compaction.pre_tokens`/`post_tokens` are named
   `tokens_before`/`tokens_after` here, fine, but make them `u64`, wend stores i64 and casts.

3. **MAJOR** (section 3 `Session`, `Other`). wend's worktree-state and bridge-session records and the ai-title/custom-title split
   have no home. Minimal fix, no new types: add `Session.continues: Option<String>` (from `worktree-state.continuesSessionId`,
   the only field wend uses to link sessions), emit `worktree-state` and `bridge-session` lines as `Other { kind, data }` with the
   raw line object as `data` (document that `data` is the whole raw record for line-based products), and keep `Session.title`
   with the precedence custom > ai > first prompt. Do not add `Worktree`/`Bridge` structs.

4. **MAJOR** (section 5, qoop-history: locate by id). `discover(roots)` lists everything. qoop's `locate` is O(directories), by
   file name, with an id allowlist, symlink-safe canonicalization under the base dir, and a hard "never pick newest on miss"
   rule. Without it in agentparse, both consumers keep their own copy of ~300 lines and the id-shape security check.
   Change: add
   `pub fn locate(agent: Agent, id: &str, roots: &Roots) -> Result<SessionRef>` and `pub fn valid_id(id: &str) -> bool`, port
   qoop's `locate.rs` logic (Claude `<slug>/<id>.jsonl`, Codex newest-first day dirs, Gemini suffix of first 8 id chars, OpenCode
   one row lookup). Two corrections to carry over: Codex file names can be `...-<id>_<rollout>.jsonl` and `.jsonl.zst`, and the
   in-file check must compare `payload.id`, not `payload.session_id` (qoop checks `session_id`, which is the ROOT id for the 32
   child files per research-codex.md line 105, so qoop cannot locate Codex children today). Add `Error::BadId`.

5. **MAJOR** (section 5, qoop-history: bounded tail, `truncated`, skipped count). `parse_reader` "accepts any slice of lines"
   but the tail logic stays unowned: seek to `len - 8MiB`, drop the torn first line, silently drop an unterminated last line
   (live writer), report `truncated`. Both Claude and Codex tails need it, Gemini needs the whole-file size guard
   (`TooLarge`). Three plan details also conflict with this use:
   - D8 turns a partial last line into a Warning; qoop wants it dropped silently so healthy live files show 0 skipped.
   - `Entry.line` is "1-based source line" but a tail window starts mid-file; lines would be wrong. Define: window-relative
     for tail reads, and set `Session.truncated = true`.
   - A Codex tail has no `session_meta` (first line), so `cwd`, `models`, `cli_version` are empty; say so.
   Change: one function, no options struct:
   `pub fn parse_tail(r: &SessionRef, max_bytes: u64) -> Result<Session>` plus `Session.truncated: bool`
   (true when older content was deliberately not read; false for OpenCode and any whole read). Skipped-line count is
   `session.warnings.len()`; add a doc line saying so (qoop shows it).

6. **MAJOR** (section 4.2, 6 Claude). qoop-history reads `queue-operation` (enqueue/dequeue/remove/popAll) to surface what a
   person typed while the agent was busy, and treats `[Request interrupted by user` and compaction summaries as not typed. The
   plan maps `queue-operation` to "per research or Other". If it is `Other`, qoop must re-implement the queue state machine on
   top of entries and the point of the library is lost. Change: the Claude reader emits a `Message { role: User, synthetic: false }`
   at enqueue time with the enqueue timestamp (and none for popAll'd texts), and marks `[Request interrupted by user...]` as
   `synthetic: true`. Add `Message::text(&self) -> String` (join of `Part::Text`), which both consumers need; today every caller
   would re-join parts.

7. **MAJOR** (qoop `dir.rs` `typed_in`, 812 lines, not mentioned anywhere in the plan). It scans every session of a repo
   streaming line by line and parsing only user lines ("a gigabyte of history costs about a second"). A full-detail parse
   (Strings for every tool output, three copies of stdout, see F8) will not match that. Decide explicitly in the plan:
   either `typed_in` stays outside agentparse for now (then say "qoop-history migrates `history()` only; `dir.rs` keeps its
   scanner") or P5 measures full parse per GB against the 1 s/GB figure and adds a gate. Do NOT answer with a `ParseDepth`
   option, that is the knob the owner rejects. Also add to section 8: parse time of an 8 MiB tail window (qoop `history` is a
   per-invocation CLI call, latency is the product).

8. **MAJOR** (section 3 `ToolCall`/`ToolOutput`, D4). Big bodies are stored two or three times: `input` (raw) and `tool`
   (`Write.content`, `Patch.patch`, `Code.source`, `Edit.edits[].old/new` all copy them), and a Claude Bash result is in
   `output.text`, `output.data` (`toolUseResult.stdout`) and `shell.stdout`. For wend's full-history indexing that multiplies
   RAM on the biggest sessions. Change: pick one home per body. Proposal: the typed `Tool` holds small facets only
   (`Write { path }`, `Patch` unit, `Code { language }`, `Edit { path, replace_all }` with `ToolCall::input` as the one place the
   bodies live), and `ToolOutput.data` is `None` whenever `shell`/`files` already captured its content (keep it only for
   unmodeled results). If the owner wants typed edits kept, then drop `input` for those calls instead. Either way, section 10
   risk 3 should be a gate with a number (peak RSS on the largest local session), not a "measure later".

9. **MINOR** (section 3 serde). Verified safe, with two traps to write into the plan:
   (a) `Other { kind, data: serde_json::Value }` fails to deserialize when `data` is absent (`missing field data`), while `null`
   works; add `#[serde(default)]` on `data` since the skip rules skip empty values. (b) `Part` is `#[serde(tag = "type")]` with no
   `rename_all` in the plan text (the rename is claimed globally but `Part` and `Tool` need it listed): without it Part
   serializes as `"Text"`, `"ToolCall"` while `EntryKind` gives `"message"`; my scratch output showed the mixed casing. State
   `rename_all = "snake_case"` on every tagged enum. Also `Entry` with flatten cannot use `deny_unknown_fields`, fine, just
   never add it.

10. **MINOR** (section 5 `Source`, `SessionRef`, `parse_reader`). Duplicated identity: `Source::Sqlite { path, session_id }` repeats
    `SessionRef.id`, and `Session` repeats `agent`, `id`, `source` that `SessionRef` already holds. `parse_reader(r, source)` takes a
    `Source` only to stamp it, and a reader has no source. Change: `Source::Sqlite { path }` (id lives on the session),
    `Session.source: Option<Source>`, `parse_reader(agent: Agent, r: impl BufRead) -> Session` (source `None`; `parse_file`
    and `parse` fill it). Collapse the four public modules (`claude::`, `codex::`, `gemini::`, `opencode::`) to ONE public
    streaming entry `parse_reader(agent, r)` (Gemini G1 reads to string inside; Codex `.zst` is
    `parse_reader(Codex, BufReader::new(StreamingDecoder::new(f)?))` inside `parse_file`), plus the OpenCode functions only as
    private. `list`/`parse_db`/`parse_storage` need not be public: `discover` and `parse(&SessionRef)` already dispatch to them.
    That removes 7 public functions and one concept.

11. **MINOR** (section 5 `parse_file`). Detection "from the path and first lines" is a heuristic every real caller does not
    need (qoop and wend always know the agent). Change: `parse_file(agent: Agent, path: &Path) -> Result<Session>`; keep the
    content sniff only in `main.rs` for the CLI `show <file>`. Then `Error::NotASession` stays for "this file is not a
    transcript of that agent".

12. **MINOR** (section 5 `Error`). Variants carry bare `String`s with no `#[error]` text in the plan. qoop requires printable
    errors with no absolute paths and distinguishes NotFound(product, id), BadId, TooLarge, Store. Change:
    `Io(#[from] io::Error)`, `NotASession(&'static str)`, `NotFound { agent: Agent, id: String }`, `BadId`, `TooLarge`,
    `Sqlite(String)` (rusqlite error text, NOT the rusqlite type, see F13), all with `#[error("...")]`. Note `io::Error`
    Display does not include the path, so the "no absolute paths in errors" property holds.

13. **MINOR** (D2, section 5 OpenCode). Keep rusqlite types OUT of the public API (they are today). Taking a `&Connection` for one
    shared handle during indexing would make every rusqlite bump an agentparse semver break and defeat the range. Opening the DB
    per `parse` call is fine (sqlite open is sub-millisecond), but state the open mode in the plan since qoop depends on it:
    `SQLITE_OPEN_READ_ONLY`, busy timeout 2 s, never write. Verified result of the range test: see Experiments. Add the range
    to CI as a matrix job `cargo update -p rusqlite --precise 0.32.1` and `0.37.0` so a future rusqlite API change inside the
    range is caught. Also say plainly: a future consumer on rusqlite 0.41+ fails to resolve until the range is widened
    (links = "sqlite3" allows one copy).

14. **MINOR** (section 7, `Roots::from_env`). `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_DIR` are not backed by any research file
    (grep: only `OPENCODE_DB` is, and it may be a name or an absolute path). Verify each against upstream before shipping or drop
    them; `Roots` fields should all be directories except opencode (a file), so name that field `opencode_db`. qoop uses fixed
    `$HOME` paths today; `from_env()` returning `Roots` with pub fields serves both consumers, nothing else is needed.

15. **MINOR** (over-engineering, section 3). Deletions and merges, in order of value:
    - Delete `Entry.turn_id` (neither consumer reads it, no variant needs it; add when asked).
    - Merge `Notice` and `Error` into one `Notice { level: Level, kind, text, status: Option<u16>, retry: Option<u32> }` with
      `Level { Info, Warning, Error }`. Same fields, one variant fewer.
    - Rename the string fields called `agent` (`ParentLink.agent`, `Message.agent`, `Tool::Task.agent`) to `agent_name`: they
      collide with the `Agent` enum (the product) and mean "subagent type / mode". qoop calls the product enum `Product`; keeping
      `Agent` is fine, the clash is the field names.
    - `Session.format: String` with ten magic values: make it a `#[non_exhaustive] enum Format` or drop it. Consumers branching
      on era (wend's Codex fallback) must not string-compare.
    - `Tool::Question`, `Tool::Plan` unit variants carry no data (the input is in `ToolCall.input`): fine as discriminators,
      but fold `Skill { name }` and `Question`/`Plan` only if a consumer uses them; none does today. Keep if the owner wants the
      full set.
    - `Message.sidechain` is redundant with `Session.parent.is_some()` for subagent files; keep only because older Claude files
      mix sidechain lines in the main file (say that in the comment).

16. **MINOR** (section 1/7 CLI). The CLI was not requested. Keep `show <file|id> [--json]` (the "look at a real session" check
    the owner will demand) and `stats` (section 8 numbers). Delete `list` (stats prints it) and `json` (it is `show --json`; a
    flag here is not a config knob of the library). Fewer commands, same coverage. Gate: `src/main.rs` stays std-only as planned.

17. **MINOR** (section 9 P2). Four parallel implementers plus a shared `CARGO_TARGET_DIR` exceeds the owner's cap of about 3
    concurrent heavy jobs and will contend on the target-dir lock. Run 3 at once (claude+gemini last, they are smallest), or
    two waves. Also D2 says the timestamp parser is "40 lines"; the qoop precedent is about 120 lines with tests, plan the real
    size, and note wend already carries chrono so wend gains nothing from the hand parser either way. The hand-written RFC3339
    parser is still the right call for a library (no date crate in the dependency graph of the consumers that have none;
    qoop has none), reuse qoop's `ts.rs` verbatim including its days-from-civil and offset handling.

18. **MINOR** (D2 ruzstd). Sound. Verified the decoder works over a standard `zstd -3` stream and upstream writes exactly that.
    Keep `ruzstd = "0.9"`, only used in `parse_file` for `.jsonl.zst`. Requirements to add: a real `.jsonl.zst` fixture built
    with `zstd -3`, a test for a file truncated mid-frame (decoder error becomes a Warning plus partial parse, not a failure),
    and note `locate` must find `.jsonl.zst` (F4). Tailing a `.zst` is not possible (no seek): `parse_tail` decodes the whole
    stream and keeps the last window; document it.

## Can qoop-history be rebuilt on this API?

After F4, F5, F6, F7 (or an explicit exclusion of `dir.rs`): yes for `history()`.
`history(program, id, dirs, last, role)` becomes:
`locate(agent, id, &roots)?` then `parse_tail(&r, 8 << 20)?` then
`session.messages().filter(|m| typed(m, role)).rev().take(last)` with `Message::text()`. What qoop keeps: `detect_product`
(qoop-detect), the `BaseDirs`-to-`Roots` mapping, the Window/Error rendering, ~10 lines of last-N. What it loses today without
the changes above: Codex child lookup (F4), live-file silence (F5), queued-typing messages (F6). Features covered as is:
OpenCode lookup by id, Gemini suffix lookup, role filter, skipped counts (`warnings.len()`).

## Can wend-core's parser be replaced?

Claude: yes after F1, F2, F3 (graph ids per line, logical parent, compact summary flag, continues link, worktree/bridge via
`Other`, title precedence). Per-line records map to `Entry.lines`; wend's `fts_text` is `Message::text()` plus tool outputs it
chooses. Codex: yes, the plan's event/response dedupe replaces wend's two-pass fallback and fixes the child `session_id`
collision (research-codex.md line 489). OpenCode: yes; wend reads all sessions in one pass, `discover` + `parse` per session
reopens the DB each time, acceptable at sqlite open cost (verify in the corpus run, one number: ms per session open).
wend can then drop `tracing` warn calls in the parser and read `warnings` instead. wend keeps chrono for its own code.

VERDICT: changes required
