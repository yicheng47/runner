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

// The UI exposes only first-class runtimes. Shell + aider remain hidden
// until their paths are fully supported.
export const RUNTIME_OPTIONS: RuntimeOption[] = [
  {
    value: "codex",
    label: "Codex",
    defaultCommand: "codex",
    description: "OpenAI Codex CLI",
  },
  {
    value: "claude-code",
    label: "Claude Code",
    defaultCommand: "claude",
    description: "Anthropic Claude Code CLI",
  },
  {
    value: "qoder",
    label: "Qoder",
    defaultCommand: "qodercli",
    description: "Qoder CLI",
  },
  {
    value: "trae",
    label: "TRAE CLI",
    defaultCommand: "traecli",
    description: "TRAE CLI",
  },
];

const RUNTIMES_CLEARING_ON_RESIZE = new Set([
  "claude-code",
  "codex",
  "qoder",
  "trae",
]);

export function runtimeClearsOnResize(runtime: string): boolean {
  return RUNTIMES_CLEARING_ON_RESIZE.has(runtime);
}

// Runtimes that surface the "Permission mode" dropdown on the runner
// edit form. Source of truth: the backend's
// `router::runtime::permission_mode_args` knows which (runtime, mode)
// pairs map to flags — if the runtime appears in
// `PERMISSION_MODES_BY_RUNTIME` below with at least one non-default
// mode, the dropdown applies.
const RUNTIMES_WITH_PERMISSION_MODE = new Set([
  "claude-code",
  "codex",
  "qoder",
  "trae",
]);

export function runtimeSupportsPermissionMode(runtime: string): boolean {
  return RUNTIMES_WITH_PERMISSION_MODE.has(runtime);
}

// Per-runtime "thinking effort" enum. Hand-synced with backend
// `router::runtime::model_effort_args`, which maps:
//   - claude-code → `--effort <level>` (case-insensitive,
//     `low / medium / high / xhigh / max` per `claude --help`).
//   - codex → `-c model_reasoning_effort=<lowercased>`; options below
//     are the refreshed per-model catalog union verified with
//     codex-cli 0.146.0.
// An empty value means "inherit the CLI default" — the runner row
// stores NULL and `model_effort_args` emits no flag.
export interface EffortOption {
  value: string;
  label: string;
  description?: string;
}

const EFFORT_INHERIT: EffortOption = {
  value: "",
  label: "Inherit CLI default",
  description: "Don't pass the agent's effort flag.",
};

export const EFFORT_OPTIONS_BY_RUNTIME: Record<
  string,
  ReadonlyArray<EffortOption>
> = {
  "claude-code": [
    EFFORT_INHERIT,
    { value: "low", label: "low" },
    { value: "medium", label: "medium" },
    { value: "high", label: "high" },
    { value: "xhigh", label: "xhigh" },
    { value: "max", label: "max" },
  ],
  // codex-cli 0.146.0's refreshed catalog lists `low / medium / high /
  // xhigh / max / ultra` for gpt-5.6-sol and gpt-5.6-terra;
  // gpt-5.6-luna stops at `max`, and the other visible models stop at
  // `xhigh`. Unlike 0.130.0, passing an unknown effort to a
  // config-loading command no longer reports the enum variants, so
  // Runner exposes the catalog union. Historically stored `none` /
  // `minimal` values coerce to "Inherit CLI default" via the edit
  // drawer's safe-value guard.
  codex: [
    EFFORT_INHERIT,
    { value: "low", label: "Low", description: "Fast responses with lighter reasoning." },
    { value: "medium", label: "Medium", description: "Balances speed and reasoning depth." },
    { value: "high", label: "High", description: "Greater reasoning depth for complex problems." },
    { value: "xhigh", label: "Extra high", description: "Extra reasoning depth for complex problems." },
    { value: "max", label: "Max", description: "Maximum reasoning depth for the hardest problems." },
    { value: "ultra", label: "Ultra", description: "Maximum reasoning with automatic task delegation." },
  ],
  trae: [
    EFFORT_INHERIT,
    { value: "low", label: "Low", description: "Fast responses with lighter reasoning." },
    { value: "medium", label: "Medium", description: "Balances speed and reasoning depth." },
    { value: "high", label: "High", description: "Greater reasoning depth for complex problems." },
    { value: "xhigh", label: "Extra high", description: "Extra reasoning depth for complex problems." },
  ],
};

export function runtimeSupportsEffort(runtime: string): boolean {
  return runtime in EFFORT_OPTIONS_BY_RUNTIME;
}

export interface ModelOption {
  value: string;
  label: string;
  description?: string;
}

