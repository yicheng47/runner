import {
  Columns2,
  Columns3,
  MessageSquare,
} from "lucide-react";

import type { DirectSessionEntry } from "../lib/api";
import type { ChatAttentionState } from "../lib/chatAttention";
import { chatTabIsLive, derivedChatTabTitle } from "../lib/chatTabs";
import { findLeaf, leaves, type PaneLayout } from "../lib/paneLayout";
import { SidebarTabRow } from "./SidebarTabRow";

export function ChatTabGroup({
  layout,
  members,
  active,
  onActivate,
  onContextMenu,
  dragging,
  attention,
}: {
  layout: PaneLayout;
  members: DirectSessionEntry[];
  active: boolean;
  onActivate: (session: DirectSessionEntry) => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  dragging?: boolean;
  attention: ChatAttentionState;
}) {
  const focused = findLeaf(layout.root, layout.focusedPaneId)?.sessionId;
  const target = members.find((member) => member.session_id === focused) ?? members[0];
  if (!target) return null;

  const name = layout.name ?? derivedChatTabTitle(members);
  const pinned = members.length > 0 && members.every((member) => member.pinned);
  const live = chatTabIsLive(members);
  const paneCount = leaves(layout.root).length;
  const SplitIcon = paneCount >= 3 ? Columns3 : Columns2;

  return (
    <SidebarTabRow
      dragging={dragging}
      selected={active}
      label={name}
      icon={paneCount > 1 ? SplitIcon : MessageSquare}
      iconActive={live}
      pinned={pinned}
      attention={attention}
      onClick={() => onActivate(target)}
      onContextMenu={onContextMenu}
      title={name}
    />
  );
}
