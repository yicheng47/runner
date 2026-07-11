// Static keymap registry — impl 0025 (feature #257 v1). The single
// documented list of Runner's keyboard shortcuts, rendered read-only
// by the Settings → Keyboard shortcuts pane.
//
// v1 is presentation-only: handlers keep their hardcoded keys and this
// registry documents them. Each handler carries a one-line comment
// pointing back here so edits don't drift silently. Rebinding
// (capture UI, conflict detection, persistence, handler indirection)
// is a designed follow-up, deliberately out of v1 scope.

export type KeymapScope = "global" | "chat-split" | "mission";

export interface KeymapEntry {
  id: string;
  title: string;
  description: string;
  /** Rendered as mono key chips, e.g. "⌘K". */
  keys: string[];
  scope: KeymapScope;
}

export const KEYMAP: readonly KeymapEntry[] = [
  {
    // Handler: File → New Window menu accelerator (src-tauri, impl 0018).
    id: "new-window",
    title: "New window",
    description: "Open another Runner window.",
    keys: ["⌘N"],
    scope: "global",
  },
  {
    // Handler: Sidebar.tsx ⌘T listener.
    id: "new-chat",
    title: "New chat",
    description: "Start a chat in a new tab.",
    keys: ["⌘T"],
    scope: "global",
  },
  {
    // Handler: Sidebar.tsx ⌘K listener.
    id: "command-palette",
    title: "Command palette",
    description: "Search missions, chats, runners, and crews.",
    keys: ["⌘K"],
    scope: "global",
  },
  {
    // Handler: AppShell.tsx ⌘S listener. ⌘\ still works as a legacy
    // alias picked up during development; it is not rendered as a chip.
    id: "toggle-sidebar",
    title: "Toggle sidebar",
    description: "Collapse or expand the app sidebar.",
    keys: ["⌘S"],
    scope: "global",
  },
  {
    // Handler: App.tsx SettingsShortcut.
    id: "open-settings",
    title: "Open settings",
    description: "Open this settings page.",
    keys: ["⌘,"],
    scope: "global",
  },
  {
    // Handler: Sidebar.tsx sidebarNavigationDirectionFromKey.
    id: "page-navigation",
    title: "Previous / next page",
    description: "Step back and forward through recently viewed missions and chats.",
    keys: ["⇧⌘[", "⇧⌘]"],
    scope: "global",
  },
  {
    // Handler: App.tsx zoom listener.
    id: "app-zoom",
    title: "Zoom",
    description: "Scale the whole app up, down, or back to 100%.",
    keys: ["⌘+", "⌘−", "⌘0"],
    scope: "global",
  },
  {
    // Handler: RunnerChat.tsx cyclePaneFocus listener.
    id: "pane-focus",
    title: "Focus previous / next pane",
    description: "Cycle focus across panes while a chat is split.",
    keys: ["⌘[", "⌘]"],
    scope: "chat-split",
  },
  {
    // Handler: RunnerChat.tsx closeFocusedPane listener.
    id: "close-pane",
    title: "Close pane",
    description: "Collapse the focused pane while a chat is split.",
    keys: ["⌘W"],
    scope: "chat-split",
  },
  {
    // Handler: MissionWorkspace.tsx ⌘1–9 listener.
    id: "mission-feed",
    title: "Mission feed",
    description: "Jump to the event feed tab in a mission workspace.",
    keys: ["⌘1"],
    scope: "mission",
  },
  {
    // Handler: MissionWorkspace.tsx ⌘1–9 listener.
    id: "mission-slots",
    title: "Runner slots",
    description: "Jump to an open runner terminal tab in a mission workspace.",
    keys: ["⌘2–⌘9"],
    scope: "mission",
  },
];
