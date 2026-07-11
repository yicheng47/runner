// Diagnostics pane — logs and troubleshooting tools. Mirrors the
// Help → "Reveal logs in Finder" menu item: both routes invoke
// `runner_logs_reveal`, which resolves
// `~/Library/Logs/com.wycstudios.runner/` and hands it to the opener
// plugin. Dedicated pane (not buried in About) so #10/#13/#14
// diagnostic affordances have a home to grow into.

import { useState } from "react";
import { FolderOpen } from "lucide-react";

import { invoke } from "@tauri-apps/api/core";

import { PaneHeader, SettingsCard, SettingsRow } from "./shared";

export function DiagnosticsPane() {
  const [busy, setBusy] = useState(false);
  const reveal = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("runner_logs_reveal");
    } catch (e) {
      // Best-effort: the only failure path is opener refusing the
      // path or the log dir not yet existing. The backend creates
      // the dir on demand, so a swallow here keeps the pane quiet —
      // a follow-up could surface a toast.
      console.error("reveal logs failed", e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <PaneHeader
        title="Diagnostics"
        subtitle="Logs and troubleshooting tools."
      />
      <SettingsCard>
        <SettingsRow
          label="Application logs"
          sub="Open the folder containing runner.log so you can attach it to a bug report."
        >
          <button
            type="button"
            onClick={() => void reveal()}
            disabled={busy}
            className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg disabled:cursor-not-allowed disabled:opacity-60"
          >
            <FolderOpen aria-hidden className="h-3 w-3" />
            Reveal logs in Finder
          </button>
        </SettingsRow>
      </SettingsCard>
    </>
  );
}
