// Terminal pane — xterm appearance settings for the runner terminal.

import { useState } from "react";

import {
  notifySameWindowStorage,
  readTerminalCursorStyle,
  readTerminalFontFamily,
  readTerminalFontSize,
  readTerminalScrollback,
  readTerminalTheme,
  STORAGE_TERMINAL_CURSOR_STYLE,
  STORAGE_TERMINAL_FONT_FAMILY,
  STORAGE_TERMINAL_FONT_SIZE,
  STORAGE_TERMINAL_SCROLLBACK,
  STORAGE_TERMINAL_THEME,
  TERMINAL_CURSOR_STYLE_OPTIONS,
  TERMINAL_FONT_FAMILY_OPTIONS,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  TERMINAL_SCROLLBACK_OPTIONS,
  TERMINAL_THEME_ACCENTS,
  TERMINAL_THEME_LABELS,
  TERMINAL_THEME_OPTIONS,
  type TerminalCursorStyle,
  type TerminalFontFamily,
  type TerminalTheme,
  writeTerminalCursorStyle,
  writeTerminalFontFamily,
  writeTerminalFontSize,
  writeTerminalScrollback,
  writeTerminalTheme,
} from "../../lib/settings";
import { StyledSelect } from "../ui/StyledSelect";
import { PaneHeader, SettingsCard, SettingsRow, Stepper } from "./shared";

export function TerminalPane() {
  const [fontSize, setFontSizeState] = useState<number>(() =>
    readTerminalFontSize(),
  );
  const [fontFamily, setFontFamilyState] = useState<TerminalFontFamily>(() =>
    readTerminalFontFamily(),
  );
  const [cursorStyle, setCursorStyleState] = useState<TerminalCursorStyle>(
    () => readTerminalCursorStyle(),
  );
  const [scrollback, setScrollbackState] = useState<number>(() =>
    readTerminalScrollback(),
  );
  const [theme, setThemeState] = useState<TerminalTheme>(() =>
    readTerminalTheme(),
  );
  const setFontSize = (next: number) => {
    setFontSizeState(next);
    writeTerminalFontSize(next);
    notifySameWindowStorage(STORAGE_TERMINAL_FONT_SIZE, String(next));
  };
  const setFontFamily = (next: TerminalFontFamily) => {
    setFontFamilyState(next);
    writeTerminalFontFamily(next);
    notifySameWindowStorage(STORAGE_TERMINAL_FONT_FAMILY, next);
  };
  const setCursorStyle = (next: TerminalCursorStyle) => {
    setCursorStyleState(next);
    writeTerminalCursorStyle(next);
    notifySameWindowStorage(STORAGE_TERMINAL_CURSOR_STYLE, next);
  };
  const setScrollback = (next: number) => {
    setScrollbackState(next);
    writeTerminalScrollback(next);
    notifySameWindowStorage(STORAGE_TERMINAL_SCROLLBACK, String(next));
  };
  const setTheme = (next: TerminalTheme) => {
    setThemeState(next);
    writeTerminalTheme(next);
    notifySameWindowStorage(STORAGE_TERMINAL_THEME, next);
  };
  return (
    <>
      <PaneHeader
        title="Terminal"
        subtitle="xterm appearance settings for the runner terminal."
      />
      <SettingsCard>
        <SettingsRow label="Theme" sub="ANSI palette for the embedded terminal.">
          <StyledSelect
            value={theme}
            options={TERMINAL_THEME_OPTIONS.map((id) => ({
              value: id,
              label: TERMINAL_THEME_LABELS[id],
              swatchColor: TERMINAL_THEME_ACCENTS[id],
            }))}
            onChange={(v) => setTheme(v as TerminalTheme)}
          />
        </SettingsRow>
        <SettingsRow
          label="Font family"
          sub="Typeface used by the embedded terminal."
        >
          <StyledSelect
            value={fontFamily}
            options={TERMINAL_FONT_FAMILY_OPTIONS.map((f) => ({
              value: f,
              label: f,
            }))}
            onChange={(v) => setFontFamily(v as TerminalFontFamily)}
          />
        </SettingsRow>
        <SettingsRow
          label="Terminal font size"
          sub="Glyph size for the embedded terminal."
        >
          <FontSizeStepper value={fontSize} onChange={setFontSize} />
        </SettingsRow>
        <SettingsRow
          label="Cursor style"
          sub="Block, underline, or bar — affects the prompt caret only."
        >
          <StyledSelect
            value={cursorStyle}
            options={TERMINAL_CURSOR_STYLE_OPTIONS.map((c) => ({
              value: c,
              label: c[0].toUpperCase() + c.slice(1),
            }))}
            onChange={(v) => setCursorStyle(v as TerminalCursorStyle)}
          />
        </SettingsRow>
        <SettingsRow
          label="Scrollback"
          sub="Lines kept in history per session. Higher uses more memory."
        >
          <div className="flex items-center gap-2">
            <StyledSelect
              value={String(scrollback)}
              options={TERMINAL_SCROLLBACK_OPTIONS.map((n) => ({
                value: String(n),
                label: n.toLocaleString(),
              }))}
              onChange={(v) => setScrollback(Number.parseInt(v, 10))}
            />
            <span className="text-[12px] text-fg-2">lines</span>
          </div>
        </SettingsRow>
      </SettingsCard>
    </>
  );
}

function FontSizeStepper({
  value,
  onChange,
}: {
  value: number;
  onChange: (v: number) => void;
}) {
  const clamped = Math.max(
    TERMINAL_FONT_SIZE_MIN,
    Math.min(TERMINAL_FONT_SIZE_MAX, value),
  );
  return (
    <Stepper
      valueCellWidth={64}
      decDisabled={clamped <= TERMINAL_FONT_SIZE_MIN}
      incDisabled={clamped >= TERMINAL_FONT_SIZE_MAX}
      decAriaLabel="Decrease terminal font size"
      incAriaLabel="Increase terminal font size"
      onDec={() => onChange(Math.max(TERMINAL_FONT_SIZE_MIN, clamped - 1))}
      onInc={() => onChange(Math.min(TERMINAL_FONT_SIZE_MAX, clamped + 1))}
    >
      <span className="flex items-center gap-[3px]">
        <span className="font-mono text-[12px] font-medium text-fg">
          {clamped}
        </span>
        <span className="font-mono text-[10px] text-fg-3">px</span>
      </span>
    </Stepper>
  );
}
