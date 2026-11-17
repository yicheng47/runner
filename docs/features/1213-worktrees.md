# 1213 — Git worktrees

> Tracking issue: [#1213](https://github.com/yicheng47/runner/issues/1213). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: m1 (`runnerd` and CLI) implemented in PR #1350 (2026-11-17), following the design signed off by Jason on 2026-11-16 and the [decisions](#decisions-settled-with-this-spec) below; m2 (the app) follows in a later PR.
> Design: `runner/desktop/specs/1213-worktrees.pen` in `yeelinks_design`, revision `685b8c9`, board `A0AUx`. Sections: 1 the launcher's Checkout control and its folder coupling, 2 New worktree…, 3 the sidebar rules with their edge-case table and both views side by side, 4 the Info panel, 5 cleanup at archive.
> Supersedes: [#403](https://github.com/yicheng47/runner/issues/403), closed as not planned on 2026-09-22. Related: [#1141](./1141-git-changes.md) (Changes view), [#1345](https://github.com/yicheng47/runner/issues/1345) (Changes: choose the checkout and the base), [#748](./archive/748-mission-link.md) (chats linked to missions), [#1290](https://github.com/yicheng47/runner/issues/1290) (archiving linked missions).

## Motivation

Running agents in parallel on one repository needs one checkout per task. Runner's own workflow does this by hand for every mission: `git worktree add .worktrees/<branch> -b <branch> origin/main`, then `runner mission start --cwd <that path>`, then `git worktree remove` and a branch delete after merge. Every person who runs two crews on one repository repeats this, or lets them collide in a shared checkout, where #1141's Changes mixes their edits.

The app already reads worktrees but cannot make one: Files (#634) lists them in its picker, and the launcher has a project and a folder but no way to say "in a fresh checkout". Agents are the main callers ([agent environment](../product/agent-environment.md)), so a chat that starts a mission must be able to ask for its own checkout in the same command.

This reverses the non-goal in [vision §6](../product/vision.md), "It does not create git worktrees". The boundary moves from "Runner never touches checkouts" to "Runner manages checkouts; it never writes history": it can create, show and remove a worktree, and commits, merges, pushes and pull requests stay with the agent and git.

## Principle

A worktree is a **checkout chosen at launch**, not a sidebar container. A session's cwd is fixed for its lifetime and stays the only record of where it runs. Which worktree a session is in, and its branch, is derived from that cwd and git whenever it is shown, never stored on the session or the mission. Runner stores nothing about worktrees: no table, no column and no migration (decision 9), so a `git switch` inside a worktree shows on the next refresh. The project → tab/mission tree and #748's chat → linked-mission groups stay as they are.

## Scope

### Model (runnerd)

- **Git is the source of truth** for which worktrees exist and their branches. `files/git.rs` already runs and parses `git worktree list --porcelain [-z]`; the worktree service reuses it.
- **Runner's worktrees** are the linked worktrees directly under the main checkout's `.worktrees/`, whoever made them. They are the only ones Runner offers to remove. A worktree a person made there by hand counts as one, which is safe because removal never forces, refuses a dirty tree and keeps the branch; worktrees elsewhere, such as Codex's or `../x`, never do.
- **Where a worktree lives** (decision 2): `<main checkout>/.worktrees/<branch with "/" replaced by "-">`. Git has no default location; this is Runner's own convention, which keeps the checkout and its build output on the repository's volume. On the first create in a repository, runnerd adds `/.worktrees/` to the common git directory's `info/exclude` unless it is already ignored. The exclude is local, so the repository itself is never changed, the main checkout's `git status` stays clean, and `git add -A` there cannot commit a worktree as an embedded repository.
- **Git-aware project membership.** `ProjectScope::Infer` keeps path containment first, so a project registered at a worktree's own path stays its own project. A cwd that no project folder contains falls back to git: if it is in a linked worktree whose main checkout is a project's folder, it belongs to that project. Worktrees made outside the repository by Codex (`~/.codex/worktrees/`), by `git worktree add ../x` or by Runner then file under the right project.
- **Create and start is one request.** A chat, terminal or mission start can carry `worktree: { branch, base? }`. runnerd resolves it in this order:
  1. A worktree already has the branch checked out: start there. Asking twice never fails, which matters to agents that retry.
  2. The branch exists and no worktree has it: `git worktree add <path> <branch>`.
  3. Otherwise: `git worktree add -b <branch> <path> <base>`.

  Before step 1, a registration git still keeps for a folder that is gone (`prunable`), at the target path or holding the branch, is cleared with `git worktree remove`, so step 1 only matches worktrees whose folder exists and a folder deleted with `rm -rf` never blocks the add with "already registered". When `git worktree add` fails, the start fails with git's stderr and nothing is recorded or spawned. A spawn that fails after a successful add keeps the empty worktree, so the retry reuses it. Creates in one repository are serialized in runnerd so two starts for the same branch cannot race.
- **Base** (decision 3): `origin/HEAD` as it is locally, falling back to the main checkout's `HEAD`. No automatic fetch, since a fetch can prompt for credentials or hang. The base is used for `git worktree add` and not recorded.
- **Checking a name** as it is typed: a request returns one of *new branch from <base>*, *existing branch at <commit>*, *already checked out in <path>*, or *invalid* with git's reason from `git check-ref-format --branch`. The launcher's hint line is built from it.
- **Listing refs for the base** reuses #1345's `changes_branches` request: local branches, then each remote's branches.
- **Removal** is `git worktree remove <path>` for one of Runner's worktrees, never `--force`. runnerd refuses while any unarchived session's cwd is inside it, and returns git's reason when git refuses. The branch always survives; Runner deletes no branches.
- **Trust.** A new worktree is a new cwd, so each runtime's existing trust seeding (`seed_project_trust`) runs for it as for any start.

### Launcher (design section 1)

- A **Checkout control** sits in the Location row between the project and the path when the folder is in a git repository. It shows the checkout's branch with a kind icon: a house for the main checkout, a git folder for a linked worktree. A folder outside any repository shows no control, so the row is exactly today's. The control appears once runnerd has answered, without a placeholder.
- **The folder decides; the project and the checkout follow it**, as the project picker already does in `start_chat/project_field.rs`. Change keeps its job, opening the folder browser: any folder can be chosen, and the project and checkout are re-derived from it. Picking a checkout rewrites the folder to that checkout, keeping the subfolder when it exists there and taking the checkout's root otherwise.
- **The Checkout menu** ("Start in") has a filter field, then the main checkout and every linked worktree, in #1345's checkout-picker style: branch, short commit, `clean` or the uncommitted count, the path, and a tag for who works there ("mission · coder", "chat"). New worktree… is last.
- The control works the same for chats, terminals and missions. An empty split pane has no project or folder control and gets none. Remote sessions get no control (see [Remote](#remote)).

### New worktree… (design section 2)

- Picking it opens a form under the Location row with an **empty, focused branch field**. The person names the branch; Runner never suggests or generates one (Jason, 2026-11-16). The placeholder reads "Branch name, e.g. feat/123-login".
- Beside the field, a **base chip** ("from origin/main") opens the base list: the default first, naming what it resolves to, then local branches, then each remote's, with a filter and the same keys as #1345's base picker.
- A **hint line** always says what Start will do: "New branch from origin/main, in .worktrees/feat-1213-worktrees", "Checks out the existing branch (a41e0c2), in .worktrees/…" with the base dimmed, "… is already checked out in …. Starts there.", or the invalid-name reason in red.
- While the field is empty or invalid, the start list is dimmed and Enter does nothing. ↓ moves from the field to the start list and Enter starts the highlighted agent there.
- The path in the Location row shows where the worktree will be, dimmed, once there is a name; before that it stays the current folder.
- A failed create leaves the launcher open with git's message in the form, and nothing started. Change, or picking another checkout, leaves New worktree mode and discards the form.

### Sidebar (design section 3, decision 1)

One rule for both views: **the line says which project and which checkout, never which folder.**

- **Projects view.** Rows whose session runs outside the main checkout take a second line with the branch, in the Active row's style (44 px row, 10 px icon, 11 px text, `$text-low`). All other rows keep today's single line.
- **Active view.** Every row keeps the project name, the only cue to which repository a row belongs in a flat list. A row outside the main checkout reads `<project> › <branch>` with the branch icon in place of the folder icon. **Subfolders are no longer shown**: a chat in `runner/crates/runner-app` reads `runner`. This changes `active_location` in `sidebar/active.rs`, which today appends the path under the project.
- **Edge cases**, each drawn in both views in the design's table:
  - A main checkout switched to another branch shows no branch: the line marks a linked worktree, not a branch other than the default.
  - A worktree made elsewhere, such as Codex's, looks the same as one Runner made.
  - A subfolder of a worktree shows the branch only.
  - A branch switched inside a worktree is followed on the next refresh.
  - A detached `HEAD` shows the short commit with a commit icon.
  - A tab whose panes are in different checkouts, including the main checkout and one worktree, shows "2 checkouts" with a layers icon; its tooltip lists each pane's branch. A tab whose panes share one worktree shows that branch.
  - A worktree whose folder is gone shows "missing · <branch>" in `$warn` with an alert icon, the status leading so truncation never hides it. The branch is found as restore finds it: git's `prunable` entry for the path while git keeps it, then the one local branch whose flattened name equals the folder's name, and the folder name only when neither answers.
  - Long branches truncate at the end; the row's tooltip holds the full branch and path.
  - Pinned rows and terminals follow the same rule. Selection, status icons and the trailing slot are unchanged.
  - A session on another computer takes its branch from that host's runnerd; while the host is offline the row falls back to one line rather than a stale branch.
  - Before git answers at startup, a row shows what today's app shows, with no placeholder.
- **Linked groups.** The parent chat and each mission follow the rule on their own rows. `sidebar/link.rs` already draws the connectors relative to the row's height, branching at its middle and curving into the last mission's branch, so the taller rows need no new connector logic.
- Branches refresh from the watcher #634 already runs for each checkout, not by polling per row.

### Info panel (design section 4, decision 7)

The chat Info panel and the mission rail's Info add a `branch` row under `cwd` **only when the cwd is in a linked worktree**; the main checkout's panel is unchanged. Every linked worktree adds a marker, "Worktree · from origin/main", where the base is the one Changes resolves for that checkout (`auto_base`); with no base it says "Worktree". Files needs no change.

### Cleanup at archive (design section 5, decision 4)

- When archiving would leave one of Runner's worktrees with no unarchived chat or mission, the archive confirmation offers **Remove worktree `<branch>`**: "Deletes runner/.worktrees/<folder>. The branch stays, so restoring the chat checks it out again."
- runnerd checks `git status` first. A clean tree is **checked by default**. A tree with changes shows the option unavailable, with an alert icon and the reason in `$warn` ("Has 3 uncommitted changes and 1 untracked file, so git won't remove it. It stays; runner worktree remove can remove it later."); the archive itself goes ahead.
- Archiving a chat with linked missions (#1290) lists one Remove worktree row per worktree the archive empties, each with its own box.
- If git still refuses at removal time, the archive stands and a toast gives git's reason; the worktree stays.
- Worktrees outside `.worktrees/` are never offered.

### Restore and resume after removal (decision 6)

A worktree can be removed outside Runner, by `git worktree remove`, `git worktree prune` or deleting the folder. Nothing Runner stores goes stale: the session keeps its cwd and its project, which is decided at start and stored, and its row shows as missing. A session still running there keeps its process; Runner neither stops it nor recreates the folder under it.

Restoring a chat or mission from Settings → Archived, resuming a stopped session, or adding a role to a mission, whose worktree is gone checks the surviving branch out again at the same path before the session starts. These are actions a person or agent takes on that session. The automatic resume when runnerd starts (`daemon/boot.rs`) is not: a session whose cwd is missing then stays stopped and shows as missing, so a worktree removed on purpose is never brought back unasked. Nothing records the branch, so runnerd finds it from git's `prunable` entry for the path while git keeps one, clearing that entry with `git worktree remove` before the add; otherwise it takes the one local branch whose flattened name equals the folder's name. No match, or more than one, takes the fallback below. Keeping the cwd matters: Claude Code files each conversation under `~/.claude/projects/<cwd>` and Runner checks for that file before resuming (`runtimes/claude_code/mod.rs`), so a Claude chat started in another folder would lose its history. If the branch is gone too, the restore says so and offers **Restore in main checkout**, warning that a Claude chat starts without its history.

### CLI

- `runner worktree create <branch> [--project P | --cwd DIR] [--base REF]` prints the path. `runner worktree list [--project P]` lists every worktree with its branch, path and whether it is one of Runner's. `runner worktree remove <branch|path>` removes one of Runner's worktrees and refuses others. All three support `--json`.
- `runner chat start` and `runner mission start` gain `--worktree <branch> [--base REF]`, resolved as in [Model](#model-runnerd), so a retried command is idempotent.
- Runner's own crew flow in `AGENTS.md` becomes `runner worktree create`, commit the brief, then `runner mission start --worktree` (or `--cwd`).
- The CLI reference is regenerated with `make guide-cli`.

### Remote

Worktree operations run on the host whose runnerd owns the session. The launcher offers them only where Files works, so a session on another computer shows no Checkout control. Display (sidebar and Info) uses the owning host's answers.

### Language

Every string goes through `crates/runner-app/i18n/` with keys in `en.toml` and `zh-hans.toml`. Branch names, refs, paths, commit ids and git's messages stay as git gives them.

## Non-goals

Commits, staging, merging, pushing or opening pull requests. Deleting branches. A worktree per mission slot: a mission's members share one checkout. Moving a session, running or stopped, into another checkout, including from the side panel, since a Claude chat moved to another cwd loses its conversation; a follow-up could allow it for stopped sessions of runtimes whose resume does not depend on the cwd. Setup and teardown scripts such as `pnpm install`. `.worktreeinclude` (decision 5, later). Removing worktrees outside `.worktrees/`. Storing worktrees, branches or bases in the database. Worktrees for folders outside git. Fetching before a create.

## Risks

- **Core review.** The new start field and requests touch the client protocol and session launch, so the runnerd work takes `core-review`. The schema is unchanged.
- **Windows paths.** Spelling differs (`\`, drive letters, case), and deep `.worktrees/<branch>` paths can pass `MAX_PATH`. Paths are compared through the existing normalization used by project membership, and a create that fails on length surfaces git's message.
- **Old gits.** `git worktree list -z` already falls back without `-z`; `git worktree add` and `remove` exist in every git Runner supports.
- **Shared repository state.** Branches and the object store are shared across worktrees; two sessions cannot check out one branch, which git enforces and the name check reports before Start.
- **Disk and build cost.** Each worktree is a full checkout with its own build output. Cleanup at archive keeps the count down; Runner never removes a worktree without asking.

## Verification

### Automated

- **Name check and create**, in fixture repositories built with the git CLI: new branch from `origin/HEAD`; no `origin` falls back to the main checkout's `HEAD`; existing branch checked out; branch already in a worktree returns its path; invalid names; path collision returns git's stderr and records nothing; a spawn failure after add keeps the worktree; two concurrent creates of one branch produce one worktree.
- **Exclude**: the first create adds `/.worktrees/` to the common `info/exclude` once, not when already ignored, and never touches `.gitignore`.
- **Membership**: a cwd in a linked worktree inside the repository, in `../x` and in a Codex-style folder elsewhere files under the main checkout's project; a project registered at a worktree path stays its own.
- **Removal**: refuses while an unarchived session's cwd is inside; refuses worktrees outside `.worktrees/`, and accepts a hand-made one inside it; returns git's reason for a dirty tree; never deletes the branch.
- **Restore**: a worktree removed with `git worktree remove` is checked out again at the same path by matching the flattened branch name; a folder deleted by hand is restored from git's `prunable` entry; two branches that flatten to the folder's name, or a deleted branch, report the fallback.
- **Removed outside Runner**: with a session marked to resume, removing its worktree by `git worktree remove` and by deleting the folder, then starting runnerd, leaves the session stopped and the folder absent; an explicit resume afterwards recreates it. A `--worktree` create for a branch whose folder was deleted by hand clears the stale registration and succeeds. The missing row's branch reads `feat/x`, not `feat-x`, after `git worktree remove`.
- **Live branch**: after `git switch` inside a worktree, the derived branch for its sessions and mission is the new one.
- **Location strings**: `active_location` and the Projects line for every case in the design's table, including detached `HEAD`, "2 checkouts", missing and remote-offline.
- **CLI**: `--worktree` idempotency, `--json` shapes, and the regenerated reference.

### Live checks

m1 changes no screen, but git-aware membership moves rows in the sidebar and every start goes through the changed launch path, so m1's `qa` runs these on macOS in a fixture repository under `/tmp` registered as a project, with an extra worktree made by `git worktree add ../outside`. Windows path spelling is covered by the automated tests on Windows CI; a native Windows run is recorded as a gap unless a Windows machine is available.

- **m1-1 Starts without a worktree**: a chat, a terminal and a crew mission started as today run in their chosen folder and resume after a development daemon restart. Reason: the start request and launch path changed.
- **m1-2 Membership**: a chat started in `../outside` files under the fixture's project in both the Projects and Active views; a second project registered at a worktree's own path keeps its chats. Reason: `ProjectScope::Infer` now falls back to git.
- **m1-3 CLI create-and-start**: `runner chat start --worktree feat/x` and `runner mission start --worktree feat/y` create `.worktrees/feat-x` and `.worktrees/feat-y`, the app shows each session under the fixture's project, and its terminal's `pwd` is the worktree; repeating either command reuses the worktree without an error. Reason: the create-and-start request and its idempotency.
- **m1-4 CLI removal**: `runner worktree remove feat/x` refuses while the chat is unarchived, succeeds after it is archived with the branch kept, and refuses the `../outside` worktree. Reason: the removal rules.

Selected regression cases for m1: the sidebar's Projects and Active rows and the launcher's project/folder rows in [workspace](../tests/regression/workspace.md), because membership decides both.

m2's `qa` runs these on macOS and Windows, and on Linux when a Linux machine is available, in a fixture repository under `/tmp` registered as a project, with an extra worktree made by `git worktree add ../outside`:

1. **Checkout control**: the menu lists the main checkout, the outside worktree and a Runner one; picking one sets the folder; Change into a worktree folder follows; a non-git folder shows no control. Reason: new launcher control.
2. **New worktree**: an empty name blocks Start; a new branch creates the worktree and starts a chat in it; an existing branch is checked out; a branch already in a worktree starts there; an invalid name and a path collision show their messages and start nothing. Reason: create-and-start.
3. **Mission in a worktree**: `runner mission start --worktree` and the launcher both start every member in the worktree, and the mission resumes there after a development daemon restart. Reason: launch cwd and resume.
4. **Sidebar**: Projects and Active rows match the design for the main checkout, a Runner worktree, the outside worktree, a mixed tab and a linked group, in dark and light. Reason: new row rules and the Active location change.
5. **Info**: the branch row appears only in worktrees. Reason: panel change.
6. **Archive**: a clean worktree is removed with its box checked and the branch survives; a dirty one shows the option unavailable and stays; a chat with two linked missions lists two rows. Reason: removal.
7. **Restore**: restoring the archived chat recreates its worktree and a Claude chat resumes with its history; after deleting the branch, the fallback warns. Reason: decision 6.
8. **Windows paths**: creation, membership and removal with drive-letter paths. Reason: platform path spelling.

Selected regression cases: the sidebar's Projects and Active rows and the launcher's project/folder rows in [workspace](../tests/regression/workspace.md), and linked-group display from #748.

## Guide pages

[Projects](../guide/concepts/projects.md) (worktrees file under their project), [Missions](../guide/concepts/missions.md) and [Your first mission](../guide/getting-started/first-mission.md) (starting in a worktree), [Build a crew](../guide/guides/build-a-crew.md), [Chats and windows](../guide/guides/chats-and-windows.md) (the Checkout control, the sidebar line, cleanup at archive), [Drive Runner from agents](../guide/guides/drive-runner-from-agents.md) (`--worktree`), each with its zh-CN mirror, and the CLI reference. Also [vision §6](../product/vision.md), [agent environment](../product/agent-environment.md) ("What it does not provide"), the [roadmap](../roadmap.md)'s product boundaries, and the Worktrees and Crew Missions sections of `AGENTS.md`.

## Delivery

Proposed, for Jason to confirm before a crew starts: two pull requests to `main`, without an umbrella.

| Step | What | Crew | Labels |
|---|---|---|---|
| m1 — runnerd and CLI | Worktree service (name check, refs, create-and-start, remove, restore), git-aware membership, `--worktree` and `runner worktree`, `AGENTS.md`'s crew flow. No new screens; the sidebar regroups through membership | hydrogen trio | `core-review`, `risk-high`, `needs-live-test` |
| m2 — the app | Checkout control and New worktree…, sidebar lines and the Active location change, Info row, archive option and restore fallback, guide pages | claude trio | `frontend-only`, `needs-live-test` |

m1 leaves today's starts as they are, since the start field is optional and the app does not use the new requests until m2. Its one visible change is membership: a chat or mission started after m1 in a worktree outside a project's folder files under that project. The project is decided at start and stored, so existing rows stay where they are. The spec lands with m1.

## Decisions settled with this spec

Agreed by Jason on 2026-11-16.

1. **Sidebar**: a branch line under rows outside the main checkout; the Active location shows project and checkout, never subfolders. Branch tags beside the title and worktree folder nodes were rejected.
2. **Location**: `<repo>/.worktrees/<flattened branch>`, hidden through `.git/info/exclude`.
3. **Base**: `origin/HEAD` locally, falling back to the main checkout's `HEAD`; no fetch.
4. **Removal default**: checked when clean; unavailable with the reason when dirty.
5. **`.worktreeinclude`**: later.
6. **Restore after removal**: recreate the worktree at the same path; if the branch is gone, offer Restore in main checkout with a warning.
7. **Info**: the branch row only in linked worktrees.
8. **Branch names** are always given by the person; Runner never generates one.
9. **No stored worktree state** (Jason, 2026-11-17): the session's or mission's cwd plus git is the only record, so the display follows `git switch`. A `worktrees` table and a `worktree_branch` column were both considered and dropped; removal is offered by location under `.worktrees/`, and restore finds the branch from git or the folder name.
10. **Removed outside Runner** (Jason, 2026-11-17): only an explicit restore, resume or add-role recreates a missing worktree; runnerd's automatic resume at startup leaves the session stopped and missing.
