# #1248 item 3 — Split the Files view modules

Jason approved the codex duo on 2026-11-12 for [#1248](https://github.com/yicheng47/runner/issues/1248) item 3, following the test extraction in #1276. The reviewer uses Codex `gpt-6-astra`, `xhigh`, standard speed. Work in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/chore-1248-files-modules`, branch `chore/1248-files-modules`, based on `origin/main` at `1178a7f6c0f03deb355dde78d95ff16efb8b664b`. Use `/tmp/1248-files-` for scratch artifacts.

## Problem and scope

`crates/runner-app/src/surfaces/files/view.rs` and `viewer.rs` each contain more than 2,000 lines and large `impl FilesView` blocks. The former combines roots, the tree, picker, watching, git status and layout; the latter combines file reads, tabs, find, Open in and rendering. Their names do not help a reader find the owner of a behavior.

Split these implementations into focused modules named for their responsibilities. Cover tree/roots and root selection, watcher/git-status updates, open-file reads/tabs, in-file find, and Open in/reveal. Keep composition/rendering with its responsibility or in a clearly named composition module. Replace or reduce the ambiguous `view.rs`/`viewer.rs` organization rather than leaving two large implementations behind. Choose the smallest useful set of modules; do not create a file for each method or introduce an abstraction layer.

Read the existing `files/mod.rs`, `view.rs`, `viewer.rs` and `tests.rs`, plus the adjacent model/helper modules when their callers are moved. There are already `tree.rs`, `tabs.rs`, `picker.rs`, `finder.rs`, `document.rs`, `links.rs` and `markdown.rs`; preserve their responsibilities and avoid confusing the file-name finder with the in-file find bar. Keep `files::FilesView` as the caller-facing re-export and retain existing external interfaces. Prefer a module tree that preserves private access; limit any necessary visibility changes to this Files subtree and explain them in review.

This is a mechanical production-code reorganization. Preserve method bodies and signatures except for necessary import/path/visibility adjustments. Keep `FilesView` and `Viewer` state ownership, field types, initialization, GPUI entity ownership, listener/subscription creation, callback capture and order, task lifetimes, request generation checks and runtime behavior unchanged. Preserve rendering expressions, element IDs, dimensions, focus/key handling, watcher sequencing, reload behavior, selection and all text. Moving whole methods and associated helpers is in scope; redesigning their implementation is not.

Audit relative paths, cfg gates, macros and any module/type-name-dependent behavior affected by relocation. Existing tests, assertions, fixtures and goldens stay unchanged except for necessary import/path adjustments. A test module whose containing implementation is renamed may acquire a new qualified name; account for such mappings explicitly rather than losing or weakening tests. No additional behavioral tests are needed for a pure move.

Out of scope: NativeRoot state extraction, Markdown parser consolidation, daemon launch/account refactors, broad production-file splitting elsewhere, new Files behavior, UI/design changes or cleanup of unrelated helpers. If behavior changes appear necessary, report the concrete obstacle instead of quietly expanding scope.

## Concurrency

At setup the open PRs are #1277 (Usage/footer styling) and #1278 (Codex archive state); neither touches `surfaces/files/`. Do not modify their worktrees. Recheck before editing and before final readiness; apply the repository catch-up rule if new changes on main overlap this work.

## Validation and review

Changed behavior: none. No live checks are required while the diff remains a mechanical module split with unchanged behavior and coverage. No app launch or real agent chats are part of validation. The reviewer must explicitly confirm this boundary in the clean verdict; any behavior change falls outside this brief.

Before testing, name the selected checks and why they cover the move:

- Compare the original and final implementations with a complete method/helper/type inventory and a move-aware diff. Account for every moved declaration and verify unchanged bodies after only formatting and necessary path/visibility normalization. The reviewer independently checks the candidate commit and caller wiring, especially private access and GPUI callbacks. Temporary audit scripts belong under `/tmp/1248-files-`, not in the product.
- Capture the `runner-app` test list before and after and account for every affected test, including any intentional module-path rename. Preserve ignored status and platform cfgs. Existing Files in-process tests cover root switching and stale replies, watcher lifetime/status updates, tree navigation, open/close/cycle tabs, reload and focus, find and stale results, Open in/reveal, and source/rendered-file behavior; these exercise the moved paths without native UI testing.
- Run `make verify` with this worktree's own `target/`, logging output and checking the exit code. It covers app tests and compilation/import/visibility errors, workspace Clippy, updater-feature Clippy on macOS, and formatting. Do not repeat passed tests without new changes, failures or unresolved concerns.
- Complete the required fresh CI on macOS, Windows, Linux x86_64 and Linux arm64. This covers platform-specific imports, reveal/open commands and cfg-gated paths. Preserve any failure and retry evidence; never weaken tests or update goldens to make the refactor pass.

Record the old-to-new module map, source-equivalence review, test-list accounting and actual validation results in the PR description and a concise `docs/tests/1248-files-modules.md`. Repair existing internal documentation references to relocated files where needed; user-guide changes are unnecessary for unchanged behavior.

Reference the PR as "Part of #1248" and leave the umbrella issue open. Use `no-behavior-change` and `risk-low`. The final independent review names the current head and the reason no live tests are required. The previous mission's passes are context, not validation of this candidate.

## Authorization

The approved codex duo may commit, push this branch and open one PR against `main`, then drive review and CI green. No merge is authorized for this mission. Fold this setup brief into the implementation history as the repository guide requires.
