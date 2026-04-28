// Embedded xterm.js bound to a single live session.
//
// Direct chat and the mission workspace mount one of these per session and
// keep them alive while switching between tabs/routes. Output keeps streaming
// into hidden instances so each PTY's scrollback survives UI switches.
//
// Setup: WebGL renderer for cell-row alignment, base64 PTY frames to preserve
// raw bytes, backend snapshot replay for late attach, and SIGWINCH dance on
// attach so claude-code/codex repaint onto a fresh grid.

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
  seq: number;
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
  const onExitRef = useRef(onExit);
  const onErrorRef = useRef(onError);
  const activeRef = useRef(active ?? false);

  // Keep the latest sessionId visible to the data/resize callbacks without
  // re-creating the terminal on prop change. The session listener below
  // re-binds when sessionId changes — and so does the SIGWINCH attach
  // dance that wakes claude-code into repainting.
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    onExitRef.current = onExit;
  }, [onExit]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  useEffect(() => {
    activeRef.current = active ?? false;
  }, [active]);

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
    const initialRect = containerRef.current.getBoundingClientRect();
    if (initialRect.width > 0 && initialRect.height > 0) {
      fit.fit();
    }
    // Don't auto-focus on mount: in the workspace, multiple
    // RunnerTerminals mount at once before any tab is selected, and the
    // last-mounted one would steal focus and shove the page into its
    // own scroll position. The activation effect below grabs focus when
    // the tab becomes active.

    const onDataDisposable = term.onData((data) => {
      const sid = sessionIdRef.current;
      if (!sid) return;
      void api.session.injectStdin(sid, data).catch((e) => {
        onErrorRef.current?.(String(e));
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
      if (!activeRef.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
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
  }, []);

  // Subscribe to the bound session's output + exit. The listener is registered
  // before snapshot replay so live chunks that arrive during the command round
  // trip are buffered and merged by seq.
  useEffect(() => {
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let cancelled = false;
    let replayDone = false;
    let lastWrittenSeq = 0;
    const pendingLive: OutputEvent[] = [];

    const writeOutput = (ev: OutputEvent) => {
      termRef.current?.write(decodeBase64Chunk(ev.data));
    };

    void (async () => {
      const [fnOut, fnExit] = await Promise.all([
        listen<OutputEvent>("session/output", (event) => {
          if (event.payload.session_id !== sessionId) return;
          if (!replayDone) {
            pendingLive.push(event.payload);
            return;
          }
          if (event.payload.seq <= lastWrittenSeq) return;
          writeOutput(event.payload);
          lastWrittenSeq = event.payload.seq;
        }),
        listen<ExitEvent>("session/exit", (event) => {
          if (event.payload.session_id !== sessionId) return;
          onExitRef.current?.(event.payload);
        }),
      ]);
      if (cancelled) {
        fnOut();
        fnExit();
        return;
      }
      unlistenOutput = fnOut;
      unlistenExit = fnExit;

      let snapshot: OutputEvent[] = [];
      try {
        snapshot = await api.session.outputSnapshot(sessionId);
      } catch (e) {
        onErrorRef.current?.(String(e));
      }
      if (cancelled) return;

      termRef.current?.reset();
      for (const ev of snapshot) {
        writeOutput(ev);
        lastWrittenSeq = Math.max(lastWrittenSeq, ev.seq);
      }
      replayDone = true;
      for (const ev of pendingLive) {
        if (ev.seq <= lastWrittenSeq) continue;
        writeOutput(ev);
        lastWrittenSeq = ev.seq;
      }
      pendingLive.length = 0;

      // Do not resize here: hidden terminal panes mount before they are
      // measurable, and sending that hidden geometry to TUIs makes them paint
      // their startup screen into a tiny grid. The activation effect below
      // owns the SIGWINCH dance once the pane is visible.
    })();

    return () => {
      cancelled = true;
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [sessionId]);

  // Activation effect: when this tab moves to the front, wait for the pane
  // to become measurable, fit to its container, repaint the WebGL/canvas
  // renderer with the current scrollback, and grab focus so keystrokes flow
  // into the expected PTY.
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let raf1 = 0;
    let raf2 = 0;

    const activate = () => {
      if (cancelled) return;
      const t = termRef.current;
      const fit = fitRef.current;
      const node = containerRef.current;
      if (!t || !fit || !node) return;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      try {
        fit.fit();
        t.refresh(0, t.rows - 1);
        t.focus();
        const cols = t.cols;
        const rows = t.rows;
        void api.session.resize(sessionId, Math.max(2, cols - 1), rows)
          .then(() => api.session.resize(sessionId, cols, rows))
          .catch(() => {
            // session may have exited
          });
      } catch {
        // Layout not ready yet — the next activation / resize will drive it.
      }
    };

    raf1 = window.requestAnimationFrame(() => {
      raf2 = window.requestAnimationFrame(activate);
    });

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(raf1);
      window.cancelAnimationFrame(raf2);
    };
  }, [active, sessionId]);

  return (
    <div className="h-full w-full overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
}
