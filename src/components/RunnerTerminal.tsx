// Embedded xterm.js bound to a single live session.
//
// Direct chat and the mission workspace mount one of these per session and
// keep them alive while switching between tabs/routes. Output keeps streaming
// into hidden instances so each PTY's scrollback survives UI switches.
//
// Setup: active-pane WebGL renderer for cell-row alignment, base64 PTY frames
// to preserve raw bytes, backend snapshot replay for late attach, and SIGWINCH
// dance on attach so claude-code/codex/qoder/trae repaint onto a fresh grid.

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { api, type PaneSurface } from "../lib/api";
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
  blankDanceDecision,
  createBlankRecheckGate,
} from "../lib/terminalBlank";
import {
  activationResizeRequest,
  shouldDelayTerminalResize,
  shouldPushTerminalSize,
  terminalSizeAfterDisabledChange,
  terminalSizeAfterRejectedPush,
  type TerminalGridSize,
} from "../lib/terminalResize";
import { replayOutputAtWidths } from "../lib/terminalReplay";
import { TERMINAL_SCROLLBAR_WIDTH_PX } from "../lib/terminalSizing";
import { observeBackingScale, stalesTextureAtlas } from "../lib/textureAtlas";
import { eventMatchesShortcut } from "../lib/keymap";
import { handleTerminalPaste } from "../lib/terminalPaste";
import { runtimeClearsOnResize } from "./ui/runtimes";

