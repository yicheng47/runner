// Runtime catalog. Single source of truth for the runtimes the v0 UI
// exposes. Kept in a `.ts` file (not the component) so RuntimeSelect's
// React Fast-Refresh boundary stays clean.

import type { PermissionMode } from "../../lib/types";

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

// Runtimes that surface the "Permission mode" dropdown on the runner
// edit form. Source of truth: the backend's
// `router::runtime::permission_mode_args` knows which (runtime, mode)
// pairs map to flags — if the runtime appears in
// `PERMISSION_MODES_BY_RUNTIME` below with at least one non-default
// mode, the dropdown applies.
const RUNTIMES_WITH_PERMISSION_MODE = new Set(["claude-code", "codex"]);

export function runtimeSupportsPermissionMode(runtime: string): boolean {
  return RUNTIMES_WITH_PERMISSION_MODE.has(runtime);
}

export interface PermissionModeOption {
  value: PermissionMode;
  label: string;
  description: string;
  /// Marks the destructive choice (Bypass) so the dropdown can paint
  /// it with the danger palette.
  danger?: boolean;
}

// Per-runtime mode lists. claude-code and codex genuinely differ:
// claude-code has a separate `acceptEdits` mode (auto-edits but ask
// for shell/network), codex doesn't. Codex's `on-failure` mode is
// deprecated per `codex --help`, so we use `on-request` for the
// middle ground.
//
// Hand-synced with backend `router::runtime::permission_mode_args`.
// The Rust tests pin the exact (runtime, mode) → flag mapping; this
// file only owns the UI-facing labels and descriptions.
export const PERMISSION_MODES_BY_RUNTIME: Record<
  string,
  ReadonlyArray<PermissionModeOption>
> = {
  "claude-code": [
    {
      value: "default",
      label: "Default",
      description: "Ask for every tool, shell command, and write.",
    },
    {
      value: "accept_edits",
      label: "Accept edits",
      description:
        "Auto-accept file edits and common filesystem commands; still ask for shell, network, and writes outside the workspace. Available on every plan.",
    },
    {
      value: "auto",
      label: "Auto",
      description:
        "Real auto with a server-side classifier. Requires Max / Team / Enterprise / API plan + a supported model (Opus 4.7 on Max). Not available on Pro.",
    },
    {
      value: "bypass",
      label: "Bypass",
      description:
        "Skip every check. Triggers a one-time consent dialog the first time per user account.",
      danger: true,
    },
  ],
  codex: [
    {
      value: "default",
      label: "Default",
      description: "Codex's built-in approval cadence (untrusted commands).",
    },
    {
      value: "auto",
      label: "Auto",
      description:
        "Auto-runs in the workspace; the model decides when to ask the user for approval (`--ask-for-approval on-request`).",
    },
    {
      value: "bypass",
      label: "Bypass",
      description: "Never ask. Auto-runs everything in the workspace.",
      danger: true,
    },
  ],
};

// Hand-synced with backend `router::runtime::mode_match_pairs`. Each
// (mode, runtime) maps to a list of (flag, expected_value | null)
// pairs the inference helper looks for. `null` means the flag is
// value-less.
const MODE_MATCH_PAIRS_BY_RUNTIME: Record<
  string,
  Partial<Record<PermissionMode, ReadonlyArray<readonly [string, string | null]>>>
> = {
  "claude-code": {
    accept_edits: [["--permission-mode", "acceptEdits"]],
    auto: [["--permission-mode", "auto"]],
    bypass: [["--permission-mode", "bypassPermissions"]],
  },
  codex: {
    auto: [
      ["--ask-for-approval", "on-request"],
      ["--sandbox", "workspace-write"],
    ],
    bypass: [
      ["--ask-for-approval", "never"],
      ["--sandbox", "workspace-write"],
    ],
  },
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

function modePairMatches(
  runtime: string,
  args: string[],
  mode: PermissionMode,
): boolean {
  // Legacy: pre-rename claude-code rows used the standalone
  // `--dangerously-skip-permissions` flag for Bypass. Read it as
  // Bypass so the dropdown loads the right initial value; the strip
  // helper still cleans it up on save.
  if (
    runtime === "claude-code" &&
    mode === "bypass" &&
    args.includes("--dangerously-skip-permissions")
  ) {
    return true;
  }
  const pairs = MODE_MATCH_PAIRS_BY_RUNTIME[runtime]?.[mode];
  if (!pairs || pairs.length === 0) return false;
  return pairs.every(([flag, expected]) =>
    flagValueMatches(args, flag, expected),
  );
}

/// Derive the dropdown's initial value from a row's stored args. Probe
/// order is most-aggressive first: `bypass` → `auto` → `accept_edits`
/// → `default`. A row carrying flags for multiple modes resolves to
/// the most-aggressive one because that's what the runtime CLI would
/// honor at spawn time.
///
/// Conflicting / partial / unrecognized values fall through to
/// `default`.
///
/// Mirror of `router::runtime::infer_permission_mode` (Rust).
/// Algorithm and edge cases are pinned by tests on the Rust side
/// since the project has no frontend test runner today.
export function inferPermissionMode(
  runtime: string,
  args: string[],
): PermissionMode {
  if (modePairMatches(runtime, args, "bypass")) return "bypass";
  if (modePairMatches(runtime, args, "auto")) return "auto";
  if (modePairMatches(runtime, args, "accept_edits")) return "accept_edits";
  return "default";
}

// Match shape per runtime: (flag_name, takes_value).
//   - codex: `--ask-for-approval <value>` and `--sandbox <value>` —
//     the next token is consumed as the value (and `--flag=value`
//     is stripped as a single token).
//   - claude-code: `--permission-mode <value>` (value-bearing) plus
//     the legacy standalone `--dangerously-skip-permissions` flag,
//     so old rows get cleaned up on the next save.
// Mirror of `router::runtime::strip_permission_flags`.
const PERMISSION_STRIP_KEYS_BY_RUNTIME: Record<
  string,
  ReadonlyArray<readonly [string, boolean]>
> = {
  "claude-code": [
    ["--dangerously-skip-permissions", false],
    ["--permission-mode", true],
  ],
  codex: [
    ["--ask-for-approval", true],
    ["--sandbox", true],
  ],
};

/// Strip every permission-mode flag from `args`. Used by the runner
/// Create / Edit forms so the visible Args field shows only the
/// user's extra flags — the dropdown owns the permission-mode flags,
/// and the backend re-applies the canonical pair at save time via
/// `apply_permission_mode`.
///
/// Mirror of `router::runtime::strip_permission_flags` (Rust).
export function stripPermissionFlags(
  runtime: string,
  args: string[],
): string[] {
  const keys = PERMISSION_STRIP_KEYS_BY_RUNTIME[runtime];
  if (!keys || keys.length === 0) return args.slice();
  const out: string[] = [];
  let i = 0;
  while (i < args.length) {
    const arg = args[i];
    const exact = keys.find(([name]) => name === arg);
    if (exact) {
      // For takes_value flags, also skip the next token (its value).
      i += exact[1] && i + 1 < args.length ? 2 : 1;
      continue;
    }
    if (
      keys.some(([name, takesValue]) => takesValue && arg.startsWith(`${name}=`))
    ) {
      i += 1;
      continue;
    }
    out.push(arg);
    i += 1;
  }
  return out;
}
