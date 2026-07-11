import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Bundled UI fonts — variable-axis files served by `@fontsource-
// variable/*`. Importing them here registers `@font-face` blocks
// before our own CSS loads, so the rest of the bundle (and the
// `--font-app` var written by `applyAppFont()`) can reach them
// without a network call. "System UI" is the OS-native option
// and intentionally has no bundled file.
import "@fontsource-variable/inter";
import "@fontsource-variable/geist";
import "@fontsource-variable/roboto";

import "./index.css";
import {
  applyAppFont,
  applyAppTheme,
  STORAGE_APP_DARK_VARIANT,
  STORAGE_APP_FONT_FAMILY,
  STORAGE_APP_LIGHT_VARIANT,
  STORAGE_APP_THEME,
  subscribeOsThemeChange,
} from "./lib/settings";

// Apply the resolved chrome theme *before* React mounts so dark-mode
// users don't see a white flash on the first paint. `applyAppTheme()`
// is a pure DOM write — no React state, no re-renders. `applyAppFont()`
// sets `--font-app` for the same reason: pick the user's preferred
// face before first paint so we don't flash the default and re-paint.
applyAppTheme();
applyAppFont();

// Live OS-pref change for `auto` intent. Listener stays for the
// lifetime of the process; no teardown is needed in a single-window
// Tauri app.
subscribeOsThemeChange();

// Reapply when the Settings page flips Theme or Light variant. The
// Appearance pane writes through `writeAppTheme` / `writeLightVariant`
// and then fires a synthesized `storage` event (via
// `notifySameWindowStorage`) so other surfaces — including this root
// listener — pick up the change without a reload.
window.addEventListener("storage", (e) => {
  if (
    e.key === STORAGE_APP_THEME ||
    e.key === STORAGE_APP_LIGHT_VARIANT ||
    e.key === STORAGE_APP_DARK_VARIANT
  ) {
    applyAppTheme();
  } else if (e.key === STORAGE_APP_FONT_FAMILY) {
    applyAppFont();
  }
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
