// Create a runner from the top-level Runners page (C8.5).
//
// Distinct from `AddSlotModal` (which creates a runner *and* adds it to a
// specific crew in one shot) — this surface only owns the runner row.
// Crew membership is a separate concern handled from Crew Detail's Add
// Slot modal.
//
// Form shape mirrors AddSlotModal's; the two share field validation
// (HANDLE_RE) and runtime presets but not the submission flow.

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import type { CreateRunnerInput, Runner } from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";
import { Field, Input, Textarea } from "./ui/Field";

const RUNTIMES = ["shell", "claude-code", "codex", "aider"] as const;

// Mirrors src-tauri/src/commands/runner.rs::validate_handle. Kept in sync
// for instant UX feedback; the backend is the source of truth.
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
  const [role, setRole] = useState("");
  const [runtime, setRuntime] = useState<string>("shell");
  const [command, setCommand] = useState("");
  const [argsText, setArgsText] = useState("");
  const [workingDir, setWorkingDir] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setHandle("");
      setDisplayName("");
      setRole("");
      setRuntime("shell");
      setCommand("");
      setArgsText("");
      setWorkingDir("");
      setSystemPrompt("");
      setError(null);
    }
  }, [open]);

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
    role.trim().length > 0 &&
    command.trim().length > 0 &&
    !submitting;

  const submit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const input: CreateRunnerInput = {
      handle,
      display_name: displayName.trim(),
      role: role.trim(),
      runtime,
      command: command.trim(),
      args: argsText.trim() ? argsText.trim().split(/\s+/) : [],
      working_dir: workingDir.trim() || null,
      system_prompt: systemPrompt.trim() || null,
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
          <span className="text-base font-semibold text-neutral-900">
            New runner
          </span>
          <span className="text-xs font-normal text-neutral-500">
            Defines a reusable agent. Add it to a crew or chat with it
            directly from the Runners page.
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
        <div className="grid grid-cols-2 gap-3">
          <Field
            id="new-runner-handle"
            label="Handle"
            hint="globally unique, immutable"
            error={handleError}
          >
            <div className="flex items-center rounded-md border border-neutral-300 bg-neutral-50 px-2.5 py-1.5 text-sm focus-within:border-neutral-400 focus-within:bg-white focus-within:ring-2 focus-within:ring-neutral-300">
              <span className="select-none pr-1 font-mono font-semibold text-neutral-400">
                @
              </span>
              <input
                id="new-runner-handle"
                autoFocus
                value={handle}
                placeholder="reviewer"
                onChange={(e) => setHandle(e.target.value.toLowerCase())}
                className="flex-1 bg-transparent font-mono text-neutral-900 outline-none placeholder:text-neutral-400"
              />
            </div>
          </Field>
          <Field id="new-runner-display-name" label="Display name">
            <Input
              id="new-runner-display-name"
              value={displayName}
              placeholder="e.g. Implementer"
              onChange={(e) => setDisplayName(e.target.value)}
            />
          </Field>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <Field id="new-runner-role" label="Role">
            <Input
              id="new-runner-role"
              value={role}
              placeholder="e.g. impl, reviewer, architect"
              onChange={(e) => setRole(e.target.value)}
            />
          </Field>
          <Field id="new-runner-runtime" label="Runtime">
            <select
              id="new-runner-runtime"
              value={runtime}
              onChange={(e) => setRuntime(e.target.value)}
              className="w-full rounded-md border border-neutral-300 bg-white px-2.5 py-1.5 text-sm text-neutral-900 focus:outline-none focus:ring-2 focus:ring-neutral-400"
            >
              {RUNTIMES.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          </Field>
        </div>

        <Field id="new-runner-command" label="Command" hint="the binary to spawn">
          <Input
            id="new-runner-command"
            value={command}
            placeholder="e.g. claude, codex, sh"
            onChange={(e) => setCommand(e.target.value)}
          />
        </Field>

        <Field id="new-runner-args" label="Args" hint="whitespace-separated">
          <Input
            id="new-runner-args"
            value={argsText}
            placeholder="e.g. --dangerously-skip-permissions"
            onChange={(e) => setArgsText(e.target.value)}
          />
        </Field>

        <Field
          id="new-runner-working-dir"
          label="Working directory"
          hint="optional — fallback when a mission/session doesn't specify one"
        >
          <Input
            id="new-runner-working-dir"
            value={workingDir}
            placeholder="/absolute/path"
            onChange={(e) => setWorkingDir(e.target.value)}
          />
        </Field>

        <Field id="new-runner-system-prompt" label="System prompt" hint="optional">
          <Textarea
            id="new-runner-system-prompt"
            rows={4}
            value={systemPrompt}
            placeholder="Behavioral instructions for this runner."
            onChange={(e) => setSystemPrompt(e.target.value)}
          />
        </Field>

        {error ? <p className="text-xs text-red-600">{error}</p> : null}
      </form>
    </Modal>
  );
}
