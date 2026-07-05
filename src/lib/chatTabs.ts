// Sidebar CHAT list grouping (impl 0023): partition the flat recent-direct
// rows into the tabs that own them. A tab with ≥2 members present in the list
// renders as an accordion group; every other chat stays a loose leaf row.
//
// This generalizes impl 0020's active-only `clusterActiveGroupRows`: instead
// of clustering just the on-screen tab, it walks the whole tab set so every
// multi-member tab (active or background) surfaces as one unit. Grouping keys
// on member count, not pane count — a split tab holding a single chat (its
// other panes empty) is a leaf, never an empty-child accordion (decision 1).

import { visibleSessionIds, type PaneLayout } from "./paneLayout";

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
