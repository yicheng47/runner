# 586 — Shell status: process detection first

> Tracking issue: [#586](https://github.com/yicheng47/runner/issues/586)
> Priority: P2. Platforms: macOS and Windows, with different process APIs.
> Decision, 2026-09-13: try process detection first; keep Ghostty-style semantic shell integration as a later improvement. No shell detector changes in v0.8.9.

## Motivation

A shell running `sleep 4` can become Idle after two seconds because Runner currently infers activity from byte arrival. Foreground process detection improves the ordinary shell → command → shell transition without injecting shell startup scripts. Try that smaller change and evaluate it in live use before implementing semantic prompt integration.

This applies to Runner's shell tabs, panes, and drawer shells. It does not classify agent turns or fix agent inbox routing: Codex and Claude Code remain foreground processes while working and while awaiting input. Their lifecycle belongs to [#347](./archive/347-hook-based-session-status.md).

## First step: process detection

- On Unix, compare the PTY's foreground process group with the shell's process group. A different foreground group indicates a foreground job; returning ownership to the shell indicates that job has finished or been suspended. Sample independently of PTY output so silent commands and empty prompts still produce both transitions.
- Do not merely veto the byte detector's one Busy→Idle event. That can leave its internal state and the published status out of sync with no output to trigger recovery. Keep process-derived status coherent across polling, exit, and session replacement.
- State the limits: foreground ownership does not cover every builtin, shells with job control disabled, or work happening inside a foreground program. Background jobs should not keep an otherwise ready shell Busy. A foreground `codex`, `ssh`, or `vim` remains a foreground command even while it is awaiting input.
- Windows has no `tcgetpgrp` equivalent. Runner's existing Job Object query (`ActiveProcesses > 1`) measures other live processes and can include background helpers; it is not the Unix foreground check. Evaluate a Windows shell-child detector separately, document its weaker guarantee, and test persistent/background children before choosing its behavior.
- Restrict the process detector to shell sessions. Preserve process-exit handling and avoid presenting unavailable observations as confirmed Idle.

## Later: semantic shell integration

Ghostty uses shell startup integration and `preexec`/`precmd` equivalents to emit OSC 133 prompt and command markers. This can provide command boundaries, exit status, timing, and prompt navigation that process inspection cannot provide. It remains a separate phase, justified by those benefits after the process detector has been tried.

Decoding markers and making shells emit them are separate tasks. Runner's current `vte` 0.15 parser logs/discards unknown OSC sequences internally; it does not forward raw unknown OSC parameters to Runner's event handler. Supporting OSC 133 needs an explicit parser extension or observer, plus boundary semantics and tests. Decoding alone benefits only shells already emitting markers.

Bundled injection must preserve each shell's startup chain and user configuration. Ghostty is a macOS/Linux reference, not a native Windows implementation; PowerShell and other Windows shells need their own integration and validation. Remote shells and multiplexers require markers to survive their own environment. Shell markers describe the outer command and cannot report an agent's internal turn status.

## Verification

- An idle prompt remains Idle; `sleep 4` stays Busy for its full foreground lifetime and returns to Idle without needing any output or a nonempty prompt.
- Test a pipeline, foreground/background transitions, suspended jobs, a background helper, shell exit, and session replacement. Document builtin and job-control limits.
- Test macOS and native Windows separately; do not claim cross-platform equivalence from a shared method name.
- Launch Codex from a shell and verify the detector only describes the foreground command's lifetime, not thinking/ready/approval states inside Codex.
- If the semantic phase is implemented, test split OSC sequences, exit codes, nested commands, startup chaining, and shells that emit no markers.

## References

- Runner: `crates/runner-backend/src/session/process/{unix,windows}.rs`, `session/pty_runtime.rs`, `session/manager/`, and `ops/session.rs`.
- Ghostty: `src/shell-integration/` and `src/termio/shell_integration.zig` in `~/repos/gui/ghostty`; Zellij: foreground process discovery in `~/repos/gui/zellij`. Zellij's process discovery is not an agent Busy/Idle protocol.
- [#584](./archive/584-title-status-detection.md) records the shipping title heuristic; [#587](./archive/587-terminal-provided-titles.md) displays titles without interpreting them as activity.
