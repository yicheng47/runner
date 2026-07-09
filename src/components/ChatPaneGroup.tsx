// Pane group for the direct-chat surface (impl 0020).
//
// ONE render path for every arrangement: a single chat is a group of one.
// The component always renders the pane tree (chrome: headers, focus ring,
// gutters, empty states — headers/ring only when grouped) with the flat
// terminal stack geometry-synced onto the pane bodies. Unifying the paths
// is the point: the earlier split/classic dual-path meant every mode
// transition restyled wrappers and re-parented overlays, and that seam is
// where a string of dogfooding bugs lived.
//
// Terminals never remount on layout changes by construction: each session's
// wrapper lives in this component's flat stack (stable tree position, keyed
// by session id) and is imperatively sized/positioned onto its pane's body
// rect. (Portal re-parenting was rejected: React remounts portal children
// when the container node changes — `updatePortal` compares `containerInfo`
// by identity.)

import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Archive, MoreHorizontal, Terminal, SquarePen, X } from "lucide-react";
import { Group, Panel, Separator } from "react-resizable-panels";

import {
  findLeaf,
  leafForSession,
  recordSplitSizes,
  type PaneLayout,
  type PaneLeaf,
  type PaneNode,
} from "../lib/paneLayout";
import type { DirectChatDisplayStatus } from "../lib/directChatStatus";
import type { SessionStatus } from "../lib/types";
import type { SecondaryState } from "../lib/windowFocus";
import { DuplicateSubjectOverlay } from "./DuplicateSubjectOverlay";
import {
  RunnerTerminal,
  type RunnerTerminalHandle,
} from "./RunnerTerminal";
import {
  ResumeButton,
  ResumingButton,
  StopButton,
} from "./ui/SessionControl";

interface ExitEvent {
  session_id: string;
  mission_id: string | null;
  exit_code: number | null;
  success: boolean;
}

/** One attached session in the flat terminal stack. */
export interface PaneChat {
  id: string;
  status: SessionStatus;
}

export interface ChatPaneGroupProps {
  /** The layout to render: the store group when the open chat is a member,
   *  else an ephemeral single-leaf layout for it. */
  layout: PaneLayout;
  /** True when the layout has 2+ panes — gates the pane chrome (headers,
   *  focus ring). A group of one renders a bare full-bleed body. */
  grouped: boolean;
  /** Every attached session (the hidden pool included). */
  chats: PaneChat[];
  /** A resume/start is in flight for this session — blank its canvas so
   *  the pill reads on a pristine surface. */
  transitionalFor: (sessionId: string) => boolean;
  /** Per-pane transitional / session-ended overlay, built by RunnerChat
   *  which owns those states. */
  overlayFor: (sessionId: string) => React.ReactNode;
  /** Resume in flight for this session — drives the header pill. */
  resumingFor: (sessionId: string) => boolean;
  onStopSession: (sessionId: string) => void;
  onResumeSession: (sessionId: string) => void;
  onArchiveSession: (sessionId: string) => void;
  /** Collapse a pane (sibling reflows) — the empty pane's `×`. */
  onClosePane: (paneId: string) => void;
  terminalBg: string;
  nameFor: (sessionId: string) => string;
  statusFor: (sessionId: string) => DirectChatDisplayStatus;
  runtimeFor: (sessionId: string) => string;
  secondaryFor: (sessionId: string) => SecondaryState | undefined;
  dismissedSecondary: ReadonlySet<string>;
  terminalRefFor: (sessionId: string) => (h: RunnerTerminalHandle | null) => void;
  onFocusPane: (leaf: PaneLeaf) => void;
  onNewChat: (paneId: string) => void;
  onDismissSecondary: (sessionId: string) => void;
  onTerminalExit: (ev: ExitEvent) => void;
  onTerminalError: (msg: string) => void;
}

