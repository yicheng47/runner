// Pin semantics for the on-screen chat group (spec 34 follow-up): the
// active split group behaves as ONE pinned unit in the sidebar. Three
// concerns, all pure so they unit-test like paneLayout's ops:
//   - a chat added to a group with a pinned member inherits the pin,
//   - pin/unpin on a member fans out to every member,
//   - the CHAT list renders members as one contiguous block.
// Group membership stays in the frontend layout store; the recent-direct
// sort SQL is intentionally group-blind.

interface PinnableRow {
  session_id: string;
  pinned: boolean;
}

export function pinnedSessionIds(
  rows: readonly PinnableRow[],
): Set<string> {
  return new Set(rows.filter((r) => r.pinned).map((r) => r.session_id));
}

/**
 * Whether a chat just added to the active group should inherit a pin:
 * true when any existing member is pinned and the new chat isn't
 * already. Never asks to unpin — adding to an unpinned group leaves the
 * new chat's pin state alone.
 */
export function shouldInheritPinOnAdd(
  existingMemberIds: readonly string[],
  pinnedIds: ReadonlySet<string>,
  newSessionId: string,
): boolean {
  return (
    !pinnedIds.has(newSessionId) &&
    existingMemberIds.some(
      (id) => id !== newSessionId && pinnedIds.has(id),
    )
  );
}

/**
 * Sessions to write when toggling a chat's pin to `nextPinned`. A member
 * of the active group drags every member with it (skipping ones already
 * in the target state, so drifted pre-fix groups converge); a chat
 * outside the active group keeps single-session behavior.
 */
export function groupPinTargets(
  toggledSessionId: string,
  activeGroupSessionIds: readonly string[],
  pinnedIds: ReadonlySet<string>,
  nextPinned: boolean,
): string[] {
  if (!activeGroupSessionIds.includes(toggledSessionId)) {
    return [toggledSessionId];
  }
  return activeGroupSessionIds.filter(
    (id) => pinnedIds.has(id) !== nextPinned,
  );
}

/**
 * Reorder the recent-direct rows so active-group members render as one
 * contiguous block in pane order, anchored where the group's
 * highest-sorted member already sits. Non-members keep their relative
 * order, and because the anchor is the best-sorted member, a fully
 * unpinned group can never float up into the pinned cluster. Returns
 * the input unchanged when fewer than two members are in the list.
 */
export function clusterActiveGroupRows<T extends { session_id: string }>(
  rows: readonly T[],
  activeGroupSessionIds: readonly string[],
): readonly T[] {
  const memberIds = new Set(activeGroupSessionIds);
  const byId = new Map(rows.map((r) => [r.session_id, r]));
  const block = activeGroupSessionIds
    .map((id) => byId.get(id))
    .filter((r): r is T => r !== undefined);
  if (block.length < 2) return rows;
  const anchor = rows.findIndex((r) => memberIds.has(r.session_id));
  const out: T[] = [];
  rows.forEach((r, i) => {
    if (i === anchor) out.push(...block);
    else if (!memberIds.has(r.session_id)) out.push(r);
  });
  return out;
}
