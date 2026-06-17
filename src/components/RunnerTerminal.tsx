// Embedded xterm.js bound to a single live session.
//
// Direct chat and the mission workspace mount one of these per session and
// keep them alive while switching between tabs/routes. Output keeps streaming
// into hidden instances so each PTY's scrollback survives UI switches.
//
// Setup: WebGL renderer for cell-row alignment, base64 PTY frames to preserve
// raw bytes, backend snapshot replay for late attach, and SIGWINCH dance on
// attach so claude-code/codex repaint onto a fresh grid.

import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";

import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { api } from "../lib/api";
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
  /** Runtime kind of the runner driving this session (e.g.
   *  `"claude-code"`, `"codex"`, `"shell"`). Used to gate the
   *  scrollback-clear on resize: TUI agents whose `SIGWINCH` repaint
   *  policy fully redraws the screen get a hard-clear before the
   *  resize lands, so the previous frame doesn't stay visible in
   *  scrollback. Plain shells skip the clear and keep their history.
   *  See `runtimeClearsOnResize`. */
  runnerRuntime: string;
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
}

/**
 * Imperative handle exposed to the parent so resume/spawn callers can
 * size the backend PTY to the actual xterm geometry before the child
 * is forked. Without this, `pty_runtime` defaults to 80×24 and the
 * agent CLI's first paint wraps at the default cols until the next
 * user-driven SIGWINCH (#resume-pty-size-mismatch).
 */
export interface RunnerTerminalHandle {
  /**
   * Refit against the current container and return the resolved xterm
   * cols/rows. Returns null if the terminal isn't mounted yet or the
   * container has no measurable size (e.g. hidden via `display:none`).
   */
  measure(): { cols: number; rows: number } | null;
}

/**
 * Should resizing this runtime hard-clear xterm's scrollback before
 * pushing the new geometry to the backend?
 *
 * For TUI agents (claude-code, codex) the SIGWINCH-driven repaint
 * fully redraws the screen at the new dims; without the pre-clear,
 * the prior frame stays visible above the new one ("stacking"
 * regression on every resize). For plain shells we leave scrollback
 * alone — the user's prior command output is meaningful history.
 */
function runtimeClearsOnResize(runtime: string): boolean {
  return runtime === "claude-code" || runtime === "codex";
}

function decodeBase64Chunk(data: string): Uint8Array {
  const raw = atob(data);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i += 1) {
    bytes[i] = raw.charCodeAt(i);
  }
  return bytes;
}

export const RunnerTerminal = forwardRef<
  RunnerTerminalHandle,
  RunnerTerminalProps
