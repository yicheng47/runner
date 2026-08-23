# Start Project modal

Tracking issue: [#383](https://github.com/yicheng47/runner/issues/383). Status: shipped (#383 closed 2026-08-01; native `open_project_modal` in `surfaces/sidebar.rs`).

## Motivation

The sidebar's "Add project" (+) jumps straight into a native directory picker and silently creates the project (`Sidebar.tsx` `addProject`): name is hardcoded to the directory basename, and the picker starts wherever macOS last left it. There is no chance to set the name at creation time (rename-after is the only path), the configured default working directory (`settings.defaultWorkingDir`) is ignored, and the flow is asymmetric with its sibling actions — starting a chat or a mission both open a modal (`StartChatModal`, `StartMissionModal`). Project creation should mirror them.

## Scope

A `StartProjectModal` component mirroring the sibling start modals (same `Modal` shell, buttons, field styling), opened by the sidebar's Add project (+) in place of the direct picker call.

Fields:

- **Directory** — text input plus a Browse button that opens the native picker (`openDialog({ directory: true, defaultPath })`). Prefilled with `readDefaultWorkingDir()` when the setting is non-empty, else empty. Browse starts from the current field value, falling back to the default working dir.
- **Name** — prefilled with the basename of the directory field and follows directory changes (pick a new folder → name updates to its basename) until the user edits the name manually; after a manual edit the name sticks.

Create calls the existing `api.project.create(name, cwd)` and keeps today's post-create behavior: Projects section opens, the new project becomes active, the tree refreshes. Create is disabled while either field is empty. Enter submits, Esc closes — same keyboard contract as the sibling modals.

## Non-Goals

- Backend changes — `project_create` ships as-is; no directory-exists or duplicate checks beyond what it already does.
- Any change to project rename or set-cwd flows.

## Implementation Notes

- `src/components/StartProjectModal.tsx` — new component; `src/components/Sidebar.tsx` `addProject` becomes "open modal", post-create logic moves into the modal's `onCreated` callback.
- Naming collision: `StartProjectModal.test.tsx` already exists but tests project-*scoped* chat/mission modals. Rename that file (e.g. `projectScopedStartModals.test.tsx`) so the new component's test can take the canonical name.

## Verification

- Vitest: name follows directory until manually edited, then sticks; prefill from `readDefaultWorkingDir()`; create disabled on empty fields.
- Manual: + opens the modal with defaults populated; Browse starts at the default working dir; created project lands selected in an open Projects section.
