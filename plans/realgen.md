# Real session transcripts (generated 2026-10-13)

Scratch project (git init, a.txt with 3 lines, test.sh = stdout "out-line", stderr "err-line", exit 3):
<scratchpad>/realgen/demo
CLI stdout/stderr captures sit one level up (../*.out, ../*.err).

Base prompt: Run `ls -la`, then run `sh test.sh` and tell me its exit code, then read a.txt and change its second line to 'two edited'. Reply in one short line.
Strict prompt (P2): same, but "Run exactly `sh test.sh` as the whole command (no echo, no wrapper, no redirect, no semicolon)". Needed because models like to append `; echo $?`, which hides the real exit code (the shell then exits 0).

## Gemini: FAILED (no auth), no session produced
- Attempt 1: `gemini -p "<base prompt>" --yolo` (run in demo dir), rc=41
  - stderr: `Approval mode overridden to "default" because the current folder is not trusted.` and `Please set an Auth method in your ~/.gemini/settings.json or specify one of the following environment variables before running: GEMINI_API_KEY, GOOGLE_GENAI_USE_VERTEXAI, GOOGLE_GENAI_USE_GCA`
- Attempt 2: `gemini -p "say hi" --yolo --skip-trust`: same auth error (trust warning gone).
- Cause: ~/.gemini/settings.json has only hooks, no auth type, no oauth_creds.json, no GEMINI_* env vars. Needs the user to log in (`gemini` interactively) or export GEMINI_API_KEY. Existing older sessions only under ~/.gemini/tmp/*/chats/*.jsonl (not from this run).

## OpenCode v2.0.27: OK (two sessions)
- Run A: `opencode run -m opencode/big-pickle --auto --format json "<base prompt>"`, rc=0
  - DB: ~/.local/share/opencode/opencode.db, session id `ses_ecd1af9ecffejfXDdbhV6U1ziB`, table session_v2 / session_message (title "Check test.sh exit code and edit a.txt")
  - Model wrapped the command as `sh test.sh; echo "EXIT_CODE=$?"`, so metadata.exit is 0 here.
- Run B (strict prompt + background shell step): `opencode run -m opencode/big-pickle --auto --format json "<P2> Also, as a separate step, start \`sleep 2; echo bg-done\` as a background shell job if you have a background option, then check its output."`, rc=0
  - DB: same file, session id `ses_ecd18fd61ffe5pYW5e5JMSVpcg` (title "Run commands, edit a.txt, check bg job")
- The default model was not tried; `opencode/big-pickle` worked first time (no 410).
- Structure (session_message rows, ordered by seq, columns: id, session_id, type, seq, time_created, time_updated, data JSON):
  - Run A: user(4), assistant(5, 22, 31), idle(36). Assistant data = {time, agent, model{id,providerID,variant}, content[], snapshot, finish, rawFinish, cost, tokens}. content[] items: type reasoning | tool | text. Many tool calls are batched in one assistant row (row 5 holds reasoning + shell ls + shell test.sh + more).
  - Run B: user(4), assistant(5,16,25,34,41,49), synthetic(56), assistant(57), idle(62).
  - Tool item: {type:"tool", id:"call_...", name, executed:false, state{status, input, content[{type:"text",text}], metadata}, time{created,ran,completed}}.
  - Tool names seen: shell, read, edit.
  - ls -la: name shell, input {command}, metadata.exit 0.
  - sh test.sh (Run B, real exit 3): name shell, input {command:"sh test.sh"}, state.status "completed" (not an error status), state.metadata.exit = 3, state.content = two text parts: "out-line\nerr-line\n" (stdout and stderr merged in one text part, no stderr field) and "Exited with code 3".
  - Run A test.sh: content text "out-line\nEXIT_CODE=3\nerr-line\n" (stdout before stderr, exit visible only via the echo text), metadata.exit 0.
  - read: name read, input {path}, content text "Read file a.txt, lines 1-3\n1: one..." .
  - edit: name edit, input {path, oldString, newString}, metadata.files[{file, patch (unified diff), status, additions, deletions}].
  - Background shell (Run B): name shell, input {background:true, command}, state.status "completed" but metadata.status "running" and metadata.shellID "sh_132e735ee001UNBjuWj6mHlXQv"; content text "Command moved to the background (shell ID: ...)". Later the finish arrives as a separate row type `synthetic` (seq 56) with metadata {source:"shell", shellID, jobID, state:"completed", exit:0, truncated} and text `<shell id="..." state="complet...` containing bg-done. No tool call carries the final output, correlate by shellID.
  - Final answer: assistant row with content [{type:"text"}], finish "stop". idle row data {time, outcome:"succeeded"}.

## Codex (codex-cli 0.162.1): OK
- First try failed: `codex exec --full-auto "<prompt>"`, rc=2, `error: unexpected argument '--full-auto' found` (flag removed in this version).
- Working command: `codex exec -s workspace-write "<base prompt>"`, rc=0
- Session file: ~/.codex/sessions/2026/10/13/rollout-2026-10-13T04-34-15-01a132e5-5f39-7ad3-a316-d71d10e7c68b.jsonl
- Record order (top-level `type` / `payload.type`): session_meta; event_msg task_started; response_item message x4 (developer/user context); world_state; turn_context; response_item message; event_msg item_completed x2 (UserMessage, AgentMessage); response_item message; response_item custom_tool_call (name "exec"); token_usage_record; event_msg item_completed x3 (CommandExecution x3); response_item custom_tool_call_output; event_msg token_count; response_item custom_tool_call (name "exec", this one runs tools.apply_patch); token_usage_record; item_completed (FileChange); custom_tool_call_output; token_count; item_completed (AgentMessage); response_item message (final); token_usage_record; token_count; event_msg task_complete.
- Tools here are JS snippets: custom_tool_call.input is JS text like `text(await tools.exec_command({cmd:"ls -la",...})); ... tools.exec_command({cmd:"sh test.sh"...}); ... cat a.txt`. The first custom_tool_call batches three shell commands (ls -la, sh test.sh, cat a.txt); the second is `tools.apply_patch("*** Begin Patch ...")`. There is no separate read tool, the model used `cat`.
- Exit code 3 and stderr locations for `sh test.sh`:
  - event_msg item_completed, payload.item.type "CommandExecution": fields id, process_id, command ["/bin/zsh","-lc","sh test.sh"], cwd (file:// URL), parsed_cmd [{type:"unknown",cmd}], source "unified_exec_startup", status "failed", exit_code 3, duration{secs,nanos}, and output text "err-line\nout-line\n" (stderr and stdout merged, stderr first here, so ordering is not reliable).
  - The matching response_item custom_tool_call_output has payload.output = a JSON-encoded string of [{type:"input_text",text:"Script completed\nWall time ... Output:\n"},{type:"input_text",text:"<JSON string with chunk_id, wall_time_seconds, exit_code, original_token_count, output>"}], i.e. exit_code is double-encoded (JSON inside text inside JSON). For ls -la exit_code is 0 there; the three commands' outputs are in the same single output array.
  - Other CommandExecution items: ls -la (exit 0, parsed_cmd type list_files), cat a.txt (exit 0, parsed_cmd type read with name/path).
- Edit shows up as event_msg item_completed with item.type "FileChange" and as the apply_patch custom_tool_call; its output is "{}".

## Claude Code: OK (two sessions)
- Run A: `claude -p "<base prompt>" --permission-mode acceptEdits --allowedTools "Bash Read Edit" --model haiku`, rc=0
  - Session file: ~/.claude/projects/<escaped scratch demo dir>/28d25c76-eac5-4b51-be33-5a338262e608.jsonl
  - Model ran `sh test.sh; echo "exit code: $?"`, so the tool result is a SUCCESS (is_error false) with text "out-line\nerr-line\nexit code: 3".
- Run B (strict prompt): `claude -p "<P2>" --permission-mode acceptEdits --allowedTools "Bash Read Edit" --model haiku`, rc=0
  - Session file: ~/.claude/projects/<escaped scratch demo dir>/1569f2cc-c236-45ae-b3fe-7d24d4a547da.jsonl
- Record types in order (Run B, collapsed): queue-operation x2, attachment x3, user (prompt), attachment x9, atis-latch, attachment x5, last-prompt, atis-latch, assistant (thinking), assistant (tool_use Bash ls), assistant (tool_use Bash sh test.sh), assistant (tool_use Read)... then user tool_result records, last-prompt, atis-latch, assistant (tool_use Edit), user tool_result, attachment, assistant (text), system, last-prompt, cost-state. Each content block is its own `assistant` line (thinking, each tool_use, text), tool_results are `user` lines with message.content[{type:"tool_result",tool_use_id,content,is_error}].
- Tool names: Bash, Read, Edit.
- Exit code 3 and stderr locations (Run B, command exactly `sh test.sh`):
  - user line, message.content[0] = {type:"tool_result", is_error:true, tool_use_id:"toolu_01VeYcVVvHv1DxwTPyb1VhEA", content:"Exit code 3\nout-line\nerr-line"}.
  - The same line's top-level `toolUseResult` is a plain STRING "Error: Exit code 3\nout-line\nerr-line" (not an object) in the failure case.
  - stdout and stderr are merged in order, no separate stderr field in the error case.
- Success case (ls -la, or Run A test.sh with the echo wrapper): toolUseResult is an OBJECT {stdout, stderr:"", interrupted, isImage, noOutputExpected}; stderr is "" even though err-line was printed (the merged text is in stdout in Run A: "out-line\nerr-line\nexit code: 3").
- Read toolUseResult: {type, file{...}}; Edit toolUseResult: {filePath, oldString, newString, originalFile, structuredPatch, replaceAll, userModified}.

## Gaps
- Gemini has no fresh session (auth). Existing real Gemini files: ~/.gemini/tmp/<slug>/chats/session-*.jsonl (older ones, e.g. ~/.gemini/tmp/qass-1/chats/session-2026-10-10T21-18-3c69a60b.jsonl).
- Claude background shell (run_in_background) was not generated.