>(function RunnerTerminal(
  { sessionId, runnerRuntime, onExit, onError, active, disabled },
  ref,
) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // Live WebglAddon handle so the visibility / focus / font-change
  // listeners below can call `clearTextureAtlas()` on it. The WebGL
  // renderer caches every distinct (codepoint, fg, bg, style) cell
  // into a GPU texture atlas; in dev (Vite HMR re-mounts) and on
  // long-lived sessions with bold + italic + many ANSI colors the
  // atlas occasionally desyncs and ASCII codepoints render with the
  // wrong glyph until a resize triggers a refit. Rebuilding the
  // atlas on the same lifecycle hooks where we already refresh /
  // refit makes the corruption window a few hundred milliseconds at
  // worst instead of "until the user touches the window edge."
  const webglRef = useRef<WebglAddon | null>(null);
  const sessionIdRef = useRef<string>(sessionId);
  const onExitRef = useRef(onExit);
  const onErrorRef = useRef(onError);
  const activeRef = useRef(active ?? false);
  // Mirrors the `runnerRuntime` prop into a ref so the resize handler
  // — declared inside the long-lived mount effect — sees the current
  // runtime kind without a re-render restarting the whole xterm.
  const runnerRuntimeRef = useRef<string>(runnerRuntime);
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
  // Snapshot replay is deferred until the pane is both active and
  // measurable. Mission workspaces mount every slot's RunnerTerminal
  // at once with `activeTab="feed"` by default — every slot pane is
  // `display:none`, the mount-effect's `fit.fit()` is skipped (zero-
  // size rect), and xterm sits at the constructor default 80×24.
  // Replaying snapshot bytes into that 80-col grid bakes wrong cell
  // positions into the buffer, and a later `fit.fit()` on tab focus
  // can't move them. So we cache the fetched bytes here and drain
  // them only once the pane has come to the front and fit at real
  // cols. See #mission-tab-return-drift.
  const pendingSnapshotRef = useRef<OutputEvent[] | null>(null);
  const pendingLiveRef = useRef<OutputEvent[]>([]);
  const lastWrittenSeqRef = useRef(0);
  const replayDoneRef = useRef(false);
  // Bound to the latest snapshot effect's drain helper so the
  // activation effect (declared after it) can request a drain
  // without lifting the whole closure into module scope. Cleared on
  // sessionId change so a stale closure can't keep writing into the
  // previous session's xterm grid.
  const tryDrainReplayRef = useRef<(() => void) | null>(null);

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

  useEffect(() => {
    runnerRuntimeRef.current = runnerRuntime;
  }, [runnerRuntime]);

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
      // OSC 8 hyperlinks (emitted by claude-code and other modern CLIs) are
      // handled by xterm natively, not by WebLinksAddon. The default activator
      // calls window.open() which is a silent no-op in WKWebView/Tauri, so we
      // route them through the same plugin-opener path as regex-detected URLs.
      // Gated on Cmd/Ctrl to match standard terminal behaviour.
      linkHandler: {
        activate: (event, uri) => {
          if (!event.metaKey && !event.ctrlKey) return;
          void openUrl(uri).catch((err) => {
            console.error("[terminal] OSC 8 openUrl failed:", err);
          });
        },
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const webLinks = new WebLinksAddon((event, uri) => {
      // Standard terminal behaviour: only open on Cmd+click (macOS) /
      // Ctrl+click (other platforms). A plain click does nothing, so a
      // click that lands on a URL while selecting text can't open it.
      if (!event.metaKey && !event.ctrlKey) return;
      void openUrl(uri).catch((err) => {
        console.error("[terminal] openUrl failed:", err);
      });
    });
    term.loadAddon(webLinks);
    term.open(containerRef.current);
    // WebGL renderer + context-loss recovery. Without the
    // onContextLoss hook, a single GPU reset / driver hiccup / dev
    // HMR remount would leave xterm rendering against a dead context
    // and the canvas freezes mid-frame. Disposing the addon on loss
    // lets xterm fall back to the DOM renderer for the rest of this
    // mount — degraded but functional, no more frozen panes.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        webglRef.current = null;
      });
      term.loadAddon(webgl);
      webglRef.current = webgl;
    } catch {
      // No WebGL — fall through to canvas. RunnerChat does the same.
    }
    const initialRect = containerRef.current.getBoundingClientRect();
    if (initialRect.width > 0 && initialRect.height > 0) {
      fit.fit();
      // Push the freshly-fitted dims to the backend right here, before
      // the snapshot effect below fires its outputSnapshot RPC. The
      // backend's buffered bytes were emitted by the agent at whatever
      // cols the PTY was last sized to — if that differs from xterm's
      // current cols (common on route returns: chat → mission, mission
      // → chat), replaying those bytes at the new cols drifts every
      // absolute-positioned glyph and leaves the alt-screen blank
      // (#mission-tab-return-drift). Pushing first ensures backend +
      // xterm agree on cols before we read the snapshot, and the
      // SIGWINCH-driven repaint that follows arrives via the live
      // listener at the same cols xterm now uses.
      //
      // Hidden panes (rect 0) skip this — the activation effect picks
      // up the push when they come to the front, same as before.
      lastPushedColsRef.current = term.cols;
      lastPushedRowsRef.current = term.rows;
      // sessionIdRef is initialized with the prop value (line ~124),
      // so this reads the right id on initial mount without forcing
      // sessionId into the mount-effect's deps (which is intentionally
      // `[]` to avoid tearing down the whole xterm on session swap).
      void api.session
        .resize(sessionIdRef.current, term.cols, term.rows)
        .catch(() => {
          // session may have exited before mount; nothing to do
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

    // App-level Command shortcuts that should win even while xterm owns
    // focus. Ctrl shortcuts are left to the PTY/TUI.
    // WKWebView/xterm can keep these from reaching AppShell's global
    // keydown listener, so dispatch the same shell event from here and
    // return false to keep the shortcut out of the PTY.
    //
    // Shift+Enter → ESC+CR so claude-code/codex insert a newline in their
    // input frame instead of submitting. Plain Enter falls through to the
    // default \r emission via onData above.
    //
    // We must intercept both keydown AND keypress: WKWebView fires a
    // legacy `keypress` for Shift+Enter, and xterm's `_keyPress` will
    // emit \r (same as plain Enter) unless this handler also returns
    // false for that event (see #99).
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.metaKey) {
        const key = e.key.toLowerCase();
        if (key === "s" || e.key === "\\") {
          e.preventDefault();
          window.dispatchEvent(new Event("runner:toggle-sidebar"));
          return false;
        }
      }
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

    // Dedupe by last-pushed dims. See `lastPushedColsRef` comment for why.
    const pushSize = () => {
      const t = termRef.current;
      const sid = sessionIdRef.current;
      if (!t || !sid || disabledRef.current) return;
      if (
        t.cols === lastPushedColsRef.current &&
        t.rows === lastPushedRowsRef.current
      ) {
        return;
      }
      lastPushedColsRef.current = t.cols;
      lastPushedRowsRef.current = t.rows;
      // Clear the visible region before the SIGWINCH-driven redraw
      // lands for full-screen TUI agents. Without this, claude-code /
      // codex repaint at the new dims and the prior frame's visible
      // rows get pushed into scrollback as the new paint arrives —
      // the "stacking" UX bug. We deliberately do NOT also write
      // `\x1b[3J` (erase saved lines): wiping the scrollback on every
      // resize made it impossible to scroll up to older conversation
      // history after touching the window edge. The visible-region
      // wipe alone is enough to prevent the duplicated-frame artifact,
      // and any older scrollback the user had accumulated stays
      // intact. Plain shells skip the wipe entirely and keep their
      // history. See docs/impls/0011-pty-host-terminal-runtime.md
      // §"Per-runtime clear-on-resize".
      if (runtimeClearsOnResize(runnerRuntimeRef.current)) {
        // ESC[2J — erase visible region
        // ESC[H  — cursor home
        t.write("\x1b[2J\x1b[H");
      }
      void api.session.resize(sid, t.cols, t.rows).catch(() => {
        // session may have exited
      });
    };
    // Refit + push backend geometry when the pane is active and
    // measurable. Hidden panes don't refit/push — the activation
    // effect picks up the new metrics when they come to the front.
    const refitAndPush = () => {
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
    window.addEventListener("resize", refitAndPush);
    // Panel toggles (left sidebar collapse, right rail) animate the
    // container's width without firing window-resize, so the xterm
    // grid and backend PTY geometry stay stale until the user nudges
    // the OS window (#108). Observing the container catches those
    // CSS-driven size changes; refitAndPush's activeRef + measurable-
    // rect guards keep hidden panes from pushing stale geometry to
    // the backend.
    const ro = new ResizeObserver(() => refitAndPush());
    ro.observe(containerRef.current);

    const refreshTerm = () => {
      const t = termRef.current;
      if (!t) return;
      try {
        // Rebuild the WebGL glyph atlas before the redraw. The atlas
        // occasionally desyncs while the app is backgrounded (other
        // GL apps stealing the GPU, OS compositor recycling, dev HMR
        // re-running effects), and the symptom is plain ASCII
        // codepoints rendering with the wrong glyph until a resize
        // forces a refit. Clearing the atlas here costs one frame's
        // worth of glyph re-rasterization on focus / tab-visible /
        // font-change and eliminates the "resize-to-fix" workaround.
        webglRef.current?.clearTextureAtlas();
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
          // Cell metrics changed — refit, push the new PTY geometry,
          // and drop the atlas. The atlas indexes cells by their
          // rendered pixel dimensions; a stale cache after a font
          // change can leave a band of pre-change glyphs at the new
          // size until something else evicts them.
          webglRef.current?.clearTextureAtlas();
          refitAndPush();
        } else if (e.key === STORAGE_TERMINAL_FONT_FAMILY) {
          t.options.fontFamily = resolveTerminalFontStack(
            readTerminalFontFamily(),
          );
          webglRef.current?.clearTextureAtlas();
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
  // before snapshot replay so live chunks that arrive during the command round
  // trip are buffered and merged by seq.
  useEffect(() => {
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let cancelled = false;
    // Fresh sessionId → fresh replay bookkeeping. Done in the body
    // (not just on cleanup) so an early prop change can't leak state
    // from the previous session's still-pending fetch.
    pendingSnapshotRef.current = null;
    pendingLiveRef.current = [];
    lastWrittenSeqRef.current = 0;
    replayDoneRef.current = false;

    const writeOutput = (ev: OutputEvent) => {
      termRef.current?.write(decodeBase64Chunk(ev.data));
    };

    // Replay drains only when (a) the snapshot RPC has returned,
    // (b) the pane is currently active, and (c) the container has a
    // measurable rect so the in-line fit gives us real cols/rows.
    // Until all three line up we keep the bytes parked on
    // pendingSnapshotRef and pendingLiveRef; activation / resize
    // observers re-call this helper as conditions change.
    const tryDrainReplay = () => {
      if (replayDoneRef.current) return;
      if (!activeRef.current) return;
      const t = termRef.current;
      const fit = fitRef.current;
      const node = containerRef.current;
      if (!t || !fit || !node) return;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      if (pendingSnapshotRef.current === null) return;

      try {
        fit.fit();
      } catch {
        // teardown in progress
        return;
      }

      t.reset();
      for (const ev of pendingSnapshotRef.current) {
        writeOutput(ev);
        lastWrittenSeqRef.current = Math.max(
          lastWrittenSeqRef.current,
          ev.seq,
        );
      }
      pendingSnapshotRef.current = null;
      for (const ev of pendingLiveRef.current) {
        if (ev.seq <= lastWrittenSeqRef.current) continue;
        writeOutput(ev);
        lastWrittenSeqRef.current = ev.seq;
      }
      pendingLiveRef.current = [];
      replayDoneRef.current = true;
    };
    tryDrainReplayRef.current = tryDrainReplay;

    void (async () => {
      const [fnOut, fnExit] = await Promise.all([
        listen<OutputEvent>("session/output", (event) => {
          if (event.payload.session_id !== sessionId) return;
          if (!replayDoneRef.current) {
            pendingLiveRef.current.push(event.payload);
            // The snapshot may have already arrived and be waiting
            // on activation; nudge the drain in case the live event
            // arrived after the user just brought the pane forward.
            tryDrainReplay();
            return;
          }
          if (event.payload.seq <= lastWrittenSeqRef.current) return;
          writeOutput(event.payload);
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

      // Park the snapshot for the activation effect to drain. For
      // panes that are already active and measurable this is a
      // straight drain; for `display:none` panes (mission's
      // non-active slots) the bytes sit here until tab focus.
      pendingSnapshotRef.current = snapshot;
      tryDrainReplay();
    })();

    return () => {
      cancelled = true;
      tryDrainReplayRef.current = null;
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
        // Drain any pending snapshot replay now that we have a real
        // cols/rows — this is the path that fires when mission slot
        // tabs (mounted hidden by default, snapshot already fetched
        // into pendingSnapshotRef) finally come to the front.
        tryDrainReplayRef.current?.();
        // Drop the WebGL glyph atlas before the refresh: in-app tab /
        // route switches (Mission ↔ Chat, mission tab switches) keep
        // hidden terminals mounted, and the atlas can desync while a
        // pane is off-screen (other panes painting into the same GL
        // context, OS compositor recycling, dev HMR). Without this,
        // coming back to a hidden pane shows mis-rendered glyphs until
        // a resize forces a refit — the bug `refreshTerm`'s atlas
        // clear was meant to prevent, but `refreshTerm` is wired to
        // `focus` / `visibilitychange` and those don't fire on in-app
        // tab switches.
        webglRef.current?.clearTextureAtlas();
        t.refresh(0, t.rows - 1);
        t.focus();
        // No agent to dance with for stopped/crashed/resuming/starting
        // sessions — the wipe would clear the last-paint behind the
        // SessionEndedOverlay (the scrollback the `disabled` prop is
        // meant to preserve) and the resize ioctls would reject. Keep
        // the fit + refresh + focus above (correct for any active
        // pane) but bail before the wipe + dance.
        if (disabledRef.current) return;
        const cols = t.cols;
        const rows = t.rows;
        // Force the agent into a full redraw on every activation.
        // While the pane was hidden, live PTY bytes wrote into
        // xterm's buffer (the session-effect's "session/output"
        // listener doesn't gate on active), so the buffer may be
        // mid-frame (agent halfway through a redraw at switch
        // time) or dim-mismatched (window / panel resize while
        // hidden moved the container without us refitting). The
        // refresh() above only repaints that broken state — we
        // need the *agent* to redraw, which means a SIGWINCH.
        //
        // Both Linux (tty_do_resize) and macOS (TIOCSWINSZ in
        // ttioctl) compare the incoming winsize against the cached
        // value and only signal SIGWINCH when the bytes differ —
        // a same-size resize is a kernel no-op. So a single
        // unconditional api.session.resize wouldn't be enough on
        // its own. We dance through (cols, rows-1) → (cols, rows)
        // so both ioctls produce a SIGWINCH and the agent repaints
        // at the final dims.
        //
        // Perturb *rows*, not cols. An earlier draft used cols-1
        // and corrupted scrollback: claude-code (and similar TUIs)
        // wrap their own text by emitting explicit `\n` at the
        // computed cols boundary, so any intermediate paint at
        // cols-1 deposits hard-wrapped narrow lines into xterm's
        // buffer. xterm can only soft-reflow width-wrapped lines;
        // explicit newlines stick. Every tab return then left a
        // layer of narrower lines above the current paint, visible
        // on scroll-up. Row perturbation keeps content width at
        // cols throughout — the (rows-1) intermediate is just one
        // line shorter, and the second SIGWINCH's `\x1b[2J`-led
        // repaint at the final rows count cleans up.
        //
        // For TUI runtimes, wipe the viewport first so the user
        // sees a clean black canvas during the brief gap before
        // the redraw lands instead of the scattered mid-frame
        // mess (#177). Plain shells skip the wipe — matches
        // pushSize's behavior above (runtimeClearsOnResize), they
        // keep their history and don't repaint on SIGWINCH.
        if (runtimeClearsOnResize(runnerRuntimeRef.current)) {
          t.write("\x1b[2J\x1b[H");
        }
        lastPushedColsRef.current = cols;
        lastPushedRowsRef.current = rows;
        // Guard the pathological 1-row case so we never ioctl to
        // 0 rows. portable-pty would reject it, but more importantly
        // we want both directions of the dance to produce a real
        // winsize-diff and therefore SIGWINCH.
        const nudgedRows = rows > 1 ? rows - 1 : rows + 1;
        void api.session
          .resize(sessionId, cols, nudgedRows)
          .then(() => api.session.resize(sessionId, cols, rows))
          .catch(() => {
            // session may have exited between the two ioctls
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

  useImperativeHandle(
    ref,
    () => ({
      measure: () => {
        const t = termRef.current;
        const fit = fitRef.current;
        const node = containerRef.current;
        if (!t || !fit || !node) return null;
        const rect = node.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) {
          // Hidden pane (display:none via MissionWorkspace's Pane
          // wrapper) — no rect to fit against. If a prior activation
          // already fit this terminal, t.cols/t.rows still hold those
          // dims, and they're far more useful at resume time than
          // returning null (which forces the resume RPC to pass null
          // → backend defaults to 80×24 → agent paints its `--resume`
          // conversation history at 80 cols, and for main-screen TUIs
          // those hard-wrapped lines stick in scrollback). 80×24 is
          // the constructor default / "never fit" sentinel; treat it
          // the same as null so callers can still fall back.
          if (t.cols === 80 && t.rows === 24) return null;
          return { cols: t.cols, rows: t.rows };
        }
        try {
          // Force a fit before reading dims: stopped tabs gate their
          // resize listeners on activeRef, so cols/rows can be stale
          // (often still 80×24 from the initial Terminal construction)
          // by the time the user clicks Resume.
          fit.fit();
          return { cols: t.cols, rows: t.rows };
        } catch {
          return null;
        }
      },
    }),
    [],
  );

  return (
    <div className="h-full w-full overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
});
