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

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  MoreHorizontal,
  PanelRight,
  PanelRightDashed,
  Pin,
  PinOff,
  SquarePen,
  Terminal,
} from "lucide-react";
import { Group, Panel, Separator } from "react-resizable-panels";

import {
  RunnerTerminal,
  type RunnerTerminalHandle,
} from "../components/RunnerTerminal";
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
import { chunkIndicatesTuiReady, isFreshSpawn } from "../lib/sessionLifecycle";
import { useDelayedFlag } from "../lib/useDelayedFlag";
import { useResizableWidth } from "../hooks/useResizableWidth";
import { useTerminalBg } from "../lib/useTerminalBg";
import {
  markArchivingSession,
  unmarkArchivingSession,
  useArchivingSession,
} from "../lib/archivingState";
import { DuplicateSubjectOverlay } from "../components/DuplicateSubjectOverlay";
import { LayoutPicker } from "../components/LayoutPicker";
import { StartChatModal } from "../components/StartChatModal";
import {
  applyPreset,
  assignSessionToPane,
  closePane,
  findLeaf,
  focusPane,
  getPaneLayout,
  isSplit,
  leafForSession,
  leaves,
  recordSplitSizes,
  resetPaneLayout,
  usePaneLayout,
  visibleSessionIds,
  type PaneLeaf,
  type PaneNode,
  type PresetKind,
} from "../lib/paneLayout";
import {
  isSecondaryFor,
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
const PANEL_MIN = 200;
const PANEL_MAX = 480;
const PANEL_DEFAULT = 320;

interface DirectSessionPane {
  id: string;
  label: string;
  status: SessionStatus;
  exitCode: number | null;
}

type DirectChatDisplayStatus = SessionActivityState | "stopped" | "crashed";

function directChatDisplayStatus(
  status: SessionStatus,
  activity: SessionActivityState | undefined,
): DirectChatDisplayStatus {
  if (status === "stopped" || status === "crashed") return status;
  return activity ?? "busy";
}

export default function RunnerChat() {
  const { sessionId: sessionIdParam } = useParams<{
    sessionId: string;
  }>();
  const location = useLocation();
  const navigate = useNavigate();
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
  const [recentRows, setRecentRows] = useState<DirectSessionEntry[]>([]);
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
  // True while the user has clicked Resume and we're waiting for the
  // resumed PTY to come back online. Drives the cyan status pill, the
  // header "Resuming…" affordance, and the centered Resuming pill
  // overlay on the cleared terminal canvas.
  const [resuming, setResuming] = useState<boolean>(false);
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
  const archiving = useArchivingSession(sessionId);
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
  const latestActivity = sessionId ? activityBySession[sessionId] : undefined;
  const displayStatus = directChatDisplayStatus(status, latestActivity);
  const exitCode = activeSession?.exitCode ?? null;
  const backTarget = chatMeta?.handle ? `/runners/${chatMeta.handle}` : "/runners";
  const backLabel = chatMeta?.handle ? "Back to runner" : "Back to runners";
  // Archived rows can be reached by direct URL but render read-only.
  // We don't attach a PTY, mount RunnerTerminal, or expose Resume /
  // End / Archive — the row is terminal and the operator can only
  // read the meta + go back to the runner.
  const isArchived = chatMeta?.archived_at != null;

  // Split-view layout (impl 0020). Module store shared with Sidebar; the
  // single-pane preset renders the classic path below, split presets render
  // the pane tree. Reset on surface unmount (decision 6).
  const layout = usePaneLayout();
  const splitActive = isSplit(layout);
  useEffect(() => () => resetPaneLayout(), []);

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
  const { secondary: isSecondary, primaryLabel } = (sessionId
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
  const showDuplicateOverlay =
    isSecondary && sessionId != null && !dismissedSecondary.has(sessionId);

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
    const targetId = sessionId;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<SessionUpdatedEvent>("session/updated", (event) => {
      if (event.payload.session_id !== targetId) return;
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

  // Clear `resuming` once the agent has settled on a steady frame.
  // Heuristic: wait for the first output chunk, then for output to
  // go idle for ~400ms (TUIs like codex/claude-code emit their
  // banner + prompt frame as a burst of chunks; idle = paint done).
  // Enforce a 1s minimum visible duration so the loader doesn't
  // flash on fast paints. Hard 10s fallback handles the pathological
  // silent-agent case (e.g. shell runtime that produces no output).
  useEffect(() => {
    if (!resuming || !sessionId) return;
    const RESUMING_MIN_VISIBLE_MS = 1000;
    const RESUMING_IDLE_DEBOUNCE_MS = 400;
    const RESUMING_HARD_TIMEOUT_MS = 10_000;
    const startTs = performance.now();
    const targetId = sessionId;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    let idleTimer: number | null = null;

    const finish = () => {
      if (!cancelled) setResuming(false);
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

    // No snapshot fast-path on resume. The starting-pill effect uses
    // one because the lead's PTY may have been alive for seconds
    // before the workspace mounts; here the new PTY hasn't been
    // forked yet when this effect fires — `resumeChat` calls
    // `api.session.resume` concurrently with this effect, and the
    // backend purges `output_buffers` at the *start* of resume. A
    // snapshot launched alongside resume would race the purge and
    // could see the pre-stop session's stale `\x1b[?2004h`, clearing
    // the overlay before the new PTY exists. The live listener
    // alone is fine: resume's RPC is fast (~200ms), so the listener
    // attaches well before the new TUI emits its ready signal.
    void listen<{ session_id: string; data: string }>(
      "session/output",
      (event) => {
        if (event.payload.session_id !== targetId) return;
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
  }, [resuming, sessionId]);

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
      if (!cancelled) setStarting(false);
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
    // TUI-ready escape before this effect's listener attached.
    void api.session
      .outputSnapshot(targetId)
      .then((snapshot) => {
        if (cancelled) return;
        if (snapshot.some((ev) => chunkIndicatesTuiReady(ev.data))) {
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
      setResuming(false);
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
  ]);

  // ---- split view (impl 0020) ------------------------------------------

  // Pane the StartChatModal is filling (empty-pane flow); null = closed.
  const [paneModalTarget, setPaneModalTarget] = useState<string | null>(null);

  // While split, a URL session that isn't on screen anywhere loads into
  // the focused pane — covers command-palette jumps and any plain
  // navigation that doesn't know about the layout. Deliberately narrow:
  // a session already visible in some pane is left alone, otherwise the
  // empty-pane flow (focus parked on the empty pane, modal open, URL
  // still on the origin chat) would yank the origin chat out of its
  // pane. The sidebar handles its own move-not-copy on row click.
  useEffect(() => {
    if (!splitActive || !sessionId || !metaLoaded || isArchived) return;
    if (leafForSession(layout.root, sessionId)) return;
    assignSessionToPane(layout.focusedPaneId, sessionId);
  }, [splitActive, sessionId, metaLoaded, isArchived, layout]);

  // Focus a pane and follow it with the URL (decision 7): topbar, right
  // rail, and deep-link state all key off `/chats/:sessionId`. Replace
  // instead of push so pane-hopping doesn't pile up history entries.
  const focusPaneAndFollow = useCallback(
    (leaf: PaneLeaf) => {
      focusPane(leaf.id);
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
    [sessionId, navigate, directSessions, terminals],
  );

  const pickPreset = useCallback(
    (kind: PresetKind) => {
      const prev = getPaneLayout();
      const visible =
        prev.root.kind === "split"
          ? visibleSessionIds(prev.root)
          : sessionId
            ? [sessionId]
            : [];
      const next = applyPreset(kind, sessionId, visible);
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

  // Cmd+W while split collapses the focused pane; the session keeps
  // running in the hidden pool (decision 8). Single-pane keeps the OS
  // default (Close Window) — this listener only exists while split.
  const closeFocusedPane = useCallback(() => {
    const next = closePane(getPaneLayout().focusedPaneId);
    const focused = findLeaf(next.root, next.focusedPaneId);
    if (focused?.sessionId && focused.sessionId !== sessionId) {
      navigate(`/chats/${focused.sessionId}`, { replace: true });
    }
  }, [sessionId, navigate]);

  useEffect(() => {
    if (!splitActive) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      if (e.key !== "w" && e.key !== "W") return;
      e.preventDefault();
      e.stopPropagation();
      closeFocusedPane();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, { capture: true });
  }, [splitActive, closeFocusedPane]);

  // Geometry sync (decision 4, geometry-sync variant). Terminals stay in
  // the flat absolutely-positioned stack they've always lived in — their
  // React tree position never changes, so xterm never remounts. Visible
  // sessions' wrappers are imperatively sized/positioned to their pane's
  // body rect; a ResizeObserver per pane body keeps them glued through
  // gutter drags, window resizes, and panel toggles. (A portal-per-pane
  // approach was rejected: React remounts portal children when the
  // container node changes — `updatePortal` compares `containerInfo` by
  // identity — which is exactly the remount this feature must avoid.)
  // All geometry bookkeeping lives in one closure object created once —
  // plain captured variables instead of refs, because the callback-ref
  // factories are invoked during render where touching `ref.current` is
  // off-limits (react-hooks/refs). Registration happens at commit time via
  // the returned callbacks; nothing here drives a React update.
  const [paneGeo] = useState(createPaneGeometry);
  useEffect(() => () => paneGeo.dispose(), [paneGeo]);

  // Position wrappers before paint on every commit — pane tree changes,
  // session assignment changes, and wrapper mounts all land here. The RO
  // inside paneGeo keeps them glued through gutter drags and resizes.
  // Leaving split mode clears the inline geometry, since it would
  // otherwise override the single-pane wrappers' `inset-0` sizing.
  useLayoutEffect(() => {
    paneGeo.setRoot(layout.root);
    if (splitActive) paneGeo.sync();
    else paneGeo.clearWrapGeometry();
  });

  async function endChat() {
    if (!sessionId) return;
    const targetId = sessionId;
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
  }

  // Resume path mirrors Pencil node `GZhHO` — the cyan transitional
  // state. Calls `session_resume`, which respawns a PTY for the same
  // row id and hands the agent CLI its prior `agent_session_key`.
  // Sequence:
  //   1. Flip into resuming mode and bump clearVersion so the active
  //      RunnerTerminal wipes its xterm canvas. The backend also
  //      drops the prior buffer in `purge_output_buffer`, so any
  //      remount during the resume window starts blank too.
  //   2. Await the resume RPC. The new PTY's first chunk continues
  //      the seq counter (we keep it across forget) so the live
  //      listener doesn't drop the agent's repaint.
  //   3. Flip the local pane status back to running. Clearing the
  //      `resuming` flag waits for chatMeta.status to confirm the
  //      DB-backed truth (the sync effect drives that).
  async function resumeChat() {
    if (!sessionId) return;
    const targetId = sessionId;
    setResuming(true);
    setErr(null);
    clearActivity(targetId);
    try {
      const dims = terminals.get(targetId)?.measure() ?? null;
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
      setResuming(false);
    }
  }

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
  // stays in the DB so a future Archived workspace surface can list
  // it, but it's gone from the live tray. We navigate back to the
  // appropriate parent surface. Mirrors `Sidebar.archiveSession`: the backend
  // refuses to archive a running row (`commands::session::session_archive`
  // → "kill before archiving"), so kill first when the row is live.
  async function archiveChat() {
    if (!sessionId) return;
    const targetId = sessionId;
    const effectiveStatus = activeSession?.status ?? chatMeta?.status;
    markArchivingSession(targetId);
    clearActivity(targetId);
    try {
      if (effectiveStatus === "running") {
        // Mark the kill as user-initiated so the exit handler reads it
        // as "stopped" rather than "crashed" (matches endChat's
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
      navigate(backTarget);
    } catch (e) {
      setErr(String(e));
    } finally {
      // Same defer as Sidebar.archiveSession — see that finally for
      // the full rationale on the React-18 batched-emit race.
      setTimeout(() => unmarkArchivingSession(targetId), 0);
    }
  }

  // Header layout mirrors Pencil node `NLa0k` inside `u6woG`:
  // 36px terminal-icon avatar, vertical title stack (handle + DIRECT
  // chip + meta line), and a right cluster of status pill + Stop +
  // kebab. `resuming` is a transitional control state; the steady
  // display model is busy / idle / stopped / crashed.
  type ChatState = DirectChatDisplayStatus | "resuming";
  const chatState: ChatState = resuming ? "resuming" : displayStatus;
  const statusBadgeClass =
    chatState === "busy"
      ? "bg-accent/10 text-accent"
      : chatState === "idle"
        ? "bg-accent/5 text-fg-2"
        : chatState === "crashed"
          ? "bg-danger/10 text-danger"
          : chatState === "resuming"
            ? "bg-info/15 text-info"
            : "bg-line-strong text-fg-2";
  const statusDotClass =
    chatState === "busy"
      ? "bg-accent"
      : chatState === "idle"
        ? "bg-accent/35"
        : chatState === "crashed"
          ? "bg-danger"
          : chatState === "resuming"
            ? "bg-info"
            : "bg-fg-3";
  const statusLabel = chatState === "resuming" ? "resuming…" : displayStatus;
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

  // ---- split-view render helpers (impl 0020) ----------------------------
  // Plain functions, not components: defining component types inside the
  // render body would give them a fresh identity every commit and remount
  // the whole pane tree (Group included) on every render.

  const paneRow = (sid: string) =>
    recentRows.find((r) => r.session_id === sid) ?? null;
  const paneName = (sid: string) => {
    const row = paneRow(sid);
    if (row) {
      return row.title ?? (row.handle ? `@${row.handle}` : row.display_name);
    }
    return directSessions.find((s) => s.id === sid)?.label ?? "chat";
  };
  const paneDisplayStatus = (sid: string): DirectChatDisplayStatus => {
    const live =
      directSessions.find((s) => s.id === sid)?.status ??
      paneRow(sid)?.status ??
      "running";
    return directChatDisplayStatus(live, activityBySession[sid]);
  };

  // Pane chrome per Pencil `g0xSY`/`vdRKp`: 34px header (icon · name ·
  // CHAT chip · status dot) over the body; the focused pane carries the
  // 1px accent ring. The body div is the geometry target the session's
  // terminal wrapper is glued to.
  const renderPaneNode = (node: PaneNode): ReactNode => {
    if (node.kind === "leaf") {
      const focused = node.id === layout.focusedPaneId;
      const sid = node.sessionId;
      const sec = sid ? secondaryBySession.get(sid) : undefined;
      const showPaneOverlay =
        sid != null && (sec?.secondary ?? false) && !dismissedSecondary.has(sid);
      const displayStatus = sid ? paneDisplayStatus(sid) : null;
      return (
        <section
          key={node.id}
          onMouseDownCapture={() => focusPaneAndFollow(node)}
          className={`relative flex h-full w-full flex-col overflow-hidden border ${
            focused ? "border-accent" : "border-line"
          }`}
        >
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
              {sid ? paneName(sid) : "Empty pane"}
            </span>
            <span className="shrink-0 rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
              Chat
            </span>
            {displayStatus ? (
              <span className="flex shrink-0 items-center gap-1.5">
                <span
                  className={`inline-block h-1.5 w-1.5 rounded-full ${paneStatusDotClass(displayStatus)}`}
                />
                <span className="text-[11px] text-fg-2">{displayStatus}</span>
              </span>
            ) : null}
          </header>
          <div
            ref={(el) => paneGeo.paneBodyRefFor(node.id)(el)}
            style={{ backgroundColor: terminalBg }}
            className="relative min-h-0 flex-1"
          >
            {sid == null ? (
              <EmptyPaneBody
                onNewChat={() => {
                  focusPane(node.id);
                  setPaneModalTarget(node.id);
                }}
              />
            ) : showPaneOverlay ? (
              // Per-pane duplicate-subject gate (impl 0018 × 0020): this
              // pane's session is owned by another window, so no terminal
              // is mounted for it (see renderTerminalPane) and the
              // overlay scopes to this pane only.
              <DuplicateSubjectOverlay
                kind="chat"
                primaryLabel={sec?.primaryLabel ?? null}
                onStayHere={() =>
                  setDismissedSecondary((prev) => new Set(prev).add(sid))
                }
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
          className={`shrink-0 transition-colors hover:bg-accent/40 ${
            horizontal ? "w-1.5" : "h-1.5"
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

  // Whether the flat terminal stack renders at all. Split mode keeps it up
  // unconditionally (bar archived deep links) — gating on `metaLoaded`
  // would unmount every xterm on each pane-focus navigation. Single-pane
  // keeps today's gates: no PTY mounts for archived rows, secondary
  // windows, or before chatMeta resolves.
  const renderStack = splitActive
    ? !isArchived
    : metaLoaded && !isArchived && !isSecondary;

  // One terminal wrapper per session, both modes. The element shape is
  // identical across modes (same key, same position, RunnerTerminal first
  // child), so single↔split transitions reuse the mounted terminals.
  // Single-pane: the URL session's wrapper is full-bleed (`inset-0`),
  // the rest stack hidden — exactly the classic behavior. Split: visible
  // wrappers are geometry-synced onto their pane bodies; a session owned
  // by another window (impl 0018) mounts nothing and its pane shows the
  // duplicate-subject overlay instead.
  const renderTerminalPane = (s: DirectSessionPane): ReactNode => {
    if (splitActive && secondaryBySession.get(s.id)?.secondary) return null;
    const paneLeaf = splitActive ? leafForSession(layout.root, s.id) : null;
    const isUrl = s.id === sessionId;
    const visible = splitActive ? paneLeaf !== null : isUrl;
    const transitional = isUrl && (resuming || starting);
    const dead = s.status !== "running";
    // Pane visual state: while resuming/starting the active pane is fully
    // blank so the centered cyan pill reads on a pristine canvas. When
    // stopped, the pane dims to 45% under the Session ended card.
    const paneOpacity =
      visible && transitional
        ? "opacity-0"
        : visible && dead
          ? "opacity-45"
          : "";
    return (
      <div
        key={s.id}
        ref={(el) => paneGeo.termWrapRefFor(s.id)(el)}
        // `backgroundColor` is inlined from `useTerminalBg()` so the
        // frame tracks the active terminal palette; a theme switch flips
        // the frame + canvas in lockstep.
        style={{ backgroundColor: terminalBg }}
        onMouseDownCapture={
          paneLeaf ? () => focusPaneAndFollow(paneLeaf) : undefined
        }
        className={`absolute ${splitActive ? "" : "inset-0"} p-4 ${visible ? "block" : "hidden"} ${paneOpacity} transition-opacity`}
      >
        <RunnerTerminal
          ref={(h) => terminals.refFor(s.id)(h)}
          sessionId={s.id}
          runnerRuntime={
            paneRow(s.id)?.agent_runtime ??
            chatMeta?.agent_runtime ??
            runner?.runtime ??
            ""
          }
          // While the resume/start loader is up the canvas is hidden, so
          // xterm behaves as inactive (no resize pushes, no focus); when
          // the flag clears, the activation effect fits + repaints.
          active={visible && !transitional}
          autoFocus={
            !splitActive || (visible && paneLeaf?.id === layout.focusedPaneId)
          }
          disabled={dead || transitional}
          onExit={onTerminalExit}
          onError={setErr}
        />
        {splitActive && isUrl ? (
          // Split mode scopes the URL session's transitional/session-ended
          // overlays to its own pane; single-pane keeps them page-level.
          archiving ? (
            <ArchivingOverlay withScrim />
          ) : chatState === "resuming" ? (
            <ResumingOverlay />
          ) : starting && activeSession ? (
            <StartingOverlay label="Starting chat…" />
          ) : activeSession && status !== "running" ? (
            <SessionEndedOverlay
              status={status}
              exitCode={exitCode}
              resumable={chatMeta?.resumable ?? true}
              onResume={() => void resumeChat()}
              onArchive={() => void archiveChat()}
              variant="inline"
            />
          ) : null
        ) : null}
      </div>
    );
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
            <Terminal aria-hidden className="h-[18px] w-[18px] text-accent" />
          </div>
          {/* Title block — typography + gaps mirror
              MissionWorkspace's header so the bottom border lands at
              the same y on both pages. Title was previously
              font-mono text-[15px] which made chat's title row
              ~2px taller, pushing its meta row down and offsetting
              the divider visually. */}
          <div className="flex min-w-0 flex-col gap-0.5">
            <div className="flex items-center gap-2">
              <h1 className="truncate text-[14px] font-semibold leading-tight text-fg">
                {titleLabel}
              </h1>
              <span className="rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                Chat
              </span>
              {/* Status pill moved next to the title so it stops
                  competing with the Stop / Resume control on the
                  right — the action button already implies the
                  current state, and a pill at the same edge read
                  redundant. */}
              <span
                className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${statusBadgeClass}`}
                title={`session ${sessionId ? sessionId.slice(-6) : "—"}`}
              >
                <span className={`inline-block h-1.5 w-1.5 rounded-full ${statusDotClass}`} />
                {sessionId ? statusLabel : "starting"}
              </span>
              {isArchived ? (
                <span className="inline-flex shrink-0 items-center rounded border border-line bg-raised px-2 py-0.5 text-[10px] font-medium text-fg-2">
                  Archived · read-only
                </span>
              ) : null}
            </div>
            {metaParts.length > 0 ? (
              <div className="flex min-w-0 items-center gap-2 text-[11px] leading-tight text-fg-3">
                {metaParts.map((part, i) => (
                  <span
                    key={i}
                    className={`truncate ${
                      i > 0 ? "before:mr-2 before:text-line-strong before:content-['·']" : ""
                    } ${i === 0 || (i === metaParts.length - 1 && chatMeta?.cwd) ? "font-mono" : ""}`}
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
            <LayoutPicker active={layout.preset} onPick={pickPreset} />
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
          ) : chatState === "resuming" ? (
            <ResumingButton />
          ) : status === "running" && sessionId ? (
            <StopButton onClick={() => void endChat()} />
          ) : sessionId ? (
            // Stopped/crashed → Resume, matching
            // Pencil node `HLXK6` in `vS5ce`. Same action the inline
            // SessionEndedOverlay card fires; mirroring it in the
            // topbar lets the user recover without scrolling to the
            // bottom of the feed. If `agent_session_key` is missing,
            // the backend resume path starts a fresh agent process
            // for the same row; the overlay subtitle explains that
            // history is unavailable.
            <ResumeButton onClick={() => void resumeChat()} />
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
              onPin={() => {
                setKebabOpen(false);
                void togglePin();
              }}
              onRename={() => {
                setKebabOpen(false);
                void renameChatPrompt();
              }}
              onArchive={() => {
                setKebabOpen(false);
                void archiveChat();
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
              {/* Dashed-bar variant = "panel exists but is collapsed";
                  flips to the solid `PanelRight` when the panel is
                  open. Two clearly distinct states, both Obsidian-
                  flavored (no chevrons). */}
              <PanelRightDashed aria-hidden className="h-4 w-4" />
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

      {/* Keep one xterm mounted per direct session. Hidden panes still receive
          PTY output into their buffers, so switching sessions preserves the
          real terminal state. When the active pane's session has exited the
          xterm dims and a "Session ended" card overlays the center —
          mirrors Pencil node `vS5ce`.
          Archived rows render a static placeholder instead — no xterm,
          no PTY listener, no live data path. */}
      <div
        ref={(el) => paneGeo.containerRef(el)}
        className="relative flex-1 overflow-hidden p-4"
      >
        {splitActive && !isArchived ? (
          // Split view (impl 0020): the pane tree is pure chrome (headers,
          // focus ring, gutters, empty states); the terminals live in the
          // flat absolute stack below — its own stable slot, shared with
          // the single-pane path so preset changes never move a terminal's
          // tree position — and are geometry-synced onto their pane bodies.
          <div className="h-full w-full">{renderPaneNode(layout.root)}</div>
        ) : !metaLoaded && sessionId ? (
          // Neutral loading state until chatMeta resolves. Rendering
          // the terminal map here would briefly mount RunnerTerminal
          // for archived rows (chatMeta null → isArchived false) and
          // fire a session/output subscribe + outputSnapshot call
          // before the read-only branch takes over. The flash is short
          // but visible and contradicts the 'no PTY listener' goal.
          // Centered cyan pill mirrors the resume transitional state
          // so any "session is coming up" moment reads consistently
          // — gated on `showNavLoadingPill` so the common fast-IPC
          // path doesn't flash on every chat-to-chat navigation.
          showNavLoadingPill ? <StartingOverlay label="Starting chat…" /> : null
        ) : isArchived ? (
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
        ) : isSecondary ? (
          // Secondary window (impl 0018): the primary owns the PTY, so we
          // mount no terminal here. The duplicate-subject overlay below
          // covers this area with the "Focus that window" affordance.
          null
        ) : directSessions.length === 0 ? (
          // Same delayed-pill treatment as the metaLoaded gate above —
          // the terminal map hasn't upserted the active session pane
          // yet but the next render almost always brings it in.
          showNavLoadingPill ? <StartingOverlay label="Starting chat…" /> : null
        ) : null}
        {/* The flat terminal stack — one stable slot for both modes, so
            flipping between single-pane and split never moves a terminal's
            tree position (= never remounts xterm). */}
        {renderStack ? directSessions.map((s) => renderTerminalPane(s)) : null}
        {splitActive && !isArchived ? (
          // Split view renders its overlays per pane (duplicate-subject in
          // the pane chrome, transitional/session-ended inside the focused
          // session's wrapper) — nothing page-wide to add here.
          null
        ) : showDuplicateOverlay ? (
          // Duplicate-subject overlay (impl 0018) wins over the
          // transitional overlays: this window doesn't own the PTY, so
          // "Resuming…" / "Starting…" / "Session ended" would be
          // misleading here.
          <DuplicateSubjectOverlay
            kind="chat"
            primaryLabel={primaryLabel}
            onStayHere={() =>
              sessionId
                ? setDismissedSecondary((prev) => new Set(prev).add(sessionId))
                : undefined
            }
          />
        ) : isSecondary ? (
          // Overlay dismissed but still secondary: never surface the
          // SessionEnded card's Resume/Archive actions — they call
          // session_resume / session_kill on a PTY the primary owns.
          null
        ) : archiving ? (
          // Archiving wins over the resume + start + ended overlays
          // — the session is on its way out, so reading "Resuming…"
          // / "Starting…" / "Session ended" mid-flight would be
          // misleading.
          <ArchivingOverlay withScrim />
        ) : chatState === "resuming" ? (
          <ResumingOverlay />
        ) : starting && activeSession ? (
          // Min-1s overlay while the freshly-attached agent CLI
          // boots. The terminal underneath is hidden via
          // `opacity-0` above so the pill reads on a clean canvas.
          <StartingOverlay label="Starting chat…" />
        ) : activeSession && status !== "running" ? (
          <SessionEndedOverlay
            status={status}
            exitCode={exitCode}
            // chatMeta.resumable is `true` iff agent_session_key is
            // non-NULL on the row. False for shell runtimes and for
            // codex chats whose post-spawn capture hasn't completed
            // — in either case "Resume" actually starts a fresh
            // agent process, so the overlay copy shouldn't promise a
            // preserved conversation. We default to true while
            // chatMeta is still loading so we don't briefly mislabel
            // a resumable session.
            resumable={chatMeta?.resumable ?? true}
            onResume={() => void resumeChat()}
            onArchive={() => void archiveChat()}
            variant="inline"
          />
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
        onClose={() => setPaneModalTarget(null)}
        defaultRunnerId={chatMeta?.runner_id ?? undefined}
        onStarted={(spawned) => {
          const target = paneModalTarget;
          setPaneModalTarget(null);
          if (target) {
            assignSessionToPane(target, spawned.id);
            focusPane(target);
          }
          navigate(`/chats/${spawned.id}`, {
            state: { sessionStatus: "running" },
          });
        }}
      />
    </div>
  );
}

/// Empty split pane (Pencil `t0YBp`): New chat funnels into the
/// StartChatModal with the focused chat's runner preselected; the sidebar
/// is the other fill path.
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

/// Registry of live RunnerTerminal handles, keyed by session id. A closure
/// over plain Maps instead of a ref so `refFor` can be called during render
/// (react-hooks/refs forbids `ref.current` there); the returned callbacks
/// are stable per session so React doesn't detach/reattach them per commit.
function createTerminalRegistry() {
  const handles = new Map<string, RunnerTerminalHandle>();
  const cbs = new Map<string, (h: RunnerTerminalHandle | null) => void>();
  return {
    refFor(sessionId: string) {
      let cb = cbs.get(sessionId);
      if (!cb) {
        cb = (h) => {
          if (h) handles.set(sessionId, h);
          else handles.delete(sessionId);
        };
        cbs.set(sessionId, cb);
      }
      return cb;
    },
    get(sessionId: string): RunnerTerminalHandle | null {
      return handles.get(sessionId) ?? null;
    },
  };
}

/// Split-view geometry sync (impl 0020, decision 4 — geometry-sync
/// variant). Terminals stay in the flat absolutely-positioned stack they
/// have always lived in — their React tree position never changes, so
/// xterm never remounts. Each visible session's wrapper is imperatively
/// sized/positioned onto its pane's body rect; a shared ResizeObserver
/// keeps them glued through gutter drags, window resizes, and panel
/// toggles. (Portal-per-pane was rejected: React remounts portal children
/// when the container node changes — `updatePortal` compares
/// `containerInfo` by identity — which is exactly the remount this
/// feature must avoid.) Same closure-over-Maps shape as the terminal
/// registry, for the same render-time-refs reason.
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
          if (el) termWraps.set(sessionId, el);
          else termWraps.delete(sessionId);
        };
        termWrapCbs.set(sessionId, cb);
      }
      return cb;
    },
    clearWrapGeometry() {
      for (const wrap of termWraps.values()) {
        wrap.style.left = "";
        wrap.style.top = "";
        wrap.style.width = "";
        wrap.style.height = "";
      }
    },
    dispose() {
      ro?.disconnect();
    },
  };
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
  return (
    <aside
      ref={asideRef}
      aria-hidden={!open}
      style={{ width: open ? width : 0 }}
      className={`relative flex shrink-0 flex-col overflow-hidden bg-panel transition-[width,border-left-width] duration-200 ease-in-out ${
        open ? "border-l border-line" : "border-l-0"
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
              <PanelRight aria-hidden className="h-4 w-4" />
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
  onPin,
  onRename,
  onArchive,
}: {
  pinned: boolean;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onArchive: () => void;
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
          <KebabItem
            icon={pinned ? PinOff : Pin}
            label={pinned ? "Unpin" : "Pin"}
            onClick={onPin}
          />
          <KebabItem icon={SquarePen} label="Rename" onClick={onRename} />
          <KebabItem icon={Archive} label="Archive" onClick={onArchive} danger />
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
