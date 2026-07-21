import type { NodeRow } from "./api";

type RootOrderNode = Pick<NodeRow, "id" | "parent_id" | "type">;

export function orderedRootNodeIdsAfterProjectDrop(
  nodes: readonly RootOrderNode[],
  draggedId: string,
  requestedIndex: number,
): string[] {
  const rootNodes = nodes.filter((node) => node.parent_id === null);
  const projectIds = rootNodes
    .filter((node) => node.type === "project" && node.id !== draggedId)
    .map((node) => node.id);
  const index = Math.max(0, Math.min(requestedIndex, projectIds.length));
  projectIds.splice(index, 0, draggedId);

  // Both sidebar sections share this root scope, so only replace project
  // slots instead of rebuilding the payload from the filtered PROJECTS list.
  let projectIndex = 0;
  return rootNodes.map((node) =>
    node.type === "project" ? projectIds[projectIndex++] : node.id,
  );
}
