// Direct-chat pane (C8.5) — `/runners/:handle/chat/:sessionId`.
//
// One-on-one PTY between the human and the runner's CLI. No mission, no
// orchestrator, no event bus. The session was already spawned by the
// Runner Detail page's "Chat now" button via `session_start_direct`; this
// page just subscribes to the existing session/output + session/exit
// streams and lets the user inject stdin.
//
// Modeled on the C6 debug pane minus the mission concepts. xterm.js is
// out of scope for v0 — a `<pre>` with capped scrollback is fine for the
// "I want to ask @architect a quick question" use case.

import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

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

export default function RunnerChat() {
  const { handle, sessionId: sessionIdParam } = useParams<{
    handle: string;
    sessionId: string;
  }>();
  const sessionId = sessionIdParam ?? "";
  const navigate = useNavigate();

  const [output, setOutput] = useState("");
  const [status, setStatus] = useState<SessionStatus>("running");
  const [exitCode, setExitCode] = useState<number | null>(null);
  const [input, setInput] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const sessionIdRef = useRef(sessionId);

  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  // Subscribe to PTY output + exit. Filter on session_id (not mission_id —
  // direct chats have mission_id == null).
  useEffect(() => {
    let unlistenOut: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;

    void listen<OutputEvent>("session/output", (event) => {
      if (event.payload.session_id !== sessionIdRef.current) return;
      setOutput((prev) => (prev + event.payload.text).slice(-32_000));
    }).then((fn) => {
      unlistenOut = fn;
    });

    void listen<ExitEvent>("session/exit", (event) => {
      if (event.payload.session_id !== sessionIdRef.current) return;
      setStatus(event.payload.success ? "stopped" : "crashed");
      setExitCode(event.payload.exit_code);
    }).then((fn) => {
      unlistenExit = fn;
    });

    return () => {
      unlistenOut?.();
      unlistenExit?.();
    };
  }, []);

  async function inject() {
    if (!input || status !== "running") return;
    try {
      await api.session.injectStdin(sessionId, input + "\n");
      setInput("");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function endChat() {
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
              session {sessionId.slice(-6)} · {status}
              {exitCode != null ? ` · exit ${exitCode}` : ""}
            </span>
          </div>
          <div className="flex gap-2">
            {status === "running" ? (
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
            {output || "(no output yet)"}
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
              disabled={status !== "running"}
              className="flex-1 rounded border border-neutral-300 bg-white p-2 text-xs disabled:bg-neutral-50"
            />
            <button
              onClick={() => void inject()}
              disabled={status !== "running" || !input}
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
