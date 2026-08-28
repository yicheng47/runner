# Local skills — plan

Milestones for [feature 05](../../features/05-runner-skills.md). Sizes: S < 50 lines, M 50–300, L > 300 or cross-cutting. Line numbers are as of `main` at `156a2b8` (2026-08-28) and will drift. Each milestone is one mission; its brief goes under [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/) when drafted (`local-skills-s1-backend.md`, …) and the landing moves the section into [impl_log.md](impl_log.md).

## S0 — Design (Jason, Pencil) — S

In `design/runner.pen`, reusing the Agents card and the Model field's suggestion popup as the grammar:

- **Settings → Skills pane**: one card per runtime; header line (runtime name, `N personal · M bundled`, Refresh, Probe bundled skills with the cached version / *stale* / *never probed*); the skill row (name, faint one-line description, source badge `personal` / `project` / `bundled`, flag badges `manual` / `hidden` / `symlink` / `problem`, runner-handle chips for who restricts to it, Reveal in Finder + Copy name on hover); the codex card's caption.
- **Runner form — Skills field** under Model: the select with `All skills (51 visible)` / `Only selected…`; the checklist popup (search on top, `Personal` / `Project` / `Bundled` groups, checkbox rows with name + description); the chip summary line with the `missing` tone; the budget hint past 20.
- **Runner detail**: the chips under a `Skills` label.

Gate: nodes exist and Jason has pointed the S2 brief at their ids. Not blocking S1.

## S1 — Backend: catalog, schema, spawn merge — M, `runner-backend` only

No UI. Ships alone; every existing runner keeps today's behaviour.

