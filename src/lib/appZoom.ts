// App-zoom apply path shared by the Settings stepper and the global
// Cmd/Ctrl +/-/0 keyboard shortcuts. Lives here (not in `settings.ts`)
// so the Tauri import stays out of the leaf-level settings helpers.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { invoke } from "@tauri-apps/api/core";

import {
  notifySameWindowStorage,
  readAppZoom,
  STORAGE_APP_ZOOM,
  writeAppZoom,
  ZOOM_STEPS,
} from "./settings";

const SIDEBAR_TOGGLE_GLYPH_X = 94.3;
const SIDEBAR_TOGGLE_GLYPH_INSET = 6.3;

export function syncTitlebarZoom(zoom: number): Promise<void> {
  const gutter =
    Math.round(
      (SIDEBAR_TOGGLE_GLYPH_X / zoom - SIDEBAR_TOGGLE_GLYPH_INSET) *
        10_000,
    ) / 10_000;
  document.documentElement.style.setProperty(
    "--titlebar-sidebar-toggle-gutter",
    `${gutter}px`,
  );
  try {
    return invoke<void>("window_set_titlebar_zoom", { zoom }).catch(() => {});
  } catch {
    return Promise.resolve();
  }
}

export function applyAppZoom(next: number): void {
  writeAppZoom(next);
  void syncTitlebarZoom(next);
  try {
    void getCurrentWebview()
      .setZoom(next)
      .catch(() => {
        // best-effort — webview swap or platform refusal shouldn't block.
      });
  } catch {
    // No Tauri runtime (dev browser preview).
  }
  notifySameWindowStorage(STORAGE_APP_ZOOM, String(next));
}

export function nudgeAppZoom(direction: 1 | -1 | "reset"): void {
  if (direction === "reset") {
    applyAppZoom(1.0);
    return;
  }
  const cur = readAppZoom(); // already snapped to a known step
  const idx = ZOOM_STEPS.indexOf(cur);
  const safe = idx === -1 ? ZOOM_STEPS.indexOf(1.0) : idx;
  const nextIdx =
    direction === 1
      ? Math.min(ZOOM_STEPS.length - 1, safe + 1)
      : Math.max(0, safe - 1);
  applyAppZoom(ZOOM_STEPS[nextIdx]);
}
