// Edit an existing runner template in place.
//
// Handle is intentionally read-only: per arch §2.2 it's the template's
// identity for direct chat / CLI lookups, and renaming would break
// historical attribution. Matches the UpdateRunnerInput contract in
// src-tauri/src/commands/runner.rs.

import { useEffect, useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { api } from "../lib/api";
import type {
  PermissionMode,
  Runner,
  UpdateRunnerInput,
} from "../lib/types";
import { Button } from "./ui/Button";
import { Drawer } from "./ui/Overlay";
import { Field, Input, Textarea } from "./ui/Field";
import { RuntimeSelect } from "./ui/RuntimeSelect";
import { StyledSelect } from "./ui/StyledSelect";
import {
  EFFORT_OPTIONS_BY_RUNTIME,
  PERMISSION_MODES_BY_RUNTIME,
  RUNTIME_OPTIONS,
  inferPermissionMode,
  runtimeSupportsEffort,
  runtimeSupportsPermissionMode,
  stripPermissionFlags,
} from "./ui/runtimes";

export function RunnerEditDrawer({
  open,
  runner,
  onClose,
  onSaved,
}: {
  open: boolean;
  runner: Runner | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const [displayName, setDisplayName] = useState("");
  const [runtime, setRuntime] = useState<string>(RUNTIME_OPTIONS[0].value);
  // Command is bound to runtime — the field below is read-only — but
  // we keep the value in state (not derived) so that opening an
  // existing runner with a custom command (e.g. `/opt/homebrew/bin/
  // claude` from before the bind) preserves that custom binary
  // unless the user explicitly changes the runtime. Changing
  // runtime writes the new runtime's `defaultCommand` here.
  const [command, setCommand] = useState("");
  const [argsText, setArgsText] = useState("");
  const [workingDir, setWorkingDir] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  // "Permission mode" dropdown — initial state inferred from the
  // row's stored args. The dropdown OWNS the permission flags: the
  // visible Args field strips them on display so the user sees only
  // their extra flags, and the backend re-applies the canonical pair
  // on save (see `commands::runner::update` →
  // `router::runtime::apply_permission_mode`). Defaults to
  // `accept_edits` to match the seed and the backend's
  // `default_permission_mode()`.
  const [permissionMode, setPermissionMode] =
    useState<PermissionMode>("accept_edits");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open && runner) {
      setDisplayName(runner.display_name);
      setRuntime(runner.runtime);
      setCommand(runner.command);
      setArgsText(stripPermissionFlags(runner.runtime, runner.args).join(" "));
      setWorkingDir(runner.working_dir ?? "");
      setSystemPrompt(runner.system_prompt ?? "");
      setModel(runner.model ?? "");
      // Coerce historically-stored effort values that aren't in this
      // runtime's current enum (e.g. an old codex row with
      // `minimal`, dropped from the picker) to "" so what's saved
      // matches what's shown.
      {
        const loaded = runner.effort ?? "";
        const validEfforts = EFFORT_OPTIONS_BY_RUNTIME[runner.runtime] ?? [];
        setEffort(
          validEfforts.some((o) => o.value === loaded) ? loaded : "",
        );
      }
      setPermissionMode(inferPermissionMode(runner.runtime, runner.args));
      setError(null);
    }
  }, [open, runner]);

  const canSubmit =
    runner !== null &&
    displayName.trim().length > 0 &&
    !submitting;

  const browseWorkingDir = async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Pick a working directory",
      });
      if (typeof picked === "string") setWorkingDir(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const submit = async () => {
    if (!runner || !canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const input: UpdateRunnerInput = {
        display_name: displayName.trim(),
        runtime,
        command: command.trim(),
        args: argsText.trim() ? argsText.trim().split(/\s+/) : [],
        working_dir: workingDir.trim() || null,
        system_prompt: systemPrompt.trim() || null,
        model: model.trim() || null,
        effort: effort.trim() || null,
        // Send the mode only for runtimes that support it —
        // otherwise the backend's permission-flag helper is a no-op
        // anyway, but keeping the field undefined for shell/unknown
        // makes the contract explicit (`None` mode on the Rust side
        // preserves args verbatim).
        ...(runtimeSupportsPermissionMode(runtime)
          ? { permission_mode: permissionMode }
          : {}),
      };
      await api.runner.update(runner.id, input);
      await onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Drawer
      open={open && runner !== null}
      onClose={submitting ? () => {} : onClose}
      title={
        runner ? (
          <span className="flex items-center gap-2">
            Edit runner
            <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-xs font-normal text-fg-2">
              @{runner.handle}
            </span>
          </span>
        ) : (
          "Edit runner"
        )
      }
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={!canSubmit}>
            {submitting ? "Saving…" : "Save"}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field id="edit-display-name" label="Display name">
          <Input
            id="edit-display-name"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </Field>

        <Field id="edit-runtime" label="Runtime">
          <RuntimeSelect
            id="edit-runtime"
            value={runtime}
            onChange={(opt) => {
              setRuntime(opt.value);
              // Runtime change is the explicit signal to normalize
              // Command to the new runtime's defaultCommand. Without
              // a runtime change we keep whatever was saved on the
              // row so custom commands aren't wiped silently.
              setCommand(opt.defaultCommand);
              // Coerce effort to "" if the current value isn't in
              // the new runtime's enum, so the saved value tracks
              // what the dropdown displays. claude-code's `max` is
              // not in codex's enum; codex's `none / minimal` aren't
              // in claude-code's.
              const nextEffortOptions =
                EFFORT_OPTIONS_BY_RUNTIME[opt.value] ?? [];
              if (!nextEffortOptions.some((o) => o.value === effort)) {
                setEffort("");
              }
            }}
          />
        </Field>

        <Field
          id="edit-command"
          label="Command"
          hint="resolved from runtime · PATH lookup"
        >
          <Input id="edit-command" value={command} disabled readOnly />
        </Field>

        <Field
          id="edit-args"
          label="Args"
          hint="extra flags · whitespace-separated"
        >
          <Input
            id="edit-args"
            value={argsText}
            placeholder="--mcp-debug"
            onChange={(e) => setArgsText(e.target.value)}
          />
        </Field>

        <Field
          id="edit-model"
          label="Model"
          hint="optional · claude-code / codex: e.g. claude-opus-4-7"
        >
          <Input
            id="edit-model"
            value={model}
            placeholder="claude-opus-4-7"
            onChange={(e) => setModel(e.target.value)}
          />
        </Field>

        {runtimeSupportsEffort(runtime) ? (() => {
          const effortOptions = EFFORT_OPTIONS_BY_RUNTIME[runtime] ?? [];
          // Effort enums differ per runtime — claude-code's `max` is
          // not in codex's enum; codex's `none / minimal` aren't in
          // claude-code's. Coerce out-of-set values to the empty
          // sentinel ("Inherit CLI default") so the dropdown can't
          // render an unknown trigger.
          const safeEffort = effortOptions.some((o) => o.value === effort)
            ? effort
            : "";
          return (
            <Field
              id="edit-effort"
              label="Thinking effort"
              hint="optional · resolves to the runtime's native effort flag"
            >
              <StyledSelect
                className="w-full"
                value={safeEffort}
                options={effortOptions.map((o) => ({
                  value: o.value,
                  label: o.label,
                  description: o.description,
                }))}
                onChange={(v) => setEffort(v)}
              />
            </Field>
          );
        })() : null}

        {runtimeSupportsPermissionMode(runtime) ? (() => {
          const modeOptions = PERMISSION_MODES_BY_RUNTIME[runtime] ?? [];
          // Mode space is per-runtime: a mode that's valid for the
          // prior runtime might not exist for the new one (e.g.
          // codex has no `accept_edits`). Coerce to `default` when
          // the picked mode isn't in the new runtime's list so the
          // dropdown doesn't render an empty trigger.
          const safeValue = modeOptions.some((o) => o.value === permissionMode)
            ? permissionMode
            : "default";
          const current = modeOptions.find((o) => o.value === safeValue);
          return (
            <div className="flex items-start justify-between gap-6">
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="text-[13px] font-medium text-fg">
                  Permission mode
                </span>
                <span className="text-[11px] text-fg-2">
                  {current?.description}
                </span>
              </div>
              <div className="shrink-0 pt-0.5">
                <StyledSelect
                  className="min-w-[180px]"
                  value={safeValue}
                  options={modeOptions.map((o) => ({
                    value: o.value,
                    label: o.label,
                    description: o.description,
                    danger: o.danger,
                  }))}
                  onChange={(v) => setPermissionMode(v as PermissionMode)}
                />
              </div>
            </div>
          );
        })() : null}

        <Field
          id="edit-working-dir"
          label="Working directory"
          hint="optional"
        >
          <div className="flex items-center gap-2">
            <Input
              id="edit-working-dir"
              value={workingDir}
              onChange={(e) => setWorkingDir(e.target.value)}
              className="min-w-0 flex-1"
            />
            <Button
              onClick={() => void browseWorkingDir()}
              disabled={submitting}
            >
              Browse…
            </Button>
          </div>
        </Field>

        <Field
          id="edit-system-prompt"
          label="System prompt"
          hint="optional"
        >
          <Textarea
            id="edit-system-prompt"
            rows={6}
            value={systemPrompt}
            onChange={(e) => setSystemPrompt(e.target.value)}
          />
        </Field>

        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </form>
    </Drawer>
  );
}
