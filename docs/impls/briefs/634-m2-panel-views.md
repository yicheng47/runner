# 634 m2 — side panel views

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m2-panel-views`, on branch `feat/634-m2-panel-views`, cut from the umbrella `origin/feat/634-file-preview` at `2e24068d`. This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. The umbrella follows `main` by rebase, and this branch follows the umbrella by rebase (`git rebase origin/feat/634-file-preview`); never merge either into it. m1 runs in parallel on `feat/634-m1-files-service` in `runner-core` and `runner-daemon` only; if it merges first, rebase onto the new umbrella.

## Problem and outcome

#634 adds a Files view to the side panel. Before the view has content (m3), the panel needs the shape it will live in: Jason decided on #1072 (2026-10-28) that side-panel views sit on an internal view contract with Files as its first new view, and on 2026-11-03 that the header separates the session's views from the project's and that terminal-only tabs get a panel. This mission builds that contract, moves Info and Sessions onto it unchanged, redraws the header and adds the terminal tab's panel. Files is a registered view with an empty body until m3.

## Read first

- [The plan](../634-file-preview/plan.md), Phase 2, m2. It is the authoritative scope; this brief adds only what the plan leaves to the mission. The [spec](../../features/634-file-preview.md), Files view and its root, wins on behaviour.
- The design, exported as PNGs in this worktree's ignored `target/design-ref/` from `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design` (commit `e58d8a0`): `header-files-on-right.png` (board `uBcLR`, the chosen header for a mission and a chat), `terminal-tab-files-panel.png` (board `F9OyhG`, a terminal-only tab's panel) and `mission-with-file-open.png` (the mission header in use; the open file and tree are m4's, not yours). They win on visual detail; match them with the existing theme tokens, the sample data is illustrative, and the Files body content in them belongs to m3. Do not delete them or open `.pen` files.
- `crates/runner-app/src/surfaces/panes/` (`panel.rs`, `header.rs`, `mod.rs`, including `side_panel_open`, which today hides the panel for terminal tabs), and `surfaces/mission_workspace/rail.rs` (the mission's Sessions and Info).

## Deliverables

1. **The view contract**, internal to `runner-app`. A view declares its id, label and icon, its group (the session's views or the project's), the surfaces it appears on (chat, terminal-only tab, mission) and whether it is on, which m6 wires to Settings (always on here). It renders from a context naming the subject (session or mission), its working directory, its project and whether it is remote. Keep it as small as these three views need; no plugin loading, no registry beyond what the panels use.
2. **Info and Sessions on the contract** with no visible change: a chat's Info, a mission's Sessions and Info.
3. **Files registered** in the project group on every surface, with an empty body.
4. **The header**: the session's views as icons on the left; the project's as labelled buttons on the right beside the collapse control, with the divider as drawn. For a chat and a mission.
5. **The terminal tab's panel**: a tab of only terminals gets a side panel holding Files alone, with only the Files button; it shares a chat's open state, width, toggle and resize. A tab splitting a chat with a terminal still follows the last focused chat (the existing behaviour and its test).
6. The guide: the side panel's header and the terminal tab's panel in [Chats and windows](../../guide/guides/chats-and-windows.md) and [Missions](../../guide/concepts/missions.md), kept short since Files has no content yet.

Non-goals: the Files view's content, the panel widening, Settings, any daemon or protocol change (m1 owns those), and reordering views. If a design question arises that the plan, spec and PNGs do not settle, ask Jason through Runner rather than choosing silently.

## Validation

Unit tests for the contract's grouping and surface filtering, and `side_panel_open`'s new terminal-tab behaviour; the header for a chat and a mission and the terminal tab's panel from snapshots in both themes, using the existing GPUI test patterns. Existing panel tests keep passing, including the split-terminal and minimum-width ones. `runner-app` tests and workspace clippy with the CI profile, logged with exit codes.

Live checks are required, since the header and a new panel change what a person sees. The reviewer's clean review names them; expected: the header in a chat, a mission, a terminal-only tab and a mixed tab against the PNGs; Info and Sessions unchanged; the panel toggle and resize unchanged in each. The operator adds the `qa` role after the clean review; the crew does not run the app.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole working-tree diff against the plan, spec, PNGs and this brief, must-fix first with file:line pointers, checking in particular: that Info and Sessions render exactly as before, that the contract stays minimal, the terminal tab's panel sharing the chat panel's state, and the mixed-tab focus rule.

## Authorization

Approved crew: the claude duo. Once review is clean the crew may commit, push this branch and open a PR against `feat/634-file-preview` (not `main`), titled `feat(ui): side panel views with Files on the right (#634 m2)`, labelled `frontend-only`, `risk-medium` and `needs-live-test`, and drive CI green on every job. Jason merges it into the umbrella as a rebase merge; the crew does not merge.
