import { Columns2, Columns3, MessageSquare, MoreHorizontal, Pin } from "lucide-react";

import type { DirectSessionEntry } from "../lib/api";
import { derivedChatTabTitle } from "../lib/chatTabs";
import { findLeaf, leaves, type PaneLayout } from "../lib/paneLayout";

export const CHAT_TAB_DRAG_TYPE = "application/x-runner-chat-tab";

export function ChatTabGroup({
  layout,
  members,
  active,
  onActivate,
  onContextMenu,
  onDragStart,
  onDragEnd,
  dragging,
}: {
  layout: PaneLayout;
  members: DirectSessionEntry[];
  active: boolean;
  onActivate: (session: DirectSessionEntry) => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  onDragStart?: (tabId: string) => void;
  onDragEnd?: () => void;
  dragging?: boolean;
}) {
  const focused = findLeaf(layout.root, layout.focusedPaneId)?.sessionId;
  const target = members.find((member) => member.session_id === focused) ?? members[0];
  if (!target) return null;

  const name = layout.name ?? derivedChatTabTitle(members);
  const pinned = members.length > 0 && members.every((member) => member.pinned);
  const paneCount = leaves(layout.root).length;
  const SplitIcon = paneCount >= 3 ? Columns3 : Columns2;

  return (
    <div
      draggable={layout.id.length > 0}
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData(CHAT_TAB_DRAG_TYPE, layout.id);
        onDragStart?.(layout.id);
      }}
      onDragEnd={onDragEnd}
      onContextMenu={(event) => {
        event.preventDefault();
        onContextMenu({ x: event.clientX, y: event.clientY });
      }}
      className={`group relative flex items-center gap-1.5 rounded border px-2.5 py-1.5 text-xs transition-colors transition-opacity ${
        dragging ? "opacity-40" : ""
      } ${
        active
          ? "border-sidebar-selected-border bg-sidebar-selected text-fg"
          : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
      }`}
    >
      <button
        type="button"
        onClick={() => onActivate(target)}
        title={name}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
      >
        {pinned ? (
          <Pin aria-hidden className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3" />
        ) : null}
        {paneCount > 1 ? (
          <SplitIcon aria-hidden className={`h-3 w-3 shrink-0 ${active ? "text-accent" : "text-fg-2"}`} />
        ) : (
          <MessageSquare aria-hidden className={`h-3 w-3 shrink-0 ${active ? "text-accent" : "text-fg-2"}`} />
        )}
        <span className={`truncate ${active ? "font-semibold" : ""}`}>{name}</span>
        {paneCount > 1 ? (
          <span className="ml-auto shrink-0 rounded bg-line px-1 py-px text-[9px] text-fg-3">
            {paneCount}
          </span>
        ) : null}
      </button>
      <button
        type="button"
        onClick={(event) => onContextMenu({ x: event.clientX, y: event.clientY })}
        title="More actions"
        aria-label="More actions"
        className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 transition-opacity hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
      >
        <MoreHorizontal aria-hidden className="h-3 w-3" />
      </button>
    </div>
  );
}
