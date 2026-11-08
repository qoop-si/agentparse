# Changelog

## 0.1.0 (2026-11-08)


### ⚠ BREAKING CHANGES

* Tool::Task has a new `name` field; code that builds the variant or matches it without `..` must add it.

### Features

* name the agent a task call spawns or messages ([dce1313](https://github.com/qoop-si/agentparse/commit/dce1313c16a6569ddbd6f4b221a240ad014a9202))
* parse Claude Code, Codex, OpenCode and Gemini sessions into one model ([1351d05](https://github.com/qoop-si/agentparse/commit/1351d0599d7429e7af7d9ebac61b18ca3a5cd068))
* read call outcomes, questions and session state from Claude text ([76670fb](https://github.com/qoop-si/agentparse/commit/76670fbae830aa137f0c757fde97ee460908b0dc))


### Bug Fixes

* **claude:** read a teammate's message handed over as a prompt as an agent's ([fd779f3](https://github.com/qoop-si/agentparse/commit/fd779f3ca632270bb0188870482bd1241248cc2b))
* **codex:** read the commands and exit codes of code mode cells ([3c5be8e](https://github.com/qoop-si/agentparse/commit/3c5be8e8a35c09b2c55c3ff4d9934930abbe1c9b))
* **codex:** treat hook additional context as harness context ([ee233f1](https://github.com/qoop-si/agentparse/commit/ee233f172dfd8303d1248e10cee0679313873514))
* keep background work running and surface the records the UIs show ([6f19199](https://github.com/qoop-si/agentparse/commit/6f19199cc8ba26637400b3276ea1f7a7b6cd10cc))
* **opencode:** mark created files from a v2 write as added ([faf0e71](https://github.com/qoop-si/agentparse/commit/faf0e71e2269993fb9a966371fd165cdfdf95823))
