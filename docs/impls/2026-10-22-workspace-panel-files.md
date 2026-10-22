# Workspace panel tabs and read-only files implementation plan

> For agentic workers: use subagent-driven-development or executing-plans. Work only in `.worktrees/codex-634-panel-files`; preserve the unrelated staged avatar work in the root checkout.

**Goal:** Bring screenshot-inspired closable side-panel tabs, a lazy workspace tree and read-only previews to both direct chats and Teams.

**Architecture:** Reuse the existing chat panel and mission rail shells and splitters. A shared GPUI panel entity owns context-scoped tabs, directory expansion, loading/error state and file preview entities. A small read-only filesystem module runs on background executors. Existing mission Markdown rendering/parsing is reused, not replaced. Parent Info/Roles/Meta views remain authoritative. No daemon protocol or database migration is needed for this local read-only surface.

**Tech stack:** Rust, GPUI, current Markdown renderer, std filesystem, existing background executor and image support. No webview, new editor, or new package. Reuse the already-locked libc on Unix for opened-handle confinement checks.

**Spec:** `docs/features/634-file-preview-and-review.md`, narrowed by the user's approved 2026-10-22 screenshots and scope below. Git change status, diffs, review comments and browser embedding remain later work.

## Global constraints

- Existing Info, Roles and Meta functionality must remain accessible. Panel content tabs are distinct from chat Tabs and Panes.
- Each chat/mission owns its tabs; switching contexts must not show another workspace's tree or preview. Canonical file identities deduplicate tabs.
- Filesystem access is read-only, bounded and asynchronous. Resolve symbolic links and reject paths outside the selected workspace. Hide `.git`; never recursively enumerate a repository before expansion.
- Markdown, UTF-8 text/code/JSON and common raster images are internal previews. PDF/Office/video/binary/oversized files have explicit external-open fallbacks. No edit, upload, share or Git review buttons without implementation.
- A new-tab home offers Files and the existing terminal action. Preserve tab/preview scroll state across switches. Closing an active tab selects a deterministic neighbour; closing the last tab shows home.
- Support collapsed/expanded panel widths and existing drag splitters without changing main chat selection.
- Directory and preview refresh is explicit at minimum, plus background refresh while visible; stale async results cannot change another tab/context. Report any live-refresh boundary truthfully.
- Keep the running dev and installed Runner untouched unless the user asks to deploy/restart; native checks must use an isolated candidate and owned fixtures.

## File boundaries and contracts

`crates/runner-app/src/workspace_files.rs` (library module):
- `DirectoryEntry { path: PathBuf, name: String, is_dir: bool, is_symlink: bool, ignored: bool }`.
- `PreviewContent::{Markdown(String), Text(String), Image, Unsupported(String)}`.
- `FilePreview { path: PathBuf, bytes: u64, content: PreviewContent }`.
- `resolve_path(root: &Path, path: &Path) -> Result<PathBuf, String>` canonicalises root/target and rejects escapes.
- `read_directory(root: &Path, path: &Path) -> Result<Vec<DirectoryEntry>, String>` lazy, sorted directories first, excludes `.git`.
- `read_preview(root: &Path, path: &Path) -> Result<FilePreview, String>` bounded regular-file reading; no FIFO/device blocking.

`crates/runner-app/src/file_preview.rs` (binary module):
- `FilePreviewView::new(root: PathBuf, preview: FilePreview, cx: &mut Context<Self>) -> Self`.
- `FilePreviewView::replace(&mut self, preview: FilePreview, cx: &mut Context<Self>)` preserves scroll where reasonable.
- `Render` owns content only; parent provides filename toolbar, reload and external-open controls. Reuse Markdown parser/rendering; handle local relative images safely; provide text selection/copy, text line numbers and image zoom/fit. No external network image loads.

`crates/runner-app/src/workspace_panel/{mod.rs,state.rs,view.rs,tests.rs}` (binary modules):
- `PanelBuiltin::{Info, Roles, Meta}` identifies unchanged host views.
- Shared entity handles builtin, Files, New and canonical File tabs, current root, per-directory loading/expanded states and per-file preview entities.
- Parent observes and subscribes to panel events (terminal request, collapse) to use the existing actions; parent renders the builtin view when selected, otherwise the shared panel content.
- Header and body are owned by one panel state; no parallel shadow state in settings or daemon.

## Task 1 — bounded filesystem access
- [x] Add tests for ordering, nested expansion, hidden metadata, path traversal/symlink escapes, deleted/unsupported/oversized/binary files and Markdown/text/image classification.
- [x] Run failing tests, implement the adapter and run passing tests.
- [x] Expose through `lib.rs`; no global config or protocol changes.

## Task 2 — reusable preview entity
- [x] Reuse existing Markdown facilities and add tests for text rows/line numbers, local image resolution and path boundaries.
- [x] Implement Markdown, text and image previews with independent scroll/selection state.
- [x] Implement clear unsupported/error and external-link behaviour; no fake editing.

## Task 3 — panel tabs and tree
- [x] Test tab IDs/deduplication, close-neighbour rules, last-tab home, context roots, async generation guards and tree expansion.
- [x] Implement horizontal overflow tabs, new-tab home, Files tree, loading/error/empty states, refresh and selected-file rows.
- [x] Add expanded/normal width behaviour and parent events; retain a file's preview entity on switching tabs.

## Task 4 — direct chat and Team integration
- [x] Replace only side-panel headers/content routing; preserve existing builtin renderers and collapse/resize behaviour.
- [x] Resolve roots from session cwd, project path and default directory; Team uses mission cwd. Missing directories get an honest empty/error state.
- [x] Keep context state separate and notify cached parent regions when tab selection changes.
- [x] Add integration/layout checks: original builtin access, Files/new/file switch and close, narrow/expanded widths and chat/mission context changes.
- [x] Update English/Chinese copy and guide pages; state readonly/format boundaries.

## Validation and delivery
- [x] `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` (new and existing native UI tests).
- [x] `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` and formatting/diff checks.
- [x] Build app + CLI candidate. Select relevant MIS-FEED / workspace side-panel live checks plus feature-specific file/tab checks, not a full regression. Record unavailable native/platform checks explicitly.
- [x] Independent read-only code review focused on context isolation, symlink/size limits, race guards and tab lifetime.
- [x] Leave implementation in its own branch unless asked to commit/PR/merge/restart. No release or changes to the installed app.

Native results and remaining gaps are retained in `docs/tests/2026-10-22-workspace-panel-files.md`; only its identified v4 cases passed. This is not a full native regression pass.
