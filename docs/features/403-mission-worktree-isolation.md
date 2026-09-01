# Opt-in worktree isolation per mission

Tracking issue: [#403](https://github.com/yicheng47/runner/issues/403). Status: planned.

## Motivation

Every mission on a project runs in the same checkout. Two missions fanned out on one repo step on each other's edits, and any mission steps on the human's working tree — the operator can't safely keep editing while a crew works. The integration point for fixing this already exists: `mission.cwd` is the highest-priority working directory at spawn (`src-tauri/src/session/manager/spawn.rs:261` resolves mission cwd → runner `working_dir` → inherit), and the resume-stability comment beside it (`spawn.rs:265`) already demands that a mission's cwd never change across respawns — which is exactly what a worktree created once at mission start provides. Isolation is therefore a mission-start decorator: create the worktree, point `mission.cwd` at it, and spawn, resume, sessions, and the event log need zero changes. It also becomes the injection point for later per-runner skill materialization (spec 73's per-agent skill dirs want a session-scoped tree to write into).

Worktree use is opt-in per mission. Most missions are one crew on one repo with no concurrent sibling; the default stays the project checkout so the solo flow is untouched.

## Scope

- **Schema**: migration `0021_mission_worktree.sql` adds `worktree_branch TEXT NULL` to `missions`; expose on `Mission` (`src-tauri/src/model.rs`, `src/lib/types.ts`). A set `worktree_branch` marks `cwd` as app-owned — the discriminator that lets cleanup act only on directories runner created, never on a user-typed cwd.
- **Creation** (`mission_start`, `src-tauri/src/commands/mission.rs`): when the caller opts in and the resolved project cwd is a git repo, run `git worktree prune` (heals manually deleted trees), then `git worktree add <repo>/.worktrees/<short-id>-<slug> -b mission/<short-id>-<slug> HEAD`, append `.worktrees/` to `.git/info/exclude` if absent (not `.gitignore` — don't dirty the user's tree), and set `mission.cwd` to the worktree path before slots spawn. The short id comes from the mission id, killing title collisions. A failed `worktree add` fails mission start with the git stderr — no partial missions in an ambiguous cwd.
- **Sharing**: all slots run in the one mission worktree. Crew coordination happens through signals in a shared tree; that stays the model.
- **UI** (`src/components/StartMissionModal.tsx`): a "Run in isolated worktree" toggle, default off, disabled with a hint when the selected project cwd is not a git repo. The mission workspace header (`src/pages/MissionWorkspace.tsx`) shows a branch chip (`mission/<short-id>-<slug>`) so the operator always knows where the crew is writing.
- **Cleanup**: `mission_archive` on a worktree mission offers removal via `git worktree remove` — which refuses a dirty tree by default, so unmerged agent work is never destroyed silently; a dirty tree archives with the worktree left in place and the UI says so. The branch always survives; merge-back is manual in v1 (the chip tells the human what to PR).
- **MCP**: `mission_start` (`src-tauri/src/mcp/tools/mission.rs`) grows an optional `worktree: bool`, same semantics as the modal toggle.

## Non-Goals

- Per-slot worktrees — one tree per mission; adversarial parallel-attempt patterns can revisit later.
- Auto-merge, PR creation, or any merge-back automation.
- Base-ref picker — v1 branches from repo HEAD at mission start.
- Worktree support for non-git projects or a per-project "always isolate" default (revisit once the toggle has usage).
- Migrating existing missions; the column is null for all history.

## Implementation Phases

1. **Backend** — migration, `worktree_branch` plumbing through repo/model/commands, creation + prune in `mission_start`, archive-time removal path, MCP flag; rust tests (worktree created and cwd points into it; branch naming; non-git opt-in rejected; dirty-tree removal refused and archive still succeeds; prune heals a deleted tree).
2. **Frontend** — start-modal toggle with git-repo gating, branch chip in the mission header, archive prompt copy for the dirty-tree case; vitest coverage alongside `projectScopedStartModals.test.tsx`.
3. **Docs** — README index entry; note the app-owned-cwd discriminator in the arch docs if session docs reference cwd resolution.

## Verification

- `cargo test --workspace`: the backend tests above.
- Vitest: toggle renders only for git projects, start payload carries the flag, branch chip renders from `worktree_branch`.
- Manual: start two missions on the same project, one isolated — `git worktree list` shows the mission tree, sessions' PTYs land in `.worktrees/<slug>`, the other mission and the human's checkout stay untouched; archive the clean one (worktree removed, branch remains), dirty the other and archive (worktree left, UI states why); `--resume` a worktree mission's session after app restart and confirm it resumes against the same cwd.
