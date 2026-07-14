# 40 — Projects

> Tracking issue: [#292](https://github.com/yicheng47/runner/issues/292).

## Motivation

Runner has no project entity — cwd is metadata, not identity (stated outright in `docs/features/01-archived-tab.md:15`). Every chat and mission carries a working directory, but nothing binds the work you do in one repo into a durable home. As Runner is dogfooded across many repos at once (runner, quill, memory, alpha, …), the flat MISSION and CHAT lists scatter that work with no repo-scoped grouping — you re-pick the cwd every time and hunt for "the chats I had about quill" across an undifferentiated list.

A **Project** fixes this: a named container bound to a working directory. Open a project and new chats/missions default to its cwd and collect under it in the sidebar. This mirrors the Codex/Cursor "Projects" model — a Projects section whose rows are directories, with tasks nested underneath, and a `+` offering "Start from scratch" or "Use an existing folder."

This revives the shelved `docs/features/archive/17-sidebar-folders.md` (Arc-style folders grouping missions + chats, motivated verbatim by "which project does this belong to"), which was narrowed away when feature 38 scoped folders to chat tabs only and deferred "folders for missions."

## Scope

### In scope (v1)

- **`projects` table + FK.** A project is `(id, name, cwd, position, collapsed, created_at)`. Both `sessions` and `missions` gain a nullable `project_id` FK (`ON DELETE SET NULL`).
- **Project CRUD.** Create, rename, rebind cwd, collapse/expand, reorder, delete. New `commands/project.rs` + `repo/project.rs` mirroring the folder command shape; `api.project.*` on the frontend.
- **Codex-style create menu.** The PROJECTS section `+` opens a menu with **Start from scratch** (create a new directory under a picked parent, then bind it) and **Use an existing folder** (native folder picker → bind). Both reuse the already-wired Tauri dialog plugin (`WorkingDirField` / `openDialog`).
- **PROJECTS sidebar section.** A new collapsible section listing project rows (folder icon + name + collapse chevron + kebab). Each project expands to nest **both** its missions and its chats, reusing the existing `MissionRow` and chat-tab (`renderTabItems`) row components. Ungrouped work (project_id NULL) stays in the existing MISSION and CHAT sections (Codex's "Tasks").
- **cwd inheritance.** Creating a chat or mission inside a project pre-fills the Start modal's cwd from `projects.cwd` and stamps the new row's `project_id`. A "New chat / New mission in project" affordance lives on the project row.
- **Move in/out.** Assign an existing chat or mission to a project (or remove it) via the row context menu; reorder projects.
- **Non-destructive delete.** Deleting a project unbinds its chats/missions (`SET NULL` → they fall back to the ungrouped sections) and never touches the on-disk directory.

### Out of scope (deferred)

- **Folding chat folders (feature 38) into projects.** Folders and projects coexist in v1. Nesting chat folders inside a project (`folders.project_id`) is a follow-up.
- **Backfill / auto-grouping** existing sessions by their cwd into projects — v1 starts empty; a "group existing chats by cwd" importer is a follow-up.
- **Git/repo awareness** (branch, dirty state, remote) beyond the bound directory path.
- **Per-project defaults** (a default runner/crew, per-project settings/skills).
- **Nested projects.**

### Key decisions

1. **Projects are a new cwd-bound entity, not reused folders.** Folders (feature 38) are structurally chat-tab-only: `tabs.folder_id → folders ON DELETE RESTRICT`, `folder_delete` archives member tabs whose `layout.slots` are direct-chat session ids, and the `paneLayout.ts` store models chat-pane splits with no concept of missions or cwd. Projects nest missions too and must not inherit that RESTRICT-delete/archive lifecycle. A separate `projects` table keeps the two clean.
2. **`project_id` FK on both `missions` and `sessions`, `ON DELETE SET NULL`** — the shelved feature-17 shape. Delete = non-destructive unbind, deliberately unlike folder delete (which archives). Archiving stays a per-item action.
3. **cwd binding reuses the existing spawn path.** `projects.cwd` becomes a new default source ahead of `readDefaultWorkingDir()` in the Start modals; `spawn.rs` resolution is unchanged (`mission.cwd` / `runner.working_dir` stay authoritative once set). No spawn-path rewrite.
4. **Start from scratch vs Use an existing folder.** "Start from scratch" creates a new directory (picked parent + typed name) then binds; "Use an existing folder" binds an existing one. Both go through the dialog plugin already used by `WorkingDirField`, `StartChatModal`, and `StartMissionModal`.
5. **Placement + state.** PROJECTS is a new collapsible section (above MISSION/CHAT). Per-project collapse persists in the DB (`projects.collapsed`, mirroring `folders.collapsed`), not localStorage. Section-open state follows the existing `runner.sidebar.*` flag pattern.
6. **Global, per-window active project.** Projects are global like folders/tabs and sync across windows via the existing `chat/layout-changed`-style fanout. Which project is "active" (scoping new-chat cwd) is per-window view state.

## Implementation Phases

### Phase 1 — schema + commands

Migration `0011_projects.sql`: `projects` table + `project_id` on `sessions` and `missions` (`ON DELETE SET NULL`). `repo/project.rs`, `commands/project.rs` (`project_create/list/rename/set_cwd/set_collapsed/reorder/delete`), register in `lib.rs`. Extend the `MissionSummary` and `DirectSessionEntry` DTOs with `project_id` so the sidebar can group.

### Phase 2 — create + bind flow

The `+` create menu (Start from scratch / Use an existing folder), project creation with a bound cwd, `project_id` stamping when a chat/mission is started inside a project, and cwd pre-fill of the Start modals from `projects.cwd`.

### Phase 3 — sidebar PROJECTS section

Project rows nesting missions + chat tabs; collapse/expand; rename + rebind; move-to-project / remove-from-project context menus on chat and mission rows; reorder. Ungrouped items remain in the existing MISSION/CHAT sections.

### Phase 4 — polish + docs

Non-destructive delete flow + empty states, `docs/arch/arch.md` §3.6 update (add Project above the Window→Folder→Tab hierarchy), a coexistence note with feature 38, and `pnpm exec tsc --noEmit` / `pnpm run lint` / `pnpm test`.

## Open questions

- **Chat folders vs projects.** Do feature-38 chat folders eventually fold into projects (`folders.project_id`) or stay a separate chat-only grouping? Lean: coexist in v1, nest in a follow-up.
- **Active-project scoping.** Should focusing a project make ⌘T / ⌘N default new chats into it (cwd + assignment)? Lean: yes — that's the main ergonomic win.
- **Backfill.** Offer to group existing sessions by cwd into projects on first run? Lean: no for v1 (start empty), follow-up importer.
- **Delete depth.** Should project delete optionally also archive its chats/missions (like folder delete)? Lean: no — unbind only.

## Design first

Per the design-first workflow, mock the PROJECTS section, the `+` create menu (Start from scratch / Use an existing folder), and the project row states (collapsed / expanded with nested missions + chats, rename) in a feature-scoped `.pen` file before coding, using the Codex Projects layout as the reference.

## Verification (sketch)

- [ ] Create a project via "Use an existing folder" → it appears in PROJECTS bound to that directory; restart → project, cwd, and collapsed state restore.
- [ ] "Start from scratch" creates a new directory and binds the project to it.
- [ ] Start a chat and a mission from inside a project → both default to the project's cwd and nest under it; ungrouped work stays in MISSION/CHAT.
- [ ] Move an existing chat and mission into a project via context menu, then back out → they regroup correctly and survive restart.
- [ ] Delete a project → its chats/missions become ungrouped (nothing archived); the on-disk folder is untouched.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`, `cargo test --workspace` clean.
