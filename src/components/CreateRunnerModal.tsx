// Create a runner from the top-level Runners page (C8.5).
//
// Distinct from `AddSlotModal` — that one only selects an existing runner
// for a crew slot. This surface owns runner creation; crew membership lives
// on Crew Detail.

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import { useT } from "../lib/i18n";
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
  const t = useT();
  const [handle, setHandle] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [runtime, setRuntime] = useState<string>(RUNTIME_OPTIONS[0].value);
  const [command, setCommand] = useState(RUNTIME_OPTIONS[0].defaultCommand);
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
  // Where the agent runs: "wsl" (default) or "native" (Windows host).
  const [executionTarget, setExecutionTarget] = useState<string>("wsl");
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
      setExecutionTarget("wsl");
      setError(null);
    }
  }, [open]);

  // Command is bound to the runtime's `defaultCommand` for WSL runners
  // (the binary inside the distro: claude / codex). For "native" Windows
  // runners the Command field is editable so you can point at a host
  // command (powershell, a Windows-installed agent, …); we only auto-sync
  // back to the runtime default while the target is WSL or the runtime
  // changes.
  useEffect(() => {
    if (executionTarget !== "native") {
      setCommand(
        RUNTIME_OPTIONS.find((o) => o.value === runtime)?.defaultCommand ??
          RUNTIME_OPTIONS[0].defaultCommand,
      );
    }
  }, [runtime, executionTarget]);

  const handleError = (() => {
    if (!handle) return null;
    if (!HANDLE_RE.test(handle))
      return t(
        "Lowercase letters, digits, '-' or '_'; must start with a letter or digit; up to 32 chars.",
      );
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
      execution_target: executionTarget,
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
          <span className="text-base font-semibold text-fg">{t("New runner")}</span>
          <span className="text-xs font-normal text-fg-3">
            {t("Reusable across crews and chats.")}
          </span>
        </div>
      }
      widthClass="w-full max-w-xl"
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            {t("Cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={!canSubmit}>
            {submitting ? t("Creating…") : t("Create runner")}
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
        <Field id="new-runner-handle" label={t("Handle")} error={handleError}>
          <div className="flex items-center rounded border border-line-strong bg-bg px-2.5 py-1.5 text-sm focus-within:border-fg-3">
            <span className="select-none pr-1 font-mono font-semibold text-fg-3">
              @
            </span>
            <input
              id="new-runner-handle"
              autoFocus
              value={handle}
              placeholder={t("architect")}
              onChange={(e) => setHandle(e.target.value.toLowerCase())}
              className="flex-1 bg-transparent font-mono text-fg outline-none placeholder:text-fg-3"
            />
          </div>
        </Field>

        <Field id="new-runner-display-name" label={t("Display name")}>
          <Input
            id="new-runner-display-name"
            value={displayName}
            placeholder={t("Architect")}
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </Field>

        <Field id="new-runner-runtime" label={t("Runtime")}>
          <RuntimeSelect
            id="new-runner-runtime"
            value={runtime}
            onChange={(opt) => setRuntime(opt.value)}
          />
        </Field>

        <Field
          id="new-runner-command"
          label={t("Command")}
          hint={
            executionTarget === "native"
              ? t(
                  "the Windows-installed agent to run (e.g. codex, claude), launched via cmd.exe so .cmd shims resolve",
                )
              : undefined
          }
        >
          <Input
            id="new-runner-command"
            value={command}
            disabled={executionTarget !== "native"}
            readOnly={executionTarget !== "native"}
            onChange={(e) => setCommand(e.target.value)}
          />
        </Field>

        <Field
          id="new-runner-args"
          label={t("Args")}
          hint={t("extra flags · whitespace-separated")}
        >
          <Input
            id="new-runner-args"
            value={argsText}
            placeholder={t("--mcp-debug")}
            onChange={(e) => setArgsText(e.target.value)}
          />
        </Field>

        <Field
          id="new-runner-model"
          label={t("Model")}
          hint={t(
            "optional · blank uses the runtime's own model · type a name or pick an alias",
          )}
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
              label={t("Permission mode")}
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

        <Field
          id="new-runner-exec-target"
          label={t("Execution target")}
          hint={t(
            "where the agent runs · WSL by default · Windows runs the command natively on the host",
          )}
        >
          <StyledSelect
            className="w-full"
            value={executionTarget}
            options={[
              {
                value: "wsl",
                label: t("WSL"),
                description: t(
                  "Run the agent inside WSL via wsl.exe (claude/codex installed in your distro).",
                ),
              },
              {
                value: "native",
                label: t("Windows"),
                description: t(
                  "Run the command directly on the Windows host (powershell, cmd, a Windows-installed agent).",
                ),
              },
            ]}
            onChange={(v) => setExecutionTarget(v)}
          />
        </Field>

        <Field id="new-runner-working-dir" label={t("Working directory")}>
          <WorkingDirField
            id="new-runner-working-dir"
            value={workingDir}
            onChange={setWorkingDir}
            disabled={submitting}
          />
        </Field>

        <Field id="new-runner-system-prompt" label={t("Default system prompt")}>
          <Textarea
            id="new-runner-system-prompt"
            rows={5}
            value={systemPrompt}
            placeholder={t(
              "You are the architect for this crew. When a mission starts, decompose the goal into 2–4 tasks and assign each to a @handle in the crew.",
            )}
            onChange={(e) => setSystemPrompt(e.target.value)}
          />
        </Field>

        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </form>
    </Modal>
  );
}
