# 05 — Cross-platform, agent-agnostic MCP & skills management

Tracking: [#73](https://github.com/yicheng47/runner/issues/73)

> Rewritten 2026-07-15. The original spec at this number ("Skills + MCPs management per runner") attached skills/MCPs to runner templates and injected them through a per-spawn synthetic agent home built from symlink overlays. That direction is superseded: the per-runner-only surface buried the catalog inside edit forms, and the symlink-overlay home is a Unix-only mechanism. The data-model and materialization details below carry forward what still holds; see git history for the full original text.

## Motivation

Every coding agent ships its own way to configure MCP servers and skills — claude-code reads `~/.claude.json` + `~/.claude/skills/`, codex reads `~/.codex/config.toml`, and each agent's settings UI is specific to that agent and that machine. There's no single place to manage these, and nothing that works the same across agents or across platforms.

Runner already coordinates multiple agents from one app, so it's the natural home for one central, agent-agnostic place to define and manage MCP servers and skills: define a server or a skill once, and let Runner apply it to whichever agent a runner is backed by.

## Direction

- **Central catalog.** A dedicated management surface (settings-style, like Codex's "MCP servers" screen) to create / edit / delete reusable MCP servers and skills. One catalog, not buried inside per-runner edit forms.
- **Agent-agnostic.** Definitions are stored in Runner's own neutral shape and materialized into whatever the target agent expects (claude-code JSON, codex TOML, skill directories). The user defines an MCP/skill once; Runner handles the per-agent translation.
- **Cross-platform.** Both the management surface and the apply mechanism must work on macOS, Linux, and Windows — no Unix-only assumptions.

## Reference analysis: skills-manager

[`xingkongliang/skills-manager`](https://github.com/xingkongliang/skills-manager) (cloned at `~/repos/skills-manager`) is a shipped Tauri 2 + React app that manages **skills only** across ~10 coding agents. It has no MCP support, so it informs the skills half of this feature and the general architecture. What it does, concretely:

### Their model

- **Central library.** One canonical copy of every skill lives in a user-configurable central repo (`~/.skills-manager/skills/<skill>/` by default) plus a SQLite index. Agents never own the canonical copy; they receive it.
- **Tool adapter registry** (`src-tauri/src/core/tool_adapters.rs`). Each supported agent is a data record, not code: `key`, `display_name`, `relative_skills_dir` (e.g. `.claude/skills`), `relative_detect_dir` (existence check ⇒ "agent installed"), optional `project_relative_skills_dir` when the project-local path differs from the global one (OpenCode: `~/.config/opencode/skills` vs `.opencode/skills`), `additional_scan_dirs` for discovery-only locations, and per-agent path overrides. User-defined custom agents are the same record shape stored in settings. Adding an agent is adding a row.
- **Apply = sync with a cross-platform ladder** (`sync_engine.rs`). Per-agent configurable mode: symlink (default) or copy. On Windows: try `symlink_dir` → fall back to a **directory junction** (needs no privilege on local NTFS) → fall back to copy. Guards refuse syncs where source/destination overlap in either direction (their issues #61 and #199 — recursive copy self-nesting and source deletion).
- **Filesystem is the source of truth.** Workspace pages *scan the agent's actual skills directory* rather than trusting the app's own records — skills installed outside the manager surface too, with an "adopt into library" flow. Every skill card shows a live per-agent sync badge derived from what's actually on disk.
- **Scopes.** Global workspace (agent's user-level dir), project workspaces (`<repo>/.claude/skills/` etc.), linked workspaces (any directory as a skills root). Presets = named skill groups applied as a one-time copy, not a live sync.
- **Skill identity and updates** (`sync_metadata.rs`). Each library skill carries a versioned `.meta.json`: id, path key, enabled, tags, and a `source` record (`type` git/local/archive, ref, branch, subpath). Content hashing detects local drift and upstream updates for git-sourced skills.
- **Skill format** (`docs/skill-format-detection-spec.md`). A skill is a directory containing `SKILL.md` with YAML frontmatter — the skills.sh / vercel-labs convention. `README.md` / `CLAUDE.md` are explicitly not entry files.

### What Runner borrows

1. **The adapter-registry shape.** A neutral per-agent record describing where skills/config live, how to detect the agent, and per-agent overrides — extended in Runner to also describe the MCP config file format (JSON at `~/.claude.json` vs TOML at `~/.codex/config.toml`) and merge strategy.
2. **Copy-capable apply with the Windows ladder.** Symlink where possible, junction on NTFS, copy as the universal fallback — plus their src/dst overlap guards. This directly resolves the cross-platform constraint that killed the old spec's symlink-overlay agent home.
3. **Scan-don't-trust.** The management surface should show what the agent actually sees (scan the real dirs/config), surface externally-added entries, and offer adoption — not maintain a parallel belief that drifts.
4. **Source metadata + content hash** on catalog entries, so "imported from git / local / hand-written" is recorded and update checks are possible later without redesign.
5. **`SKILL.md` + YAML frontmatter** as the on-disk skill format — it's the ecosystem convention; claude-code loads it natively.

### What Runner does differently

- **MCP servers are first-class alongside skills.** skills-manager syncs directories; MCP definitions materialize by *merging into per-agent config files* (JSON/TOML), which needs structured read-modify-write per agent, not file sync. This is the half with no reference implementation.
- **Runner spawns the agents it manages.** skills-manager mutates global agent state and hopes the next CLI launch picks it up; Runner controls spawn, so it can materialize deterministically per session/runner if we choose to — the old spec's isolation idea remains available as an apply mechanism, minus the symlink-only implementation.
- **Enablement can bind to runners**, not just to "the agent globally": Runner has runner templates as a natural attachment point, which no external manager has.

## To be decided

- How enablement maps to runners: global on/off per agent vs per-runner attachment vs both (global default + per-runner override).
- The apply mechanism: write into each agent's native global config (skills-manager style, visible to the user's own CLI use outside Runner) vs a Runner-owned isolated agent home per spawn (deterministic, invisible outside Runner) vs both per entry. Whichever is chosen must use the copy-capable ladder above, not symlink-only overlays.
- Skills support per agent: claude-code loads `SKILL.md` natively; codex has no skill loader — likely system-prompt composition at spawn (the old spec's `## Skills` prepend survives as the codex path).
- Whether the catalog lives in SQLite (Runner-native, like runners/crews) or as files in an app-owned directory (skills-manager style, trivially inspectable/backup-able). Skills have file-shaped content either way.

## Out of scope (v1)

Versioning / history, import / export, OAuth helpers for MCP auth, sandboxed MCP execution, auto-discovery of installed MCPs, live-reload mid-session, marketplace integration, git-backed library sync.
