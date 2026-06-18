// Start Chat modal — sibling of StartMissionModal for the sidebar
// CHAT `+`. Lets the user pick a runner, optionally name the chat,
// and override the working directory before spawning a direct PTY.
//
// The runner-picker dropdown mirrors StartMissionModal's CrewPicker
// (same classes / role / outside-click ref). The Modal shell already
// owns Escape + backdrop close; only the inner dropdown needs its
// own dismiss handlers.

import { useEffect, useMemo, useRef, useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, X } from "lucide-react";

import { api, type RuntimeDefinition } from "../lib/api";
import {
  readDefaultChatRuntime,
  readDefaultWorkingDir,
} from "../lib/settings";
import type { Runner, SpawnedSession } from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";
import { StyledSelect } from "./ui/StyledSelect";

interface StartChatModalProps {
  open: boolean;
  onClose: () => void;
  /** Called after spawn (and rename if title was provided). Caller owns
   *  navigation to the spawned chat URL. */
  onStarted: (spawned: SpawnedSession) => void;
}

type ChatMode = "runner" | "runtime";
const STORAGE_START_CHAT_MODE = "runner.startChat.mode";

export function StartChatModal({
  open,
  onClose,
  onStarted,
}: StartChatModalProps) {
  const [runners, setRunners] = useState<Runner[]>([]);
  const [runtimes, setRuntimes] = useState<RuntimeDefinition[]>([]);
  const [mode, setModeState] = useState<ChatMode>(() => readStartChatMode());
  const [runnerId, setRunnerId] = useState<string>("");
  const [runtimeName, setRuntimeName] = useState<string>("");
  const [runnerPickerOpen, setRunnerPickerOpen] = useState(false);
  const [title, setTitle] = useState("");
  // Tracks whether the user has typed in the title field. While false
  // the title auto-derives from the picked runner's handle; once true
  // their text sticks even if they change the runner.
  const [titleEdited, setTitleEdited] = useState(false);
  // Synchronous mirror of `titleEdited` for closures that run *between*
  // state-set and the next render — specifically the list-load `.then()`
  // resolution and the onChange handler. We set the ref alongside every
  // setTitleEdited call instead of mirroring via a passive effect, so
  // there's no one-tick lag in either direction.
  const titleEditedRef = useRef(false);
  const [cwd, setCwd] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const runnerPickerRef = useRef<HTMLDivElement | null>(null);

  // Reset on **close**, not open — so the *first* render after `open`
  // flips back to true already paints clean state instead of flashing
  // the previous session's selection. The open-path effect below then
  // only has to drive the fetch.
  useEffect(() => {
    if (open) return;
    setRunners([]);
    setRuntimes([]);
    setRunnerId("");
    setRuntimeName("");
    setRunnerPickerOpen(false);
    setModeState(readStartChatMode());
    setTitle("");
    setTitleEdited(false);
    titleEditedRef.current = false;
    setCwd("");
    setError(null);
    setSubmitting(false);
  }, [open]);

  // Open-path: kick off the runner-list fetch and seed runnerId / title
  // atomically when it lands. State is already clean from the close-path
  // reset, so we don't repeat those resets here. The `cancelled` flag
  // closes a stale-write race: if the user opens then closes (or
  // reopens) before the promise resolves, the late `.then()` would
  // otherwise undo the close-path's wipe and flash prior state.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void api.runner
      .list()
      .then((rows) => {
        if (cancelled) return;
        const first = rows[0] ?? null;
        setRunners(rows);
        setRunnerId(first?.id ?? "");
        // Atomic initial title fill — the ref is the authoritative
        // signal here, since a keystroke during the pre-load window
        // would have flipped it true synchronously.
        if (first && mode === "runner" && !titleEditedRef.current) {
          setTitle(defaultTitleFor(first));
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, mode]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void api.runtime
      .list()
      .then((rows) => {
        if (cancelled) return;
        const preferred = readDefaultChatRuntime();
        const selected =
          rows.find((runtime) => runtime.name === preferred) ?? rows[0] ?? null;
        setRuntimes(rows);
        setRuntimeName(selected?.name ?? "");
        if (selected && !titleEditedRef.current && mode === "runtime") {
          setTitle(defaultTitleForRuntime(selected));
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, mode]);

  // Inner dropdown's own dismiss handlers. The Modal shell handles
  // Escape/backdrop for the dialog itself; this scopes to the
  // popover only.
  useEffect(() => {
    if (!runnerPickerOpen) return;
    const onPointerDown = (e: MouseEvent) => {
      if (!runnerPickerRef.current?.contains(e.target as Node)) {
        setRunnerPickerOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setRunnerPickerOpen(false);
    };
    window.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [runnerPickerOpen]);

  const selectedRunner = useMemo(
    () => runners.find((r) => r.id === runnerId) ?? null,
    [runners, runnerId],
  );
  const selectedRuntime = useMemo(
    () => runtimes.find((runtime) => runtime.name === runtimeName) ?? null,
    [runtimes, runtimeName],
  );

  const setMode = (next: ChatMode) => {
    setModeState(next);
    writeStartChatMode(next);
    if (titleEditedRef.current) return;
    if (next === "runner") setTitle(defaultTitleFor(selectedRunner));
    else setTitle(defaultTitleForRuntime(selectedRuntime));
  };

  // Follow runner-picker changes: while the user hasn't typed in the
  // title field, re-derive the title from the currently-picked runner.
  // The *initial* fill lives in the open-effect's list-load path
  // (atomic with picking the first runner); this effect only handles
  // subsequent user-driven picks.
  useEffect(() => {
    if (!open) return;
    if (titleEdited) return;
    if (mode === "runner") setTitle(defaultTitleFor(selectedRunner));
    else setTitle(defaultTitleForRuntime(selectedRuntime));
  }, [open, mode, selectedRunner, selectedRuntime, titleEdited]);

  const browseCwd = async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Pick a working directory",
      });
      if (typeof picked === "string") setCwd(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const start = async () => {
    if (mode === "runner" && !selectedRunner) return;
    if (mode === "runtime" && !selectedRuntime) return;
    setSubmitting(true);
    setError(null);
    try {
      // Effective cwd precedence (matches Runners.tsx / RunnerDetail.tsx
      // + the explicit-override extension):
      //   user typed value  → use as-is
      //   else runner has its own working_dir → null (backend uses
      //                                         the runner's dir)
      //   else readDefaultWorkingDir() or null
      // The input is left blank by default and the placeholder shows
      // what blank-leave will produce, so we don't have to thread a
      // separate "edited" flag for this field.
      const trimmedCwd = cwd.trim();
      const spawned =
        mode === "runner" && selectedRunner
          ? await api.session.startDirect(
              selectedRunner.id,
              trimmedCwd.length > 0
                ? trimmedCwd
                : selectedRunner.working_dir
                  ? null
                  : (readDefaultWorkingDir() || null),
              null,
              null,
            )
          : await api.session.startRuntime(
              selectedRuntime!.name,
              trimmedCwd.length > 0 ? trimmedCwd : (readDefaultWorkingDir() || null),
              null,
              null,
            );
      const trimmedTitle = title.trim();
      if (trimmedTitle.length > 0) {
        try {
          await api.session.rename(spawned.id, trimmedTitle);
        } catch (e) {
          // The chat is already spawned; sidebar's context menu can
          // rename it later. Don't block navigation on a rename hiccup.
          console.error("StartChatModal: session_rename failed", e);
        }
      }
      onStarted(spawned);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={submitting ? () => {} : onClose}
      title={
        <div className="flex items-center justify-between gap-4">
          <div className="flex flex-col gap-0.5">
            <span className="text-base font-semibold text-fg">
              Start a chat
            </span>
            <span className="text-xs font-normal text-fg-2">
              Spawns a direct PTY in the selected directory.
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-50"
            aria-label="Close start chat"
          >
            <X aria-hidden className="h-3.5 w-3.5" />
          </button>
        </div>
      }
      widthClass="w-full max-w-[560px]"
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => void start()}
            disabled={
              submitting ||
              (mode === "runner" && (!runnerId || runners.length === 0)) ||
              (mode === "runtime" && (!runtimeName || runtimes.length === 0))
            }
          >
            {submitting ? "Starting…" : "Start chat"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-5">
        {error ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
            {error}
          </div>
        ) : null}

        <div className="grid grid-cols-2 rounded-md border border-line bg-bg p-0.5">
          {(["runner", "runtime"] as const).map((option) => (
            <button
              key={option}
              type="button"
              role="tab"
              aria-selected={mode === option}
              onClick={() => setMode(option)}
              disabled={submitting}
              className={`cursor-pointer rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:cursor-default disabled:opacity-60 ${
                mode === option
                  ? "bg-raised text-fg"
                  : "text-fg-2 hover:text-fg"
              }`}
            >
              {option === "runner" ? "Runner" : "Direct"}
            </button>
          ))}
        </div>

        {mode === "runner" ? (
          <Field label="Runner">
            <div ref={runnerPickerRef} className="relative">
              <button
                type="button"
                disabled={submitting || runners.length === 0}
                onClick={() => setRunnerPickerOpen((v) => !v)}
                className="flex w-full cursor-pointer items-center gap-3 rounded-md border border-line bg-bg px-3 py-2.5 text-left transition-colors hover:border-line-strong focus:border-fg-3 focus:outline-none disabled:cursor-default disabled:opacity-60"
                aria-haspopup="listbox"
                aria-expanded={runnerPickerOpen}
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-[13px] font-semibold text-fg">
                    {selectedRunner ? `@${selectedRunner.handle}` : "No runners yet"}
                  </span>
                  <span className="block truncate text-[11px] text-fg-2">
                    {summarizeRunner(selectedRunner)}
                  </span>
                </span>
                <ChevronDown aria-hidden className="h-3.5 w-3.5 text-fg-3" />
              </button>
              {runnerPickerOpen ? (
                <div
                  role="listbox"
                  className="absolute left-0 right-0 top-full z-30 mt-1 max-h-56 overflow-y-auto rounded-md border border-line bg-panel p-1 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
                >
                  {runners.map((r) => (
                    <button
                      key={r.id}
                      type="button"
                      role="option"
                      aria-selected={r.id === runnerId}
                      onClick={() => {
                        setRunnerId(r.id);
                        setRunnerPickerOpen(false);
                      }}
                      className={`flex w-full cursor-pointer items-center justify-between gap-3 rounded px-2.5 py-2 text-left transition-colors hover:bg-raised ${
                        r.id === runnerId ? "bg-raised" : ""
                      }`}
                    >
                      <span className="min-w-0">
                        <span className="block truncate font-mono text-[13px] font-semibold text-fg">
                          @{r.handle}
                        </span>
                        <span className="block truncate text-[11px] text-fg-2">
                          {summarizeRunner(r)}
                        </span>
                      </span>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
            {runners.length === 0 ? (
              <p className="mt-1 text-[11px] text-warn">
                No runners yet. Create one from the runner page first.
              </p>
            ) : null}
          </Field>
        ) : (
          <Field label="Agent runtime">
            <StyledSelect
              value={runtimeName}
              options={runtimes.map((runtime) => ({
                value: runtime.name,
                label: runtime.display_name,
              }))}
              onChange={(next) => setRuntimeName(next)}
            />
          </Field>
        )}

        <Field
          label="Chat name"
          subtitle="Optional. Leave blank to use the default label."
        >
          <input
            value={title}
            onChange={(e) => {
              setTitle(e.target.value);
              titleEditedRef.current = true;
              setTitleEdited(true);
            }}
            placeholder="e.g. quick-debug"
            disabled={submitting}
            className="rounded-md border border-line bg-bg px-3 py-2 text-[13px] text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>

        <Field label="Working directory">
          <div className="flex items-center gap-2">
            <input
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              placeholder={
                mode === "runner"
                  ? cwdPlaceholderFor(selectedRunner)
                  : cwdPlaceholderForRuntime()
              }
              disabled={submitting}
              className="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
            />
            <Button onClick={() => void browseCwd()} disabled={submitting}>
              Browse…
            </Button>
          </div>
          <p className="text-[11px] text-fg-2">
            Leave blank to use the default working directory.
          </p>
        </Field>
      </div>
    </Modal>
  );
}

function Field({
  label,
  subtitle,
  children,
}: {
  label: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-semibold text-fg">{label}</span>
      {children}
      {subtitle ? (
        <span className="text-[11px] text-fg-3">{subtitle}</span>
      ) : null}
    </label>
  );
}

function summarizeRunner(runner: Runner | null): string {
  if (!runner) return "Create a runner first.";
  const wd = runner.working_dir ?? "no working dir";
  return `${runner.runtime} · ${wd}`;
}

// Default Chat name when the user hasn't typed anything. Uses the
// runner's `handle` (not `display_name`) so the title mirrors the URL
// path the chat will land at.
function defaultTitleFor(runner: Runner | null): string {
  if (!runner) return "";
  return `Chat with @${runner.handle}`;
}

function defaultTitleForRuntime(runtime: RuntimeDefinition | null): string {
  if (!runtime) return "";
  return `Chat with ${runtime.display_name}`;
}

// Dynamic placeholder for the working-directory input. Shows what
// blank-leave will produce: the runner's own working_dir if set,
// else the global settings default, else a parenthetical hint that
// no directory will be passed.
function cwdPlaceholderFor(runner: Runner | null): string {
  if (runner?.working_dir) return runner.working_dir;
  const fallback = readDefaultWorkingDir();
  if (fallback) return fallback;
  return "(no working directory)";
}

function cwdPlaceholderForRuntime(): string {
  const fallback = readDefaultWorkingDir();
  if (fallback) return fallback;
  return "(no working directory)";
}

function readStartChatMode(): ChatMode {
  try {
    return localStorage.getItem(STORAGE_START_CHAT_MODE) === "runtime"
      ? "runtime"
      : "runner";
  } catch {
    return "runner";
  }
}

function writeStartChatMode(mode: ChatMode): void {
  try {
    localStorage.setItem(STORAGE_START_CHAT_MODE, mode);
  } catch {
    // best-effort
  }
}
