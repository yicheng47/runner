// Create a runner from the top-level Runners page (C8.5).
//
// Distinct from `AddSlotModal` — that one only selects an existing runner
// for a crew slot. This surface owns runner creation; crew membership lives
// on Crew Detail.

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import { readDefaultWorkingDir } from "../lib/settings";
import type {
  CreateRunnerInput,
  PermissionMode,
  Runner,
} from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";
import { Field, Input, Textarea } from "./ui/Field";
import { ModelField } from "./ui/ModelField";
import { RuntimeSelect } from "./ui/RuntimeSelect";
import { StyledSelect } from "./ui/StyledSelect";
import { WorkingDirField } from "./ui/WorkingDirField";
import {
  PERMISSION_MODES_BY_RUNTIME,
  RUNTIME_OPTIONS,
  runtimeSupportsPermissionMode,
} from "./ui/runtimes";

// Mirrors src-tauri/src/commands/runner.rs::validate_handle.
const HANDLE_RE = /^[a-z0-9][a-z0-9_-]{0,31}$/;

export function CreateRunnerModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (runner: Runner) => void | Promise<void>;
}) {
  const [handle, setHandle] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [runtime, setRuntime] = useState<string>(RUNTIME_OPTIONS[0].value);
  const [argsText, setArgsText] = useState("");
  const [model, setModel] = useState("");
  const [workingDir, setWorkingDir] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  // "Permission mode" dropdown — defaults to Auto. The form always
  // sends an explicit mode, and the backend writes the runtime's
  // canonical mode flags onto the stored args column at create time
  // (see commands::runner::create → router::runtime::apply_permission_mode),
  // so the user never has to type the flags themselves.
  const [permissionMode, setPermissionMode] =
    useState<PermissionMode>("auto");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setHandle("");
      setDisplayName("");
      setRuntime(RUNTIME_OPTIONS[0].value);
      setArgsText("");
      setModel("");
      setWorkingDir(readDefaultWorkingDir());
      setSystemPrompt("");
      setPermissionMode("auto");
      setError(null);
    }
  }, [open]);

  // Command is bound to runtime — each runtime's `defaultCommand` is
  // the binary we actually spawn. The Command field below is read-only
  // confirmation of what `claude-code` / `codex` resolves to on PATH.
  const command =
    RUNTIME_OPTIONS.find((o) => o.value === runtime)?.defaultCommand ??
    RUNTIME_OPTIONS[0].defaultCommand;

  const handleError = (() => {
    if (!handle) return null;
    if (!HANDLE_RE.test(handle))
      return "Lowercase letters, digits, '-' or '_'; must start with a letter or digit; up to 32 chars.";
    return null;
  })();

  const canSubmit =
    handle.length > 0 &&
    handleError === null &&
    displayName.trim().length > 0 &&
    !submitting;

  const submit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const input: CreateRunnerInput = {
      handle,
      display_name: displayName.trim(),
      runtime,
      command: command.trim(),
      args: argsText.trim() ? argsText.trim().split(/\s+/) : [],
      working_dir: workingDir.trim() || null,
      system_prompt: systemPrompt.trim() || null,
      model: model.trim() || null,
      // Send the mode only for runtimes that support it — keeps the
      // contract explicit for shell / unknown (where the backend
      // helper is a no-op anyway).
      ...(runtimeSupportsPermissionMode(runtime)
        ? { permission_mode: permissionMode }
        : {}),
    };
    try {
      const runner = await api.runner.create(input);
      await onCreated(runner);
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
        <div className="flex flex-col gap-0.5">
          <span className="text-base font-semibold text-fg">New runner</span>
          <span className="text-xs font-normal text-fg-3">
            Reusable across crews and chats.
          </span>
        </div>
      }
      widthClass="w-full max-w-xl"
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={!canSubmit}>
            {submitting ? "Creating…" : "Create runner"}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field id="new-runner-handle" label="Handle" error={handleError}>
          <div className="flex items-center rounded border border-line-strong bg-bg px-2.5 py-1.5 text-sm focus-within:border-fg-3">
            <span className="select-none pr-1 font-mono font-semibold text-fg-3">
              @
            </span>
            <input
              id="new-runner-handle"
              autoFocus
              value={handle}
              placeholder="architect"
              onChange={(e) => setHandle(e.target.value.toLowerCase())}
              className="flex-1 bg-transparent font-mono text-fg outline-none placeholder:text-fg-3"
            />
          </div>
        </Field>

        <Field id="new-runner-display-name" label="Display name">
          <Input
            id="new-runner-display-name"
            value={displayName}
            placeholder="Architect"
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </Field>

        <Field id="new-runner-runtime" label="Runtime">
          <RuntimeSelect
            id="new-runner-runtime"
            value={runtime}
            onChange={(opt) => setRuntime(opt.value)}
          />
        </Field>

        <Field id="new-runner-command" label="Command">
          <Input
            id="new-runner-command"
            value={command}
            disabled
            readOnly
          />
        </Field>

        <Field
          id="new-runner-args"
          label="Args"
          hint="extra flags · whitespace-separated"
        >
          <Input
            id="new-runner-args"
            value={argsText}
            placeholder="--mcp-debug"
            onChange={(e) => setArgsText(e.target.value)}
          />
        </Field>

        <Field
          id="new-runner-model"
          label="Model"
          hint="optional · blank uses the runtime's own model · type a name or pick an alias"
        >
          <ModelField
            id="new-runner-model"
            runtime={runtime}
            model={model}
            onModelChange={setModel}
          />
        </Field>

        {runtimeSupportsPermissionMode(runtime) ? (() => {
          const modeOptions = PERMISSION_MODES_BY_RUNTIME[runtime] ?? [];
          // Mode space is per-runtime: a mode picked under one runtime
          // might not exist under another (e.g. codex has no
          // `accept_edits`). Coerce to `default` when the picked mode
          // isn't in the new runtime's list.
          const safeValue = modeOptions.some((o) => o.value === permissionMode)
            ? permissionMode
            : "default";
          const current = modeOptions.find((o) => o.value === safeValue);
          return (
            <Field
              id="new-runner-permission-mode"
              label="Permission mode"
              hint={current?.description}
            >
              <StyledSelect
                className="w-full"
                value={safeValue}
                options={modeOptions.map((o) => ({
                  value: o.value,
                  label: o.label,
                  description: o.description,
                  danger: o.danger,
                }))}
                onChange={(v) => setPermissionMode(v as PermissionMode)}
              />
            </Field>
          );
        })() : null}

        <Field id="new-runner-working-dir" label="Working directory">
          <WorkingDirField
            id="new-runner-working-dir"
            value={workingDir}
            onChange={setWorkingDir}
            disabled={submitting}
          />
        </Field>

        <Field id="new-runner-system-prompt" label="Default system prompt">
          <Textarea
            id="new-runner-system-prompt"
            rows={5}
            value={systemPrompt}
            placeholder="You are the architect for this crew. When a mission starts, decompose the goal into 2–4 tasks and assign each to a @handle in the crew."
            onChange={(e) => setSystemPrompt(e.target.value)}
          />
        </Field>

        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </form>
    </Modal>
  );
}
