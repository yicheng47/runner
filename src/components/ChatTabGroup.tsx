// Sidebar accordion for a multi-pane tab (impl 0023). A tab with ≥2 member
// chats renders as a disclosure header (chevron · split icon · name · pin
// marker) over a left rail wrapping the member rows. Single-member and
// un-tabbed chats never reach here — they stay flat `SessionRow` leaves.
//
// The members REUSE `SessionRow` so rename, pin, status dot, context menu and
// the focused-pane accent bar keep working by construction; only the header +
// rail are new chrome. Collapse is per-tab and persisted (the chevron toggles
// it on any tab, active or not). The active on-screen tab carries an accent
// rail and marks its focused member with the selected fill.

import { useEffect, useRef, useState } from "react";

import {
  ChevronDown,
  ChevronRight,
  Columns2,
  Columns3,
  Pin,
} from "lucide-react";

import type { DirectSessionEntry } from "../lib/api";
import {
  activatePaneLayoutForSession,
  findLeaf,
  setGroupName,
  setTabCollapsed,
  type PaneLayout,
} from "../lib/paneLayout";
import type { SessionActivityState } from "../lib/types";
import { SessionRow } from "./Sidebar";

function memberLabel(session: DirectSessionEntry): string {
  return (
    session.title ??
    (session.handle ? `@${session.handle}` : session.display_name)
  );
}

export function ChatTabGroup({
  layout,
  members,
  active,
  focusedSessionId,
  activity,
  renamingId,
  onOpenChat,
  onActivateTab,
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
  onMemberContextMenu: (
    session: DirectSessionEntry,
    anchor: { x: number; y: number },
  ) => void;
  onMemberRenameSubmit: (sessionId: string, title: string | null) => void;
  onMemberRenameCancel: () => void;
}) {
  const collapsed = !!layout.collapsed;
  const [renaming, setRenaming] = useState(false);

  const paneFocusedSessionId =
    findLeaf(layout.root, layout.focusedPaneId)?.sessionId ?? null;
  const nameSource =
    members.find((m) => m.session_id === paneFocusedSessionId) ?? members[0];
  const name = layout.name ?? memberLabel(nameSource);
  const pinned = members.every((m) => m.pinned);

  const Chevron = collapsed ? ChevronRight : ChevronDown;
  const SplitIcon = members.length >= 3 ? Columns3 : Columns2;

  const toggleCollapse = () => {
    setTabCollapsed(members[0].session_id, !(layout.collapsed ?? false));
  };

  const submitRename = (raw: string) => {
    const next = raw.trim() || null;
    setRenaming(false);
    if (next === layout.name) return;
    // setGroupName writes the active tab, so adopt this one first — renaming a
    // background tab makes it current, which is the sensible read of "edit it".
    activatePaneLayoutForSession(members[0].session_id);
    setGroupName(next);
  };

  return (
    <div className="flex flex-col gap-0.5">
      <div className="group relative flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-colors hover:bg-sidebar-selected/60">
        <button
          type="button"
          onClick={toggleCollapse}
          title={collapsed ? "Expand tab" : "Collapse tab"}
          aria-label={collapsed ? "Expand tab" : "Collapse tab"}
          aria-expanded={!collapsed}
          className="flex shrink-0 cursor-pointer items-center gap-1.5 text-fg-2 hover:text-fg"
        >
          <Chevron aria-hidden className="h-3 w-3" />
          <SplitIcon
            aria-hidden
            className={`h-3 w-3 ${active ? "text-accent" : "text-fg-2"}`}
          />
        </button>
        {renaming ? (
          <TabNameInput
            initial={layout.name ?? ""}
            placeholder={name}
            onSubmit={submitRename}
            onCancel={() => setRenaming(false)}
          />
        ) : (
          <button
            type="button"
            onClick={() => onActivateTab(nameSource)}
            onDoubleClick={() => setRenaming(true)}
            title={name}
            className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
          >
            {pinned ? (
              <Pin
                aria-hidden
                className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3"
              />
            ) : null}
            <span
              className={`truncate text-fg ${active ? "font-semibold" : ""}`}
            >
              {name}
            </span>
          </button>
        )}
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

function TabNameInput({
  initial,
  placeholder,
  onSubmit,
  onCancel,
}: {
  initial: string;
  placeholder: string;
  onSubmit: (next: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);
  return (
    <input
      ref={inputRef}
      value={draft}
      placeholder={placeholder}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onSubmit(draft);
        } else if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
      onBlur={() => {
        if (draft.trim() === initial.trim()) onCancel();
        else onSubmit(draft);
      }}
      className="min-w-0 flex-1 bg-transparent text-xs font-semibold text-fg outline-none placeholder:text-fg-3"
    />
  );
}
