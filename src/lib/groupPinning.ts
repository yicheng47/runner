// Pin semantics for the on-screen chat group (spec 34 follow-up): the
// active split group behaves as ONE pinned unit in the sidebar. Two
// concerns, both pure so they unit-test like paneLayout's ops:
//   - a chat added to a group with a pinned member inherits the pin,
//   - pin/unpin on a member fans out to every member.
// Group membership stays in the frontend layout store; the recent-direct
// sort SQL is intentionally group-blind. Sidebar list ordering (which rows
// cluster into which tab) lives in `chatTabs.ts`.

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
