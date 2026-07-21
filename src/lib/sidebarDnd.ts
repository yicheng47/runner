import type { NodeRow } from "./api";

type SidebarOrderNode = Pick<
  NodeRow,
  "id" | "parent_id" | "type" | "pinned_position"
>;

export function orderedVisibleNodeIdsAfterDrop(
  targetIds: readonly string[],
  draggedId: string,
  requestedIndex: number,
): string[] {
  const remaining = targetIds.filter((id) => id !== draggedId);
  const index = Math.max(0, Math.min(requestedIndex, remaining.length));
  remaining.splice(index, 0, draggedId);
  return remaining;
}

export function completeUnpinnedScopeOrder(
  nodes: readonly SidebarOrderNode[],
  parentId: string | null,
  draggedId: string,
  orderedVisibleIds: readonly string[],
): string[] {
  const pinnedIds = new Set(
    nodes
      .filter((node) => node.pinned_position !== null)
      .map((node) => node.id),
  );
  const visible = orderedVisibleIds.filter((id) => !pinnedIds.has(id));
  const visibleSet = new Set(visible);
  const siblings = nodes.filter(
    (node) =>
      node.parent_id === parentId &&
      node.id !== draggedId &&
      node.pinned_position === null,
  );
  // Root leaf moves deliberately keep project rows ahead of the leaf section;
  // project-only moves preserve the existing interleaved slots below.
  const projectsFirst =
    parentId === null
      ? siblings
          .filter((node) => node.type === "project")
          .map((node) => node.id)
      : [];
  const hidden = siblings
    .filter((node) => !(parentId === null && node.type === "project"))
    .map((node) => node.id)
    .filter((id) => !visibleSet.has(id));
  return [...projectsFirst, ...visible, ...hidden];
}

export function orderedPinnedNodeIdsAfterDrop(
  nodes: readonly SidebarOrderNode[],
  visiblePinnedIds: readonly string[],
  draggedId: string,
  requestedIndex: number,
): string[] {
  const allPinnedIds = nodes
    .filter((node) => node.pinned_position !== null)
    .sort(
      (a, b) =>
        (a.pinned_position ?? 0) - (b.pinned_position ?? 0),
    )
    .map((node) => node.id);
  const pinnedSet = new Set(allPinnedIds);
  const visible = visiblePinnedIds.filter(
    (id, index) => pinnedSet.has(id) && visiblePinnedIds.indexOf(id) === index,
  );
  const visibleSet = new Set(visible);
  if (!visibleSet.has(draggedId)) return allPinnedIds;
  const reorderedVisible = orderedVisibleNodeIdsAfterDrop(
    visible,
    draggedId,
    requestedIndex,
  );
  let visibleIndex = 0;
  return allPinnedIds.map((id) =>
    visibleSet.has(id) ? reorderedVisible[visibleIndex++] : id,
  );
}

export function orderedRootNodeIdsAfterProjectDrop(
  nodes: readonly SidebarOrderNode[],
  draggedId: string,
  requestedIndex: number,
): string[] {
  const rootNodes = nodes.filter(
    (node) => node.parent_id === null && node.pinned_position === null,
  );
  const projectIds = rootNodes
    .filter((node) => node.type === "project" && node.id !== draggedId)
    .map((node) => node.id);
  const index = Math.max(0, Math.min(requestedIndex, projectIds.length));
  projectIds.splice(index, 0, draggedId);

  // Both origin sections share this root scope, so only replace project
  // slots instead of rebuilding from the filtered PROJECTS list.
  let projectIndex = 0;
  return rootNodes.map((node) =>
    node.type === "project" ? projectIds[projectIndex++] : node.id,
  );
}
