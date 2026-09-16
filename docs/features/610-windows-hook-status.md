# 610 — Hook-based session status on Windows, plus the 347 follow-ups

> Tracking issue: [#610](https://github.com/yicheng47/runner/issues/610)
> Priority: P1. Platforms: Windows for phase 1; macOS and Windows for the rest.
> Continues [347](./archive/347-hook-based-session-status.md), which shipped the hook-based status model on macOS for Claude Code, Codex and GitHub Copilot CLI (PRs #588, #589, #609) and closed on 2026-09-16. The program record, slice plan and log stay in [`docs/impls/347-hook-status/`](../impls/347-hook-status/README.md).

## Motivation

On Windows every runtime still runs on the estimated baseline: byte activity and the title spinner. Working and Idle are guesses, and Approval needed and Answer needed never appear, so a crew delivery can land on a CLI prompt that is waiting on a dialog. The CLIs support hooks on Windows; the gap is Runner's own gate, `hooks_supported(cfg!(windows))`, and the unverified pieces behind it: command quoting for the hook commands, Windows paths in the feed and plugin files, payload append and cleanup, and each CLI's native hook execution under ConPTY.

## Scope

### Phase 1 — Windows hook status

Enable the Claude Code, Codex and Copilot status bridges on Windows, one runtime at a time, each gated on a JASONPC smoke:

- Verify each CLI executes its hook command there: Claude Code through `--settings`, Codex through `-c hooks.<Event>`, Copilot through `--plugin-dir`.
- Port the reporter to something Windows runs natively; Copilot's is a `sh` script today.
- Verify feed paths, payload append and cleanup, and the SessionStart rekey hook path that Windows already runs; reuse it where it fits.
- Lift the gate per runtime as it passes. Same status vocabulary and UI, no new surfaces; the estimated baseline stays wherever hooks are not validated.

### Phase 2 — deferred details

Slice 5 of the program record: `Working · Compacting context` and `Using tools`; sidebar attention for response failures; the interruption outcome in the Idle tooltip; elapsed time on a wait; the nonblocking-ask `Still working` case. Each is additive to the shipped layout.

### Phase 3 — drop the title heuristic

Remove title-spinner classification only where a runtime and platform have validated hook coverage; keep the baseline everywhere else.

### Known unsupported, out of scope until observable

Codex human waits (no safe surfaced-wait boundary in 0.154 hooks or rollout); Copilot `ErrorOccurred` until a real fatal model-call payload is captured; Claude MCP URL and browser flows.

## Verification

- On JASONPC, per runtime: a prompt shows Working without the estimated tooltip and Idle on completion; an approval dialog shows Approval needed while visible; a question shows Answer needed; the feed file is created under app data and removed on session end; a session with hooks disabled falls back to estimated status.
- The macOS behaviour and every existing adapter test are unchanged.
- While on JASONPC, the [606](./archive/606-rail-glyph-liveness.md) rail check: in both themes a project with a running session and one without, a shell tab, a mission row and a provider chat; live generic glyphs full-strength text, stopped ones dimmed, provider marks keep their hue; collapse and expand a project for the closed and open folder; drag a row and confirm the accent drop indicator.
- `docs/tests/` gains a Windows smoke record per runtime in the shape of the Codex and Copilot ones.
