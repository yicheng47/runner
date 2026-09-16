# Hook-based agent status — slice plan

Slices for [#347](https://github.com/yicheng47/runner/issues/347) ([spec](../../../features/archive/347-hook-based-session-status.md), [README](README.md), [log](impl_log.md)). Each lands green on its own. The spec's own "Implementation phases" section is the narrative version; this is the unit of work.

| Slice | Scope | State |
| --- | --- | --- |
| 0 | Design and capability audit: ten canvas frames, per-runtime matrix verified against installed binaries | Landed 2026-09-14, `1218288` |
| 1 | Claude Code hook *source* behind today's Busy/Idle — injection, status file, notify watcher, `hook` source and latch, interrupt recovery | Landed 2026-09-14, `9584330` |
| 2 | The status vocabulary and its UI | Merged via [PR #588](https://github.com/yicheng47/runner/pull/588), `36dc888`; Jason passed smoke. |
| 3 | Codex adapter and Claude follow-ups | Merged via [PR #589](https://github.com/yicheng47/runner/pull/589), `3dd37b0`; review, both-platform CI, and all reported smoke tests passed. Codex human-wait holds remain unsupported. |
| 4 | TRAE CLI adapter | Not planned — Runner hook integration unsupported; no enterprise account for validation. |
| 5 | Deferred details, one at a time | Not started |
| 6 | Remove title-spinner classification | Only for runtime/platform combinations with validated hook coverage; preserve unsupported-platform fallback. |
| 7 | GitHub Copilot CLI adapter | Implemented on `feat/540-copilot-runtime` in [540 mission 2](../gpui-rewrite/briefs/540-m2-copilot-hook-status.md); working-tree review clean in Runner message `01M2MASYZ4ZSBEXGAC66Q5H3YB`, Jason's manual smoke passed 2026-09-16. |

## Slice 2 — vocabulary and UI

The slice that makes the feature visible, including `Answer needed`. Claude Code 2.1.270 supplies named question/plan tool events and delayed surfaced-dialog notifications. Both internal passes were implemented in mission `01M2EX7VV58BNFWBN7G1JQVPWC`: normalized backend state and pane headers, then sidebar rollups and mission surfaces. Inline follow-ups reconcile cancelled questions from structured transcript records and retain finished outcomes across late tool results. The final presentation uses Idle for both confirmed and estimated inactivity, adds Response failed, and follows option C for single-pane tabs plus the centered split-pane design in `X7FJf`. Per-pane unread/error acknowledgement persists in migration 0021. Jason confirmed the smoke test passed; final inline review, local checks and CI passed, and PR #588 is merged. The interruption tooltip remains slice 5. [Mission brief](../gpui-rewrite/briefs/347-slice-2-status-ui.md).

## Slice 3 — Codex

Per-invocation `--enable hooks --dangerously-bypass-hook-trust` and additive `-c hooks.<Event>` definitions; no persistent hook/trust/config/home changes. Explicit invocation hook configuration/opt-out selects baseline-only. Reuse Claude's feed transport and normalized runtime/status path on macOS; Windows remains baseline-only.

Implemented work, completion/continuation, generation/session/turn ownership, child isolation, native interruption plus correlated turn_aborted readiness, teardown and bridge-loss fallback. Stop is display-only; late results cannot overwrite interruption/completion and old turn submissions cannot rebind ownership. Independent draft protection remains unchanged.

The earlier “no question event” and “hold-and-cancel interval” claims were wrong. Native 0.154.0 emits PreToolUse(request_user_input) even when rejected, and a valid Plan call plus rollout context can precede a user hook denial without a visible dialog. PermissionRequest likewise precedes automatic approval. No safe surfaced-wait boundary was found within the supported hook/rollout path, so this slice has no Approval needed/Answer needed or Codex human-wait delivery protection. The opt-in Default-mode question can be nonblocking, with that distinction absent from its hook payload. [Evidence and focused smoke](../../../tests/347-codex-hooks-smoke.md).

## Slice 4 — TRAE CLI (not planned)

Runner hook integration is unsupported. Jason no longer has an enterprise account to test TRAE CLI and removed this adapter from the plan on 2026-09-14 after PR #589 merged. Keep estimated byte-activity/title status and existing process lifecycle behavior. The upstream manual audit is retained as historical evidence only; it does not promise future implementation.

## Slice 5 — deferred details

`Working · Compacting context` and `Using tools`; sidebar attention for response failures (the Claude `StopFailure` label was pulled into slice 2 on 2026-09-14); the interruption outcome in the Idle tooltip; elapsed time on a wait; the nonblocking-ask `Still working` case. Each is additive to the slice-2 layout.

## Slice 6 — remove the title heuristic

Remove title-spinner classification only where the runtime and platform have validated hook coverage. Windows and unsupported runtimes still use the existing title and byte-activity baseline; do not remove their fallback merely because the macOS adapters shipped. Byte activity remains the permanent baseline.

## Slice 7 — GitHub Copilot CLI

Runner regenerates `<app data>/copilot-hooks` at startup and mounts it additively on each macOS Copilot invocation. The static reporter inherits a per-session feed path and generation, reuses the shared bounded side-file transport, and leaves user configuration untouched. A user `disableAllHooks` setting emits no reports and therefore never latches the baseline off. Windows remains baseline-only.

Copilot 1.0.83 maps prompt/tool/compaction events to Working and `Stop(end_turn)` to Idle, corrected by later work after a continuation. PermissionRequest alone never waits; a surfaced `Notification(permission_prompt)` raises Approval needed. `PreToolUse(AskUserQuestion)` raises Answer needed immediately and `Notification(elicitation_dialog)` confirms it. Tool results or correlated transcript completion clear ownership, including Escape/Ctrl+C cancellation through Runner's input signal. Subagent events do not affect the parent. `ErrorOccurred` stays unmapped until a real fatal model-call payload is captured. [Evidence and focused smoke](../../../tests/540-copilot-hooks-smoke.md).

## Unscheduled

**Windows status-hook integration — agreed follow-up (Jason, 2026-09-14).** Enable Claude/Codex/Copilot status injection and observation after validating their Windows hook execution, paths, payload writes and cleanup. This is a Runner integration gap; the CLIs support hooks, and Claude fullscreen/theme settings plus the existing SessionStart rekey hook are already configured on Windows. Reuse that execution path where suitable instead of assuming a separate helper is mandatory. Shared status/UI remains available through the baseline until native verification passes. See [README](README.md#open).
