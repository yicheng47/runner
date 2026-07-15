// Embedded xterm.js bound to a single live session.
//
// Direct chat and the mission workspace mount one of these per session and
// keep them alive while switching between tabs/routes. Output keeps streaming
// into hidden instances so each PTY's scrollback survives UI switches.
//
// Setup: WebGL renderer for cell-row alignment, base64 PTY frames to preserve
// raw bytes, backend snapshot replay for late attach, and SIGWINCH dance on
// attach so claude-code/codex repaint onto a fresh grid.

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { api, type PasteImageMimeType } from "../lib/api";
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
import {
  shouldDelayTerminalResize,
  type TerminalGridSize,
} from "../lib/terminalResize";
import { eventMatchesShortcut } from "../lib/keymap";

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

const MAX_PENDING_LIVE_EVENTS = 4096;
const SIDEBAR_TOGGLE_EVENT = "runner:toggle-sidebar";
const SIDEBAR_NAVIGATE_EVENT = "runner:navigate-sidebar-page";
const RUNNER_TERMINAL_CYCLE_EVENT = "runner:cycle-terminal";
const OPEN_SETTINGS_EVENT = "runner:open-settings";

function normalizePasteImageMime(type: string): PasteImageMimeType | null {
  switch (type.trim().toLowerCase()) {
    case "image/png":
      return "image/png";
    case "image/jpeg":
    case "image/jpg":
      return "image/jpeg";
    default:
      return null;
  }
}

