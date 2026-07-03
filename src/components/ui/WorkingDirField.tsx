import { useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { useT } from "../../lib/i18n";
import { Button } from "./Button";
import { Input, Textarea } from "./Field";

// Working-directory picker shared by the runner create/edit forms and
// Settings, paired with a native directory Browse dialog. Defaults to a
// wrapping textarea so a long absolute path shows in full — good in the
// full-width modal forms. Pass `singleLine` for narrow contexts (the
// Settings row), where wrapping a path just breaks it into ugly
// mid-segment fragments; there it's a plain truncating input instead.
// Use `className` to constrain the width.
export function WorkingDirField({
  id,
  value,
  onChange,
  placeholder,
  disabled,
  className,
  singleLine,
}: {
  id?: string;
  value: string;
  onChange: (path: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  singleLine?: boolean;
}) {
  const t = useT();
  const resolvedPlaceholder = placeholder ?? t("/absolute/path");
  const [picking, setPicking] = useState(false);
  const browse = async () => {
    if (picking) return;
    setPicking(true);
    try {
      const result = await openDialog({
        directory: true,
        multiple: false,
        defaultPath: value || undefined,
        title: t("Pick a working directory"),
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
    <div
      className={`flex gap-2 ${singleLine ? "items-center" : "items-start"} ${className ?? ""}`}
    >
      {singleLine ? (
        <Input
          id={id}
          value={value}
          placeholder={resolvedPlaceholder}
          onChange={(e) => onChange(e.target.value)}
          className="min-w-0 flex-1 font-mono"
        />
      ) : (
        <Textarea
          id={id}
          rows={2}
          value={value}
          placeholder={resolvedPlaceholder}
          onChange={(e) => onChange(e.target.value)}
          className="min-w-0 flex-1 resize-y break-all"
        />
      )}
      <Button onClick={() => void browse()} disabled={disabled || picking}>
        {t("Browse…")}
      </Button>
    </div>
  );
}
