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
// Terminal theme. Single setting that does not auto-swap with the app
// theme — `runner` (dark) stays the default even in light app mode,
// because most TUIs (claude-code, codex) assume a dark canvas. Light
// palettes ship in the list for users who explicitly want them.
export const STORAGE_TERMINAL_THEME = "settings.terminalTheme";
export const STORAGE_DEFAULT_WORKING_DIR = "settings.defaultWorkingDir";
export const STORAGE_APP_THEME = "settings.appTheme";
export const STORAGE_APP_LIGHT_VARIANT = "settings.appLightVariant";
export const STORAGE_APP_DARK_VARIANT = "settings.appDarkVariant";
export const STORAGE_APP_BRAND_TINT = "settings.appBrandTint";
export const STORAGE_APP_FONT_FAMILY = "settings.appFontFamily";

// Chrome theme — the user's *intent*, not the resolved surface.
// `auto` defers to `prefers-color-scheme`; `light`/`dark` pin regardless
// of the OS. Resolution happens in `applyAppTheme()`.
export type AppTheme = "auto" | "light" | "dark";
export const APP_THEME_OPTIONS: readonly AppTheme[] = ["auto", "light", "dark"];
const DEFAULT_APP_THEME: AppTheme = "auto";

// Light theme variant — Codex Light is the v1 default; Catppuccin
// Latte ships alongside as a warmer, more saturated alternative.
// Solarized Paper is still designed (frame `iBOyT` / `pLbNm`) but
// deferred; adding new variants is a pure additive change here.
export type LightVariant = "codex" | "catppuccin-latte";
export const LIGHT_VARIANT_OPTIONS: readonly LightVariant[] = [
  "codex",
  "catppuccin-latte",
];
export const LIGHT_VARIANT_LABELS: Record<LightVariant, string> = {
  codex: "Codex Light",
  "catppuccin-latte": "Catppuccin Latte",
};
// Swatch the SettingsModal dropdown row paints next to each label.
// Matches `--color-accent` for that variant so the picker previews
// the accent the user is about to switch into.
export const LIGHT_VARIANT_ACCENTS: Record<LightVariant, string> = {
  codex: "#339CFF",
  "catppuccin-latte": "#8839EF",
};
const DEFAULT_LIGHT_VARIANT: LightVariant = "codex";

// Dark theme variant — Runner (Carbon chrome, neon-green accent) is
// the canonical dark; Catppuccin Mocha pairs symmetrically with the
// Catppuccin Latte light variant for users who want a warmer, more
// saturated palette across both surfaces.
export type DarkVariant = "carbon" | "catppuccin-mocha";
export const DARK_VARIANT_OPTIONS: readonly DarkVariant[] = [
  "carbon",
  "catppuccin-mocha",
];
export const DARK_VARIANT_LABELS: Record<DarkVariant, string> = {
  carbon: "Runner",
  "catppuccin-mocha": "Catppuccin Mocha",
};
export const DARK_VARIANT_ACCENTS: Record<DarkVariant, string> = {
  carbon: "#00FF9C",
  "catppuccin-mocha": "#CBA6F7",
};
const DEFAULT_DARK_VARIANT: DarkVariant = "carbon";

// In-sidebar chevron brand mark color. Now always picks up
// `var(--color-accent)` for the active theme (sky-blue in Codex Light,
// neon-green in Runner). The "tint off" mode that pinned the chevron
// to `BRAND_MARK_PINNED_COLOR` was removed from Settings; the constant
// stays exported in case external surfaces still want to pin to the
// `.icns` shade.
export const BRAND_MARK_PINNED_COLOR = "#00FF9C";

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

// App-wide UI font. "Inter" is the current default — what `body` has
// been using since v1. "System UI" picks the OS-native font instead
// (SF Pro on macOS, Segoe UI on Windows, system fallback on Linux),
// for users who prefer their app to match the rest of their desktop
// chrome. Mono is intentionally absent — that lives on the Terminal
// font picker.
export type AppFontFamily = "Inter" | "Geist" | "Roboto" | "System UI";
export const APP_FONT_FAMILY_OPTIONS: readonly AppFontFamily[] = [
  "Inter",
  "Geist",
  "Roboto",
  "System UI",
];
const DEFAULT_APP_FONT_FAMILY: AppFontFamily = "Inter";

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
  | "catppuccin-mocha"
  | "solarized-dark";
