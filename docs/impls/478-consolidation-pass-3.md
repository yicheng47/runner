# 478 — Consolidation pass 3: split `sidebar.rs`

Tracking issue: [#478](https://github.com/yicheng47/runner/issues/478). Chore, P2. Third of four passes. Branch: **`chore/478-pass-3-sidebar` already exists and is checked out** — it carries this brief; work on it, do not create another.

**Read [`archive/478-consolidation-pass-2.md`](./archive/478-consolidation-pass-2.md) first and in full.** Pass 2 split `mission_workspace.rs` the same way and its brief was corrected during that mission; that corrected text is the pattern, the method, and the gate. This brief gives only the cut and what differs. Where they disagree, pass 2 wins on method, this one on the cut.

## What ships

`crates/runner-app/src/surfaces/sidebar.rs` (5,060 lines) becomes the directory `crates/runner-app/src/surfaces/sidebar/`. No fields, behavior, or logic change. Permitted mechanical changes are pass 2's, plus `pub(crate) use` re-exports, which pass 2 did not need (see below). Parsed item equality is the gate.

## What is different from pass 2

**1. No platform `cfg` anywhere.** One `#[cfg(...)]` in 5,060 lines, and it is `#[cfg(test)]`. The hazard that dominated pass 2 — an import pruned on macOS because its only consumer is Windows-gated — cannot occur. Confirm with your own grep rather than taking it from me.

**2. Other modules import this one by path, and that must keep working.** This is the constraint pass 2 did not have. `mission_workspace.rs` was reached only through one re-export; `sidebar.rs` is reached four ways:

- `surfaces/mod.rs:34` — `pub(crate) use sidebar::{default_session_label, session_label, ProjectModal, Sidebar};`
- `panes.rs:13` — `use crate::surfaces::sidebar::{direct_chat_display_status, DirectChatDisplayStatus};`
- `chat.rs:2045` — `super::sidebar::archive_targets_for_chats(…)`
- `chat.rs:2150` — `super::sidebar::archive_all_confirmation_body(…)`

**Invariant: every item that is `pub(crate)` today still resolves at `crate::surfaces::sidebar::<name>` afterwards.** The nine are `archive_targets_for_chats`, `archive_all_confirmation_body`, `SidebarRename`, `ProjectModal`, `Sidebar`, `DirectChatDisplayStatus`, `direct_chat_display_status`, `session_label`, `default_session_label`. Where the cut puts one in a child, add a `pub(crate) use <child>::<name>;` in `sidebar/mod.rs`. **The proof is that `panes.rs`, `chat.rs` and `surfaces/mod.rs` are not in your diff.** If any of the three needs an edit, the re-export is missing — fix the re-export, not the caller.

**3. Expect far more `pub(super)` on free functions.** Pass 2 widened 74 methods and moved 7 helpers to `mod.rs`. Here 33 of 37 free functions have a caller outside the file the cut puts them in, mostly because `tests.rs` exercises them directly. Expected, not a smell. Rule unchanged: compiler-driven `pub(super)`, never `pub(crate)` for anything not already `pub(crate)`, and a helper moves to `mod.rs` only when **three or more** files call it — here that is `node_project_id` alone. Do not contort the cut to avoid widenings.

**4. Private fields crossing a sibling boundary move with their type to the parent.** Human-approved correction (2026-09-13): move `SidebarForkMenuTarget` (original lines 4001–4005, including its derive) unchanged into `mod.rs`; keep its builder `sidebar_fork_menu_target` in `elements.rs`. Its private `session_id` and `disabled_reason` fields are read from `menus.rs` and `tests.rs`; widening only the type does not expose its fields. Methods crossing sibling boundaries get compiler-driven `pub(super)`, while private fields crossing them are fixed by relocating their declaring type to the parent, with no field visibility changes. The other child type, `DirectChatDisplayStatus`, stays in `elements.rs` and retains its re-export. Pass 4 inherits this rule.

## The cut

Line numbers are `sidebar.rs` on `main` at the branch point. Each item carries its doc comment, attributes, and any `const` immediately above it.

| File | From | Holds |
|---|---|---|
| `mod.rs` | 1–27, 49–58, 172–378, 3328–3334, 3700–3707, 4001–4005 | Module declarations, the `pub(crate) use` re-exports, `ArchiveErrorTarget`, `ArchiveSessionOperation`, and every type with its small inherent impl: `SidebarRow`, `SidebarRenameTarget`, `SidebarRename`, `ProjectModal`, `SidebarNodeDrag` and its `Render`, `SidebarMenuAction`, `WorkspaceEntry` with `WORKSPACE_ENTRIES`, the `Sidebar` struct, `SidebarForkMenuTarget`, and `impl Render for Sidebar` |
| `state.rs` | 379–516 | `new` … `focus_shell_terminal`: construction, accessors, settings, store refresh, error reporting, shell notify and focus |
| `activation.rs` | 517–674, 707–769 | `impl NativeRoot`: `tab_label`, both prune fns, transient dismissal, window activation, shortcut-row sync, `mark_active_tab_viewed`, active project, the archiving accessors, `clear_sidebar_drag`, `activate_sidebar_session`, `open_chat_session` |
| `archive.rs` | 60–171, 675–706, 1520–1595, 1597–1668 | The five archive planning free fns, `archive_chat_sessions`, `archive_all_sessions`, `archive_sessions`, `finish_sidebar_archive`, `close_archived_pane` |
| `rows.rs` | 771–919, 1109–1213 | `dismiss_transients` … `scope_rows`, then `toggle_project` and the three rename methods |
| `shortcuts.rs` | 920–1108 | `shortcut_row_walk` … `activate_sidebar_session`: the walk, refresh, selection, modifier and key handling, pill visibility |
| `menus.rs` | 1214–1519, 3524–3699 | `open_sidebar_context_menu` … `handle_sidebar_menu_action`, plus the menu-entry builders and `sidebar_tab_icon` |
| `project.rs` | 1669–1858, 3335–3523 | The project-modal `impl NativeRoot` block, `render_sidebar_overlays`, `render_project_modal` |
| `drag.rs` | 1860–2101 | `clear_sidebar_drag` … `commit_sidebar_drop` |
| `view.rs` | 28–47, 2102–2614 | The two scroll helpers, `render_sidebar_contents`, `render_section_header`, `render_sidebar_row` |
| `rows_render.rs` | 2615–3327 | `render_tab_row` … `render_inline_rename_row` |
| `elements.rs` | 3709–4180 | The presentational builders and label helpers: `project_name_from_path` through `default_session_label_parts`, including `DirectChatDisplayStatus`; `SidebarForkMenuTarget` moves to `mod.rs` per the approved correction above |
| `tests.rs` | 4181–5060 | The `#[cfg(test)] mod tests` body, declared from `mod.rs` as `#[cfg(test)] mod tests;` |

Largest lands near 880 lines (`tests.rs`), then ~715 (`rows_render.rs`). If a boundary falls inside an item, move the whole item and say which range you adjusted. `sidebar_logic.rs` is a separate sibling module, out of scope, do not fold it in.

## Rules of the road

Pass 2's, unchanged, with the file scope being `crates/runner-app/src/surfaces/sidebar*` plus this brief. Moves only, with the mechanical exceptions above. Stage by path. Do not run the app. `surfaces/mod.rs` must not need an edit.

Mission authorization: after the reviewer reports clean, PR mode is authorized — commit, push, open the PR titled `chore(ui): split sidebar.rs into one file per concern (478 pass 3)`, drive CI green on both required checks. Do not merge. No worktrees, no extra checkouts, no extra agents.

## Verification

Pass 2's three gates, method unchanged, so reuse its audit rather than writing a new one:

1. **AST item equality**, keyed by `Type::method`, validated by the same five negative controls (body statement, literal, match arm, `pub(super)` widened to `pub(crate)`, dropped derive). All must fail and name the item. `sidebar.rs` holds four `impl Sidebar` blocks and three `impl NativeRoot` blocks, so name-only keying will produce phantoms — pass 2 saw six from three colliding names and this surface has more.
2. **Test count.** `cargo test --workspace 2>&1 | grep -c '^test .* ok$'` is **1179** on `main` at the branch point.
3. **`make verify` green**, plus `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`. Cite a log from one untouched run; pass 2's cited log was internally inconsistent because the tree changed mid-run. Set `pipefail` or read cargo's status directly — a piped cargo returns the pipe's status.

Also report: the line count of every new file; every `pub(super)` item as `file::item` with a count; the `pub(crate) use` re-exports and the nine external paths they preserve; that `panes.rs`, `chat.rs` and `surfaces/mod.rs` are absent from the diff; and the platform-cfg grep result.

## Non-goals

`crews.rs` and `runners.rs` (pass 4), `sidebar_logic.rs`, every other oversized file, behavior, styling, test behavior, and docs other than corrections to this brief. The `docs/arch/` module map waits until all four passes land.