export function ChatPaneGroup({
  layout,
  grouped,
  chats,
  transitionalFor,
  overlayFor,
  resumingFor,
  onStopSession,
  onResumeSession,
  onArchiveSession,
  onClosePane,
  terminalBg,
  nameFor,
  statusFor,
  runtimeFor,
  secondaryFor,
  dismissedSecondary,
  terminalRefFor,
  onFocusPane,
  onNewChat,
  onDismissSecondary,
  onTerminalExit,
  onTerminalError,
}: ChatPaneGroupProps) {
  // Geometry bookkeeping in one closure object created once — plain
  // captured variables instead of refs, because the callback-ref factories
  // run during render, where touching `ref.current` is off-limits
  // (react-hooks/refs). Registration happens at commit time.
  const [paneGeo] = useState(createPaneGeometry);
  useEffect(() => () => paneGeo.dispose(), [paneGeo]);

  // Position wrappers before paint when the pane tree changes — pane
  // splits/closes and session assignment both rebuild `layout.root`. A
  // wrapper that mounts WITHOUT a tree change (a session attaching into an
  // already-hydrated pane — restored-split hydration in RunnerChat) is
  // synced by `termWrapRefFor` at commit instead. The RO inside paneGeo
  // keeps everything glued through gutter drags and resizes, so we
  // deliberately do NOT re-sync on every unrelated re-render (e.g. a
  // resume toggling `resumingIds`): an all-panes getBoundingClientRect
  // read + geometry write per commit is redundant, and a stray sub-pixel
  // delta on a sibling wrapper would needlessly perturb its terminal.
  useLayoutEffect(() => {
    paneGeo.setRoot(layout.root);
    paneGeo.sync();
  }, [layout.root, paneGeo]);

  // Plain render functions, not components: defining component types here
  // would give them a fresh identity every commit and remount the whole
  // pane tree (Group included) on every render.

  const renderPaneNode = (node: PaneNode): React.ReactNode => {
    if (node.kind === "leaf") {
      const focused = node.id === layout.focusedPaneId;
      const sid = node.sessionId;
      const sec = sid ? secondaryFor(sid) : undefined;
      const showPaneOverlay =
        sid != null && (sec?.secondary ?? false) && !dismissedSecondary.has(sid);
      return (
        <section
          key={node.id}
          // Clicks focus the pane — except on the header's Stop/Resume
          // controls: stopping a background pane shouldn't yank focus
          // (and the URL) over to it.
          onMouseDownCapture={(e) => {
            if ((e.target as HTMLElement).closest("[data-pane-controls]")) {
              return;
            }
            onFocusPane(node);
          }}
          // Connected-pane look: no box border of its own — the shared
          // divider is the Separator — but the border slot stays occupied
          // (transparent) so the focus ring doesn't shift pane-body rects
          // when it moves. Bare body when not grouped.
          className={`relative flex h-full w-full flex-col overflow-hidden ${
            grouped
              ? `border ${focused ? "border-accent" : "border-transparent"}`
              : ""
          }`}
        >
          {grouped ? (
            <header className="flex h-[34px] shrink-0 items-center gap-2 border-b border-line bg-panel px-3.5">
              <Terminal
                aria-hidden
                className={`h-[13px] w-[13px] shrink-0 ${
                  focused ? "text-accent" : "text-fg-3"
                }`}
              />
              <span
                className={`min-w-0 truncate text-[13px] font-medium ${
                  focused ? "text-fg" : "text-fg-2"
                }`}
              >
                {sid ? nameFor(sid) : "Empty pane"}
              </span>
              <span className="shrink-0 rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                Chat
              </span>
              {sid ? (
                <span className="flex shrink-0 items-center gap-1.5">
                  <span
                    className={`inline-block h-1.5 w-1.5 rounded-full ${paneStatusDotClass(statusFor(sid))}`}
                  />
                  <span className="text-[11px] text-fg-2">
                    {statusFor(sid)}
                  </span>
                </span>
              ) : null}
              {/* Per-pane lifecycle controls — each pane owns its own
                  session; the topbar's Stop all / Resume all aggregates.
                  Hidden for empty panes and sessions owned by another
                  window (impl 0018). */}
              {sid && !sec?.secondary ? (
                <span
                  data-pane-controls
                  className="ml-auto flex shrink-0 items-center gap-1"
                >
                  {resumingFor(sid) ? (
                    <ResumingButton />
                  ) : statusFor(sid) === "stopped" ||
                    statusFor(sid) === "crashed" ? (
                    <ResumeButton
                      onClick={() => onResumeSession(sid)}
                      title="Resume this chat"
                    />
                  ) : (
                    <StopButton
                      onClick={() => onStopSession(sid)}
                      title="Stop this chat"
                    />
                  )}
                  <PaneKebab onArchive={() => onArchiveSession(sid)} />
                </span>
              ) : null}
              {/* Empty pane: no session controls — just a dismiss. Same
                  collapse as Cmd+W, made discoverable. */}
              {sid == null ? (
                <span
                  data-pane-controls
                  className="ml-auto flex shrink-0 items-center"
                >
                  <button
                    type="button"
                    title="Close pane"
                    aria-label="Close pane"
                    onClick={() => onClosePane(node.id)}
                    className="inline-flex h-6 w-6 cursor-pointer items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg"
                  >
                    <X aria-hidden className="h-3.5 w-3.5" />
                  </button>
                </span>
              ) : null}
            </header>
          ) : null}
          <div
            ref={(el) => paneGeo.paneBodyRefFor(node.id)(el)}
            style={{ backgroundColor: terminalBg }}
            className="relative min-h-0 flex-1"
          >
            {sid == null ? (
              <EmptyPaneBody onNewChat={() => onNewChat(node.id)} />
            ) : showPaneOverlay ? (
              // Per-pane duplicate-subject gate (impl 0018 × 0020): this
              // pane's session is owned by another window, so no terminal
              // is mounted for it and the overlay scopes to this pane.
              <DuplicateSubjectOverlay
                kind="chat"
                primaryLabel={sec?.primaryLabel ?? null}
                onStayHere={() => onDismissSecondary(sid)}
              />
            ) : null}
          </div>
        </section>
      );
    }
    const horizontal = node.orientation === "row";
    return (
      <Group
        key={node.id}
        orientation={horizontal ? "horizontal" : "vertical"}
        className="flex h-full w-full"
        // The visible divider is 1px; this widens the pointer hit area so
        // it stays comfortably draggable.
        resizeTargetMinimumSize={{ coarse: 24, fine: 10 }}
        onLayoutChanged={(l) => {
          const a = l[`${node.id}:a`];
          const b = l[`${node.id}:b`];
          if (typeof a === "number" && typeof b === "number") {
            recordSplitSizes(node.id, [a, b]);
          }
        }}
      >
        <Panel
          id={`${node.id}:a`}
          defaultSize={`${node.sizes[0]}%`}
          minSize={120}
          className="h-full w-full"
        >
          {renderPaneNode(node.a)}
        </Panel>
        <Separator
          className={`shrink-0 bg-line-strong transition-colors data-[separator=active]:bg-accent data-[separator=hover]:bg-accent/60 ${
            horizontal ? "w-px" : "h-px"
          }`}
        />
        <Panel
          id={`${node.id}:b`}
          defaultSize={`${node.sizes[1]}%`}
          minSize={120}
          className="h-full w-full"
        >
          {renderPaneNode(node.b)}
        </Panel>
      </Group>
    );
  };

  // One terminal wrapper per attached session. Sessions in a pane are
  // geometry-synced onto its body; the rest stack hidden and keep their
  // buffers. A session owned by another window (impl 0018) mounts nothing
  // — its pane shows the duplicate-subject overlay from the chrome layer.
  const renderTerminalPane = (chat: PaneChat): React.ReactNode => {
    if (secondaryFor(chat.id)?.secondary) return null;
    const paneLeaf = leafForSession(layout.root, chat.id);
    const visible = paneLeaf !== null;
    const transitional = transitionalFor(chat.id);
    const dead = chat.status !== "running";
    // Pane visual state: while resuming/starting the pane is fully blank
    // so the centered pill reads on a pristine canvas; when stopped it
    // dims to 45% under the Session ended card.
    const paneOpacity =
      visible && transitional
        ? "opacity-0"
        : visible && dead
          ? "opacity-45"
          : "";
    return (
      <div
        key={chat.id}
        ref={(el) => paneGeo.termWrapRefFor(chat.id)(el)}
        // `backgroundColor` is inlined from `useTerminalBg()` so the frame
        // tracks the active terminal palette across theme switches.
        style={{ backgroundColor: terminalBg }}
        onMouseDownCapture={
          paneLeaf ? () => onFocusPane(paneLeaf) : undefined
        }
        className={`absolute p-4 ${visible ? "block" : "hidden"}`}
      >
        {/* Dim only the canvas — the overlays below are siblings, so they
            stay at full opacity over the dimmed terminal. */}
        <div className={`h-full w-full ${paneOpacity} transition-opacity`}>
          <RunnerTerminal
            ref={terminalRefFor(chat.id)}
            sessionId={chat.id}
            runnerRuntime={runtimeFor(chat.id)}
            // While the resume/start loader is up the canvas is hidden, so
            // xterm behaves as inactive (no resize pushes, no focus); when
            // the flag clears, the activation effect fits + repaints.
            active={visible && !transitional}
            autoFocus={visible && paneLeaf.id === layout.focusedPaneId}
            disabled={dead || transitional}
            onExit={onTerminalExit}
            onError={onTerminalError}
          />
        </div>
        {overlayFor(chat.id)}
      </div>
    );
  };

  return (
    <div
      ref={(el) => paneGeo.containerRef(el)}
      className="relative h-full w-full overflow-hidden"
    >
      <div className="h-full w-full">{renderPaneNode(layout.root)}</div>
      {chats.map((chat) => renderTerminalPane(chat))}
    </div>
  );
}

