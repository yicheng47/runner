// General pane — defaults and startup behavior. All settings persist
// to localStorage for now (no backend settings store yet). "Default
// working directory" is read by StartMissionModal, CreateRunnerModal,
// and the direct-chat spawn sites via the helpers in
// `src/lib/settings.ts`; "Default crew" still has no consumer
// (follow-up). The "Remember window position" row from the design
// ships with #271, not here.

import { useEffect, useState } from "react";

import { api } from "../../lib/api";
import { applyAppZoom } from "../../lib/appZoom";
import {
  readAppZoom,
  readDefaultWorkingDir,
  STORAGE_APP_ZOOM,
  writeDefaultWorkingDir,
  ZOOM_STEPS,
} from "../../lib/settings";
import { StyledSelect } from "../ui/StyledSelect";
import { WorkingDirField } from "../ui/WorkingDirField";
import { PaneHeader, SettingsCard, SettingsRow, Stepper } from "./shared";

export function GeneralPane() {
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
  // Keep the visible % in sync when zoom changes from outside the pane
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
      <SettingsCard>
        <SettingsRow
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
        </SettingsRow>
        <SettingsRow
          label="Default working directory"
          sub="Cwd new chats inherit unless overridden."
        >
          <WorkingDirField
            singleLine
            className="w-[280px]"
            value={defaultWorkingDir}
            onChange={setDefaultWorkingDir}
          />
        </SettingsRow>
        <SettingsRow
          label="App zoom"
          sub="Whole-app scale. Doesn't apply to the runner terminal canvas — see Terminal pane."
        >
          <ZoomStepper value={appZoom} onChange={setAppZoom} />
        </SettingsRow>
      </SettingsCard>
    </>
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
  // user can always move with `−`/`+`; nothing in the pane hard-blocks
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
