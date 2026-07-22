// Full-page settings — impl 0025. `/settings/:pane?` renders its own
// two-column surface WITHOUT the app Sidebar: the settings sidebar
// replaces it in the same slot (same `--color-sidebar` fill and the
// same persisted width, deliberately, so the takeover reads as
// continuous). Per-window, like any route.
//
// Rendered by AppShell's takeover layer, NOT as the matched route
// element — the shell (and PersistentSurfaces' terminals) must stay
// mounted underneath. That means `useParams` can't see `:pane?` here;
// the pane comes from `matchPath` on the location instead.
//
// "Back to app" returns to the location the user came from — entry
// points pass it via navigation state (`{ from }`); direct loads fall
// back to `/`.

import { useMemo, useRef, useState, type ComponentType } from "react";
import { matchPath, useLocation, useNavigate } from "react-router-dom";
import {
  Archive,
  ArrowLeft,
  FileText,
  Info,
  Keyboard,
  MessageSquare,
  Plug,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  Sun,
  Terminal,
} from "lucide-react";

import { AboutPane } from "../components/settings/AboutPane";
import { AppearancePane } from "../components/settings/AppearancePane";
import { ArchivedPane } from "../components/settings/ArchivedPane";
import { ChatPane } from "../components/settings/ChatPane";
import { DiagnosticsPane } from "../components/settings/DiagnosticsPane";
import { GeneralPane } from "../components/settings/GeneralPane";
import { McpPane } from "../components/settings/McpPane";
import { ShortcutsPane } from "../components/settings/ShortcutsPane";
import { TerminalPane } from "../components/settings/TerminalPane";
import { UpdatesPane } from "../components/settings/UpdatesPane";
import { useResizableWidth } from "../hooks/useResizableWidth";

// Same domain and storage key as the app sidebar (Sidebar.tsx — keep
// in sync), so the settings sidebar opens at whatever width the app
// sidebar has and a drag on either surface carries over to the other.
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 240;
const STORAGE_WIDTH = "runner.sidebar.width";

type PaneKey =
  | "general"
  | "chat"
  | "appearance"
  | "terminal"
  | "shortcuts"
  | "mcp"
  | "updates"
  | "diagnostics"
  | "about"
  | "archived";

const PANES: Record<
  PaneKey,
  {
    label: string;
    icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
    render: () => React.ReactNode;
  }
> = {
  general: { label: "General", icon: SettingsIcon, render: () => <GeneralPane /> },
  chat: { label: "Chat", icon: MessageSquare, render: () => <ChatPane /> },
  appearance: { label: "Appearance", icon: Sun, render: () => <AppearancePane /> },
  terminal: { label: "Terminal", icon: Terminal, render: () => <TerminalPane /> },
  shortcuts: {
    label: "Keyboard shortcuts",
    icon: Keyboard,
    render: () => <ShortcutsPane />,
  },
  mcp: { label: "MCP", icon: Plug, render: () => <McpPane /> },
  updates: { label: "Updates", icon: RefreshCw, render: () => <UpdatesPane /> },
  diagnostics: {
    label: "Diagnostics",
    icon: FileText,
    render: () => <DiagnosticsPane />,
  },
  about: { label: "About", icon: Info, render: () => <AboutPane /> },
  archived: {
    label: "Archived chats & missions",
    icon: Archive,
    render: () => <ArchivedPane />,
  },
};

const NAV_GROUPS: { label: string; panes: PaneKey[] }[] = [
  {
    label: "App",
    panes: ["general", "chat", "appearance", "terminal", "shortcuts"],
  },
  { label: "Integrations", panes: ["mcp"] },
  { label: "System", panes: ["updates", "diagnostics", "about"] },
  { label: "Archived", panes: ["archived"] },
];

function isPaneKey(value: string | undefined): value is PaneKey {
  return value != null && value in PANES;
}

