# #1266 — Switch sidebar view

## Starting point

Fix [#1266](https://github.com/yicheng47/runner/issues/1266) with the **codex trio**, as Jason explicitly requested on 2026-11-12. Branch: `feat/1266-switch-sidebar-view`. Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1266-switch-sidebar-view`. Base: `origin/main` at `b841b51a63fae25426fd9a36b4da23b927213efa`.

## Outcome and scope

Implement the [feature spec](../../features/1266-switch-sidebar-view.md), derived from the fully specified, `agent-ready` issue. Add the rebindable `switch-sidebar-view` action (⌘. / Ctrl+.), switching Projects ↔ Active through the current list-filter behavior and expanding a collapsed sidebar. Include local persistence, the remote computer path, updated number-shortcut rows, Settings and command palette discovery, both translations, and Windows/Linux terminal-priority behavior. Reuse existing visual components; no new UI design decisions are open.

The spec and issue are authoritative. Keep this to the new shortcut and its integration. Update the two guide pages the spec names and their existing Chinese counterparts; regenerate no CLI docs.

## Read first

- `AGENTS.md`, especially Test Scope, Commit And PR Conventions and Crew Missions.
- `crates/runner-app/src/keymap.rs`: configurable entries, platform bindings and terminal-priority routing.
- `crates/runner-app/src/lib.rs` and `surfaces/app_shell/{mod.rs,actions.rs,sidebar.rs}`: actions, `toggle_sidebar`, the list-filter handler and collapsed-sidebar state.
- `surfaces/sidebar/{model.rs,activation.rs,shortcuts.rs}` and `surfaces/remote_client/tests.rs`: `toggle_active_only`, `sync_sidebar_shortcut_rows` and remote state coverage.
- `surfaces/command_palette.rs`, `surfaces/settings/page/shortcuts.rs`, and `i18n/{en,zh-hans}.toml`.

## Validation and handoffs

List changed behaviors and selected checks with their reasons before testing. Automated coverage must exercise the new action and effective binding, terminal-priority routing and tag, collapsed expansion, local persistence and remote sidebar row synchronization. Run runner-app tests, workspace Clippy and fmt checks; drive all required CI green.

Use the codex trio's frozen-candidate review → QA handoff. Reviewer names the selected native checks; QA executes them through computer use, taking the development-app lock first. QA preflights access to a Windows machine and computer use on its first turn, without building or launching. Required native checks are the spec's view switching/expansion and persistence, updated number selection, Settings rebind/default removal, command palette execution, and Windows Ctrl+. routing plus Outside terminals. Reuse only the affected portions of WS-ACTIVE-04, WS-ACTIVE-02 and TERM-INPUT-06, not their full matrices. Test the remote route in-process; no new remote setup is required.

QA records candidate SHA, pass/fail/blocked per check, screenshots, and settings restored in `docs/tests/1266-switch-sidebar-view.md`. Record missing platform prerequisites explicitly. A PR may report an incomplete QA verdict, but keeps `needs-live-test` until every selected required check passes; never represent missing Windows native coverage as passed. The final handoff includes the PR URL, reviewed SHA, checks/CI and any remaining live-test gap.

## Authorization

Jason approved the codex trio (coder, reviewer and QA) for #1266. The mission may commit locally, push its branch and open a pull request under the standard Crew Missions workflow; merge, nightly and release are not authorized.