interface OutputEvent {
  session_id: string;
  mission_id: string | null;
  seq: number;
  width?: number;
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

interface RunnerTerminalProps {
  sessionId: string;
  /** Runtime kind of the runner driving this session (e.g.
   *  `"claude-code"`, `"codex"`, `"qoder"`, `"trae"`, `"shell"`). Used to gate the
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
  /** True while this terminal's pane is visible. A visible terminal may
   *  still be disabled, e.g. a stopped mission slot that should replay
   *  dimmed scrollback without accepting input. */
  active?: boolean;
  /** True when the containing pane currently uses `display:none`. This
   *  distinguishes a stale internal tab canvas from a keep-alive surface
   *  hidden with `visibility:hidden`, whose xterm buffer stays live. */
  hiddenByDisplayNone: boolean;
  /** Whether activation may steal keyboard focus. Defaults to true (the
   *  single-visible-terminal surfaces want focus to follow activation).
   *  Split chat (impl 0020) shows several active terminals at once and
   *  passes false for every pane except the focused one, so mounting a
   *  sibling pane can't yank keystrokes away from the focused chat. */
  autoFocus?: boolean;
  /** Stop forwarding keystrokes to the backend.
   *  Set by the parent when the bound session has exited so stray
   *  input on the dimmed pane doesn't surface a "session not found"
   *  error from the now-empty live map. */
  disabled?: boolean;
  /** Suppress geometry pushes while a spawn/resume is transitional.
   *  Stopped panes keep this false so their measured size is persisted
   *  for the next resume even though keyboard input stays disabled. */
  resizeDisabled?: boolean;
  /** Pane box this terminal sits in. Every grid pushed to the backend is
   *  also cached under this surface so a spawn the backend initiates
   *  (MCP `mission_start` / `session_start_direct`) can fork at a width
   *  a real pane had. Mission and chat boxes differ, so they never share
   *  a cache entry. See docs/impls/0039. */
  paneSurface: PaneSurface;
}

/**
 * Imperative handle exposed to the parent so resume/spawn callers can
 * size the backend PTY to the actual xterm geometry before the child
 * is forked. When this is unavailable, SessionManager falls back to
 * the persisted last-applied size and only uses 80×24 for a session
 * that has never been sized (#resume-pty-size-mismatch).
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
  {
    sessionId,
    runnerRuntime,
    onExit,
    onError,
    active,
    hiddenByDisplayNone,
    autoFocus,
    disabled,
    resizeDisabled,
    paneSurface,
  },
  ref,
) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // Live active-pane WebglAddon handle so the font-change listeners below
  // can call `clearTextureAtlas()` on it. Hidden terminals retain their xterm
  // buffer but release their GPU context, keeping the long-lived chat pane
  // pool below WKWebView's context limit.
  const webglRef = useRef<WebglAddon | null>(null);
  const sessionIdRef = useRef<string>(sessionId);
  const onExitRef = useRef(onExit);
  const onErrorRef = useRef(onError);
  const activeRef = useRef(active ?? false);
  // Mirrors the `runnerRuntime` prop into a ref so the resize handler
  // — declared inside the long-lived mount effect — sees the current
  // runtime kind without a re-render restarting the whole xterm.
  const runnerRuntimeRef = useRef<string>(runnerRuntime);
  // Mirrors `paneSurface` so the resize funnel — declared inside the
  // long-lived mount effect — reads it without re-running the effect.
  const paneSurfaceRef = useRef<PaneSurface>(paneSurface);
  // Mirrors the `disabled` prop into a ref so the onData/resize
  // closures don't capture a stale value across the long-lived
  // terminal effect.
  const disabledRef = useRef<boolean>(disabled ?? false);
  const resizeDisabledPropRef = useRef<boolean>(resizeDisabled ?? false);
  const resizeDisabledRef = useRef<boolean>(resizeDisabled ?? false);
  const replayResizeSuppressionCountRef = useRef(0);
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
  const reassertSizeRef = useRef<(() => void) | null>(null);
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
  // Activation leaves its refresh here until it succeeds. The container
  // ResizeObserver retries it when a display:none pane becomes measurable.
  const activationRefreshRef = useRef<(() => boolean) | null>(null);
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
  // Live writes still queued in xterm's async write buffer. The
  // blank-grid dance check (#312) reads the parsed buffer
  // synchronously, so bytes in this window are invisible to it — a
  // pane whose repaint is queued would misread as blank and dance,
  // defeating the transitional latch. While writes are pending,
  // blank-driven dances defer; the gate coalesces however many defer
  // observations into ONE recheck when the queue drains (see
  // createBlankRecheckGate). The gate tracks the terminal, not the
  // session: writes from a previous session flush into the same xterm
  // instance, so the write count deliberately survives session swaps —
  // only the recheck request is cancelled there.
  const [blankGate] = useState(createBlankRecheckGate);

  // Keep the latest sessionId visible to the data/resize callbacks without
  // re-creating the terminal on prop change. The session listener below
  // re-binds when sessionId changes — and so does the SIGWINCH attach
  // dance that wakes claude-code into repainting.
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    resizeDisabledPropRef.current = resizeDisabled ?? false;
    resizeDisabledRef.current =
      resizeDisabledPropRef.current ||
      replayResizeSuppressionCountRef.current > 0;
  }, [resizeDisabled]);

  const setReplayResizeSuppressed = useCallback((suppressed: boolean) => {
    replayResizeSuppressionCountRef.current = Math.max(
      0,
      replayResizeSuppressionCountRef.current + (suppressed ? 1 : -1),
    );
    resizeDisabledRef.current =
      resizeDisabledPropRef.current ||
      replayResizeSuppressionCountRef.current > 0;
  }, []);

  useEffect(() => {
    const nextDisabled = disabled ?? false;
    const wentLive = disabledRef.current && !nextDisabled;
    const lastPushed = terminalSizeAfterDisabledChange(
      {
        cols: lastPushedColsRef.current,
        rows: lastPushedRowsRef.current,
      },
      disabledRef.current,
      nextDisabled,
    );
    disabledRef.current = nextDisabled;
    lastPushedColsRef.current = lastPushed.cols;
    lastPushedRowsRef.current = lastPushed.rows;
    if (wentLive) reassertSizeRef.current?.();
  }, [disabled]);

  useEffect(() => {
    runnerRuntimeRef.current = runnerRuntime;
  }, [runnerRuntime]);

  useEffect(() => {
    paneSurfaceRef.current = paneSurface;
  }, [paneSurface]);

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

  // Attach the WebGL renderer only from refreshActiveTerminal, i.e. only
  // once the pane is active AND its container has a measurable rect. The
  // addon reads canvas geometry at load; attaching it on a bare `active`
  // flip could initialize against a hidden / not-yet-settled layout, and
  // when the subsequent fit resolves the SAME cols/rows, xterm never
  // fires the resize that would correct the renderer — the pane paints
  // stale until a real grid change (the "wrong until I resize the
  // window" tab-return artifact). Deferring creation to the measurable
  // path closes that window.
  const ensureWebglRenderer = useCallback(() => {
    const term = termRef.current;
    if (!term || webglRef.current) return;
    let webgl: WebglAddon | null = null;
    try {
      webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl?.dispose();
        if (webglRef.current === webgl) webglRef.current = null;
      });
      term.loadAddon(webgl);
      webglRef.current = webgl;
    } catch {
      webgl?.dispose();
      // No WebGL — xterm keeps its DOM renderer.
    }
  }, []);

  const rejectSizePush = useCallback((cols: number, rows: number) => {
    const lastPushed = terminalSizeAfterRejectedPush(
      {
        cols: lastPushedColsRef.current,
        rows: lastPushedRowsRef.current,
      },
      { cols, rows },
    );
    lastPushedColsRef.current = lastPushed.cols;
    lastPushedRowsRef.current = lastPushed.rows;
  }, []);

  // Cache every grid that actually reaches the backend, from whichever
  // push path sent it. Not folded into `sendBackendResize`: mission slot
  // terminals mount under `display:none`, so the mount-time push is
  // skipped and their first real grid arrives through
  // `refreshActiveTerminal`'s own resize calls — which set `lastPushed*`
  // themselves, making the next `pushSize` dedupe away. Routing only the
  // one funnel would leave the cache empty for exactly the panes #367 is
  // about. Best-effort: a failed write just leaves the previous grid.
  const cachePaneGrid = useCallback((cols: number, rows: number) => {
    void api.session
      .recordPaneGrid(paneSurfaceRef.current, cols, rows)
      .catch(() => {});
  }, []);
  // Mirrored for `sendBackendResize`, which lives inside the long-lived
  // mount effect and must not take a dependency that re-runs it.
  const cachePaneGridRef = useRef(cachePaneGrid);
  cachePaneGridRef.current = cachePaneGrid;

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
      const deferUntilReplayFlushed = () => {
        if (!replayFlushPendingRef.current) return false;
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
      };
      if (deferUntilReplayFlushed()) return true;
      try {
        const beforeCols = t.cols;
        const beforeRows = t.rows;
        ensureWebglRenderer();
        tryDrainReplayRef.current?.();
        if (deferUntilReplayFlushed()) return true;
        fit.fit();
        if (t.cols !== beforeCols || t.rows !== beforeRows) {
          console.info(
            `[terminal] refresh-fit session=${sessionIdRef.current} ` +
              `${beforeCols}x${beforeRows} -> ${t.cols}x${t.rows} ` +
              `disabled=${disabledRef.current} forceDance=${forceResizeDance} ` +
              `pushBackend=${pushBackendSize}`,
          );
        }
        t.refresh(0, t.rows - 1);
        if (focus && !disabledRef.current) t.focus();
        if (
          (!forceResizeDance && !pushBackendSize) ||
          resizeDisabledRef.current
        ) {
          if ((forceResizeDance || pushBackendSize) && resizeDisabledRef.current) {
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
        if (disabledRef.current) {
          if (
            cols === lastPushedColsRef.current &&
            rows === lastPushedRowsRef.current
          ) {
            return true;
          }
          console.info(
            `[terminal] refresh-push-stopped session=${sid} cols=${cols} rows=${rows} ` +
              `prev=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
          );
          lastPushedColsRef.current = cols;
          lastPushedRowsRef.current = rows;
          cachePaneGrid(cols, rows);
          void api.session.resize(sid, cols, rows).catch(() => {
            rejectSizePush(cols, rows);
          });
          return true;
        }
        // Blank grid must dance (#312). A running session behind an
        // empty grid has nothing to lose to a forced repaint and no
        // other way to gain content: the plain push below dedupes
        // against lastPushed, and even a sent same-size TIOCSWINSZ is
        // a kernel no-op — so without the dance nothing ever asks the
        // agent to paint. The transitional latch exists to protect
        // retained content from a double repaint; a blank grid has
        // none, so the latch's guarantee is untouched for non-blank
        // panes (evaluated fresh on every retry — content that
        // arrived in between keeps the plain-push path). A blank read
        // taken while live bytes sit unparsed in xterm's async write
        // queue is stale — the queued bytes may BE the repaint — so
        // that case defers: plain push now, re-check after the queue
        // flushes.
        let mustDance = forceResizeDance;
        if (!mustDance) {
          const decision = blankDanceDecision(t, blankGate.pendingWrites());
          if (decision === "dance") {
            mustDance = true;
            console.info(
              `[terminal] blank-dance session=${sid} cols=${cols} rows=${rows} ` +
                `lastPushed=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
            );
          } else if (decision === "defer") {
            console.info(
              `[terminal] blank-dance-defer session=${sid} ` +
                `pendingWrites=${blankGate.pendingWrites()}`,
            );
            blankGate.requestRecheck();
          }
        }
        if (!mustDance) {
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
          cachePaneGrid(cols, rows);
          void api.session.resize(sid, cols, rows).catch(() => {
            rejectSizePush(cols, rows);
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
        // The settled grid, not the nudge — `nudgedRows` is a one-ioctl
        // SIGWINCH trick, never a size any pane is left at.
        cachePaneGrid(cols, rows);
        const nudgedRows = rows > 1 ? rows - 1 : rows + 1;
        void api.session
          .resize(sid, cols, nudgedRows)
          .then(() => api.session.resize(sid, cols, rows))
          .catch(() => {
            rejectSizePush(cols, rows);
          });
        return true;
      } catch {
        // Layout not ready yet — the next activation / resize will drive it.
        return false;
      }
    },
    [ensureWebglRenderer, rejectSizePush, blankGate, cachePaneGrid],
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
      scrollbar: { width: TERMINAL_SCROLLBAR_WIDTH_PX },
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
    function sendBackendResize(current: TerminalGridSize) {
      const sid = sessionIdRef.current;
      if (!sid) return;
      console.info(
        `[terminal] push-size session=${sid} cols=${current.cols} rows=${current.rows} ` +
          `prev=${lastPushedColsRef.current}x${lastPushedRowsRef.current}`,
      );
      lastPushedColsRef.current = current.cols;
      lastPushedRowsRef.current = current.rows;
      void api.session.resize(sid, current.cols, current.rows).catch(() => {
        rejectSizePush(current.cols, current.rows);
      });
      // Same value, same moment — no separate measuring pass.
      cachePaneGridRef.current(current.cols, current.rows);
    }
    function pushBackendResize() {
      if (resizeDisabledRef.current) return false;
      const current = { cols: term.cols, rows: term.rows };
      if (
        !shouldPushTerminalSize(current, {
          cols: lastPushedColsRef.current,
          rows: lastPushedRowsRef.current,
        })
      ) {
        return false;
      }
      sendBackendResize(current);
      return true;
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
      // sessionIdRef is initialized with the prop value (line ~124),
      // so this reads the right id on initial mount without forcing
      // sessionId into the mount-effect's deps (which is intentionally
      // `[]` to avoid tearing down the whole xterm on session swap).
      pushBackendResize();
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
    //
    // Copying a *file* takes the same interception point but the other
    // branch (feature 55): the clipboard carries no text at all, only
    // `public.file-url`, so xterm's default paste inserts nothing. We
    // ask NSPasteboard for the paths and inject them instead, which is
    // what Terminal.app / iTerm2 / Ghostty do.
    const onPaste = (e: ClipboardEvent) => {
      const sid = sessionIdRef.current;
      if (!sid || disabledRef.current) return;
      void handleTerminalPaste(e, {
        clipboardFilePaths: () => api.session.clipboardFilePaths(),
        injectStdin: (text) => api.session.injectStdin(sid, text),
        pasteImage: (bytes, mimeType) => api.session.pasteImage(bytes, mimeType),
        onError: (message) => onErrorRef.current?.(message),
      });
    };
    const textarea = term.textarea;
    textarea?.addEventListener("paste", onPaste, { capture: true });

    // Dedupe by last-pushed dims. See `lastPushedColsRef` comment for why.
    const pushSize = () => {
      const t = termRef.current;
      const sid = sessionIdRef.current;
      if (
        !t ||
        !sid ||
        resizeDisabledRef.current
      ) {
        return;
      }
      const current = { cols: t.cols, rows: t.rows };
      if (
        !shouldPushTerminalSize(
          current,
          {
            cols: lastPushedColsRef.current,
            rows: lastPushedRowsRef.current,
          },
        )
      ) {
        return;
      }
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
      const skipLocalClear =
        replayJustDrainedRef.current || disabledRef.current;
      if (runtimeClearsOnResize(runnerRuntimeRef.current) && !skipLocalClear) {
        // ESC[2J — erase visible region
        // ESC[H  — cursor home
        t.write("\x1b[2J\x1b[H");
      }
      if (!disabledRef.current) replayJustDrainedRef.current = false;
      sendBackendResize(current);
    };
    reassertSizeRef.current = () => {
      if (replayFlushPendingRef.current) {
        replayAfterFlushRef.current.push(() => {
          reassertSizeRef.current?.();
        });
        return;
      }
      const node = containerRef.current;
      if (!node) return;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      try {
        fit.fit();
        pushBackendResize();
      } catch {
        // layout not ready; the next measurable refit will push
      }
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
    // Refit + push backend geometry whenever the pane is measurable.
    // PersistentSurfaces keeps inactive surfaces laid out under
    // visibility:hidden, so they stay sized while rendering, focus,
    // WebGL, and wake handling remain gated by `active`.
    function refitAndPush({
      allowPendingLargeDrop = false,
    }: { allowPendingLargeDrop?: boolean } = {}) {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      if (replayFlushPendingRef.current) {
        replayAfterFlushRef.current.push(() => {
          refitAndPush({ allowPendingLargeDrop });
        });
        return;
      }
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
    // CSS-driven size changes. The measurable-rect guard still excludes
    // internal display:none panes while invisible persistent surfaces
    // keep their PTY geometry current.
    const ro = new ResizeObserver(() => {
      const activationRefresh = activationRefreshRef.current;
      if (activationRefresh?.()) {
        if (activationRefreshRef.current === activationRefresh) {
          activationRefreshRef.current = null;
        }
      }
      refitAndPush();
    });
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
    // Real system wake, from the NSWorkspace observer in wake.rs (#360).
    // Sleep can re-initialize the display or reset the GPU underneath a
    // live atlas, leaving every cached glyph rasterized for conditions
    // that no longer hold; `t.refresh()` on the focus path only marks
    // lines dirty, so the stale rasters get redrawn faithfully. Dropping
    // the cache is the entire remedy — the addon re-rasterizes lazily on
    // the redraw it requests itself, and geometry is #352/#363's problem,
    // not this one. Deliberately not hung on focus or visibilitychange:
    // those correlate with wake but fire on every alt-tab.
    let unlistenAppWoke: (() => void) | null = null;
    let appWokeCancelled = false;
    void listen("app/woke", () => {
      webglRef.current?.clearTextureAtlas();
    }).then((fn) => {
      if (appWokeCancelled) {
        fn();
        return;
      }
      unlistenAppWoke = fn;
    });
    // Lid opened on a different monitor, or a display that came back at
    // another scale. Independent of wake in both directions: a monitor
    // move fires no wake, and a GPU reset fires no resolution change.
    const disposeBackingScale = observeBackingScale(() => {
      webglRef.current?.clearTextureAtlas();
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
        } else if (e.key === STORAGE_TERMINAL_FONT_FAMILY) {
          t.options.fontFamily = resolveTerminalFontStack(
            readTerminalFontFamily(),
          );
        } else if (e.key === STORAGE_TERMINAL_CURSOR_STYLE) {
          t.options.cursorStyle = readTerminalCursorStyle();
        } else if (e.key === STORAGE_TERMINAL_SCROLLBACK) {
          t.options.scrollback = readTerminalScrollback();
        } else if (e.key === STORAGE_TERMINAL_THEME) {
          t.options.theme = resolveTerminalTheme(readTerminalTheme());
        }
        // Cell metrics changed — refit, push the new PTY geometry, and
        // drop the atlas. The atlas indexes cells by their rendered
        // pixel dimensions; a stale cache after a font change can leave
        // a band of pre-change glyphs at the new size until something
        // else evicts them. App zoom rides along on weaker evidence —
        // see `stalesTextureAtlas` for why it is kept despite not
        // reproducing the symptom.
        if (stalesTextureAtlas(e.key)) {
          webglRef.current?.clearTextureAtlas();
          refitAndPush();
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
      appWokeCancelled = true;
      unlistenAppWoke?.();
      disposeBackingScale();
      focusCancelled = true;
      unlistenFocus?.();
      wakeRafs.forEach((id) => window.cancelAnimationFrame(id));
      wakeTimers.forEach((id) => window.clearTimeout(id));
      clearStableResizeSchedule();
      textarea?.removeEventListener("paste", onPaste, { capture: true });
      onDataDisposable.dispose();
      webglRef.current = null;
      reassertSizeRef.current = null;
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [refreshActiveTerminal, rejectSizePush]);

  // A mounted hidden terminal still owns its CPU-side xterm buffer, but it
  // must not hold a WebGL context: RunnerChat deliberately keeps every
  // attached session mounted, and WKWebView starts evicting old contexts at
  // roughly 16. Active chat splits and mission tabs need at most three.
  //
  // Creation lives in `ensureWebglRenderer` (invoked by
  // refreshActiveTerminal once the pane is measurable); this effect only
  // releases the context when the pane leaves the foreground. A genuine
  // GPU context loss disposes the addon immediately so xterm falls back
  // to its DOM renderer; the next activation/wake refresh restores WebGL
  // instead of degrading permanently.
  useEffect(() => {
    if (!active) return;
    return () => {
      webglRef.current?.dispose();
      webglRef.current = null;
    };
  }, [active]);

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
    // The gate's write count is NOT reset: queued writes from the
    // previous session still flush into this same xterm instance and
    // decrement it. Only the deferred recheck request is stale.
    blankGate.cancelRecheck();
    pendingLiveOverflowRef.current = false;
    snapshotRefreshPendingRef.current = false;
    replayJustDrainedRef.current = false;

    const writeOutput = (ev: OutputEvent) => {
      const t = termRef.current;
      if (!t) return;
      // Counted so the blank-dance check can tell "settled blank grid"
      // from "repaint still queued in xterm's async parser" — see
      // blankGate. The write callback fires after the chunk is parsed
      // into the buffer; when the queue drains, at most one deferred
      // recheck runs against the settled content. Fixed opts: defer
      // only ever replaces a plain backend push (a forced dance never
      // defers), and the original pass already applied its focus.
      blankGate.beginWrite();
      t.write(decodeBase64Chunk(ev.data), () => {
        if (!blankGate.endWrite()) return;
        window.requestAnimationFrame(() => {
          refreshActiveTerminal({ pushBackendSize: true });
        });
      });
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

      const fallbackWidth = t.cols;
      try {
        fit.fit();
      } catch {
        // teardown in progress
        return false;
      }
      const target = { cols: t.cols, rows: t.rows };

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

      if (queued.length === 0) {
        replayDoneRef.current = true;
        return true;
      }

      replayFlushPendingRef.current = true;
      const onReplayFlushed = () => {
        replayFlushPendingRef.current = false;
        replayJustDrainedRef.current = true;
        const callbacks = replayAfterFlushRef.current.splice(0);
        for (const cb of callbacks) cb();
      };

      const takePendingChunks = () => {
        if (pendingLiveOverflowRef.current) return [];
        const pending = pendingLiveRef.current;
        pendingLiveRef.current = [];
        const next: Array<{ data: Uint8Array; width?: number }> = [];
        for (const ev of pending) {
          if (ev.seq <= lastWrittenSeqRef.current) continue;
          lastWrittenSeqRef.current = ev.seq;
          next.push({
            data: decodeBase64Chunk(ev.data),
            width: ev.width,
          });
        }
        return next;
      };

      void replayOutputAtWidths({
        terminal: t,
        initialChunks: queued.map((ev) => ({
          data: decodeBase64Chunk(ev.data),
          width: ev.width,
        })),
        takePendingChunks,
        fallbackWidth,
        target,
        setResizeSuppressed: setReplayResizeSuppressed,
        shouldContinue: () =>
          !cancelled &&
          termRef.current === t &&
          sessionIdRef.current === sessionId,
      })
        .then((completed) => {
          if (
            !completed ||
            cancelled ||
            termRef.current !== t ||
            sessionIdRef.current !== sessionId
          ) {
            return;
          }
          if (pendingLiveOverflowRef.current) {
            replayFlushPendingRef.current = false;
            pendingSnapshotRef.current = [];
            tryDrainReplayRef.current?.();
            return;
          }
          replayDoneRef.current = true;
          onReplayFlushed();
        })
        .catch((error) => {
          if (
            cancelled ||
            termRef.current !== t ||
            sessionIdRef.current !== sessionId
          ) {
            return;
          }
          replayDoneRef.current = true;
          onReplayFlushed();
          onErrorRef.current?.(String(error));
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
      blankGate.cancelRecheck();
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [
    sessionId,
    refreshActiveTerminal,
    blankGate,
    setReplayResizeSuppressed,
  ]);

  // Latch WHY this terminal is inactive. `active=false` with
  // `disabled=true` is the transitional resume/start window — the canvas
  // sits under an opacity-0 loader but xterm keeps painting, so the
  // coming activation must NOT run the wake dance: double-SIGWINCHing a
  // codex pane that already holds content makes its repaint push a
  // duplicated, reflow-garbled frame into scrollback (restart → resume
  // panes one by one; Resume all was immune only because siblings were
  // still disabled when each settled). An ordinary inactive pane may be
  // hidden either by PersistentSurfaces' visibility:hidden layer, where
  // xterm stays live, or by an internal display:none tab, where the canvas
  // can go stale. Only the latter keeps the #108 wake dance.
  //
  // A terminal that mounts already active also starts with this ref false:
  // its canvas is fresh, snapshot replay + t.refresh() paint retained
  // content, and blankDanceDecision remains the targeted wake path when
  // the new grid is empty.
  const wasTransitionalRef = useRef(false);
  const canvasWasDisplayNoneRef = useRef(false);
  useEffect(() => {
    if (!active) {
      wasTransitionalRef.current = disabledRef.current;
      canvasWasDisplayNoneRef.current = hiddenByDisplayNone;
    }
  }, [active, hiddenByDisplayNone]);

  // Activation effect: when this tab moves to the front, fit to its container,
  // repaint the freshly-created WebGL renderer with the current scrollback,
  // and grab focus so keystrokes flow into the expected PTY. If layout is not
  // measurable yet, the container ResizeObserver keeps retrying this refresh.
  useEffect(() => {
    if (!active) {
      activationRefreshRef.current = null;
      return;
    }
    const request = activationResizeRequest({
      canvasWasDisplayNone: canvasWasDisplayNoneRef.current,
      wasTransitional: wasTransitionalRef.current,
    });
    console.info(
      `[terminal] activate session=${sessionIdRef.current} ` +
        `dance=${request.forceResizeDance} ` +
        `displayNone=${canvasWasDisplayNoneRef.current} ` +
        `transitional=${wasTransitionalRef.current}`,
    );
    const refresh = () =>
      refreshActiveTerminal({
        focus: autoFocusRef.current,
        ...request,
      });
    activationRefreshRef.current = refresh;
    if (refresh()) activationRefreshRef.current = null;

    return () => {
      if (activationRefreshRef.current === refresh) {
        activationRefreshRef.current = null;
      }
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
          // returning null. A null resume size lets the backend use its
          // persisted last-applied dimensions before falling back to
          // 80×24. The constructor's 80×24 is the "never fit" sentinel,
          // so treat it as null rather than overriding a persisted size.
          if (t.cols === 80 && t.rows === 24) return null;
          return { cols: t.cols, rows: t.rows };
        }
        try {
          if (replayFlushPendingRef.current) {
            const proposed = fit.proposeDimensions();
            return proposed
              ? { cols: proposed.cols, rows: proposed.rows }
              : null;
          }
          // Force a fit before reading dims so a stopped pane reflects
          // its latest measurable container when the user clicks Resume.
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
    // Keep this attribute in sync with terminalSizing's
    // TERMINAL_HOST_SESSION_ATTR: it lets launch auto-resume find this host
    // and measure the box the terminal will fit to (impl 0036, rung 1)
    // without a surface-local terminal registry.
    <div
      data-terminal-session={sessionId}
      className="h-full w-full overflow-hidden"
    >
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
});
