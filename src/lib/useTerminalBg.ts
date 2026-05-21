// React hook that exposes the active terminal palette's background
// color and re-renders when the user picks a new terminal theme.
//
// Both RunnerChat and MissionWorkspace paint a padding wrapper around
// the xterm canvas; that wrapper used to be hard-pinned to Carbon's
// chrome (#15161B). Now that each terminal theme owns its own bg
// (Solarized Dark #002B36, Catppuccin Latte #EFF1F5, …) the wrapper
// has to track the theme so the canvas + frame stay seamless.
//
// Subscribes to the same-window storage event the rest of the
// settings UI uses, so a Settings → Terminal Theme change updates
// the wrapper live without a page reload.

import { useEffect, useState } from "react";

import {
  readTerminalTheme,
  resolveTerminalBg,
  STORAGE_TERMINAL_THEME,
} from "./settings";

export function useTerminalBg(): string {
  const [bg, setBg] = useState<string>(() =>
    resolveTerminalBg(readTerminalTheme()),
  );
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_TERMINAL_THEME) return;
      setBg(resolveTerminalBg(readTerminalTheme()));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return bg;
}