/// Geometry sync (impl 0020, decision 4). Closure over plain Maps instead
/// of refs so the callback-ref factories can be called during render
/// (react-hooks/refs forbids `ref.current` there); the returned callbacks
/// are stable per key so React doesn't detach/reattach them per commit.
function createPaneGeometry() {
  let container: HTMLDivElement | null = null;
  let ro: ResizeObserver | null = null;
  let root: PaneNode | null = null;
  const paneBodies = new Map<string, HTMLDivElement>();
  const termWraps = new Map<string, HTMLDivElement>();
  const paneBodyCbs = new Map<string, (el: HTMLDivElement | null) => void>();
  const termWrapCbs = new Map<string, (el: HTMLDivElement | null) => void>();

  const sync = () => {
    if (!container || !root) return;
    const cRect = container.getBoundingClientRect();
    for (const [paneId, bodyEl] of paneBodies) {
      const leaf = findLeaf(root, paneId);
      if (!leaf?.sessionId) continue;
      const wrap = termWraps.get(leaf.sessionId);
      if (!wrap) continue;
      const r = bodyEl.getBoundingClientRect();
      wrap.style.left = `${r.left - cRect.left}px`;
      wrap.style.top = `${r.top - cRect.top}px`;
      wrap.style.width = `${r.width}px`;
      wrap.style.height = `${r.height}px`;
    }
  };

  return {
    sync,
    containerRef(el: HTMLDivElement | null) {
      container = el;
    },
    setRoot(next: PaneNode) {
      root = next;
    },
    paneBodyRefFor(paneId: string) {
      let cb = paneBodyCbs.get(paneId);
      if (!cb) {
        cb = (el) => {
          const prev = paneBodies.get(paneId);
          if (prev) ro?.unobserve(prev);
          if (el) {
            paneBodies.set(paneId, el);
            ro ??= new ResizeObserver(sync);
            ro.observe(el);
          } else {
            paneBodies.delete(paneId);
          }
        };
        paneBodyCbs.set(paneId, cb);
      }
      return cb;
    },
    termWrapRefFor(sessionId: string) {
      let cb = termWrapCbs.get(sessionId);
      if (!cb) {
        cb = (el) => {
          if (el) {
            termWraps.set(sessionId, el);
            // A wrapper can mount a commit AFTER its pane body has already
            // settled: restored-split hydration attaches the session
            // (adding it to `chats`) once the pane tree is stable, so
            // `layout.root` doesn't change and neither the geometry
            // layoutEffect nor the pane-body ResizeObserver fires. Position
            // it now from the existing pane rects. On the component's first
            // mount `root`/`container` aren't set yet so this no-ops,
            // leaving the layoutEffect to drive the initial sync.
            sync();
          } else {
            termWraps.delete(sessionId);
          }
        };
        termWrapCbs.set(sessionId, cb);
      }
      return cb;
    },
    dispose() {
      ro?.disconnect();
    },
  };
}

