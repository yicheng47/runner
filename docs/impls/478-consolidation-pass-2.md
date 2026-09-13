# 478 — Consolidation pass 2: split `mission_workspace.rs`

Tracking issue: [#478](https://github.com/yicheng47/runner/issues/478). Chore, P2. Second of four passes; pass 1 shipped 2026-09-05 in [#479](https://github.com/yicheng47/runner/pull/479) and its tail in [#578](https://github.com/yicheng47/runner/pull/578). Branch: **`chore/478-pass-2-mission-workspace` already exists and is checked out** — it carries this brief; work on it, do not create another.

This pass **fixes the split pattern** passes 3 and 4 copy.

## What ships

`crates/runner-app/src/surfaces/mission_workspace.rs` (7,058 lines) becomes the directory `crates/runner-app/src/surfaces/mission_workspace/` with one file per concern. **Nothing else changes.** Not a signature, not a field, not a line of logic, not a rustfmt reflow. Every byte that is not a `use`, a `mod` declaration, or a re-export exists in the new tree exactly as it exists in the old file.

No file over ~900 lines, most under 700, so an agent can read a whole concern before editing it.

## The pattern

`crates/runner-app/src/surfaces/settings/` is the in-repo precedent — read it first. `settings/mod.rs` is module declarations plus the one shared type; each child carries its own `use` block.

- **Privacy needs no widening.** A child module can name an ancestor's private items, struct fields included: `self.mission_id` in `mission_workspace/state.rs` compiles against the private field in `mod.rs`. If you need `pub(crate)` to make the split compile, stop and report it.
- **`impl` blocks can live anywhere in the crate.** Both `impl MissionWorkspace` and `impl NativeRoot` split freely; neither type moves.
- **Imports.** The old file opens with `use super::*;` and `use crate::*;`. In a child, `use super::*;` now means the `mission_workspace` module, so the old one becomes `use crate::surfaces::*;`; `use crate::*;` is unchanged. Start each child with the parent's full `use` block plus those, then run `cargo fix --allow-dirty --allow-staged -p runner-app` and `make fmt` to prune. Do not hand-curate imports beyond what that leaves.
- **`surfaces/mod.rs`** keeps `pub(crate) mod mission_workspace;` and `pub(crate) use mission_workspace::MissionWorkspace;` exactly as they read today. No other module's imports change. If one does, the cut is wrong.
- **`#[cfg(target_os = ...)]` items travel with their attribute.** A lost `cfg` compiles here and breaks the Windows job, which is the single most likely way this pass goes wrong.

## The cut

Line numbers are `mission_workspace.rs` at the branch point. Each item carries its doc comment, attributes, and any `const` immediately above it.

| File | From | Holds |
|---|---|---|
| `mod.rs` | 1–308, 988–1004, 6599–6625 | Module declarations, the two consts, every type (`MissionTab`, `MissionRailView`, `MissionTransitionKind`, `MissionTransition`, `SlotOverlayState`, `MissionMenuAction`, `MissionRenameModal`, `DeliveryBlocked`, the `MissionWorkspace` struct, `MissionLoadResult`, `MissionRailResizeDrag` and its `Render`, `CachedTerminalSize`), and `impl Render for MissionWorkspace` |
| `state.rs` | 310–846, 948–987 | `new` … `handle_app_store_update`, plus `leave_archived_mission`, `open_crew_editor`, `set_sidebar_archiving`: construction, predicates, `rebuild_event_projection`, accessors, settings, layout load and persist, terminal style |
| `routing.rs` | 1005–1164 | The whole `impl NativeRoot` block: `set_route`, `record_current_runtime_location`, `navigate_runtime_page`, `open_mission`, `estimated_mission_terminal_size`, `sync_mission_subject_ownership` |
| `attach.rs` | 1166–1795 | `open_mission` … `focus_mission_drawer_terminal`: copy entities, size estimators, grid hint, attach and ensure, `begin_mission_transition`, subject ownership, focus |
| `drawer.rs` | 1796–2181 | `toggle_terminal_drawer` … `terminal_status`: drawer lifecycle, resize, interactivity |
| `events.rs` | 2182–2561 | `handle_mission_workspace_event`, `resync_mission_events`, `refresh_open_mission` |
| `actions.rs` | 2562–3224 | `configure_mission_action_menu` … `on_mission_rename_key_down`: action menu, pin, slot actions and confirms, stop / resume / archive, rename |
| `input.rs` | 3226–3524 | `cycle_mission_tab` … `submit_or_clear_mission_input`: tab selection, key, copy, scroll, paste |
| `view.rs` | 847–947, 3525–4212 | Titlebar chrome (`workspace_titlebar_padding`, `render_open_sidebar_button`, `render_titlebar_drag_area`) and `render_mission_workspace` … `render_mission_feed_surface`: shell, overlays, header, notices, load error, drawer render, loaded mission |
| `feed.rs` | 4213–4534, 4889–5385 | `render_mission_tabs` … `copy_feed_selection`, then `render_mission_feed_block` … `answer_mission_question` |
| `composer.rs` | 4535–4888 | `mission_composer_roster` … `post_mission_composer` |
| `terminal_pane.rs` | 5386–5725 | `render_mission_terminal_pane`, the inbox-blocked pill, the stopped / paused / duplicate overlays |
| `rail.rs` | 5726–6598 | `render_mission_rail` … `render_mission_rename_modal`: rail, runners rail, `reveal_mission_cwd`, meta panel, rename modal |
| `tests.rs` | 6626–7058 | The `#[cfg(test)] mod tests` body, declared from `mod.rs` as `#[cfg(test)] mod tests;` |

If a boundary lands inside an item, move the whole item and say which range you adjusted. If a helper is used from two new files, keep it where the table puts it and let the sibling call it, or move it to `mod.rs`; pick the smaller change and note it.

## Rules of the road

- **Moves only.** No renames, no reordering, no extracted helpers, no new abstractions, no comment rewrites, no clippy-suggested tidying. Anything worth changing goes in the handoff, not the diff. A pass that also improves things cannot be reviewed.
- Do not touch any file outside `crates/runner-app/src/surfaces/mission_workspace*`. `surfaces/mod.rs` should not need an edit; if it does, that is a finding.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`) — Jason smoke-tests. Verify with `make verify`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `chore(ui): split mission_workspace.rs into one file per concern (478 pass 2)`, drive CI green (`gh pr checks <n> --watch`; required checks `Rust / macOS` **and** `Rust / Windows` — Windows is the one that catches a lost `cfg`). Do not merge: Jason merges. No worktrees, no extra checkouts, no extra agents.

## Verification

The diff must be provably a move. Three gates, all in the handoff:

1. **Content equality.** Every non-import line survives:

   ```sh
   git show main:crates/runner-app/src/surfaces/mission_workspace.rs \
     | grep -vE '^\s*$|^use |^pub\(crate\) use ' | sed 's/^[[:space:]]*//' | sort > /tmp/mw-before.txt
   cat crates/runner-app/src/surfaces/mission_workspace/*.rs \
     | grep -vE '^\s*$|^use |^pub\(crate\) use |^(pub\(crate\) )?mod |^#\[cfg\(test\)\]$' | sed 's/^[[:space:]]*//' | sort > /tmp/mw-after.txt
   diff /tmp/mw-before.txt /tmp/mw-after.txt
   ```

   Paste the diff; it should be empty. Any line it shows needs a reason (a `cfg(test)` attribute moved onto the `mod` declaration is fine, a changed line of logic is not).

2. **Test count.** `cargo test --workspace 2>&1 | grep -c '^test .* ok$'` is **1179** on `main` at the branch point. It must still be 1179.

3. **`make verify` green**, plus `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`.

Also report: the line count of every new file, and anything you had to make more visible (there should be nothing).

## Non-goals

`sidebar.rs`, `crews.rs`, `runners.rs` (passes 3 and 4), every other oversized file, behavior, styling, tests, docs. The `docs/arch/` module map waits until all four passes land.