// "default" maps to the empty value: an empty field stores NULL and
// `model_effort_args` emits no `--model` flag, so the runtime uses its
// own default. Listed explicitly (not just reachable by clearing the
// field) so it can be re-picked after selecting another model.
const MODEL_DEFAULT: ModelOption = {
  value: "",
  label: "default",
  description: "Use the agent's own default model.",
};

// Curated model suggestions surfaced in the Model combobox's dropdown.
// They're shortcuts, not a closed set — the field is a free-text input,
// so any model name can be typed.
//
// claude-code's `--model` accepts version-stable aliases (`opus` always
// resolves to the latest Opus), so suggesting them never goes stale.
// Following the Orca precedent, codex mirrors the current visible
// entries from its refreshed catalog while free-text passthrough handles
// every persisted or newly released model between refreshes. qoder/trae
// expose runtime-specific catalogs, so they offer only `default` here
// and let the user type any full name.
export const MODEL_SUGGESTIONS_BY_RUNTIME: Record<
  string,
  ReadonlyArray<ModelOption>
> = {
  "claude-code": [
    MODEL_DEFAULT,
    { value: "fable", label: "fable", description: "Latest Claude Fable." },
    { value: "opus", label: "opus", description: "Latest Claude Opus." },
    { value: "sonnet", label: "sonnet", description: "Latest Claude Sonnet." },
    { value: "haiku", label: "haiku", description: "Latest Claude Haiku." },
  ],
  codex: [
    MODEL_DEFAULT,
    {
      value: "gpt-5.6-sol",
      label: "gpt-5.6-sol",
      description: "Latest frontier agentic coding model.",
    },
    {
      value: "gpt-5.6-terra",
      label: "gpt-5.6-terra",
      description: "Balanced agentic coding model for everyday work.",
    },
    {
      value: "gpt-5.6-luna",
      label: "gpt-5.6-luna",
      description: "Fast and affordable agentic coding model.",
    },
    {
      value: "gpt-5.5",
      label: "gpt-5.5",
      description:
        "Frontier model for complex coding, research, and real-world work.",
    },
    {
      value: "gpt-5.4",
      label: "gpt-5.4",
      description: "Strong model for everyday coding.",
    },
    {
      value: "gpt-5.4-mini",
      label: "gpt-5.4-mini",
      description:
        "Small, fast, and cost-efficient model for simpler coding tasks.",
    },
    {
      value: "gpt-5.3-codex-spark",
      label: "gpt-5.3-codex-spark",
      description: "Ultra-fast coding model.",
    },
  ],
  qoder: [MODEL_DEFAULT],
  trae: [MODEL_DEFAULT],
};

export function modelSuggestions(runtime: string): ReadonlyArray<ModelOption> {
  return MODEL_SUGGESTIONS_BY_RUNTIME[runtime] ?? [];
}

export interface PermissionModeOption {
  value: PermissionMode;
  label: string;
  description: string;
  /// Marks the destructive choice (Bypass) so the dropdown can paint
  /// it with the danger palette.
  danger?: boolean;
}

// Per-runtime mode lists. Claude-code has a separate `acceptEdits`
// mode (auto-edits but ask for shell/network), while codex does not.
// Codex's `on-failure` mode is deprecated per `codex --help`, so its
// middle ground uses `on-request`. Qoder exposes only Default and the
// live-probed `auto` mode for now.
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
        "Auto-run in the workspace and ask only when the model decides approval is needed (`--ask-for-approval on-request`).",
    },
    {
      value: "bypass",
      label: "Bypass",
      description:
        "Never ask while keeping Codex's workspace-write sandbox (`--ask-for-approval never`).",
      danger: true,
    },
  ],
  qoder: [
    {
      value: "default",
      label: "Default",
      description: "Use Qoder's built-in permission mode.",
    },
    {
      value: "auto",
      label: "Auto",
      description: "Run with Qoder's verified `--permission-mode auto` mode.",
    },
  ],
  trae: [
    {
      value: "default",
      label: "Default",
      description: "TRAE CLI's built-in approval cadence.",
    },
    {
      value: "auto",
      label: "Auto",
      description:
        "Use TRAE CLI's native auto-reviewer (`--permission-mode auto`).",
    },
    {
      value: "bypass",
      label: "Bypass",
      description:
        "Bypass TRAE CLI permission prompts (`--permission-mode bypass_permissions`).",
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
  qoder: {
    auto: [["--permission-mode", "auto"]],
  },
  trae: {
    auto: [["--permission-mode", "auto"]],
    bypass: [["--permission-mode", "bypass_permissions"]],
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
/// Algorithm and edge cases are pinned by the Rust and TypeScript tests.
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
//   - qoder/trae: `--permission-mode <value>` (value-bearing).
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
  qoder: [["--permission-mode", true]],
  trae: [["--permission-mode", true]],
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
