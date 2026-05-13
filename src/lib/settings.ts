// Shared helpers for the localStorage-backed settings used by both
// the Settings modal and the surfaces that consume those settings
// (e.g. UpdateContext). All settings persist via the same `"1"` /
// `"0"` encoding the modal writes — keep this file the single source
// of truth so the modal and its consumers can't drift apart.

export const STORAGE_AUTO_INSTALL_UPDATES = "settings.autoInstallUpdates";
export const STORAGE_SIDEBAR_COLLAPSED = "runner.sidebar.collapsed";
export const STORAGE_APP_ZOOM = "settings.appZoom";
export const STORAGE_TERMINAL_FONT_SIZE = "settings.terminalFontSize";
export const STORAGE_TERMINAL_FONT_FAMILY = "settings.terminalFontFamily";
export const STORAGE_TERMINAL_CURSOR_STYLE = "settings.terminalCursorStyle";
export const STORAGE_TERMINAL_SCROLLBACK = "settings.terminalScrollback";

// Public domain for the App-zoom and Terminal-* controls. Kept here (not
// in SettingsModal) so the readers can snap/clamp to the same domain the
// UI presents — boot and storage-event consumers can't drift onto off-
// step or out-of-range values that the modal would never offer.
export const ZOOM_STEPS: readonly number[] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];
export const TERMINAL_FONT_SIZE_MIN = 10;
export const TERMINAL_FONT_SIZE_MAX = 20;

export type TerminalFontFamily =
  | "System default"
  | "Menlo"
  | "Monaco"
  | "SF Mono"
  | "JetBrains Mono"
  | "Fira Code";
export const TERMINAL_FONT_FAMILY_OPTIONS: readonly TerminalFontFamily[] = [
  "System default",
  "Menlo",
  "Monaco",
  "SF Mono",
  "JetBrains Mono",
  "Fira Code",
];

export type TerminalCursorStyle = "block" | "underline" | "bar";
export const TERMINAL_CURSOR_STYLE_OPTIONS: readonly TerminalCursorStyle[] = [
  "block",
  "underline",
  "bar",
];

export const TERMINAL_SCROLLBACK_OPTIONS: readonly number[] = [
  1000, 5000, 10000, 50000,
];

const DEFAULT_APP_ZOOM = 1.0;
const DEFAULT_TERMINAL_FONT_SIZE = 13;
const DEFAULT_TERMINAL_FONT_FAMILY: TerminalFontFamily = "System default";
const DEFAULT_TERMINAL_CURSOR_STYLE: TerminalCursorStyle = "block";
const DEFAULT_TERMINAL_SCROLLBACK = 10000;

// The system default stack RunnerTerminal used to ship as a literal. Kept
// as the fallback chain so that even when a user picks a specific face
// (JetBrains Mono, Fira Code) we still degrade gracefully on machines
// where that font isn't installed.
const SYSTEM_FONT_STACK =
  'Menlo, "SF Mono", Monaco, Consolas, "Liberation Mono", monospace';

export function readStoredBool(key: string, defaultValue: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (raw == null) return defaultValue;
    return raw === "1";
  } catch {
    return defaultValue;
  }
}

export function writeStoredBool(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    // best-effort — Safari private mode rejects setItem; in-session
    // state still works, persistence is what's lost.
  }
}

export function readAppZoom(): number {
  try {
    const raw = localStorage.getItem(STORAGE_APP_ZOOM);
    if (raw == null) return DEFAULT_APP_ZOOM;
    const parsed = Number.parseFloat(raw);
    if (!Number.isFinite(parsed) || parsed <= 0) return DEFAULT_APP_ZOOM;
    // Snap to the nearest known step so off-step persisted values (older
    // builds, hand-edited localStorage) still resolve to a value the UI
    // can move from. No write-back — silent normalization on read only.
    let nearest = ZOOM_STEPS[0];
    let best = Math.abs(nearest - parsed);
    for (let i = 1; i < ZOOM_STEPS.length; i += 1) {
      const d = Math.abs(ZOOM_STEPS[i] - parsed);
      if (d < best) {
        best = d;
        nearest = ZOOM_STEPS[i];
      }
    }
    return nearest;
  } catch {
    return DEFAULT_APP_ZOOM;
  }
}

