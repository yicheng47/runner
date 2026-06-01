import { useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { Button } from "./Button";
import { Textarea } from "./Field";

// Working-directory picker shared by the runner create/edit forms and
// Settings. A wrapping textarea (so a long absolute path shows in full
// instead of truncating) paired with a native directory Browse dialog.
// Fills its parent by default; pass `className` to constrain the width
// (e.g. the fixed-width Settings row).
export function WorkingDirField({
  id,
  value,
  onChange,
  placeholder = "/absolute/path",
  disabled,
  className,
}: {
  id?: string;
  value: string;
  onChange: (path: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}) {
  const [picking, setPicking] = useState(false);
  const browse = async () => {
    if (picking) return;
    setPicking(true);
    try {
      const result = await openDialog({
        directory: true,
        multiple: false,
        defaultPath: value || undefined,
        title: "Pick a working directory",
      });
      if (typeof result === "string" && result) onChange(result);
    } catch {
      // best-effort — the dialog plugin can throw on backend mis-
      // configuration; cancel is silent rather than a stack trace.
    } finally {
      setPicking(false);
    }
  };
  return (
    <div className={`flex items-start gap-2 ${className ?? ""}`}>
      <Textarea
        id={id}
        rows={2}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className="min-w-0 flex-1 resize-y break-all"
      />
      <Button onClick={() => void browse()} disabled={disabled || picking}>
        Browse…
      </Button>
    </div>
  );
}
