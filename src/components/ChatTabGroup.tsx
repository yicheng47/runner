// Sidebar accordion for a multi-pane tab (impl 0023). A tab with ≥2 member
// chats renders as a disclosure header (pin marker · split icon · name · a
// trailing collapse chevron) over a left rail wrapping the member rows.
// Single-member and un-tabbed chats never reach here — they stay flat
// `SessionRow` leaves.
//
// The members REUSE `SessionRow` so rename, pin, status dot, context menu and
// the focused-pane accent bar keep working by construction; only the header +
// rail are new chrome. Collapse is per-tab and persisted; clicking the row (or
// the trailing chevron) toggles it on any tab, active or not. The active
// on-screen tab carries an accent rail and marks its focused member with the
// selected fill.

import { ChevronDown, Columns2, Columns3, Pin } from "lucide-react";

import type { DirectSessionEntry } from "../lib/api";
import { derivedChatTabTitle } from "../lib/chatTabs";
import {
  findLeaf,
  setTabCollapsed,
  type PaneLayout,
} from "../lib/paneLayout";
import type { SessionActivityState } from "../lib/types";
import { SessionRow } from "./Sidebar";

export function ChatTabGroup({
  layout,
  members,
  active,
  focusedSessionId,
  activity,
  renamingId,
  onOpenChat,
  onActivateTab,
  onTabContextMenu,
  onMemberContextMenu,
  onMemberRenameSubmit,
  onMemberRenameCancel,
}: {
  layout: PaneLayout;
  /** Member rows in slot order; always ≥2 (see `buildChatListItems`). */
  members: DirectSessionEntry[];
  /** This tab owns the currently-open chat: accent rail + focused fill. */
  active: boolean;
  /** Focused pane's chat, or null when this tab isn't active. */
  focusedSessionId: string | null;
  activity: Record<string, SessionActivityState | undefined>;
  renamingId: string | null;
  /** Member-row click: focus that pane (may fill an empty on-screen pane). */
  onOpenChat: (session: DirectSessionEntry) => void;
  /** Header click: activate this tab, never fill another tab's empty pane. */
  onActivateTab: (session: DirectSessionEntry) => void;
  onTabContextMenu: (
    members: DirectSessionEntry[],
    anchor: { x: number; y: number },
  ) => void;
  onMemberContextMenu: (
    session: DirectSessionEntry,
    anchor: { x: number; y: number },
  ) => void;
  onMemberRenameSubmit: (sessionId: string, title: string | null) => void;
  onMemberRenameCancel: () => void;
}) {
  const collapsed = !!layout.collapsed;

  const paneFocusedSessionId =
    findLeaf(layout.root, layout.focusedPaneId)?.sessionId ?? null;
  const nameSource =
    members.find((m) => m.session_id === paneFocusedSessionId) ?? members[0];
  const name = layout.name ?? derivedChatTabTitle(members);
  const pinned = members.every((m) => m.pinned);

  const SplitIcon = members.length >= 3 ? Columns3 : Columns2;

  const toggleCollapse = () => {
    setTabCollapsed(members[0].session_id, !(layout.collapsed ?? false));
  };

  return (
    <div className="flex flex-col gap-0.5">
      <div
        onContextMenu={(e) => {
          e.preventDefault();
          onTabContextMenu(members, { x: e.clientX, y: e.clientY });
        }}
        className="group relative flex items-center gap-1.5 rounded-md border border-transparent px-2.5 py-1.5 text-xs transition-colors hover:border-sidebar-selected-border hover:bg-sidebar-selected/40"
      >
        <button
          type="button"
          onClick={() => {
            // The whole row toggles this tab's expand/collapse, so you
            // never need to hit the chevron. Opening a collapsed tab also
            // activates it; collapsing just hides its members in place.
            const willExpand = collapsed;
            toggleCollapse();
            if (willExpand) onActivateTab(nameSource);
          }}
          title={name}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
        >
          {pinned ? (
            <Pin
              aria-hidden
              className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3"
            />
          ) : null}
          <SplitIcon
            aria-hidden
            className={`h-3 w-3 shrink-0 ${active ? "text-accent" : "text-fg-2"}`}
          />
          <span className={`truncate text-fg ${active ? "font-semibold" : ""}`}>
            {name}
          </span>
        </button>
        {/* Disclosure chevron trails the title. A single glyph that
            rotates between states (down = open, right = collapsed) so it
            doesn't visually jump the way swapping two glyphs does. */}
        <button
          type="button"
          onClick={toggleCollapse}
          title={collapsed ? "Expand tab" : "Collapse tab"}
          aria-label={collapsed ? "Expand tab" : "Collapse tab"}
          aria-expanded={!collapsed}
          className="flex shrink-0 cursor-pointer items-center text-fg-2 hover:text-fg"
        >
          <ChevronDown
            aria-hidden
            className={`h-3 w-3 transition-transform ${
              collapsed ? "-rotate-90" : ""
            }`}
          />
        </button>
      </div>
      {collapsed ? null : (
        <div
          className={`flex flex-col gap-0.5 border-l-2 pl-[13px] ${
            active ? "border-accent/30" : "border-line"
          }`}
        >
          {members.map((member) => {
            const memberFocused =
              active && member.session_id === focusedSessionId;
            return (
              <SessionRow
                key={member.session_id}
                session={member}
                activity={activity[member.session_id]}
                selected={memberFocused}
                paneFocused={memberFocused}
                renaming={renamingId === member.session_id}
                onClick={() => onOpenChat(member)}
                onContextMenu={(anchor) => onMemberContextMenu(member, anchor)}
                onRenameSubmit={(title) =>
                  onMemberRenameSubmit(member.session_id, title)
                }
                onRenameCancel={onMemberRenameCancel}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}
