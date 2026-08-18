# 46 — Sidebar project reorder

Tracking: [#324](https://github.com/yicheng47/runner/issues/324)

## Motivation

Project rows in the PROJECTS section render in `nodes.position` order, which today is frozen at creation order — there is no way to move a project up or down. As projects became the primary grouping surface (feature 40, node tree 44), the list grew past the point where creation order matches working order: active projects end up buried under dormant ones.

The backend is already done. `repo::node::move_and_reorder` explicitly allows a `project` node to move within the root scope (`node.rs:361-362`), and the `node_move` command handles `NodeType::Project` in its post-move emit match. Only the frontend affordance is missing: project rows render as `ContainerDropRow` (a drop target for tabs and missions) but are never wrapped in `SortableNavRow`, so they cannot be dragged themselves.

## Scope

- Drag a project row up/down within the PROJECTS section to reorder it. Projects stay root-level — no nesting under folders or other projects (already enforced by `move_and_reorder`).
- Reuse the existing sidebar dnd-kit machinery: wrap project rows in `SortableNavRow` (or a project-specific equivalent) inside the section's existing `DndContext`, with drop indicators consistent with tab/mission drag.
- A project row keeps its current dual role: dragging the row itself reorders projects; dropping a tab/mission node onto it still reparents (the `ContainerDropRow` behavior). Disambiguate by the dragged node's type: while a `project` node is dragged, other project rows present as reorder positions, not containers.
- Out of scope: reordering via context menu or keyboard, nesting projects, and any change to the CHATS & MISSIONS section's ordering behavior.

### Design note — shared root scope

There are no per-section root nodes: root is `parent_id = NULL`, and the two sections are type filters over one shared scope with one position-space. `node_move` validates `ordered_ids` as the complete root scope, so a project reorder must submit **all** root node ids — the project rows in their new order interleaved with the non-project root nodes in their existing relative order. Keep the shared root; introducing explicit section nodes is a migration with no user-visible gain.

## Implementation Phases

### Phase 1 — frontend drag wiring

- Make project rows draggable inside the PROJECTS section: sortable wrapper with the node id, disabled while the project is being renamed.
- Extend the drag handlers (`handleRowDragStart/Over/End`) to recognize a dragged `project` node: compute the drop index among project rows only, suppress container-drop highlighting on project rows for project drags, and show the standard insertion marker.
- On drop, build the full root `ordered_ids` (reordered projects + untouched non-project root nodes in their current relative order) and call `moveNode(id, null, orderedIds)`.

### Phase 2 — validation

- `pnpm exec tsc --noEmit`, `pnpm run lint`.
- Manual pass over the Verification list in a dev build.

## Verification

- [ ] Drag a project above/below another: order persists across app restart.
- [ ] Reordering projects does not disturb CHATS & MISSIONS row order.
- [ ] Dropping a tab or mission onto a project row still moves it into the project.
- [ ] While dragging a project, project rows do not light up as containers.
- [ ] A project drag cannot land inside a folder, another project, or the CHATS & MISSIONS section.
- [ ] Rename-in-progress project rows are not draggable.

## Relevant Code

- `src/components/Sidebar.tsx` — PROJECTS section render (`projectNodes.map`, ~`:2108`), `SortableNavRow` (~`:2772`), `ContainerDropRow`, drag handlers, `moveNode` call (~`:1541`).
- `src-tauri/src/repo/node.rs:349-408` — `move_and_reorder` (already allows project-at-root; validates full-scope `ordered_ids`).
- `src-tauri/src/commands/node.rs:169-203` — `node_move` command.
- `src-tauri/migrations/0014_nodes.sql` — node tree schema; root = NULL parent.
