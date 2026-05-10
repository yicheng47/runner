// Shared helpers for the localStorage-backed settings used by both
// the Settings modal and the surfaces that consume those settings
// (e.g. UpdateContext). All settings persist via the same `"1"` /
// `"0"` encoding the modal writes — keep this file the single source
// of truth so the modal and its consumers can't drift apart.

export const STORAGE_AUTO_INSTALL_UPDATES = "settings.autoInstallUpdates";
export const STORAGE_SIDEBAR_COLLAPSED = "runner.sidebar.collapsed";

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
