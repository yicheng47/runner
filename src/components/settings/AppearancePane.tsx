// Appearance pane — theme, palette, and font.

import { useState } from "react";
import { Monitor, Moon, Sun } from "lucide-react";

import {
  APP_FONT_FAMILY_OPTIONS,
  APP_THEME_OPTIONS,
  DARK_VARIANT_ACCENTS,
  DARK_VARIANT_LABELS,
  DARK_VARIANT_OPTIONS,
  LIGHT_VARIANT_ACCENTS,
  LIGHT_VARIANT_LABELS,
  LIGHT_VARIANT_OPTIONS,
  notifySameWindowStorage,
  readAppFontFamily,
  readAppTheme,
  readDarkVariant,
  readLightVariant,
  STORAGE_APP_DARK_VARIANT,
  STORAGE_APP_FONT_FAMILY,
  STORAGE_APP_LIGHT_VARIANT,
  STORAGE_APP_THEME,
  type AppFontFamily,
  type AppTheme,
  type DarkVariant,
  type LightVariant,
  writeAppFontFamily,
  writeAppTheme,
  writeDarkVariant,
  writeLightVariant,
} from "../../lib/settings";
import { StyledSelect } from "../ui/StyledSelect";
import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function AppearancePane() {
  // Theme = user *intent* (auto/light/dark). lightVariant + darkVariant
  // are which palette to use on each side; v1 ships Codex Light + Carbon
  // & Plasma. Writes go through `writeAppTheme` / `writeLightVariant`
  // then fire a same-window storage event so `applyAppTheme()` in
  // `main.tsx` flips `<html data-theme>` without a reload.
  const [theme, setThemeState] = useState<AppTheme>(() => readAppTheme());
  const [lightVariant, setLightVariantState] = useState<LightVariant>(() =>
    readLightVariant(),
  );
  const [darkVariant, setDarkVariantState] = useState<DarkVariant>(() =>
    readDarkVariant(),
  );
  const [fontFamily, setFontFamilyState] = useState<AppFontFamily>(() =>
    readAppFontFamily(),
  );
  const setTheme = (next: AppTheme) => {
    setThemeState(next);
    writeAppTheme(next);
    notifySameWindowStorage(STORAGE_APP_THEME, next);
  };
  const setLightVariant = (next: LightVariant) => {
    setLightVariantState(next);
    writeLightVariant(next);
    notifySameWindowStorage(STORAGE_APP_LIGHT_VARIANT, next);
  };
  const setDarkVariant = (next: DarkVariant) => {
    setDarkVariantState(next);
    writeDarkVariant(next);
    notifySameWindowStorage(STORAGE_APP_DARK_VARIANT, next);
  };
  const setFontFamily = (next: AppFontFamily) => {
    setFontFamilyState(next);
    writeAppFontFamily(next);
    notifySameWindowStorage(STORAGE_APP_FONT_FAMILY, next);
  };
  return (
    <>
      <PaneHeader title="Appearance" subtitle="Theme, palette, and font." />
      <SettingsCard>
        <SettingsRow label="Theme" sub="Match the OS, or pin to light or dark.">
          <ThemeSegmented value={theme} onChange={setTheme} />
        </SettingsRow>
        <SettingsRow
          label="Light theme"
          sub="Picked when the OS is light or Theme = Light."
        >
          <StyledSelect
            value={lightVariant}
            options={LIGHT_VARIANT_OPTIONS.map((id) => ({
              value: id,
              label: LIGHT_VARIANT_LABELS[id],
              swatchColor: LIGHT_VARIANT_ACCENTS[id],
            }))}
            onChange={(v) => setLightVariant(v as LightVariant)}
          />
        </SettingsRow>
        <SettingsRow
          label="Dark theme"
          sub="Picked when the OS is dark or Theme = Dark."
        >
          <StyledSelect
            value={darkVariant}
            options={DARK_VARIANT_OPTIONS.map((id) => ({
              value: id,
              label: DARK_VARIANT_LABELS[id],
              swatchColor: DARK_VARIANT_ACCENTS[id],
            }))}
            onChange={(v) => setDarkVariant(v as DarkVariant)}
          />
        </SettingsRow>
        <SettingsRow
          label="App font"
          sub="UI typeface across the app. Doesn't apply to the embedded terminal — see Terminal pane."
        >
          <StyledSelect
            value={fontFamily}
            options={APP_FONT_FAMILY_OPTIONS.map((f) => ({
              value: f,
              label: f,
            }))}
            onChange={(v) => setFontFamily(v as AppFontFamily)}
          />
        </SettingsRow>
      </SettingsCard>
    </>
  );
}

// Segmented Auto · Light · Dark control. Mirrors the Pencil node `J0lKR`
// in `runner-mvp-design.pen` — 3 cells inside a rounded container, active
// cell carries the raised surface + accent label, inactive cells stay
// muted on the bg.
function ThemeSegmented({
  value,
  onChange,
}: {
  value: AppTheme;
  onChange: (next: AppTheme) => void;
}) {
  const ICONS: Record<AppTheme, typeof Monitor> = {
    auto: Monitor,
    light: Sun,
    dark: Moon,
  };
  const LABELS: Record<AppTheme, string> = {
    auto: "Auto",
    light: "Light",
    dark: "Dark",
  };
  return (
    <div
      role="radiogroup"
      aria-label="Theme"
      className="flex items-center gap-0.5 rounded-md border border-line bg-bg p-0.5"
    >
      {APP_THEME_OPTIONS.map((option) => {
        const Icon = ICONS[option];
        const active = option === value;
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option)}
            className={`flex cursor-pointer items-center gap-1.5 rounded-[4px] px-2.5 py-[5px] text-[12px] font-medium transition-colors ${
              active ? "bg-raised text-fg" : "text-fg-2 hover:text-fg"
            }`}
          >
            <Icon aria-hidden className="h-3 w-3" />
            <span>{LABELS[option]}</span>
          </button>
        );
      })}
    </div>
  );
}
