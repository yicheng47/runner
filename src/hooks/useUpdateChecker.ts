// Auto-update hook — wraps `@tauri-apps/plugin-updater` into a simple
// state machine: idle → checking → available → downloading → ready.
// Mirrors Quill's `useUpdateChecker` so the surfaces (sidebar prompt
// card + Updates pane) can share one context and the same UI states.

import { useCallback, useRef, useState } from "react";

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "error";

export interface UpdateCheckOptions {
  silent?: boolean;
}

const UPDATE_STATUSES: readonly UpdateStatus[] = [
  "idle",
  "checking",
  "available",
  "downloading",
  "ready",
  "error",
];

// Dev-only escape hatch: seed the state machine from localStorage so
// the update surfaces (About button ladder, sidebar prompt pill/card)
// can be smoke-tested without staging a real release — outside a
// signed build, `check()` never yields an update, so "ready" is
// otherwise unreachable in dev. In the devtools console:
//   localStorage.setItem("runner.dev.updateStatus", "ready"); location.reload()
// and remove the key to restore normal behavior. The resting-state
// guard in checkForUpdate keeps the launch auto-check from clobbering
// a seeded non-resting status. DEV-gated: release builds never read it.
function readDevStatusOverride(): UpdateStatus | null {
  if (!import.meta.env.DEV) return null;
  try {
    const raw = localStorage.getItem("runner.dev.updateStatus");
    return raw && (UPDATE_STATUSES as readonly string[]).includes(raw)
      ? (raw as UpdateStatus)
      : null;
  } catch {
    return null;
  }
}

export interface UpdateState {
  status: UpdateStatus;
  update: Update | null;
  progress: number;
  error: string | null;
  checkForUpdate: (options?: UpdateCheckOptions) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  restart: () => Promise<void>;
}

export function useUpdateChecker(): UpdateState {
  const [status, setStatus] = useState<UpdateStatus>(
    () => readDevStatusOverride() ?? "idle",
  );
  const [update, setUpdate] = useState<Update | null>(null);
  // A seeded "downloading" gets a mid-flight progress value so the
  // About pane's bar reads as a real download, not a frozen 0%.
  const [progress, setProgress] = useState(() =>
    readDevStatusOverride() === "downloading" ? 37 : 0,
  );
  const [error, setError] = useState<string | null>(null);
  // Guard against concurrent checks — auto-check + manual click can race.
  const checking = useRef(false);
  // A manual request that overlaps a silent background check upgrades
  // that in-flight check so a failure is visible to the user.
  const surfaceCheckError = useRef(false);
  // Mirror of `status` readable from the stable `checkForUpdate`
  // closure. Update it synchronously with React state so another
  // trigger cannot slip into the render gap after a check finishes.
  const statusRef = useRef<UpdateStatus>(status);
  const setCurrentStatus = useCallback((next: UpdateStatus) => {
    statusRef.current = next;
    setStatus(next);
  }, []);

  const checkForUpdate = useCallback(async (options?: UpdateCheckOptions) => {
    const silent = options?.silent ?? false;
    if (checking.current) {
      if (!silent) surfaceCheckError.current = true;
      return;
    }
    // Only re-check from resting states. Multiple surfaces auto-check
    // (UpdateContext's launch/interval/focus triggers, Updates on mount)
    // and they can
    // overlap — a late check must not clobber an already-found update
    // or an in-flight/finished download back to "checking".
    if (statusRef.current !== "idle" && statusRef.current !== "error") {
      return;
    }
    checking.current = true;
    surfaceCheckError.current = !silent;
    setCurrentStatus("checking");
    setError(null);
    try {
      const result = await check();
      if (result) {
        setUpdate(result);
        setCurrentStatus("available");
      } else {
        setCurrentStatus("idle");
      }
    } catch (e) {
      if (surfaceCheckError.current) {
        setError(e instanceof Error ? e.message : String(e));
        setCurrentStatus("error");
      } else {
        setError(null);
        setCurrentStatus("idle");
      }
    } finally {
      checking.current = false;
      surfaceCheckError.current = false;
    }
  }, [setCurrentStatus]);

  const downloadAndInstall = useCallback(async () => {
    if (!update) return;
    setCurrentStatus("downloading");
    setProgress(0);
    try {
      let totalLen = 0;
      let downloaded = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          totalLen = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          if (totalLen > 0) {
            setProgress(Math.round((downloaded / totalLen) * 100));
          }
        } else if (event.event === "Finished") {
          setProgress(100);
        }
      });
      setCurrentStatus("ready");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setCurrentStatus("error");
    }
  }, [setCurrentStatus, update]);

  const restart = useCallback(async () => {
    await relaunch();
  }, []);

  return {
    status,
    update,
    progress,
    error,
    checkForUpdate,
    downloadAndInstall,
    restart,
  };
}
