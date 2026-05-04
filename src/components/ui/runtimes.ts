// Runtime catalog. Single source of truth for the runtimes the v0 UI
// exposes. Kept in a `.ts` file (not the component) so RuntimeSelect's
// React Fast-Refresh boundary stays clean.

export interface RuntimeOption {
  value: string;
  label: string;
  // The binary the runtime runs by default. Used by callers to pre-fill
  // the Command input on selection change.
  defaultCommand: string;
  description?: string;
}

// v0 narrows runtimes to just claude-code and codex. shell + aider were
// dropped to avoid encouraging untested paths; add them back here when
// they become first-class.
export const RUNTIME_OPTIONS: RuntimeOption[] = [
  {
    value: "claude-code",
    label: "claude-code",
    defaultCommand: "claude",
    description: "Anthropic Claude Code CLI",
  },
  {
    value: "codex",
    label: "codex",
    defaultCommand: "codex",
    description: "OpenAI Codex CLI",
  },
];

// Runtimes that surface a "Skip approval prompts" toggle on the
// runner edit form. Source of truth: the backend's
// `router::runtime::bypass_permission_args` knows the canonical
// flags per runtime — if it returns a non-empty Vec, the toggle
// applies. Hand-synced; if a future runtime opts in, add it here
// AND in `bypass_permission_args` (in lockstep).
const RUNTIMES_WITH_BYPASS_TOGGLE = new Set(["claude-code", "codex"]);

export function runtimeSupportsBypassToggle(runtime: string): boolean {
  return RUNTIMES_WITH_BYPASS_TOGGLE.has(runtime);
}

// Hand-synced with the backend's `router::runtime::infer_skip_approval_prompts`.
// Each entry is a (flag, expected_value | null) pair; `null` means the
// flag is value-less (just check presence). For value-bearing flags
// the user's args may use either separated form (`--flag value`) or
// equals form (`--flag=value`). Conflicting values read as toggle-off.
const BYPASS_PAIRS_BY_RUNTIME: Record<
  string,
  ReadonlyArray<readonly [string, string | null]>
> = {
  "claude-code": [["--dangerously-skip-permissions", null]],
  codex: [
    ["--ask-for-approval", "never"],
    ["--sandbox", "workspace-write"],
  ],
};

function flagValueMatches(
  args: string[],
  flag: string,
  expected: string | null,
): boolean {
  if (expected === null) {
    return args.includes(flag);
  }
  const equalsToken = `${flag}=${expected}`;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === equalsToken) return true;
    if (arg === flag && i + 1 < args.length && args[i + 1] === expected) {
      return true;
    }
  }
  return false;
}

/// Derive the toggle's initial state from existing args: true iff
/// every canonical (flag, expected_value) pair for the runtime is
/// present in `args`, in either separated form (`--flag value`) or
/// equals form (`--flag=value`). Conflicting values (e.g. codex's
/// `--ask-for-approval=on-failure`) read as toggle-off — the user
/// clearly didn't choose the "skip" semantic, and silently flipping
/// the toggle on would let the backend's strip helper wipe their
/// custom value.
///
/// Mirror of `router::runtime::infer_skip_approval_prompts` (Rust).
/// Algorithm and edge cases are pinned by tests on the Rust side
/// since the project has no frontend test runner today.
export function inferSkipApprovalPrompts(
  runtime: string,
  args: string[],
): boolean {
  const pairs = BYPASS_PAIRS_BY_RUNTIME[runtime];
  if (!pairs || pairs.length === 0) return false;
  return pairs.every(([flag, expected]) =>
    flagValueMatches(args, flag, expected),
  );
}
