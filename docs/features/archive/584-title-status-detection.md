# 584 — Title-spinner status heuristic

> Tracking issue: [#584](https://github.com/yicheng47/runner/issues/584)
> Implemented in [#585](https://github.com/yicheng47/runner/pull/585), included in v0.8.9. Fixes the captured regression in [#583](https://github.com/yicheng47/runner/issues/583).
> Decision, 2026-09-13: ship the merged heuristic unchanged in v0.8.9, then replace agent status detection with [lifecycle hooks (#347)](../347-hook-based-session-status.md).

## Problem and implemented behavior

The byte detector reports Busy on output and Idle after two seconds of silence. Codex 0.154.0 can keep animating its composer after completing a turn, so output keeps a ready session Busy and suppresses the idle-gated inbox reconciliation nudge. A title-spinner heuristic improves this captured case without changing the status vocabulary or UI.

`TitleStatus` in `crates/runner-terminal/src/terminal.rs` arms on the first nonempty title starting with a braille character (`U+2800`–`U+28FF`). That classifies Busy; a subsequent nonempty title without that prefix classifies Idle. Consecutive identical classifications are deduplicated. Blank titles and title resets do not produce an Idle transition or disarm status detection. Sessions that never show a matching prefix stay on byte-derived status.

Once armed, title reports use `source: "title"` and suppress `source: "forwarder"` byte transitions. Existing input-submit and explicit agent reports remain separate status writers. [Architecture §5.10](../../arch/arch.md#510-busy--idle-inference) documents the actual precedence. `InputTracker` describes input/composer state and influences local-input suppression; it is not a third lifecycle detector.

## Evidence and limits

The retained `codex-title-working.ndjson` fixture has a braille working interval from 0.177 s to 7.428 s, followed by 46 output events over 3.465 s with no gap longer than 153 ms. The title heuristic goes Idle while the byte detector would remain Busy. The `claude-session.ndjson` fixture brackets work with braille titles from 5.038 s to 7.891 s; its `✳` titles are resting frames and must not be classified as Busy.

These are observations of specific CLI behavior, not a stable protocol. Custom title settings, disabled title output, changed glyphs, approval prompts, or an unexpected update sequence can make the result wrong or stale. The implementation is runtime-agnostic and cannot prove that a title came from an agent declaring readiness. It does not establish a durable guarantee for inbox delivery.

No shell detector was implemented in #585. A quiet `sleep 4` can still read Idle before finishing. Shell titles such as a command name or a new working directory cannot reliably distinguish prompt from work; a foreground process query describes command lifetime, not an agent's internal turn.

## Follow-up direction

- [#347 — Hook-based agent status](../347-hook-based-session-status.md): use Claude Code and Codex lifecycle events; remove spinner classification when that replacement lands. Do not let output silence override a healthy hook-driven turn.
- [#586 — Shell status](../586-shell-status-detection.md): try process detection first, with explicit Unix/Windows limits; consider Ghostty-style semantic shell integration later.
- [#587 — Terminal-provided titles](./587-terminal-provided-titles.md): display child-supplied text and animation independently of activity, routing, and persistent names.

The earlier draft treated title text as an authoritative declaration, promised exact shell detection, and described foreground detection as dropped in favor of immediate shell injection. Those claims are superseded by the decisions above. This archive records the shipped scope, not an implementation plan for the hook replacement.
