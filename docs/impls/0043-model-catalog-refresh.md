# Model catalog refresh: seed codex, add fable, catch up effort enums

## Status

Planned. Tracking issue [#386](https://github.com/yicheng47/runner/issues/386) — resolved in reduced form: the dynamic `codex app-server model/list` path stays documented in the issue but is not built; the catalog stays hardcoded with free-text passthrough, per the Orca precedent (`agent-session-option-catalog-claude-codex.ts`: "keep this seed short and allow unknown persisted ids to pass through").

## Problem

The picker machinery already exists — `ModelField` is an editable combobox over `MODEL_SUGGESTIONS_BY_RUNTIME`, with `default` (empty → no `--model` flag) listed explicitly and free text always allowed; `EFFORT_OPTIONS_BY_RUNTIME` feeds the effort dropdowns. But the data is stale in three ways:

1. **codex offers only `default`** — the old comment ("codex has no alias scheme — full names rot every release") predates a usable equilibrium: a short seed of current names plus passthrough covers reality, and is what Orca ships.
2. **claude-code's list predates the Claude 5 family** — no `fable` alias, while `claude --model fable` is the top-tier pick.
3. **codex's effort list stops at `xhigh`** — the enum comment was verified against codex-cli 0.130.0; current codex (0.146.0) advertises and accepts `max` (in daily use here) and `ultra` for some models.

## Key Decisions

1. **Hardcoded seed + `default` + free text, both runtimes. No CLI querying.** The dynamic `model/list` fetch (verified working, see #386) is deliberately not built: it adds a subprocess JSON-RPC dance to solve staleness that passthrough already absorbs. Revisit trigger recorded in #386.
2. **Codex seed stays short** (current generation only): `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5`, with one-line descriptions from the CLI's own `model/list` metadata. Older names are typed, not listed.
3. **Claude seed adds `fable`**; keeps `opus`, `sonnet`, `haiku`. Aliases are version-stable so the list doesn't rot.
4. **Effort lists stay per-runtime, not per-model.** codex gains `max` and (if the installed CLI accepts it) `ultra`. Per-model narrowing (sol-only tiers) is complexity the free-form enum doesn't need yet.
5. **Verify against the installed CLIs, not docs.** The codex enum is cheaply probed by passing an invalid value (`-c model_reasoning_effort=bogus`) to a config-loading subcommand and reading the `unknown variant … expected one of …` error, the same technique as the 0.130.0 comment. Update the comment's version stamp with what was verified.

## Goals

- Codex Model combobox shows `default` + the four seeded names with descriptions; typing any other name still works and persists.
- Claude Model combobox shows `fable` alongside the existing aliases.
- Codex Effort dropdown offers `max` (and `ultra` if verified accepted); existing stored values keep loading (the edit drawer's safe-value guard must not coerce a stored `max` once it is a listed option).
- No backend changes: `model_effort_args` already passes values verbatim.

## Non-Goals

- Dynamic `model/list` fetching over `codex app-server` (documented in #386, not built).
- Per-model effort menus, `opus[1m]`-style variant aliases, or model pickers for qoder/trae beyond `default`.
- #380 (surfacing default model/effort in the Agent view) — this refresh feeds it later but doesn't build it.

## Implementation Notes

- `src/components/ui/runtimes.ts` — the whole change lives here: `MODEL_SUGGESTIONS_BY_RUNTIME` (codex seed, claude `fable`), `EFFORT_OPTIONS_BY_RUNTIME` (codex `max`/`ultra`), and the stale comments beside both (the "no alias scheme" rationale and the 0.130.0 enum stamp) rewritten to match the new stance.
- `src/components/RunnerEditDrawer.tsx` — verify the effort safe-value guard behavior with the widened list; no code change expected.
- Consumers (`ModelField`, `CreateRunnerModal`, `AddSlotModal`, `RunnerEditDrawer`) need no changes.

## Validation

- Vitest: existing runtimes/ModelField tests extended — codex suggestions non-empty, claude list includes `fable`, codex effort list includes `max`; stored-value coercion test for a runner row with `effort: "max"`.
- CLI probes (documented in the updated comments): codex effort enum via invalid-value error on codex-cli 0.146.0; claude alias set sanity-checked against `claude --help` model flag text.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual: create-runner and edit-drawer combos show the new entries; a free-typed unseeded model (e.g. `gpt-5.4-mini`) persists and round-trips.
