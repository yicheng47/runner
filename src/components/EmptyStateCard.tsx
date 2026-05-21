// Centered "No X yet" card used by every list page (Runners, Crews,
// Missions). Mirrors the empty-state pattern in design frames `GmFmi`,
// `OCv6q`, `D0oz3`: 520px panel-bordered card, 64px circular icon badge
// in accent-tinted dark green, 20px headline, centered description, and
// a primary CTA matching the page's header button.

import type { ReactNode } from "react";

export function EmptyStateCard({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  action: ReactNode;
}) {
  return (
    <div className="flex flex-1 items-center justify-center">
      <div className="flex w-full max-w-[520px] flex-col items-center gap-5 rounded-xl border border-line bg-panel/40 p-12 text-center">
        {/* Accent-tinted circle behind the icon. Was hardcoded to
            Carbon's #0F1A14 / #1F3329 (faint-green tints); now routed
            through `bg-accent/10` + `border-accent/30` so the tint
            tracks the active theme's accent (green/sky-blue/mauve). */}
        <div className="flex h-16 w-16 items-center justify-center rounded-full border border-accent/30 bg-accent/10 text-accent">
          {icon}
        </div>
        <h2 className="text-xl font-semibold text-fg">{title}</h2>
        <p className="max-w-sm text-sm leading-relaxed text-fg-2">
          {description}
        </p>
        <div className="pt-2">{action}</div>
      </div>
    </div>
  );
}
