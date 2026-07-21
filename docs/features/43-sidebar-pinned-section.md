# 43 — Sidebar pinned section

> Tracking issue: [#317](https://github.com/yicheng47/runner/issues/317)

## Motivation

Pinned chat tabs and pinned missions need one predictable home. PINNED is a derived view over the node tree that collects the things a user wants to keep at hand without changing their project or root membership.

This spec reflects the node-tree model shipped in #318 and supersedes the pre-remodel version of this document. `nodes.pinned_position` is non-NULL for pinned nodes and orders the global PINNED section; pinning remains an overlay on the node's existing `parent_id`.

## Scope

- Render a PINNED section at the top of the sidebar nav containing every pinned tab and mission, whether its original parent is a project or root, ordered by `pinned_position`.
- Hide PINNED entirely when no nodes are pinned.
- Remove pinned rows from their origin project or root scopes. Origin scopes render only unpinned rows in `position` order; the interim pinned-first ordering inside each container is retired.
- Preserve each row's type-specific rendering, click target, context menu, pin action, and attention state.
- Support drag-to-reorder inside PINNED through `node_reorder_pinned(ordered_ids)`. The payload must contain every currently pinned node exactly once, and the backend rewrites `pinned_position` to match it.
- While a pinned row is dragged, other pinned rows act as reorder positions rather than project containers or parent-scope targets.
- Keep pin and unpin on the context menu. Dragging into or out of PINNED does not pin, unpin, or reparent a node.
- Keep `parent_id` unchanged while a node is pinned and treat `position` as dormant. Unpinning appends the node to the end of its original parent scope by rewriting `position` to the visible scope's maximum plus one.
- Make every parent-scope reorder and complete-set validation operate on unpinned members only. This includes the full-root payload used by project reorder, so a pinned root leaf neither invalidates nor participates in a root project reorder.

## Out of scope

- Pinning or unpinning by dragging into or out of PINNED.
- Changing `parent_id` or the meaning of parent-scoped `position` while pinning.
- Merging mission and tab domain models.

## Implementation

1. Partition resolved pinned rows into a global PINNED view and omit them from project/root views.
2. Add the repository operation, Tauri command, and frontend API for `node_reorder_pinned` with complete pinned-set validation.
3. Reuse the sidebar reorder-position drag pattern for PINNED and submit the complete pinned order.
4. Exclude pinned nodes from backend parent-scope validation and every frontend parent-scope `ordered_ids` construction.
5. On unpin, recompute the node's parent-scoped `position` at the visible end rather than restoring its dormant value.

## Verification

- [ ] Pin a root chat tab, a project-nested chat tab, and a mission; all appear once in PINNED in `pinned_position` order and disappear from their origin scopes.
- [ ] Drag pinned rows to reorder them; reload and confirm the order persists.
- [ ] Pinned rows cannot be dropped into project/root scopes, and unpinned rows cannot be dropped into PINNED.
- [ ] Pin a project-nested tab, add and reorder siblings, then unpin it; the tab reappears at the end of that project.
- [ ] Pin a root tab, reorder projects, and confirm the project reorder succeeds without moving or unpinning the tab.
- [ ] Unpin the final pinned row and confirm the PINNED section disappears.
- [ ] Row click targets, context menus, working/unread attention, and status indicators behave unchanged in PINNED.
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `pnpm test`, `pnpm exec tsc --noEmit`, and `pnpm run lint` pass.
