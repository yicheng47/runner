import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";

import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import {
  api,
  type RuntimeExecutableStatus,
  type RuntimeDiscoveryOutcome,
  type RuntimeOverrideError,
  type RuntimeRowState,
  type RuntimeShellStatus,
  type RuntimeStatusResponse,
} from "../../lib/api";
import {
  readDisabledAgents,
  readDefaultRuntime,
  writeAgentEnabled,
  writeDefaultRuntime,
} from "../../lib/settings";
import { StyledSelect } from "../ui/StyledSelect";
import { Toggle } from "../ui/Toggle";
import { RUNTIME_OPTIONS } from "../ui/runtimes";
import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function AgentsPane() {
  const [status, setStatus] = useState<RuntimeStatusResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [defaultRuntime, setDefaultRuntimeState] = useState(() =>
    readDefaultRuntime(),
  );
  const [disabledAgents, setDisabledAgents] = useState(() =>
    readDisabledAgents(),
  );
  const availableAgents = useMemo(
    () =>
      (status?.runtimes ?? []).filter(
        (runtime) =>
          (runtime.effective_source === "detected" ||
            runtime.effective_source === "override") &&
          !disabledAgents.has(runtime.name),
      ),
    [disabledAgents, status],
  );

  const load = useCallback(async () => {
    try {
      setStatus(await api.runtime.status());
      setError(null);
    } catch (loadError) {
      setError(errorMessage(loadError));
    }
  }, []);

  useEffect(() => {
    void load();
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen("runtime/changed", () => {
      if (!disposed) void load();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [load]);

  useEffect(() => {
    if (!status) return;
    const stored = readDefaultRuntime();
    const disabled = disabledAgents.has(stored);
    const unavailable =
      !status.shell.checking &&
      !availableAgents.some((runtime) => runtime.name === stored);
    if (stored && (disabled || unavailable)) {
      setDefaultRuntimeState("");
      writeDefaultRuntime("");
    }
  }, [availableAgents, disabledAgents, status]);

  const setDefaultRuntime = (runtime: string) => {
    setDefaultRuntimeState(runtime);
    writeDefaultRuntime(runtime);
  };

  const setAgentEnabled = (agent: string, enabled: boolean) => {
    writeAgentEnabled(agent, enabled);
    setDisabledAgents((current) => {
      const next = new Set(current);
      if (enabled) next.delete(agent);
      else next.add(agent);
      return next;
    });
    if (!enabled && defaultRuntime === agent) {
      setDefaultRuntimeState("");
      writeDefaultRuntime("");
    }
  };

  const refresh = async () => {
    if (refreshing || status?.shell.checking) return;
    setRefreshing(true);
    try {
      setStatus(await api.runtime.refresh());
      setError(null);
    } catch (refreshError) {
      setError(errorMessage(refreshError));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <>
      <PaneHeader
        title="Agents"
        subtitle="Discover, enable, and override built-in agent executables."
      />
      <SettingsCard>
        <SettingsRow
          label="Default agent"
          sub="Pre-selected when starting a direct chat in Direct mode."
        >
          <StyledSelect
            id="agents-default-runtime"
            value={defaultRuntime}
            options={[
              { value: "", label: "First available" },
              ...availableAgents.map((runtime) => ({
                value: runtime.name,
                label: runtime.display_name,
              })),
            ]}
            onChange={setDefaultRuntime}
          />
        </SettingsRow>
      </SettingsCard>
      <ShellEnvironmentCard
        shell={status?.shell ?? null}
        refreshing={refreshing}
        onRefresh={() => void refresh()}
      />
      <div className="flex flex-col divide-y divide-line overflow-hidden rounded-xl border border-line bg-panel">
        {status?.runtimes.map((runtime) => (
          <RuntimeRow
            key={runtime.name}
            runtime={runtime}
            enabled={!disabledAgents.has(runtime.name)}
            probeOutcome={status.shell.outcome}
            onStatus={setStatus}
            onEnabledChange={(enabled) =>
              setAgentEnabled(runtime.name, enabled)
            }
          />
        )) ??
          RUNTIME_OPTIONS.map((runtime) => (
            <div
              key={runtime.value}
              className="h-[108px] animate-pulse bg-raised/20 px-5 py-4"
            />
          ))}
      </div>
      <p className="text-[12px] leading-[1.5] text-fg-3">
        Disabled agents stay configured but are hidden from agent pickers.
        Overrides apply to new sessions that use the agent&apos;s default command;
        runners with a custom command keep it.
      </p>
      {error ? (
        <div className="flex items-start justify-between gap-3 rounded-xl border border-red-500/30 bg-red-500/10 px-4 py-3">
          <p className="min-w-0 text-[12px] text-red-300">{error}</p>
          <button
            type="button"
            onClick={() => void load()}
            className="shrink-0 cursor-pointer text-[12px] font-medium text-red-200 underline"
          >
            Retry
          </button>
        </div>
      ) : null}
    </>
  );
}

function ShellEnvironmentCard({
  shell,
  refreshing,
  onRefresh,
}: {
  shell: RuntimeShellStatus | null;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const checking = refreshing || shell?.checking;
  const description = shellDescription(shell);
  return (
    <div className="rounded-xl border border-line bg-panel px-5 py-4">
      <div className="flex items-center justify-between gap-8">
        <div className="min-w-0">
          <div className="text-[13px] font-semibold text-fg">
            Shell environment
          </div>
          <p className="mt-0.5 text-[12px] leading-[1.45] text-fg-2">
            {description}
          </p>
        </div>
        <button
          type="button"
          disabled={Boolean(checking)}
          onClick={onRefresh}
          className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
        >
          {checking ? (
            <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw aria-hidden className="h-3 w-3" />
          )}
          {checking ? "Checking…" : "Refresh"}
        </button>
      </div>
    </div>
  );
}

function RuntimeRow({
  runtime,
  enabled,
  probeOutcome,
  onStatus,
  onEnabledChange,
}: {
  runtime: RuntimeExecutableStatus;
  enabled: boolean;
  probeOutcome: RuntimeDiscoveryOutcome | null;
  onStatus: (status: RuntimeStatusResponse) => void;
  onEnabledChange: (enabled: boolean) => void;
}) {
  const [draft, setDraft] = useState(runtime.override_path ?? "");
  const [validation, setValidation] = useState<string | null>(
    runtime.invalid_reason,
  );
  const [saving, setSaving] = useState(false);
  const [focused, setFocused] = useState(false);
  const cancelBlurRef = useRef(false);

  useEffect(() => {
    if (focused || saving) return;
    setDraft(runtime.override_path ?? "");
    setValidation(runtime.invalid_reason);
  }, [
    focused,
    runtime.invalid_reason,
    runtime.override_path,
    saving,
  ]);

  const commit = async (nextValue: string) => {
    if (saving) return;
    const next = nextValue.trim();
    if (next === (runtime.override_path ?? "")) {
      setValidation(runtime.invalid_reason);
      return;
    }
    setSaving(true);
    setValidation(null);
    try {
      const nextStatus = next
        ? await api.runtime.setOverride(runtime.name, next)
        : await api.runtime.clearOverride(runtime.name);
      onStatus(nextStatus);
      setDraft(
        nextStatus.runtimes.find((row) => row.name === runtime.name)
          ?.override_path ?? "",
      );
    } catch (saveError) {
      setValidation(errorMessage(saveError));
    } finally {
      setSaving(false);
    }
  };

  const browse = async () => {
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        title: `Choose ${runtime.display_name} executable`,
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      setDraft(path);
      await commit(path);
    } catch (browseError) {
      setValidation(errorMessage(browseError));
    }
  };

  const reset = async () => {
    if (saving) return;
    setSaving(true);
    setValidation(null);
    try {
      const nextStatus = await api.runtime.clearOverride(runtime.name);
      onStatus(nextStatus);
      setDraft("");
    } catch (clearError) {
      setValidation(errorMessage(clearError));
    } finally {
      setSaving(false);
    }
  };

  const state: RuntimeRowState = validation
    ? "invalid_override"
    : runtime.state;
  const autoPath =
    runtime.state === "checking"
      ? "Auto — detecting…"
      : runtime.detected_path
        ? `Auto — ${runtime.detected_path}`
      : `Auto — ${runtime.command} not found on PATH`;
  const showReset = Boolean(runtime.override_path || draft);

  return (
    <div className="flex flex-col gap-3 px-5 py-4">
      <div className="flex items-center gap-2.5">
        <span className="text-[13px] font-semibold text-fg">
          {runtime.display_name}
        </span>
        <RuntimeBadge
          state={state}
          probeOutcome={probeOutcome}
          saving={saving}
        />
        <span className="min-w-0 flex-1" />
        <span className="font-mono text-[11px] text-fg-3">
          {runtime.command}
        </span>
        <span className="text-[11px] text-fg-3">
          {enabled ? "Enabled" : "Disabled"}
        </span>
        <Toggle
          on={enabled}
          onChange={onEnabledChange}
          ariaLabel={`${enabled ? "Disable" : "Enable"} ${runtime.display_name}`}
        />
      </div>
      <div className="flex items-center gap-2">
        <input
          value={draft}
          disabled={saving}
          placeholder={autoPath}
          aria-label={`${runtime.display_name} executable override`}
          onFocus={() => setFocused(true)}
          onChange={(event) => {
            setDraft(event.target.value);
            setValidation(null);
          }}
          onBlur={(event) => {
            setFocused(false);
            if (cancelBlurRef.current) {
              cancelBlurRef.current = false;
              return;
            }
            void commit(event.currentTarget.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") {
              cancelBlurRef.current = true;
              setDraft(runtime.override_path ?? "");
              setValidation(runtime.invalid_reason);
              event.currentTarget.blur();
            }
          }}
          className={`h-8 min-w-0 flex-1 rounded-md border bg-bg px-3 font-mono text-[11px] text-fg outline-none transition-colors placeholder:text-fg-3 disabled:opacity-60 ${
            validation
              ? "border-red-500/70 focus:border-red-400"
              : "border-line focus:border-line-strong"
          }`}
        />
        <button
          type="button"
          disabled={saving}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => void browse()}
          className="h-8 shrink-0 cursor-pointer rounded-md border border-line bg-raised px-3 text-[12px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
        >
          Browse…
        </button>
        {showReset ? (
          <button
            type="button"
            disabled={saving}
            onPointerDown={(event) => event.preventDefault()}
            onClick={() => void reset()}
            className="h-8 shrink-0 cursor-pointer px-2 text-[12px] font-medium text-fg-2 transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-60"
          >
            Reset to auto
          </button>
        ) : null}
      </div>
      <RuntimeCaption
        runtime={runtime}
        state={state}
        probeOutcome={probeOutcome}
        validation={validation}
      />
    </div>
  );
}

function RuntimeBadge({
  state,
  probeOutcome,
  saving,
}: {
  state: RuntimeRowState;
  probeOutcome: RuntimeDiscoveryOutcome | null;
  saving: boolean;
}) {
  if (saving) {
    return <Badge label="Saving…" tone="muted" spinning />;
  }
  switch (state) {
    case "detected":
      return <Badge label="Detected" tone="accent" />;
    case "override":
      return <Badge label="Override" tone="neutral" />;
    case "not_found":
      return <Badge label="Not found" tone="danger" />;
    case "checking":
      return <Badge label="Checking…" tone="muted" spinning />;
    case "probe_timed_out":
      return (
        <Badge
          label={
            probeOutcome === "timeout" ? "Probe timed out" : "Detection failed"
          }
          tone="warning"
        />
      );
    case "invalid_override":
      return <Badge label="Invalid" tone="danger" />;
  }
}

function Badge({
  label,
  tone,
  spinning = false,
}: {
  label: string;
  tone: "accent" | "neutral" | "danger" | "warning" | "muted";
  spinning?: boolean;
}) {
  const classes = {
    accent: "bg-accent/10 text-accent",
    neutral: "bg-fg/5 text-fg",
    danger: "bg-red-500/10 text-red-300",
    warning: "bg-amber-400/10 text-amber-300",
    muted: "bg-fg-3/10 text-fg-3",
  }[tone];
  const dot = {
    accent: "bg-accent",
    neutral: "bg-fg",
    danger: "bg-red-400",
    warning: "bg-amber-300",
    muted: "bg-fg-3",
  }[tone];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${classes}`}
    >
      {spinning ? (
        <Loader2 aria-hidden className="h-2.5 w-2.5 animate-spin" />
      ) : (
        <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      )}
      {label}
    </span>
  );
}

function RuntimeCaption({
  runtime,
  state,
  probeOutcome,
  validation,
}: {
  runtime: RuntimeExecutableStatus;
  state: RuntimeRowState;
  probeOutcome: RuntimeDiscoveryOutcome | null;
  validation: string | null;
}) {
  let text: string | null = null;
  let danger = false;
  if (validation) {
    text = validation;
    danger = true;
  } else if (state === "override") {
    text = runtime.detected_path
      ? `Detected: ${runtime.detected_path}`
      : `${runtime.command} was not found automatically.`;
  } else if (state === "not_found") {
    text = `Install ${runtime.display_name} or set an explicit executable path.`;
  } else if (state === "probe_timed_out") {
    const failure =
      probeOutcome === "timeout" ? "Login shell timed out" : "Shell detection failed";
    text = runtime.detected_path
      ? `${failure} — using the last resolved path: ${runtime.detected_path}`
      : `${failure}. Refresh or set an explicit executable path.`;
  } else if (state === "checking" && runtime.detected_path) {
    text = `Using last detected path: ${runtime.detected_path}`;
  }
  return text ? (
    <p
      className={`font-mono text-[11px] leading-[1.4] ${
        danger ? "text-red-300" : "text-fg-3"
      }`}
    >
      {text}
    </p>
  ) : null;
}

function shellDescription(shell: RuntimeShellStatus | null) {
  if (!shell) return "Loading shell discovery status…";
  const selected = shell.shell;
  const duration =
    shell.duration_ms == null
      ? ""
      : ` in ${(shell.duration_ms / 1000).toFixed(1)} s`;
  if (shell.checking) {
    if (!selected) return "Checking login shell for PATH and proxy settings…";
    return shell.using_last_known_good
      ? `Checking ${selected}; agents keep using the last saved environment.`
      : `Checking ${selected} for PATH and proxy settings…`;
  }
  switch (shell.outcome) {
    case "ok":
      return `PATH captured from your login shell (${selected})${duration}. Spawned agents inherit it.`;
    case "timeout":
      return `${selected} did not respond${duration}; agents keep using the last saved environment.`;
    case "spawn_error":
      return `Could not start ${selected}; refresh after fixing the login shell or set an override below.`;
    case "empty_capture":
      return `${selected} returned no usable environment; refresh or set an override below.`;
    case "no_shell":
      return "No supported login shell was configured; set an executable override below.";
    default:
      return selected
        ? `Waiting to check ${selected}.`
        : "Waiting to check the login shell.";
  }
}

function errorMessage(error: unknown) {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as RuntimeOverrideError).message === "string"
  ) {
    return (error as RuntimeOverrideError).message;
  }
  return error instanceof Error ? error.message : String(error);
}
