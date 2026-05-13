// Direct-chat pane (C8.5) — `/runners/:handle/chat`.
//
// One-on-one PTY between the human and the runner's CLI. No mission, no
// orchestrator, no event bus. Each direct session gets its own mounted
// RunnerTerminal pane so switching chats preserves xterm's real screen and
// scrollback while the backend output snapshot covers late attach / reload.
//
// Uses xterm.js so real TUIs (claude-code, codex) render correctly with
// ANSI colors, cursor movement, and mouse tracking. A plain `<pre>`
// can't interpret the control sequences these agents emit.

import { useCallback, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  Loader2,
  MoreHorizontal,
  PanelRightClose,
  PanelRightOpen,
  Pin,
  PinOff,
  Play,
  Square,
  SquarePen,
  Terminal,
} from "lucide-react";

import { RunnerTerminal } from "../components/RunnerTerminal";
import {
  ArchivingOverlay,
  ResumingOverlay,
  SessionEndedOverlay,
} from "../components/SessionEndedOverlay";
import { api, type DirectSessionEntry } from "../lib/api";
import {
  markArchivingSession,
  unmarkArchivingSession,
  useArchivingSession,
} from "../lib/archivingState";
import type { Runner, SessionStatus, WarningEvent } from "../lib/types";

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
// The session id rides on the URL itself (`/runners/:handle/chat/:sessionId`),
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

interface DirectSessionPane {
  id: string;
  handle: string;
  status: SessionStatus;
  exitCode: number | null;
}

