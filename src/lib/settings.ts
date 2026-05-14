// Shared helpers for the localStorage-backed settings used by both
// the Settings modal and the surfaces that consume those settings
// (e.g. UpdateContext). All settings persist via the same `"1"` /
// `"0"` encoding the modal writes — keep this file the single source
// of truth so the modal and its consumers can't drift apart.

import type { ITheme } from "@xterm/xterm";

export const STORAGE_AUTO_INSTALL_UPDATES = "settings.autoInstallUpdates";
export const STORAGE_SIDEBAR_COLLAPSED = "runner.sidebar.collapsed";
export const STORAGE_APP_ZOOM = "settings.appZoom";
export const STORAGE_TERMINAL_FONT_SIZE = "settings.terminalFontSize";
export const STORAGE_TERMINAL_FONT_FAMILY = "settings.terminalFontFamily";
export const STORAGE_TERMINAL_CURSOR_STYLE = "settings.terminalCursorStyle";
export const STORAGE_TERMINAL_SCROLLBACK = "settings.terminalScrollback";
export const STORAGE_TERMINAL_THEME = "settings.terminalTheme";

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

export type TerminalTheme =
  | "runner"
  | "dracula"
  | "tokyo-night"
  | "nord"
  | "gruvbox-dark"
  | "catppuccin-mocha";
export const TERMINAL_THEME_OPTIONS: readonly TerminalTheme[] = [
  "runner",
  "dracula",
  "tokyo-night",
  "nord",
  "gruvbox-dark",
  "catppuccin-mocha",
];
export const TERMINAL_THEME_LABELS: Record<TerminalTheme, string> = {
  runner: "Runner",
  dracula: "Dracula",
  "tokyo-night": "Tokyo Night",
  nord: "Nord",
  "gruvbox-dark": "Gruvbox Dark",
  "catppuccin-mocha": "Catppuccin Mocha",
};
export const DEFAULT_TERMINAL_THEME: TerminalTheme = "runner";

// Locked surfaces: every bundled theme overrides foreground + ANSI 16 +
// cursor + selectionBackground, but background and cursorAccent are
// pinned to the app chrome (#15161B) so the terminal stays seamless
// with the surrounding panel. Don't unlock these per-theme — a light
// background would flash white during a remount before the WebGL
// renderer catches up.
const TERMINAL_THEME_LOCKED_BG = "#15161B";

export const TERMINAL_THEMES: Record<TerminalTheme, ITheme> = {
  runner: {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#DCDCE0",
    cursor: "#00FF9C",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#272930",
    black: "#15161B",
    red: "#FF4D6D",
    green: "#00FF9C",
    yellow: "#FFB020",
    blue: "#39E5FF",
    magenta: "#C792EA",
    cyan: "#39E5FF",
    white: "#DCDCE0",
    brightBlack: "#5A5C66",
    brightRed: "#FF7B8E",
    brightGreen: "#5FFFB8",
    brightYellow: "#FFCB6B",
    brightBlue: "#82AAFF",
    brightMagenta: "#C792EA",
    brightCyan: "#89DDFF",
    brightWhite: "#FFFFFF",
  },
  dracula: {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#F8F8F2",
    cursor: "#F8F8F2",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#44475A",
    black: "#21222C",
    red: "#FF5555",
    green: "#50FA7B",
    yellow: "#F1FA8C",
    blue: "#BD93F9",
    magenta: "#FF79C6",
    cyan: "#8BE9FD",
    white: "#F8F8F2",
    brightBlack: "#6272A4",
    brightRed: "#FF6E6E",
    brightGreen: "#69FF94",
    brightYellow: "#FFFFA5",
    brightBlue: "#D6ACFF",
    brightMagenta: "#FF92DF",
    brightCyan: "#A4FFFF",
    brightWhite: "#FFFFFF",
  },
  "tokyo-night": {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#A9B1D6",
    cursor: "#C0CAF5",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#28344A",
    black: "#15161E",
    red: "#F7768E",
    green: "#9ECE6A",
    yellow: "#E0AF68",
    blue: "#7AA2F7",
    magenta: "#BB9AF7",
    cyan: "#7DCFFF",
    white: "#A9B1D6",
    brightBlack: "#414868",
    brightRed: "#F7768E",
    brightGreen: "#9ECE6A",
    brightYellow: "#E0AF68",
    brightBlue: "#7AA2F7",
    brightMagenta: "#BB9AF7",
    brightCyan: "#7DCFFF",
    brightWhite: "#C0CAF5",
  },
  nord: {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#D8DEE9",
    cursor: "#D8DEE9",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#4C566A",
    black: "#3B4252",
    red: "#BF616A",
    green: "#A3BE8C",
    yellow: "#EBCB8B",
    blue: "#81A1C1",
    magenta: "#B48EAD",
    cyan: "#88C0D0",
    white: "#E5E9F0",
    brightBlack: "#4C566A",
    brightRed: "#BF616A",
    brightGreen: "#A3BE8C",
    brightYellow: "#EBCB8B",
    brightBlue: "#81A1C1",
    brightMagenta: "#B48EAD",
    brightCyan: "#8FBCBB",
    brightWhite: "#ECEFF4",
  },
  "gruvbox-dark": {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#EBDBB2",
    cursor: "#EBDBB2",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#3C3836",
    black: "#282828",
    red: "#CC241D",
    green: "#98971A",
    yellow: "#D79921",
    blue: "#458588",
    magenta: "#B16286",
    cyan: "#689D6A",
    white: "#A89984",
    brightBlack: "#928374",
    brightRed: "#FB4934",
    brightGreen: "#B8BB26",
    brightYellow: "#FABD2F",
    brightBlue: "#83A598",
    brightMagenta: "#D3869B",
    brightCyan: "#8EC07C",
    brightWhite: "#EBDBB2",
  },
  "catppuccin-mocha": {
    background: TERMINAL_THEME_LOCKED_BG,
    foreground: "#CDD6F4",
    cursor: "#F5E0DC",
    cursorAccent: TERMINAL_THEME_LOCKED_BG,
    selectionBackground: "#585B70",
    black: "#45475A",
    red: "#F38BA8",
    green: "#A6E3A1",
    yellow: "#F9E2AF",
    blue: "#89B4FA",
    magenta: "#F5C2E7",
    cyan: "#94E2D5",
    white: "#BAC2DE",
    brightBlack: "#585B70",
    brightRed: "#F38BA8",
    brightGreen: "#A6E3A1",
    brightYellow: "#F9E2AF",
    brightBlue: "#89B4FA",
    brightMagenta: "#F5C2E7",
    brightCyan: "#94E2D5",
    brightWhite: "#A6ADC8",
  },
};

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

export function readTerminalTheme(): TerminalTheme {
  try {
    const raw = localStorage.getItem(STORAGE_TERMINAL_THEME);
    if (raw == null) return DEFAULT_TERMINAL_THEME;
    return (TERMINAL_THEME_OPTIONS as readonly string[]).includes(raw)
      ? (raw as TerminalTheme)
      : DEFAULT_TERMINAL_THEME;
  } catch {
    return DEFAULT_TERMINAL_THEME;
  }
}

export function writeTerminalTheme(value: TerminalTheme): void {
  try {
    localStorage.setItem(STORAGE_TERMINAL_THEME, value);
  } catch {
    // best-effort
  }
}

export function resolveTerminalTheme(id: TerminalTheme): ITheme {
  return TERMINAL_THEMES[id];
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
