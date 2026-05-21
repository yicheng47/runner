import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import {
  applyAppTheme,
  STORAGE_APP_LIGHT_VARIANT,
  STORAGE_APP_THEME,
  subscribeOsThemeChange,
} from "./lib/settings";

// Apply the resolved chrome theme *before* React mounts so dark-mode
// users don't see a white flash on the first paint. `applyAppTheme()`
// is a pure DOM write — no React state, no re-renders.
applyAppTheme();

// Live OS-pref change for `auto` intent. Listener stays for the
// lifetime of the process; no teardown is needed in a single-window
// Tauri app.
subscribeOsThemeChange();

// Reapply when the SettingsModal flips Theme or Light variant. The
// modal writes through `writeAppTheme` / `writeLightVariant` and then
// fires a synthesized `storage` event (via `notifySameWindowStorage`)
// so other surfaces — including this root listener — pick up the
// change without a reload.
window.addEventListener("storage", (e) => {
  if (e.key !== STORAGE_APP_THEME && e.key !== STORAGE_APP_LIGHT_VARIANT) return;
  applyAppTheme();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