function inferPasteImageMime(
  itemType: string,
  file: File,
): PasteImageMimeType | null {
  const fromType =
    normalizePasteImageMime(itemType) ?? normalizePasteImageMime(file.type);
  if (fromType) return fromType;

  const name = file.name.toLowerCase();
  if (name.endsWith(".png")) return "image/png";
  if (name.endsWith(".jpg") || name.endsWith(".jpeg")) return "image/jpeg";
  return null;
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
  /** True while this terminal's pane is visible/measurable. A visible
   *  terminal may still be disabled, e.g. a stopped mission slot that
   *  should replay dimmed scrollback without accepting input. */
  active?: boolean;
  /** Whether activation may steal keyboard focus. Defaults to true (the
   *  single-visible-terminal surfaces want focus to follow activation).
   *  Split chat (impl 0020) shows several active terminals at once and
   *  passes false for every pane except the focused one, so mounting a
   *  sibling pane can't yank keystrokes away from the focused chat. */
  autoFocus?: boolean;
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
  /** Grab keyboard focus (no-op while disabled). Split chat calls this
   *  when pane focus moves without a remount, e.g. a pane-header click. */
  focus(): void;
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
  { sessionId, runnerRuntime, onExit, onError, active, autoFocus, disabled },
  ref,
) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // Live WebglAddon handle so the font-change listeners below can call
  // `clearTextureAtlas()` on it. The WebGL renderer caches every distinct
  // (codepoint, fg, bg, style) cell into a GPU texture atlas keyed by
  // rendered pixel size, so a font-size / font-family change must evict
  // the atlas or a band of pre-change glyphs lingers at the old metrics
  // until something else rebuilds it. (The old cross-pane atlas-corruption
  // mitigation is gone: that "wrong glyph" bug was an upstream atlas
  // page-merge defect, fixed in @xterm/addon-webgl — see the renderer
  // setup below.)
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
  // Mirrors `autoFocus` for the activation effect below.
  const autoFocusRef = useRef<boolean>(autoFocus ?? true);
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
  const tryDrainReplayRef = useRef<(() => boolean) | null>(null);
  const replayFlushPendingRef = useRef(false);
  const replayAfterFlushRef = useRef<Array<() => void>>([]);
  const pendingLiveOverflowRef = useRef(false);
  const snapshotRefreshPendingRef = useRef(false);
  // A just-replayed snapshot already paints the current TUI frame,
  // including SGR-dependent background cells. The activation resize
  // dance should still wake the backend PTY, but must not locally
  // clear those cells first or Codex can repaint text without the
  // gray input background.
  const replayJustDrainedRef = useRef(false);

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
    autoFocusRef.current = autoFocus ?? true;
  }, [autoFocus]);

  const refreshActiveTerminal = useCallback(
    ({
      focus = false,
      forceResizeDance = false,
      pushBackendSize = false,
    }: {
      focus?: boolean;
      forceResizeDance?: boolean;
      pushBackendSize?: boolean;
    } = {}) => {
      if (!activeRef.current) return false;
      const t = termRef.current;
      const fit = fitRef.current;
      const node = containerRef.current;
      if (!t || !fit || !node) return false;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      try {
        const beforeCols = t.cols;
        const beforeRows = t.rows;
        fit.fit();
        if (t.cols !== beforeCols || t.rows !== beforeRows) {
          console.info(
            `[terminal] refresh-fit session=${sessionIdRef.current} ` +
              `${beforeCols}x${beforeRows} -> ${t.cols}x${t.rows} ` +
              `disabled=${disabledRef.current} forceDance=${forceResizeDance} ` +
              `pushBackend=${pushBackendSize}`,
          );
        }
        tryDrainReplayRef.current?.();
        if (replayFlushPendingRef.current) {
          if (focus && !disabledRef.current) t.focus();
          replayAfterFlushRef.current.push(() => {
            window.requestAnimationFrame(() => {
              refreshActiveTerminal({
                focus,
                forceResizeDance,
                pushBackendSize,
              });
            });
          });
          return true;
        }
        t.refresh(0, t.rows - 1);
        if (focus && !disabledRef.current) t.focus();
        if ((!forceResizeDance && !pushBackendSize) || disabledRef.current) {
          if ((forceResizeDance || pushBackendSize) && disabledRef.current) {
            console.info(
              `[terminal] push-suppressed session=${sessionIdRef.current} ` +
                `cols=${t.cols} rows=${t.rows} ` +
                `lastPushed=${lastPushedColsRef.current}x${lastPushedRowsRef.current} ` +
                `forceDance=${forceResizeDance} pushBackend=${pushBackendSize}`,
            );
          }
          return true;
        }
        const sid = sessionIdRef.current;
        if (!sid) return true;
        const cols = t.cols;
        const rows = t.rows;
        if (!forceResizeDance) {
          if (
            cols === lastPushedColsRef.current &&
            rows === lastPushedRowsRef.current
          ) {
            console.info(
              `[terminal] refresh-push-skip session=${sid} cols=${cols} rows=${rows} ` +
                `prev=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
            );
            return true;
          }
          console.info(
            `[terminal] refresh-push session=${sid} cols=${cols} rows=${rows} ` +
              `prev=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
          );
          lastPushedColsRef.current = cols;
          lastPushedRowsRef.current = rows;
          void api.session.resize(sid, cols, rows).catch(() => {
            // session may have exited
          });
          return true;
        }
        // Force a full TUI redraw even when the final geometry
        // matches the backend's cached winsize. Same-size TIOCSWINSZ
        // calls are kernel no-ops on macOS/Linux, so we perturb rows
        // only: width stays constant, avoiding hard-wrapped narrow
        // lines in scrollback, while both ioctls still emit SIGWINCH.
        //
        // Skip the local clear when the grid size hasn't changed since
        // the last push: the clear exists to stop reflow stacking, and
        // with unchanged dims codex overdraws in place — clearing first
        // discards SGR background cells (the gray input box) that the
        // SIGWINCH repaint doesn't re-emit. Fresh split panes hit this
        // on activation right after their first paint (impl 0020).
        const dimsUnchanged =
          cols === lastPushedColsRef.current &&
          rows === lastPushedRowsRef.current;
        const skipLocalClear = replayJustDrainedRef.current || dimsUnchanged;
        console.info(
          `[terminal] resize-dance session=${sid} cols=${cols} rows=${rows} ` +
            `lastPushed=${lastPushedColsRef.current}x${lastPushedRowsRef.current} ` +
            `skipLocalClear=${skipLocalClear}`,
        );
        if (runtimeClearsOnResize(runnerRuntimeRef.current) && !skipLocalClear) {
          t.write("\x1b[2J\x1b[H");
        }
        replayJustDrainedRef.current = false;
        lastPushedColsRef.current = cols;
        lastPushedRowsRef.current = rows;
        const nudgedRows = rows > 1 ? rows - 1 : rows + 1;
        void api.session
          .resize(sid, cols, nudgedRows)
          .then(() => api.session.resize(sid, cols, rows))
          .catch(() => {
            // session may have exited between the two ioctls
          });
        return true;
      } catch {
        // Layout not ready yet — the next activation / resize will drive it.
        return false;
      }
    },
    [],
  );

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
    // WebGL renderer + context-loss recovery. The glyph-atlas corruption
    // that used to garble sibling panes under agentic-CLI output — a wide
    // range of styled glyphs forces atlas page merges, and a merge bug
    // sampled the wrong page → correct layout, wrong glyphs — is fixed
    // upstream in @xterm/addon-webgl >=0.20.0-beta.219 (xtermjs/xterm.js
    // #5883). So we no longer coordinate atlases across panes; each
    // terminal just guards its own context loss. Without the onContextLoss
    // hook, a GPU reset / driver hiccup / dev HMR remount would leave
    // xterm rendering against a dead context and the canvas freezes
    // mid-frame. Disposing the addon on loss reverts this terminal to the
    // DOM renderer for the rest of the mount — degraded but functional,
    // and it also covers the rarer GPU-process-death corruption the
    // atlas-merge fix doesn't (the VSCode dom-fallback pattern).
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        webglRef.current = null;
      });
      term.loadAddon(webgl);
      webglRef.current = webgl;
    } catch {
      // No WebGL — xterm keeps its DOM renderer. RunnerChat does the same.
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
        if (eventMatchesShortcut(e, "toggle-sidebar")) {
          e.preventDefault();
          window.dispatchEvent(new Event(SIDEBAR_TOGGLE_EVENT));
          return false;
        }
        if (eventMatchesShortcut(e, "open-settings")) {
          e.preventDefault();
          window.dispatchEvent(new Event(OPEN_SETTINGS_EVENT));
          return false;
        }
        if (eventMatchesShortcut(e, "page-previous")) {
          e.preventDefault();
          window.dispatchEvent(
            new CustomEvent(SIDEBAR_NAVIGATE_EVENT, {
              detail: { direction: "previous" },
            }),
          );
          return false;
        }
        if (eventMatchesShortcut(e, "page-next")) {
          e.preventDefault();
          window.dispatchEvent(
            new CustomEvent(SIDEBAR_NAVIGATE_EVENT, {
              detail: { direction: "next" },
            }),
          );
          return false;
        }
        if (
          eventMatchesShortcut(e, "pane-previous") ||
          eventMatchesShortcut(e, "mission-tab-previous")
        ) {
          e.preventDefault();
          window.dispatchEvent(
            new CustomEvent(RUNNER_TERMINAL_CYCLE_EVENT, {
              detail: { direction: "previous" },
            }),
          );
          return false;
        }
        if (
          eventMatchesShortcut(e, "pane-next") ||
          eventMatchesShortcut(e, "mission-tab-next")
        ) {
          e.preventDefault();
          window.dispatchEvent(
            new CustomEvent(RUNNER_TERMINAL_CYCLE_EVENT, {
              detail: { direction: "next" },
            }),
          );
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
    // the hood), and as a side effect NSPasteboard's image
    // representation can become the *icon* for that temp file rather
    // than the original image bytes. So the agent CLI's own clipboard
    // image read (triggered by Ctrl-V) gets a generic file icon instead
    // of what the user copied (#79).
    //
    // Fix: read the bytes off the `ClipboardEvent`'s File ourselves
    // (still the original image at that point), ship them to Rust with
    // the image MIME type, which writes them back to the matching
    // NSPasteboard flavor so the agent's existing pbpaste-based flow
    // returns the real bytes.
    // Then inject Ctrl-V (`\x16`) — claude-code / codex see Ctrl-V as
    // they would in a host terminal, attach the image with their
    // native `[Image x]` placeholder. Pure-text pastes fall through
    // to xterm.js's default behavior unchanged.
    const onPaste = (e: ClipboardEvent) => {
      const sid = sessionIdRef.current;
      if (!sid || disabledRef.current) return;
      const items = e.clipboardData?.items;
      if (!items) return;
      let imageFile: File | null = null;
      let imageMimeType: PasteImageMimeType | null = null;
      for (let i = 0; i < items.length; i += 1) {
        const it = items[i];
        if (it.kind !== "file") continue;
        const file = it.getAsFile();
        if (!file) continue;
        const mimeType = inferPasteImageMime(it.type, file);
        if (!mimeType) continue;
        imageFile = file;
        imageMimeType = mimeType;
        break;
      }
      if (!imageFile || !imageMimeType) return;
      const file = imageFile;
      const mimeType = imageMimeType;
      e.preventDefault();
      e.stopImmediatePropagation();
      void (async () => {
        try {
          const buf = await file.arrayBuffer();
          await api.session.pasteImage(new Uint8Array(buf), mimeType);
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
      console.info(
        `[terminal] push-size session=${sid} cols=${t.cols} rows=${t.rows} ` +
          `prev=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
      );
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
      // history. See docs/impls/archive/0011-pty-host-terminal-runtime.md
      // §"Per-runtime clear-on-resize".
      const skipLocalClear = replayJustDrainedRef.current;
      if (runtimeClearsOnResize(runnerRuntimeRef.current) && !skipLocalClear) {
        // ESC[2J — erase visible region
        // ESC[H  — cursor home
        t.write("\x1b[2J\x1b[H");
      }
      replayJustDrainedRef.current = false;
      void api.session.resize(sid, t.cols, t.rows).catch(() => {
        // session may have exited
      });
    };
    let stableResizeTimer: number | null = null;
    let stableResizeRaf: number | null = null;
    let pendingStableResize: TerminalGridSize | null = null;
    const clearStableResizeSchedule = () => {
      if (stableResizeTimer !== null) {
        window.clearTimeout(stableResizeTimer);
        stableResizeTimer = null;
      }
      if (stableResizeRaf !== null) {
        window.cancelAnimationFrame(stableResizeRaf);
        stableResizeRaf = null;
      }
    };
    const scheduleStableRefit = () => {
      if (stableResizeTimer !== null || stableResizeRaf !== null) return;
      stableResizeTimer = window.setTimeout(() => {
        stableResizeTimer = null;
        stableResizeRaf = window.requestAnimationFrame(() => {
          stableResizeRaf = null;
          refitAndPush({ allowPendingLargeDrop: true });
        });
      }, 150);
    };
    // Refit + push backend geometry when the pane is active and
    // measurable. Hidden panes don't refit/push — the activation
    // effect picks up the new metrics when they come to the front.
    function refitAndPush({
      allowPendingLargeDrop = false,
    }: { allowPendingLargeDrop?: boolean } = {}) {
      if (!activeRef.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      try {
        const beforeCols = term.cols;
        const beforeRows = term.rows;
        const proposed = fit.proposeDimensions();
        if (!proposed) return;
        if (
          shouldDelayTerminalResize({
            clearsOnResize: runtimeClearsOnResize(runnerRuntimeRef.current),
            current: { cols: beforeCols, rows: beforeRows },
            proposed,
            pending: pendingStableResize,
            allowPending: allowPendingLargeDrop,
          })
        ) {
          pendingStableResize = proposed;
          console.info(
            `[terminal] refit-delay session=${sessionIdRef.current} ` +
              `${beforeCols}x${beforeRows} -> ${proposed.cols}x${proposed.rows} ` +
              `disabled=${disabledRef.current} allowPending=${allowPendingLargeDrop}`,
          );
          scheduleStableRefit();
          return;
        }
        pendingStableResize = null;
        clearStableResizeSchedule();
        fit.fit();
        if (term.cols !== beforeCols || term.rows !== beforeRows) {
          console.info(
            `[terminal] refit session=${sessionIdRef.current} ` +
              `${beforeCols}x${beforeRows} -> ${term.cols}x${term.rows} ` +
              `disabled=${disabledRef.current}`,
          );
        }
        pushSize();
      } catch {
        // teardown
      }
    }
    const onResize = () => refitAndPush();
    window.addEventListener("resize", onResize);
    // Panel toggles (left sidebar collapse, right rail) animate the
    // container's width without firing window-resize, so the xterm
    // grid and backend PTY geometry stay stale until the user nudges
    // the OS window (#108). Observing the container catches those
    // CSS-driven size changes; refitAndPush's activeRef + measurable-
    // rect guards keep hidden panes from pushing stale geometry to
    // the backend.
    const ro = new ResizeObserver(() => refitAndPush());
    ro.observe(containerRef.current);

    const onVisibility = () => {
      if (document.visibilityState === "visible") scheduleWakeRefit();
    };
    const onWindowFocus = () => {
      scheduleWakeRefit();
    };
    const wakeRafs = new Set<number>();
    const wakeTimers = new Set<number>();
    let wakeRefitScheduled = false;
    let wakeResizeDancePending = false;
    let wakeResetTimer: number | null = null;
    const scheduleWakeRaf = (cb: () => void) => {
      const id = window.requestAnimationFrame(() => {
        wakeRafs.delete(id);
        cb();
      });
      wakeRafs.add(id);
      return id;
    };
    const scheduleWakeTimer = (cb: () => void, delay: number) => {
      const id = window.setTimeout(() => {
        wakeTimers.delete(id);
        cb();
      }, delay);
      wakeTimers.add(id);
      return id;
    };
    const scheduleWakeReset = (delay: number) => {
      if (wakeResetTimer !== null) {
        window.clearTimeout(wakeResetTimer);
        wakeTimers.delete(wakeResetTimer);
      }
      wakeResetTimer = scheduleWakeTimer(() => {
        wakeResetTimer = null;
        wakeRefitScheduled = false;
        wakeResizeDancePending = false;
      }, delay);
    };
    const runWakeRefit = () => {
      const refreshed = wakeResizeDancePending
        ? refreshActiveTerminal({ forceResizeDance: true })
        : refreshActiveTerminal({ pushBackendSize: true });
      if (wakeResizeDancePending && refreshed) {
        wakeResizeDancePending = false;
      }
    };
    function scheduleWakeRefit(forceResizeDance = false) {
      if (forceResizeDance) wakeResizeDancePending = true;
      if (wakeRefitScheduled) {
        if (forceResizeDance) {
          scheduleWakeRaf(runWakeRefit);
          scheduleWakeTimer(runWakeRefit, 250);
          scheduleWakeTimer(runWakeRefit, 750);
          scheduleWakeReset(1000);
        }
        return;
      }
      wakeRefitScheduled = true;
      scheduleWakeRaf(() => {
        scheduleWakeRaf(runWakeRefit);
      });
      // macOS wake/focus can fire before WKWebView has settled its
      // final layout rect. Real app resume gets a longer retry window
      // for the SIGWINCH dance; ordinary focus/visibility wakes stay
      // local unless the container size actually changed.
      scheduleWakeTimer(runWakeRefit, 250);
      if (forceResizeDance) scheduleWakeTimer(runWakeRefit, 750);
      scheduleWakeReset(forceResizeDance ? 1000 : 300);
    }
    window.addEventListener("focus", onWindowFocus);
    document.addEventListener("visibilitychange", onVisibility);
    let unlistenAppResumed: (() => void) | null = null;
    let appResumedCancelled = false;
    void listen("app/resumed", () => {
      scheduleWakeRefit(true);
    }).then((fn) => {
      if (appResumedCancelled) {
        fn();
        return;
      }
      unlistenAppResumed = fn;
    });
    let unlistenFocus: (() => void) | null = null;
    let focusCancelled = false;
    try {
      void getCurrentWindow()
        .onFocusChanged(({ payload: focused }) => {
          if (focused) scheduleWakeRefit();
        })
        .then((fn) => {
          if (focusCancelled) {
            fn();
            return;
          }
          unlistenFocus = fn;
        })
        .catch(() => {
          // Browser-level focus/visibility listeners still apply.
        });
    } catch {
      // No Tauri runtime (dev browser preview).
    }

    // Live updates from the Settings page. localStorage's `storage` event
    // doesn't fire in the originating window, so the pane dispatches a
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
      window.removeEventListener("resize", onResize);
      window.removeEventListener("focus", onWindowFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("storage", onStorage);
      appResumedCancelled = true;
      unlistenAppResumed?.();
      focusCancelled = true;
      unlistenFocus?.();
      wakeRafs.forEach((id) => window.cancelAnimationFrame(id));
      wakeTimers.forEach((id) => window.clearTimeout(id));
      clearStableResizeSchedule();
      textarea?.removeEventListener("paste", onPaste, { capture: true });
      onDataDisposable.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [refreshActiveTerminal]);

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
    replayFlushPendingRef.current = false;
    replayAfterFlushRef.current = [];
    pendingLiveOverflowRef.current = false;
    snapshotRefreshPendingRef.current = false;
    replayJustDrainedRef.current = false;

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
      if (replayDoneRef.current) return false;
      if (!activeRef.current) return false;
      const t = termRef.current;
      const fit = fitRef.current;
      const node = containerRef.current;
      if (!t || !fit || !node) return false;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      if (pendingSnapshotRef.current === null) return false;
      if (pendingLiveOverflowRef.current) {
        if (!snapshotRefreshPendingRef.current) {
          snapshotRefreshPendingRef.current = true;
          const refreshSessionId = sessionIdRef.current;
          let refreshed = false;
          void api.session
            .outputSnapshot(refreshSessionId)
            .then((snapshot) => {
              if (cancelled || sessionIdRef.current !== refreshSessionId) {
                return;
              }
              const maxSnapshotSeq = snapshot.reduce(
                (max, ev) => Math.max(max, ev.seq),
                0,
              );
              pendingSnapshotRef.current = snapshot;
              pendingLiveRef.current = pendingLiveRef.current.filter(
                (ev) => ev.seq > maxSnapshotSeq,
              );
              pendingLiveOverflowRef.current = false;
              refreshed = true;
            })
            .catch((e) => {
              if (!cancelled) onErrorRef.current?.(String(e));
            })
            .finally(() => {
              if (sessionIdRef.current === refreshSessionId) {
                snapshotRefreshPendingRef.current = false;
                if (refreshed) tryDrainReplayRef.current?.();
              }
            });
        }
        return false;
      }

      try {
        fit.fit();
      } catch {
        // teardown in progress
        return false;
      }

      t.reset();
      const queued: OutputEvent[] = [];
      for (const ev of pendingSnapshotRef.current) {
        queued.push(ev);
        lastWrittenSeqRef.current = Math.max(
          lastWrittenSeqRef.current,
          ev.seq,
        );
      }
      pendingSnapshotRef.current = null;
      for (const ev of pendingLiveRef.current) {
        if (ev.seq <= lastWrittenSeqRef.current) continue;
        queued.push(ev);
        lastWrittenSeqRef.current = ev.seq;
      }
      pendingLiveRef.current = [];
      replayDoneRef.current = true;

      if (queued.length === 0) {
        return true;
      }

      replayFlushPendingRef.current = true;
      const onReplayFlushed = () => {
        replayFlushPendingRef.current = false;
        replayJustDrainedRef.current = true;
        const callbacks = replayAfterFlushRef.current.splice(0);
        for (const cb of callbacks) cb();
      };

      queued.forEach((ev, index) => {
        const isLast = index === queued.length - 1;
        termRef.current?.write(
          decodeBase64Chunk(ev.data),
          isLast ? onReplayFlushed : undefined,
        );
      });
      return true;
    };
    tryDrainReplayRef.current = tryDrainReplay;

    void (async () => {
      const [fnOut, fnExit] = await Promise.all([
        listen<OutputEvent>("session/output", (event) => {
          if (event.payload.session_id !== sessionId) return;
          if (!replayDoneRef.current) {
            pendingLiveRef.current.push(event.payload);
            if (pendingLiveRef.current.length > MAX_PENDING_LIVE_EVENTS) {
              pendingLiveRef.current.splice(
                0,
                pendingLiveRef.current.length - MAX_PENDING_LIVE_EVENTS,
              );
              pendingLiveOverflowRef.current = true;
            }
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
      replayAfterFlushRef.current = [];
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [sessionId, refreshActiveTerminal]);

  // Latch WHY this terminal is inactive. `active=false` with
  // `disabled=true` is the transitional resume/start window — the canvas
  // sits under an opacity-0 loader but xterm keeps painting, so the
  // coming activation must NOT run the wake dance: double-SIGWINCHing a
  // codex pane that already holds content makes its repaint push a
  // duplicated, reflow-garbled frame into scrollback (restart → resume
  // panes one by one; Resume all was immune only because siblings were
  // still disabled when each settled). `disabled=false` means the pane
  // was genuinely hidden (display:none tab) — that return path keeps the
  // dance, which exists to wake a stale canvas (#108, impl 0011).
  const wasTransitionalRef = useRef(false);
  useEffect(() => {
    if (!active) wasTransitionalRef.current = disabledRef.current;
  }, [active]);

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
      const dance = !wasTransitionalRef.current;
      console.info(
        `[terminal] activate session=${sessionIdRef.current} dance=${dance}`,
      );
      refreshActiveTerminal({
        focus: autoFocusRef.current,
        forceResizeDance: dance,
        pushBackendSize: !dance,
      });
    };

    raf1 = window.requestAnimationFrame(() => {
      raf2 = window.requestAnimationFrame(activate);
    });

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(raf1);
      window.cancelAnimationFrame(raf2);
    };
  }, [active, sessionId, refreshActiveTerminal]);

  useImperativeHandle(
    ref,
    () => ({
      focus: () => {
        if (!disabledRef.current) termRef.current?.focus();
      },
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
          const beforeCols = t.cols;
          const beforeRows = t.rows;
          fit.fit();
          if (t.cols !== beforeCols || t.rows !== beforeRows) {
            console.info(
              `[terminal] measure-fit session=${sessionIdRef.current} ` +
                `${beforeCols}x${beforeRows} -> ${t.cols}x${t.rows} ` +
                `lastPushed=${lastPushedColsRef.current}x${lastPushedRowsRef.current} ` +
                `disabled=${disabledRef.current} active=${activeRef.current}`,
            );
          }
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
