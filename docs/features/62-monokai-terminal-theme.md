# Monokai terminal theme

Tracking issue: [#406](https://github.com/yicheng47/runner/issues/406). Status: planned.

## Motivation

The terminal theme picker ships Runner, Catppuccin Mocha, and Solarized Dark; Monokai — the palette the primary user actually wants in the panes daily — is missing. The theme system (`src/lib/settings.ts`) was explicitly built so variants are a pure additive change: union member + palette + label/accent entries, with the Settings dropdown, live repaint (`RunnerTerminal.tsx:1080`), and canvas background tracking (`resolveTerminalBg()`) all driven from the constants.

## Scope

- **`src/lib/settings.ts`**: add `"monokai"` to `TerminalTheme`, `TERMINAL_THEME_OPTIONS`, `TERMINAL_THEME_LABELS` ("Monokai"), and `TERMINAL_THEME_ACCENTS` — swatch `#F92672`, Monokai's signature pink, following the "most identifiable hue" convention.
- **`TERMINAL_THEMES` palette** — classic Monokai, ANSI-16 mapped:

  ```ts
  monokai: {
    background: "#272822",
    foreground: "#F8F8F2",
    cursor: "#F8F8F2",
    cursorAccent: "#272822",
    selectionBackground: "#49483E",
    overviewRulerBorder: "transparent",
    black: "#272822",
    red: "#F92672",
    green: "#A6E22E",
    yellow: "#E6DB74",
    blue: "#66D9EF",
    magenta: "#AE81FF",
    cyan: "#A1EFE4",
    white: "#F8F8F2",
    brightBlack: "#75715E",
    brightRed: "#F92672",
    brightGreen: "#A6E22E",
    brightYellow: "#F4BF75",
    brightBlue: "#66D9EF",
    brightMagenta: "#AE81FF",
    brightCyan: "#A1EFE4",
    brightWhite: "#F9F8F5",
  },
  ```

- No UI changes: the Settings dropdown and swatch row render from the option list; the terminal option-refresh path already applies theme switches to live sessions.

## Non-Goals

- Monokai app-chrome (light/dark variant) theming — terminal palette only.
- Monokai Pro / filter variants; one canonical Monokai.

## Implementation Phases

1. Constants + palette in `src/lib/settings.ts`.
2. Verification pass (below). No new components, no migrations.

## Verification

- `pnpm exec tsc --noEmit`, `pnpm run lint`.
- Manual: Settings → Terminal → theme dropdown shows Monokai with pink swatch; selecting it repaints open panes live (claude-code and codex TUIs legible: status lines, diffs, spinners); workspace padding wrapper tracks the `#272822` canvas seamlessly; selection persists across relaunch.