export default function RunnerChat() {
  const { handle, sessionId: sessionIdParam } = useParams<{
    handle: string;
    sessionId: string;
  }>();
  const location = useLocation();
  const navigate = useNavigate();
  const state = location.state as RunnerChatLocationState | null;

  const sessionId = sessionIdParam ?? null;
  const [directSessions, setDirectSessions] = useState<DirectSessionPane[]>([]);
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
  // True while the user has clicked Resume and we're waiting for the
  // resumed PTY to come back online. Drives the cyan status pill, the
  // header "Resuming…" affordance, and the centered Resuming pill
  // overlay on the cleared terminal canvas.
  const [resuming, setResuming] = useState<boolean>(false);
  // True while either this chat's own archiveChat or the sidebar's
  // session-archive flow has marked this session id as archiving.
  // Drives the centered amber pill + scrim over the chat body so the
  // backend's session_kill grace + archive RPC don't read as a hang.
  const archiving = useArchivingSession(sessionId);
  // Bumped before each resume to tell RunnerTerminal to reset its
  // xterm canvas so the agent's repaint lands on a blank terminal
  // instead of stacking on top of the prior session's banner.
  const [clearVersion, setClearVersion] = useState<number>(0);
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
  // Router reuses RunnerChat when moving between
  // `/runners/:handle/chat/:sessionId` routes, so this must be keyed
  // by the URL params instead of a one-shot boolean.
  const startedKeyRef = useRef<string | null>(null);

  const activeSession = directSessions.find((s) => s.id === sessionId) ?? null;
  const status = activeSession?.status ?? "running";
  const exitCode = activeSession?.exitCode ?? null;
  // Archived rows can be reached by direct URL but render read-only.
  // We don't attach a PTY, mount RunnerTerminal, or expose Resume /
  // End / Archive — the row is terminal and the operator can only
  // read the meta + go back to the runner.
  const isArchived = chatMeta?.archived_at != null;

  const upsertSession = useCallback((next: DirectSessionPane) => {
    setDirectSessions((prev) => {
      const found = prev.find((s) => s.id === next.id);
      if (!found) return [...prev, next];
      return prev.map((s) =>
        s.id === next.id
          ? {
              ...s,
              handle: next.handle,
              status: next.status,
              exitCode: next.exitCode,
            }
          : s,
      );
    });
  }, []);

  const attach = useCallback(
    (id: string, sessionHandle: string, status: SessionStatus = "running") => {
      setErr(null);
      upsertSession({
        id,
        handle: sessionHandle,
        status,
        exitCode: null,
      });
    },
    [upsertSession],
  );

  const onTerminalExit = useCallback((ev: ExitEvent) => {
    const userEnded = killedSessionsRef.current.has(ev.session_id);
    const nextStatus = ev.success || userEnded ? "stopped" : "crashed";
    killedSessionsRef.current.delete(ev.session_id);
    setDirectSessions((prev) =>
      prev.map((s) =>
        s.id === ev.session_id
          ? { ...s, status: nextStatus, exitCode: ev.exit_code }
          : s,
      ),
    );
  }, []);

  // Pull the runner config so the header can show the runtime
  // (`claude-code`, `codex`, …) next to the @handle. One-shot per
  // handle change.
  useEffect(() => {
    if (!handle) {
      setRunner(null);
      return;
    }
    let cancelled = false;
    void api.runner
      .getByHandle(handle)
      .then((r) => {
        if (!cancelled) setRunner(r);
      })
      .catch(() => {
        if (!cancelled) setRunner(null);
      });
    return () => {
      cancelled = true;
    };
  }, [handle]);

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
      return;
    }
    try {
      const rows = await api.session.listRecentDirect();
      const found = rows.find((r) => r.session_id === sessionId);
      if (found) {
        setChatMeta(found);
        return;
      }
      // Missed in the visible list — could be archived. The
      // unfiltered get returns the row regardless so the read-only
      // branch below can render the right UX.
      const archived = await api.session.get(sessionId);
      setChatMeta(archived);
    } catch (e) {
      console.error("RunnerChat: refreshChatMeta failed", e);
    }
  }, [sessionId]);

  useEffect(() => {
    void refreshChatMeta();
  }, [refreshChatMeta]);

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
    setDirectSessions((prev) =>
      prev.map((s) =>
        s.id === chatMeta.session_id
          ? { ...s, status: chatMeta.status }
          : s,
      ),
    );
  }, [chatMeta]);

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

    void listen<{ session_id: string }>("session/output", (event) => {
      if (event.payload.session_id !== targetId) return;
      scheduleIdleTimer();
    }).then((fn) => {
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
    const requestKey = [handle ?? "", sessionId ?? ""].join(" ");
    if (startedKeyRef.current === requestKey) return;
    startedKeyRef.current = requestKey;
    setErr(null);
    // Skip attach for archived rows: the workspace renders read-only,
    // so mounting RunnerTerminal would spawn a PTY listener for a row
    // that's terminal by definition.
    if (sessionId && handle && !isArchived) {
      attach(sessionId, handle, state?.sessionStatus ?? "stopped");
    }
  }, [attach, handle, sessionId, state?.sessionStatus, isArchived]);

  async function endChat() {
    if (!sessionId || !handle) return;
    const targetId = sessionId;
    killedSessionsRef.current.add(targetId);
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
    if (!sessionId || !handle) return;
    const targetId = sessionId;
    setResuming(true);
    setClearVersion((v) => v + 1);
    setErr(null);
    try {
      await api.session.resume(targetId, null, null);
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
    const current = chatMeta?.title ?? (handle ? `@${handle}` : "");
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
  }, [sessionId, chatMeta, handle, refreshChatMeta]);

  // Archive: hide this chat from the sidebar's SESSION tray. The row
  // stays in the DB so a future Archived workspace surface can list
  // it, but it's gone from the live tray. We navigate back to the
  // runner detail since this chat surface no longer maps to anything
  // discoverable. Mirrors `Sidebar.archiveSession`: the backend
  // refuses to archive a running row (`commands::session::session_archive`
  // → "kill before archiving"), so kill first when the row is live.
  async function archiveChat() {
    if (!sessionId || !handle) return;
    const targetId = sessionId;
    const targetHandle = handle;
    const wasRunning = chatMeta?.status === "running";
    markArchivingSession(targetId);
    try {
      if (wasRunning) {
        // Mark the kill as user-initiated so the exit handler reads it
        // as "stopped" rather than "crashed" (matches endChat's
        // pattern). Without this the sidebar would briefly show a
        // crashed row before the archive RPC removes it.
        killedSessionsRef.current.add(targetId);
        await api.session.kill(targetId);
      }
      await api.session.archive(targetId);
      navigate(`/runners/${targetHandle}`);
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
  // kebab. Status colors come from the runner-status semantics.
  // 3-way derived state: "resuming" overrides whatever the row says
  // because it's the user-driven transitional state. Mirrors the
  // three Pencil frames (running u6woG / stopped vS5ce / resuming
  // GZhHO).
  type ChatState = "running" | "stopped" | "crashed" | "resuming";
  const chatState: ChatState = resuming
    ? "resuming"
    : status === "running"
      ? "running"
      : status === "crashed"
        ? "crashed"
        : "stopped";
  const statusBadgeClass =
    chatState === "running"
      ? "bg-accent/10 text-accent"
      : chatState === "crashed"
        ? "bg-danger/10 text-danger"
        : chatState === "resuming"
          ? "bg-[#0F1E26] text-[#39E5FF]"
          : "bg-line-strong text-fg-2";
  const statusDotClass =
    chatState === "running"
      ? "bg-accent"
      : chatState === "crashed"
        ? "bg-danger"
        : chatState === "resuming"
          ? "bg-[#39E5FF]"
          : "bg-fg-3";
  const statusLabel = chatState === "resuming" ? "resuming…" : status;
  const titleLabel =
    chatMeta?.title ?? (handle ? `@${handle}` : "chat");
  const metaParts = [
    runner ? `${runner.runtime}-${runner.handle}` : null,
    chatMeta?.started_at
      ? `started ${formatRelative(chatMeta.started_at)}`
      : null,
    chatMeta?.cwd ?? runner?.working_dir ?? null,
    exitCode != null ? `exit ${exitCode}` : null,
  ].filter((s): s is string => !!s);

  return (
    <div className="flex h-full flex-1 flex-row bg-bg">
      <div className="flex min-w-0 flex-1 flex-col">
      <header className="flex items-center justify-between gap-4 border-b border-line bg-panel px-6 pb-3.5 pt-9">
        <div className="flex min-w-0 items-center gap-3.5">
          {/* Avatar — Pencil node `dnPId`. 36×36 with `bg`, `border-line`
              stroke; the terminal glyph carries the accent fill. */}
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-line bg-bg">
            <Terminal aria-hidden className="h-[18px] w-[18px] text-accent" />
          </div>
          <div className="flex min-w-0 flex-col gap-[3px]">
            <div className="flex items-center gap-2.5">
              {/* Title — JetBrains Mono 15/600 per node `U9Fx8f`.
                  Falls back to `@handle` when the row has no custom
                  title; both render in mono since either way it's an
                  identifier-shaped string. */}
              <span className="truncate font-mono text-[15px] font-semibold text-fg">
                {titleLabel}
              </span>
              <span className="rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                Chat
              </span>
              {/* Status pill moved next to the title so it stops
                  competing with the Stop / Resume control on the
                  right — the action button already implies the
                  current state, and a pill at the same edge read
                  redundant. */}
              <span
                className={`flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${statusBadgeClass}`}
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
              <div className="flex min-w-0 items-center gap-2 text-[11px] text-fg-2">
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
          {isArchived ? (
            // Archived rows are terminal: no Resume / Stop / Archive.
            // Only surface the navigation escape hatch.
            <button
              onClick={() => navigate(`/runners/${handle}`)}
              className="cursor-pointer rounded border border-line bg-raised px-2.5 py-1.5 text-xs text-fg hover:border-fg-3"
            >
              Back to runner
            </button>
          ) : chatState === "resuming" ? (
            <button
              type="button"
              disabled
              className="flex cursor-not-allowed items-center gap-1.5 rounded border border-[#1F3D4D] bg-[#0F1E26] px-2.5 py-1.5 text-xs text-[#39E5FF]"
            >
              <Loader2
                aria-hidden
                className="h-3 w-3 animate-spin text-[#39E5FF]"
              />
              Resuming…
            </button>
          ) : chatState === "running" && sessionId ? (
            <button
              onClick={() => void endChat()}
              className="flex cursor-pointer items-center gap-1.5 rounded border border-line bg-raised px-2.5 py-1.5 text-xs text-fg hover:border-fg-3"
            >
              <Square aria-hidden className="h-3 w-3 text-danger" />
              Stop
            </button>
          ) : sessionId && (chatMeta?.resumable ?? true) ? (
            // Stopped/crashed with a resumable row → Resume, matching
            // Pencil node `HLXK6` in `vS5ce`. Same action the inline
            // SessionEndedOverlay card fires; mirroring it in the
            // topbar lets the user recover without scrolling to the
            // bottom of the feed. `chatMeta?.resumable` falls back to
            // true while the row is still loading so we don't briefly
            // misrender as "Back to runner."
            <button
              onClick={() => void resumeChat()}
              className="flex cursor-pointer items-center gap-1.5 rounded border border-[#1F4D33] bg-[#0F2418] px-2.5 py-1.5 text-xs font-medium text-accent hover:border-accent"
            >
              <Play aria-hidden className="h-3 w-3" />
              Resume
            </button>
          ) : (
            // Last-resort fallback: no session yet, or the row is
            // genuinely non-resumable (no agent_session_key on file).
            <button
              onClick={() => navigate(`/runners/${handle}`)}
              className="cursor-pointer rounded border border-line bg-raised px-2.5 py-1.5 text-xs text-fg hover:border-fg-3"
            >
              Back to runner
            </button>
          )}
          {/* Topbar overflow menu — Pin / Rename / Archive, matching
              the design's `session_ctx_menu` (`P5CLA` / `L31Zb`) and
              the mission topbar's `MissionKebab`. Hidden for archived
              rows: Pin/Rename make no sense for a terminal row, and
              Archive is a no-op. */}
          {sessionId && chatMeta && !isArchived ? (
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
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded border border-line bg-bg text-fg-2 hover:border-fg-3 hover:text-fg"
            >
              <PanelRightOpen aria-hidden className="h-4 w-4" />
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
      <div className="relative flex-1 overflow-hidden p-4">
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
        ) : directSessions.length === 0 ? (
          <div className="text-sm text-fg-3">Starting…</div>
        ) : (
          directSessions.map((s) => {
            const active = s.id === sessionId;
            const dead = s.status !== "running";
            // Pane visual state: while resuming the active pane is
            // fully blank (we already wiped it via clearVersion); the
            // centered Resuming pill below reads on a pristine
            // canvas. When stopped, the pane dims to 45% and the
            // Session ended card overlays it.
            const paneOpacity = active
              ? resuming
                ? "opacity-0"
                : dead
                  ? "opacity-45"
                  : ""
              : "";
            return (
              <div
                key={s.id}
                className={`absolute inset-4 ${active ? "block" : "hidden"} ${paneOpacity} transition-opacity`}
              >
                <RunnerTerminal
                  sessionId={s.id}
                  // While the loader is up the canvas is hidden, so
                  // we want xterm to behave as inactive (no resize
                  // pushes, no focus). When `resuming` flips off,
                  // `active && !resuming` flips true, which triggers
                  // RunnerTerminal's activation effect — fit() +
                  // refresh() + focus + winsize push — and clears the
                  // half-painted canvas frame the user otherwise sees.
                  active={active && !resuming}
                  disabled={dead || resuming}
                  clearVersion={active ? clearVersion : undefined}
                  onExit={onTerminalExit}
                  onError={setErr}
                />
              </div>
            );
          })
        )}
        {archiving ? (
          // Archiving wins over the resume + ended overlays — the
          // session is on its way out, so reading "Resuming…" or
          // "Session ended" mid-flight would be misleading.
          <ArchivingOverlay withScrim />
        ) : chatState === "resuming" ? (
          <ResumingOverlay />
        ) : activeSession && chatState !== "running" ? (
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
        open={panelOpen}
        onClose={() => setPanelOpen(false)}
      />
    </div>
  );
}

// SessionEndedOverlay + ResumingOverlay live in
// `../components/SessionEndedOverlay` and are shared with
// MissionWorkspace's slot PTY tabs.

// Right-side panel matching Pencil node `IFS3p` inside `GZhHO`. Spans
// the full height of the chat surface (the topbar lives in the chat
// column to its left, not above it). The panel-header strip mirrors
// the topbar's pt-9 + h-9 + pb-3.5 structure so the bottom divider
// lines up across both columns. The whole aside slides via a
// CSS width transition: open = w-80, closed = w-0 (with the inner
// w-80 wrapper kept intact and clipped by overflow-hidden so layout
// doesn't reflow during the animation).
function RunnerSidePanel({
  runner,
  open,
  onClose,
}: {
  runner: Runner | null;
  open: boolean;
  onClose: () => void;
}) {
  return (
    <aside
      aria-hidden={!open}
      className={`flex shrink-0 flex-col overflow-hidden bg-panel transition-[width,border-left-width] duration-200 ease-in-out ${
        open ? "w-80 border-l border-line" : "w-0 border-l-0"
      }`}
    >
      <div className="flex h-full w-80 flex-col">
        <header className="flex shrink-0 items-center justify-end border-b border-line px-5 pb-3.5 pt-9">
          <div className="flex h-9 items-center">
            <button
              type="button"
              onClick={onClose}
              title="Collapse panel"
              aria-label="Collapse panel"
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-2 hover:bg-raised hover:text-fg"
            >
              <PanelRightClose aria-hidden className="h-4 w-4" />
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
          ) : (
            <p className="text-xs text-fg-3">Loading runner…</p>
          )}
        </div>
      </div>
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
