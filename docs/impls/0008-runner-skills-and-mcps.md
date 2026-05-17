# Runner skills + MCPs — implementation plan

> Tracking issue: [#73](https://github.com/yicheng47/runner/issues/73).
> Spec: [`docs/features/05-runner-skills.md`](../features/05-runner-skills.md).
>
> This doc captures the implementation sequencing, risks to validate
> before committing to the design, and the small spec amendments we
> agreed on while planning. The spec stays the source of truth for
> *what* we're building; this doc records *how* and *in what order*.

## Approach summary

The spec is solid and matches the codebase's existing patterns
(per-session dir under `<app_data>/sessions/<session_id>/`,
`SpawnSpec.env` plumbing, the `extra_env` parameter on
`SessionManager::base_spawn_spec`). We don't need to change
`SpawnSpec`, the launcher, or the runtime trait — `render_launch_script`
already exports arbitrary env, and `base_spawn_spec` already accepts
an `extra_env: BTreeMap<String, String>` argument that's merged on
top of `runner.env` (`src-tauri/src/session/manager.rs:437`). The
agent_home builder returns the env vars it wants set; the spawn
path threads them through `extra_env`. That's the entire wiring
seam.

## Sequencing

### Phase 1 — schema + CRUD (independent, low-risk)

Ship as its own PR. Doesn't touch spawn, can land in parallel with
phase-2 design validation.

- Migration is **`0005_skills_and_mcps.sql`**, not `0004` —
  `0004_mission_archived_at.sql` already exists.
- `commands/skill.rs`, `commands/mcp.rs`, plus the
  `runner_skills_set` / `runner_mcps_set` extensions on
  `commands/runner.rs`. Setters run in one transaction
  (delete-by-runner-id, insert with `position = i`).
- Types in `src-tauri/src/model.rs` and `src/lib/types.ts`.
- Unit tests at the density already in `commands/runner.rs`.

This unblocks the UI work (phase 4) in parallel because the Tauri
commands exist as soon as phase 1 lands.

### Phase 2 — agent_home + spawn integration (load-bearing risk)

The seam is `base_spawn_spec`'s `extra_env` argument. Build the
agent_home up front, return the env vars to set
(`HOME=<agent_home>` for claude-code, `CODEX_HOME=<agent_home>/.codex`
for codex, none for others), hand them in as `extra_env`. No
surgery on `SpawnSpec` or `render_launch_script`.

- New module `src-tauri/src/session/agent_home.rs`.
- Gated on "any skill/MCP attached" — empty attachment sets keep
  today's exact behavior. That doubles as a kill switch: if we
  ship and find a regression, an empty attachment list is the
  rollback.
- Cleanup on `stopped` / `crashed`, gated by
  `RUNNER_KEEP_AGENT_HOME=1`.

**Do codex first.** `CODEX_HOME` is scoped — it only redirects
codex's own config dir. `HOME=<agent_home>` for claude-code is
*broad*: it also reroutes `~/.gitconfig`, `~/.ssh`, `~/.cache`,
anything any subprocess of the agent resolves via `$HOME`. The
codex path lets us validate the symlink-and-overlay pattern with
much less blast radius before taking it to claude-code.

### Phase 3 — prompt composition (skills only)

Phased differently from the spec: ship phase 2 **without** the
prepended `## Skills` block for claude-code. The on-disk
materialization already covers it; the prepend is only required
for codex (no native loader). Add the claude-code prepend later if
we observe that a behavioral skill ("be concise") isn't getting
picked up reliably via the native `Skill` tool. Keeps phase 3
codex-only and trims one cross-cutting change in the same PR as
phase 2.

### Phase 4 — UI (parallelizable with phase 2)

Can start as soon as phase 1 lands — the Tauri commands are
enough to build against. Sidebar entries, Skills/MCPs pages,
runner edit drawer integration. No dependency on agent_home.

### Phase 5 — Pencil design + polish

As in the spec. Reuses existing list-page styling.

## Two things to validate before committing to the design

These are cheap to test and de-risk phase 2 substantially. Do them
before writing the agent_home builder.

### 1. claude-code auth on macOS

On recent claude-code versions, credentials live in the macOS
Keychain, not on disk in `.credentials.json`. If that's true at
the version we ship against:

- **Good news:** `HOME` swap is fine for auth. Keychain access is
  by uid, not by `$HOME`.
- **Validation:** build a throwaway agent_home by hand, run
  `HOME=<that> claude` from a shell, confirm auth works and
  `--resume` finds an existing conversation through the
  symlinked `projects/`.

If auth breaks under the `HOME` swap, the fallbacks are:

- (a) Check whether claude-code respects a narrower env
  (`CLAUDE_CONFIG_DIR` or similar). `claude --help` and the
  claude-code source are the cheap reads. A narrower env means
  no subprocess collateral.
- (b) Accept the broader `HOME` swap but symlink-through *every*
  entry of the real `$HOME`, not just `~/.claude/`. Loses some
  of the deterministic-set guarantee for non-Runner subprocesses
  but keeps the design.

### 2. claude-code's skill loader file format

The spec assumes `~/.claude/skills/<slug>/SKILL.md` with the
markdown body alone. Worth verifying the `Skill` tool actually
lists from that path at the runtime version we ship against, and
whether it wants frontmatter (name, description, allowed-tools,
etc.).

- **Validation:** drop one hand-written `~/.claude/skills/test-skill/SKILL.md`,
  start `claude`, ask it to list / invoke skills.

If the format is richer than "raw markdown body," extend the
`skills.content` column semantic accordingly (probably keep
storing the body as-is and synthesize frontmatter at materialize
time from the row's `name` / `description`).

## Spec amendments agreed during planning

1. **Migration number is 0005, not 0004.** `0004_mission_archived_at.sql`
   already exists.
2. **Phase the prepended `## Skills` block.** Spec decision #6
   (belt-and-braces) stays correct as a goal, but for claude-code
   ship phase 2 without it. Add the prepend only if we see a
   reliability gap with the native `Skill` tool. Codex still gets
   the prepend from day one — it's the only delivery path there.
3. **Codex before claude-code in phase 2.** Lower blast radius
   validation pass for symlink-and-overlay.
4. **Gate phase 2 on "any attachment present."** Empty attachment
   sets skip the agent_home build entirely. Doubles as a kill
   switch.

## Open questions (resolve during phase-1 design)

- Does `claude-code` ever write back to `~/.claude.json` mid-session
  (e.g. project-trust prompts, last-used model)? If so, our
  synthesized file may get clobbered, or worse, our generated MCP
  entries may get persisted to a real-looking on-disk file the user
  doesn't know exists. Worth a `lsof`/`fs_usage` check on a real
  session.
- Does the encoded-cwd `projects/` symlink survive claude-code's
  atomic-rename writes for new conversation `.jsonl` files? Spec
  #2 says yes (writes follow the symlink), but verify on macOS
  APFS (rename-into-symlinked-dir semantics are filesystem-dependent).

## Implementation notes

- `base_spawn_spec` is in `src-tauri/src/session/manager.rs:425`.
  It's the only function that builds a `SpawnSpec` for the
  mission / direct-chat paths; both `spawn` and `spawn_direct`
  funnel through it. One injection point covers all spawn entry
  points.
- The `claude_code_conversation_exists` helper in
  `src-tauri/src/router/runtime.rs:647` reads `HOME` from the
  *parent's* env, not from `SpawnSpec.env`. That's intentional
  (the check runs before spawn) and remains correct under our
  design — the spawned child sees the synthetic `HOME` but the
  `projects/` directory is symlinked through to the user's real
  `~/.claude/projects/`, so the encoded-cwd `*.jsonl` is
  reachable under both views.
