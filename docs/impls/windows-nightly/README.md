# 437 — Windows nightly: program record

Implementation program for [feature 437 — Windows nightly](../../features/437-windows-nightly.md) ([#437](https://github.com/yicheng47/runner/issues/437)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [local-skills](../local-skills/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) is the four-phase plan with the per-file touch list, [impl_log.md](impl_log.md) is the dated log and the Windows nightly stamps. Mission briefs are sent verbatim as the mission goal and are recorded in the log entry for that phase.

## Status (2026-09-05)

Phases 0 and 1 have landed on the long-lived `nightly-windows` branch, plus the first Phase 1 checklist fix. Two Windows nightlies are published on the public `nightly-win` pre-release; the newest is `Runner-Nightly-0.7.5.20260905.1323-x64.zip`. Jason's PC pass on the first zip: the app opens; the doubled title bar was filed as [#484](https://github.com/yicheng47/runner/issues/484) and fixed the same day in [#485](https://github.com/yicheng47/runner/pull/485). The rest of the Phase 1 checklist (database path, Settings, PowerShell pane, Ctrl+C, IME, DPI) is not yet reported. Next: the PC pass on the 1323 zip, then the Phase 2 brief. Open question: an unsigned Inno Setup installer for the friend, raised twice on 2026-09-05 and not yet decided.

## End state

A `nightly-win` pre-release zip on the public releases page, built by `nightly.yml` from `nightly-windows`, that a Windows user downloads without a GitHub account: `Runner.exe` with `runner-agent-cli.exe` and `runner-mcp.exe` beside it, x64, unsigned, no installer, no updater. Direct chats and crew missions run natively on ConPTY with Job Objects, the `runner` CLI reaches agents through both shims, and the MCP proxy talks to the app over a named pipe. macOS is byte-identical after every phase.

## Decisions that bind

The plan's [Decisions](plan.md#decisions) section is the full list. The ones a mission is most likely to trip over:

1. **`nightly-windows` is the base branch, not `main`.** Created 2026-09-05 from `main` at `7cf7dd1`, checked out in the `../runner-windows` worktree. Every phase and fix PR bases on it and merges into it; `nightly.yml` builds from it; `ci.yaml` triggers on it. `main` stays the macOS product. It has no branch protection, so `gh pr merge --auto` merges at once there.
2. **The release tag is `nightly-win`, never `nightly-windows`.** A tag with the branch's name makes `git push origin nightly-windows` an ambiguous refspec after the first cut. Decided during the Phase 1 review.
3. **Workflow-file PRs land with a local `--no-ff` merge.** The `gh` token lacks the `workflow` scope, so `gh pr merge` refuses any PR that touches `.github/workflows/`. #482, #483 and #485 all landed as `git checkout nightly-windows && git merge --no-ff <branch> && git push`; GitHub still records the PR as merged.
4. **Keymap: `Ctrl+Shift+<letter>`, plain `Ctrl` for everything else, and nothing a terminal needs.** `Ctrl+[` is ESC and `Ctrl+PageUp/PageDown` cycle panes and mission tabs; `Ctrl+Shift+PageUp/PageDown` are page history; feed copy is plain `Ctrl+C`, terminal copy/paste `Ctrl+Shift+C/V`; fullscreen is `F11`; Quit and both Hide bindings are dropped on Windows.
5. **Runner draws its own title row on Windows.** The OS bar is hidden (`appears_transparent: true`), the 44px header is the title row with the sidebar toggle at its left and the three caption buttons at the window's top-right, rendered once at the app root above every overlay. gpui-ce performs minimize, maximize and close itself on the non-client mouse-up, so the buttons carry no click handlers. Decided in #484 after the first PC pass; the plan's original "standard OS title bar" is superseded.
6. **The PC is a run box.** Builds and tests come from CI. Missions never launch `Runner.exe` over ssh and never install anything on the PC unprompted.
7. **cfg-split, verbatim moves, no macOS change.** Platform code is selected at compile time; unix bodies move without edits; every phase leaves the macOS build, tests and `Rust / macOS` unchanged. Windows-only tests compile in CI but do not run until Phase 2 adds `cargo test` to the Windows job.
