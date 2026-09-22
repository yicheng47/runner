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
- Application core: Rust, SQLite via `rusqlite`, exposed by `crates/runner-backend`.
- PTY runtime: `portable-pty`.
- Event transport: append-only NDJSON logs watched through `notify`.
- Bundled CLI: `runner`, built from the `crates/runner-cli/` workspace member.

## Project Map

- `crates/runner-app/`: GPUI application, terminal renderer, and terminal fixture corpus.
- `crates/runner-backend/`: UI-agnostic application core, including SQLite, session manager, event bus, router, and MCP server.
- `crates/runner-cli/`: the bundled `runner` CLI, used by spawned agents inside a mission and by people, scripts and agents outside one.
- `crates/runner-core/`: shared event-log primitives.
- `design/`: Pencil source files.
- `docs/arch/`: architecture references (how it works).
- `docs/product/`: product vision and direction (why we're building this, what surfaces matter).
- `docs/features/`: in-progress feature specs, named `{tracking-issue}-{slug}.md` since 2026-09-01 (file the issue first); shipped specs live in `docs/features/archive/`.
- `docs/impls/`: implementation plans; shipped plans live in `docs/impls/archive/`, mission briefs in `docs/impls/briefs/`.
- `docs/tests/`: validation and smoke-test plans; records of shipped features live in `docs/tests/archive/`.
- `docs/roadmap.md`: where the project is, mirrored from the GitHub milestones and dated.
- `docs/tech/`: deep dives on the libraries Runner builds on (how the dependencies work), pinned to the versions in `Cargo.lock`.

## Development Commands

- Start the native app against the development database: `make run`.
- Format: `make fmt`.
- Clippy: `make clippy`.
- Workspace tests: `make test`.
- Local validation: `make verify` (check + test + clippy + fmt-check).

CI runs Clippy and workspace tests with `--profile ci`, which uses lighter dependency optimization and debug information. Reproduce it with `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and `cargo test --locked --workspace --no-fail-fast --profile ci --timings`; macOS also checks formatting and Clippy with `--features updater`. Normal development uses the `dev` profile; releases use level 3 optimization with thin LTO and Cargo's default codegen units.

On Windows, use `.\make.cmd run` to build and start the app with its CLI sidecars, or `.\make.cmd build` to build only. Add `--release` for optimized binaries under `target\release`; the default development binaries are under `target\debug`. Use `.\make.cmd clean` to remove the Cargo target directory, or `.\make.cmd clean --release` to clean only release outputs; close the development app and finish other builds first. Installed apps, user data, Rust toolchains, and Cargo's shared dependency cache are kept. The script works in PowerShell and Command Prompt without GNU Make; use the native Cargo commands in [Local Windows development](docs/arch/windows.md#local-windows-development) for checks.

Prefer the smallest check that covers the change. For native UI changes, run the `runner-app` tests plus workspace clippy; for core behavior, run the relevant crate tests.

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

- Use focused commits with an imperative subject.
- Common scopes: `db`, `commands`, `ui`, `event-log`, `session`, `event-bus`,
  `router`, `cli`, `mission`, `docs`, `validation`.
- Example: `fix(session): preserve terminal geometry on tab switch`.
- For validation branches, keep PR descriptions current when scope changes.
- Do not add tool-specific co-author trailers unless the user explicitly asks.

## Crew Missions

A crew mission ends in an open pull request, never a merge. The crew works on its own branch in its own worktree, commits, pushes, opens the PR against `main`, and drives CI green on both platforms; then it stops. It does not merge the PR, delete its branch or worktree, or cut a nightly or release. Jason reviews the PR and does the final merge. Every mission brief states this in its authorization section, and a crew whose brief is silent on it follows this rule anyway.

## Notes For Agent Runtimes

This repository is intentionally agent-agnostic. Claude Code, Codex, or any
other assistant should read `AGENTS.md` as the shared guide. Tool-specific
instruction files may exist only as compatibility entrypoints and should point
back here.