export const TERMINAL_THEME_OPTIONS: readonly TerminalTheme[] = [
  "runner",
  "catppuccin-mocha",
  "solarized-dark",
];
export const TERMINAL_THEME_LABELS: Record<TerminalTheme, string> = {
  runner: "Runner",
  "catppuccin-mocha": "Catppuccin Mocha",
  "solarized-dark": "Solarized Dark",
};
// Swatch color shown next to each label in the terminal-theme
// dropdown — mirrors the Appearance dropdowns. Picks the palette's
// most identifiable hue (Runner's neon green, Catppuccin's mauve,
// Solarized's blue) instead of the cursor color, which is often a
// neutral gray and wouldn't read as a brand swatch.
export const TERMINAL_THEME_ACCENTS: Record<TerminalTheme, string> = {
  runner: "#00FF9C",
  "catppuccin-mocha": "#CBA6F7",
  "solarized-dark": "#268BD2",
};
export const DEFAULT_TERMINAL_THEME: TerminalTheme = "runner";

// Each terminal theme now carries its own background — claude-code /
// codex paint dark/light cards on top either way, so locking the
// canvas to a single shade only flattened theme identity. The
// surrounding workspace padding wrapper reads `resolveTerminalBg()`
// and tracks the theme's bg so the canvas + frame stay seamless even
// across theme switches.

