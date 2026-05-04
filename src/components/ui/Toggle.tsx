// Pill-style on/off switch. Matches the private Toggle inside
// SettingsModal.tsx — split out here so the runner-edit forms
// (CreateRunnerModal, RunnerEditDrawer) can reuse it without
// cross-component imports.

export function Toggle({
  on,
  onChange,
  ariaLabel,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={ariaLabel}
      onClick={() => onChange(!on)}
      className={`flex h-[18px] w-8 cursor-pointer items-center rounded-full p-0.5 transition-colors ${
        on ? "justify-end bg-accent/15" : "justify-start bg-raised"
      }`}
    >
      <span
        className={`block h-3.5 w-3.5 rounded-full ${
          on ? "bg-accent" : "bg-fg-3"
        }`}
      />
    </button>
  );
}
