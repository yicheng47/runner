// Embedded xterm.js bound to a single live session.
//
// The mission workspace mounts one of these per session in the roster and
// keeps them all alive simultaneously (stacked via absolute positioning).
// Output keeps streaming into hidden instances so switching tabs preserves
// each PTY's scrollback — without that, a `echo hi` typed in @lead while
// @impl was the active tab would be lost from xterm's buffer the moment
// the user switched back.
//
// This component is the second xterm consumer in the app — RunnerChat is
// the first. Setup mirrors RunnerChat's: WebGL renderer for cell-row
// alignment, base64 PTY frames to preserve raw bytes, SIGWINCH dance on
// attach so claude-code repaints onto a fresh grid.

import { useEffect, useRef } from "react";

import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { api } from "../lib/api";

interface OutputEvent {
  session_id: string;
  mission_id: string | null;
  data: string;
}

interface ExitEvent {
  session_id: string;
  mission_id: string | null;
  exit_code: number | null;
  success: boolean;
}

interface RunnerTerminalProps {
  sessionId: string;
  /** Notified when the bound session emits an exit event. */
  onExit?: (ev: ExitEvent) => void;
  /** Surface terminal-side errors (stdin push failures, resize errors). */
  onError?: (msg: string) => void;
  /** True while this terminal's tab is the foremost one in the workspace.
   *  Every terminal stays mounted (z-stacked) so each PTY's xterm
   *  scrollback survives tab-switching, but only the active one needs to
   *  refresh + claim focus when the user comes back to it. */
  active?: boolean;
}

const TERMINAL_THEME = {
  background: "#0E0E10",
  foreground: "#EDEDF0",
  cursor: "#00FF9C",
  cursorAccent: "#0E0E10",
  selectionBackground: "#1F2127",
  black: "#0E0E10",
  red: "#FF4D6D",
  green: "#00FF9C",
  yellow: "#FFB020",
  blue: "#39E5FF",
  magenta: "#C792EA",
  cyan: "#39E5FF",
  white: "#EDEDF0",
  brightBlack: "#5A5C66",
  brightRed: "#FF7B8E",
  brightGreen: "#5FFFB8",
  brightYellow: "#FFCB6B",
  brightBlue: "#82AAFF",
  brightMagenta: "#C792EA",
  brightCyan: "#89DDFF",
  brightWhite: "#FFFFFF",
};

function decodeBase64Chunk(data: string): Uint8Array {
  const raw = atob(data);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i += 1) {
    bytes[i] = raw.charCodeAt(i);
  }
  return bytes;
}

export function RunnerTerminal({
  sessionId,
  onExit,
  onError,
  active,
}: RunnerTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionIdRef = useRef<string>(sessionId);

  // Keep the latest sessionId visible to the data/resize callbacks without
  // re-creating the terminal on prop change. The session listener below
  // re-binds when sessionId changes — and so does the SIGWINCH attach
  // dance that wakes claude-code into repainting.
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    if (!containerRef.current) return;
    const term = new Terminal({
      cols: 80,
      rows: 24,
      theme: TERMINAL_THEME,
      fontFamily:
        'Menlo, "SF Mono", Monaco, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
    } catch {
      // No WebGL — fall through to canvas. RunnerChat does the same.
    }
    fit.fit();
    // Don't auto-focus on mount: in the workspace, multiple
    // RunnerTerminals mount at once before any tab is selected, and the
    // last-mounted one would steal focus and shove the page into its
    // own scroll position. The activation effect below grabs focus when
    // the tab becomes active.

    const onDataDisposable = term.onData((data) => {
      const sid = sessionIdRef.current;
      if (!sid) return;
      void api.session.injectStdin(sid, data).catch((e) => {
        onError?.(String(e));
      });
    });

    const pushSize = () => {
      const t = termRef.current;
      const sid = sessionIdRef.current;
      if (!t || !sid) return;
      void api.session.resize(sid, t.cols, t.rows).catch(() => {
        // session may have exited
      });
    };
    const onResize = () => {
      try {
        fit.fit();
        pushSize();
      } catch {
        // teardown
      }
    };
    window.addEventListener("resize", onResize);

    const refreshTerm = () => {
      const t = termRef.current;
      if (!t) return;
      try {
        t.refresh(0, t.rows - 1);
      } catch {
        // teardown
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") refreshTerm();
    };
    window.addEventListener("focus", refreshTerm);
    document.addEventListener("visibilitychange", onVisibility);

    termRef.current = term;
    fitRef.current = fit;

    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("focus", refreshTerm);
      document.removeEventListener("visibilitychange", onVisibility);
      onDataDisposable.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [onError]);

  // Subscribe to the bound session's output + exit. Re-runs on sessionId
  // change, which means the workspace can recycle a RunnerTerminal across
  // sessions if it ever needs to (currently it mounts one per session and
  // keeps it for the mission's lifetime).
  useEffect(() => {
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      const [fnOut, fnExit] = await Promise.all([
        listen<OutputEvent>("session/output", (event) => {
          if (event.payload.session_id !== sessionId) return;
          termRef.current?.write(decodeBase64Chunk(event.payload.data));
        }),
        listen<ExitEvent>("session/exit", (event) => {
          if (event.payload.session_id !== sessionId) return;
          onExit?.(event.payload);
        }),
      ]);
      if (cancelled) {
        fnOut();
        fnExit();
        return;
      }
      unlistenOutput = fnOut;
      unlistenExit = fnExit;

      // SIGWINCH dance: nudge cols by -1 and back so the agent (claude-code,
      // codex, etc.) emits a fresh redraw onto our blank grid. Without
      // this, the pane sits empty until the user types.
      const t = termRef.current;
      if (t) {
        const cols = t.cols;
        const rows = t.rows;
        try {
          await api.session.resize(sessionId, Math.max(2, cols - 1), rows);
          await api.session.resize(sessionId, cols, rows);
        } catch {
          // session may have exited
        }
      }
    })();

    return () => {
      cancelled = true;
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [sessionId, onExit]);

  // Activation effect: when this tab moves to the front, fit to the
  // (possibly newly-laid-out) container, repaint the WebGL canvas with
  // the current scrollback, and grab focus so keystrokes flow into the
  // expected PTY. Stacked-but-occluded panes can have stale layout
  // dimensions, so we always re-fit before refreshing.
  useEffect(() => {
    if (!active) return;
    const t = termRef.current;
    const fit = fitRef.current;
    if (!t || !fit) return;
    try {
      fit.fit();
      t.refresh(0, t.rows - 1);
      t.focus();
    } catch {
      // Layout not ready yet — the next focus / resize will drive it.
    }
    // Also push the (possibly changed) grid down to the PTY so the
    // agent renders at full width.
    void api.session.resize(sessionId, t.cols, t.rows).catch(() => {
      // session may have exited
    });
  }, [active, sessionId]);

  return (
    <div className="h-full w-full overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
}