export default function SettingsPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const paneParam = matchPath("/settings/:pane?", location.pathname)?.params
    .pane;
  const pane: PaneKey = isPaneKey(paneParam) ? paneParam : "general";
  const [query, setQuery] = useState("");
  // Where "Back to app" returns. Captured once — pane switches
  // re-thread it through navigation state, but the ref survives even
  // if a switch drops the state.
  const fromRef = useRef<string>(
    (location.state as { from?: string } | null)?.from ?? "/",
  );
  // Width + resize state, mirroring the app sidebar's handle: the
  // aside ref lets the hook write style.width directly during drag
  // instead of re-rendering per mousemove.
  const asideRef = useRef<HTMLElement>(null);
  const { width, onResizeStart } = useResizableWidth({
    storageKey: STORAGE_WIDTH,
    defaultWidth: SIDEBAR_DEFAULT,
    min: SIDEBAR_MIN,
    max: SIDEBAR_MAX,
    edge: "right",
    targets: [asideRef],
  });

  // Sidebar search filters navigation by pane label (impl 0025
  // decision 7) — row-level content search is deferred.
  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    return NAV_GROUPS.map((group) => ({
      ...group,
      panes: q
        ? group.panes.filter((key) =>
            PANES[key].label.toLowerCase().includes(q),
          )
        : group.panes,
    })).filter((group) => group.panes.length > 0);
  }, [query]);

  return (
    <div className="flex h-screen overflow-hidden bg-bg text-fg">
      <aside
        ref={asideRef}
        style={{ width }}
        className="relative flex shrink-0 select-none flex-col border-r border-line bg-sidebar"
      >
        {/* Drag strip under the macOS traffic lights, mirroring the
            app sidebar's title-bar band. */}
        <div data-tauri-drag-region className="h-8 shrink-0" />
        <div className="flex shrink-0 flex-col gap-3 px-4 pb-3 pt-1">
          <button
            type="button"
            onClick={() => navigate(fromRef.current || "/")}
            className="flex cursor-pointer items-center gap-2 rounded border border-transparent px-2 py-1.5 text-left text-fg-2 transition-colors hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg focus:border-sidebar-selected-border focus:bg-sidebar-selected/40 focus:text-fg focus:outline-none"
          >
            <ArrowLeft aria-hidden className="h-3.5 w-3.5" />
            <span className="text-[13px]">Back to app</span>
          </button>
          {/* Raised field in the sidebar's own tonal family (bg-raised
              would vanish here — Carbon's --color-raised and
              --color-sidebar are the same hex). The selected-nav tint
              reads as a soft lift above the rail rather than an inset
              hole. */}
          <div className="flex h-8 items-center gap-2 rounded-md border border-sidebar-selected-border bg-sidebar-selected px-2.5">
            <Search aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search settings…"
              className="min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-fg-3"
            />
          </div>
        </div>
        <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-3 py-2">
          {groups.length === 0 ? (
            <p className="px-2.5 py-1 text-xs text-fg-3">No matching settings.</p>
          ) : (
            groups.map((group) => (
              <div key={group.label} className="flex flex-col gap-0.5">
                <div className="px-2.5 pb-1 text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
                  {group.label}
                </div>
                {group.panes.map((key) => {
                  const item = PANES[key];
                  const Icon = item.icon;
                  const active = key === pane;
                  return (
                    <button
                      key={key}
                      type="button"
                      onClick={() =>
                        navigate(`/settings/${key}`, {
                          state: { from: fromRef.current },
                        })
                      }
                      className={`flex cursor-pointer items-center gap-2.5 rounded border px-2.5 py-1.5 text-left text-sm transition-colors ${
                        active
                          ? "border-sidebar-selected-border bg-sidebar-selected font-semibold text-fg shadow-sm"
                          : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
                      }`}
                    >
                      <Icon
                        aria-hidden
                        className={`h-3.5 w-3.5 shrink-0 ${
                          active ? "text-fg" : "text-fg-2"
                        }`}
                      />
                      <span className="truncate">{item.label}</span>
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </nav>
        <div
          onPointerDown={onResizeStart}
          title="Drag to resize"
          className="absolute right-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
        />
      </aside>
      <main className="relative flex flex-1 flex-col overflow-hidden">
        <div
          data-tauri-drag-region
          className="pointer-events-auto absolute left-0 right-0 top-0 z-10 h-7"
        />
        <div className="flex-1 overflow-y-auto">
          <div className="px-10 pb-16 pt-14">
            <div className="mx-auto flex max-w-[760px] flex-col gap-5">
              {PANES[pane].render()}
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
