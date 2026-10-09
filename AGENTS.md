# Runner Agent Guide

This file is the repo-wide guide for any coding assistant working on Runner.
Keep shared conventions here instead of putting them only in a tool-specific
file such as `CLAUDE.md`.

## Product Context

Runner is a local desktop app for coordinating multiple CLI coding agents from one UI. Users create reusable roles, compose them into crews, start missions, and interact with each session through a real PTY.

Core vocabulary:

- **Role**: a configured CLI agent runtime and system prompt, reusable across crews.
- **Crew**: a named set of slots, each filled by a role, with exactly one lead.
- **Mission**: a live run of a crew, with one session per slot.
- **Session**: one spawned agent process attached to a PTY.
- **Event**: an NDJSON log entry used for mission coordination.

Surface hierarchy (strict — do not blur these in code, docs, or UI copy):

- **Window**: a real OS window (⇧⌘N). Multi-window support is impl 0018.
- **Tab**: one group of panes shown on a window's chat surface — the unit
  the layout picker builds and the sidebar highlights (formerly "chat
  group" / "split"). ⌘N starts a chat in a new tab.
- **Pane**: one slot inside a tab, holding a single chat session. Panes
  are filled from a pane's own New chat button or a sidebar pick.

## Stack

- Native UI: GPUI with `alacritty_terminal` as the terminal model and render buffer.
- Background daemon: `runnerd` runs the Rust application core and SQLite via `rusqlite`, exposed by `crates/runner-daemon`; the app and CLI are clients.
- PTY runtime: `portable-pty`.
- Event transport: append-only NDJSON logs watched through `notify`.
- Bundled CLI: `runner`, built from the `crates/runner-cli/` workspace member.

## Project Map

- `crates/runner-app/`: GPUI application, terminal renderer, and terminal fixture corpus.
- `crates/runner-daemon/`: `runnerd` and its UI-agnostic application core, including SQLite, the session manager, event bus and router, and the client-protocol server the app and CLI connect to.
- `crates/runner-cli/`: the bundled `runner` CLI, used by spawned agents inside a mission and by people, scripts and agents outside one.
- `crates/runner-core/`: shared types, event-log primitives, client protocol and daemon launch/connection.
- `design/`: Pencil source files.
- `docs/arch/`: architecture references (how it works).
- `docs/product/`: product vision and direction (why we're building this, what surfaces matter).
- `docs/features/`: in-progress feature specs, named `{tracking-issue}-{slug}.md` since 2026-09-01 (file the issue first); shipped specs live in `docs/features/archive/`.
- `docs/impls/`: implementation plans; shipped plans live in `docs/impls/archive/`, mission briefs in `docs/impls/briefs/`.
- `docs/tests/`: validation and smoke-test plans; records of shipped features live in `docs/tests/archive/`.
- `docs/roadmap.md`: where the project is, mirrored from the GitHub milestones and dated.
- `docs/tech/`: deep dives on the libraries Runner builds on (how the dependencies work), pinned to the versions in `Cargo.lock`.

## Development Commands

- Start the native app against the development database: `make run`. It stops the development daemon before launching, so the rebuilt sidecar owns the sessions. `runner-dev daemon stop` stops that daemon explicitly.
- Format: `make fmt`.
- Clippy: `make clippy`.
- Workspace tests: `make test`.
- Local validation: `make verify` (check + test + clippy + fmt-check).

CI runs Clippy and workspace tests with the `ci` Cargo profile, which uses lighter dependency optimization and debug information. Install the pinned prebuilt nextest locally on macOS with `curl -LsSf https://get.nexte.st/0.9.148/mac | tar zxf - -C "${CARGO_HOME:-$HOME/.cargo}/bin"`; Windows installation is documented in [Windows validation](docs/arch/windows.md#validation). Reproduce CI with `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci --timings`; macOS also checks formatting and Clippy with `--features updater`. `make test` continues to use `cargo test`. Normal development uses the `dev` profile; releases use level 3 optimization with thin LTO and Cargo's default codegen units.

On Windows, use `.\make.cmd run` to build and start the app with its CLI sidecars, or `.\make.cmd build` to build only. Add `--release` for optimized binaries under `target\release`; the default development binaries are under `target\debug`. Use `.\make.cmd clean` to remove the Cargo target directory, or `.\make.cmd clean --release` to clean only release outputs; close the development app and finish other builds first. Installed apps, user data, Rust toolchains, and Cargo's shared dependency cache are kept. The script works in PowerShell and Command Prompt without GNU Make; use the native Cargo commands in [Local Windows development](docs/arch/windows.md#local-windows-development) for checks.

Prefer the smallest check that covers the change. For native UI changes, run the `runner-app` tests plus workspace clippy; for core behavior, run the relevant crate tests.

## Test Scope

Feature validation covers the behavior changed by the diff and regressions directly affected by it. Before testing, list the changed behaviors and selected checks, with a concrete reason connecting each check to the change. Select relevant cases from the [live regression suite](docs/tests/regression/README.md) and add feature-specific checks; do not run the whole suite for an ordinary feature. Unchanged IME, agent runtimes, authentication, session lifecycle and other shared behavior are not automatic feature checks merely because the tested UI contains them. For a card-padding change, for example, verify terminal backgrounds and selection stay inside the card; add IME checks only if the change affects input or composition positioning.

Broaden coverage for major changes, such as a runtime/session/daemon rewrite, a persistence migration or a refactor affecting behavior across the app, or when Jason explicitly requests broader testing. State why the change needs that coverage. Required automated gates and CI still apply. Reusing the full-smoke procedure's setup, candidate identity, isolation, evidence and cleanup rules does not expand the selected cases or authorize agent/account use. An unavailable prerequisite blocks an in-scope check; an unrelated check is outside scope, not a feature blocker. A feature pass means every selected required check passed, and must not be described as a full regression pass. Preserve failures and record any later scope change explicitly.

## Full Smoke Tests

When Jason explicitly requests a complete smoke test for a major refactor or important release, follow [Full smoke test procedure](docs/tests/full-smoke-test.md) for setup, isolation, candidate identity, computer use and cleanup, and select cases from the maintained [live regression suite](docs/tests/regression/README.md): active smoke-tier rows before releases, all active rows for a complete regression. Add the change's feature-specific checks and record platform or runtime gaps explicitly. Ordinary implementation and CI do not authorize launching real agents; an explicit live smoke-test request supplies authorization for the bounded test chats and missions in the procedure. Use development data, preserve existing sessions, and archive only the test chats and missions you created. Record runs separately in `docs/tests/runs/YYYY-MM-DD-<platform>.md` using the suite's case IDs and evidence format; the [#777 run](docs/tests/archive/777-runtime-adapter-smoke.md) is the first historical runtime example.

## Worktrees

The checkout at the repository root stays on `main`. Every branch of work gets its own linked worktree under `.worktrees/`, named after the branch with its slashes flattened: `fix/659-session-start-status` lives in `.worktrees/fix-659-session-start-status`. Create it with `git worktree add .worktrees/<flattened-branch> -b <branch> origin/main` and remove it with `git worktree remove` once the branch has merged. Because the directory name is the branch name, an editor window's title says which work it holds, and either name can be derived from the other.

`.worktrees/` is ignored, and nesting is safe in the ways that matter: `git clean -xfd` skips a nested worktree and reports it as a skipped repository, so removing one takes a deliberate `-ff`; the workspace members in `Cargo.toml` are explicit paths, so a nested checkout is never part of a build; and ripgrep honours the ignore, so a search from the root checkout does not cross into another branch's copy.

Each worktree carries its own `target/` and pays for its own build. Do not point them at a shared `CARGO_TARGET_DIR`: Cargo takes an exclusive lock on the target directory, so concurrent worktrees would queue behind each other instead of building in parallel.

Work stays inside its own worktree. When several are live at once, treat the others as another machine's checkout. Design files are the exception: `.pen` files are opened and edited only in the root checkout on `main`, even when the code they describe is on a branch.

## Engineering Conventions

- Follow existing local patterns before adding new abstractions.
- Keep platform window chrome in `crates/runner-app/src/platform_ui/{macos,windows}.rs` and font mappings in the adjacent `fonts_{macos,windows}.rs`, selected at compile time. Windows layout changes must preserve the macOS implementation; share sidebar and workspace content across platforms.
- Keep changes scoped to the request. Avoid unrelated refactors.
- Do not revert user changes. If the working tree is dirty, inspect first and
  preserve unrelated edits.
- Use structured APIs and parsers when available instead of ad hoc string
  manipulation.
- Keep comments rare and useful. Explain non-obvious intent, not mechanics.
- Treat `design/runner-mvp-design.pen` as the historical MVP canvas. Put new product work in a feature-scoped `.pen` file and keep UI aligned with the file and node referenced by the user or feature spec. The active canvas is `design/runner.pen`, the product canvas of screens and `cmp/` components; feature specs go in `design/specs/<issue>-<slug>.pen`, one file per spec, since 2026-09-18. A spec file holds only the frames that spec needs, plus the `cmp/` components they reference; it is never a full copy of `runner.pen`. Design files live in the root checkout on `main`, never in a worktree: design and spec are settled on `main` first, and a branch's code follows them. The gpui-rewrite's parity exception (plan decision 1) ended at the `v0.6.0` cutover on 2026-08-23.
- `README.md` and `README.zh-CN.md` change together: a PR that edits one edits the other, and a paragraph that cannot be translated yet is marked `<!-- TODO zh-CN -->` rather than left silently behind.
- Do not add repo conventions only to an agent-specific file. Update this file
  and leave tool-specific files as pointers if needed.

## Commit And PR Conventions

- Use focused commits with an imperative subject. A crew mission's pull request is one commit; see Crew Missions.
- Common scopes: `db`, `commands`, `ui`, `event-log`, `session`, `event-bus`,
  `router`, `cli`, `mission`, `docs`, `validation`.
- Example: `fix(session): preserve terminal geometry on tab switch`.
- A doc-only change (`docs/`, the READMEs, this file) is committed straight to `main` from the root checkout, with no branch, worktree or pull request; a mission brief still goes on its mission's branch. Open a PR for docs only when the edit is large and spans several files, such as a restructure or a rename across the docs tree.
- For validation branches, keep PR descriptions current when scope changes.
- Bring a branch up to date by rebasing it onto `origin/main` and pushing with `git push --force-with-lease`; never merge `main` into a branch. A PR's history stays its own commits on top of `main`.
- Do not add tool-specific co-author trailers unless the user explicitly asks.

## Crew Missions

A crew mission ends in an open pull request by default. The crew works on its own branch in its own worktree, commits, pushes, opens the PR against `main`, and drives CI green on both platforms; then it stops. Jason reviews the PR and does the final merge, or explicitly asks the crew to merge after review and CI are clean. Crews do not cut a nightly or release without explicit authorization. Every mission brief states this in its authorization section, and a crew whose brief is silent on it follows this rule anyway.

An explicit merge request includes GitHub's configured automatic deletion of the merged remote branch; no separate confirmation is needed. Local branch and worktree cleanup still requires explicit authorization; see Post-mission cleanup.

A mission runs in its worktree. Before starting it, create the worktree as described under Worktrees and commit the brief on its branch; then start the mission with the worktree as its directory, `runner mission start --crew <crew> --cwd <repo>/.worktrees/<flattened-branch> …`, not `--project runner`. Every slot's agent and shell then start in the worktree instead of the root checkout, the mission still lands under the runner project because the project is inferred from the directory, and the brief names the same path.

A program that runs on an umbrella branch, such as #645's `feat/645-runnerd`, creates each mission's worktree from the umbrella instead of `origin/main` and opens the mission's PR against the umbrella. The umbrella follows `main` by rebase between missions and reaches `main` as one PR when the program's gate passes; see the program's plan in `docs/impls/`.

A mission lands as a single commit. Before pushing, the crew squashes everything on its branch, the brief commit included, into one commit on top of `main` (or of the umbrella, for an umbrella program), with a subject that names the change rather than the brief. Fixes after the push, from review or CI, are folded into that commit with `git commit --amend` and pushed with `git push --force-with-lease`, so the pull request always shows one commit. Split into more than one commit only when the mission covers changes that are unrelated to each other, such as a fix and an independent cleanup that could each be reverted alone, and say why in the pull request body. Every mission brief states this in its authorization section, and a crew whose brief is silent on it follows this rule anyway.

### Post-mission cleanup

When Jason asks for the post-mission cleanup, usually together with the merge, it covers three steps, in this order, once the PR has merged:

1. **Archive the mission.** Stop it if a session is still live (`runner mission stop <id>`), then `runner mission archive <id>`.
2. **Remove the worktree and branches.** `git worktree remove .worktrees/<flattened-branch>`, then delete the local branch. Squash and rebase merges leave the branch unmerged by ancestry, so confirm the PR merged with `gh pr view` rather than `git branch --merged`. GitHub deletes the remote branch on merge; delete it by hand only if it is still there.
3. **Archive the docs**, in one doc-only commit on `main`. Move the test record to `docs/tests/archive/` and promote its lasting live checks into [`docs/tests/regression/`](docs/tests/regression/README.md), reconciled with current code, or have the record state why none apply. Run results stay separate from maintained cases. Prune the mission briefs (git history keeps them) unless [`docs/impls/briefs/README.md`](docs/impls/briefs/README.md) keeps one as a reference. When the tracking issue closes, which for a multi-PR feature means after its last PR, move the spec to `docs/features/archive/` and its [`docs/features/README.md`](docs/features/README.md) entry to Shipped. Repoint every link to the moved files and update [`docs/roadmap.md`](docs/roadmap.md).

## Notes For Agent Runtimes

This repository is intentionally agent-agnostic. Claude Code, Codex, or any
other assistant should read `AGENTS.md` as the shared guide. Tool-specific
instruction files may exist only as compatibility entrypoints and should point
back here.
