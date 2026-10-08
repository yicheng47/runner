# 723 — Cursor Agent CLI runtime

Implement the Cursor Agent CLI half of [#723](https://github.com/yicheng47/runner/issues/723) for Runner. The user wants Cursor and Grok scheduled separately. This mission covers **Cursor Agent only**; do not implement Grok or close #723. The fork is `ruizer/runner`, and the eventual PR targets `yicheng47/runner:main` with `Part of #723` in its body.

Work only in `/Users/lr/我的/runner/.worktrees/feat-723-cursor-agent` on `feat/723-cursor-agent`, created from `origin/main` at `4726184b`. The root checkout stays on `main`. Do not create another branch or worktree, read files in sibling projects, or share another worktree's Cargo target directory.

Existing uncommitted Cursor-related edits in this worktree are part of this task. Inspect and preserve them; do not stash, reset, restore, or overwrite them.

## Read first

- `AGENTS.md`, especially the project boundary, worktrees, and crew mission rules.
- `docs/arch/runtime-integration.md`: its P0 acceptance contract governs what can be called supported.
- `docs/features/archive/644-antigravity-runtime.md` and `docs/features/archive/777-runtime-adapter.md` for precedent; adapt to verified Cursor behavior rather than copying another runtime's flags.
- `crates/runner-core/src/runtime.rs`, `crates/runner-core/src/protocol/runtime_metadata.rs`, `crates/runner-daemon/src/runtimes/`, the session launch and status paths, and the app's existing runtime selection surfaces.
- The issue's Cursor section and Cursor's official CLI documentation. Verify current command syntax before encoding flags. Use runtime key `cursor`, display name Cursor and default executable `cursor-agent`; accept `agent` only after validating it as Cursor Agent because Grok may use the same name.

The requesting machine did not have `cursor-agent` on PATH when this mission was prepared on 2026-10-08. The requester later installed both entrypoints, observed version `2026.10.01-e373342`, and asked to uninstall them. Do not claim live Cursor behavior has been verified from passing unit tests. Do not inspect another project's files to infer Cursor behavior.

## Deliverables

1. Add `docs/features/723-cursor-agent-runtime.md` with Motivation, Scope, Implementation Phases, Verification, the issue URL, and a platform evidence table. Link it from `docs/features/README.md`. Keep the spec focused on Cursor, though #723 also names Grok.
2. Register Cursor Agent as a stable runtime and carry that identity through role and crew selection, direct chats, persisted sessions, CLI/API, Settings discovery and override, managed Runner skill, and the app's runtime displays. Preserve unknown and legacy runtime handling.
3. Implement the interactive PTY launch through a Cursor-owned adapter: first-turn persona and mission goal, unattended mission permissions, direct-chat approval behavior, model/default semantics, process lifecycle, and exact conversation identity and resume. Do not guess a session ID from a global newest chat or cwd. Treat concurrent slots in one cwd as a required case.
4. If reliable chat-key capture or unattended execution cannot be established from documented behavior or a reproducible probe, report the blocker in the mission bus and keep the unsupported path honest. Do not paper over a P0 gap by labeling the runtime supported.
5. Add focused tests for runtime identity, launch arguments, permission flag stripping, prompt delivery, key capture and resume, concurrent-session isolation, and capability boundaries. Update both READMEs and relevant runtime documentation together. Capture a terminal fixture only from real Cursor output; do not fabricate one.
6. Query account-specific Cursor models through the existing background discovery/cache pipeline. Keep semantic hooks/status, native fork, MCP management, and update UI as separate follow-ups unless required for a correct P0 integration. Do not broaden this mission to Grok.

## Verification and handoff

The coder checks the existing diff, finishes implementation and tests, then hands the complete working-tree diff to the reviewer through Runner. The reviewer reads that diff against `docs/arch/runtime-integration.md` and gives must-fix findings with file:line references. The coder fixes them and repeats the handoff until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. Do not start additional agents, crews or subagents.

Run the relevant `runner-core`, `runner-daemon`, and `runner-app` tests; workspace Clippy with warnings denied; `cargo fmt --all --check`; and `git diff --check`. Record commands and exit codes. Gate Unix-only test imports and helpers with `#[cfg(unix)]` for Windows CI. On this Mac, GPUI needs `--features gpui_platform/runtime_shaders` because Xcode is unavailable; do not install Xcode. Do not start or operate the Runner dev app or live Cursor sessions from the crew. Native macOS and Windows behavior must be labeled unverified until a human-authorized smoke run supplies evidence.

After clean review, squash the branch, including this brief, into one focused commit on top of current `origin/main`, push to the fork, and open a PR against upstream `main` with `Part of #723`. Do not use `Fixes #723` or close the issue while Grok remains. Watch macOS and Windows CI, amend and force-with-lease push fixes after renewed review, and stop at an open PR with CI green. Never merge, delete the branch or worktree, cut a nightly, or release. If a P0 blocker remains, report it before opening a PR that claims support; a draft PR may carry reviewable partial work with the blocker stated plainly.

Final Runner handoff: PR URL or blocker, changed files, test commands and results, review verdict, platform and live-runtime evidence gaps, and concrete human smoke checks needed to certify Cursor support.