/// Per-pane overflow menu — Archive for this pane's chat. Mirrors the
/// topbar `ChatKebab` shape; a single item today, room for per-pane
/// Pin/Rename later. Lives inside the header's `data-pane-controls`
/// cluster so opening it doesn't move pane focus.
function PaneKebab({ onArchive }: { onArchive: () => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Pane actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="inline-flex h-6 w-6 cursor-pointer items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg"
      >
        <MoreHorizontal aria-hidden className="h-3.5 w-3.5" />
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-50 mt-1 flex w-36 flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onArchive();
            }}
            className="flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] text-danger hover:bg-line"
          >
            <Archive aria-hidden className="h-3.5 w-3.5 text-danger" />
            <span>Archive</span>
          </button>
        </div>
      ) : null}
    </div>
  );
}

/// Empty pane (Pencil `t0YBp`): New chat funnels into the StartChatModal
/// with the focused chat's runner preselected; the sidebar is the other
/// fill path.
function EmptyPaneBody({ onNewChat }: { onNewChat: () => void }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3.5 p-4 text-center">
      <SquarePen aria-hidden className="h-[22px] w-[22px] text-fg-3" />
      <span className="text-[13px] font-medium text-fg-2">
        No chat in this pane
      </span>
      <button
        type="button"
        onClick={onNewChat}
        className="cursor-pointer rounded-md bg-accent px-3.5 py-[7px] text-[12px] font-semibold text-bg transition-colors hover:bg-accent/90"
      >
        New chat
      </button>
      <span className="text-[11px] text-fg-3">
        or pick a chat from the sidebar
      </span>
    </div>
  );
}

// Pane-header status dot, mirroring the sidebar's chat-row dot palette.
function paneStatusDotClass(status: DirectChatDisplayStatus): string {
  switch (status) {
    case "busy":
      return "bg-accent";
    case "idle":
      return "bg-accent/35";
    case "crashed":
      return "bg-danger";
    case "stopped":
      return "bg-fg-3";
  }
}
