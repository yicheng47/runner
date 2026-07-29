import { invoke } from "@tauri-apps/api/core";

/**
 * Mirror a `[launch-dims]` diagnostic line into the backend's rotating file
 * log via the `frontend_log` command. Packaged builds have no webview
 * console, and the launch-resume width bug (#366) only reproduces there —
 * this is the only route these lines have into
 * `~/Library/Logs/…/runner.log`. Best-effort: a missing Tauri runtime
 * (browser preview) degrades to the console line alone.
 */
export function logLaunchDims(message: string): void {
  const line = `[launch-dims] ${message}`;
  console.info(line);
  try {
    void invoke("frontend_log", { level: "info", message: line }).catch(
      () => {
        // Browser preview has no Tauri runtime.
      },
    );
  } catch {
    // invoke can throw synchronously outside Tauri.
  }
}
