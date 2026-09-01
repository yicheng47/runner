# Local skills — plan

Milestones for [feature 73](../../features/73-runner-skills.md). Sizes: S < 50 lines, M 50–300, L > 300 or cross-cutting. Line numbers are as of `main` at `3479624` (2026-08-28) and will drift. Each milestone is one mission; its brief goes under [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/) when drafted (`local-skills-m1-skills-pane.md`, …) and the landing moves the section into [impl_log.md](impl_log.md).

Order, decided 2026-08-28 (Jason): **see first, control later.** M1 is only a place to view skills by runtime; the allowlist follows in two steps, backend then app.

## S0 — Design (Jason, Pencil) — S

In `design/runner.pen`, reusing the Agents card as the grammar; skills-manager's Global Workspace page and skill sheet (`~/repos/ai/skills-manager`, spec §Reference) are the layout reference. For M1 only the first item is needed:

- **Settings → Skills pane**: one card per runtime; header line (runtime name, faint scanned path, `N skills`, Refresh); a search field; the skill row (name, faint one-line description, flag badges `manual` / `hidden` / `symlink` / `problem`, the global Toggle on claude-code rows with the off state at reduced opacity and an `Other(value)` badge, Reveal in Finder + Copy name on hover); the detail panel (title, description, path + `SKILL.md` link, file list, rendered body with a frontmatter table); the two captions (claude-code: bundled skills exist off-disk; codex: no per-session control); the empty / missing-root row.
- Before M3: the runner-form Skills field (`All skills` / `Only selected…`), the checklist popup (search, flat checkbox rows), the chip summary with the `missing` tone, the budget hint; runner-handle chips per skill row (reverse index); the detail-page chips.

Gate: nodes exist and the milestone's brief points at their ids.

## M1 — View and switch skills by runtime — M, `runner-backend` + `runner-app`

Viewing plus the claude-code global toggle. Nothing about how Runner spawns changes; the toggle edits what Claude Code loads everywhere (spec §Global on/off).

