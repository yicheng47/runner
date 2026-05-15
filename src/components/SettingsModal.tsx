// Settings modal — Pencil shells `hnxWB` (General default), `Wx8dI`
// (Updates), `Ohaky` (About). Sidebar nav on the left + per-pane
// content on the right.
//
// All settings persist to localStorage for now: there's no backend
// settings store yet, but the surfaces are in place so individual
// settings can land without UI churn. "Default working directory"
// is read by StartMissionModal, CreateRunnerModal, and the direct-
// chat spawn sites via the helpers in `src/lib/settings.ts`;
// "Default crew" still has no consumer (follow-up).
//
// Entry point: AppShell mounts a button (`Settings` link in the
// sidebar) that toggles `open`.

import { useEffect, useRef, useState } from "react";
import {
  BookText,
  Download,
  ExternalLink,
  Info,
  Loader2,
  Minus,
  Plus,
  RefreshCw,
  RotateCcw,
  Scale,
  Settings as SettingsIcon,
  Terminal,
  X,
} from "lucide-react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

import { api } from "../lib/api";
import { applyAppZoom } from "../lib/appZoom";
import {
  notifySameWindowStorage,
  readAppZoom,
  readDefaultWorkingDir,
  readStoredBool,
  readTerminalCursorStyle,
  readTerminalFontFamily,
  readTerminalFontSize,
  readTerminalScrollback,
  readTerminalTheme,
  STORAGE_APP_ZOOM,
  STORAGE_AUTO_INSTALL_UPDATES,
  STORAGE_TERMINAL_CURSOR_STYLE,
  STORAGE_TERMINAL_FONT_FAMILY,
  STORAGE_TERMINAL_FONT_SIZE,
  STORAGE_TERMINAL_SCROLLBACK,
  STORAGE_TERMINAL_THEME,
  TERMINAL_CURSOR_STYLE_OPTIONS,
  TERMINAL_FONT_FAMILY_OPTIONS,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  TERMINAL_SCROLLBACK_OPTIONS,
  TERMINAL_THEME_LABELS,
  TERMINAL_THEME_OPTIONS,
  type TerminalCursorStyle,
  type TerminalFontFamily,
  type TerminalTheme,
  writeDefaultWorkingDir,
  writeStoredBool,
  writeTerminalCursorStyle,
  writeTerminalFontFamily,
  writeTerminalFontSize,
  writeTerminalScrollback,
  writeTerminalTheme,
  ZOOM_STEPS,
} from "../lib/settings";
import { useUpdate } from "../contexts/UpdateContext";
import { Button } from "./ui/Button";
import { StyledSelect } from "./ui/StyledSelect";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

type Pane = "general" | "terminal" | "updates" | "about";

