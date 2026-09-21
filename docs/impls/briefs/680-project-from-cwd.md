# 680 — a mission or chat started inside a project's directory belongs to that project

[#680](https://github.com/yicheng47/runner/issues/680), P2, milestone 0.11. Work in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-680-project-from-cwd`, a linked worktree of the repository. Stay inside it; the other worktrees under `.worktrees/` and the checkout at the repository root hold other people's live work. You are on branch `feat/680-project-from-cwd`, cut from `main` at `32a653d`; its first commit carries this brief. Build on it; do not rebase, squash or create another branch.

Read first: this brief; the issue end to end, including the paragraph explaining why this is **not** a regression; `crates/runner-backend/src/ops/project.rs`; `ops/mission.rs` around `start`; `ops/session.rs` around the direct-chat start; `repo/project.rs`; `AGENTS.md`.

## The gap

Runner binds a project to a directory, and resolution has only ever run one way. `ops::project::resolve_cwd` (`ops/project.rs:66`) takes a project and supplies its cwd. Nothing takes a cwd and finds the project, so a mission or chat started from the CLI inside a project's own directory lands unfiled. Since the repository moved to one worktree per branch under `.worktrees/`, every crew mission runs from a subdirectory of a project path, so this is now the normal case rather than an edge.

## Deliverable, part 1: resolve both directions at one seam

**Widen `resolve_cwd` rather than adding a helper that one caller remembers to call.** Make it resolve membership and directory together — project in, cwd out, *and* cwd in, project out — returning both values, and let the changed signature make the compiler enumerate every caller. There are four, and they must all behave the same:

- `ops/mission.rs:196` — `input.cwd = project::resolve_cwd(...)`, whose result must now also write back `input.project_id`, because `:240` stores that on the mission row and `:252` passes it to `repo::node::ensure_mission_node` to place the sidebar node under the project. A resolver that returns an id nobody writes back fixes nothing visible.
- `ops/session.rs:939` and `ops/session.rs:1076` — the direct-chat paths, which need the same write-back for the session row and its node.
- `mcp/tools/session.rs:218` — the socket tool layer resolving cwd on its own rather than through the op. It is a caller like any other; leave it consistent, do not special-case it.

`repo::project` has `list`, `get`, `create`, `rename`, `delete` and no query by path, so add one there.

### The matching rules

- **Longest ancestor wins.** This machine has projects at both `/Users/jason/repos/yicheng47` and `/Users/jason/repos/yicheng47/runner`; without this every mission collapses into the broader one.
- **Compare path components, not string prefixes.** `…/yicheng47/runner-wt` string-starts-with `…/yicheng47/runner`. That is a real path that existed on this machine yesterday, and a `starts_with` would silently file a sibling checkout under the wrong project. Make this a test, not a comment.
- **An explicit project still wins**, and an explicit cwd still overrides a project's bound cwd — today's behaviour in both directions is preserved exactly.
- **The inference never changes the cwd.** It only fires when a cwd is already present, so there is nothing to fill in.
- **No match means unfiled.** That is today's behaviour and it stays. Nothing is auto-created: inferring a project is not inventing one.
- **Decide and state the normalisation.** Symlinked and `/private`-prefixed paths on macOS, case-insensitive volumes, trailing separators, Windows separators and drive letters. Whatever you choose, say in the handoff what a stored project cwd is compared against and what is normalised on each side.

## Deliverable, part 2: tell the agents

`crates/runner-cli/src/help.rs` is the version-matched guide behind `runner help agents`. It mentions projects twice in 46 lines and never says a mission should be filed in one. Add a line stating that a mission or chat started inside a project's directory belongs to that project, and that `--project` overrides. Keep the guide's existing voice and length discipline — this is one line, not a section.

### The traps

- **The app must not change.** The start-mission modal and the sidebar's project menu already pass an explicit `project_id`; explicit input wins, so their behaviour is untouched. If a test of theirs changes, something is wrong with the precedence.
- **Sidebar placement is the visible half.** A mission whose row has a `project_id` but whose node was created at the root still looks unfiled. Check `ensure_mission_node` and the direct-chat equivalent receive the resolved id.
- **`resolve_cwd` has tests at `ops/project.rs:198` and `:214`** covering the existing direction and the unknown-project error. They keep passing, adapted to the new signature; do not delete them to make room.

Out of this mission: moving existing missions into projects retroactively, any change to project creation or deletion, the sidebar UI, `mission_set_project`, and `docs/` beyond this brief.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff with one question in front: **does every one of the four call sites end up with the same behaviour, and is the path matching safe on the `runner` / `runner-wt` case?** Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree; do not touch any other worktree or the repository root checkout. Do not launch or restart the Runner app — `target/debug/runner` is the GUI binary — and touch no real Runner data directory or database; every test uses `open_in_memory` or a temporary file.

**After the reviewer's clean verdict: commit, push `feat/680-project-from-cwd`, and open a PR against `main` that closes #680.** Then drive CI green with `gh pr checks <pr> --watch`, both the macOS and the Windows job — Windows matters here because path handling is exactly where the two platforms differ. **Do not merge.** Jason does the quality check and the final merge himself.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`.

Tests the resolver must carry: an exact cwd match; a descendant several levels down, as `.worktrees/<branch>` is; nested projects picking the longest; the `runner` / `runner-wt` sibling **not** matching; no project bound anywhere leaving it unfiled; an explicit project beating an inferable cwd; an explicit cwd beating a project's bound cwd. Plus one test per call site proving the resolved id reaches the stored row.

The PR body carries the manual check for Jason: `runner mission start` and `runner chat` from a worktree under `~/repos/yicheng47/runner`, with no `--project`, both appearing under the `runner` project in the sidebar.

## Handoff

Final Runner handoff, posted on the feed: branch and base commit; every file and function changed; the resolver's signature and each of the four call sites with what it now writes back; the normalisation decision and what it compares; the test list with what each one would catch; checks with results; the reviewer's explicit no-remaining-must-fix verdict; and the PR number.
