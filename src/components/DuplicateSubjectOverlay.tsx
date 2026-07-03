// Arc-style duplicate-subject overlay (impl 0018, spec 12).
//
// When two windows look at the same mission / direct chat, the
// most-recently-focused one is primary and owns the PTY; the other(s) render
// this card over the content area. It's a soft hint, not a modal: the
// terminal mount is gated by `isSecondary` regardless, so "Stay here" only
// hides the card (to read the feed / metadata underneath) — it does not grant
// this window terminal ownership. Dismissal isn't persisted, so navigating
// away and back to the duplicated subject re-shows it (spec decision 6).

import { AppWindow } from "lucide-react";

import { api } from "../lib/api";
import { useT } from "../lib/i18n";

export function DuplicateSubjectOverlay({
  kind,
  primaryLabel,
  onStayHere,
}: {
  /** Which surface is duplicated — tailors the copy. */
  kind: "mission" | "chat";
  /** Label of the primary window, target of "Focus that window". Null only
   *  in the transient window before the focus map names a primary. */
  primaryLabel: string | null;
  onStayHere: () => void;
}) {
  const t = useT();
  const noun = kind === "mission" ? t("mission") : t("chat");
  return (
    <>
      <div className="pointer-events-none absolute inset-0 z-20 bg-bg/70 backdrop-blur-sm" />
      <div className="absolute inset-0 z-30 flex items-center justify-center p-6">
        <div className="flex w-full max-w-md flex-col items-center gap-4 rounded-xl border border-line bg-panel p-6 text-center shadow-[0_8px_30px_rgba(0,0,0,0.67)]">
          <div className="flex h-11 w-11 items-center justify-center rounded-lg border border-line bg-bg text-accent">
            <AppWindow aria-hidden className="h-5 w-5" />
          </div>
          <div className="flex flex-col gap-1.5">
            <h2 className="text-[15px] font-semibold text-fg">
              {t("Open in another window")}
            </h2>
            <p className="text-[13px] leading-relaxed text-fg-2">
              {t(
                "Another window is already driving this {noun}. Only one window can own the terminal at a time, so this view is read-only until you focus it here.",
                { noun },
              )}
            </p>
          </div>
          <div className="flex items-center gap-3 pt-0.5">
            <button
              type="button"
              onClick={() => {
                if (primaryLabel) {
                  void api.window.focusOther(primaryLabel).catch(() => {
                    // best-effort; the target window may have just closed,
                    // in which case the focus map will promote this one.
                  });
                }
              }}
              disabled={!primaryLabel}
              className="flex cursor-pointer items-center gap-2 rounded-md bg-accent px-3.5 py-2 text-[13px] font-semibold text-bg transition-colors hover:bg-accent/90 disabled:cursor-default disabled:opacity-50"
            >
              <AppWindow aria-hidden className="h-3.5 w-3.5" />
              {t("Focus that window")}
            </button>
            <button
              type="button"
              onClick={onStayHere}
              className="cursor-pointer text-[13px] text-fg-2 underline-offset-2 transition-colors hover:text-fg hover:underline"
            >
              {t("Stay here")}
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