const PANES: { key: Pane; label: string; subtitle: string; icon: typeof SettingsIcon }[] = [
  {
    key: "general",
    label: "General",
    subtitle: "Startup & defaults",
    icon: SettingsIcon,
  },
  {
    key: "terminal",
    label: "Terminal",
    subtitle: "xterm appearance",
    icon: Terminal,
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
          {pane === "terminal" ? <TerminalPane /> : null}
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
  // Thin wrapper around the shared `lib/settings` helpers so the
  // modal and any non-React reader (e.g. UpdateContext) can't drift
  // on encoding. Both sides go through `readStoredBool` /
  // `writeStoredBool`.
  const [value, setValue] = useState<boolean>(() => readStoredBool(key, initial));
  const set = (v: boolean) => {
    setValue(v);
    writeStoredBool(key, v);
  };
  return [value, set];
}

function GeneralPane() {
  // Default crew selector. Persisted to localStorage today (no
  // backend settings store yet); the StartMissionModal can read the
  // same key to pre-fill its crew picker once that wiring lands.
  const [crews, setCrews] = useState<{ id: string; name: string }[]>([]);
  const [defaultCrewId, setDefaultCrewIdState] = useState<string>(() => {
    try {
      return localStorage.getItem("settings.defaultCrewId") ?? "";
    } catch {
      return "";
    }
  });
  // Default working directory. Picked via Tauri's dialog plugin
  // (open({ directory: true })) so the value is always an absolute
  // path the OS confirmed exists.
  const [defaultWorkingDir, setDefaultWorkingDirState] = useState<string>(
    () => readDefaultWorkingDir(),
  );
  useEffect(() => {
    let cancelled = false;
    void api.crew
      .list()
      .then((rows) => {
        if (cancelled) return;
        setCrews(rows.map((c) => ({ id: c.id, name: c.name })));
      })
      .catch(() => {
        // best-effort — leave the dropdown empty if the list query
        // fails; the user can retry by reopening Settings.
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const setDefaultCrewId = (id: string) => {
    setDefaultCrewIdState(id);
    try {
      if (id) localStorage.setItem("settings.defaultCrewId", id);
      else localStorage.removeItem("settings.defaultCrewId");
    } catch {
      // best-effort
    }
  };
  const setDefaultWorkingDir = (path: string) => {
    setDefaultWorkingDirState(path);
    writeDefaultWorkingDir(path);
  };
  // App zoom — snap-to-step value driven by `ZOOM_STEPS`. Persist + apply
  // immediately so the user feels the change while picking. The boot-time
  // apply in `App.tsx` is what makes it survive restarts. Goes through
  // the shared `applyAppZoom` so the stepper and the global Cmd+/- path
  // can't drift.
  const [appZoom, setAppZoomState] = useState<number>(() => readAppZoom());
  const setAppZoom = (next: number) => {
    setAppZoomState(next);
    applyAppZoom(next);
  };
  // Keep the visible % in sync when zoom changes from outside the modal
  // (Cmd+/-/0 shortcut). `applyAppZoom` synthesizes a storage event after
  // each write so we get a single notification path.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_APP_ZOOM) return;
      setAppZoomState(readAppZoom());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return (
    <>
      <PaneHeader title="General" subtitle="Defaults and startup behavior." />
      <Row
        label="Default crew"
        sub="Pre-selected when starting a new mission."
      >
        <StyledSelect
          value={defaultCrewId}
          options={[
            { value: "", label: "No default" },
            ...crews.map((c) => ({ value: c.id, label: c.name })),
          ]}
          onChange={setDefaultCrewId}
        />
      </Row>
      <Row
        label="Default working directory"
        sub="Cwd new chats inherit unless overridden."
      >
        <WorkingDirInput
          value={defaultWorkingDir}
          onChange={setDefaultWorkingDir}
        />
      </Row>
      <Row
        label="App zoom"
        sub="Whole-app scale. Doesn't apply to the runner terminal canvas — see Terminal pane."
      >
        <ZoomStepper value={appZoom} onChange={setAppZoom} />
      </Row>
    </>
  );
}

function TerminalPane() {
  const [fontSize, setFontSizeState] = useState<number>(() =>
    readTerminalFontSize(),
  );
  const [fontFamily, setFontFamilyState] = useState<TerminalFontFamily>(() =>
    readTerminalFontFamily(),
  );
  const [cursorStyle, setCursorStyleState] = useState<TerminalCursorStyle>(
    () => readTerminalCursorStyle(),
  );
  const [scrollback, setScrollbackState] = useState<number>(() =>
    readTerminalScrollback(),
  );
  const [theme, setThemeState] = useState<TerminalTheme>(() =>
    readTerminalTheme(),
  );
  const setFontSize = (next: number) => {
    setFontSizeState(next);
    writeTerminalFontSize(next);
    notifySameWindowStorage(STORAGE_TERMINAL_FONT_SIZE, String(next));
  };
  const setFontFamily = (next: TerminalFontFamily) => {
    setFontFamilyState(next);
    writeTerminalFontFamily(next);
    notifySameWindowStorage(STORAGE_TERMINAL_FONT_FAMILY, next);
  };
  const setCursorStyle = (next: TerminalCursorStyle) => {
    setCursorStyleState(next);
    writeTerminalCursorStyle(next);
    notifySameWindowStorage(STORAGE_TERMINAL_CURSOR_STYLE, next);
  };
  const setScrollback = (next: number) => {
    setScrollbackState(next);
    writeTerminalScrollback(next);
    notifySameWindowStorage(STORAGE_TERMINAL_SCROLLBACK, String(next));
  };
  const setTheme = (next: TerminalTheme) => {
    setThemeState(next);
    writeTerminalTheme(next);
    notifySameWindowStorage(STORAGE_TERMINAL_THEME, next);
  };
  return (
    <>
      <PaneHeader
        title="Terminal"
        subtitle="xterm appearance settings for the runner terminal."
      />
      <Row
        label="Theme"
        sub="ANSI palette for the embedded terminal. Background stays locked to app chrome."
      >
        <StyledSelect
          value={theme}
          options={TERMINAL_THEME_OPTIONS.map((id) => ({
            value: id,
            label: TERMINAL_THEME_LABELS[id],
          }))}
          onChange={(v) => setTheme(v as TerminalTheme)}
        />
      </Row>
      <Row
        label="Font family"
        sub="Typeface used by the embedded terminal."
      >
        <StyledSelect
          value={fontFamily}
          options={TERMINAL_FONT_FAMILY_OPTIONS.map((f) => ({
            value: f,
            label: f,
          }))}
          onChange={(v) => setFontFamily(v as TerminalFontFamily)}
        />
      </Row>
      <Row
        label="Terminal font size"
        sub="Glyph size for the embedded terminal."
      >
        <FontSizeStepper value={fontSize} onChange={setFontSize} />
      </Row>
      <Row
        label="Cursor style"
        sub="Block, underline, or bar — affects the prompt caret only."
      >
        <StyledSelect
          value={cursorStyle}
          options={TERMINAL_CURSOR_STYLE_OPTIONS.map((c) => ({
            value: c,
            label: c[0].toUpperCase() + c.slice(1),
          }))}
          onChange={(v) => setCursorStyle(v as TerminalCursorStyle)}
        />
      </Row>
      <Row
        label="Scrollback"
        sub="Lines kept in history per session. Higher uses more memory."
      >
        <div className="flex items-center gap-2">
          <StyledSelect
            value={String(scrollback)}
            options={TERMINAL_SCROLLBACK_OPTIONS.map((n) => ({
              value: String(n),
              label: n.toLocaleString(),
            }))}
            onChange={(v) => setScrollback(Number.parseInt(v, 10))}
          />
          <span className="text-[12px] text-fg-2">lines</span>
        </div>
      </Row>
    </>
  );
}

// Generic [−] <value> [+] stepper matching pencil nodes `KfYfw` and `CcFty`.
// Caller renders the value cell's contents and supplies its width.
function Stepper({
  valueCellWidth,
  decDisabled,
  incDisabled,
  onDec,
  onInc,
  decAriaLabel,
  incAriaLabel,
  children,
}: {
  valueCellWidth: number;
  decDisabled?: boolean;
  incDisabled?: boolean;
  onDec: () => void;
  onInc: () => void;
  decAriaLabel: string;
  incAriaLabel: string;
  children: React.ReactNode;
}) {
  const buttonClass =
    "flex h-[30px] w-[30px] shrink-0 cursor-pointer items-center justify-center text-fg-3 transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:text-fg-3";
  return (
    <div className="flex h-[30px] items-center rounded-md border border-line bg-bg">
      <button
        type="button"
        onClick={onDec}
        disabled={decDisabled}
        aria-label={decAriaLabel}
        className={buttonClass}
      >
        <Minus aria-hidden className="h-3.5 w-3.5" />
      </button>
      <div
        style={{ width: valueCellWidth }}
        className="flex h-[30px] items-center justify-center border-x border-line"
      >
        {children}
      </div>
      <button
        type="button"
        onClick={onInc}
        disabled={incDisabled}
        aria-label={incAriaLabel}
        className={buttonClass}
      >
        <Plus aria-hidden className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function ZoomStepper({
  value,
  onChange,
}: {
  value: number;
  onChange: (v: number) => void;
}) {
  // Snap a possibly-stale persisted value to the nearest known step so the
  // user can always move with `−`/`+`; nothing in the modal hard-blocks
  // off-step values.
  const idx = ZOOM_STEPS.findIndex((s) => Math.abs(s - value) < 0.001);
  const currentIdx = idx === -1 ? ZOOM_STEPS.indexOf(1.0) : idx;
  const pct = Math.round(ZOOM_STEPS[currentIdx] * 100);
  return (
    <Stepper
      valueCellWidth={56}
      decDisabled={currentIdx <= 0}
      incDisabled={currentIdx >= ZOOM_STEPS.length - 1}
      decAriaLabel="Decrease zoom"
      incAriaLabel="Increase zoom"
      onDec={() => onChange(ZOOM_STEPS[Math.max(0, currentIdx - 1)])}
      onInc={() =>
        onChange(ZOOM_STEPS[Math.min(ZOOM_STEPS.length - 1, currentIdx + 1)])
      }
    >
      <span className="font-mono text-[12px] font-medium text-fg">{pct}%</span>
    </Stepper>
  );
}

function FontSizeStepper({
  value,
  onChange,
}: {
  value: number;
  onChange: (v: number) => void;
}) {
  const clamped = Math.max(
    TERMINAL_FONT_SIZE_MIN,
    Math.min(TERMINAL_FONT_SIZE_MAX, value),
  );
  return (
    <Stepper
      valueCellWidth={64}
      decDisabled={clamped <= TERMINAL_FONT_SIZE_MIN}
      incDisabled={clamped >= TERMINAL_FONT_SIZE_MAX}
      decAriaLabel="Decrease terminal font size"
      incAriaLabel="Increase terminal font size"
      onDec={() => onChange(Math.max(TERMINAL_FONT_SIZE_MIN, clamped - 1))}
      onInc={() => onChange(Math.min(TERMINAL_FONT_SIZE_MAX, clamped + 1))}
    >
      <span className="flex items-center gap-[3px]">
        <span className="font-mono text-[12px] font-medium text-fg">
          {clamped}
        </span>
        <span className="font-mono text-[10px] text-fg-3">px</span>
      </span>
    </Stepper>
  );
}

function UpdatesPane() {
  const [autoInstall, setAutoInstall] = useStoredBool(
    STORAGE_AUTO_INSTALL_UPDATES,
    true,
  );
  const [version, setVersion] = useState<string>("");
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
  // Status-driven copy + action — mirrors the design's three active
  // panes (`pYv9W` available, `u4odWB` downloading, `KVWlJ` ready).
  // The same row swaps into each state so users always see ONE
  // current action.
  const sub = (() => {
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
  const subTone =
    status === "available" || status === "ready"
      ? "text-accent"
      : "text-fg-2";
  return (
    <>
      <PaneHeader
        title="Updates"
        subtitle="Stay current with the latest version."
      />
      {/* Version card — same shell, different right-hand action per state. */}
      <div className="flex items-center justify-between gap-4 rounded-lg border border-line bg-bg p-3.5">
        <div className="flex min-w-0 flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-fg">Runner</span>
            <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-[10px] text-fg-2">
              v{version || "0.0.0"}
            </span>
          </div>
          <span className={`truncate text-[11px] ${subTone}`}>{sub}</span>
        </div>
        <UpdatesAction
          status={status}
          onCheck={() => void checkForUpdate()}
          onDownload={() => void downloadAndInstall()}
          onRestart={() => void restart()}
        />
      </div>
      {status === "downloading" ? (
        <div className="-mt-2 h-[3px] w-full overflow-hidden rounded-full bg-raised">
          <div
            className="h-full rounded-full bg-accent transition-[width] duration-200"
            style={{ width: `${progress}%` }}
          />
        </div>
      ) : null}
      <Row
        label="Install updates automatically"
        sub="Download and apply updates in the background. Restart needed to finish."
      >
        <Toggle on={autoInstall} onChange={setAutoInstall} />
      </Row>
    </>
  );
}

function UpdatesAction({
  status,
  onCheck,
  onDownload,
  onRestart,
}: {
  status: ReturnType<typeof useUpdate>["status"];
  onCheck: () => void;
  onDownload: () => void;
  onRestart: () => void;
}) {
  // No button slot during download — the inline progress bar is the
  // affordance. Cancel isn't supported by the plugin (no abort
  // handle), so we don't pretend to offer one.
  if (status === "checking") {
    return (
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-panel px-3 py-1.5 text-[12px] font-medium text-fg-2">
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
        className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-bg transition-colors hover:bg-accent/90"
      >
        <Download aria-hidden className="h-3 w-3" />
        Download &amp; install
      </button>
    );
  }
  if (status === "downloading") {
    return (
      <div className="flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-panel px-3 py-1.5 text-[12px] font-medium text-fg-2">
        <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
        Installing
      </div>
    );
  }
  if (status === "ready") {
    return (
      <button
        type="button"
        onClick={onRestart}
        className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-bg transition-colors hover:bg-accent/90"
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
      className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-panel px-3 py-1.5 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg"
    >
      <RefreshCw aria-hidden className="h-3 w-3" />
      Check now
    </button>
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
    void openUrl(url).catch(() => {
      // Fallback: window.open works in dev (browser preview) when
      // the Tauri opener plugin isn't available.
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

// Typed input + Browse… button — matches the working-directory
// control in StartMissionModal / CreateRunnerModal so the three
// surfaces feel like one family. Typing an empty string flows
// through `writeDefaultWorkingDir` (which removes the key); the
// Browse button overlays Tauri's native directory dialog.
function WorkingDirInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (path: string) => void;
}) {
  const [picking, setPicking] = useState(false);
  const choose = async () => {
    if (picking) return;
    setPicking(true);
    try {
      const result = await openDialog({
        directory: true,
        multiple: false,
        defaultPath: value || undefined,
      });
      if (typeof result === "string" && result) {
        onChange(result);
      }
    } catch {
      // best-effort — the dialog plugin can throw on backend mis-
      // configuration; cancel is silent rather than a stack trace.
    } finally {
      setPicking(false);
    }
  };
  return (
    <div className="flex w-[260px] items-center gap-2">
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="/Users/you/projects/foo"
        className="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
      />
      <Button onClick={() => void choose()} disabled={picking}>
        Browse…
      </Button>
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
