# 478 — Consolidation pass 2: split `mission_workspace.rs`

Tracking issue: [#478](https://github.com/yicheng47/runner/issues/478). Chore, P2. Second of four passes; pass 1 shipped 2026-09-05 in [#479](https://github.com/yicheng47/runner/pull/479) and its tail in [#578](https://github.com/yicheng47/runner/pull/578). Branch: **`chore/478-pass-2-mission-workspace` already exists and is checked out** — it carries this brief; work on it, do not create another.

This pass **fixes the split pattern** passes 3 and 4 copy.

## What ships

`crates/runner-app/src/surfaces/mission_workspace.rs` (7,058 lines) becomes the directory `crates/runner-app/src/surfaces/mission_workspace/` with one file per concern. No fields, behavior, or logic change. The permitted mechanical changes are imports and module declarations, split `impl` wrappers, compiler-proven `pub(super)` visibility, and rustfmt reflow required by widened signatures or the dedented test module. Parsed item equality is the pass/fail gate; preserve every comment and account for the text differences separately.

No file over ~900 lines, most under 700, so an agent can read a whole concern before editing it.

## The pattern

`crates/runner-app/src/surfaces/settings/` is the in-repo precedent — read it first. `settings/mod.rs` is module declarations plus the one shared type; each child carries its own `use` block.

- **Fields and methods have different privacy rules.** Struct fields declared in `mod.rs` are visible to every descendant module, so `self.mission_id` from `state.rs` needs no change. Private inherent methods are visible only in their defining module and its descendants, so a method moved into `state.rs` and called from `attach.rs` produces E0624.
- **Preserve the original visibility scope with `pub(super)`.** Apply it only to items the compiler rejects. From a child, `super` names `mission_workspace`, allowing sibling access while keeping the item inaccessible outside the directory. Never widen to `pub(crate)` or `pub`; if either seems necessary, stop and report it because the item is reached from outside `mission_workspace` and the cut is wrong.
- **Visibility changes are compiler-driven only.** Move the code, build, and add `pub(super)` to what E0624 names, one round at a time. Do not pre-emptively mark methods that might be shared. The same rule covers private free functions and private types declared in a child and used by a sibling. List every changed item as `file::item`, with a count, in the handoff so review can confirm each cross-file caller.
- **`impl` blocks can live anywhere in the crate.** Both `impl MissionWorkspace` and `impl NativeRoot` split freely; neither type moves.
- **Imports.** The old file opens with `use super::*;` and `use crate::*;`. In a child, `use super::*;` now means the `mission_workspace` module, so the old one becomes `use crate::surfaces::*;`; `use crate::*;` is unchanged. Start each child with the parent's full `use` block plus those, then run `cargo fix --allow-dirty --allow-staged -p runner-app --all-targets` and `make fmt` to prune. The compiler reports 13 imports without machine-applicable fixes: remove the 12 unused `futures::StreamExt as _` copies, keeping the one used by `state.rs::new`, and the unused `routing.rs` `use super::*;`. No other hand-curation is authorized.
- **Audit imports across platforms.** List every removed or flagged import per file and check its consumers in platform-gated blocks before review. Keep `rail.rs`'s `MouseButton` on its own `#[cfg(windows)] use gpui::MouseButton;` line. Move the macOS-gated `SIDEBAR_TOGGLE_GLYPH_INSET` / `SIDEBAR_TOGGLE_GLYPH_X` import to `view.rs` with its attribute; `mod.rs` no longer carries it. Keep `rail.rs`'s plain `use std::process::Command;`, since complementary macOS and Windows arms both use it. The remaining platform sites in `input.rs`, `view.rs`, `rail.rs`, and `tests.rs` introduce no other import with only platform-gated consumers.
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

- **Moves only, with the mechanical exceptions above.** No renames, no reordering, no extracted helpers, no new abstractions, no comment rewrites, no clippy-suggested tidying. Anything worth changing goes in the handoff, not the diff. A pass that also improves things cannot be reviewed.
- Do not touch any file outside `crates/runner-app/src/surfaces/mission_workspace*`, except this brief to record the human-approved corrections. `surfaces/mod.rs` should not need an edit; if it does, that is a finding.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`) — Jason smoke-tests. Verify with `make verify`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `chore(ui): split mission_workspace.rs into one file per concern (478 pass 2)`, drive CI green (`gh pr checks <n> --watch`; required checks `Rust / macOS` **and** `Rust / Windows` — Windows is the one that catches a lost `cfg`). Do not merge: Jason merges. No worktrees, no extra checkouts, no extra agents.

## Verification

The diff must be provably a move. Three gates, all in the handoff:

1. **AST content equality.** Parse the original file from `main` and every file in the new directory with `syn`. Ignore imports and module declarations, flatten `impl` wrappers into individually compared members, normalize only the approved `pub(super)` prefixes, and ignore signature trailing commas. All **203 original items** must match the **203 moved items**, with zero differences. Method bodies, fields, existing visibility, doc comments, and item/body attributes remain part of the comparison. Check ordinary comments separately. Keep the audit source and rerun instructions in `/tmp` or the scratchpad for the reviewer; do not commit the audit script.

   Validate the comparator with three separate negative controls against temporary after-side source files: delete a method-body statement, change an integer or string literal, and drop a match arm. Each must fail and identify the changed item; paste all three failures, then rerun the unmodified tree and show zero differences. Do not mutate the repository for these controls.

   Keep the raw sorted-line diff as evidence, not a pass/fail gate. Enumerate its categories with counts: multiline import continuations and import attributes, the **8 added `impl MissionWorkspace` wrappers** and their braces, the **5 rustfmt-wrapped `pub(super)` signatures**, the **2 unwrapped dedented test expressions**, and the removed test-module brace. The reviewer must inspect all seven rustfmt deltas individually. Also verify module placement and platform-specific imports, which the normalized AST comparison deliberately does not prove.

   The earlier prediction of one extra original test-module brace was incomplete: splitting the implementation adds eight closing braces, so the net change is **seven added bare braces**. A line filter that removes whole import statements and `impl` headers leaves those seven braces, **40 lines** from the seven formatting deltas, and the one added `#[cfg(windows)]` import attribute. That **48-line residue** is fully mechanical; parsed item equality remains the gate.

2. **Test count.** `cargo test --workspace 2>&1 | grep -c '^test .* ok$'` is **1179** on `main` at the branch point. It must still be 1179.

3. **`make verify` green**, plus `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`.

When piping Cargo output through `grep`, `tee`, or `tail`, enable `set -o pipefail` or inspect Cargo's exit status directly. A successful final pipeline command does not prove Cargo succeeded. A local Windows cross-check is optional here: the installed Rust target alone cannot compile bundled SQLite without Windows C headers; the Windows CI job remains required.

Also report: the line count of every new file, every compiler-proven `pub(super)` item as `file::item` with a count, and the per-file import audit. No fields change visibility, and no item gains `pub(crate)` or `pub`.

## Non-goals

`sidebar.rs`, `crews.rs`, `runners.rs` (passes 3 and 4), every other oversized file, behavior, styling, test behavior, and docs other than the corrections to this brief. The `docs/arch/` module map waits until all four passes land.
