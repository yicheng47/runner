# Model catalog refresh: seed codex, add fable, catch up effort enums

## Status

Implemented (#386 closed 2026-08-07, `d4c082f`, `3f58c29`). Tracking issue [#386](https://github.com/yicheng47/runner/issues/386) — resolved in reduced form: the dynamic `codex app-server model/list` path stays documented in the issue but is not built; the catalog stays hardcoded with free-text passthrough, per the Orca precedent (`agent-session-option-catalog-claude-codex.ts`: "keep this seed short and allow unknown persisted ids to pass through"). During implementation, the mission author explicitly expanded the scope to mirror every model visible in the installed Codex catalog, add model/effort controls to Direct chat creation, and make detected agents user-enableable across Start Chat and runner create/edit selectors.

## Problem

The picker machinery already exists — `ModelField` is an editable combobox over `MODEL_SUGGESTIONS_BY_RUNTIME`, with `default` (empty → no `--model` flag) listed explicitly and free text always allowed; `EFFORT_OPTIONS_BY_RUNTIME` feeds the effort dropdowns. But the data is stale in three ways:

1. **codex offers only `default`** — the old comment ("codex has no alias scheme — full names rot every release") predates a usable equilibrium: a short seed of current names plus passthrough covers reality, and is what Orca ships.
2. **claude-code's list predates the Claude 5 family** — no `fable` alias, while `claude --model fable` is the top-tier pick.
3. **codex's effort list stops at `xhigh`** — the enum comment was verified against codex-cli 0.130.0; current codex (0.146.0) advertises and accepts `max` (in daily use here) and `ultra` for some models.
4. **Direct chat creation cannot select model or effort** — the runner forms already have the picker machinery, but Direct mode only selects an agent and working directory.
5. **Agent selectors offer catalog entries that are not installed** — executable discovery already knows which agents are usable, but the selection surfaces do not consume that status or let the user hide an installed agent.

## Key Decisions

1. **Hardcoded seed + `default` + free text, both runtimes. No CLI querying.** The dynamic `model/list` fetch (verified working, see #386) is deliberately not built: it adds a subprocess JSON-RPC dance to solve staleness that passthrough already absorbs. Revisit trigger recorded in #386.
2. **Codex mirrors the installed CLI's visible catalog:** `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, and `gpt-5.3-codex-spark`, with one-line descriptions from the refreshed catalog. Free-text passthrough still covers future, legacy, and private model names.
3. **Claude seed adds `fable`**; keeps `opus`, `sonnet`, `haiku`. Aliases are version-stable so the list doesn't rot.
4. **Effort lists stay per-runtime, not per-model.** codex gains `max` and (if the installed CLI accepts it) `ultra`. Per-model narrowing (sol-only tiers) is complexity the free-form enum doesn't need yet.
5. **Verify against the installed CLIs, not docs.** On codex-cli 0.146.0 the prior invalid-value probe no longer returns the enum error, so the implementation records the refreshed catalog evidence instead: Sol/Terra advertise through `ultra`, Luna through `max`, and the remaining visible models through `xhigh`.
6. **Direct chat defaults remain true sentinels.** Model and effort display `default`, serialize as `null`, and emit no CLI flag until the user selects a value. Runtime-only sessions persist chosen model/effort beside their recorded agent command so resume does not silently return to CLI defaults.
7. **Agent availability is detected and user-controlled.** Selection surfaces offer only agents with a detected executable or valid override and whose Settings → Agents toggle is enabled. Toggles default enabled; disabling an agent preserves its override and existing runners but hides it from new Start Chat and runner create/edit choices. Internal APIs and stored fields keep the precise `runtime` term, while visible picker copy uses `Agent`.

## Goals

- Codex Model combobox shows `default` + the seven models visible in the verified refreshed catalog; typing any other name still works and persists.
- Claude Model combobox shows `fable` alongside the existing aliases.
- Codex Effort dropdown offers `max` (and `ultra` if verified accepted); existing stored values keep loading (the edit drawer's safe-value guard must not coerce a stored `max` once it is a listed option).
- Direct chat creation exposes Model and Thinking effort with `default` selected semantically, and runtime-only resume preserves explicit choices.
- Start Chat and runner create/edit Agent selectors show detected, enabled agents only; Settings → Agents owns a default-enabled toggle for each catalog agent.

## Non-Goals

- Dynamic `model/list` fetching over `codex app-server` (documented in #386, not built).
- Per-model effort menus, `opus[1m]`-style variant aliases, or model pickers for qoder/trae beyond `default`.
- #380 (surfacing default model/effort in the Agent view) — this refresh feeds it later but doesn't build it.

## Implementation Notes

- `src/components/ui/runtimes.ts` — `MODEL_SUGGESTIONS_BY_RUNTIME` (codex seed, claude `fable`), `EFFORT_OPTIONS_BY_RUNTIME` (codex `max`/`ultra`), and the stale comments beside both rewritten to match the verified catalog.
- `src/components/RunnerEditDrawer.tsx` — keep the existing effort safe-value guard behavior with the widened list and pin it with coverage; the guard itself needs no behavior change.
- `src/components/StartChatModal.tsx`, `src/lib/api.ts`, and the session command/manager — pass Direct mode model/effort into the transient runner and persist them for runtime-only resume.
- `src/components/settings/AgentsPane.tsx`, `src/lib/settings.ts`, and shared selector helpers — persist default-enabled agent toggles and filter selections by executable status plus user preference.
- `src/components/CreateRunnerModal.tsx` and `src/components/RunnerEditDrawer.tsx` — consume the same detected/enabled Agent option source while preserving an existing runner's disabled current agent until the user switches away.

## Validation

- Vitest: existing runtimes/ModelField tests extended — exact codex seed, claude list includes `fable`, codex effort list includes `max`; stored-value coercion test for a runner row with `effort: "max"`; Direct mode sentinel/pass-through tests; executable discovery refresh and detected/enabled filtering tests; runner create/edit filtering tests; Agents toggle persistence test.
- Rust: runtime-only spawn records model/effort, and resume reconstructs the transient runner with both values.
- CLI probes (documented in the updated comments): codex-cli 0.146.0 refreshed catalog inspected and the former invalid-value enum probe confirmed no longer diagnostic; claude alias set sanity-checked against `claude --help` model flag text.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual: create-runner and edit-drawer combos show the new entries; a free-typed unseeded model persists and round-trips; disabling an agent hides it from new choices without changing an existing runner.
