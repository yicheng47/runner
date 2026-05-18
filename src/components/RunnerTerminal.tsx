// Embedded xterm.js bound to a single live session.
//
// Direct chat and the mission workspace mount one of these per session and
// keep them alive while switching between tabs/routes. Output keeps streaming
// into hidden instances so each PTY's scrollback survives UI switches.
//
// Setup: WebGL renderer for cell-row alignment, base64 PTY frames to preserve
// raw bytes, backend snapshot replay for late attach, and SIGWINCH dance on
// attach so claude-code/codex repaint onto a fresh grid.

import { useCallback, useEffect, useRef } from "react";

import { listen } from "@tauri-apps/api/event";
import { debug as logDebug } from "@tauri-apps/plugin-log";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { api } from "../lib/api";
import type { SessionOutputSnapshot } from "../lib/types";
import {
  readTerminalCursorStyle,
  readTerminalFontFamily,
  readTerminalFontSize,
  readTerminalScrollback,
  readTerminalTheme,
  resolveTerminalFontStack,
  resolveTerminalTheme,
  STORAGE_TERMINAL_CURSOR_STYLE,
  STORAGE_TERMINAL_FONT_FAMILY,
  STORAGE_TERMINAL_FONT_SIZE,
  STORAGE_TERMINAL_SCROLLBACK,
  STORAGE_TERMINAL_THEME,
} from "../lib/settings";

