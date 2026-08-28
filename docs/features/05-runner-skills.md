# 05 — Local skills management

Tracking: [#73](https://github.com/yicheng47/runner/issues/73). Status: specced 2026-08-28, not scheduled. Implementation program: [`docs/impls/local-skills/`](../impls/local-skills/README.md) (milestones S0 design → S1 backend → S2 app → S3 later).

> Rewritten 2026-08-28. The 2026-07-15 text at this number described an agent-agnostic MCP + skills catalog modelled on skills-manager (central library, per-agent sync ladder, adopt-into-catalog). That framing answered "where do skills live"; the problem Jason actually has is "which skills does *this session* see". Claude Code has shipped the primitives that answer it since then, so this spec narrows to skills, per runner, claude-code first, and defers the MCP catalog to a later spec. The earlier text stays in git history.

## Motivation

A session sees every skill installed on the machine. On this machine that is 36 user skills under `~/.claude/skills/` (the memory repo's `install.sh` symlinks the same set into `~/.claude/skills` and `~/.codex/skills`) plus Claude Code's own bundled skills — 51 names in the model's context. Two things go wrong at that size:

- **Wrong skill.** A trading runner has `lark-*`, `media`, `music` and `books` in front of it; a reviewer runner has `daily-brief`. Every extra description is a chance for the model to pick a skill that does not belong to the role.
- **Silent loss.** Since Claude Code 2.1.129 the skill listing has a budget — `skillListingBudgetFraction`, default 1 % of the context window, roughly 15–25 skills — and past it Claude Code drops whole descriptions for the least-used skills, ranked by recency and frequency. With 51 loaded, some skills are invisible to the model on any given launch, and which ones depends on history. A runner cannot rely on its skill being seen.

Runner is the process that spawns these sessions and already passes `--settings` to every claude-code spawn (`claude_settings_args`, `crates/runner-backend/src/router/runtime.rs`). Claude Code's `skillOverrides` setting is exactly a per-name visibility map, and it rides that flag. Verified on claude-code 2.1.250 on 2026-08-28: injecting `--settings '{"skillOverrides":{…34 names…:"off"}}'` into a headless launch took the model's own reported skill list from 51 names to 15 (the two kept plus 13 bundled), with `~/.claude/settings.json` byte-identical before and after. The earlier leak bugs (anthropics/claude-code #54996, #50631: `"off"` still listed to the model) were closed 2026-05-04 and do not reproduce.

So the feature is small at its core: **a runner declares which skills it wants, Runner hides the rest at spawn.** Nothing is copied, symlinked, or written into the user's agent homes.

## Principles

- **Read-only over the user's homes.** Runner reads `~/.claude/skills` and the project's `.claude/skills`; it never writes there. Same posture as MCP registration (only `mcp_servers.runner` is ever touched) and feature 65's config reads.
- **Allowlist, computed at spawn.** The runner stores what it *wants*; the `"off"` set is derived from what exists at spawn time. A skill added next week is hidden from restricted runners automatically instead of leaking in.
- **Scan, don't trust.** The catalog is derived from the filesystem on every read; Runner keeps no parallel list that can drift.
- **claude-code first, honestly.** Codex has skills but no per-launch visibility control. The picker is hidden for other runtimes rather than pretending.
- **Pencil-first.** The Skills pane and the picker are drawn in `design/runner.pen` before the app phase (README §Decisions that still bind).

## Mechanism

### What Claude Code provides (2.1.250)

| primitive | scope | what it does |
|---|---|---|
| `skillOverrides: { name: "on" \| "name-only" \| "user-invocable-only" \| "off" }` | settings key | `"off"` hides the skill from the model and the `/` menu; `"name-only"` keeps the name without the description. Plugin skills are exempt. |
| `--settings <file-or-json>` | per launch | merges on top of the user's settings for that session only. Runner already sends `{"tui":"fullscreen"}`. |
| `disable-model-invocation: true` (frontmatter) | per skill, permanent | description never enters context; `/name` still works. Runner reads it, never sets it. |
| `skillListingBudgetFraction`, `skillListingMaxDescChars`, `SLASH_COMMAND_TOOL_CHAR_BUDGET` | settings / env | the listing budget. Runner reports against it; v1 does not change it. |
| `--plugin-dir <dir>` | per launch | loads a plugin (skills namespaced `/plugin:skill`) for the session only. The route for skills that are **not** in the global set — phase 3, not v1. Per-skill symlinks inside a plugin are skipped silently; only a symlinked `skills/` directory or real folders load. |

Sources the model sees, by name: `~/.claude/skills/<name>/SKILL.md` (personal), `<cwd>/.claude/skills/<name>/` up to the repo root (project, after workspace trust), bundled skills shipped inside the CLI (`code-review`, `simplify`, `loop`, `schedule`, `claude-api`, `dataviz`, `update-config`, `keybindings-help`, `fewer-permission-prompts`, `workflow-authoring`, `run`, `init`, `security-review` on 2.1.250 — the list drifts by version), and plugin skills.

### Catalog (derived)

`runner-backend` gains `skills.rs`: `pub fn skill_catalog(runtime: &str, home: &Path, cwd: Option<&Path>) -> SkillCatalog`. Pure over injected paths, never errors; a malformed `SKILL.md` becomes an entry with a `problem` string, not a failure.

- **claude-code**: personal dir + project dir walk (frontmatter `name` if present, else folder name; `description`; `disable-model-invocation`; `user-invocable`; path; symlink target if any) + the cached bundled list. Precedence as Claude Code applies it: personal over project for the same name, project over bundled.
- **codex**: `$CODEX_HOME/skills/<name>/SKILL.md` (`~/.codex/skills`) + `<cwd>/.agents/skills`. Listed for information; no visibility control exists, and the entry says so.
- **qoder / trae**: not scanned in v1.

Bundled skills cannot be read from disk — they live inside the CLI. Runner learns them with one explicit probe: `claude -p --output-format stream-json --verbose --max-turns 0 ""`-style headless launch whose `init` message carries `skills` and `slash_commands`; the names that are not in any scanned directory are the bundled set. The probe costs a model call, so it runs only from the Skills pane's **Probe bundled skills** button, and its result is cached in the settings store keyed by `claude --version`; a version change marks the cache stale and the pane says so. No probe → bundled skills are treated as unknown and left `"on"` (they are never hidden by accident).

### Runner model

- Migration `0021_runner_skills.sql`: `ALTER TABLE runners ADD COLUMN skills_json TEXT NULL`. `NULL` = all skills (today's behaviour, the default for every existing runner); a JSON array of names = only these.
- `Runner.skills: Option<Vec<String>>` through repo, ops (`CreateRunnerInput` / `UpdateRunnerInput` gain `skills`), and the MCP tools `runner_create` / `runner_update` / `runner_get` / `runner_list`. A new read-only MCP tool `skill_list { runtime, cwd? }` returns the catalog so an external controller can build the array from real names.
- Names are stored as the user picked them; a stored name that no longer exists is kept (it may come back) and shown as *missing* in the form, never silently dropped.

### Spawn

`claude_settings_args(runtime, runner_args)` becomes `claude_settings_args(runtime, runner_args, skills: Option<&[String]>, known: &[String])` and emits one `--settings` JSON:

```json
{ "tui": "fullscreen", "skillOverrides": { "<every known name not in skills>": "off" } }
```

- `known` is computed at spawn: personal dir ∪ project dir for the session's cwd ∪ cached bundled names. Computed at spawn so the allowlist tracks the filesystem, not the form's last view.
- `skills == None` → no `skillOverrides` key at all; the emitted flag is byte-identical to today's.
- **Merging with a runner's own `--settings`.** Today a `--settings` in the runner's `args` short-circuits Runner's flag entirely. That stays correct only while Runner's payload is cosmetic; with `skillOverrides` in it, Runner must merge: read the runner's value (inline JSON or a file path), deep-merge Runner's keys on top (`skillOverrides` entries union, the runner's explicit entries win), emit one combined `--settings`, and drop the runner's flag from the argv. A file that cannot be read → log, fall back to Runner's payload alone, and surface the problem on the runner card.
- Direct chats spawned from a runner (Start Chat) inherit the runner's `skills`; missions inherit per slot from the slot's runner. Slot-level override is phase 3.
- Non-claude runtimes: `skills` is ignored at spawn and the form hides the field.

### Budget awareness

The picker shows `N visible` where N = picked count + bundled count (or the full known count when `skills == None`), and an inline note past 20: *"Above ~20 skills Claude Code starts dropping descriptions (listing budget, 1 % of context). Pick fewer or tighten descriptions."* It is a hint, not a gate; v1 does not set `skillListingBudgetFraction`.

## Surfaces

### Settings → Skills (new pane, between Agents and MCP)

One card per runtime that has a catalog (claude-code, codex), same card grammar as Agents:

- Header: runtime name, `N personal · M bundled` (claude) or `N personal` (codex), a **Refresh** ghost button, and for claude-code **Probe bundled skills** with the cached version or *stale* / *never probed*.
- A list of skills: name, first sentence of the description (faint, one line, ellipsised), a source badge (`personal`, `bundled`, `project` only when a project cwd is chosen in the pane's cwd field), and flags as tiny badges: `manual` (`disable-model-invocation`), `hidden` (`user-invocable: false`), `symlink`, `problem` in danger tone with the parse error on hover.
- Per skill, the runners that restrict to it, as small handle chips (`@trader`, `@reviewer`) — the reverse index answers "who uses this" without a second surface.
- Row actions: **Reveal in Finder** (opens the folder), **Copy name**.
- Read-only: no create, edit, delete, enable. Editing a skill is the user's editor's job; enabling is the runner form's job. The codex card carries one caption: *"Codex has no per-session skill control; all of these load in every codex session."*

### Runner create / edit

A **Skills** field directly under **Model** (claude-code runtimes only; the field is removed, not disabled, for others):

- A `StyledSelect` with two options: **All skills** (`51 visible`) and **Only selected…**. Choosing the second opens a checklist popup (search field on top, grouped `Personal` / `Project` / `Bundled`, each row name + one-line description, checkbox) — the popup component follows `ModelField`'s suggestion popup and the select's option list; occlusion rules from feature 62 apply.
- Below the field a summary line: the picked names as chips (handle-style), a `missing` chip tone for names no longer on disk, and the budget hint when over 20.
- Switching the Agent select to a non-claude runtime hides the field and keeps the stored value (it comes back if the runtime does); switching back restores it.
- The runner detail page shows the same chips under a **Skills** label; the runner card in the list shows nothing new.

### MCP

`runner_create` / `runner_update` accept `"skills": null | ["name", …]`; `runner_get` returns it; `skill_list` returns the catalog. The `runner` CLI is untouched.

## Non-Goals (v1)

- The MCP-server catalog half of #73 — a later spec; Runner's own MCP registration (`Settings → MCP`) is unrelated and stays.
- Creating, editing, deleting or installing skills; git sources, update checks, content hashes, marketplaces, the skills-manager adopt flow. `Reveal in Finder` is the whole authoring story.
- Per-chat overrides in Start Chat, and per-slot overrides in the crew editor (phase 3).
- Changing the listing budget, or writing `disable-model-invocation` into anyone's `SKILL.md`.
- Codex visibility control. The only route is a managed `CODEX_HOME` mirror (symlinked `auth.json`, copied `config.toml`, a curated `skills/`), the same machinery feature 52 declined; it stays declined until codex ships a per-launch control.
- Plugins as a management surface. `--plugin-dir` kits are phase 3 for skills outside the global set only; nothing in v1 touches `~/.claude/plugins`.
- Cross-platform apply ladders (symlink → junction → copy): Runner is macOS-only and copies nothing in v1 anyway.
- qoder / trae catalogs.

## Implementation phases

0. **Design** — `design/runner.pen`: the Skills pane card and list row, the runner-form Skills field with its checklist popup and chip summary, the detail-page block. Reuses the Agents card and the model-field popup grammar; the only new element is the checklist row.
1. **Backend** — `skills.rs` catalog + frontmatter parsing + bundled-probe cache; migration 0021 and `skills` through repo/ops/MCP (+ `skill_list`); `claude_settings_args` extension with the `--settings` merge; spawn wiring for direct chats and mission slots. Tests below. Ships alone as a nightly-safe change (no UI, defaults preserve behaviour).
2. **App** — Settings → Skills pane; runner form field + popup + chips; detail block; budget hint. Smoke test: two runners with different picks, `claude -p`-style "list the skills you can see" from each chat.
3. **Later, each its own issue when felt** — slot-level override in the crew editor (feature 59 pattern); `--plugin-dir` kits for skills outside `~/.claude/skills` (copy, never symlink, into `<app-data>/kits/<handle>/`); codex once a per-launch control exists.

## Verification

- `skills.rs`: frontmatter name/description/flags parsed; folder-name fallback; malformed file → `problem` entry; personal-over-project precedence; symlinked `skills/` entries flagged; bundled cache honoured only when the version matches.
- `claude_settings_args`: `skills == None` → unchanged output; picked subset → exactly the complement of `known` marked `"off"`; unknown picked names never appear in the map; bundled names hidden only when the cache is populated; merge with an inline `--settings` JSON, with a `--settings` file, and with an unreadable file (fallback + warning); non-claude runtimes unchanged.
- Repo/ops/MCP: `skills` round-trips as `null` and as an array; `runner_update` with `skills: null` clears to all.
- App: form field hidden for codex, restored on switching back; missing names rendered as `missing`; budget hint threshold.
- Manual: runner A = `daily-brief` + `scan`, runner B = all; in each chat ask "list every skill you can see" — A reports the two plus bundled, B the full set; `shasum ~/.claude/settings.json` unchanged across both spawns; a runner whose `args` carry `--settings '{"theme":"dark"}'` keeps the theme and still gets the overrides.