export function writeAppZoom(value: number): void {
  try {
    localStorage.setItem(STORAGE_APP_ZOOM, String(value));
  } catch {
    // best-effort — see writeStoredBool.
  }
}

export function readTerminalFontSize(): number {
  try {
    const raw = localStorage.getItem(STORAGE_TERMINAL_FONT_SIZE);
    if (raw == null) return DEFAULT_TERMINAL_FONT_SIZE;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed) || parsed <= 0) return DEFAULT_TERMINAL_FONT_SIZE;
    if (parsed < TERMINAL_FONT_SIZE_MIN) return TERMINAL_FONT_SIZE_MIN;
    if (parsed > TERMINAL_FONT_SIZE_MAX) return TERMINAL_FONT_SIZE_MAX;
    return parsed;
  } catch {
    return DEFAULT_TERMINAL_FONT_SIZE;
  }
}

export function writeTerminalFontSize(value: number): void {
  try {
    localStorage.setItem(STORAGE_TERMINAL_FONT_SIZE, String(value));
  } catch {
    // best-effort — see writeStoredBool.
  }
}

export function readTerminalFontFamily(): TerminalFontFamily {
  try {
    const raw = localStorage.getItem(STORAGE_TERMINAL_FONT_FAMILY);
    if (raw == null) return DEFAULT_TERMINAL_FONT_FAMILY;
    return (TERMINAL_FONT_FAMILY_OPTIONS as readonly string[]).includes(raw)
      ? (raw as TerminalFontFamily)
      : DEFAULT_TERMINAL_FONT_FAMILY;
  } catch {
    return DEFAULT_TERMINAL_FONT_FAMILY;
  }
}

export function writeTerminalFontFamily(value: TerminalFontFamily): void {
  try {
    localStorage.setItem(STORAGE_TERMINAL_FONT_FAMILY, value);
  } catch {
    // best-effort
  }
}

export function readTerminalCursorStyle(): TerminalCursorStyle {
  try {
    const raw = localStorage.getItem(STORAGE_TERMINAL_CURSOR_STYLE);
    if (raw == null) return DEFAULT_TERMINAL_CURSOR_STYLE;
    return (TERMINAL_CURSOR_STYLE_OPTIONS as readonly string[]).includes(raw)
      ? (raw as TerminalCursorStyle)
      : DEFAULT_TERMINAL_CURSOR_STYLE;
  } catch {
    return DEFAULT_TERMINAL_CURSOR_STYLE;
  }
}

export function writeTerminalCursorStyle(value: TerminalCursorStyle): void {
  try {
    localStorage.setItem(STORAGE_TERMINAL_CURSOR_STYLE, value);
  } catch {
    // best-effort
  }
}

export function readTerminalScrollback(): number {
  try {
    const raw = localStorage.getItem(STORAGE_TERMINAL_SCROLLBACK);
    if (raw == null) return DEFAULT_TERMINAL_SCROLLBACK;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) return DEFAULT_TERMINAL_SCROLLBACK;
    return TERMINAL_SCROLLBACK_OPTIONS.includes(parsed)
      ? parsed
      : DEFAULT_TERMINAL_SCROLLBACK;
  } catch {
    return DEFAULT_TERMINAL_SCROLLBACK;
  }
}

export function writeTerminalScrollback(value: number): void {
  try {
    localStorage.setItem(STORAGE_TERMINAL_SCROLLBACK, String(value));
  } catch {
    // best-effort
  }
}

// localStorage's `storage` event only fires in *other* windows by spec.
// Runner is single-window, so we synthesize one ourselves after writing
// so same-window consumers (RunnerTerminal, GeneralPane) can react via
// a single mechanism. Wrap the constructor in try/catch — older runtimes
// without `StorageEvent`'s ctor still get apply-on-next-mount.
export function notifySameWindowStorage(
  key: string,
  value: string | null,
): void {
  try {
    window.dispatchEvent(new StorageEvent("storage", { key, newValue: value }));
  } catch {
    // best-effort
  }
}

export function resolveTerminalFontStack(label: TerminalFontFamily): string {
  return label === "System default"
    ? SYSTEM_FONT_STACK
    : `'${label}', ${SYSTEM_FONT_STACK}`;
}