// Debug-only trace helpers. Lines flow through @tauri-apps/plugin-log →
// tauri-plugin-log → runner.log and are dropped in release builds by
// the plugin-log level filter (src-tauri/src/lib.rs: default_level).
// Tagged `term:` for grep convenience.
function altMode(term: Terminal | null): string {
  if (!term) return "<no-term>";
  try {
    return term.buffer.active.type;
  } catch {
    return "<unknown>";
  }
}
function traceTerm(msg: string, fields: Record<string, unknown> = {}): void {
  const parts = Object.entries(fields)
    .map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
    .join(" ");
  void logDebug(`term: ${msg}${parts ? " " + parts : ""}`).catch(() => {
    // Best-effort. plugin-log invoke can reject; never propagate.
  });
}

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
  /** Stop forwarding keystrokes / resize events to the backend.
   *  Set by the parent when the bound session has exited so stray
   *  input on the dimmed pane doesn't surface a "session not found"
   *  error from the now-empty live map. The xterm buffer stays
   *  visible (and scrollable) — only the input/resize pipes shut
   *  off. */
  disabled?: boolean;
  /** When this number increments, the xterm buffer is reset to a
   *  blank canvas. Used by the parent before driving a resume so the
   *  agent's repaint lands on an empty terminal instead of stacking
   *  on top of the prior session's banner + scrollback. */
  clearVersion?: number;
}

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
  disabled,
  clearVersion,
}: RunnerTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionIdRef = useRef<string>(sessionId);
  const onExitRef = useRef(onExit);
  const onErrorRef = useRef(onError);
  const activeRef = useRef(active ?? false);
  // Mirrors the `disabled` prop into a ref so the onData/resize
  // closures don't capture a stale value across the long-lived
  // terminal effect.
  const disabledRef = useRef<boolean>(disabled ?? false);
  // Last (cols, rows) pushed to the backend. Shared between `pushSize`
  // (mount-effect scope) and the activation effect's trailing resize so
  // neither hammers the backend with identical dims. During a drag both
  // the window 'resize' listener AND the container `ResizeObserver` fire,
  // so without this we were sending 2–3 identical `session_resize` IPCs
  // per cols value — tmux dedupes the SIGWINCH but the round-trips still
  // lengthen the redraw window the user perceives.
  const lastPushedColsRef = useRef(0);
  const lastPushedRowsRef = useRef(0);
  const replayDoneRef = useRef(false);
  const replayInFlightRef = useRef(false);
  const pendingLiveRef = useRef<OutputEvent[]>([]);
  const lastWrittenSeqRef = useRef(0);
  const outputListenerReadyRef = useRef<Promise<void>>(Promise.resolve());

  const writeOutputEvent = useCallback((ev: OutputEvent) => {
    termRef.current?.write(decodeBase64Chunk(ev.data));
  }, []);

  // Keep the latest sessionId visible to the data/resize callbacks without
  // re-creating the terminal on prop change. The session listener below
  // re-binds when sessionId changes — and so does the SIGWINCH attach
  // dance that wakes claude-code into repainting.
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    disabledRef.current = disabled ?? false;
  }, [disabled]);

  // Parent-driven canvas wipe (used by the resume flow). The first
  // render's value is the initial — we don't want to reset on mount,
  // only on subsequent bumps. We achieve that by skipping the very
  // first effect run via a ref.
  const lastClearVersionRef = useRef<number | undefined>(clearVersion);
  useEffect(() => {
    if (lastClearVersionRef.current === clearVersion) return;
    lastClearVersionRef.current = clearVersion;
    traceTerm("clearVersion fired", {
      sessionId: sessionIdRef.current,
      clearVersion,
      altBefore: altMode(termRef.current),
    });
    termRef.current?.reset();
  }, [clearVersion]);

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
      theme: resolveTerminalTheme(readTerminalTheme()),
      fontFamily: resolveTerminalFontStack(readTerminalFontFamily()),
      fontSize: readTerminalFontSize(),
      cursorBlink: true,
      cursorStyle: readTerminalCursorStyle(),
      scrollback: readTerminalScrollback(),
      allowProposedApi: true,
      scrollSensitivity: 3,
      fastScrollSensitivity: 8,
      smoothScrollDuration: 125,
      // OSC 8 hyperlinks (emitted by claude-code and other modern CLIs) are
      // handled by xterm natively, not by WebLinksAddon. The default activator
      // calls window.open() which is a silent no-op in WKWebView/Tauri, so we
      // route them through the same plugin-opener path as regex-detected URLs.
      linkHandler: {
        activate: (_event, uri) => {
          void openUrl(uri).catch((err) => {
            console.error("[terminal] OSC 8 openUrl failed:", err);
          });
        },
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const webLinks = new WebLinksAddon((_event, uri) => {
      // Plain click opens, matching the feed's link behavior. xterm's link
      // service only fires `activate` on a click that lands inside a detected
      // URL (with cursor=pointer), so drag-to-select doesn't accidentally
      // trigger this — the iTerm Cmd+click parity wasn't worth the
      // discoverability cost.
      void openUrl(uri).catch((err) => {
        console.error("[terminal] openUrl failed:", err);
      });
    });
    term.loadAddon(webLinks);
    term.open(containerRef.current);
    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
    } catch {
      // No WebGL — fall through to canvas. RunnerChat does the same.
    }
    const initialRect = containerRef.current.getBoundingClientRect();
    traceTerm("mount", {
      sessionId: sessionIdRef.current,
      rectW: Math.round(initialRect.width),
      rectH: Math.round(initialRect.height),
      preCols: term.cols,
      preRows: term.rows,
    });
    if (initialRect.width > 0 && initialRect.height > 0) {
      fit.fit();
      traceTerm("mount post-fit", {
        sessionId: sessionIdRef.current,
        postCols: term.cols,
        postRows: term.rows,
      });
    }
    // Don't auto-focus on mount: in the workspace, multiple
    // RunnerTerminals mount at once before any tab is selected, and the
    // last-mounted one would steal focus and shove the page into its
    // own scroll position. The activation effect below grabs focus when
    // the tab becomes active.

    const onDataDisposable = term.onData((data) => {
      const sid = sessionIdRef.current;
      if (!sid || disabledRef.current) return;
      void api.session.injectStdin(sid, data).catch((e) => {
        onErrorRef.current?.(String(e));
      });
    });

    // Shift+Enter → ESC+CR so claude-code/codex insert a newline in their
    // input frame instead of submitting. Plain Enter falls through to the
    // default \r emission via onData above.
    //
    // We must intercept both keydown AND keypress: WKWebView fires a
    // legacy `keypress` for Shift+Enter, and xterm's `_keyPress` will
    // emit \r (same as plain Enter) unless this handler also returns
    // false for that event (see #99).
    term.attachCustomKeyEventHandler((e) => {
      if (
        e.key === "Enter" &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.metaKey
      ) {
        if (e.type === "keydown") {
          const sid = sessionIdRef.current;
          if (sid && !disabledRef.current) {
            void api.session.injectStdin(sid, "\x1b\r").catch((err) => {
              onErrorRef.current?.(String(err));
            });
          }
        }
        return false;
      }
      return true;
    });

    // Image paste support. We can't trust the OS clipboard across the
    // WKWebView boundary: when the user presses Cmd+V over the webview,
    // WebKit materializes the image into a `File` (a temp file under
    // the hood), and as a side effect NSPasteboard's `public.png`
    // representation becomes the *icon* for that temp file rather than
    // the original screenshot bytes. So the agent CLI's own
    // `pbpaste -Prefer png` (triggered by Ctrl-V) gets a generic file
    // icon instead of what the user copied (#79).
    //
    // Fix: read the bytes off the `ClipboardEvent`'s File ourselves
    // (still the original screenshot at that point), ship them to Rust,
    // which writes them back to NSPasteboard's `public.png` so the
    // agent's existing pbpaste-based flow returns the real bytes.
    // Then inject Ctrl-V (`\x16`) — claude-code / codex see Ctrl-V as
    // they would in a host terminal, attach the image with their
    // native `[Image x]` placeholder. Pure-text pastes fall through
    // to xterm.js's default behavior unchanged.
    const onPaste = (e: ClipboardEvent) => {
      const sid = sessionIdRef.current;
      if (!sid || disabledRef.current) return;
      const items = e.clipboardData?.items;
      if (!items) return;
      // PNG-only for now. The clipboard-restore path writes the
      // bytes verbatim into NSPasteboard's `public.png` flavor, so
      // non-PNG payloads would end up labeled PNG with non-PNG bytes
      // and decode as garbage in the agent. macOS screenshots are
      // PNG; JPEG/GIF/WebP support needs either a per-MIME OSType
      // map or a transcode step — out of scope for v1 (#79
      // follow-up).
      let imageFile: File | null = null;
      for (let i = 0; i < items.length; i += 1) {
        const it = items[i];
        if (it.type === "image/png") {
          imageFile = it.getAsFile();
          if (imageFile) break;
        }
      }
      if (!imageFile) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      void (async () => {
        try {
          const buf = await imageFile.arrayBuffer();
          await api.session.pasteImage(new Uint8Array(buf));
          await api.session.injectStdin(sid, "\x16");
        } catch (err) {
          onErrorRef.current?.(String(err));
        }
      })();
    };
    const textarea = term.textarea;
    textarea?.addEventListener("paste", onPaste, { capture: true });

    let resizeTimer: number | null = null;

    // Dedupe by last-pushed dims. See `lastPushedColsRef` comment for why.
    const pushSize = () => {
      const t = termRef.current;
      const sid = sessionIdRef.current;
      if (!t || !sid || disabledRef.current) return;
      if (!replayDoneRef.current) return;
      if (
        t.cols === lastPushedColsRef.current &&
        t.rows === lastPushedRowsRef.current
      ) {
        return;
      }
      lastPushedColsRef.current = t.cols;
      lastPushedRowsRef.current = t.rows;
      traceTerm("pushSize", {
        sessionId: sid,
        cols: t.cols,
        rows: t.rows,
        alt: altMode(t),
      });
      void api.session.resize(sid, t.cols, t.rows).catch(() => {
        // session may have exited
      });
    };
    const schedulePushSize = () => {
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        resizeTimer = null;
        pushSize();
      }, 140);
    };
    // Refit immediately, but coalesce backend geometry pushes. Claude Code's
    // main-screen redraws on SIGWINCH are expensive and can leave stale
    // intermediate frames in scrollback if we forward every drag tick.
    const refitAndPush = () => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      try {
        fit.fit();
        schedulePushSize();
      } catch {
        // teardown
      }
    };
    window.addEventListener("resize", refitAndPush);
    // Panel toggles (left sidebar collapse, right rail) animate the
    // container's width without firing window-resize, so the xterm
    // grid and backend PTY geometry stay stale until the user nudges
    // the OS window (#108). Observing the container catches those
    // CSS-driven size changes; measurable-rect guards keep truly hidden
    // panes from pushing stale geometry to the backend.
    const ro = new ResizeObserver(() => refitAndPush());
    ro.observe(containerRef.current);

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

    // Live updates from SettingsModal. localStorage's `storage` event
    // doesn't fire in the originating window, so the modal dispatches a
    // synthetic one after each write (via `notifySameWindowStorage`). We
    // always re-read through the typed readers so the clamp/normalize
    // path is identical to mount-time — an out-of-range write can't
    // poison `term.options`.
    const onStorage = (e: StorageEvent) => {
      const t = termRef.current;
      if (!t) return;
      try {
        if (e.key === STORAGE_TERMINAL_FONT_SIZE) {
          t.options.fontSize = readTerminalFontSize();
          // Cell metrics changed — refit and push the new PTY geometry
          // so an active streaming TUI doesn't keep writing against
          // stale cols/rows until the next window resize.
          refitAndPush();
        } else if (e.key === STORAGE_TERMINAL_FONT_FAMILY) {
          t.options.fontFamily = resolveTerminalFontStack(
            readTerminalFontFamily(),
          );
          refitAndPush();
        } else if (e.key === STORAGE_TERMINAL_CURSOR_STYLE) {
          t.options.cursorStyle = readTerminalCursorStyle();
        } else if (e.key === STORAGE_TERMINAL_SCROLLBACK) {
          t.options.scrollback = readTerminalScrollback();
        } else if (e.key === STORAGE_TERMINAL_THEME) {
          t.options.theme = resolveTerminalTheme(readTerminalTheme());
        }
      } catch {
        // xterm may reject runtime mutation of some options; the next
        // mount will pick up the persisted value either way.
      }
    };
    window.addEventListener("storage", onStorage);

    termRef.current = term;
    fitRef.current = fit;

    return () => {
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      ro.disconnect();
      window.removeEventListener("resize", refitAndPush);
      window.removeEventListener("focus", refreshTerm);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("storage", onStorage);
      textarea?.removeEventListener("paste", onPaste, { capture: true });
      onDataDisposable.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, []);

  // Subscribe to the bound session's output + exit. The listener is registered
  // before snapshot replay. We intentionally do NOT fetch the snapshot here:
  // mission panes mount while hidden on the Feed tab, and replaying tmux's
  // visible grid into xterm's default 80-col hidden buffer bakes in the drift
  // that #150 is about. The activation effect below fetches the first replay
  // once the pane is measurable.
  useEffect(() => {
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let cancelled = false;
    let markReady: () => void = () => {};
    outputListenerReadyRef.current = new Promise((resolve) => {
      markReady = resolve;
    });

    replayDoneRef.current = false;
    replayInFlightRef.current = false;
    pendingLiveRef.current = [];
    lastWrittenSeqRef.current = 0;

    void (async () => {
      const [fnOut, fnExit] = await Promise.all([
        listen<OutputEvent>("session/output", (event) => {
          if (event.payload.session_id !== sessionId) return;
          if (!replayDoneRef.current) {
            pendingLiveRef.current.push(event.payload);
            return;
          }
          if (event.payload.seq <= lastWrittenSeqRef.current) return;
          writeOutputEvent(event.payload);
          lastWrittenSeqRef.current = event.payload.seq;
        }),
        listen<ExitEvent>("session/exit", (event) => {
          if (event.payload.session_id !== sessionId) return;
          onExitRef.current?.(event.payload);
        }),
      ]);
      if (cancelled) {
        fnOut();
        fnExit();
        markReady();
        return;
      }
      unlistenOutput = fnOut;
      unlistenExit = fnExit;
      markReady();
    })();

    return () => {
      cancelled = true;
      markReady();
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [sessionId, writeOutputEvent]);

  // Activation effect: when this tab moves to the front, wait for the pane
  // to become measurable, fit to its container, replay the first snapshot at
  // that exact grid, repaint the renderer, and grab focus for live panes.
  useEffect(() => {
    let cancelled = false;
    let raf1 = 0;
    let raf2 = 0;

    const flushPendingLive = () => {
      const pending = pendingLiveRef.current;
      pendingLiveRef.current = [];
      for (const ev of pending) {
        if (ev.seq <= lastWrittenSeqRef.current) continue;
        writeOutputEvent(ev);
        lastWrittenSeqRef.current = ev.seq;
      }
    };

    const replayAtSize = async (cols: number, rows: number) => {
      if (replayDoneRef.current || replayInFlightRef.current) return;
      replayInFlightRef.current = true;
      try {
        await outputListenerReadyRef.current;
        if (cancelled || sessionIdRef.current !== sessionId) return;
        const live = !disabledRef.current;
        let snapshot: SessionOutputSnapshot;
        try {
          snapshot = live
            ? await api.session.attachSnapshot(sessionId, cols, rows)
            : await api.session.outputSnapshot(sessionId);
        } catch (err) {
          if (!live) throw err;
          // If the pane exited between render and activation, fall back
          // to the bounded in-memory buffer so the terminal still shows
          // whatever scrollback the manager retained.
          snapshot = await api.session.outputSnapshot(sessionId);
        }
        if (cancelled || sessionIdRef.current !== sessionId) return;

        const t = termRef.current;
        if (!t) return;
        t.reset();
        t.clear();
        const lastSeq = snapshot.last_seq;
        for (const ev of snapshot.events) {
          writeOutputEvent(ev);
        }
        lastWrittenSeqRef.current = Math.max(
          lastWrittenSeqRef.current,
          lastSeq,
        );
        replayDoneRef.current = true;
        if (live) {
          lastPushedColsRef.current = cols;
          lastPushedRowsRef.current = rows;
        }
        flushPendingLive();
        try {
          t.refresh(0, t.rows - 1);
        } catch {
          // teardown
        }
        if (live && activeRef.current) t.focus();
      } catch (e) {
        if (!cancelled) onErrorRef.current?.(String(e));
      } finally {
        replayInFlightRef.current = false;
      }
    };

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
        const cols = t.cols;
        const rows = t.rows;
        if (!replayDoneRef.current) {
          void replayAtSize(cols, rows);
          return;
        }
        if (!activeRef.current) return;
        if (disabledRef.current) return;
        t.focus();
        // Single resize is enough once xterm enters alt-screen at attach
        // time (see docs/impls/0009). The earlier cols-1 → cols dance was
        // there to coax claude-code into a repaint that would land where
        // the user could see it; with the alt-screen state correct, the
        // agent's single SIGWINCH redraw lands in the right buffer.
        // Dedupe against the last value pushSize sent — common when this
        // effect fires immediately after the mount-time fit pushed the
        // same dims.
        if (
          cols !== lastPushedColsRef.current ||
          rows !== lastPushedRowsRef.current
        ) {
          lastPushedColsRef.current = cols;
          lastPushedRowsRef.current = rows;
          void api.session.resize(sessionId, cols, rows).catch(() => {
            // session may have exited
          });
        }
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
  }, [active, sessionId, writeOutputEvent]);

  return (
    <div className="h-full w-full overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
}
