// Pill-style on/off switch. Shared by the settings panes and the
// runner-edit forms (CreateRunnerModal, RunnerEditDrawer).

export function Toggle({
  on,
  onChange,
  disabled,
  ariaLabel,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={`flex h-[18px] w-8 cursor-pointer items-center rounded-full p-0.5 transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
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
