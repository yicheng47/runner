# 478 — Consolidation pass 4: split `crews.rs` and `runners.rs`

Tracking issue: [#478](https://github.com/yicheng47/runner/issues/478). Status: shipped 2026-09-13 in [#581](https://github.com/yicheng47/runner/pull/581) (mission `01M2CZC69PGG7CRN4A1H55AWP5` on codex-crew, 9 min launch to merge, no must-fix). Chore, P2. Last of four passes; with it #478 is complete.

**Read [`478-consolidation-pass-3.md`](./478-consolidation-pass-3.md) and [`478-consolidation-pass-2.md`](./478-consolidation-pass-2.md) first.** Between them they hold the pattern, the four privacy rules, the audit, and the gate. This brief gives only the two cuts and what differs. Where they disagree, the later pass wins on method and this one wins on the cut.

## What ships

Two directory splits in one PR:

- `crates/runner-app/src/surfaces/crews.rs` (3,494 lines) → `surfaces/crews/`, 10 files
- `crates/runner-app/src/surfaces/runners.rs` (3,337 lines) → `surfaces/runners/`, 10 files

**Two separate trees, not one shared module.** The two files independently declare five identically named items — `FORM_WIDTH`, `FIELD_WIDTH`, `runtime_models`, `trimmed_option`, `error_banner`. Separate module trees keep them apart and that is correct today. Do not create a shared file, a common parent, or a `surfaces/entities/` to deduplicate them. Whether those five should ever be shared is a real question and it is not this pass's.

## What is different from passes 2 and 3

This is the easiest of the four. Both hazards that cost the earlier passes time are absent, and I have checked both rather than assuming:

**1. No platform `cfg` in either file.** One `#[cfg(...)]` each and both are `#[cfg(test)]`. Pass 2's pruned-import hazard cannot occur. Confirm with your own grep.

**2. No external path importers, so no re-exports needed.** Pass 3 had nine crate-visible items reached from `panes.rs` and `chat.rs`. Here each file has exactly one `pub(crate)` item — `CrewSurfaces` and `RunnerSurfaces` — each reached only through its `surfaces/mod.rs` re-export, and both land in their own `mod.rs` by the cut. So `surfaces/mod.rs` needs no edit and no `pub(crate) use` is required. If you find yourself adding either, stop and report it.

**3. The field-privacy rule from pass 3 applies to exactly three items, and I have found all three.** A type declared in a child whose private fields are read from a sibling must move to `mod.rs`; `pub(super)` on the type does not expose its fields.

- **`RunnerEditResolution`** (runners.rs:2669) — built at 2728 in `resolve_runner_edit`, consumed at **runners.rs:470** inside `open_runner_edit`, a different region. Fields `runtime`, `runtime_pinned`, `command`, `model`, `effort`. **Move to `runners/mod.rs`.**
- **`RuntimeLayerResolution`** (runners.rs:2678) — built at 2692 and 2712, and its fields are read from the test module at 3221 and 3293 onward. **Move to `runners/mod.rs`.**
- **`CrewNameRefresh`** (crews.rs:3087) — unit-variant enum, so E0616 is impossible, but it is named from the editor region at 700–708 and from tests. It needs `pub(super)`, not a move.

Every other type in both files already lands in `mod.rs`. Take the compiler's word over mine if it disagrees, and say so.

## The cut — `crews.rs`

| File | From | Holds |
|---|---|---|
| `mod.rs` | 1–29, 30–156, 157–189 | Imports, `FORM_WIDTH`, `FIELD_WIDTH`, every type (`SlotDrag` and its `Render`, `CrewEditorState`, `CreateCrewForm`, `AddSlotForm`, `CrewMenuAction`, `SlotMenuAction`, `CrewDeleteConfirm`, `SlotRemoveConfirm`, `CrewSurfaces`), and `impl CrewSurfaces` |
| `list.rs` | 191–621 | `open_crews` … `handle_crew_menu_action`: navigation, query, paging, the surface, page and card renders, the crew menu |
| `editor.rs` | 622–1132 | `load_crew_editor`, `render_crew_editor`, `on_crew_name_key_down`, `save_crew_name` |
| `editor_sections.rs` | 1133–1456 | The goal and conventions sections with their start, cancel and save |
| `create.rs` | 1457–1729 | The create-crew modal and `render_crew_delete_confirm` |
| `add_slot.rs` | 1730–2450 | The whole add-slot modal |
| `slots.rs` | 2451–2900 | Slot list and row, slot menu, lead, drag clear, reorder, remove confirm |
| `overlays.rs` | 2901–3052 | `confirm_slot_remove`, `advance_crew_delete`, `render_crew_overlays`, `finish_crew_update` |
| `logic.rs` | 3053–3364 | The free functions and `CrewNameRefresh` |
| `tests.rs` | 3365–3494 | The test module, declared from `mod.rs` |

## The cut — `runners.rs`

| File | From | Holds |
|---|---|---|
| `mod.rs` | 1–33, 34–138, 139–171, plus 2669–2684 moved | Imports, the two consts, every type, `impl RunnerSurfaces`, and the two resolution structs relocated per point 3 |
| `forms.rs` | 173–673 | `refresh_runner_form_runtimes`, `open_create_runner`, `open_runner_edit` |
| `create.rs` | 704–1052 | Create-runner interactions and the modal |
| `edit.rs` | 1053–1527 | Edit interactions and the drawer |
| `delete.rs` | 674–703, 1528–1607 | `render_entity_overlays`, the delete confirm, `confirm_runner_delete` |
| `list.rs` | 1608–2139 | Navigation, query, paging, loads, the surface, page and card renders |
| `detail.rs` | 2140–2523 | `render_runner_detail` and `render_runner_detail_body` |
| `menu.rs` | 2524–2663 | `open_runner_menu`, `handle_runner_menu_action`, `start_runner_chat` |
| `logic.rs` | 2685–3213 | The free functions and `RunnerFormKind` |
| `tests.rs` | 3214–3337 | The test module, declared from `mod.rs` |

Largest file in either tree lands near 720. If a boundary falls inside an item, move the whole item and say which range you adjusted.

## Rules of the road

Pass 3's, unchanged, with the file scope being `crates/runner-app/src/surfaces/crews*` and `crates/runner-app/src/surfaces/runners*` plus this brief. Moves only, with the mechanical exceptions the earlier briefs allow. Stage by path. Do not run the app. `surfaces/mod.rs` must not need an edit.

Mission authorization: after the reviewer reports clean, PR mode is authorized — commit, push, open the PR titled `chore(ui): split crews.rs and runners.rs into one file per concern (478 pass 4)`, drive CI green on both required checks. Do not merge. No worktrees, no extra checkouts, no extra agents.

## Verification

Pass 2's three gates, method unchanged, **run once per surface**. Reuse the audit rather than writing a new one; it takes the surface as a parameter.

1. **AST item equality**, keyed by `Type::method`, validated by the five negative controls. Report two separate results, one per surface, and say so — a single combined pass number would hide a regression in one tree.
2. **Test count.** `cargo test --workspace 2>&1 | grep -c '^test .* ok$'` is **1179** on `main` at the branch point, measured after pass 3 landed. It must still be 1179.
3. **`make verify` green**, plus `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`. One untouched run, cited honestly. Set `pipefail` or read cargo's status directly. Four `warning:` lines are expected and are on `main` too: the future-incompatibility notice for `block` and `proc-macro-error2`, once per phase.

Also report, per surface: the line count of every new file; every `pub(super)` item as `file::item` with a count; that `surfaces/mod.rs` is absent from the diff; the platform-cfg grep result; and confirmation that the two resolution structs moved and `CrewNameRefresh` did not.

## Non-goals

Deduplicating the five colliding names, `sidebar_logic.rs`, every other oversized file, behavior, styling, test behavior, and docs other than corrections to this brief.

**When this lands, all four passes are done.** The `docs/arch/` module map and the decision about the files that have grown since the 2026-09-04 audit both come after, and neither is in scope here.

## Outcome

Both cuts landed as prescribed. `crews/` is 10 files, largest `add_slot.rs` at 750; `runners/` is 10 files, largest `logic.rs` at 562. Seventy-six compiler-proven `pub(super)` items, `surfaces/mod.rs` untouched, no shared module created. The three predicted field-privacy items resolved as written: `RunnerEditResolution` and `RuntimeLayerResolution` moved to `runners/mod.rs`, `CrewNameRefresh` took `pub(super)`. One the brief missed, `RunnerFormKind`, needed `pub(super)` for the same reason and the compiler found it.
