// Direct-chat pane (C8.5) — `/runners/:handle/chat`.
//
// One-on-one PTY between the human and the runner's CLI. No mission, no
// orchestrator, no event bus. The chat page is responsible for spawning
// the session itself (rather than the Runner Detail page doing it before
// navigating) so the event listener can attach BEFORE the PTY's reader
// thread starts emitting. Without this ordering, fast-exit runners or
// startup failures can finish before the listener exists, leaving the
// pane stuck at "running" with no output.
//
// Modeled on the C6 debug pane minus the mission concepts. xterm.js is
// out of scope for v0 — a `<pre>` with capped scrollback is fine for the
// "I want to ask @architect a quick question" use case.

import { useEffect, useRef, useState } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";

import { api } from "../lib/api";
import type { SessionStatus } from "../lib/types";
import { AppShell } from "../components/AppShell";

interface OutputEvent {
  session_id: string;
  mission_id: string | null;
  text: string;
}

interface ExitEvent {
  session_id: string;
  mission_id: string | null;
  exit_code: number | null;
  success: boolean;
}

interface RunnerChatLocationState {
  runnerId: string;
  cwd: string | null;
}

export default function RunnerChat() {
  const { handle } = useParams<{ handle: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const state = location.state as RunnerChatLocationState | null;

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [output, setOutput] = useState("");
  const [status, setStatus] = useState<SessionStatus>("running");
  const [exitCode, setExitCode] = useState<number | null>(null);
  const [input, setInput] = useState("");
  const [err, setErr] = useState<string | null>(null);

  // Mutable mirrors so the listen() callback can branch on the latest id
  // without React re-subscribing every state change.
  const sessionIdRef = useRef<string | null>(null);
  // Pre-spawn buffer: the listener is attached before we have a session
  // id, but the PTY's reader thread can emit between `spawn_direct`
  // returning and our promise resolving. Anything that arrives in that
  // window goes here and is replayed once we know our id.
  const preSpawnBuffer = useRef<{
    outputs: OutputEvent[];
    exits: ExitEvent[];
  }>({ outputs: [], exits: [] });
  // Flag we set the moment we kick off the spawn so the unlisten cleanup
  // can avoid double-spawning under React strict-mode double-mount.
  const startedRef = useRef(false);

  useEffect(() => {
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let cancelled = false;

    const consumeOutput = (ev: OutputEvent) => {
      setOutput((prev) => (prev + ev.text).slice(-32_000));
    };
    const consumeExit = (ev: ExitEvent) => {
      setStatus(ev.success ? "stopped" : "crashed");
      setExitCode(ev.exit_code);
    };

    void Promise.all([
      listen<OutputEvent>("session/output", (event) => {
        const sid = sessionIdRef.current;
        if (sid === null) {
          // Don't know our id yet — buffer; we'll filter on drain.
          preSpawnBuffer.current.outputs.push(event.payload);
          return;
        }
        if (event.payload.session_id !== sid) return;
        consumeOutput(event.payload);
      }),
      listen<ExitEvent>("session/exit", (event) => {
        const sid = sessionIdRef.current;
        if (sid === null) {
          preSpawnBuffer.current.exits.push(event.payload);
          return;
        }
        if (event.payload.session_id !== sid) return;
        consumeExit(event.payload);
      }),
    ]).then(([fnOut, fnExit]) => {
      if (cancelled) {
        fnOut();
        fnExit();
        return;
      }
      unlistenOutput = fnOut;
      unlistenExit = fnExit;
    });

    // Kick off the spawn AFTER subscriptions are set up. The runner id
    // lives in router state — direct chats don't survive a refresh in v0
    // (we don't persist a transcript), so if state is missing we bounce
    // back to the runner detail page.
    if (!state || !state.runnerId) {
      setErr("Direct chat must be opened from the runner detail page.");
      return;
    }
    if (startedRef.current) return;
    startedRef.current = true;
    void api.session
      .startDirect(state.runnerId, state.cwd)
      .then((spawned) => {
        sessionIdRef.current = spawned.id;
        setSessionId(spawned.id);
        // Drain anything that arrived while we didn't know our id yet,
        // keeping only events that match.
        for (const ev of preSpawnBuffer.current.outputs) {
          if (ev.session_id === spawned.id) consumeOutput(ev);
        }
        for (const ev of preSpawnBuffer.current.exits) {
          if (ev.session_id === spawned.id) consumeExit(ev);
        }
        preSpawnBuffer.current = { outputs: [], exits: [] };
      })
      .catch((e: unknown) => {
        setErr(String(e));
      });

    return () => {
      cancelled = true;
      unlistenOutput?.();
      unlistenExit?.();
    };
    // We deliberately depend only on the runnerId/cwd we want to spawn
    // for. Re-running on every render would fire a fresh PTY every time
    // React re-rendered the page; we already guard via startedRef.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state?.runnerId, state?.cwd]);

  async function inject() {
    if (!input || status !== "running" || !sessionId) return;
    try {
      await api.session.injectStdin(sessionId, input + "\n");
      setInput("");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function endChat() {
    if (!sessionId) return;
    try {
      await api.session.kill(sessionId);
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <AppShell>
      <div className="flex h-full flex-1 flex-col bg-neutral-50">
        <header className="flex items-center justify-between gap-4 border-b border-[#E5E5E5] bg-white px-8 pb-4 pt-9">
          <div className="flex items-baseline gap-2 text-sm text-neutral-500">
            <Link to="/runners" className="hover:text-neutral-800">
              Runners
            </Link>
            <span className="text-neutral-300">›</span>
            <Link
              to={`/runners/${handle}`}
              className="hover:text-neutral-800"
            >
              @{handle}
            </Link>
            <span className="text-neutral-300">›</span>
            <span className="text-neutral-900">direct chat</span>
            <span className="ml-2 text-[11px] text-neutral-400">
              {sessionId
                ? `session ${sessionId.slice(-6)} · ${status}`
                : "starting…"}
              {exitCode != null ? ` · exit ${exitCode}` : ""}
            </span>
          </div>
          <div className="flex gap-2">
            {status === "running" && sessionId ? (
              <button
                onClick={() => void endChat()}
                className="rounded border border-neutral-300 bg-white px-3 py-1.5 text-xs font-semibold text-neutral-700 hover:bg-neutral-50"
              >
                End chat
              </button>
            ) : (
              <button
                onClick={() => navigate(`/runners/${handle}`)}
                className="rounded border border-neutral-300 bg-white px-3 py-1.5 text-xs font-semibold text-neutral-700 hover:bg-neutral-50"
              >
                Back to runner
              </button>
            )}
          </div>
        </header>

        {err ? (
          <div className="mx-8 mt-4 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
            {err}
          </div>
        ) : null}

        <div className="flex flex-1 flex-col gap-3 overflow-hidden p-6 font-mono text-sm">
          <pre className="flex-1 overflow-auto whitespace-pre-wrap rounded border border-neutral-300 bg-neutral-900 p-3 text-xs leading-tight text-neutral-100">
            {output || (sessionId ? "(no output yet)" : "(starting session…)")}
          </pre>
          <div className="flex gap-2">
            <input
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void inject();
              }}
              placeholder={
                status === "running"
                  ? "Type a line, ↵ to send (a newline is appended)"
                  : "Session is no longer running."
              }
              disabled={status !== "running" || !sessionId}
              className="flex-1 rounded border border-neutral-300 bg-white p-2 text-xs disabled:bg-neutral-50"
            />
            <button
              onClick={() => void inject()}
              disabled={status !== "running" || !input || !sessionId}
              className="rounded bg-neutral-900 px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-40"
            >
              Send
            </button>
          </div>
        </div>
      </div>
    </AppShell>
  );
}
