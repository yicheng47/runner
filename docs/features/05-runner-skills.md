# 05 — Skills + MCPs management per runner

> Tracking issue: [#73](https://github.com/yicheng47/runner/issues/73)

## Motivation

Today a runner template carries a single `system_prompt` field — one
free-text blob that doubles as the agent's role, its house rules, and
any project-specific instructions. The moment two runners share the
same "always cite file:line", "answer in bullets", or "use the staging
DB, never prod" guidance, the user has to copy-paste the same
paragraph into each runner's system prompt and remember to update
all of them in lockstep when it changes.

Same shape one layer over: every runner today inherits whatever
MCP servers the user happens to have configured globally in
`~/.claude.json` / `~/.codex/config.toml`. There is no per-runner
control. If the user wants `@impl` to have the GitHub MCP and `@review`
to have a separate read-only filesystem MCP, the only lever today is
"edit your global config and hope nothing else breaks."

The user's mental model — borrowed from claude-code's own loader — is:

- A **skill** is a small, named, reusable block of instructions
  ("how to write SQL migrations", "house style guide") that the agent
  loads natively (claude-code's `Skill` tool lazy-loads from
  `~/.claude/skills/<slug>/SKILL.md`).
- An **MCP server** is a named tool/resource provider the agent can
  call (`mcp__github__create_issue`, etc.).

Runners pick from a shared pool of each. Edit the skill / MCP entry
once; every attached runner gets the update on next spawn.

A third pressure lands on the same surface: **context durability**.
When a runner's session grows long enough to trigger the CLI's
auto-compaction (claude-code's `/compact`, codex's equivalent), the
agent's effective grasp of its role and rules weakens. The original
`system_prompt` may still be in context, but attention degrades with
distance ("lost in the middle"), and compaction itself can paraphrase
away the specifics. A spawn-time prompt blob is a one-shot impression.
Skills materialized to disk and an MCP config loaded per-session are
*re-readable* — the CLI rediscovers them post-compact through its own
native loader, with no router intervention. So the per-spawn
`agent_home` does double duty: it scopes the attached tools/skills to
the runner, *and* it gives the runner a durable identity surface that
survives compaction.

## Scope

### In scope (v1)

#### Data model

- **`skills` table** in the SQLite store:
  - `id` ULID, `slug` UNIQUE NOT NULL (kebab-case),
  - `name`, `description`, `content` (markdown body of `SKILL.md`),
  - `created_at`, `updated_at`.
- **`mcp_servers` table:**
  - `id`, `slug` UNIQUE NOT NULL, `name`, `description`,
  - `transport` — one of `"stdio"`, `"http"`, `"sse"`,
  - `command` (stdio), `args` JSON-encoded text (stdio),
  - `url` (http/sse),
  - `env` JSON-encoded object (stdio: passed to the MCP server
    process; http/sse: ignored),
  - `headers` JSON-encoded object (http/sse: sent on requests;
    stdio: ignored),
  - `created_at`, `updated_at`.
- **Join tables** with `position` for stable ordering and
  cascade-delete on either side:
  - `runner_skills(runner_id, skill_id, position)`
  - `runner_mcps(runner_id, mcp_id, position)`

#### Injection mechanism

This is the central design call. v1 uses a **per-spawn isolated
config home** built fresh at every spawn, with the agent CLI's
`HOME` (or runtime-specific equivalent) pointed at it. The agent
loads skills and MCPs natively, exactly as it would from a real
home directory — no special flags, no system-prompt hacks, no
working-directory pollution.

The session manager already constructs per-session shim directories
under `<app_data>/sessions/<session_id>/` (see `SpawnSpec.shim_dir`,
`bundled_bin_dir`). The same pattern extends to a sibling
`<app_data>/sessions/<session_id>/agent_home/` that is the agent's
synthetic home. Layout (claude-code):

```
<agent_home>/
├── .claude/
│   ├── skills/
│   │   └── <skill-slug>/SKILL.md  ← materialized from skills.content
│   ├── settings.json              ← symlink → real ~/.claude/settings.json
│   ├── .credentials.json          ← symlink → real ~/.claude/.credentials.json
│   ├── projects/                  ← symlink → real ~/.claude/projects/
│   ├── plugins/                   ← symlink → real ~/.claude/plugins/
│   ├── todos/                     ← symlink → real ~/.claude/todos/
│   └── (every other entry)        ← symlinked through unchanged
└── .claude.json                   ← generated: user's ~/.claude.json
                                     with attached MCPs merged into
                                     "mcpServers"
```

Spawn the child with `env["HOME"] = agent_home`. The agent's own
config-discovery code does the rest: it reads `~/.claude.json` (now
ours), walks `~/.claude/skills/` (now ours), and resolves auth
through `.credentials.json` (still the user's real file via the
symlink). Resume keeps working because `~/.claude/projects/<encoded-
cwd>/<uuid>.jsonl` is symlinked back to the user's real history,
including the encoded-cwd path scheme already handled by
`router::runtime::claude_code_conversation_exists`.

For codex, the equivalent layout uses `CODEX_HOME=<agent_home>/.codex`
because codex respects `CODEX_HOME` natively (avoids needing the
broader `HOME` swap):

```
<agent_home>/.codex/
├── config.toml           ← generated: copy of user's ~/.codex/config.toml
│                            with attached MCPs merged as
│                            [mcp_servers.<slug>] tables
├── auth.json             ← symlink → real ~/.codex/auth.json
└── (every other entry)   ← symlinked through unchanged
```

Codex has no native skills loader, so codex runners get attached
skills via the existing `system_prompt` composition path (see
"Skills composition fallback" below).

For unknown / shell runtimes: no injection. The agent_home dir is
not built; spawn proceeds as today.

#### Lifecycle of the agent_home

- **Build** in the session manager just before spawn (after env is
  composed, before `SpawnSpec` is handed to the runtime). The build
  is a single function `build_agent_home(session_id, runner_id, ...)`
  in a new `src-tauri/src/session/agent_home.rs` module.
- **Idempotent**: if the dir exists, wipe non-symlink contents and
  rebuild. Symlinks to the user's real config are reused if their
  targets haven't changed.
- **Cleanup on session end**: remove the dir when the session
  transitions to `stopped` / `crashed` and the operator has
  navigated away. Keep across reattach so a tmux re-attach doesn't
  lose the home. A `RUNNER_KEEP_AGENT_HOME=1` env var preserves the
  dir for inspection (debug).
- **Concurrency**: the dir is per-session, so two runners spawning
  the same template concurrently get distinct agent_homes. No
  locking required.

#### Skills composition fallback (codex + as a belt-and-braces layer)

Even with native materialization for claude-code, the existing
`router::prompt` system-prompt path stays in play, because:

1. Codex has no native skill loader.
2. Some skill content is *behavioral* ("be concise", "always cite
   file:line") — useful even when claude-code's `Skill` tool would
   otherwise lazy-load it on demand. Inline composition guarantees
   the rule is read on every turn, not just when the agent
   happens to invoke `Skill`.

`router::prompt` gains a new `## Skills` block prepended to
`system_prompt`. Each attached skill renders as:

```
### <skill name> (<skill slug>)
<skill content>
```

Ordering: `runner_skills.position` ascending; ties on slug. Empty
attachment list → no `## Skills` header at all (no whitespace
surprise). Trailing newlines are normalized so the prepended
block stays clean.

This is identity-shape vs how-to: the runner's own `system_prompt`
keeps its identity ("you are @impl"); skills carry the cross-cutting
rules and live above it.

The on-disk materialization is the durable copy: after the CLI
compacts the conversation, the prepended `## Skills` block may be
summarized away, but the `SKILL.md` files inside `agent_home` are
still on disk for the `Skill` tool to re-list and re-load. The
prepended block is the cheap "always-read on every turn" layer; the
on-disk files are the "survive any compaction" layer.

#### Tauri commands

- **Skill CRUD** (mirror `commands/runner.rs` shape):
  `skill_list`, `skill_get`, `skill_create`, `skill_update`,
  `skill_delete`.
- **MCP CRUD:** `mcp_list`, `mcp_get`, `mcp_create`, `mcp_update`,
  `mcp_delete`.
- **Attachment setters** (replace-all per call — see Key decisions
  below):
  - `runner_skills_set(runner_id, skill_ids: Vec<String>)`
  - `runner_mcps_set(runner_id, mcp_ids: Vec<String>)`
  - Companion getters: `runner_skills_list(runner_id)`,
    `runner_mcps_list(runner_id)`.
- Each mutating command emits the corresponding `*/changed` event
  (`skill/changed`, `mcp/changed`, `runner/changed`).

#### UI surfaces

- **New `Skills` page** at `/skills`:
  - Sidebar entry under WORKSPACE after CREWS (icon: lucide
    `BookOpen`).
  - List view with name + description + attached-runner count.
  - "+ New skill" header button → `CreateSkillModal`.
  - Row click → `SkillEditDrawer` (slug read-only post-create).
- **New `MCPs` page** at `/mcps`:
  - Sidebar entry under Skills (icon: lucide `Plug`).
  - List view with name + transport + attached-runner count.
  - "+ New MCP" header button → `CreateMcpModal` with conditional
    fields (stdio: command / args / env; http / sse: url /
    headers / env).
  - Row click → `McpEditDrawer`.
- **Runner edit drawer + create modal** get two collapsible
  sections: "Skills" and "MCPs", each a checkbox list of all
  defined entries with name + description. Selection state lives
  in local form state; on submit, the runner CRUD call is followed
  by `runner_skills_set` and `runner_mcps_set`.
- **Runner detail page** renders attached skills and MCPs as two
  chip groups below the system-prompt section. Chip click navigates
  to the corresponding editor.

### Out of scope (deferred)

- **Per-attachment overrides** — no "attach skill X but disable
  section Y", no per-runner skill variables, no per-runner MCP env
  overrides. Edit the entry or fork it.
- **Versioning / history.** Skills and MCPs are mutable. If a
  runner was working with skill content X yesterday and the user
  edits it today, tomorrow's spawn uses today's content. Mirrors
  how `runner.system_prompt` works today.
- **Shared skill/MCP libraries.** Import / export, sync from a
  remote registry, or load from `~/.claude/skills/` directly. v1's
  pool is whatever the user types into the Skills / MCPs pages.
- **Skill / MCP exposure via Runner's own MCP server (#40).** v1
  of #40 is CRUD for crews/runners/slots. Skill + MCP CRUD via
  MCP is a small extension to that surface; defer.
- **Crew or mission-level skills/MCPs.** Issue #54 covers the
  crew-level `system_prompt_addendum` (Layer 2). Skills + MCPs
  here are Layer 1 (runner template). The two compose cleanly;
  this spec doesn't pre-empt #54.
- **MCP transports beyond stdio / http / sse.** websocket, etc.
  — neither claude-code nor codex needs these in v1.
- **Auth flows for MCP servers** beyond static headers / env. No
  OAuth helper, no token refresh. The user pastes a token into the
  `headers` field for the duration; if they want a rotation
  story, that's a follow-up.
- **Sandboxed MCP server execution.** Stdio MCP servers run as
  subprocesses of the agent CLI under whatever permissions the
  user already grants the agent. Runner doesn't add a sandbox
  layer.
- **Auto-discovery of installed MCPs.** No "scan your `~/.claude.json`
  and import existing servers." User adds entries explicitly. (A
  one-shot `runner mcp import` CLI subcommand is a clean follow-up.)
- **Live-reload of agent_home mid-session.** Editing a skill
  doesn't propagate to a running agent — the next spawn picks it
  up. Hot-reload would need claude-code/codex cooperation we don't
  have.
- **Mid-session role-pulse / periodic system reminder.** Skills on
  disk solve post-compaction recovery, but attention drift *before*
  compaction fires (long sessions where early-context instructions
  lose effective weight) is a separate concern. A router-side
  "remember your role" injection on a cadence is the natural
  complement to this spec — when to fire, what to include, how to
  avoid alarm fatigue — but is its own design. Sibling spec, not
  here.

### Key decisions

1. **Per-spawn isolated `HOME` (claude-code) / `CODEX_HOME` (codex),
   not flag-based or working-dir injection.** The agent's own
   config-discovery code is the source of truth for what's loaded;
   we just point it at our synthesized home. Pros: no per-runtime
   flag plumbing, attached skills/MCPs are deterministic (no leak
   from the user's global config), no working-dir pollution.
   Cons: more setup at spawn time. The setup is per-session and
   amortized across the session's lifetime, so the trade-off
   favors isolation.
2. **Symlink-and-overlay, not copy-everything.** Auth tokens,
   conversation history, plugins — these stay in the user's real
   `~/.claude/` and are reached via symlinks. Writes (re-auth, new
   conversation files) follow the symlinks back to the real files,
   so the user sees a continuous history regardless of which
   runner produced it. Only `.claude/skills/` and `.claude.json`
   are owned by Runner.
3. **User's global skills / MCPs do NOT leak in.** The
   `.claude/skills/` directory in `agent_home` is *not* a symlink
   to `~/.claude/skills/` — it's a fresh directory containing only
   the runner's attached skills. Same for the merged `.claude.json`:
   we start from the user's real one, but only the *attached* MCP
   slugs end up under `mcpServers`. This is the deterministic-set
   contract. (If the user wants a global skill, they attach it.)
4. **Slug is globally unique within Runner, not scoped per-runner.**
   Mirrors `runner.handle` (arch §2.2). Slug is the on-disk
   directory name (skills) and the JSON / TOML key (MCPs); a
   stable identifier that may show up in events, mentions, or
   future sharing flows. Rename is allowed for `name` and
   `description`, *not* for `slug`.
5. **`runner_skills_set` / `runner_mcps_set` replace the whole list
   per call.** Two separate add / remove commands invite consistency
   bugs. The edit drawer already builds the new list locally before
   save; matching that contract on the backend is one round-trip
   and one transaction.
6. **Skills compose into `system_prompt` (prepended) AND materialize
   to disk for claude-code.** Belt-and-braces: behavioral rules
   (concise, cite-file-line) are read every turn via the system
   prompt; reference-shaped rules (here's how to debug X) are
   lazy-loaded via the `Skill` tool. Codex relies on the system
   prompt path only. Avoids the question "did the agent even read
   the skill?" — for short rules it always has.
7. **No "enabled" flag on the join rows.** Either attached or
   detached. Toggling on/off would duplicate the
   attach/detach affordance with no semantic payoff.
8. **MCP `command` / `args` / `env` shape mirrors the
   claude-code `~/.claude.json` schema 1:1**, so the merge step is
   structural, not transformative. Reduces "MCP works in
   `~/.claude.json` but not in Runner" surprise.
9. **Skills + MCPs survive auto-compaction by design.** Both live as
   re-readable artifacts on disk inside `agent_home` — skills as
   `SKILL.md` files the CLI's `Skill` tool lazy-loads on demand;
   MCPs as config the CLI rediscovers per tool invocation. This is
   why behavioral rules ("be concise", "cite file:line") belong in
   skills, not in `runner.system_prompt`: the system prompt is the
   part most vulnerable to compaction drift, and the on-disk skill
   files are the part that isn't. Decision #6's prepended `## Skills`
   block is the always-read layer for short rules; the disk copy is
   the survives-compaction layer for everything else.

## Implementation phases

### Phase 1 — Schema + commands

- Add migration `0004_skills_and_mcps.sql`:
  - `skills(...)`, `mcp_servers(...)`, `runner_skills(...)`,
    `runner_mcps(...)` per the data-model section.
  - Indexes on `runner_skills(runner_id, position)` and
    `runner_mcps(runner_id, position)`.
- New modules:
  - `src-tauri/src/commands/skill.rs` — CRUD free functions.
  - `src-tauri/src/commands/mcp.rs` — CRUD free functions.
  - Extend `commands/runner.rs` with `runner_skills_set`,
    `runner_mcps_set`, and the companion list getters. Each
    setter runs in one transaction (delete-by-runner-id, insert
    new set with `position = i`).
- Add `Skill`, `McpServer`, link types to
  `src-tauri/src/model.rs` and `src/lib/types.ts`.
- Re-export the Tauri commands from `commands/mod.rs` and
  `lib/api.ts`.
- Unit tests: one happy path + one error case per CRUD verb,
  matching the density in `commands/runner.rs`.

### Phase 2 — Agent-home builder + spawn integration

- New module `src-tauri/src/session/agent_home.rs`:
  - `build(session_id, runner_id, runtime, db, real_home) ->
    Result<AgentHome>` returns a struct with the dir path and the
    env vars to set (`HOME` for claude-code, `CODEX_HOME` for
    codex, none for others).
  - For claude-code:
    - Create `<agent_home>/.claude/skills/<slug>/SKILL.md` for
      each attached skill (write content verbatim).
    - Symlink every entry of `<real_home>/.claude/` into
      `<agent_home>/.claude/` *except* `skills/` (we own it).
    - Read the user's `<real_home>/.claude.json` (or empty
      object if absent), merge attached MCPs into `mcpServers`
      under their slug as the key, write to
      `<agent_home>/.claude.json`.
  - For codex:
    - Create `<agent_home>/.codex/`, symlink every entry of
      `<real_home>/.codex/` into it *except* `config.toml`.
    - Read `<real_home>/.codex/config.toml`, merge attached
      MCPs under `[mcp_servers.<slug>]` tables, write to
      `<agent_home>/.codex/config.toml`.
  - For shell / unknown: no-op, return None.
- Wire into `session::manager::SessionManager::spawn` and
  `spawn_direct`: just before the `SpawnSpec` is built, call
  `agent_home::build` and merge the returned env into
  `SpawnSpec.env`. The dir path is stored on the in-memory
  session record so cleanup can find it later.
- Wire into the `Session` lifecycle: on transition to
  `stopped` / `crashed`, schedule a cleanup that removes the
  agent_home (gated on `RUNNER_KEEP_AGENT_HOME` env).
- Tests in `agent_home.rs`:
  - Empty skills / MCPs → still builds, claude-code gets a
    valid `.claude.json` (user's contents passed through),
    `skills/` dir is empty.
  - Symlink targets resolve to the user's real config files.
  - Two attached MCPs (one stdio, one http) round-trip into the
    merged `.claude.json` with the right `type` discriminator.
  - Codex variant round-trips MCPs into `[mcp_servers.<slug>]`
    TOML tables.

### Phase 3 — Prompt composition (skills only)

- In `router::prompt::compose_lead_launch_prompt`, accept a new
  `skills: &[SkillRef]` field on `LeadView`. Render the
  `## Skills` block prepended to `system_prompt` when non-empty.
- In `router::handlers` and `router::mod.rs` (the two callsites
  that build `LeadView`), fetch attached skills via a single
  join query and pass them through.
- Tests in `router/prompt.rs`: empty list → no header; one and
  two skills → ordering and formatting; trailing newlines
  normalized.

### Phase 4 — UI: Skills page + MCPs page + runner edit integration

- New components:
  - `Skills.tsx` page (mirrors `Runners.tsx`).
  - `MCPs.tsx` page.
  - `CreateSkillModal.tsx` / `SkillEditDrawer.tsx`.
  - `CreateMcpModal.tsx` / `McpEditDrawer.tsx` with conditional
    field rendering by transport.
- Sidebar (`Sidebar.tsx`): add entries under WORKSPACE after
  CREWS — Skills, then MCPs. Icons via lucide.
- `CreateRunnerModal.tsx` and `RunnerEditDrawer.tsx`: add two
  collapsible sections, each a checkbox list. On submit, after
  the runner CRUD call resolves, call `runner_skills_set` and
  `runner_mcps_set` with the new lists.
- `RunnerDetail.tsx`: chip groups for attached skills / MCPs
  below the system-prompt block.

### Phase 5 — Pencil design + polish

- Design the Skills page, MCPs page, and the two new
  runner-edit sections in `design/runners-design.pen`. Reuse
  Runners list page styling and existing checkbox-row primitives.
- Confirm empty-state copy for: no skills/MCPs defined, no
  attachments on a runner, MCP transport-conditional fields.
- Verify the sidebar's WORKSPACE list still reads cleanly with
  five entries (RUNNERS, CREWS, SKILLS, MCPS, ARCHIVED once #31
  lands).

## Verification

- [ ] Migration applies cleanly on a fresh DB and on an upgrade
      from an existing user DB.
- [ ] `skill_create` / `mcp_create` reject duplicate slugs.
- [ ] `skill_update` / `mcp_update` cannot change `slug`.
- [ ] Deleting a skill or MCP cascades to `runner_skills` /
      `runner_mcps`; open detail pages reflect the removal within
      ~1s of the `*/changed` event.
- [ ] `runner_skills_set` / `runner_mcps_set` are atomic — passing
      a malformed list (unknown id) leaves the prior set unchanged.
- [ ] Spawning a claude-code runner with two attached skills
      produces `<agent_home>/.claude/skills/<slug-a>/SKILL.md` and
      `<slug-b>/SKILL.md` with the expected content; the agent's
      `Skill` tool lists both within its first listing turn.
- [ ] The same spawn ALSO renders a `## Skills` block in the
      composed launch prompt (belt-and-braces layer).
- [ ] Spawning a claude-code runner with an attached stdio MCP
      results in the agent calling that MCP's tools successfully
      end-to-end (smoke: a `filesystem` MCP listing files in
      `working_dir`).
- [ ] Spawning a claude-code runner with an attached HTTP MCP
      forwards the configured headers in the request.
- [ ] User's real `~/.claude/skills/` and `~/.claude.json`
      MCP entries are NOT visible to the spawned agent unless
      explicitly attached on the runner.
- [ ] User's auth / login (`.credentials.json`) and conversation
      history (`projects/`) ARE visible — symlinks resolve to the
      real files, and resume continues to work via
      `router::runtime::claude_code_conversation_exists`.
- [ ] Codex runner with an attached MCP gets a valid
      `<agent_home>/.codex/config.toml` containing the attached
      `[mcp_servers.<slug>]` table; codex starts and lists the
      MCP server.
- [ ] Codex runner with attached skills sees them in the
      composed system prompt (codex has no native loader); no
      `<agent_home>/.codex/skills/` directory is built (codex
      ignores it).
- [ ] Editing a skill's content and re-spawning the runner picks
      up the new content (no caching surprise).
- [ ] Two concurrent spawns of the same runner template build
      distinct `agent_home` dirs (per-session) and don't collide.
- [ ] Session end cleans up the agent_home dir; with
      `RUNNER_KEEP_AGENT_HOME=1` it is preserved.
- [ ] Sidebar Skills / MCPs entries navigate correctly and survive
      page reloads.
- [ ] `cargo test --workspace` covers the new command modules,
      `agent_home::build`, and prompt composition.
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
