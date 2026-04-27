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
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";

import { RunnerTerminal } from "../components/RunnerTerminal";
import { api } from "../lib/api";
import {
  clearActiveSession,
  setActiveSession,
} from "../lib/activeSessions";
import type { SessionStatus } from "../lib/types";

interface ExitEvent {
  session_id: string;
  mission_id: string | null;
  exit_code: number | null;
  success: boolean;
}

// Two ways to land on the chat pane:
//   - "spawn" mode: come from the runner detail's `Chat now` button.
//     Carry `runnerId` (+ optional cwd) and let RunnerChat call
//     session_start_direct on mount.
//   - "attach" mode: come from the sidebar's SESSION list, which
//     already knows about a live session for this runner. Carry
//     `sessionId` and skip the spawn — re-subscribe to the existing
//     session's output stream instead.
interface RunnerChatLocationState {
  runnerId?: string;
  cwd?: string | null;
  sessionId?: string;
}

interface DirectSessionPane {
  id: string;
  handle: string;
  status: SessionStatus;
  exitCode: number | null;
}

export default function RunnerChat() {
  const { handle } = useParams<{ handle: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const state = location.state as RunnerChatLocationState | null;

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [directSessions, setDirectSessions] = useState<DirectSessionPane[]>([]);
  const [err, setErr] = useState<string | null>(null);

  // Set by `End chat` so the exit handler can distinguish a user-
  // initiated kill (we want it to read as "stopped") from an actual
  // crash. Without this, every End chat lands on status="crashed"
  // because SIGKILL bubbles up as a non-zero exit.
  const killedSessionsRef = useRef<Set<string>>(new Set());
  // Last route/session request this component attached or spawned for.
  // React Router reuses RunnerChat when moving between
  // `/runners/:handle/chat` routes, so this must be keyed by handle and
  // session state instead of a one-shot boolean.
  const startedKeyRef = useRef<string | null>(null);

  const activeSession = directSessions.find((s) => s.id === sessionId) ?? null;
  const status = activeSession?.status ?? "running";
  const exitCode = activeSession?.exitCode ?? null;

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
    (id: string, sessionHandle: string) => {
      setSessionId(id);
      setErr(null);
      upsertSession({
        id,
        handle: sessionHandle,
        status: "running",
        exitCode: null,
      });
      setActiveSession(sessionHandle, id);
    },
    [upsertSession],
  );

  const onTerminalExit = useCallback((sessionHandle: string, ev: ExitEvent) => {
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
    clearActiveSession(sessionHandle);
  }, []);

  // Attach or spawn for the current route request. Each session gets its own
  // mounted RunnerTerminal pane, so switching between direct chats preserves
  // xterm's real in-memory screen and scrollback instead of trying to replay
  // raw PTY bytes into a shared terminal.
  useEffect(() => {
    let cancelled = false;

    void (async () => {
      const requestKey = [
        handle ?? "",
        state?.sessionId ?? "",
        state?.runnerId ?? "",
        state?.cwd ?? "",
      ].join("\u0000");
      if (startedKeyRef.current === requestKey) return;
      startedKeyRef.current = requestKey;
      setSessionId(null);
      setErr(null);

      // Attach mode — caller already knows the session id (sidebar
      // re-entry).
      if (state?.sessionId && handle) {
        attach(state.sessionId, handle);
        return;
      }

      // Spawn mode — first entry from the runner detail page.
      if (state?.runnerId && handle) {
        const runnerId = state.runnerId;
        try {
          const spawned = await api.session.startDirect(
            runnerId,
            state.cwd ?? null,
            null,
            null,
          );
          if (cancelled) return;
          attach(spawned.id, handle);
        } catch (e) {
          setErr(String(e));
        }
        return;
      }

      // No location.state — typical after a window reload while on the
      // chat route. Look up the runner's live direct-chat session id
      // from the backend (the same field the sidebar consumes from
      // `runner/activity`) and re-attach.
      if (!handle) {
        setErr(
          "Direct chat must be opened from the runner detail page or the sidebar.",
        );
        return;
      }
      try {
        const runner = await api.runner.getByHandle(handle);
        if (cancelled) return;
        const activity = await api.runner.activity(runner.id);
        if (cancelled) return;
        if (activity.direct_session_id) {
          attach(activity.direct_session_id, handle);
        } else {
          setErr(
            "No live direct-chat session for this runner. Start one from the runner detail page.",
          );
        }
      } catch (e) {
        setErr(String(e));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [attach, handle, state?.runnerId, state?.cwd, state?.sessionId]);

  async function endChat() {
    if (!sessionId) return;
    killedSessionsRef.current.add(sessionId);
    try {
      await api.session.kill(sessionId);
    } catch (e) {
      setErr(String(e));
    }
  }

  const statusColor =
    status === "running"
      ? "text-accent"
      : status === "crashed"
        ? "text-danger"
        : "text-fg-2";

  return (
    <div className="flex h-full flex-1 flex-col bg-bg">
      <header className="flex items-center justify-between gap-4 border-b border-line bg-panel px-8 pb-4 pt-9">
        <div className="flex items-baseline gap-2 text-sm text-fg-2">
          <Link to="/runners" className="hover:text-fg">
            Runners
          </Link>
          <span className="text-line-strong">›</span>
          <Link to={`/runners/${handle}`} className="hover:text-fg">
            @{handle}
          </Link>
          <span className="text-line-strong">›</span>
          <span className="text-fg">direct chat</span>
          <span className="ml-2 text-[11px]">
            {sessionId ? (
              <>
                <span className="text-fg-3">session {sessionId.slice(-6)} · </span>
                <span className={statusColor}>{status}</span>
              </>
            ) : (
              <span className="text-fg-3">starting…</span>
            )}
            {exitCode != null ? (
              <span className="text-fg-3"> · exit {exitCode}</span>
            ) : null}
          </span>
        </div>
        <div className="flex gap-2">
          {status === "running" && sessionId ? (
            <button
              onClick={() => void endChat()}
              className="cursor-pointer rounded border border-line-strong bg-raised px-3 py-1.5 text-xs font-semibold text-fg hover:border-fg-3"
            >
              End chat
            </button>
          ) : (
            <button
              onClick={() => navigate(`/runners/${handle}`)}
              className="cursor-pointer rounded border border-line-strong bg-raised px-3 py-1.5 text-xs font-semibold text-fg hover:border-fg-3"
            >
              Back to runner
            </button>
          )}
        </div>
      </header>

      {err ? (
        <div className="mx-8 mt-4 rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
          {err}
        </div>
      ) : null}

      {/* Keep one xterm mounted per direct session. Hidden panes still receive
          PTY output into their buffers, so switching sessions preserves the
          real terminal state. */}
      <div className="relative flex-1 overflow-hidden p-4">
        {directSessions.length === 0 ? (
          <div className="text-sm text-fg-3">Starting…</div>
        ) : (
          directSessions.map((s) => {
            const active = s.id === sessionId;
            return (
              <div
                key={s.id}
                className={`absolute inset-4 ${active ? "block" : "hidden"}`}
              >
                <RunnerTerminal
                  sessionId={s.id}
                  active={active}
                  onExit={(ev) => onTerminalExit(s.handle, ev)}
                  onError={setErr}
                />
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