1. **Catalog** — new `crates/runner-backend/src/skills.rs` (register in `lib.rs`): `SkillCatalog { runtime, root: PathBuf, entries: Vec<SkillEntry> }`, `SkillEntry { name, description, path, symlink: Option<PathBuf>, manual: bool (disable-model-invocation), hidden: bool (user-invocable: false), problem: Option<String> }`; `pub fn skill_catalog(runtime: &str, home: &Path, codex_home: Option<&Path>) -> Option<SkillCatalog>` — `None` for runtimes without a skills dir (qoder, trae). Roots: claude-code `~/.claude/skills`, codex `$CODEX_HOME/skills` (env, else `~/.codex/skills`); a missing root yields an empty catalog with `root_exists: false`. Walk rules follow skills-manager (spec §Reference): a directory entry is a skill when it contains `SKILL.md` (exact case; `skill.md` accepted and flagged `problem: "legacy skill.md"`); dot-directories and non-directories are skipped; a directory without a marker that contains a `skills/` subtree is skipped as an embedded bundle, any other markerless directory becomes a `problem: "no SKILL.md"` row so the user sees it rather than wonders; a canonicalized `visited` set terminates symlink loops; entries sort case-insensitively by name. `name` = frontmatter `name` sanitized to one path component, else the folder name; `description` from frontmatter; frontmatter is the leading `---` block of `key: value` lines — parse `name`, `description`, `disable-model-invocation`, `user-invocable` only, no YAML crate; an unparsable block is `problem`, never an error. The entry also carries `files: Vec<String>` (top-level names in the skill folder, for the detail panel) and `global: GlobalState { On, Off, Other(String) }` read from `~/.claude/settings.json` → `skillOverrides[name]` (absent → `On`); codex entries are always `On`. Reuse the home-dir pattern from `runtime_defaults.rs` (feature 65) for path injection. Roots come from one new field on `RuntimeDefinition` (`router/runtime.rs:31`): `skills_dir: Option<&'static str>` (`.claude/skills`, `.codex/skills`, `None` for qoder/trae).
2. **Ops** — `ops::skills::skill_catalogs(core) -> Vec<SkillCatalog>` over the runtimes that have one, resolving `$HOME` once; `ops::skills::set_global_enabled(core, runtime, name, enabled) -> Result<SkillCatalog>` — claude-code only (other runtimes → error), structured read-modify-write of `skillOverrides` in `~/.claude/settings.json` following `ops/mcp.rs`'s JSON handling (`:160`): off sets `"off"`, on removes the entry, a missing file is created with just the key, an unparsable file is left alone and the error returned; returns the re-scanned catalog. No MCP tool in M1.
3. **Pane** — `surfaces/settings/skills.rs`, a `SettingsSection::Skills` between `Agents` and `Mcp` (`settings_page.rs:78`, plus the section list and route); reads the catalogs on open and on **Refresh** (pattern: `AgentsPane::refresh`, `agents.rs:176`); one card per catalog with the header, a search `TextField` over name + description (skills-manager's `Search agent skills...`), rows and captions per S0; badges from `runtime_badge`'s tone set (`agents.rs:940`); **Reveal in Finder** through the same open-path call Diagnostics uses for the log; **Copy name** via `CopyValueButton`; an empty or missing root renders one faint row. Each claude-code row carries a `Toggle` (`ui/toggle.rs`, as the Agents rows do) bound to `global`, calling `set_global_enabled` and replacing the card from the returned catalog; off rows at reduced opacity; `Other(value)` rendered as a badge beside the toggle. Row click opens the detail panel (spec §Surfaces): title, description, path + `SKILL.md` reveal link, file list, and the body rendered through `surfaces/mission_markdown.rs` with the frontmatter as a key/value table — rendered from the file on open, not cached.
4. **Tests** — `set_global_enabled` over a temp settings file: off sets `"off"`, on deletes the entry, sibling keys and other `skillOverrides` entries preserved, missing file created with only the key, unparsable file untouched + error, codex → error. `skills.rs` over temp dirs: frontmatter name/description/flags; global state absent / `on` / `off` / other; folder-name fallback; sanitized frontmatter name; malformed and missing `SKILL.md` → `problem`; legacy `skill.md` accepted and flagged; dot-directory skipped; embedded-bundle directory skipped; symlinked folder → `symlink` target recorded; a symlink loop terminates; non-directory entries ignored; case-insensitive sort; `files` listed; codex root from `CODEX_HOME`; missing root → `root_exists: false`; unknown runtime → `None`. Pane: `settings_page` section list includes Skills; a pure helper for the row's badge set and description first-sentence clamp.

Gate: `make verify`; reviewer clean; Jason's smoke test — the pane lists the 36 personal skills under Claude Code and the same 36 under Codex (all flagged `symlink` — the memory repo installs them that way), `daily-brief` shows no other flags, a deliberately broken `SKILL.md` in a scratch folder shows `problem`, search narrows by description text, the detail panel renders `daily-brief`'s body, Reveal opens the folder, Refresh picks up an added folder; toggling `music` off writes exactly `"skillOverrides": {"music": "off"}` into `~/.claude/settings.json` (diff the file), a fresh `claude` outside Runner no longer lists `/music`, toggling on removes the key; codex rows have no toggle.

## M2 — Allowlist, backend — M, `runner-backend` only

No UI. Behaviour-preserving until a runner is given picks over MCP.

1. **Schema + model** — migration `0021_runner_skills.sql`: `ALTER TABLE runners ADD COLUMN skills_json TEXT NULL`. `Runner.skills: Option<Vec<String>>` in `repo/runner.rs` (column lists at `:55` and `:72`), `CreateRunnerInput` / `UpdateRunnerInput` (`ops/runner.rs:27`, `:70`; update uses `Option<Option<Vec<String>>>` like `model`), MCP `runner_create` / `runner_update` / `runner_get` / `runner_list`, plus `skill_list { runtime }` returning M1's catalog.
2. **Global set at spawn** — `global` for a spawn = the names in the runtime's global directory whose global state is not `Off`, listed at spawn time by the same `skill_catalog` (M1); no project dimension, no bundled set — both always load (spec §Catalog).
3. **Spawn** — `router/runtime.rs`: `claude_settings_args(runtime, runner_args, skills: Option<&[String]>, global: &[String])`. Emits one `--settings` JSON: today's `{"tui":"fullscreen"}` plus `skillOverrides` = `{name: "off"}` for every `global` name not in `skills`, only when `skills` is `Some`. Merge rule (decision 4): if `runner_args` carry `--settings X` or `--settings=X`, read `X` as inline JSON or as a file, deep-merge Runner's keys on top (a `skillOverrides` map in `X` is unioned; the runner's explicit entries win), emit one flag, and strip the runner's from the argv it returns alongside (`compose` at `:583` must apply the stripped argv). Unreadable → warn, Runner's payload alone. `global` is assembled at the spawn call site (`session/manager/spawn.rs`); direct chats and mission slots both pass the runner's `skills`.
4. **Tests** — `claude_settings_args`: `None` → byte-identical to today; subset → exact complement of `global`; unknown picks absent; merge with inline JSON, with a file, with an unreadable file, and the argv strip; non-claude untouched. Repo/ops/MCP: `skills` round-trips as `null` and as an array; `runner_update` with `skills: null` clears.

Gate: `make verify`; reviewer clean; a runner updated over MCP with `skills: ["daily-brief","scan"]` spawns a chat whose argv (visible in the pane's command line) carries the override map, "list every skill you can see" in that chat returns the two plus bundled, and `shasum ~/.claude/settings.json` is unchanged.

## M3 — Allowlist, app — M, `runner-app`

Against the S0 nodes.

1. **Pane addition** — per skill, the runners that restrict to it as handle chips from `runner_list` (reverse index).
2. **Runner form** — `surfaces/runners.rs`: a `Field::new("…-skills", "Skills", …)` after the Model field (`:951` create, `:1412` edit), present only when the form's runtime is `claude-code` (follow the runtime-change path that swaps `runtime_models`); a `StyledSelect` (`All skills (M)` / `Only selected…`) whose second option opens a checklist popup built like `ModelField`'s suggestions (`ui/model_field.rs`) with a search `TextField`, flat rows, and a checkbox glyph (no checkbox widget exists — a 12 px square with the accent fill, drawn inline); chips under the field; the `missing` tone for names absent from the catalog; the budget hint past 20 picked. Stored value survives a runtime switch away and back.
3. **Runner detail** — chips under a `Skills` label in the detail page's config block (feature 58 layout).
4. **Tests** — pure helpers for picked count, hint threshold, missing detection, option-label text, popup filtering; existing runner-form tests extended for the field's runtime gating.

Gate: `make verify`; reviewer clean; Jason's smoke test — two runners with different picks, "list every skill you can see" from each chat matches the picks plus Claude Code's bundled skills; a runner with `--settings '{"theme":"dark"}'` in `args` keeps the theme and gets the overrides.

## Later, each its own issue when felt

- **Slot-level override** in the crew editor's slot drawer, feature 59 pattern (`slots.skills_override_json`), resolution runner → slot.
- **`--plugin-dir` kits** for skills outside `~/.claude/skills`: `<app-data>/kits/<handle>/` with copied skill folders (never symlinked — Claude Code skips them silently), appended at spawn; namespaced `/<handle>:<skill>`.
- **Codex** once a per-launch visibility control exists; until then the catalog card only.
- **Per-chat override** in Start Chat.
- **Budget control** — exposing `skillListingBudgetFraction` per runner, if the hint alone proves insufficient.

## Open questions

- Before M2: `skill_list` MCP tool scope — catalog only (current answer; the effective set is derivable).

## Risks

- Claude Code changes `skillOverrides` semantics or the listing budget — the manual gate ("list every skill you can see") is the detector; record the CLI version in the log at each landing.
- The `--settings` merge changes a contract runners relied on (their flag winning outright). Decision 4 keeps their explicit keys winning; the M2 brief must say so and the reviewer hunts for a case where Runner's key clobbers a runner's.
- Claude Code's bundled set drifts per version and counts against the listing budget outside Runner's control — the hint's threshold is about picks only; revisit if the budget is felt.
