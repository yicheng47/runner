# 40 — Projects

> Tracking issue: [#292](https://github.com/yicheng47/runner/issues/292).

## Motivation

Runner has no project entity — cwd is metadata, not identity (stated outright in `docs/features/01-archived-tab.md:15`). Every chat and mission carries a working directory, but nothing binds the work you do in one repo into a durable home. As Runner is dogfooded across many repos at once (runner, quill, memory, alpha, …), the flat MISSION and CHAT lists scatter that work with no repo-scoped grouping — you re-pick the cwd every time and hunt for "the chats I had about quill" across an undifferentiated list.

A **Project** fixes this: a named container bound to a working directory. Open a project and new chats/missions default to its cwd and collect under it in the sidebar. This mirrors the Codex/Cursor projects model — a PROJECT section whose rows are directories, with tasks nested underneath, and a `+` for binding another directory as a project.

This revives the shelved `docs/features/archive/17-sidebar-folders.md` (Arc-style folders grouping missions + chats, motivated verbatim by "which project does this belong to"), which was narrowed away when feature 38 scoped folders to chat tabs only and deferred "folders for missions."

## Scope

### In scope (v1)

- **`projects` table + FK.** A project is `(id, name, cwd, position, created_at)`. Both `sessions` and `missions` gain a nullable `project_id` FK (`ON DELETE SET NULL`).
- **Project CRUD.** Create, rename, collapse/expand, and delete. A project's cwd is fixed when it is added, and v1 keeps projects in creation order. New `commands/project.rs` + `repo/project.rs` mirror the folder command shape; `api.project.*` is exposed on the frontend.
- **Single add-project action.** The PROJECT section `+` directly opens the native folder picker and binds the selected directory as a project. The picker can create a new folder when needed; adding a project does not scan or import existing sessions.
- **PROJECT sidebar section.** A new collapsible section listing project rows (code-folder icon + name + collapse chevron + kebab), visually distinct from plain-folder chat groups. Each project expands to nest **both** its missions and its chats, reusing the existing `MissionRow` and chat-tab (`renderTabItems`) row components. Ungrouped work (project_id NULL) stays in the existing MISSION and CHAT sections (Codex's "Tasks").
- **cwd inheritance.** Creating a chat or mission inside a project pre-fills the Start modal's cwd from `projects.cwd` and stamps the new row's `project_id`. A "New chat / New mission in project" affordance lives on the project row.
- **Move in/out.** Assign an existing chat or mission to a project (or remove it) via the row context menu.
- **Non-destructive delete.** Deleting a project unbinds its chats/missions (`SET NULL` → they fall back to the ungrouped sections) and never touches the on-disk directory.

### Out of scope (deferred)

- **Folding chat folders (feature 38) into projects.** Folders and projects coexist in v1. Nesting chat folders inside a project (`folders.project_id`) is a follow-up.
- **Backfill / auto-grouping** existing sessions by their cwd into projects — v1 starts empty; a "group existing chats by cwd" importer is a follow-up.
- **Git/repo awareness** (branch, dirty state, remote) beyond the bound directory path.
- **Per-project defaults** (a default runner/crew, per-project settings/skills).
- **Nested projects.**
- **Project drag reorder and cwd rebinding.** Neither is exposed in the v1 sidebar; drag-and-drop can add ordering in a follow-up.

### Key decisions

1. **Projects are a new cwd-bound entity, not reused folders.** Folders (feature 38) are structurally chat-tab-only: `tabs.folder_id → folders ON DELETE RESTRICT`, `folder_delete` archives member tabs whose `layout.slots` are direct-chat session ids, and the `paneLayout.ts` store models chat-pane splits with no concept of missions or cwd. Projects nest missions too and must not inherit that RESTRICT-delete/archive lifecycle. A separate `projects` table keeps the two clean.
2. **`project_id` FK on both `missions` and `sessions`, `ON DELETE SET NULL`** — the shelved feature-17 shape. Delete = non-destructive unbind, deliberately unlike folder delete (which archives). Archiving stays a per-item action.
3. **cwd binding reuses the existing spawn path.** `projects.cwd` becomes a new default source ahead of `readDefaultWorkingDir()` in the Start modals; `spawn.rs` resolution is unchanged (`mission.cwd` / `runner.working_dir` stay authoritative once set). No spawn-path rewrite.
4. **One add action, no import.** PROJECT `+` goes straight to the native folder picker. Selecting a directory binds it under its basename; users can create a directory in the picker when needed. The action never scans the directory or imports prior sessions.
5. **Placement + state.** PROJECT is a new collapsible section (above MISSION/CHAT). Project and chat-folder collapse are per-window sidebar state, not backend data. Section-open state follows the existing `runner.sidebar.*` flag pattern.
6. **Global, per-window active project.** Projects are global like folders/tabs and sync across windows via the existing `chat/layout-changed`-style fanout. Which project is "active" (scoping new-chat cwd) is per-window view state.

## Implementation Phases

### Phase 1 — schema + commands

Migration `0011_projects.sql`: `projects` table + `project_id` on `sessions` and `missions` (`ON DELETE SET NULL`), followed by `0012_drop_collapsed_view_state.sql` to remove the initially durable project/folder collapse columns. `repo/project.rs`, `commands/project.rs` (`project_create/list/rename/set_cwd/reorder/delete`), register in `lib.rs`. Extend the `MissionSummary` and `DirectSessionEntry` DTOs with `project_id` so the sidebar can group.

### Phase 2 — create + bind flow

The PROJECT `+` folder-picker action, project creation with a bound cwd, `project_id` stamping when a chat/mission is started inside a project, and cwd pre-fill of the Start modals from `projects.cwd`.

### Phase 3 — sidebar PROJECT section

Project rows nesting missions + chat tabs; collapse/expand; rename; move-to-project / remove-from-project context menus on chat and mission rows. Ungrouped items remain in the existing MISSION/CHAT sections.

### Phase 4 — polish + docs

Non-destructive delete flow + empty states, `docs/arch/arch.md` §3.6 update (add Project above the Window→Folder→Tab hierarchy), a coexistence note with feature 38, and `pnpm exec tsc --noEmit` / `pnpm run lint` / `pnpm test`.

## Open questions

- **Chat folders vs projects.** Do feature-38 chat folders eventually fold into projects (`folders.project_id`) or stay a separate chat-only grouping? Lean: coexist in v1, nest in a follow-up.
- **Active-project scoping.** Should focusing a project make ⌘T / ⌘N default new chats into it (cwd + assignment)? Lean: yes — that's the main ergonomic win.
- **Backfill.** Offer to group existing sessions by cwd into projects on first run? Lean: no for v1 (start empty), follow-up importer.
- **Delete depth.** Should project delete optionally also archive its chats/missions (like folder delete)? Lean: no — unbind only.

## Design first

Per the design-first workflow, mock the PROJECT section, its direct `+` folder-picker action, and the project row states (collapsed / expanded with nested missions + chats, rename) in a feature-scoped `.pen` file before coding, using the Codex Projects layout as the reference.

## Verification (sketch)

- [ ] Add a project via PROJECT `+` → the native folder picker opens, the selected directory appears as a project, and no existing sessions are imported; restart → project and cwd restore while collapse defaults to expanded in the new window.
- [ ] Start a chat and a mission from inside a project → both default to the project's cwd and nest under it; ungrouped work stays in MISSION/CHAT.
- [ ] Move an existing chat and mission into a project via context menu, then back out → they regroup correctly and survive restart.
- [ ] Delete a project → its chats/missions become ungrouped (nothing archived); the on-disk folder is untouched.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`, `cargo test --workspace` clean.
