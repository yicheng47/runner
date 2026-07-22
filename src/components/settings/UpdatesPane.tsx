import { useEffect, useState } from "react";
import {
  Download,
  Loader2,
  RefreshCw,
  RotateCcw,
} from "lucide-react";

import { getVersion } from "@tauri-apps/api/app";

import { useUpdate } from "../../contexts/UpdateContext";
import { STORAGE_AUTO_INSTALL_UPDATES } from "../../lib/settings";
import { useStoredBool } from "../../lib/useStoredBool";
import { Toggle } from "../ui/Toggle";
import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function UpdatesPane() {
  const [version, setVersion] = useState("");
  const [autoInstall, setAutoInstall] = useStoredBool(
    STORAGE_AUTO_INSTALL_UPDATES,
    true,
  );
  const {
    status,
    update,
    progress,
    error,
    checkForUpdate,
    downloadAndInstall,
    restart,
  } = useUpdate();

  useEffect(() => {
    void getVersion()
      .then((value) => setVersion(value))
      .catch(() => setVersion(""));
  }, []);

  useEffect(() => {
    if (status === "idle" || status === "error") void checkForUpdate();
    // Check once when the pane opens. The shared updater guard handles
    // overlap with launch, interval, focus, and menu-triggered checks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const statusLine = (() => {
    switch (status) {
      case "checking":
        return "Checking for updates…";
      case "available":
        return update?.version
          ? `v${update.version} is available.`
          : "An update is available.";
      case "downloading":
        return `Downloading update… ${progress}%`;
      case "ready":
        return "Update ready — restart to apply.";
      case "error":
        return error ? `Couldn't check: ${error}` : "Couldn't check for updates.";
      default:
        return "You're up to date.";
    }
  })();
  const statusTone =
    status === "available" || status === "ready"
      ? "text-accent"
      : status === "error"
        ? "text-danger"
        : "text-fg-2";

  return (
    <>
      <PaneHeader
        title="Updates"
        subtitle="Check for new versions and choose how they download."
      />

      <div className="overflow-hidden rounded-xl border border-line bg-panel">
        <div className="flex items-center gap-4 p-5">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-[11px] font-medium text-fg-3">
              Current version
            </span>
            <span className="font-mono text-[16px] font-semibold text-fg">
              v{version || "0.0.0"}
            </span>
            <span className={`truncate text-[12px] ${statusTone}`}>
              {statusLine}
            </span>
          </div>
          <UpdateAction
            status={status}
            version={update?.version}
            onCheck={() => void checkForUpdate()}
            onDownload={() => void downloadAndInstall()}
            onRestart={() => void restart()}
          />
        </div>
        {status === "downloading" ? (
          <div className="h-[3px] w-full bg-raised">
            <div
              className="h-full bg-accent transition-[width] duration-200"
              style={{ width: `${progress}%` }}
            />
          </div>
        ) : null}
      </div>

      <SettingsCard>
        <SettingsRow
          label="Install updates automatically"
          sub="Download and apply updates in the background. Restart needed to finish."
        >
          <Toggle on={autoInstall} onChange={setAutoInstall} />
        </SettingsRow>
      </SettingsCard>
    </>
  );
}

function UpdateAction({
  status,
  version,
  onCheck,
  onDownload,
  onRestart,
}: {
  status: ReturnType<typeof useUpdate>["status"];
  version: string | undefined;
  onCheck: () => void;
  onDownload: () => void;
  onRestart: () => void;
}) {
  if (status === "checking") {
    return (
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg-3">
        <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
        Checking…
      </div>
    );
  }
  if (status === "available") {
    return (
      <button
        type="button"
        onClick={onDownload}
        className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-accent/40 bg-accent/10 px-3 py-1.5 text-[12px] font-semibold text-accent transition-colors hover:bg-accent/15"
      >
        <Download aria-hidden className="h-3 w-3" />
        {version ? `Download v${version}` : "Download update"}
      </button>
    );
  }
  if (status === "downloading") {
    return (
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg-3">
        <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
        Downloading…
      </div>
    );
  }
  if (status === "ready") {
    return (
      <button
        type="button"
        onClick={onRestart}
        className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-accent-ink transition-colors hover:bg-accent/90"
      >
        <RotateCcw aria-hidden className="h-3 w-3" />
        Restart to update
      </button>
    );
  }
  return (
    <button
      type="button"
      onClick={onCheck}
      className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg"
    >
      <RefreshCw aria-hidden className="h-3 w-3" />
      Check for updates
    </button>
  );
}