export const TERMINAL_THEMES: Record<TerminalTheme, ITheme> = {
  runner: {
    background: "#15161B",
    foreground: "#DCDCE0",
    cursor: "#00FF9C",
    cursorAccent: "#15161B",
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
  "catppuccin-mocha": {
    background: "#1E1E2E",
    foreground: "#CDD6F4",
    cursor: "#F5E0DC",
    cursorAccent: "#1E1E2E",
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
  // Solarized Dark — Ethan Schoonover's canonical dark palette. Bg
  // stays locked to Carbon chrome (the official base03 #002b36 would
  // be close to our #15161B but we keep it uniform across dark
  // palettes for seamless terminal canvas). ANSI 16 follows the spec
  // exactly: base02..base3 + the 6 accent hues map to black..bright
  // white in the Solarized convention.
  "solarized-dark": {
    background: "#002B36",
    foreground: "#839496",
    cursor: "#93A1A1",
    cursorAccent: "#002B36",
    selectionBackground: "#073642",
    black: "#073642",
    red: "#DC322F",
    green: "#859900",
    yellow: "#B58900",
    blue: "#268BD2",
    magenta: "#D33682",
    cyan: "#2AA198",
    white: "#EEE8D5",
    brightBlack: "#002B36",
    brightRed: "#CB4B16",
    brightGreen: "#586E75",
    brightYellow: "#657B83",
    brightBlue: "#839496",
    brightMagenta: "#6C71C4",
    brightCyan: "#93A1A1",
    brightWhite: "#FDF6E3",
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
//
// Head — Nerd Font fallbacks for the Private Use Area glyphs that
// claude-code and other modern TUIs emit on their status line (PR /
// branch / lock / cursor icons live in U+E000–U+F8FF). CSS font
// stacks fall through per-codepoint, so the Latin / digit codepoints
// keep coming from Menlo / SF Mono — the symbols fonts only win on
// the PUA codepoints they cover, and they no-op if the user hasn't
// installed any of these patches. Without this, macOS's automatic
// PUA fallback lands on Apple SD Gothic Neo / Hiragino and the
// glyph renders as a Hangul-looking character (#152 follow-up).
//
// Tail — `Apple Symbols` catches non-Nerd-Font symbol codepoints
// (extra math, arrows, miscellaneous technical) before the generic
// `monospace` fallback so the browser doesn't fall back to a CJK
// face for those either.
const SYSTEM_FONT_STACK =
  '"Symbols Nerd Font Mono", "Symbols Nerd Font", ' +
  '"JetBrainsMono Nerd Font", "FiraCode Nerd Font", ' +
  '"Hack Nerd Font Mono", ' +
  'Menlo, "SF Mono", Monaco, Consolas, "Liberation Mono", ' +
  '"Apple Symbols", monospace';

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

// Bg color for the currently-picked terminal palette. Used by the
// workspace pane that hosts the xterm canvas so the surrounding
// padding matches the canvas — without this the canvas would float
// inside a `bg-terminal-chrome` frame that no longer agrees with
// theme-native bgs (Solarized Dark #002B36, Latte #EFF1F5, etc.).
// Falls back to Runner's bg if the stored id ever drifts.
export function resolveTerminalBg(id: TerminalTheme): string {
  return TERMINAL_THEMES[id].background ?? "#15161B";
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

// OS-native UI stack — no Inter, no third-party fonts. SF Pro on
// macOS, Segoe UI on Windows, and the platform default on Linux.
// Used as the tail for "System UI" and as the fallback after every
// named-font choice so the UI renders even when the requested face
// isn't installed.
const APP_OS_FONT_STACK =
  "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif";

// `@fontsource-variable/*` registers each face under a "Variable"-
// suffixed family name (e.g. `Inter Variable`, not `Inter`). The
// picker keeps the clean labels for users; this map routes them to
// the real names so the CSS lookup actually finds the bundled font.
const APP_FONT_CSS_FAMILY: Record<
  Exclude<AppFontFamily, "System UI">,
  string
> = {
  Inter: "Inter Variable",
  Geist: "Geist Variable",
  Roboto: "Roboto Variable",
};

export function resolveAppFontStack(label: AppFontFamily): string {
  if (label === "System UI") return APP_OS_FONT_STACK;
  const cssFamily = APP_FONT_CSS_FAMILY[label];
  return `'${cssFamily}', ${APP_OS_FONT_STACK}`;
}

export function readAppFontFamily(): AppFontFamily {
  try {
    const raw = localStorage.getItem(STORAGE_APP_FONT_FAMILY);
    if (raw == null) return DEFAULT_APP_FONT_FAMILY;
    return (APP_FONT_FAMILY_OPTIONS as readonly string[]).includes(raw)
      ? (raw as AppFontFamily)
      : DEFAULT_APP_FONT_FAMILY;
  } catch {
    return DEFAULT_APP_FONT_FAMILY;
  }
}

export function writeAppFontFamily(value: AppFontFamily): void {
  try {
    localStorage.setItem(STORAGE_APP_FONT_FAMILY, value);
  } catch {
    // best-effort
  }
}

// Writes `--font-app` on `<html>`, picked up by the `body` rule in
// `index.css`. Call at boot (before React mounts) so the picked font
// is in place on first paint, and again on storage events so the
// Settings change applies without reload.
export function applyAppFont(): void {
  const root = document.documentElement;
  root.style.setProperty("--font-app", resolveAppFontStack(readAppFontFamily()));
}

export function readDefaultWorkingDir(): string {
  try {
    const raw = localStorage.getItem(STORAGE_DEFAULT_WORKING_DIR) ?? "";
    return raw.trim();
  } catch {
    return "";
  }
}

export function writeDefaultWorkingDir(value: string): void {
  try {
    const trimmed = value.trim();
    if (trimmed) localStorage.setItem(STORAGE_DEFAULT_WORKING_DIR, trimmed);
    else localStorage.removeItem(STORAGE_DEFAULT_WORKING_DIR);
  } catch {
    // best-effort
  }
}

export function readAppTheme(): AppTheme {
  try {
    const raw = localStorage.getItem(STORAGE_APP_THEME);
    if (raw == null) return DEFAULT_APP_THEME;
    return (APP_THEME_OPTIONS as readonly string[]).includes(raw)
      ? (raw as AppTheme)
      : DEFAULT_APP_THEME;
  } catch {
    return DEFAULT_APP_THEME;
  }
}

export function writeAppTheme(value: AppTheme): void {
  try {
    localStorage.setItem(STORAGE_APP_THEME, value);
  } catch {
    // best-effort
  }
}

export function readLightVariant(): LightVariant {
  try {
    const raw = localStorage.getItem(STORAGE_APP_LIGHT_VARIANT);
    if (raw == null) return DEFAULT_LIGHT_VARIANT;
    return (LIGHT_VARIANT_OPTIONS as readonly string[]).includes(raw)
      ? (raw as LightVariant)
      : DEFAULT_LIGHT_VARIANT;
  } catch {
    return DEFAULT_LIGHT_VARIANT;
  }
}

export function writeLightVariant(value: LightVariant): void {
  try {
    localStorage.setItem(STORAGE_APP_LIGHT_VARIANT, value);
  } catch {
    // best-effort
  }
}

export function readDarkVariant(): DarkVariant {
  try {
    const raw = localStorage.getItem(STORAGE_APP_DARK_VARIANT);
    if (raw == null) return DEFAULT_DARK_VARIANT;
    return (DARK_VARIANT_OPTIONS as readonly string[]).includes(raw)
      ? (raw as DarkVariant)
      : DEFAULT_DARK_VARIANT;
  } catch {
    return DEFAULT_DARK_VARIANT;
  }
}

export function writeDarkVariant(value: DarkVariant): void {
  try {
    localStorage.setItem(STORAGE_APP_DARK_VARIANT, value);
  } catch {
    // best-effort
  }
}

// Brand-mark tint is always on — the toggle was removed from Settings
// once we decided the accent-matched chevron should be the only behavior.
// Kept as a function (not a constant) so call sites and the storage-event
// path in Sidebar.tsx stay intact without a wider refactor.
export function readBrandTint(): boolean {
  return true;
}

// Returns "light" or "dark" — the resolved surface for the user's
// current intent. `auto` consults `prefers-color-scheme`; the explicit
// values short-circuit. Boot code calls this once before React mounts
// to avoid a white-flash on the first paint of dark-mode users.
export function resolveAppSurface(intent: AppTheme): "light" | "dark" {
  if (intent === "light") return "light";
  if (intent === "dark") return "dark";
  try {
    return window.matchMedia &&
      window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  } catch {
    return "dark";
  }
}

// Reads the current intent + light-variant from storage and writes the
// resolved `data-theme` attribute to `<html>`. Carbon (dark) is the
// unattributed default — when the surface is dark, the attribute is
// removed so the `@theme` block in `index.css` cascades unchanged.
//
// Call from `main.tsx` once at boot, and again whenever the user
// changes Theme / Light variant in SettingsModal (handled via the
// same `storage` event the terminal settings ride on).
export function applyAppTheme(): void {
  const intent = readAppTheme();
  const surface = resolveAppSurface(intent);
  const root = document.documentElement;
  if (surface === "light") {
    const variant = readLightVariant();
    root.setAttribute("data-theme", variant);
  } else {
    // Dark: Carbon is the unattributed default (the `@theme` block in
    // index.css), so we only set `data-theme` when the user picked a
    // non-default dark variant. Keeps the cascade simple — anything
    // without `data-theme` falls back to Carbon.
    const variant = readDarkVariant();
    if (variant === DEFAULT_DARK_VARIANT) {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", variant);
    }
  }
}

// Wire `prefers-color-scheme` change events to `applyAppTheme()` so
// `auto` intent flips live when the OS theme changes. The listener
// also re-runs for explicit Light / Dark intents (no-op there, the
// resolved surface doesn't depend on OS pref). Returns a teardown so
// callers can unmount it; for the app's single root listener, we
// keep it for the lifetime of the process.
export function subscribeOsThemeChange(): () => void {
  let media: MediaQueryList | null = null;
  try {
    media = window.matchMedia("(prefers-color-scheme: light)");
  } catch {
    return () => {};
  }
  const onChange = () => {
    applyAppTheme();
  };
  // Safari < 14 only supports the deprecated `addListener` form. Use
  // `addEventListener` when available, fall back otherwise — the
  // Tauri webview on macOS 11+ is fine with the modern API but the
  // fallback is cheap insurance.
  if (typeof media.addEventListener === "function") {
    media.addEventListener("change", onChange);
    return () => media!.removeEventListener("change", onChange);
  }
  // Deprecated path — `addListener` is `(listener: (e: MediaQueryListEvent) => void) => void`
  // on older browsers; the type isn't exposed on lib.dom.d.ts in modern TS
  // so cast through `unknown`.
  const legacy = media as unknown as {
    addListener: (cb: () => void) => void;
    removeListener: (cb: () => void) => void;
  };
  legacy.addListener(onChange);
  return () => legacy.removeListener(onChange);
}
