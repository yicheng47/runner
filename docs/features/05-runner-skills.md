# 05 — Local skills management

Tracking: [#73](https://github.com/yicheng47/runner/issues/73). Status: specced 2026-08-28, not scheduled. Implementation program: [`docs/impls/local-skills/`](../impls/local-skills/README.md) (M1 view skills by runtime → M2 allowlist backend → M3 allowlist app → later).

> Rewritten 2026-08-28. The 2026-07-15 text at this number described an agent-agnostic MCP + skills catalog modelled on skills-manager (central library, per-agent sync ladder, adopt-into-catalog). That framing answered "where do skills live"; the problem Jason actually has is "which skills does *this session* see". Claude Code has shipped the primitives that answer it since then, so this spec narrows to skills, per runner, claude-code first, and defers the MCP catalog to a later spec. The earlier text stays in git history.

## Motivation

A session sees every skill installed on the machine. On this machine that is 36 user skills under `~/.claude/skills/` (the memory repo's `install.sh` symlinks the same set into `~/.claude/skills` and `~/.codex/skills`) plus Claude Code's own bundled skills — 51 names in the model's context. Two things go wrong at that size:

- **Wrong skill.** A trading runner has `lark-*`, `media`, `music` and `books` in front of it; a reviewer runner has `daily-brief`. Every extra description is a chance for the model to pick a skill that does not belong to the role.
- **Silent loss.** Since Claude Code 2.1.129 the skill listing has a budget — `skillListingBudgetFraction`, default 1 % of the context window, roughly 15–25 skills — and past it Claude Code drops whole descriptions for the least-used skills, ranked by recency and frequency. With 51 loaded, some skills are invisible to the model on any given launch, and which ones depends on history. A runner cannot rely on its skill being seen.

Runner is the process that spawns these sessions and already passes `--settings` to every claude-code spawn (`claude_settings_args`, `crates/runner-backend/src/router/runtime.rs`). Claude Code's `skillOverrides` setting is exactly a per-name visibility map, and it rides that flag. Verified on claude-code 2.1.250 on 2026-08-28: injecting `--settings '{"skillOverrides":{…34 names…:"off"}}'` into a headless launch took the model's own reported skill list from 51 names to 15 (the two kept plus 13 bundled), with `~/.claude/settings.json` byte-identical before and after. The earlier leak bugs (anthropics/claude-code #54996, #50631: `"off"` still listed to the model) were closed 2026-05-04 and do not reproduce.

So the feature is small at its core: **a runner declares which skills it wants, Runner hides the rest at spawn.** Nothing is copied, symlinked, or written into the user's agent homes.

## Principles

- **One key in the user's home, never a file move.** Runner reads each runtime's global skills directory (`~/.claude/skills`, `~/.codex/skills`) and never writes there. The one thing it writes is the `skillOverrides` key of `~/.claude/settings.json` — the same key Claude Code's own `/skills` menu writes — for global on/off. Same posture as MCP registration (only `mcp_servers.runner` is ever touched): structured read-modify-write of one key, everything else in the file preserved byte-for-byte where the JSON allows.
- **Allowlist, computed at spawn.** The runner stores what it *wants*; the `"off"` set is derived from what exists at spawn time. A skill added next week is hidden from restricted runners automatically instead of leaking in.
- **Scan, don't trust.** The catalog is derived from the filesystem on every read; Runner keeps no parallel list that can drift.
- **claude-code first, honestly.** Codex has skills but no per-launch visibility control. The picker is hidden for other runtimes rather than pretending.
- **Pencil-first.** The Skills pane and the picker are drawn in `design/runner.pen` before the app phase (README §Decisions that still bind).

## Mechanism

### What Claude Code provides (2.1.250)

| primitive | scope | what it does |
|---|---|---|
| `skillOverrides: { name: "on" \| "name-only" \| "user-invocable-only" \| "off" }` | settings key | `"off"` hides the skill from the model and the `/` menu; `"name-only"` keeps the name without the description. Plugin skills are exempt. In `~/.claude/settings.json` it is the **global** switch (what `/skills` writes); in `--settings` it is the **per-session** one. Same key, two scopes — Runner uses both. |
| `--settings <file-or-json>` | per launch | merges on top of the user's settings for that session only. Runner already sends `{"tui":"fullscreen"}`. |
| `disable-model-invocation: true` (frontmatter) | per skill, permanent | description never enters context; `/name` still works. Runner reads it, never sets it. |
| `skillListingBudgetFraction`, `skillListingMaxDescChars`, `SLASH_COMMAND_TOOL_CHAR_BUDGET` | settings / env | the listing budget. Runner reports against it; v1 does not change it. |
| `--plugin-dir <dir>` | per launch | loads a plugin (skills namespaced `/plugin:skill`) for the session only. The route for skills that are **not** in the global set — phase 3, not v1. Per-skill symlinks inside a plugin are skipped silently; only a symlinked `skills/` directory or real folders load. |

Sources the model sees, by name: `~/.claude/skills/<name>/SKILL.md` (personal — **the only set Runner manages**), `<cwd>/.claude/skills/<name>/` up to the repo root (project, after workspace trust — the project's business, always loads), bundled skills shipped inside the CLI (`code-review`, `simplify`, `loop`, `schedule`, `claude-api`, `dataviz`, `update-config`, `keybindings-help`, `fewer-permission-prompts`, `workflow-authoring`, `run`, `init`, `security-review` on 2.1.250 — Claude Code's business, always load), and plugin skills.

### Catalog (derived)

The model is deliberately flat: **each runtime has one global skills directory, and a runner picks from its runtime's global set.** Nothing else is catalogued.

`runner-backend` gains `skills.rs`: `pub fn skill_catalog(runtime: &str, home: &Path, codex_home: Option<&Path>) -> Option<SkillCatalog>`. Pure over injected paths, never errors; a malformed `SKILL.md` becomes an entry with a `problem` string, not a failure.

- **claude-code**: `~/.claude/skills/<name>/SKILL.md` — frontmatter `name` if present, else the folder name; `description`; `disable-model-invocation`; `user-invocable`; path; symlink target if any; the folder's file list. Plus each entry's **global state** from `~/.claude/settings.json` → `skillOverrides[name]`: absent or `"on"` = on; `"off"` = off; `"name-only"` / `"user-invocable-only"` = on, shown with that value as a badge.
- **codex**: `$CODEX_HOME/skills/<name>/SKILL.md` (`~/.codex/skills`). Listed for information; no visibility control exists, and the card says so.
- **qoder / trae**: `None` — no card.

Project skills and bundled skills are outside the model on purpose: a project's `.claude/skills` is the project's decision and loads for any runner working there; bundled skills are Claude Code's and cannot be read from disk. Both always load; the pane says so in one caption each. If either ever needs control it is a later phase, not a v1 complication.

### Global on/off

The Skills pane toggles a claude-code skill on or off **for every session of that runtime, inside Runner or not**, by editing `skillOverrides` in `~/.claude/settings.json`: off → set `"off"`; on → delete the entry (Claude Code's default is on; a `"name-only"` / `"user-invocable-only"` entry is also cleared by toggling on, and the pane says so on hover). `ops::skills::set_global_enabled(runtime, name, enabled)` does a structured read-modify-write like `ops/mcp.rs` does for `mcp_servers.runner`: parse, touch one key, write back; a file that does not exist is created with only that key; a file that fails to parse is left alone and the toggle reports the error. Running sessions are unaffected until their next launch — the pane says so in its caption. Codex has no equivalent setting, so codex rows carry no toggle.

Runner never expresses "off" by moving or renaming a skill folder (skills-manager's `-disabled/` sibling): the directory is the user's, and a moved folder is invisible to every other tool that reads it.

### Runner model

- Migration `0021_runner_skills.sql`: `ALTER TABLE runners ADD COLUMN skills_json TEXT NULL`. `NULL` = all skills (today's behaviour, the default for every existing runner); a JSON array of names = only these.
- `Runner.skills: Option<Vec<String>>` through repo, ops (`CreateRunnerInput` / `UpdateRunnerInput` gain `skills`), and the MCP tools `runner_create` / `runner_update` / `runner_get` / `runner_list`. A new read-only MCP tool `skill_list { runtime, cwd? }` returns the catalog so an external controller can build the array from real names.
- Names are stored as the user picked them; a stored name that no longer exists is kept (it may come back) and shown as *missing* in the form, never silently dropped.

### Spawn

`claude_settings_args(runtime, runner_args)` becomes `claude_settings_args(runtime, runner_args, skills: Option<&[String]>, global: &[String])` and emits one `--settings` JSON:

```json
{ "tui": "fullscreen", "skillOverrides": { "<every global name not in skills>": "off" } }
```

- `global` is the runtime's global directory listed at spawn time, minus the names that are globally off (those never load anyway) — so the allowlist tracks the filesystem and the global switches, not the form's last view, and a skill added next week is hidden from restricted runners automatically.
- `skills == None` → no `skillOverrides` key at all; the emitted flag is byte-identical to today's.
- **Merging with a runner's own `--settings`.** Today a `--settings` in the runner's `args` short-circuits Runner's flag entirely. That stays correct only while Runner's payload is cosmetic; with `skillOverrides` in it, Runner must merge: read the runner's value (inline JSON or a file path), deep-merge Runner's keys on top (`skillOverrides` entries union, the runner's explicit entries win), emit one combined `--settings`, and drop the runner's flag from the argv. A file that cannot be read → log, fall back to Runner's payload alone, and surface the problem on the runner card.
- Direct chats spawned from a runner (Start Chat) inherit the runner's `skills`; missions inherit per slot from the slot's runner. Slot-level override is a later phase.
- Non-claude runtimes: `skills` is ignored at spawn and the form hides the field.

### Budget awareness

The picker shows `N picked of M` (M = the global count) and an inline note past 20 picked: *"Above ~20 skills Claude Code starts dropping descriptions (listing budget, 1 % of context). Pick fewer or tighten descriptions."* Bundled skills (13 on 2.1.250) and any project skills count against the same budget but are outside Runner's control, so the note is a hint, not a gate; v1 does not set `skillListingBudgetFraction`.

## Surfaces

### Settings → Skills (new pane, between Agents and MCP)

One card per runtime that has a catalog (claude-code, codex), same card grammar as Agents. **This pane is the first milestone (M1) and ships alone** — a place to see what each runtime can see, before any control exists:

- Header: runtime name, the scanned directory as a faint path, `N skills`, a **Refresh** ghost button.
- A list of skills: name, first sentence of the description (faint, one line, ellipsised), and flags as tiny badges: `manual` (`disable-model-invocation`), `hidden` (`user-invocable: false`), `symlink` (with the target on hover), `problem` in danger tone with the parse error on hover.
- Per row on the claude-code card, a **Toggle** (the Agents pane's `Enabled` / `Disabled` control) bound to the skill's global state; flipping it writes `~/.claude/settings.json` (§Global on/off) and re-scans. Off rows render faint. An entry with `name-only` / `user-invocable-only` shows that value as a badge next to the toggle.
- Row actions: **Reveal in Finder** (opens the folder), **Copy name**.
- Clicking a row opens a read-only detail panel (the pattern of skills-manager's skill sheet): name as title, the full description, the path and a `SKILL.md` link that reveals the file, the folder's file list (`scripts/`, `references/`, …), and `SKILL.md` rendered by the mission feed's Markdown renderer (`surfaces/mission_markdown.rs`, the same dependency-free subset) — frontmatter shown as a small key/value table above the body, not as raw `---` lines.
- Empty root: one faint row *"No skills in <path>."*; missing root: the same with *"(directory does not exist)"*.
- No create, edit, delete. Editing a skill is the user's editor's job; per-runner picking is the runner form's job (M3). The claude-code card carries a caption: *"Toggles edit `skillOverrides` in ~/.claude/settings.json and apply to new sessions everywhere. Bundled skills (`code-review`, `loop`, …) are not on disk and not listed."* The codex card carries: *"Codex has no skill on/off setting; all of these load in every codex session."*

Added in M3, once the runner field exists: per skill, the runners that restrict to it as handle chips (`@trader`, `@reviewer`) — the reverse index answers "who uses this" without a second surface.

### Runner create / edit (M3)

A **Skills** field directly under **Model** (claude-code runtimes only; the field is removed, not disabled, for others):

- A `StyledSelect` with two options: **All skills** (`51 visible`) and **Only selected…**. Choosing the second opens a checklist popup (search field on top, a flat list of the runtime's globally-on skills (off ones are not offered — they never load), each row name + one-line description, checkbox) — the popup component follows `ModelField`'s suggestion popup and the select's option list; occlusion rules from feature 62 apply.
- Below the field a summary line: the picked names as chips (handle-style), a `missing` chip tone for names no longer on disk, and the budget hint when over 20.
- Switching the Agent select to a non-claude runtime hides the field and keeps the stored value (it comes back if the runtime does); switching back restores it.
- The runner detail page shows the same chips under a **Skills** label; the runner card in the list shows nothing new.

### MCP

`runner_create` / `runner_update` accept `"skills": null | ["name", …]`; `runner_get` returns it; `skill_list` returns the catalog. The `runner` CLI is untouched.

## Reference: skills-manager

[`xingkongliang/skills-manager`](https://github.com/xingkongliang/skills-manager), cloned at `~/repos/ai/skills-manager` (`24356c3`, 2026-08-28), is the closest shipped product: a Tauri app whose *Global Workspace* page lists, per agent, everything in that agent's skills folder. Its problem is different — a central library synced into 53 agents' folders — so most of it does not apply, but its scanning rules and its per-agent page are the reference for M1.

**Borrowed**

- **Detection policy** (`docs/skill-format-detection-spec.md`): a directory is a skill only if it contains `SKILL.md` (exact case; `skill.md` accepted as legacy); `README.md` / `CLAUDE.md` never count. Name = frontmatter `name` sanitized to a single path component, else the folder name (`skill_metadata.rs::infer_skill_name`). Frontmatter parse reads `name` and `description` only and ignores everything else; no frontmatter → both `None`.
- **Walk rules** (`project_scanner.rs::read_skills_from_dir`, `scanner.rs::collect_skill_dirs`): skip dot-directories; skip a nested directory that itself contains a `skills/` subtree (embedded plugin / cache bundles); a canonicalized `visited` set so symlink loops terminate; sort case-insensitively by name.
- **Per-agent page shape** (`WorkspaceView.tsx`): agent title with the skills directory as a faint path under it, `{{count}} skills`, a search field over name + description, rows of `name` (fixed-width, semibold) + one-line truncated description + a status pill on the right, actions on hover; `No local skills found` as the empty state.
- **Skill detail sheet**: title, full description, tag chips, the path with a folder icon and a `SKILL.md` link, then the document. Runner drops the tabs (Local / Diff / Center) — there is no center copy to diff against.
- **`ToolAdapter` as data** (`tool_adapters.rs`): agent key, display name, `relative_skills_dir` (`.claude/skills`, `.codex/skills`), `additional_scan_dirs` (`.agents/skills` for codex — their comment notes `.codex/skills` is the only path Codex reads until the `.agents` migration ships). Runner's `RuntimeDefinition` gains one `skills_dir` field rather than a parallel registry; the project-dir dimension is not modelled.

**Not borrowed**

- The central library, sync engine, symlink/copy/junction ladder, `in_sync` / `local_newer` / `center_newer` / `diverged` statuses, presets, tags, git backup, marketplace — all serve "one library, many agents"; Runner's problem is "one agent, fewer skills".
- Their `enabled` flag (a sibling `<dir>-disabled/` folder the skill is moved into): Runner never moves files; visibility is `skillOverrides`.
- Cross-platform machinery; Runner is macOS-only.

## Non-Goals (v1)

- A global on/off for codex: no setting exists, and moving folders is out (§Global on/off). Codex rows are view-only.
- Project skills (`<cwd>/.claude/skills`, `.agents/skills`) and bundled skills: not catalogued, never hidden — they are the project's and the CLI's, respectively. No probe, no version cache, no project picker.
- The MCP-server catalog half of #73 — a later spec; Runner's own MCP registration (`Settings → MCP`) is unrelated and stays.
- Creating, editing, deleting or installing skills; git sources, update checks, content hashes, marketplaces, the skills-manager adopt flow. `Reveal in Finder` is the whole authoring story.
- Per-chat overrides in Start Chat, and per-slot overrides in the crew editor (phase 3).
- Changing the listing budget, or writing `disable-model-invocation` into anyone's `SKILL.md`.
- Codex visibility control. The only route is a managed `CODEX_HOME` mirror (symlinked `auth.json`, copied `config.toml`, a curated `skills/`), the same machinery feature 52 declined; it stays declined until codex ships a per-launch control.
- Plugins as a management surface. `--plugin-dir` kits are phase 3 for skills outside the global set only; nothing in v1 touches `~/.claude/plugins`.
- Cross-platform apply ladders (symlink → junction → copy): Runner is macOS-only and copies nothing in v1 anyway.
- qoder / trae catalogs.

## Implementation phases

0. **Design** — `design/runner.pen`: the Skills pane card and list row first (M1 needs only these); the runner-form Skills field with its checklist popup and chip summary, and the detail-page block, before M3. Reuses the Agents card and the model-field popup grammar; the only new element is the checklist row.
1. **M1 — View and switch skills by runtime** — `skills.rs` catalog over each runtime's global directory (frontmatter parsing, flags, problems, file list, global state) + the Settings → Skills pane as described above, including the read-only detail panel and the claude-code global toggle (`set_global_enabled`). No schema, no spawn change: nothing about how Runner spawns moves; the toggle changes what Claude Code loads everywhere, which is its point. Ships alone.
2. **M2 — Allowlist, backend** — migration 0021 and `skills` through repo/ops/MCP (+ `skill_list`); `claude_settings_args` extension with the global set listed at spawn and the `--settings` merge; spawn wiring for direct chats and mission slots. Behaviour-preserving until a runner is given picks over MCP.
3. **M3 — Allowlist, app** — the runner form field + checklist popup + chips, the detail-page block, the budget hint, the pane's reverse index. Smoke test: two runners with different picks, `claude -p`-style "list the skills you can see" from each chat.
4. **Later, each its own issue when felt** — slot-level override in the crew editor (feature 59 pattern); `--plugin-dir` kits for skills outside `~/.claude/skills` (copy, never symlink, into `<app-data>/kits/<handle>/`); codex once a per-launch control exists.

## Verification

- `ops::skills::set_global_enabled`: off sets `"off"`, on deletes the entry, other keys and unrelated `skillOverrides` entries survive byte-for-byte, a missing file is created with only the key, an unparsable file is left untouched and reported; the catalog reflects the state after each write.
- `skills.rs`: frontmatter name/description/flags parsed; global state read from `skillOverrides` (absent, `on`, `off`, other values); folder-name fallback; sanitized frontmatter name; malformed file → `problem` entry; legacy `skill.md` flagged; dot-directories and embedded bundles skipped; symlinked entries flagged with their target; symlink loops terminate; case-insensitive sort; missing root → empty catalog with `root_exists: false`; unknown runtime → `None`.
- `claude_settings_args`: `skills == None` → unchanged output; picked subset → exactly the complement of `global` marked `"off"`; unknown picked names never appear in the map; merge with an inline `--settings` JSON, with a `--settings` file, and with an unreadable file (fallback + warning); non-claude runtimes unchanged.
- Repo/ops/MCP: `skills` round-trips as `null` and as an array; `runner_update` with `skills: null` clears to all.
- App: form field hidden for codex, restored on switching back; missing names rendered as `missing`; budget hint threshold.
- Manual (M1): toggle `music` off in the pane → `~/.claude/settings.json` gains exactly `"skillOverrides": {"music": "off"}` and nothing else changes; a new `claude` session outside Runner no longer lists `/music`; toggle on → the key is gone.
- Manual (M3): runner A = `daily-brief` + `scan`, runner B = all; in each chat ask "list every skill you can see" — A reports the two plus Claude Code's bundled ones, B the full set; `shasum ~/.claude/settings.json` unchanged across both spawns; a runner whose `args` carry `--settings '{"theme":"dark"}'` keeps the theme and still gets the overrides.
