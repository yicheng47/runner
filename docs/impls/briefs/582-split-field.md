# 582 — Split `ui/field.rs` (pass 2)

Tracking issue: [#582](https://github.com/yicheng47/runner/issues/582). Chore, P3. Jason requested this mission on 2026-10-01. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/chore-582-split-field`, on the existing branch `chore/582-split-field`, created from `origin/main` at `6549cd58`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Another crew may be working in `.worktrees/feat-772-keyboard-create`; treat it as another machine's checkout.

Pass 1 (#712) split the two test modules. This pass takes the largest file in the repo, `crates/runner-app/src/ui/field.rs`: 4,251 lines at the branch point, about 2,870 of code and a 1,380-line test module. `router/runtime.rs` is left alone because #777 dismantles it, and `start_chat.rs` because PR #776 is rewriting it.

## What ships

`ui/field.rs` becomes the directory `ui/field/`, a move with no behavior change. No new file over ~700 lines. `ui/mod.rs`, `ui/list.rs` and `ui/model_field.rs` must not change: every name they import (`ui/mod.rs:38` re-exports eleven; the other two import `crate::ui::field::TextField`) must still resolve at `crate::ui::field::<name>`, through a `pub use` in `field/mod.rs` when the item moved to a child. If any file outside `ui/field*` needs an edit, the cut is wrong: stop and report.

## Read first

- `AGENTS.md`, Worktrees and Crew Missions.
- `docs/impls/archive/478-consolidation-pass-2.md`, "The pattern" and "Verification", and `478-consolidation-pass-3.md`, "What is different from pass 2". This pass inherits both, with the differences below.
- `docs/impls/briefs/582-split-test-files.md`, "Verification": its gates are reused here.

## The pattern, as it applies here

- **Three privacy errors, three fixes.** A struct field declared in `mod.rs` reaches every child as is. A private method or function moved into a child and called from another file is E0624 or E0425: add `pub(super)` to exactly what the compiler names, never `pub(crate)` or `pub`. A private field of a type declared in a child and read from another file is E0616, and `pub(super)` on the type does not help: declare that type in `mod.rs` instead. The tests count as another file; they read `TextField`, `TextBuffer` and `FieldTextState` internals directly.
- **Placement rule, which wins over the table below:** `mod.rs` declares every struct and enum whose private fields are read from more than one new file, the `TextField` struct (1336–1373) included. A type touched only by its own file travels with its impls. `impl` blocks split freely.
- **Imports.** `mod.rs` keeps the original top-of-file `use` block. Each child opens with `use super::*;` plus what it needs. Prune with `cargo fix --allow-dirty --allow-staged -p runner-app --all-targets` and `make fmt`; list anything you curate by hand.
- **`#[cfg(test)]` items keep their attribute:** `TextField::text_right_padding` (1559) and `enter_should_submit` (2837). The file has no platform `cfg` items; the `cfg!(windows)` expressions are ordinary code.
- **Order.** Items and tests keep their source order within each new file.

## The cut

Line numbers are `field.rs` at `6549cd58`. Each item carries its doc comment and attributes. If a range lands inside an item, move the whole item; if a file lands over ~700 lines or under ~80, adjust along its own seam. Post the final table (file, source ranges, line count) with the review handoff and say which boundaries moved.

| File | From | Holds |
|---|---|---|
| `mod.rs` | 1–30, shared types, 2098–2279 | Imports, `mod` declarations, `pub use` re-exports, `KeyDownInterceptor`, the two consts, the types the placement rule keeps, `impl Focusable` and `impl Render for TextField`, `input_border_color` |
| `buffer.rs` | 31–582 | `Selection`, `MarkedText`, the undo history (`EditIntent` … `is_adjacent`), `TextBuffer`, `Boundary` |
| `layout.rs` | 605–1100 | `FieldTextKey`, `Spot`, `GlyphBox`, `Stop`, `LineGeometry`, `FieldLine`, `FieldText`, `FieldTextState` |
| `element.rs` | 1101–1311 | `TextFieldElement` and its `Element` impl |
| `text_field.rs` | 583–604, 1312–1335, 1374–1588, 1705–1775, 1944–2001 | `TextFieldKind`, `FieldValidation`, the builder and setter API (`new` … `set_disabled_cursor_not_allowed`), `reveal_caret` … `text_key`, `compact_path_shown`, `render_text` |
| `pointer.rs` | 1776–1943 | `index_for_point` … `on_mouse_up`: hit testing, vertical moves, selection drags, auto-scroll |
| `input.rs` | 1589–1704, 2002–2097, 2672–2870 | `on_key_down` … `on_redo`, `impl EntityInputHandler`, `handle_key_down` … `is_word` |
| `form.rs` | 2280–2567 | `Label`, `FieldError`, `Field`, `BrowseField` |
| `working_dir.rs` | 2568–2671 | `WorkingDirField` … `effective_working_dir`, with the path compaction helpers |
| `tests/mod.rs` | 2873–3030 | Test imports and constants, `FieldHost`, the rendered-field helpers `open_field` … `wrapped_field` |
| `tests/rendered.rs` | 3031–3470 | Click, drag, vertical move, scroll, auto-grow, IME bounds, placeholder caret |
| `tests/path.rs` | 3471–3562, 4225–4250 | Path compaction, working directory precedence |
| `tests/editing.rs` | 3563–3736, 4208–4224 | Shortcuts, marked text, graphemes, normalization, Enter, validation |
| `tests/undo.rs` | 3737–4207 | `type_text`, `caret_at`, the undo history tests |

`field/mod.rs` declares `#[cfg(test)] mod tests;`.

## Rules of the road

- **Moves only**, plus the mechanical exceptions: imports, `mod` declarations, `pub use` re-exports, split `impl` wrappers, compiler-proven `pub(super)`, rustfmt reflow. No renames, extracted helpers, comment rewrites, clippy tidying or test logic changes. Anything worth changing goes in the handoff.
- Touch only `ui/field.rs` and the new `ui/field/`, plus this brief if the human approves a correction. Stage by path, never `git add -A`.
- Crews never run the dev app or drive Jason's Runner for UI checks; Jason smoke-tests. Do not start extra agents, crews or subagents.

## Verification

All three gates go in the review handoff.

1. **Parsed item equality**, exactly as gate 1 of the pass 1 brief: `syn` over the original and every new file, `use` and `mod` items ignored, `impl` members keyed `Type::method` (with the trait), only the approved `pub(super)` prefixes normalized, every original item matched once with zero differences, and the five negative controls run against temporary copies. Keep the script out of the repo.
2. **Test count.** Record `cargo test --locked -p runner-app --profile ci` passed and ignored counts before moving anything; they must be identical after. Use `set -o pipefail` or check Cargo's exit status: a green `grep` proves nothing.
3. **Checks:** workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`--features updater`), `cargo fmt --all --check`, `git diff --check`. Record the exact commands and exit codes. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

Also report each new file's line count, every `pub(super)` as `file::item` with a count, every type the placement rule moved to `mod.rs`, and the imports you curated by hand.

## Review and authorization

The coder owns the move and the gates. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against this brief, must-fix first with file:line pointers: a gate 1 re-run of its own, no widened visibility, external paths unchanged, `#[cfg(test)]` attributes intact. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main` with the subject `chore(ui): split ui/field.rs into a module directory`, pushing `chore/582-split-field`, and opening a PR against main titled the same, whose body says `Refs #582` (not `Closes`: more passes follow) and summarizes the gates. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows, checking the exit status of `gh pr checks <n> --watch`. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, the final cut table, gate results with exit codes, CI result, the reviewer's verdict, and what Jason should smoke-test (typing, selection drag, IME, undo and redo, and a Browse path field in Start Chat and Settings). Then stand by.

## Non-goals

Every other file in #582's list, behavior, test logic, and docs beyond this brief.
