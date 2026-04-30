// Settings modal — Pencil shells `hnxWB` (General default), `Wx8dI`
// (Updates), `Ohaky` (About). Sidebar nav on the left + per-pane
// content on the right.
//
// All settings persist to localStorage for now: there's no backend
// settings store yet, but the surfaces are in place so individual
// settings can land without UI churn. The "Default crew" /
// "Default working directory" pickers and update-channel /
// auto-install controls are stubbed (writes hit localStorage but no
// other surface reads them) — flagged with a "stub" hint so the
// follow-up that wires them up is obvious.
//
// Entry point: AppShell mounts a button (`Settings` link in the
// sidebar) that toggles `open`.

import { useEffect, useRef, useState } from "react";
import {
  BookText,
  Download,
  ExternalLink,
  Info,
  RefreshCw,
  Scale,
  Settings as SettingsIcon,
  X,
} from "lucide-react";

import { open as openExternal } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

type Pane = "general" | "updates" | "about";

const PANES: { key: Pane; label: string; subtitle: string; icon: typeof SettingsIcon }[] = [
  {
    key: "general",
    label: "General",
    subtitle: "Startup & defaults",
    icon: SettingsIcon,
  },
  {
    key: "updates",
    label: "Updates",
    subtitle: "Channel & auto-update",
    icon: Download,
  },
  { key: "about", label: "About", subtitle: "Version & links", icon: Info },
];

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [pane, setPane] = useState<Pane>("general");
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onMouseDown = (e: MouseEvent) => {
      if (!cardRef.current) return;
      if (!cardRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onMouseDown);
    };
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55">
      <div
        ref={cardRef}
        className="flex h-[560px] w-[680px] overflow-hidden rounded-xl border border-line bg-panel shadow-[0_14px_40px_rgba(0,0,0,0.6)]"
      >
        {/* Sidebar */}
        <aside className="flex w-[200px] shrink-0 flex-col gap-1 border-r border-line bg-bg-2 px-3 py-4">
          <div className="px-2 pb-2 text-[14px] font-bold text-fg">
            Settings
          </div>
          {PANES.map((p) => {
            const active = pane === p.key;
            const Icon = p.icon;
            return (
              <button
                key={p.key}
                type="button"
                onClick={() => setPane(p.key)}
                className={`flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors ${
                  active
                    ? "bg-accent/10 text-accent"
                    : "text-fg hover:bg-raised"
                }`}
              >
                <Icon
                  aria-hidden
                  className={`h-4 w-4 ${active ? "text-accent" : "text-fg-3"}`}
                />
                <div className="flex min-w-0 flex-col gap-px">
                  <span
                    className={`truncate text-[12px] font-medium ${
                      active ? "text-accent" : "text-fg"
                    }`}
                  >
                    {p.label}
                  </span>
                  <span
                    className={`truncate text-[10px] ${
                      active ? "text-accent/70" : "text-fg-3"
                    }`}
                  >
                    {p.subtitle}
                  </span>
                </div>
              </button>
            );
          })}
        </aside>

        {/* Content */}
        <div className="relative flex flex-1 flex-col gap-[18px] overflow-y-auto px-6 py-5">
          <button
            type="button"
            onClick={onClose}
            aria-label="Close settings"
            title="Close"
            className="absolute right-3 top-3 flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-fg-3 hover:bg-raised hover:text-fg"
          >
            <X aria-hidden className="h-4 w-4" />
          </button>
          {pane === "general" ? <GeneralPane /> : null}
          {pane === "updates" ? <UpdatesPane /> : null}
          {pane === "about" ? <AboutPane /> : null}
        </div>
      </div>
    </div>
  );
}

function PaneHeader({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div className="flex flex-col gap-1 pr-10">
      <h2 className="text-[18px] font-semibold text-fg">{title}</h2>
      <p className="text-[13px] text-fg-2">{subtitle}</p>
      <div className="mt-3 h-px w-full bg-line" />
    </div>
  );
}

// localStorage-backed boolean. Stubbed here because there's no
// backend settings store yet — we want the toggle to feel real and
// persist across reloads, even if no other surface reads the value
// today.
function useStoredBool(key: string, initial: boolean): [boolean, (v: boolean) => void] {
  const [value, setValue] = useState<boolean>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw == null ? initial : raw === "1";
    } catch {
      return initial;
    }
  });
  const set = (v: boolean) => {
    setValue(v);
    try {
      localStorage.setItem(key, v ? "1" : "0");
    } catch {
      // best-effort — Safari private mode rejects setItem; the
      // toggle still works in-session, just won't persist.
    }
  };
  return [value, set];
}

function GeneralPane() {
  const [autoStart, setAutoStart] = useStoredBool("settings.autoStartLastMission", false);
  return (
    <>
      <PaneHeader
        title="General"
        subtitle="Defaults and startup behavior."
      />
      <Row
        label="Auto-start last mission"
        sub="Resume the most recent mission when the app launches."
      >
        <Toggle on={autoStart} onChange={setAutoStart} />
      </Row>
      <Row
        label="Default crew"
        sub="Pre-selected when starting a new mission. (stub — no backend yet)"
      >
        <DisabledDropdown placeholder="Pick a crew…" />
      </Row>
      <Row
        label="Default working directory"
        sub="Cwd new chats inherit unless overridden. (stub — no backend yet)"
      >
        <DisabledDropdown placeholder="~/" mono />
      </Row>
    </>
  );
}