1. **Catalog** — new `crates/runner-backend/src/skills.rs` (register in `lib.rs`): `SkillCatalog { entries: Vec<SkillEntry> }`, `SkillEntry { name, description, source: Personal | Project | Bundled, path: Option<PathBuf>, symlink: bool, manual: bool (disable-model-invocation), hidden: bool (user-invocable: false), problem: Option<String> }`; `pub fn skill_catalog(runtime, home: &Path, cwd: Option<&Path>, bundled: &[String]) -> SkillCatalog`. Frontmatter is a `---` block of `key: value` lines — parse those keys only, no YAML crate; `name` falls back to the folder name; a missing or malformed `SKILL.md` is an entry with `problem`. Project skills: `<cwd>/.claude/skills` walked up to the repo root (the `.git` boundary), as Claude Code does. Precedence for duplicate names: personal > project > bundled, the loser dropped with `problem = "shadowed by <source>"`. Codex: `$CODEX_HOME/skills` (env, else `~/.codex/skills`) and `<cwd>/.agents/skills`, no bundled set.
2. **Bundled probe** — `pub fn probe_bundled_skills(command: &str) -> Result<Vec<String>>` runs `claude -p --output-format stream-json --verbose --max-turns 0 ""` (or the cheapest equivalent that still emits `init`; verify against 2.1.250 — the recording harness used a one-word prompt), parses the first `init` line's `skills`, subtracts the scanned personal + project names, returns the rest. Cached in the settings store as `{ version, names, probed_at }` keyed by the runtime's `--version` output; `runtime_defaults`-style path injection for tests, the real subprocess behind a trait or a feature-gated test.
3. **Schema + model** — migration `0021_runner_skills.sql`: `ALTER TABLE runners ADD COLUMN skills_json TEXT NULL`. `Runner.skills: Option<Vec<String>>` in `repo/runner.rs` (the column lists at `:55` and `:72`), `CreateRunnerInput` / `UpdateRunnerInput` (`ops/runner.rs:27`, `:70`; update uses `Option<Option<Vec<String>>>` like `model`), MCP `runner_create` / `runner_update` / `runner_get` / `runner_list`, plus `skill_list { runtime, cwd? }` returning the catalog.
4. **Spawn** — `router/runtime.rs`: `claude_settings_args(runtime, runner_args, skills: Option<&[String]>, known: &[String])`. Emits one `--settings` JSON: today's `{"tui":"fullscreen"}` plus `skillOverrides` = `{name: "off"}` for every `known` name not in `skills`, only when `skills` is `Some`. Merge rule (decision 4): if `runner_args` carry `--settings X` or `--settings=X`, read `X` as inline JSON or as a file, deep-merge Runner's keys on top (a `skillOverrides` map in `X` is unioned; the runner's explicit entries win), emit one flag, and strip the runner's from the argv it returns alongside (`compose` at `:583` must apply the stripped argv). Unreadable → warn, Runner's payload alone. `known` is assembled at the spawn call site (`session/manager/spawn.rs`) from `skill_catalog(...)` for the session's cwd plus the bundled cache; direct chats and mission slots both pass the runner's `skills`.
5. **Tests** — `skills.rs`: frontmatter keys, folder fallback, malformed file, precedence and shadowing, project walk-up stops at `.git`, symlink flag, codex dirs. Probe: parser over a recorded `init` line; cache honoured only on version match. `claude_settings_args`: `None` → byte-identical to today; subset → exact complement; unknown picks absent; bundled hidden only with a cache; merge with inline JSON, with a file, with an unreadable file, and the argv strip; non-claude untouched. Repo/ops/MCP round-trip of `null` and arrays; `runner_update` with `skills: null` clears.

Gate: `make verify`; reviewer clean; a runner updated over MCP with `skills: ["daily-brief","scan"]` spawns a chat whose argv (visible in the pane's command line) carries the override map, and `shasum ~/.claude/settings.json` is unchanged.

## S2 — App: Skills pane, runner form, detail — M, `runner-app`

Against the S0 nodes.

1. **Settings → Skills** — `surfaces/settings/skills.rs`, a `SettingsSection::Skills` between `Agents` and `Mcp` (`settings_page.rs:78`); reads `skill_catalog` per runtime on open and on Refresh (pattern: `AgentsPane::refresh`, `agents.rs:176`); Probe button drives `probe_bundled_skills` off the UI thread with a spinner and writes the cache; rows and badges per S0; reverse index from `runner_list`; Reveal in Finder via the existing open-path helper used by Diagnostics; Copy name via `CopyValueButton`.
2. **Runner form** — `surfaces/runners.rs`: a `Field::new("…-skills", "Skills", …)` after the Model field (`:951` create, `:1412` edit), present only when the form's runtime is `claude-code` (follow the runtime-change path that swaps `runtime_models`); a `StyledSelect` (`All skills` / `Only selected…`) whose second option opens a checklist popup built like `ModelField`'s suggestions (`ui/model_field.rs`) with a search `TextField`, grouped rows, and a checkbox glyph (no checkbox widget exists — a 12 px square with the accent fill, drawn inline); chips under the field; the `missing` tone for names absent from the catalog; the budget hint past 20 visible. Stored value survives a runtime switch away and back.
3. **Runner detail** — chips under a `Skills` label in the detail page's config block (feature 58 layout).
4. **Tests** — pure helpers for: visible count, hint threshold, missing detection, option-label text, popup filtering; `settings_page` section list; existing runner-form tests extended for the new field's runtime gating.

Gate: `make verify`; reviewer clean; Jason's smoke test — two runners with different picks, "list every skill you can see" from each chat matches the picks plus bundled; a runner with `--settings '{"theme":"dark"}'` in `args` keeps the theme and gets the overrides; the pane's Probe fills the bundled group and survives relaunch.

## S3 — Later, each its own issue when felt

- **Slot-level override** in the crew editor's slot drawer, feature 59 pattern (`slots.skills_override_json`), resolution runner → slot.
- **`--plugin-dir` kits** for skills outside `~/.claude/skills`: `<app-data>/kits/<handle>/` with copied skill folders (never symlinked — Claude Code skips them silently), appended at spawn; namespaced `/<handle>:<skill>`.
- **Codex** once a per-launch visibility control exists; until then the catalog card only.
- **Per-chat override** in Start Chat.
- **Budget control** — exposing `skillListingBudgetFraction` per runner, if the hint alone proves insufficient.

## Open questions (decide before S1 starts)

- Probe cost: one model call per click is accepted (spec); confirm `--max-turns 0` still yields `init` on the current CLI, otherwise the one-word prompt stays.
- Whether the bundled list should be probed lazily on first restricted spawn (invisible, costs a call the user did not ask for) — current answer no, pane-only.
- `skill_list` MCP tool scope: catalog only, or also the per-runner effective set? Current answer catalog only; the effective set is derivable.

## Risks

- Claude Code changes `skillOverrides` semantics or the listing budget — the manual gate ("list every skill you can see") is the detector; record the CLI version in the log at each landing.
- The `--settings` merge changes a contract runners relied on (their flag winning outright). Decision 4 keeps their explicit keys winning; the brief must say so and the reviewer hunts for a case where Runner's key clobbers a runner's.
- The bundled set drifts per version and the cache lags — decision 3 fails safe (never hidden without a fresh cache).
