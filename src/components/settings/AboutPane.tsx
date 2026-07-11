// About pane — version, updates, and links (impl 0025 decision 3:
// Updates merged in, the Updates tab is gone). Leads with a hero card:
// real app icon, name + version chip, status line, and the stateful
// update button. Button ladder per spec `IKGNz` in
// `design/runner-setting.pen`: IDLE → CHECKING → AVAILABLE →
// DOWNLOADING → READY; READY is the one solid-accent moment in
// settings. Auto-checks on mount — without a dedicated tab, a
// manual-only check means nobody ever sees AVAILABLE.

import { useEffect, useState } from "react";
import {
  BookText,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  RotateCcw,
  Scale,
} from "lucide-react";

import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

import appIcon from "../../assets/app-icon.png";
import { STORAGE_AUTO_INSTALL_UPDATES } from "../../lib/settings";
import { useStoredBool } from "../../lib/useStoredBool";
import { useUpdate } from "../../contexts/UpdateContext";
import { Toggle } from "../ui/Toggle";
import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function AboutPane() {
  const [version, setVersion] = useState<string>("");
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
      .then((v) => setVersion(v))
      .catch(() => setVersion(""));
  }, []);
  // Auto-check on mount, but only from a resting state — kicking a
  // check while an update is already available/downloading/ready
  // would reset shared updater state the user may be acting on.
  useEffect(() => {
    if (status === "idle" || status === "error") void checkForUpdate();
    // Run once per mount; `status` at mount time is what matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Status-driven copy — the hero's status line always narrates the
  // update ladder's current rung.
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
  const openLink = (url: string) => {
    void openUrl(url).catch(() => {
      // Fallback: window.open works in dev (browser preview) when
      // the Tauri opener plugin isn't available.
      window.open(url, "_blank");
    });
  };
  return (
    <>
      <PaneHeader title="About" subtitle="Version, updates, and links." />

      {/* Hero card — the in-app identity matches what users see in the
          Dock / file-explorer: the icon mirrors the bundled `.icns`,
          imported from `src/assets/` so Vite emits a hashed URL. */}
      <div className="overflow-hidden rounded-xl border border-line bg-panel">
        <div className="flex items-center gap-4 p-5">
          <img
            src={appIcon}
            alt="Runner icon"
            width={56}
            height={56}
            className="h-14 w-14 shrink-0 rounded-2xl"
          />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex items-center gap-2">
              <span className="text-[16px] font-bold text-fg">Runner</span>
              <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-[11px] text-fg-2">
                v{version || "0.0.0"}
              </span>
            </div>
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

      <SettingsCard>
        <LinkRow
          icon={<GithubGlyph />}
          label="GitHub"
          onClick={() => openLink("https://github.com/yicheng47/runner")}
          external
        />
        <LinkRow
          icon={<BookText aria-hidden className="h-3.5 w-3.5 text-fg-2" />}
          label="Documentation"
          onClick={() => openLink("https://github.com/yicheng47/runner#readme")}
          external
        />
        <LinkRow
          icon={<Scale aria-hidden className="h-3.5 w-3.5 text-fg-2" />}
          label="License"
          trailing={<span className="text-[12px] text-fg-3">MIT</span>}
        />
      </SettingsCard>

      <div className="flex items-center justify-center text-[11px] text-fg-3">
        © 2026 wyc studios
      </div>
    </>
  );
}

// The five-state update button (spec `IKGNz`). Neutral for states,
// accent for meaning: AVAILABLE is quiet accent (tinted fill, accent
// text/border), READY is solid accent, everything else stays neutral.
// No cancel during download — the plugin exposes no abort handle, so
// we don't pretend to offer one.
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
  // idle / error → manual re-check
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

function LinkRow({
  icon,
  label,
  onClick,
  external,
  trailing,
}: {
  icon: React.ReactNode;
  label: string;
  onClick?: () => void;
  external?: boolean;
  trailing?: React.ReactNode;
}) {
  const interactive = !!onClick;
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!interactive}
      className={`flex w-full items-center justify-between px-4 py-3 text-left ${
        interactive ? "cursor-pointer hover:bg-raised/40" : "cursor-default"
      }`}
    >
      <span className="flex items-center gap-2.5">
        <span className="flex h-3.5 w-3.5 items-center justify-center text-fg-2">
          {icon}
        </span>
        <span className="text-[13px] text-fg">{label}</span>
      </span>
      {trailing ?? (
        external ? (
          <ExternalLink aria-hidden className="h-3 w-3 text-fg-3" />
        ) : null
      )}
    </button>
  );
}

function GithubGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="currentColor"
      className="text-fg-2"
      aria-hidden
    >
      <path d="M12 .5C5.6.5.5 5.7.5 12.1c0 5.1 3.3 9.4 7.9 10.9.6.1.8-.3.8-.6v-2.1c-3.2.7-3.9-1.5-3.9-1.5-.5-1.3-1.3-1.7-1.3-1.7-1-.7.1-.7.1-.7 1.1.1 1.7 1.2 1.7 1.2 1 1.7 2.7 1.2 3.4.9.1-.7.4-1.2.7-1.5-2.6-.3-5.3-1.3-5.3-5.7 0-1.3.4-2.3 1.2-3.1-.1-.3-.5-1.5.1-3.2 0 0 1-.3 3.3 1.2.9-.3 1.9-.4 2.9-.4s2 .1 2.9.4c2.3-1.5 3.3-1.2 3.3-1.2.7 1.7.2 2.9.1 3.2.8.8 1.2 1.9 1.2 3.1 0 4.4-2.7 5.4-5.3 5.7.4.4.8 1.1.8 2.2v3.3c0 .3.2.7.8.6 4.6-1.5 7.9-5.8 7.9-10.9C23.5 5.7 18.4.5 12 .5z" />
    </svg>
  );
}
