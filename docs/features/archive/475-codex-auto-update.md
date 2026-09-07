# Suppress agent update prompts, surveys, and installation warnings

Tracking issue: [#475](https://github.com/yicheng47/runner/issues/475). Status: implemented; smoke verified 2026-09-07. Priority P2.

## Problem

Codex can display an interactive update prompt before starting a conversation. Accepting the update installs the new version and exits the process, leaving a direct chat on **Chat paused**. In a running mission, the normal exit cleanup can also stop sibling sessions.

## Decision

Runner passes `-c check_for_update_on_startup=false` to its Codex TUI launches. Jason chose flag-only suppression on 2026-09-07; CLI updates remain outside Runner. There is no updater, version polling, settings switch, prompt matcher, injected input, or automatic restart.

The override is per invocation and does not change the user's Codex configuration. Users update Codex through their existing installation method or `codex update` when supported.

Runner also sets `CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY=1` and `DISABLE_INSTALLATION_CHECKS=1` for Claude Code sessions to suppress quality surveys and installation warnings. Claude's native background updater remains enabled.

## Implementation

`trailing_runtime_args` in `crates/runner-backend/src/router/runtime.rs` adds the Codex-only override independently of model and effort preferences. The shared argument assembly covers fresh direct chats, mission slots, resumes, and the resumed TUI after a fork. The `resume <key>` prefix stays first and any initial prompt stays last. The headless `codex exec fork` materializer does not show the TUI startup prompt and needs no change.

Claude Code and TRAE do not receive this override. Existing sessions retain their original arguments; suppression takes effect on their next launch or resume.

`base_spawn_spec` in `crates/runner-backend/src/session/manager/spawn.rs` applies the two Claude-only environment variables to direct chats, mission slots, forks, and resumes, including runners with custom `--settings` arguments. They do not modify the user's global environment or Claude settings files. Running sessions pick them up on their next launch or resume.

## Verification

- The existing runtime argument assembly test covers fresh Codex launches with no model or effort override, resumed conversations with the original key, initial prompt ordering, and isolation from Claude Code and TRAE.
- `cargo test --locked -p runner-backend --lib`: 593 tests passed. Backend Clippy with `-D warnings`, formatting, and diff checks passed.
- Manual wiring smoke passed (Jason, 2026-09-07): process arguments showed `check_for_update_on_startup=false` on both a fresh Codex chat and a resumed conversation; the Claude environment check also passed for both suppression variables.
