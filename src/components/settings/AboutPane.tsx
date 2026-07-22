// About pane — app identity, version, credits, and links.

import { useEffect, useState } from "react";
import { BookText, ExternalLink, Scale } from "lucide-react";

import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

import appIcon from "../../assets/app-icon.png";
import { PaneHeader, SettingsCard } from "./shared";

export function AboutPane() {
  const [version, setVersion] = useState<string>("");
  useEffect(() => {
    void getVersion()
      .then((v) => setVersion(v))
      .catch(() => setVersion(""));
  }, []);
  const openLink = (url: string) => {
    void openUrl(url).catch(() => {
      // Fallback: window.open works in dev (browser preview) when
      // the Tauri opener plugin isn't available.
      window.open(url, "_blank");
    });
  };
  return (
    <>
      <PaneHeader title="About" subtitle="Version, credits, and links." />

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
            <span className="truncate text-[12px] text-fg-2">
              Local cockpit for coding agents.
            </span>
          </div>
        </div>
      </div>

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
