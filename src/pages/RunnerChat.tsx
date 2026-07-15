// Direct-chat pane (C8.5) — `/chats/:sessionId`.
//
// One-on-one PTY between the human and the runner's CLI. No mission, no
// orchestrator, no event bus. Each direct session gets its own mounted
// RunnerTerminal pane so switching chats preserves xterm's real screen and
// scrollback while the backend output snapshot covers late attach / reload.
//
// Uses xterm.js so real TUIs (claude-code, codex) render correctly with
// ANSI colors, cursor movement, and mouse tracking. A plain `<pre>`
// can't interpret the control sequences these agents emit.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  MoreHorizontal,
  Pin,
  PinOff,
  SquarePen,
  SquareSplitHorizontal,
  Terminal,
} from "lucide-react";

import {
  BackButton,
  ResumeButton,
  ResumingButton,
  StopButton,
} from "../components/ui/SessionControl";
import { CopyValueButton } from "../components/ui/CopyValueButton";
import {
  ArchivingOverlay,
  ResumingOverlay,
  SessionEndedOverlay,
  StartingOverlay,
} from "../components/SessionEndedOverlay";
import { api, type DirectSessionEntry } from "../lib/api";
import {
  chunkIndicatesTuiReady,
  isFreshSpawn,
  snapshotIndicatesTuiReady,
} from "../lib/sessionLifecycle";
import { useDelayedFlag } from "../lib/useDelayedFlag";
import { useResizableWidth } from "../hooks/useResizableWidth";
import { useTerminalBg } from "../lib/useTerminalBg";
import {
  isArchivingSession,
  markArchivingSession,
  unmarkArchivingSession,
  useArchivingVersion,
} from "../lib/archivingState";
import { ChatPaneGroup } from "../components/ChatPaneGroup";
import { PanelToggleGlyph } from "../components/PanelToggleGlyph";
import { createTerminalRegistry } from "../lib/terminalRegistry";
import { LayoutPicker } from "../components/LayoutPicker";
import { StartChatModal } from "../components/StartChatModal";
import {
  activatePaneLayoutForSession,
  applyPreset,
  applyPresetPure,
  assignSessionToPane,
  isFreshlyAssigned,
  closePane,
  findLeaf,
  focusPane,
  getPaneLayout,
  isGroupActiveFor,
  leafForSession,
  leaves,
  removeArchivedSessionFromLayout,
  removeSessionFromLayout,
  setGroupName,
  setRouteAnchorSession,
  usePaneLayout,
  visibleSessionIds,
  type PaneLeaf,
  type PresetKind,
} from "../lib/paneLayout";
import { useActiveProjectScope } from "../lib/projectScope";
import {
  pinnedSessionIds,
  shouldInheritPinOnAdd,
} from "../lib/groupPinning";
import {
  directChatDisplayStatus,
  type DirectChatDisplayStatus,
} from "../lib/directChatStatus";
import {
  isSecondaryFor,
  reportSubjectsNow,
  useCurrentWindowLabel,
  useReportSubjects,
  useWindowFocus,
  type SecondaryState,
} from "../lib/windowFocus";
import type {
  Runner,
  SessionActivityEvent,
  SessionActivityState,
  SessionStatus,
  SessionUpdatedEvent,
  Subject,
  WarningEvent,
} from "../lib/types";
import { eventMatchesShortcut } from "../lib/keymap";

interface ExitEvent {
  session_id: string;
  mission_id: string | null;
  exit_code: number | null;
  success: boolean;
}

// Always lands here in "attach" mode: the originating button
// (RunnerDetail "Chat now", Runners list "Chat" pill, sidebar
// SESSION list) spawns or looks up the session synchronously and
// navigates with the resulting `sessionId` baked into the URL.
// Removing in-effect spawning was the cleanest fix for the StrictMode
// double-mount race that left two visible sessions per click.
//
// The session id rides on the URL itself (`/chats/:sessionId`),
// so refresh / back-forward / paste all attach to the right chat.
// `location.state.sessionStatus` is an optional optimistic seed that
// avoids a one-tick "running" flicker when the originating row is
// already stopped — when missing we fall back to "stopped" and let the
// chatMeta fetch correct it.
interface RunnerChatLocationState {
  /** Real status of the session row at navigation time so we don't
   *  briefly seed the pane as running and let xterm forward
   *  keystrokes to a session that's no longer in the live map. */
  sessionStatus?: SessionStatus;
}

const STORAGE_PANEL_OPEN = "runner.chat.panel.open";
const STORAGE_PANEL_WIDTH = "runner.chat.panel.width";
// Dispatched by RunnerTerminal's key handler for plain Cmd+[ / Cmd+]
// keystrokes that land inside xterm; mirrors SIDEBAR_NAVIGATE_EVENT.
const RUNNER_TERMINAL_CYCLE_EVENT = "runner:cycle-terminal";
const PANEL_MIN = 200;
const PANEL_MAX = 480;
const PANEL_DEFAULT = 320;

interface DirectSessionPane {
  id: string;
  label: string;
  status: SessionStatus;
  exitCode: number | null;
}

// A chat spawned into a group with a pinned member inherits the pin so
// the sidebar keeps the group in one cluster (never unpins on add).
// Fresh fetch rather than this page's `recentRows`: pins toggled from
// the sidebar don't flow into that cache. The sidebar reorders off the
// `session/updated` emit from session_pin.
async function inheritGroupPin(
  existingMemberIds: string[],
  newSessionId: string,
): Promise<void> {
  try {
    const rows = await api.session.listRecentDirect();
    if (
      shouldInheritPinOnAdd(existingMemberIds, pinnedSessionIds(rows), newSessionId)
    ) {
      await api.session.pin(newSessionId, true);
    }
  } catch (e) {
    console.error("RunnerChat: group pin inherit failed", e);
  }
}

