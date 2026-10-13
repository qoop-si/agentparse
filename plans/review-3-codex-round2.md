Prior blocker/major findings still unresolved: **C-2, C-3 (unknown counts), C-5, C-6, C-7, C-9–13, C-15–16, C-23–24, C-26, C-30–32, C-34–35**. The remaining findings are addressed or have acceptable alternatives, including retaining terminal metadata and recipients in `data`/`extra`. Gemini’s revised total formula is arithmetically correct.

1. **Blocker | `plans/agentparse.md:415–419` | C-7:** “Last version” per message still destroys earlier content/result revisions; recovering only masked results is insufficient. **Change:** retain ordered revisions and mutation events with provenance; distinguish masking, compaction, and rollback, and specify how active ordering is derived.

2. **Blocker | `src/model.rs:291–302`, `plans/agentparse.md:386–389` | C-9–11:** A single `output` cannot represent ordered `notify` deliveries and continuation results. Interval-based `children` also asserts parentage the source does not establish. **Change:** add deliveries with timestamps/source references, cell continuation links, and independently retained execution events with explicitly inferred parent links. Track visible-output and persisted-output truncation separately.

3. **Major | `src/model.rs:537–570`, `plans/agentparse.md:227–241` | C-3/5/6:** Unknown token measurements remain zero; saturation silently hides overflow. Session totals lack completeness, so a tail’s partial sum resembles a complete total. **Change:** represent unknown measurements and completeness explicitly, retain reported totals separately, and warn on subtraction inconsistencies and addition overflow.

4. **Major | `src/model.rs:714–736` | C-5:** `finish()` overwrites any recorded session cost with summed message/compaction costs, violating the latest Claude `cost-state` rule. **Change:** preserve the recorded session cost and apply it after summing tokens and fallback costs.

5. **Major | `plans/agentparse.md:249,382–383` | C-2:** “One message per response” conflicts with splitting that response by phase/kind. Attribution after splitting is unspecified and can duplicate usage. **Change:** retain a shared response ID and attach usage exactly once, or use an explicit response grouping.

6. **Major | `plans/agentparse.md:271–275,407` | C-15/16:** Later assistant messages or turn completion still falsely prove interruption of unfinished/background processes. **Change:** require explicit interruption/termination evidence; otherwise preserve running/unknown outcome. Keep reported tool status distinct from process outcome.

7. **Major | `plans/agentparse.md:307,390–391`, `src/model.rs:429,496` | C-12/13:** Joining arbitrary argv invents shell source, and choosing whichever clock matches one experiment conflates different durations. **Change:** make shell source optional for non-shell argv; retain execution duration and tool/cell wall time separately, preserving submillisecond precision.

8. **Major | `plans/agentparse.md:360–361,374–376,400` | C-23/24/32:** UUID aliases do not preserve all merged-record timestamps/edges; child IDs remain ambiguous across Claude roots; `history_base` is merely retained, leaving referenced transcript history unread. **Change:** define per-record provenance, namespaced child identity/root links, and history resolution or an explicit unresolved/incomplete result.

9. **Major | `plans/agentparse.md:294` | C-26:** Gemini MCP splitting at the first underscore remains ambiguous when server names contain underscores. **Change:** resolve through explicit metadata/known prefixes, otherwise retain `Other` or an unresolved namespace.

10. **Major | `src/lib.rs:68–70` | New:** Tail parsing always discards a line, even when the window starts exactly at a complete record boundary. **Change:** inspect the preceding byte and discard only a genuinely partial first line.

11. **Major | `src/lib.rs:126–144` | New:** A malformed first nonblank line prevents detection of an otherwise valid session, contradicting malformed-line recovery. **Change:** scan for a recognizable valid record, preserving preceding malformed lines for parser warnings; avoid treating any arbitrary string `type` as sufficient Claude identification.

12. **Major | `src/util.rs:196–225` | New:** Blob stripping misses Codex image-generation base64 in `result`, while indiscriminately deleting ordinary strings named `signature` or `data` beside `mimeType`. **Change:** use source-shape-aware blob detection, including image-generation payloads, and preserve ordinary application fields.

13. **Major | `plans/agentparse.md:43,435–446` | C-30/34/35:** D8 still excludes database query/schema failures from fatal errors. Verification omits explicit guarantees for several previously requested cases, including revisions, repeated deliveries, late processes, WAL consistency, and timestamp units. **Change:** make database operation failures errors and enumerate those semantic checks as acceptance criteria.

14. **Minor | `src/patch.rs:41–45` | New:** Parsing continues after `*** End Patch`, attaching trailing prose to the last file’s diff. **Change:** finish the current file and stop at the end marker.

No additional serde omission/round-trip defect found. Review was read-only; tests were not run.

VERDICT: changes required.
