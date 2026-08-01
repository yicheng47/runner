import { useEffect, useId, useState } from "react";

import { basename } from "@tauri-apps/api/path";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";

import { api, type ProjectRow } from "../lib/api";
import { readDefaultWorkingDir } from "../lib/settings";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";

interface StartProjectModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (project: ProjectRow) => void | Promise<void>;
}

export function StartProjectModal({
  open,
  onClose,
  onCreated,
}: StartProjectModalProps) {
  const formId = useId();
  const cwdInputId = `${formId}-cwd`;
  const nameInputId = `${formId}-name`;
  const [cwd, setCwd] = useState("");
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setCwd(readDefaultWorkingDir());
    setName("");
    setNameEdited(false);
    setSubmitting(false);
    setError(null);
  }, [open]);

  useEffect(() => {
    if (!open || nameEdited) return;
    const trimmedCwd = cwd.trim();
    if (!trimmedCwd) {
      setName("");
      return;
    }

    let cancelled = false;
    void basename(trimmedCwd)
      .then((value) => {
        if (!cancelled) setName(value);
      })
      .catch(() => {
        if (!cancelled) setName("");
      });
    return () => {
      cancelled = true;
    };
  }, [cwd, nameEdited, open]);

  const browse = async () => {
    try {
      const defaultPath = cwd.trim() || readDefaultWorkingDir() || undefined;
      const picked = await openDialog({ directory: true, defaultPath });
      if (typeof picked === "string") setCwd(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const canCreate = Boolean(cwd.trim() && name.trim()) && !submitting;

  const create = async () => {
    if (!canCreate) return;
    setSubmitting(true);
    setError(null);
    try {
      const project = await api.project.create(name.trim(), cwd.trim());
      await onCreated(project);
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
              Start project
            </span>
            <span className="text-xs font-normal text-fg-2">
              Add a named working directory to the sidebar.
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-50"
            aria-label="Close start project"
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
            type="submit"
            form={formId}
            variant="primary"
            disabled={!canCreate}
          >
            {submitting ? "Creating…" : "Create project"}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        {error ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
            {error}
          </div>
        ) : null}

        <Field label="Directory" htmlFor={cwdInputId}>
          <div className="flex items-center gap-2">
            <input
              id={cwdInputId}
              autoFocus
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              placeholder="/Users/you/projects/runner"
              disabled={submitting}
              className="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
            />
            <Button onClick={() => void browse()} disabled={submitting}>
              Browse…
            </Button>
          </div>
        </Field>

        <Field label="Name" htmlFor={nameInputId}>
          <input
            id={nameInputId}
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setNameEdited(true);
            }}
            placeholder="runner"
            disabled={submitting}
            className="rounded-md border border-line bg-bg px-3 py-2 text-[13px] text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>
      </form>
    </Modal>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-xs font-semibold text-fg">
        {label}
      </label>
      {children}
    </div>
  );
}
