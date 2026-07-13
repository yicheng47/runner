// Direct-chat tab labels plus the legacy impl 0023 partition helper retained
// for its pure tests. Feature 38's sidebar renders PaneLayout rows directly.

import { leaves, visibleSessionIds, type PaneLayout } from "./paneLayout";

export interface TabGroupItem<T> {
  kind: "group";
  layout: PaneLayout;
  /** Member rows in pane (slot) order. Always ≥2. */
  members: T[];
}

export interface LooseChatItem<T> {
  kind: "loose";
  row: T;
}

export type ChatListItem<T> = TabGroupItem<T> | LooseChatItem<T>;

export function chatTabMemberLabel(member: {
  title?: string | null;
  handle?: string | null;
  display_name: string;
}): string {
  return (
    member.title ?? (member.handle ? `@${member.handle}` : member.display_name)
  );
}

export function derivedChatTabTitle<
  T extends {
    title?: string | null;
    handle?: string | null;
    display_name: string;
  },
>(members: readonly T[]): string {
  return members.map(chatTabMemberLabel).join(" + ");
}

export interface OrderedChatTab {
  id: string;
  pinned: boolean;
}

export function chatTabArchiveLabel(layout: PaneLayout): string {
  return leaves(layout.root).length > 1 ? "Archive all" : "Archive";
}

export function isChatTabDropIndexAllowed(
  targetTabs: readonly OrderedChatTab[],
  draggedId: string,
  draggedPinned: boolean,
  index: number,
): boolean {
  const remaining = targetTabs.filter((tab) => tab.id !== draggedId);
  const pinnedCount = remaining.filter((tab) => tab.pinned).length;
  return draggedPinned ? index <= pinnedCount : index >= pinnedCount;
}

export function orderedChatTabIdsAfterDrop(
  targetTabs: readonly OrderedChatTab[],
  draggedId: string,
  draggedPinned: boolean,
  requestedIndex: number,
): string[] {
  const remaining = targetTabs.filter((tab) => tab.id !== draggedId);
  const pinnedCount = remaining.filter((tab) => tab.pinned).length;
  const index = draggedPinned
    ? Math.max(0, Math.min(requestedIndex, pinnedCount))
    : Math.max(pinnedCount, Math.min(requestedIndex, remaining.length));
  const orderedIds = remaining.map((tab) => tab.id);
  orderedIds.splice(index, 0, draggedId);
  return orderedIds;
}

/**
 * Order the CHAT list into groups and loose rows. Backend sort order is
 * preserved: each group is anchored where its best-sorted member already
 * sits (so a fully-unpinned group can't float into the pinned cluster), and
 * loose rows keep their relative position. A chat appears exactly once — a
 * session claimed by one tab is never re-listed under another.
 */
export function buildChatListItems<T extends { session_id: string }>(
  rows: readonly T[],
  layouts: readonly PaneLayout[],
): ChatListItem<T>[] {
  const byId = new Map(rows.map((r) => [r.session_id, r]));
  const claimed = new Set<string>();
  const groupsByAnchor = new Map<number, TabGroupItem<T>>();

  for (const layout of layouts) {
    const members = visibleSessionIds(layout.root)
      .filter((id) => !claimed.has(id))
      .map((id) => byId.get(id))
      .filter((row): row is T => row !== undefined);
    if (members.length < 2) continue;
    for (const member of members) claimed.add(member.session_id);
    const memberIds = new Set(members.map((m) => m.session_id));
    const anchor = rows.findIndex((r) => memberIds.has(r.session_id));
    groupsByAnchor.set(anchor, { kind: "group", layout, members });
  }

  const items: ChatListItem<T>[] = [];
  rows.forEach((row, index) => {
    const group = groupsByAnchor.get(index);
    if (group) {
      items.push(group);
    } else if (!claimed.has(row.session_id)) {
      items.push({ kind: "loose", row });
    }
  });
  return items;
}