export default function RunnerChat() {
  const { sessionId: sessionIdParam } = useParams<{
    sessionId: string;
  }>();
  const location = useLocation();
  const navigate = useNavigate();
  const activeProject = useActiveProjectScope();
  const state = location.state as RunnerChatLocationState | null;

  const sessionId = sessionIdParam ?? null;
  const [directSessions, setDirectSessions] = useState<DirectSessionPane[]>([]);
  // Live handles to each mounted RunnerTerminal so the resume path can
  // measure the actual xterm geometry before the backend forks the new
  // PTY child (without this, pty_runtime falls back to 80×24 —
  // #resume-pty-size-mismatch) and pane-focus changes can move keyboard
  // focus. Closure registry rather than a ref: the split render path
  // resolves callbacks during render, where refs are off-limits.
  const [terminals] = useState(createTerminalRegistry);
  const [err, setErr] = useState<string | null>(null);
  // Resume-fallback banner: distinct from `err` because it isn't a failure
  // the user has to act on — the agent just couldn't resume a prior
  // conversation and started fresh.
  const [warning, setWarning] = useState<string | null>(null);
  // Runner config (for runtime label in the header).
  const [runner, setRunner] = useState<Runner | null>(null);
  // Direct-session row metadata (started_at, cwd, title) for the header
  // meta line. Pulled from session_list_recent_direct so the chat
  // surface and the sidebar agree on the truth.
  const [chatMeta, setChatMeta] = useState<DirectSessionEntry | null>(null);
  // Full recent-direct list, cached from the same fetch — split panes
  // (impl 0020) read their header name/status per session from here.
  // `rowsLoaded` flips once the first fetch succeeds; the restored-layout
  // hydration below waits for it so it never judges panes off stale data.
  const [recentRows, setRecentRows] = useState<DirectSessionEntry[]>([]);
  const [rowsLoaded, setRowsLoaded] = useState(false);
  const [activityBySession, setActivityBySession] = useState<
    Record<string, SessionActivityState | undefined>
  >({});
  // chatMeta is async (listRecentDirect → session_get fallback) and is
  // the only source of `archived_at`. Until it resolves we don't know
  // whether the URL points at an archived row, and we can't safely
  // attach a PTY or mount RunnerTerminal — that would briefly subscribe
  // to session/output and call outputSnapshot for an archived row
  // before the read-only branch kicks in. Gate both the attach effect
  // and the body render on this flag; it flips true once the first
  // refresh completes (success or miss). Per-sessionId so back-forward
  // navigation to a different chat doesn't reuse the stale flag.
  const [metaLoadedFor, setMetaLoadedFor] = useState<string | null>(null);
  const metaLoaded = sessionId != null && metaLoadedFor === sessionId;
  // Sessions with a resume in flight — Resume clicked, new PTY not yet
  // painted. Per session id: split panes resume independently (each pane
  // header has its own Resume, plus the topbar's Resume all), and each
  // in-flight id gets a ResumeSettleTracker that clears it when the agent
  // settles. Drives the cyan status pill, the header "Resuming…"
  // affordance, and the per-pane Resuming overlay on the cleared canvas.
  const [resumingIds, setResumingIds] = useState<Set<string>>(() => new Set());
  const resuming = sessionId != null && resumingIds.has(sessionId);
  const settleResume = useCallback((id: string) => {
    setResumingIds((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, []);
  // True while a freshly-attached session is still warming up — the
  // terminal has mounted but the agent CLI (claude-code / codex)
  // hasn't painted its first frame yet. Overlays the StartingOverlay
  // pill on top of the otherwise-blank xterm canvas so the user sees
  // a clear "we're booting your chat" state instead of staring at an
  // empty pane for a couple of seconds. Cleared by the same
  // first-output + idle-debounce + min-visible dance as `resuming`.
  const [starting, setStarting] = useState<boolean>(false);
  // True while either this chat's own archiveChat or the sidebar's
  // session-archive flow has marked this session id as archiving.
  // Drives the centered amber pill + scrim over the chat body so the
  // backend's session_kill grace + archive RPC don't read as a hang.
  useArchivingVersion(); // subscribe: per-pane overlays check ids below
  const archiving = sessionId != null && isArchivingSession(sessionId);
  // Right-side panel (runner identity + system prompt readout) open
  // state. Mirrors Obsidian's panel-toggle pattern: a small button at
  // the right edge of the topbar flips it. Persisted in localStorage
  // so the user's preference sticks across reloads.
  const [panelOpen, setPanelOpen] = useState<boolean>(() => {
    if (typeof localStorage === "undefined") return true;
    return localStorage.getItem(STORAGE_PANEL_OPEN) !== "0";
  });
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_PANEL_OPEN, panelOpen ? "1" : "0");
    } catch {
      // ignore quota / disabled-storage errors
    }
  }, [panelOpen]);

  // Set by `End chat` so the exit handler can distinguish a user-
  // initiated kill (we want it to read as "stopped") from an actual
  // crash. Without this, every End chat lands on status="crashed"
  // because SIGKILL bubbles up as a non-zero exit.
  const killedSessionsRef = useRef<Set<string>>(new Set());
  // Last route/session request this component attached for. React
  // Router reuses RunnerChat when moving between `/chats/:sessionId`
  // routes, so this must be keyed by the URL param instead of a
  // one-shot boolean.
  const startedKeyRef = useRef<string | null>(null);

  const activeSession = directSessions.find((s) => s.id === sessionId) ?? null;
  const status = activeSession?.status ?? chatMeta?.status ?? "running";
  const exitCode = activeSession?.exitCode ?? null;
  const backTarget = chatMeta?.handle ? `/runners/${chatMeta.handle}` : "/runners";
  const backLabel = chatMeta?.handle ? "Back to runner" : "Back to runners";
  // Archived rows can be reached by direct URL but render read-only.
  // We don't attach a PTY, mount RunnerTerminal, or expose Resume /
  // End / Archive — the row is terminal and the operator can only
  // read the meta + go back to the runner.
  const isArchived = chatMeta?.archived_at != null;

  // Split-view layout (impl 0020). Module snapshot shared with Sidebar;
  // durable tab structure comes from SQLite while focus stays per-window.
  // The split is a chat GROUP — a binding between specific sessions — not
  // a viewport mode: it renders only while the open chat is one of its
  // members. Any other chat renders the classic single pane, and the
  // group stays intact in the background until a member is opened again.
  const layout = usePaneLayout(sessionId);
  useEffect(() => {
    // This is the authoritative route signal: record the session this window
    // owns so cross-window layout hydration anchors on it (paneLayout.ts).
    setRouteAnchorSession(sessionId);
    activatePaneLayoutForSession(sessionId);
  }, [sessionId]);
  const splitActive = isGroupActiveFor(layout, sessionId);
  // What the pane group renders: the store group for member chats, an
  // ephemeral single-leaf layout for everything else — one render path,
  // a single chat is just a group of one.
  const viewLayout = useMemo(
    () =>
      splitActive
        ? layout
        : applyPresetPure("single", sessionId, sessionId ? [sessionId] : []),
    [splitActive, layout, sessionId],
  );

  // Multi-window coordination (impl 0018 + 0020). Report every visible
  // pane's session as a window subject — a split window owns each session
  // it shows, not just the focused one. Per session, if another window
  // shows the same chat with a later focus, that pane is secondary here
  // and must not mount a terminal or send stdin/resize/start.
  const focusMap = useWindowFocus();
  const myWindowLabel = useCurrentWindowLabel();
  const paneSessionIds = splitActive ? visibleSessionIds(layout.root) : [];
  const subjectIds =
    sessionId && !paneSessionIds.includes(sessionId)
      ? [sessionId, ...paneSessionIds]
      : paneSessionIds;
  useReportSubjects(
    subjectIds.map((value) => ({ type: "DirectChat", value }) as Subject),
  );
  const secondaryBySession = new Map<string, SecondaryState>(
    subjectIds.map((value) => [
      value,
      isSecondaryFor(focusMap, myWindowLabel, { type: "DirectChat", value }),
    ]),
  );
  const { secondary: isSecondary } = (sessionId
    ? secondaryBySession.get(sessionId)
    : null) ?? { secondary: false, primaryLabel: null };
  // Dismissed duplicate-subject overlays, per session id. Cleared on
  // navigation (spec-12 decision 6: dismissal isn't persisted) and pruned
  // when a session stops being secondary so a later re-flip re-shows it.
  const [dismissedSecondary, setDismissedSecondary] = useState<Set<string>>(
    () => new Set(),
  );
  useEffect(() => {
    setDismissedSecondary((prev) => (prev.size === 0 ? prev : new Set()));
  }, [sessionId]);
  const secondaryKey = subjectIds
    .filter((id) => secondaryBySession.get(id)?.secondary)
    .join(",");
  useEffect(() => {
    const stillSecondary = new Set(secondaryKey ? secondaryKey.split(",") : []);
    setDismissedSecondary((prev) => {
      const next = new Set([...prev].filter((id) => stillSecondary.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [secondaryKey]);

  // Render-gate without the flash. The two branches below
  // (chatMeta still loading, or terminals haven't upserted yet) gate
  // RunnerTerminal mounting to keep archived rows off the PTY bus,
  // but on the common fast-IPC path the gate clears in <50ms and the
  // visible cyan pill reads as a flicker on every chat-to-chat
  // navigation. Delay the pill render by 150ms: instant resolutions
  // never show a pill, slow ones still get user feedback.
  const navLoadingActive =
    sessionId != null &&
    (!metaLoaded || (!isArchived && directSessions.length === 0));
  const showNavLoadingPill = useDelayedFlag(navLoadingActive, 150, sessionId);

  const upsertSession = useCallback((next: DirectSessionPane) => {
    setDirectSessions((prev) => {
      const found = prev.find((s) => s.id === next.id);
      if (!found) return [...prev, next];
      return prev.map((s) =>
        s.id === next.id
          ? {
              ...s,
              label: next.label,
              status: next.status,
              exitCode: next.exitCode,
            }
          : s,
      );
    });
  }, []);

  const clearActivity = useCallback((id: string) => {
    setActivityBySession((prev) => {
      if (prev[id] == null) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
  }, []);

  const attach = useCallback(
    (
      id: string,
      label: string,
      status: SessionStatus = "running",
      freshSpawn = false,
    ) => {
      setErr(null);
      upsertSession({
        id,
        label,
        status,
        exitCode: null,
      });
      // Show the Starting pill over the freshly-mounted terminal
      // until the agent CLI paints. Two gates: status === "running"
      // (no pill over a stopped/crashed row — that surface is the
      // Session ended card), and `freshSpawn` (no pill when the
      // user is just switching tabs to a chat that's been running
      // for an hour — the terminal is already painted and the user
      // wants the live canvas, not a 1s flash).
      if (status === "running" && freshSpawn) setStarting(true);
    },
    [upsertSession],
  );

  const onTerminalExit = useCallback(
    (ev: ExitEvent) => {
      const userEnded = killedSessionsRef.current.has(ev.session_id);
      const nextStatus = ev.success || userEnded ? "stopped" : "crashed";
      killedSessionsRef.current.delete(ev.session_id);
      clearActivity(ev.session_id);
      setDirectSessions((prev) =>
        prev.map((s) =>
          s.id === ev.session_id
            ? { ...s, status: nextStatus, exitCode: ev.exit_code }
            : s,
        ),
      );
    },
    [clearActivity],
  );

  // Pull runner config only for runner-backed chats. Runtime-only
  // chats render from chatMeta.agent_* instead.
  useEffect(() => {
    if (!chatMeta?.runner_id) {
      setRunner(null);
      return;
    }
    let cancelled = false;
    void api.runner
      .get(chatMeta.runner_id)
      .then((r) => {
        if (!cancelled) setRunner(r);
      })
      .catch(() => {
        if (!cancelled) setRunner(null);
      });
    return () => {
      cancelled = true;
    };
  }, [chatMeta?.runner_id]);

  // Pull this chat's metadata (started_at, cwd, title) for the header.
  // Refetched on session/exit so status changes flip the pill.
  //
  // Primary lookup: `listRecentDirect` — same SELECT the sidebar
  // uses, so the two surfaces agree on un-archived rows. Archived
  // sessions are filtered out at SQL, so direct-URL navigation to
  // an archived session would miss; we fall back to the unfiltered
  // `session_get` so the chat page can detect the archived state
  // and lock the workspace read-only instead of treating the row
  // as missing.
  const refreshChatMeta = useCallback(async () => {
    if (!sessionId) {
      setChatMeta(null);
      setMetaLoadedFor(null);
      return;
    }
    try {
      const rows = await api.session.listRecentDirect();
      setRecentRows(rows);
      setRowsLoaded(true);
      const found = rows.find((r) => r.session_id === sessionId);
      if (found) {
        setChatMeta(found);
        const full = await api.session.get(sessionId);
        setChatMeta(full ?? found);
        return;
      }
      // Missed in the visible list — could be archived. The
      // unfiltered get returns the row regardless so the read-only
      // branch below can render the right UX.
      const archived = await api.session.get(sessionId);
      setChatMeta(archived);
    } catch (e) {
      console.error("RunnerChat: refreshChatMeta failed", e);
    } finally {
      // Flag flips regardless of outcome so the body render can leave
      // the neutral loading state — a network failure shouldn't strand
      // the user on a perpetual spinner.
      setMetaLoadedFor(sessionId);
    }
  }, [sessionId]);

  useEffect(() => {
    void refreshChatMeta();
  }, [refreshChatMeta]);

  useEffect(() => {
    if (!sessionId || !metaLoaded || isArchived) return;
    if (
      chatMeta?.agent_runtime !== "codex" ||
      chatMeta.agent_session_key ||
      chatMeta.status !== "running"
    ) {
      return;
    }

    const targetId = sessionId;
    const MAX_ATTEMPTS = 35;
    let cancelled = false;
    let attempts = 0;

    const poll = () => {
      attempts += 1;
      void api.session
        .get(targetId)
        .then((row) => {
          if (cancelled) return;
          if (!row) {
            window.clearInterval(interval);
            return;
          }
          setChatMeta(row);
          if (row.agent_session_key || attempts >= MAX_ATTEMPTS) {
            window.clearInterval(interval);
          }
        })
        .catch(() => {
          if (attempts >= MAX_ATTEMPTS) window.clearInterval(interval);
        });
    };

    const interval = window.setInterval(poll, 1000);
    poll();
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [
    sessionId,
    metaLoaded,
    isArchived,
    chatMeta?.agent_runtime,
    chatMeta?.agent_session_key,
    chatMeta?.status,
  ]);

  useEffect(() => {
    if (!sessionId) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen("session/exit", () => {
      void refreshChatMeta();
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sessionId, refreshChatMeta]);

  useEffect(() => {
    if (!sessionId) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    // session/updated refreshes for ANY session, not just the URL one:
    // pane headers and the codex session-key readout render from
    // recentRows, and the key-capture watcher fires session/updated for
    // whichever pane's row it lands on. session/archived keeps the
    // recent list honest when a chat is archived from any surface — the
    // prune effect below then drops its terminal from the stack.
    void Promise.all([
      listen<SessionUpdatedEvent>("session/updated", () => {
        void refreshChatMeta();
      }),
      listen("session/archived", () => {
        void refreshChatMeta();
      }),
    ]).then(([fnUpdated, fnArchived]) => {
      if (cancelled) {
        fnUpdated();
        fnArchived();
        return;
      }
      unlisten = () => {
        fnUpdated();
        fnArchived();
      };
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sessionId, refreshChatMeta]);

  // Keep the attach stack derived from the recent-direct list: a row
  // that vanishes (archived from this window, the sidebar, or another
  // window) drops its terminal and per-session flags immediately.
  // Without this, an archived pane's session lingered in the hidden
  // stack — mounted PTY listener and all — until the surface unmounted,
  // and later navigation tripped over the residue. The URL session is
  // exempt: its archived state renders the read-only branch, and a
  // just-spawned chat may briefly precede its row's first appearance.
  useEffect(() => {
    if (!rowsLoaded) return;
    const live = new Set(recentRows.map((r) => r.session_id));
    setDirectSessions((prev) => {
      const next = prev.filter((s) => live.has(s.id) || s.id === sessionId);
      return next.length === prev.length ? prev : next;
    });
    setResumingIds((prev) => {
      const next = new Set(
        [...prev].filter((id) => live.has(id) || id === sessionId),
      );
      return next.size === prev.size ? prev : next;
    });
  }, [rowsLoaded, recentRows, sessionId]);

  // Track busy/idle activity for every direct session, not just the URL
  // one — split panes (impl 0020) each show their own status dot. Entries
  // are dropped by `clearActivity` on exit/kill; no reset on navigation
  // since other panes' chats keep running through it.
  useEffect(() => {
    if (!sessionId || isArchived) {
      setActivityBySession({});
      return;
    }
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<SessionActivityEvent>("session/status", (event) => {
      setActivityBySession((prev) => ({
        ...prev,
        [event.payload.session_id]: event.payload.state,
      }));
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sessionId, isArchived]);

  // Sync the active pane's status from the DB-backed chatMeta. attach()
  // seeds new panes with `status: "running"` because the spawn path is
  // its primary caller, but the sidebar's attach path can land on a
  // stopped row — in which case we need to flip the pane to stopped
  // immediately so the dim overlay + Session ended card render and
  // RunnerTerminal stops forwarding keystrokes to a dead PTY.
  //
  // We deliberately do NOT clear the `resuming` flag here based on
  // chatMeta.status: the row is updated to running the moment
  // `session_resume` returns, but the agent hasn't actually painted
  // anything yet, so the loader would flash off before the user sees
  // the new conversation start. The dedicated resuming-output
  // listener (below) waits for the first real output chunk from the
  // agent, which is the right signal that the canvas is live again.
  useEffect(() => {
    if (!chatMeta) return;
    if (chatMeta.status !== "running") clearActivity(chatMeta.session_id);
    setDirectSessions((prev) =>
      prev.map((s) =>
        s.id === chatMeta.session_id
          ? { ...s, status: chatMeta.status }
          : s,
      ),
    );
  }, [chatMeta, clearActivity]);

  // Each in-flight resume gets a ResumeSettleTracker (rendered at the
  // bottom of the page) that clears its id from `resumingIds` once the
  // agent settles on a steady frame — see the tracker for the heuristic.

  // Mirror of the resume dismissal dance for fresh-attach. The
  // agent CLI takes a beat after spawn to print its banner, so
  // overlay the pill until: (1) we've seen output AND it's gone idle
  // for ~400ms, AND (2) the pill has been visible for at least 1s
  // (no flash on instant paints). 10s hard timeout for silent
  // runtimes (e.g. shell prompts that never emit a banner). We don't
  // require a first chunk to start the idle timer here — even an
  // entirely silent runtime should clear the pill after the
  // min-visible window, otherwise a shell session would sit behind
  // the pill until the user typed.
  useEffect(() => {
    if (!starting || !sessionId) return;
    const STARTING_MIN_VISIBLE_MS = 1000;
    const STARTING_IDLE_DEBOUNCE_MS = 400;
    const STARTING_HARD_TIMEOUT_MS = 10_000;
    const startTs = performance.now();
    const targetId = sessionId;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    let idleTimer: number | null = null;

    const finish = () => {
      if (cancelled) return;
      setStarting(false);
    };

    const scheduleIdleTimer = () => {
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      const elapsed = performance.now() - startTs;
      const delay = Math.max(
        STARTING_IDLE_DEBOUNCE_MS,
        STARTING_MIN_VISIBLE_MS - elapsed,
      );
      idleTimer = window.setTimeout(finish, delay);
    };

    // Seed the idle timer immediately so a silent agent still clears
    // after the min-visible window.
    scheduleIdleTimer();

    const hardTimeout = window.setTimeout(finish, STARTING_HARD_TIMEOUT_MS);

    // Snapshot check covers the race where the PTY emitted its
    // TUI-ready escape before this effect's listener attached. The
    // watermark keeps retained pre-resume chunks (impl 0024) from
    // counting as ready; fresh rows report 0, so the filter is inert
    // here outside resume flows.
    void Promise.all([
      api.session.outputSnapshot(targetId),
      api.session.replayWatermark(targetId),
    ])
      .then(([snapshot, watermark]) => {
        if (cancelled) return;
        if (snapshotIndicatesTuiReady(snapshot, watermark)) {
          finish();
        }
      })
      .catch(() => {
        // Best-effort; live listener still applies.
      });

    void listen<{ session_id: string; data: string }>(
      "session/output",
      (event) => {
        if (event.payload.session_id !== targetId) return;
        // TUI mounts → ready for input. Skip the idle wait. See
        // `chunkIndicatesTuiReady` for why this beats the 400ms
        // debounce when the first-turn prompt is auto-delivered.
        if (chunkIndicatesTuiReady(event.payload.data)) {
          finish();
          return;
        }
        scheduleIdleTimer();
      },
    ).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      window.clearTimeout(hardTimeout);
      unlisten?.();
    };
  }, [starting, sessionId]);

  // Surface non-fatal session warnings (today: agent-resume fallback).
  // Mounted once per page — only one direct chat is in view at a time,
  // so we don't need to filter by session id here. Re-subscribing on
  // every directSessions change would tear down and recreate the
  // listener constantly during spawn handshakes.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      const fn = await listen<WarningEvent>("session/warning", (event) => {
        setWarning(event.payload.message);
      });
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Attach for the current route request. Each session gets its own
  // mounted RunnerTerminal pane, so switching between direct chats
  // preserves xterm's real in-memory screen and scrollback instead of
  // trying to replay raw PTY bytes into a shared terminal.
  //
  // Spawn-mode used to live here: a `state.runnerId` would trigger
  // `api.session.startDirect` from this useEffect, which in StrictMode
  // dev (mount → cleanup → mount) raced its own cleanup and left two
  // visible sessions per click. The originating buttons (RunnerDetail
  // "Chat now", Runners list "Chat" pill) now spawn synchronously and
  // navigate here with the URL-encoded `sessionId`, so this effect only
  // ever runs the deterministic attach path.
  useEffect(() => {
    const requestKey = sessionId ?? "";
    // Wait for chatMeta to resolve before attaching. Without this
    // gate, the brief window between mount and the first
    // refreshChatMeta resolution would attach a PTY listener even
    // for archived sessions (chatMeta is null → isArchived false),
    // briefly subscribing to session/output + calling
    // outputSnapshot before the read-only branch takes over.
    if (!metaLoaded) return;
    // Secondary windows (impl 0018) don't own the PTY: the render path
    // filters this session's RunnerTerminal out (flip primary→secondary
    // mid-session unmounts it so it stops forwarding stdin), and we reset
    // the attach + transitional refs so a flip back to primary re-attaches
    // cleanly. Leaving startedKeyRef null is what lets that re-attach fire.
    // Other sessions' hidden panes stay mounted — with split view (impl
    // 0020) each pane is gated per session, not per window.
    if (isSecondary) {
      startedKeyRef.current = null;
      if (sessionId) settleResume(sessionId);
      setStarting(false);
      return;
    }
    if (startedKeyRef.current === requestKey) return;
    startedKeyRef.current = requestKey;
    setErr(null);
    // Skip attach for archived rows: the workspace renders read-only,
    // so mounting RunnerTerminal would spawn a PTY listener for a row
    // that's terminal by definition.
    if (sessionId && chatMeta && !isArchived) {
      // `freshSpawn` is the only signal that distinguishes "user just
      // clicked Chat now and we're navigating into the spawn" from
      // "user clicked an existing chat row in the sidebar." We key
      // off chatMeta.started_at (loaded by the gate above) so the
      // pill only fires on rows whose PTY was born seconds ago.
      attach(
        sessionId,
        chatMeta.handle ? `@${chatMeta.handle}` : chatMeta.display_name,
        chatMeta.status,
        isFreshSpawn(chatMeta?.started_at),
      );
    }
  }, [
    attach,
    sessionId,
    state?.sessionStatus,
    isArchived,
    isSecondary,
    metaLoaded,
    chatMeta,
    chatMeta?.started_at,
    settleResume,
  ]);

  // ---- split view (impl 0020) ------------------------------------------

  // Pane the StartChatModal is filling (empty-pane flow); null = closed.
  const [paneModalTarget, setPaneModalTarget] = useState<string | null>(null);

  // Hydrate a restored layout: a persisted split can reference sessions
  // this app run has never attached (nothing navigated to them yet).
  // Attach every visible pane session that still exists in the
  // recent-direct list so its terminal mounts; sweep members that are
  // gone from the list (archived while the app was closed, or from
  // another window — session/archived refreshes the rows) out of the
  // layout so a stale pane can't linger with a phantom "running" status
  // or receive aggregate Stop/Resume/Archive actions. The URL session is
  // exempt: a freshly-spawned chat is assigned + navigated to before its
  // row's first appearance, and an in-flight rows fetch from just before
  // the spawn must not evict it from its pane.
  useEffect(() => {
    if (!splitActive || !rowsLoaded) return;
    for (const sid of visibleSessionIds(layout.root)) {
      if (directSessions.some((s) => s.id === sid)) continue;
      const row = recentRows.find((r) => r.session_id === sid);
      if (row) {
        attach(
          sid,
          row.handle ? `@${row.handle}` : row.display_name,
          row.status,
          false,
        );
      } else if (sid !== sessionId && !isFreshlyAssigned(sid)) {
        // React Router v7 defers navigate() via startTransition, so a
        // just-assigned chat hits this sweep one commit BEFORE it becomes
        // the URL session — the exemption above never sees it. Fresh
        // assignments are not "vanished while the app was closed"; leave
        // them alone until the rows catch up.
        removeSessionFromLayout(sid);
      }
    }
  }, [
    splitActive,
    rowsLoaded,
    layout,
    recentRows,
    directSessions,
    sessionId,
    attach,
  ]);

  // When the surface mounts onto an already-split layout (restored from
  // storage, or kept from a previous visit), align pane focus to the URL
  // session once — topbar/right rail already describe it. Not a standing
  // rule: while live, focus deliberately leads the URL (the empty-pane
  // flow parks focus on an empty pane whose session is still elsewhere).
  const [wasSplitAtMount] = useState(splitActive);
  const syncedRestoredFocusRef = useRef(false);
  useEffect(() => {
    if (syncedRestoredFocusRef.current || !wasSplitAtMount || !sessionId) {
      return;
    }
    syncedRestoredFocusRef.current = true;
    const restoredLeaf = leafForSession(
      getPaneLayout(sessionId).root,
      sessionId,
    );
    if (restoredLeaf) focusPane(restoredLeaf.id);
  }, [wasSplitAtMount, sessionId]);

  // Keyboard focus follows the URL session while split, so a sidebar row
  // click or page-nav shortcut lands keystrokes in the chat the user just
  // picked. Gated on the session sitting in the focused pane: the
  // empty-pane flow parks focus on an empty pane and must keep it.
  useEffect(() => {
    if (!splitActive || !sessionId) return;
    const paneLeaf = leafForSession(layout.root, sessionId);
    if (paneLeaf && paneLeaf.id === layout.focusedPaneId) {
      terminals.get(sessionId)?.focus();
    }
  }, [splitActive, sessionId, layout, terminals]);

  // Note there is deliberately NO "load the URL session into a pane"
  // effect: navigation must never mutate the group. A chat that isn't a
  // member renders single-pane (splitActive is false for it); membership
  // changes only through explicit acts — the preset picker, the
  // empty-pane StartChatModal, or a sidebar pick while an empty pane is
  // focused (handled in Sidebar).

  // Focus a pane and follow it with the URL (decision 7): topbar, right
  // rail, and deep-link state all key off `/chats/:sessionId`. Replace
  // instead of push so pane-hopping doesn't pile up history entries.
  const focusPaneAndFollow = useCallback(
    (leaf: PaneLeaf) => {
      // Only a member view may move the group's focus — a click inside an
      // ephemeral single-pane view (non-member chat) must not silently
      // shift the background group's focused pane.
      if (splitActive) focusPane(leaf.id);
      if (!leaf.sessionId) return;
      terminals.get(leaf.sessionId)?.focus();
      if (leaf.sessionId === sessionId) return;
      const paneStatus = directSessions.find(
        (s) => s.id === leaf.sessionId,
      )?.status;
      navigate(`/chats/${leaf.sessionId}`, {
        replace: true,
        state: paneStatus ? { sessionStatus: paneStatus } : undefined,
      });
    },
    [splitActive, sessionId, navigate, directSessions, terminals],
  );

  const pickPreset = useCallback(
    (kind: PresetKind) => {
      // Rearranging an ACTIVE tab keeps its members. Picking a preset
      // while viewing a non-member chat creates another pane tab seeded
      // from that chat; existing pane tabs stay backgrounded.
      const prev = getPaneLayout(sessionId);
      const prevActive = isGroupActiveFor(prev, sessionId ?? null);
      const visible = prevActive
        ? visibleSessionIds(prev.root)
        : sessionId
          ? [sessionId]
          : [];
      // Rearranging keeps the group's name; a fresh group starts unnamed.
      const next = applyPreset(
        kind,
        sessionId,
        visible,
        prevActive ? prev.name : null,
      );
      // More slots than open chats: focus already sits on the first empty
      // pane (applyPreset put it there) — funnel into StartChatModal with
      // the focused chat's runner preselected. Cancel leaves the empty
      // state; the sidebar can fill it too.
      const firstEmpty = leaves(next.root).find((l) => l.sessionId === null);
      if (firstEmpty) {
        setPaneModalTarget(firstEmpty.id);
        return;
      }
      const focused = findLeaf(next.root, next.focusedPaneId);
      if (focused?.sessionId && focused.sessionId !== sessionId) {
        navigate(`/chats/${focused.sessionId}`, { replace: true });
      }
    },
    [sessionId, navigate],
  );

  // Collapse a pane (sibling reflows); the session keeps running in the
  // hidden pool (decision 8). Fired by Cmd+W (focused pane) and the
  // empty pane's `×`. Follows the collapse with a navigation when the
  // surviving focused pane holds a different chat than the URL.
  const closePaneById = useCallback(
    (paneId: string) => {
      const next = closePane(paneId);
      const focused = findLeaf(next.root, next.focusedPaneId);
      if (focused?.sessionId && focused.sessionId !== sessionId) {
        navigate(`/chats/${focused.sessionId}`, { replace: true });
      }
    },
    [sessionId, navigate],
  );

  // Closing a pane only applies while split. Single-pane keeps the OS
  // default because this listener only exists while split.
  const closeFocusedPane = useCallback(() => {
    closePaneById(getPaneLayout(sessionId).focusedPaneId);
  }, [closePaneById, sessionId]);

  useEffect(() => {
    if (!splitActive) return;
    const onKey = (e: KeyboardEvent) => {
      if (!eventMatchesShortcut(e, "close-pane")) return;
      e.preventDefault();
      e.stopPropagation();
      closeFocusedPane();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, { capture: true });
  }, [splitActive, closeFocusedPane]);

  // Two entry points mirror the sidebar's pattern: a window capture
  // listener for ordinary keystrokes, plus RunnerTerminal's
  // re-dispatched custom event for keys delivered straight to xterm.
  const cyclePaneFocus = useCallback(
    (direction: "previous" | "next") => {
      const current = getPaneLayout(sessionId);
      if (current.root.kind !== "split") return;
      const all = leaves(current.root);
      const idx = all.findIndex((l) => l.id === current.focusedPaneId);
      if (idx === -1) return;
      const delta = direction === "next" ? 1 : -1;
      focusPaneAndFollow(all[(idx + delta + all.length) % all.length]);
    },
    [focusPaneAndFollow, sessionId],
  );

  useEffect(() => {
    if (!splitActive) return;
    const onKey = (e: KeyboardEvent) => {
      const direction =
        eventMatchesShortcut(e, "pane-previous")
          ? "previous"
          : eventMatchesShortcut(e, "pane-next")
            ? "next"
            : null;
      if (!direction) return;
      e.preventDefault();
      e.stopPropagation();
      cyclePaneFocus(direction);
    };
    const onCycle = (event: Event) => {
      const direction = (event as CustomEvent<{ direction?: unknown }>).detail
        ?.direction;
      if (direction === "previous" || direction === "next") {
        cyclePaneFocus(direction);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener(RUNNER_TERMINAL_CYCLE_EVENT, onCycle);
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener(RUNNER_TERMINAL_CYCLE_EVENT, onCycle);
    };
  }, [splitActive, cyclePaneFocus]);

  // Stop/resume take a session id: split pane headers control their own
  // session, the topbar controls the URL chat (single) or every visible
  // pane (Stop all / Resume all).
  const stopSession = useCallback(
    async (targetId: string) => {
      killedSessionsRef.current.add(targetId);
      clearActivity(targetId);
      try {
        // session_kill blocks until the reader thread reaps the child
        // and emits session/exit. The exit listener should flip the
        // pane status, but we also do it eagerly here so the header
        // status pill and Stop → Back-to-runner swap don't depend on
        // the event reaching this component (e.g. if the user navigated
        // mid-kill, RunnerTerminal may be unmounted before the listener
        // processes the event).
        await api.session.kill(targetId);
        setDirectSessions((prev) =>
          prev.map((s) =>
            s.id === targetId ? { ...s, status: "stopped" } : s,
          ),
        );
        void refreshChatMeta();
      } catch (e) {
        setErr(String(e));
      }
    },
    [clearActivity, refreshChatMeta],
  );

  // Resume path mirrors Pencil node `GZhHO` — the cyan transitional
  // state. Calls `session_resume`, which respawns a PTY for the same
  // row id and hands the agent CLI its prior `agent_session_key`.
  // Sequence:
  //   1. Flip into resuming mode. The mounted RunnerTerminal keeps
  //      its xterm canvas — the transitional overlay is opacity-0 on
  //      top of it. The backend stamps a resume watermark at the
  //      current seq; claude-code keeps its output ring (impl 0024 —
  //      a remount replays pre-resume scrollback like a terminal
  //      emulator would), codex still purges (its full-frame repaint
  //      would stack over retained frames).
  //   2. Await the resume RPC. The new PTY's first chunk continues
  //      the seq counter (never reset across resume) so the live
  //      listener doesn't drop the agent's repaint.
  //   3. Flip the local pane status back to running. Clearing the
  //      resuming id waits for the agent's repaint (ResumeSettleTracker).
  const resumeSession = useCallback(
    async (targetId: string) => {
      setResumingIds((prev) => new Set(prev).add(targetId));
      setErr(null);
      clearActivity(targetId);
      try {
        // Null dims make the backend fork the PTY at its 80×24
        // default, and a TUI prints its banner into that geometry
        // before the attach resize lands — the banner bytes then
        // render permanently garbled in the pane-width xterm (the
        // agent's SIGWINCH repaint only redraws the live region).
        // measure() can be null right after a restart (terminal
        // still at the construction sentinel until geometry sync
        // lands), so give it a few frames before giving up.
        let dims = terminals.get(targetId)?.measure() ?? null;
        for (let attempt = 0; attempt < 3 && !dims; attempt += 1) {
          await new Promise<void>((resolve) =>
            window.requestAnimationFrame(() => resolve()),
          );
          dims = terminals.get(targetId)?.measure() ?? null;
        }
        await api.session.resume(
          targetId,
          dims?.cols ?? null,
          dims?.rows ?? null,
        );
        setDirectSessions((prev) =>
          prev.map((s) =>
            s.id === targetId
              ? { ...s, status: "running", exitCode: null }
              : s,
          ),
        );
        void refreshChatMeta();
      } catch (e) {
        setErr(String(e));
        settleResume(targetId);
      }
    },
    [clearActivity, refreshChatMeta, settleResume, terminals],
  );

  // Aggregate topbar actions while split: act on every visible pane.
  const visiblePaneSessions = splitActive
    ? visibleSessionIds(layout.root).map((id) => ({
        id,
        status:
          directSessions.find((s) => s.id === id)?.status ??
          recentRows.find((r) => r.session_id === id)?.status ??
          "running",
      }))
    : [];
  const anyPaneRunning = visiblePaneSessions.some(
    (s) => s.status === "running",
  );
  const anyPaneResuming = visiblePaneSessions.some((s) =>
    resumingIds.has(s.id),
  );
  const stopAllPanes = () => {
    for (const s of visiblePaneSessions) {
      if (s.status === "running") void stopSession(s.id);
    }
  };
  const resumeAllPanes = () => {
    for (const s of visiblePaneSessions) {
      if (s.status !== "running" && !resumingIds.has(s.id)) {
        void resumeSession(s.id);
      }
    }
  };
  // Sequential, background panes first: each archive empties its pane,
  // and the URL chat goes last so its handler performs a single final
  // navigation (no surviving member left → back to the runner page).
  // Concurrent archives would race the member-handoff navigation.
  const archiveAllPanes = async () => {
    for (const s of visiblePaneSessions) {
      if (s.id !== sessionId) await archiveSession(s.id);
    }
    if (sessionId && visiblePaneSessions.some((s) => s.id === sessionId)) {
      await archiveSession(sessionId);
    }
  };

  // Topbar kebab open/close. Mirrors `MissionKebab` in
  // `MissionWorkspace`; the design's `session_ctx_menu` (Pin /
  // Rename / Archive) is the single shape both surfaces converge on.
  const [kebabOpen, setKebabOpen] = useState(false);

  const togglePin = useCallback(async () => {
    if (!sessionId || !chatMeta) return;
    try {
      await api.session.pin(sessionId, !chatMeta.pinned);
      await refreshChatMeta();
    } catch (e) {
      setErr(String(e));
    }
  }, [sessionId, chatMeta, refreshChatMeta]);

  // While split, Rename names the GROUP (persisted with the layout);
  // blank clears back to the derived member-names title.
  const renameGroupPrompt = useCallback(() => {
    const proposed = window.prompt(
      "Rename group (blank = derive from chats)",
      getPaneLayout(sessionId).name ?? "",
    );
    if (proposed === null) return;
    setGroupName(proposed);
  }, [sessionId]);

  // Topbar rename uses `window.prompt()` for the same reason the
  // mission topbar does — keeps the header layout fixed and avoids
  // fiddly focus management around a button-edge input. The sidebar
  // still owns the inline-rename affordance for power users.
  const renameChatPrompt = useCallback(async () => {
    if (!sessionId) return;
    const current =
      chatMeta?.title ??
      (chatMeta?.handle ? `@${chatMeta.handle}` : (chatMeta?.display_name ?? ""));
    const next = window.prompt("Rename chat", current);
    if (next === null) return; // cancelled
    const trimmed = next.trim();
    if (!trimmed || trimmed === current) return;
    try {
      await api.session.rename(sessionId, trimmed);
      await refreshChatMeta();
    } catch (e) {
      setErr(String(e));
    }
  }, [sessionId, chatMeta, refreshChatMeta]);

  // Archive: hide this chat from the sidebar's SESSION tray. The row
  // stays in the DB so a future Archived workspace surface can list it,
  // but it's gone from the live tray. Per-session, like stop/resume: a
  // split pane's card archives its own chat and the pane empties in
  // place; navigation only happens when the archived chat owned the URL —
  // preferring a surviving group member over leaving the surface.
  // Mirrors `Sidebar.archiveSession`: the backend refuses to archive a
  // running row (`commands::session::session_archive` → "kill before
  // archiving"), so kill first when the row is live.
  const archiveSession = useCallback(
    async (targetId: string) => {
      const effectiveStatus =
        directSessions.find((s) => s.id === targetId)?.status ??
        recentRows.find((r) => r.session_id === targetId)?.status;
      markArchivingSession(targetId);
      clearActivity(targetId);
      try {
        if (effectiveStatus === "running") {
          // Mark the kill as user-initiated so the exit handler reads it
          // as "stopped" rather than "crashed" (matches stopSession's
          // pattern). Without this the sidebar would briefly show a
          // crashed row before the archive RPC removes it.
          killedSessionsRef.current.add(targetId);
          try {
            await api.session.kill(targetId);
          } catch (e) {
            // The terminal exit event can beat the SQLite metadata
            // refresh. If the process is already gone, archive can
            // still succeed; if it is truly running, the backend
            // archive call below remains the guardrail.
            console.warn("RunnerChat: session_kill before archive failed", e);
          }
        }
        await api.session.archive(targetId);
        // Empty the pane that showed it (no-op when not visible).
        removeArchivedSessionFromLayout(targetId);
        if (targetId === sessionId) {
          const survivors = visibleSessionIds(getPaneLayout(sessionId).root);
          const next = survivors.find((id) => id !== targetId);
          if (next) {
            const nextLeaf = leafForSession(getPaneLayout(sessionId).root, next);
            if (nextLeaf) focusPane(nextLeaf.id);
            navigate(`/chats/${next}`, { replace: true });
          } else {
            navigate(backTarget);
          }
        }
      } catch (e) {
        setErr(String(e));
      } finally {
        // Same defer as Sidebar.archiveSession — see that finally for
        // the full rationale on the React-18 batched-emit race.
        setTimeout(() => unmarkArchivingSession(targetId), 0);
      }
    },
    [
      directSessions,
      recentRows,
      sessionId,
      navigate,
      backTarget,
      clearActivity,
    ],
  );

  const titleLabel =
    chatMeta?.title ??
    (chatMeta?.handle
      ? `@${chatMeta.handle}`
      : (chatMeta?.display_name ?? "chat"));
  // Padding wrapper around the xterm canvas tracks the current
  // terminal palette's background so the canvas + frame stay
  // seamless across theme switches.
  const terminalBg = useTerminalBg();
  const metaParts = [
    chatMeta
      ? chatMeta.handle
        ? `${chatMeta.agent_runtime}-${chatMeta.handle}`
        : chatMeta.agent_runtime
      : null,
    chatMeta?.started_at
      ? `started ${formatRelative(chatMeta.started_at)}`
      : null,
    chatMeta?.cwd ?? runner?.working_dir ?? null,
    exitCode != null ? `exit ${exitCode}` : null,
  ].filter((s): s is string => !!s);

  // ---- pane-group view model (impl 0020) --------------------------------

  const paneRow = (sid: string) =>
    recentRows.find((r) => r.session_id === sid) ?? null;
  const paneNameFor = (sid: string) => {
    const row = paneRow(sid);
    if (row) {
      return row.title ?? (row.handle ? `@${row.handle}` : row.display_name);
    }
    return directSessions.find((s) => s.id === sid)?.label ?? "chat";
  };
  const paneStatusFor = (sid: string): DirectChatDisplayStatus => {
    const live =
      directSessions.find((s) => s.id === sid)?.status ??
      paneRow(sid)?.status ??
      "running";
    return directChatDisplayStatus(live, activityBySession[sid]);
  };
  const paneRuntimeFor = (sid: string) =>
    paneRow(sid)?.agent_runtime ??
    chatMeta?.agent_runtime ??
    runner?.runtime ??
    "";

  // Group identity for the topbar while split (the controls up there are
  // group-scoped — Stop all / Resume all / Archive all — so the title,
  // chip, and meta describe the group, not one member). Name:
  // user-given via kebab Rename, else derived from member chat names.
  const paneCount = splitActive ? leaves(layout.root).length : 0;
  const groupTitle = splitActive
    ? (layout.name ??
      (visiblePaneSessions.length > 0
        ? visiblePaneSessions.map((s) => paneNameFor(s.id)).join(" + ")
        : "Empty group"))
    : null;
  // Meta: pane count, plus the working dir when every member shares one.
  const groupCwds = visiblePaneSessions.map((s) => paneRow(s.id)?.cwd ?? null);
  const sharedGroupCwd =
    groupCwds.length > 0 && groupCwds.every((c) => c != null && c === groupCwds[0])
      ? groupCwds[0]
      : null;
  const groupMetaParts = [
    `${paneCount} ${paneCount === 1 ? "pane" : "panes"}`,
    sharedGroupCwd,
  ].filter((s): s is string => !!s);

  // Per-pane transitional flag: a session mid-resume, or the URL session
  // mid-start. Blanks the canvas so the pill reads on a pristine surface.
  const transitionalFor = (sid: string) =>
    resumingIds.has(sid) || (sid === sessionId && starting);

  // Per-pane overlay. Every pane is self-contained: a stopped pane shows
  // its own scrim + Chat-paused card (Resume scoped to that session), a
  // resuming pane its own pill — after Stop all, each pane must visibly
  // read stopped, not just the focused one. The URL session's pane
  // additionally carries the archiving/starting states and the card's
  // Archive action (archiveChat navigates away, so it stays URL-scoped).
  // When a session is secondary (owned by another window) its wrapper
  // doesn't mount at all, which suppresses these in favor of the
  // per-pane duplicate overlay.
  const overlayFor = (sid: string) => {
    if (sid !== sessionId) {
      // Archiving wins over the ended card, mirroring the URL chain —
      // "Session ended" mid-archive would be misleading.
      if (isArchivingSession(sid)) return <ArchivingOverlay withScrim />;
      if (resumingIds.has(sid)) return <ResumingOverlay />;
      const pane = directSessions.find((s) => s.id === sid);
      const paneStatus = pane?.status ?? paneRow(sid)?.status ?? "running";
      if (paneStatus === "running") return null;
      return (
        <SessionEndedOverlay
          status={paneStatus}
          exitCode={pane?.exitCode ?? null}
          resumable={paneRow(sid)?.resumable ?? true}
          onResume={() => void resumeSession(sid)}
          onArchive={() => void archiveSession(sid)}
          variant="inline"
        />
      );
    }
    return archiving ? (
      <ArchivingOverlay withScrim />
    ) : resuming ? (
      <ResumingOverlay />
    ) : starting && activeSession ? (
      <StartingOverlay label="Starting chat…" />
    ) : activeSession && status !== "running" ? (
      <SessionEndedOverlay
        status={status}
        exitCode={exitCode}
        // chatMeta.resumable is `true` iff agent_session_key is non-NULL on
        // the row; false means Resume starts a fresh agent process and the
        // overlay copy shouldn't promise a preserved conversation. Default
        // true while chatMeta is loading to avoid a mislabel flash.
        resumable={chatMeta?.resumable ?? true}
        onResume={() => void resumeSession(sid)}
        onArchive={() => void archiveSession(sid)}
        variant="inline"
      />
    ) : null;
  };

  return (
    <div className="flex h-full flex-1 flex-row bg-bg">
      <div className="flex min-w-0 flex-1 flex-col">
      {/* `data-tauri-drag-region` makes the entire header strip drag
          the window, matching macOS toolbar behavior. Interactive
          children (Stop / Resume / kebab / panel toggle) keep their
          click handlers — Tauri only enters drag mode on mousedowns
          that land on the bare header, not on buttons. */}
      <header
        data-tauri-drag-region
        className="flex items-center justify-between gap-4 border-b border-line bg-panel px-6 pb-3.5 pt-9"
      >
        <div className="flex min-w-0 items-center gap-3.5">
          {/* Avatar — 36×36, matches MissionWorkspace's mission glyph
              dimensions so the chat + mission headers line up at the
              same baseline. */}
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-line bg-bg">
            {splitActive ? (
              <SquareSplitHorizontal
                aria-hidden
                className="h-[18px] w-[18px] text-accent"
              />
            ) : (
              <Terminal aria-hidden className="h-[18px] w-[18px] text-accent" />
            )}
          </div>
          {/* Title block — typography + gaps mirror
              MissionWorkspace's header so the bottom border lands at
              the same y on both pages. Title was previously
              font-mono text-[15px] which made chat's title row
              ~2px taller, pushing its meta row down and offsetting
              the divider visually. */}
          <div className="flex min-w-0 flex-col gap-0.5">
            <div className="flex items-center gap-2">
              {/* While split the topbar describes the GROUP — its
                  controls (Stop all / Resume all / Archive all) are
                  group-scoped, so the identity must be too. Name is
                  user-given (kebab → Rename group) or derived from the
                  member chats. */}
              <h1 className="truncate text-[14px] font-semibold leading-tight text-fg">
                {splitActive ? groupTitle : titleLabel}
              </h1>
              <span className="rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                {splitActive ? "Group" : "Chat"}
              </span>
              {isArchived ? (
                <span className="inline-flex shrink-0 items-center rounded border border-line bg-raised px-2 py-0.5 text-[10px] font-medium text-fg-2">
                  Archived · read-only
                </span>
              ) : null}
            </div>
            {(splitActive ? groupMetaParts : metaParts).length > 0 ? (
              <div className="flex min-w-0 items-center gap-2 text-[11px] leading-tight text-fg-3">
                {(splitActive ? groupMetaParts : metaParts).map((part, i) => (
                  <span
                    key={i}
                    className={`truncate ${
                      i > 0 ? "before:mr-2 before:text-line-strong before:content-['·']" : ""
                    } ${i === 0 || (i > 0 && part.startsWith("/")) ? "font-mono" : ""}`}
                  >
                    {part}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {/* Layout button — left of Stop per the Pencil mock (`z1hPN`).
              Hidden for archived rows (read-only single pane). */}
          {sessionId && metaLoaded && !isArchived ? (
            // Highlight reflects the current VIEW: a non-member chat reads
            // as single-pane even while a group exists in the background.
            <LayoutPicker
              active={splitActive ? layout.preset : "single"}
              onPick={pickPreset}
            />
          ) : null}
          {isArchived ? (
            // Archived rows are terminal: no Resume / Stop / Archive.
            // Only surface the navigation escape hatch.
            <BackButton onClick={() => navigate(backTarget)}>{backLabel}</BackButton>
          ) : isSecondary ? (
            // Secondary window (impl 0018): the primary owns the PTY, so no
            // Stop/Resume here — those call session_kill / session_resume.
            // Focus the primary (via the overlay) to act on this chat.
            <BackButton onClick={() => navigate(backTarget)}>{backLabel}</BackButton>
          ) : splitActive && visiblePaneSessions.length > 0 ? (
            // Aggregate controls while split — per-pane Stop/Resume live
            // in the pane headers; the topbar acts on every visible pane.
            anyPaneRunning ? (
              <StopButton onClick={stopAllPanes}>Stop all</StopButton>
            ) : anyPaneResuming ? (
              <ResumingButton />
            ) : (
              <ResumeButton onClick={resumeAllPanes}>Resume all</ResumeButton>
            )
          ) : resuming ? (
            <ResumingButton />
          ) : status === "running" && sessionId ? (
            <StopButton onClick={() => void stopSession(sessionId)} />
          ) : sessionId ? (
            // Stopped/crashed → Resume, matching
            // Pencil node `HLXK6` in `vS5ce`. Same action the inline
            // SessionEndedOverlay card fires; mirroring it in the
            // topbar lets the user recover without scrolling to the
            // bottom of the feed. If `agent_session_key` is missing,
            // the backend resume path starts a fresh agent process
            // for the same row; the overlay subtitle explains that
            // history is unavailable.
            <ResumeButton onClick={() => void resumeSession(sessionId)} />
          ) : (
            // Last-resort fallback: no session yet.
            <BackButton onClick={() => navigate(backTarget)}>{backLabel}</BackButton>
          )}
          {/* Topbar overflow menu — Pin / Rename / Archive, matching
              the design's `session_ctx_menu` (`P5CLA` / `L31Zb`) and
              the mission topbar's `MissionKebab`. Hidden for archived
              rows: Pin/Rename make no sense for a terminal row, and
              Archive is a no-op. */}
          {sessionId && chatMeta && !isArchived && !isSecondary ? (
            <ChatKebab
              pinned={chatMeta.pinned}
              open={kebabOpen}
              onToggle={() => setKebabOpen((v) => !v)}
              onClose={() => setKebabOpen(false)}
              // Group mode: Pin is a per-chat sidebar concept, so it hides;
              // Rename names the group; Archive archives every pane.
              showPin={!splitActive}
              onPin={() => {
                setKebabOpen(false);
                void togglePin();
              }}
              renameLabel={splitActive ? "Rename group" : "Rename"}
              onRename={() => {
                setKebabOpen(false);
                if (splitActive) renameGroupPrompt();
                else void renameChatPrompt();
              }}
              archiveLabel={splitActive ? "Archive all" : "Archive"}
              onArchive={() => {
                setKebabOpen(false);
                if (splitActive) void archiveAllPanes();
                else if (sessionId) void archiveSession(sessionId);
              }}
            />
          ) : null}
          {/* Panel-toggle button — only rendered in the topbar when
              the side panel is collapsed (matches Pencil node `QfoJJ`).
              When the panel is open, the toggle lives inside the
              panel's own header at the top-right (see
              RunnerSidePanel). */}
          {!panelOpen ? (
            <button
              type="button"
              onClick={() => setPanelOpen(true)}
              title="Open side panel"
              aria-label="Open side panel"
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-2 hover:bg-raised hover:text-fg"
            >
              {/* Hollow = "panel exists but is collapsed"; flips to the
                  filled trailing column when the panel is open. Mirror of
                  the left sidebar toggle (#246), oriented to the right. */}
              <PanelToggleGlyph
                side="right"
                filled={false}
                className="h-[12.5px] w-[16px]"
              />
            </button>
          ) : null}
        </div>
      </header>

      {err ? (
        <div className="mx-8 mt-4 rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
          {err}
        </div>
      ) : null}

      {warning ? (
        <div className="mx-8 mt-4 flex items-start justify-between gap-3 rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn">
          <span>{warning}</span>
          <button
            type="button"
            onClick={() => setWarning(null)}
            className="cursor-pointer text-xs text-warn/80 hover:text-warn"
          >
            Dismiss
          </button>
        </div>
      ) : null}

      {/* The pane group owns everything between topbar and rails: pane
          chrome, the flat terminal stack (one xterm per session, hidden
          panes keep buffering), geometry sync, and per-pane overlays. A
          single chat is a group of one — same render path as a split.
          Archived rows render a static placeholder instead — no xterm,
          no PTY listener, no live data path. */}
      <div className="relative flex-1 overflow-hidden">
        {isArchived ? (
          <div className="flex h-full items-center justify-center">
            <div className="flex max-w-md flex-col items-center gap-2 rounded border border-line bg-raised px-6 py-5 text-center">
              <span className="text-[13px] font-semibold text-fg">
                Session ended — terminal closed
              </span>
              <span className="text-[12px] text-fg-2">
                This chat was archived. The PTY is gone and the workspace is
                read-only.
              </span>
            </div>
          </div>
        ) : sessionId ? (
          // The group stays mounted through navigation — chatMeta refetch
          // gaps must not unmount every xterm (each remount replays the
          // backend snapshot; keeping terminals alive is both faster and
          // avoids replay artifacts). Archived-row PTY safety is enforced
          // per session: the attach effect only attaches the URL session
          // after chatMeta resolves un-archived, so an archived deep link
          // renders an empty group until the branch above takes over.
          <>
            <ChatPaneGroup
              layout={viewLayout}
              grouped={splitActive}
              chats={directSessions}
              transitionalFor={transitionalFor}
              overlayFor={overlayFor}
              resumingFor={(sid) => resumingIds.has(sid)}
              onStopSession={(sid) => void stopSession(sid)}
              onResumeSession={(sid) => void resumeSession(sid)}
              onArchiveSession={(sid) => void archiveSession(sid)}
              onClosePane={closePaneById}
              terminalBg={terminalBg}
              nameFor={paneNameFor}
              statusFor={paneStatusFor}
              runtimeFor={paneRuntimeFor}
              secondaryFor={(sid) => secondaryBySession.get(sid)}
              dismissedSecondary={dismissedSecondary}
              terminalRefFor={(sid) => terminals.refFor(sid)}
              onFocusPane={focusPaneAndFollow}
              onNewChat={(paneId) => {
                focusPane(paneId);
                setPaneModalTarget(paneId);
              }}
              onDismissSecondary={(sid) =>
                setDismissedSecondary((prev) => new Set(prev).add(sid))
              }
              onTerminalExit={onTerminalExit}
              onTerminalError={setErr}
            />
            {/* Delayed pill while the URL session's pane is still empty
                (terminal not attached yet) — single-chat view only; group
                members hydrate quietly. */}
            {!splitActive && showNavLoadingPill ? (
              <StartingOverlay label="Starting chat…" />
            ) : null}
          </>
        ) : showNavLoadingPill ? (
          // Neutral loading state until chatMeta resolves — gated on the
          // delayed flag so the common fast-IPC path doesn't flash on
          // every chat-to-chat navigation.
          <StartingOverlay label="Starting chat…" />
        ) : null}
      </div>
      </div>
      <RunnerSidePanel
        runner={runner}
        chatMeta={chatMeta}
        open={panelOpen}
        onClose={() => setPanelOpen(false)}
      />
      {/* Empty-pane fill flow (impl 0020): auto-opened when a preset has
          more slots than open chats, or from an empty pane's New chat
          button. The focused chat's runner is preselected; the spawned
          session lands in the target pane. */}
      <StartChatModal
        open={paneModalTarget !== null}
        project={activeProject}
        onClose={() => setPaneModalTarget(null)}
        defaultRunnerId={chatMeta?.runner_id ?? undefined}
        onStarted={(spawned) => {
          const target = paneModalTarget;
          setPaneModalTarget(null);
          activatePaneLayoutForSession(sessionId);
          console.info(
            `[pane-fill] spawned=${spawned.id} target=${target ?? "NULL"} ` +
              `panes=[${visibleSessionIds(getPaneLayout(sessionId).root).join(
                ",",
              )}]`,
          );
          if (target) {
            // Read members before the assign fills the target pane.
            const memberIds = visibleSessionIds(getPaneLayout(sessionId).root);
            assignSessionToPane(target, spawned.id);
            focusPane(target);
            console.info(
              `[pane-fill] after-assign panes=[${visibleSessionIds(
                getPaneLayout(spawned.id).root,
              ).join(",")}]`,
            );
            reportSubjectsNow(
              visibleSessionIds(getPaneLayout(spawned.id).root).map(
                (value) => ({
                  type: "DirectChat",
                  value,
                }),
              ),
            );
            void inheritGroupPin(memberIds, spawned.id);
          }
          navigate(`/chats/${spawned.id}`, {
            state: { sessionStatus: "running" },
          });
        }}
      />
      {[...resumingIds].map((id) => (
        <ResumeSettleTracker key={id} sessionId={id} onSettled={settleResume} />
      ))}
    </div>
  );
}

/// Clears a session's resuming state once the agent has settled on a
/// steady frame. Heuristic: wait for the first output chunk, then for
/// output to go idle for ~400ms (TUIs like codex/claude-code emit their
/// banner + prompt frame as a burst of chunks; idle = paint done), with a
/// 1s minimum visible duration so the loader doesn't flash on fast paints
/// and a hard 10s fallback for the pathological silent-agent case. One
/// tracker mounts per in-flight resume — split panes resume concurrently.
///
/// No snapshot fast-path here (the starting-pill effect uses one): the new
/// PTY hasn't been forked yet when this mounts — `resumeSession` calls
/// `api.session.resume` concurrently, and the backend purges the output
/// buffer at the *start* of resume. A snapshot launched alongside would
/// race the purge and could see the pre-stop session's stale
/// `\x1b[?2004h`, clearing the overlay before the new PTY exists. The
/// live listener alone is fine: resume's RPC is fast (~200ms), so the
/// listener attaches well before the new TUI emits its ready signal.
function ResumeSettleTracker({
  sessionId,
  onSettled,
}: {
  sessionId: string;
  onSettled: (id: string) => void;
}) {
  useEffect(() => {
    const RESUMING_MIN_VISIBLE_MS = 1000;
    const RESUMING_IDLE_DEBOUNCE_MS = 400;
    const RESUMING_HARD_TIMEOUT_MS = 10_000;
    const startTs = performance.now();
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    let idleTimer: number | null = null;

    const finish = () => {
      if (!cancelled) onSettled(sessionId);
    };

    const scheduleIdleTimer = () => {
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      const elapsed = performance.now() - startTs;
      const delay = Math.max(
        RESUMING_IDLE_DEBOUNCE_MS,
        RESUMING_MIN_VISIBLE_MS - elapsed,
      );
      idleTimer = window.setTimeout(finish, delay);
    };

    // Hard fallback so a silent agent never strands the loader.
    const hardTimeout = window.setTimeout(finish, RESUMING_HARD_TIMEOUT_MS);

    void listen<{ session_id: string; data: string }>(
      "session/output",
      (event) => {
        if (event.payload.session_id !== sessionId) return;
        // Clear as soon as the resumed TUI is wired up to accept
        // input, not after first-reply silence. See
        // `chunkIndicatesTuiReady`.
        if (chunkIndicatesTuiReady(event.payload.data)) {
          finish();
          return;
        }
        scheduleIdleTimer();
      },
    ).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      window.clearTimeout(hardTimeout);
      unlisten?.();
    };
  }, [sessionId, onSettled]);
  return null;
}

// SessionEndedOverlay + ResumingOverlay live in
// `../components/SessionEndedOverlay` and are shared with
// MissionWorkspace's slot PTY tabs.

// Right-side panel matching Pencil node `IFS3p` inside `GZhHO`. Spans
// the full height of the chat surface (the topbar lives in the chat
// column to its left, not above it). The panel-header strip mirrors
// the topbar's pt-9 + h-9 + pb-3.5 structure so the bottom divider
// lines up across both columns. The whole aside slides via a
// CSS width transition: open = persisted width, closed = 0 (with the
// inner wrapper kept at the persisted width and clipped by
// overflow-hidden so content doesn't reflow during the animation).
// A 4px drag strip on the left edge resizes the panel, mirroring the
// left sidebar's right-edge handle.
function RunnerSidePanel({
  runner,
  chatMeta,
  open,
  onClose,
}: {
  runner: Runner | null;
  chatMeta: DirectSessionEntry | null;
  open: boolean;
  onClose: () => void;
}) {
  const asideRef = useRef<HTMLElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const { width, onResizeStart } = useResizableWidth({
    storageKey: STORAGE_PANEL_WIDTH,
    defaultWidth: PANEL_DEFAULT,
    min: PANEL_MIN,
    max: PANEL_MAX,
    edge: "left",
    targets: [asideRef, innerRef],
  });
  // Show the left divider only once the panel is fully open, and drop it the
  // instant a collapse starts. Otherwise the visible border rides the
  // animating left edge and reads as a gray bar sliding across the main area
  // during the width transition (the left sidebar dodges this only because
  // its border color happens to match its own background).
  const [borderOn, setBorderOn] = useState(open);
  useEffect(() => {
    if (!open) setBorderOn(false);
  }, [open]);
  return (
    <aside
      ref={asideRef}
      aria-hidden={!open}
      onTransitionEnd={(e) => {
        if (e.propertyName === "width" && open) setBorderOn(true);
      }}
      style={{ width: open ? width : 0 }}
      className={`relative flex shrink-0 flex-col overflow-hidden bg-panel transition-[width] duration-200 ease-in-out ${
        borderOn ? "border-l border-line" : "border-l-0"
      }`}
    >
      <div ref={innerRef} style={{ width }} className="flex h-full flex-col">
        {/* Side-panel header — same draggable-window rules as the
            workspace topbar. The lone Collapse button keeps its
            handler; everywhere else the strip drags the window. */}
        <header
          data-tauri-drag-region
          className="flex shrink-0 items-center justify-end border-b border-line px-5 pb-3.5 pt-9"
        >
          <div className="flex h-9 items-center">
            <button
              type="button"
              onClick={onClose}
              title="Collapse panel"
              aria-label="Collapse panel"
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-2 hover:bg-raised hover:text-fg"
            >
              <PanelToggleGlyph
                side="right"
                filled
                className="h-[12.5px] w-[16px]"
              />
            </button>
          </div>
        </header>
        <div className="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto p-5">
          {runner ? (
            <>
              <section className="flex flex-col gap-2.5">
                <span className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
                  Runner
                </span>
                <div className="flex flex-col gap-2.5 rounded-lg border border-line-strong bg-bg p-3.5">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-[14px] font-semibold text-fg">
                      @{runner.handle}
                    </span>
                    <span className="rounded bg-line-strong px-1.5 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                      {runner.runtime}
                    </span>
                  </div>
                  {runner.display_name ? (
                    <p className="text-[12px] text-fg-2">
                      {runner.display_name}
                    </p>
                  ) : null}
                  <div className="h-px w-full bg-line" />
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-[11px]">
                    <dt className="text-fg-3">cmd</dt>
                    <dd className="break-all font-mono text-fg-2">
                      {runner.command}
                    </dd>
                    {runner.working_dir ? (
                      <>
                        <dt className="text-fg-3">cwd</dt>
                        <dd className="break-all font-mono text-fg-2">
                          {runner.working_dir}
                        </dd>
                      </>
                    ) : null}
                    {chatMeta ? (
                      <>
                        <dt className="text-fg-3">session_key</dt>
                        <dd className="flex min-w-0 items-start gap-1.5">
                          <span className="min-w-0 flex-1 break-all font-mono text-fg-2">
                            {chatMeta.agent_session_key ?? "NULL"}
                          </span>
                          <CopyValueButton
                            value={chatMeta.agent_session_key}
                            label="Copy session_key"
                          />
                        </dd>
                      </>
                    ) : null}
                  </dl>
                </div>
              </section>
              {runner.system_prompt ? (
                <section className="flex min-h-0 flex-col gap-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
                      System prompt
                    </span>
                  </div>
                  <div className="overflow-y-auto whitespace-pre-wrap break-words rounded-md border border-line-strong bg-bg p-3 font-sans text-[12px] leading-relaxed text-fg-2">
                    {runner.system_prompt}
                  </div>
                </section>
              ) : null}
            </>
          ) : chatMeta ? (
            <section className="flex flex-col gap-2.5">
              <span className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
                Runtime
              </span>
              <div className="flex flex-col gap-2.5 rounded-lg border border-line-strong bg-bg p-3.5">
                <div className="flex items-center gap-2">
                  <span className="text-[14px] font-semibold text-fg">
                    {chatMeta.display_name}
                  </span>
                  <span className="rounded bg-line-strong px-1.5 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                    {chatMeta.agent_runtime}
                  </span>
                </div>
                <div className="h-px w-full bg-line" />
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-[11px]">
                  <dt className="text-fg-3">cmd</dt>
                  <dd className="break-all font-mono text-fg-2">
                    {chatMeta.agent_command}
                  </dd>
                  {chatMeta.cwd ? (
                    <>
                      <dt className="text-fg-3">cwd</dt>
                      <dd className="break-all font-mono text-fg-2">
                        {chatMeta.cwd}
                      </dd>
                    </>
                  ) : null}
                  <dt className="text-fg-3">session_key</dt>
                  <dd className="flex min-w-0 items-start gap-1.5">
                    <span className="min-w-0 flex-1 break-all font-mono text-fg-2">
                      {chatMeta.agent_session_key ?? "NULL"}
                    </span>
                    <CopyValueButton
                      value={chatMeta.agent_session_key}
                      label="Copy session_key"
                    />
                  </dd>
                </dl>
              </div>
            </section>
          ) : (
            <p className="text-xs text-fg-3">Loading chat…</p>
          )}
        </div>
      </div>
      {/* Drag-to-resize strip on the left edge — mirrors the left
          sidebar's right-edge handle. Inert when collapsed: the
          aside's overflow-hidden + width:0 already hides it, but we
          also skip the mousedown binding so the cursor doesn't
          briefly read as resizable inside the chat column. */}
      <div
        onPointerDown={open ? onResizeStart : undefined}
        title={open ? "Drag to resize" : undefined}
        className={
          open
            ? "absolute left-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
            : "absolute left-0 top-0 z-20 h-full w-1 bg-transparent"
        }
      />
    </aside>
  );
}

/// Topbar overflow menu for a direct chat. Pin / Rename / Archive —
/// same shape as `MissionKebab` and the design's `session_ctx_menu`
/// (Pencil node `P5CLA` in `u6woG`, `L31Zb` in `vS5ce`). Reset is
/// mission-only (a chat has no slots to respawn) so it's omitted
/// here.
function ChatKebab({
  pinned,
  open,
  onToggle,
  onClose,
  showPin = true,
  onPin,
  renameLabel = "Rename",
  onRename,
  onArchive,
  archiveLabel = "Archive",
}: {
  pinned: boolean;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  /** Hidden while split — Pin is a per-chat sidebar concept and the
   *  topbar kebab is group-scoped there. */
  showPin?: boolean;
  onPin: () => void;
  /** "Rename group" while split. */
  renameLabel?: string;
  onRename: () => void;
  onArchive: () => void;
  /** "Archive all" while split — the topbar aggregates over visible
   *  panes, mirroring Stop all / Resume all. */
  archiveLabel?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Chat actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={onToggle}
        className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-transparent text-fg-2 transition-colors hover:border-line hover:bg-raised hover:text-fg"
      >
        <MoreHorizontal aria-hidden className="h-4 w-4" />
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-50 mt-1.5 flex w-40 flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
        >
          {showPin ? (
            <KebabItem
              icon={pinned ? PinOff : Pin}
              label={pinned ? "Unpin" : "Pin"}
              onClick={onPin}
            />
          ) : null}
          <KebabItem icon={SquarePen} label={renameLabel} onClick={onRename} />
          <KebabItem
            icon={Archive}
            label={archiveLabel}
            onClick={onArchive}
            danger
          />
        </div>
      ) : null}
    </div>
  );
}

function KebabItem({
  icon: Icon,
  label,
  onClick,
  disabled,
  danger,
}: {
  icon: typeof Archive;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      className={`flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] hover:bg-line disabled:cursor-default disabled:opacity-50 disabled:hover:bg-transparent ${
        danger ? "text-danger" : "text-fg"
      }`}
    >
      <Icon aria-hidden className={`h-3.5 w-3.5 ${danger ? "text-danger" : "text-fg"}`} />
      <span>{label}</span>
    </button>
  );
}

// Compact relative time for the chat header meta line. Mirrors the
// "started 18m ago" text in the Pencil design. Falls back to a short
// absolute date for anything older than a week.
function formatRelative(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "—";
  const diffSec = Math.max(0, (Date.now() - d.getTime()) / 1000);
  if (diffSec < 60) return "just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 7) return `${diffDay}d ago`;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