function UpdatesPane() {
  const [autoInstall, setAutoInstall] = useStoredBool(
    "settings.autoInstallUpdates",
    true,
  );
  const [notifyBeforeInstall, setNotifyBeforeInstall] = useStoredBool(
    "settings.notifyBeforeInstall",
    false,
  );
  const [version, setVersion] = useState<string>("");
  useEffect(() => {
    void getVersion()
      .then((v) => setVersion(v))
      .catch(() => setVersion(""));
  }, []);
  return (
    <>
      <PaneHeader
        title="Updates"
        subtitle="Stay current with the latest version."
      />
      {/* Version card */}
      <div className="flex items-center justify-between gap-4 rounded-lg border border-line bg-bg p-3.5">
        <div className="flex flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-fg">Runner</span>
            <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-[10px] text-fg-2">
              v{version || "0.0.0"}
            </span>
          </div>
          <span className="text-[11px] text-fg-2">
            Auto-update isn't wired up yet — version check is informational.
          </span>
        </div>
        <button
          type="button"
          disabled
          title="Update check stub — backend not wired up yet"
          className="flex cursor-default items-center gap-1.5 rounded-md border border-line bg-panel px-3.5 py-2 text-[12px] font-medium text-fg-2 disabled:opacity-60"
        >
          <RefreshCw aria-hidden className="h-3 w-3" />
          Check now
        </button>
      </div>
      <Row
        label="Update channel"
        sub="Stable releases only, or opt into pre-release builds. (stub)"
      >
        <DisabledDropdown placeholder="Stable" />
      </Row>
      <Row
        label="Install updates automatically"
        sub="Download and apply updates in the background. Restart needed to finish."
      >
        <Toggle on={autoInstall} onChange={setAutoInstall} />
      </Row>
      <Row
        label="Notify me before installing"
        sub="Show a banner before applying an update so you can finish your session."
      >
        <Toggle on={notifyBeforeInstall} onChange={setNotifyBeforeInstall} />
      </Row>
    </>
  );
}

function AboutPane() {
  const [version, setVersion] = useState<string>("");
  // Platform/arch label derived from navigator.userAgent — Tauri 2's
  // dedicated `@tauri-apps/plugin-os` isn't installed yet, and the
  // pill is informational only. Falls back to "desktop" if parsing
  // fails.
  const platformLabel = (() => {
    if (typeof navigator === "undefined") return "";
    const ua = navigator.userAgent.toLowerCase();
    const arch = ua.includes("arm64") || ua.includes("aarch64")
      ? "arm64"
      : ua.includes("x86_64") || ua.includes("x64")
        ? "x86_64"
        : "";
    const os = ua.includes("mac")
      ? "darwin"
      : ua.includes("win")
        ? "windows"
        : ua.includes("linux")
          ? "linux"
          : "";
    if (!os) return "";
    return arch ? `${os} · ${arch}` : os;
  })();
  useEffect(() => {
    void getVersion()
      .then((v) => setVersion(v))
      .catch(() => setVersion(""));
  }, []);
  const openLink = (url: string) => {
    void openExternal(url).catch(() => {
      // Fallback: window.open works in dev (browser preview) when
      // the Tauri shell plugin isn't available.
      window.open(url, "_blank");
    });
  };
  return (
    <>
      {/* Identity block — replaces PaneHeader because the About pane
          centers the brand block instead of using a left-aligned
          h2 + subtitle. */}
      <div className="flex flex-col items-center gap-3.5 pb-2 pt-1">
        <div className="flex h-14 w-14 items-center justify-center rounded-2xl border border-line bg-bg">
          <RunnerGlyph />
        </div>
        <div className="flex flex-col items-center gap-1.5">
          <span className="text-[20px] font-bold text-fg">Runner</span>
          <span className="text-[12px] text-fg-2">
            Desktop editor for crews of local CLI coding agents.
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded bg-raised px-2 py-0.5 font-mono text-[11px] text-fg-2">
            v{version || "0.0.0"}
          </span>
          {platformLabel ? (
            <span className="rounded bg-raised px-2 py-0.5 font-mono text-[11px] text-fg-2">
              {platformLabel}
            </span>
          ) : null}
        </div>
      </div>
      <div className="h-px w-full bg-line" />
      <div className="flex flex-col gap-px">
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
      </div>
      <div className="flex-1" />
      <div className="flex items-center justify-center text-[11px] text-fg-3">
        © 2026 wyc studios
      </div>
    </>
  );
}

function Row({
  label,
  sub,
  children,
}: {
  label: string;
  sub?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-fg">{label}</span>
        {sub ? <span className="text-[11px] text-fg-2">{sub}</span> : null}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function Toggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className={`flex h-[18px] w-8 cursor-pointer items-center rounded-full p-0.5 transition-colors ${
        on ? "justify-end bg-accent/15" : "justify-start bg-raised"
      }`}
    >
      <span
        className={`block h-3.5 w-3.5 rounded-full ${
          on ? "bg-accent" : "bg-fg-3"
        }`}
      />
    </button>
  );
}

function DisabledDropdown({
  placeholder,
  mono = false,
}: {
  placeholder: string;
  mono?: boolean;
}) {
  return (
    <div
      className={`flex cursor-not-allowed items-center gap-1.5 rounded-md border border-line bg-bg px-3 py-2 text-[12px] text-fg-3 ${
        mono ? "font-mono" : ""
      }`}
      title="Stub — backend not wired up yet"
    >
      {placeholder}
    </div>
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
      className={`flex w-full items-center justify-between rounded-md px-3 py-2.5 text-left ${
        interactive ? "cursor-pointer hover:bg-raised" : "cursor-default"
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

function RunnerGlyph() {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="text-accent"
      aria-hidden
    >
      <path d="M14 8 L22 16 L14 24" />
    </svg>
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
