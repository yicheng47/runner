// Edit an existing runner template in place.
//
// Handle is intentionally read-only: per arch §2.2 it's the template's
// identity for direct chat / CLI lookups, and renaming would break
// historical attribution. Matches the UpdateRunnerInput contract in
// src-tauri/src/commands/runner.rs.

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import type { Runner, UpdateRunnerInput } from "../lib/types";
import { Button } from "./ui/Button";
import { Drawer } from "./ui/Overlay";
import { Field, Input, Textarea } from "./ui/Field";
import { RuntimeSelect } from "./ui/RuntimeSelect";
import { RUNTIME_OPTIONS } from "./ui/runtimes";

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
  const [command, setCommand] = useState("");
  const [argsText, setArgsText] = useState("");
  const [workingDir, setWorkingDir] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open && runner) {
      setDisplayName(runner.display_name);
      setRuntime(runner.runtime);
      setCommand(runner.command);
      setArgsText(runner.args.join(" "));
      setWorkingDir(runner.working_dir ?? "");
      setSystemPrompt(runner.system_prompt ?? "");
      setModel(runner.model ?? "");
      setEffort(runner.effort ?? "");
      setError(null);
    }
  }, [open, runner]);

  const canSubmit =
    runner !== null &&
    displayName.trim().length > 0 &&
    command.trim().length > 0 &&
    !submitting;

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
              // Edit drawer keeps the existing command — picking a
              // runtime here just changes the runtime tag, not the
              // user's already-tweaked binary path.
            }}
          />
        </Field>

        <Field id="edit-command" label="Command">
          <Input
            id="edit-command"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
          />
        </Field>

        <Field id="edit-args" label="Args" hint="whitespace-separated">
          <Input
            id="edit-args"
            value={argsText}
            onChange={(e) => setArgsText(e.target.value)}
          />
        </Field>

        <Field
          id="edit-working-dir"
          label="Working directory"
          hint="optional"
        >
          <Input
            id="edit-working-dir"
            value={workingDir}
            onChange={(e) => setWorkingDir(e.target.value)}
          />
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

        {/* Model + effort: claude-code-only today (codex's adapter
            degrades silently). Empty = inherit the agent CLI's
            default. Hint copy is intentionally permissive — the row
            stores whatever the user types so we don't have to keep
            an enum in sync with model rotations. */}
        <Field
          id="edit-model"
          label="Model"
          hint="optional · claude-code: e.g. claude-opus-4-7"
        >
          <Input
            id="edit-model"
            value={model}
            placeholder="claude-opus-4-7"
            onChange={(e) => setModel(e.target.value)}
          />
        </Field>

        <Field
          id="edit-effort"
          label="Thinking effort"
          hint="optional · claude-code: xhigh / high / medium / low / off"
        >
          <Input
            id="edit-effort"
            value={effort}
            placeholder="xhigh"
            onChange={(e) => setEffort(e.target.value)}
          />
        </Field>

        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </form>
    </Drawer>
  );
}
