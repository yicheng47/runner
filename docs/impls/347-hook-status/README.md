# Hook-based agent status — program record

Implementation program for [feature 347 — hook-based agent session status](../../features/347-hook-based-session-status.md) ([#347](https://github.com/yicheng47/runner/issues/347)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [local-skills](../local-skills/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) is the slice plan, [impl_log.md](impl_log.md) is the dated log. Mission briefs go where every brief lives, [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/).

## Status (2026-09-14)

Design and the verified capability audit landed on `main` in `1218288`. Slice 1 — hook-driven status for Claude Code behind today's Busy/Idle vocabulary — landed in `9584330`. Next is slice 2, the status vocabulary and its UI, which is gated on Jason's review of the canvas frames rather than on any remaining unknown.

Capabilities are verified against installed binaries, not documentation: Claude Code 2.1.270, Codex 0.154.0, TRAE CLI 0.120.52, all checked 2026-09-14. The per-runtime matrix is in the spec.

## Why a program, not one mission

The transport is small, but the feature crosses every layer and every runtime once: a per-spawn hook injection per CLI (three different config surfaces), a watcher and a new status source with its own precedence rules (backend), a status vocabulary that replaces a four-state dot with a nine-state model (backend + app), every consumer of that dot (pane header, tab bar, sidebar rollups, mission cards, mission tab strip), and a design pass first. Slicing it means each piece lands green on its own, and — the point of the layering below — a runtime can be useful before its adapter exists.

## Decisions that bind

1. **Minimal-first.** Ship only what each runtime can prove; an unprovable detail is left out, never approximated. Rich coverage differs by runtime and that is the design, not a gap.
2. **Runner never gains an approval or answer control.** Showing that a session is waiting on you, and focusing that pane when you click it, is the whole job. The CLI keeps its prompt.
3. **The mechanism is a port, not a design.** Per-invocation CLI injection, fire-and-forget hook bodies, a script file in a Runner-owned directory, TOML multi-line literals — all from cmux (manaflow-ai/cmux, GPL-3.0-or-later, compatible with Runner's GPL-3.0). cmux's *product* decisions are explicitly not ported.
4. **Terminal observation is a permanent baseline; a hook adapter is a per-runtime upgrade.** A new runtime is supported the day it is added, with no adapter. Adapter work is never a precondition for shipping a runtime, and a failed bridge falls back rather than going dark.
5. **Status gates delivery only for needs-you.** Delivery injects a nudge, not a message body, so a nudge typed into a working agent is queued by its TUI and read next turn. Working and Ready gate nothing. Only an open approval or question dialog holds a delivery, because it consumes keystrokes.
6. **No runtime publishes a continuation-proof turn boundary.** Idle is entered on the turn-end event and corrected by the next work event. This is safe *only* because Ready gates nothing; if that ever changes, this decision must be revisited.
7. **`Notification(idle_prompt)` is not a turn boundary on Claude Code.** 2.1.270 gates it behind `messageIdleNotifThresholdMs` (60 s default, disabled at 0, suppressed during a dialog). It is secondary confirmation only. TRAE documents an immediate one; that claim is unverified against its binary and must be checked before its adapter is built.
8. **Claude Code publishes no interrupt event** and its `Stop` excludes user interruptions, so interrupts are recovered through Runner's own input layer (`input-interrupt`, `input-escape`), not inferred from silence.

## Open

- **Windows.** Slice 1's hook command is POSIX shell with unix-only tests. It fails safe — no records, the latch never arms, the baseline keeps owning — but that is currently accidental rather than stated. Either make it shell-agnostic (Claude Code 2.1.270 supports an exec form that would also drop a shell per hook) or declare Windows baseline-only and test that.
- **Interrupt rendering after slice 2.** Slice 1 suppresses completion on a provisional Escape Idle because it has no way to say "interrupted". cmux solves the same problem by *having* an interrupted badge state (`CLI/CMUXCLI+AmpExtension.swift:432`). Once slice 2 ships the real vocabulary, prefer rendering the interrupt honestly over suppressing the completion.
