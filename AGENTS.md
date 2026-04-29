# Runner Agent Guide

This file is the repo-wide guide for any coding assistant working on Runner.
Keep shared conventions here instead of putting them only in a tool-specific
file such as `CLAUDE.md`.

## Product Context

Runner is a local desktop app for coordinating multiple CLI coding agents from
one UI. Users create reusable runners, compose them into crews, start missions,
and interact with each runner through a real PTY.

Core vocabulary:

- **Runner**: a configured CLI agent runtime, role, and system prompt.
- **Crew**: a named set of runner slots with exactly one lead.
- **Mission**: a live run of a crew, with one session per slot.
- **Session**: one spawned runner process attached to a PTY.
- **Event**: an NDJSON log entry used for mission coordination.

## Stack

- Frontend: React 19, TypeScript, Tailwind CSS 4, Vite, React Router.
- Desktop/backend: Tauri 2, Rust, SQLite via `rusqlite`.
- PTY runtime: `portable-pty`.
- Event transport: append-only NDJSON logs watched through `notify`.
- Bundled CLI: `runner`, built from the `cli/` workspace member.

## Project Map

- `src/`: React frontend.
- `src/components/`: shared UI and runtime components.
- `src/pages/`: route-level screens.
- `src-tauri/src/commands/`: Tauri command handlers.
- `src-tauri/src/session/`: PTY session manager.
- `src-tauri/src/event_bus/`: mission log watcher and Tauri event fanout.
- `src-tauri/src/router/`: signal router and runtime prompt adapter.
- `cli/`: bundled `runner` CLI used by spawned agents.
- `crates/runner-core/`: shared event-log primitives.
- `design/`: Pencil source files.
- `docs/arch/`: product and architecture references.
- `docs/impls/`: implementation plans and status logs.
- `docs/tests/`: validation and smoke-test plans.

## Development Commands

- Install deps: `pnpm install`.
- Start app in dev: `pnpm tauri dev` or `make dev`.
- Frontend typecheck: `pnpm exec tsc --noEmit` or `make typecheck`.
- Frontend lint: `pnpm run lint` or `make lint`.
- Rust tests: `cargo test --workspace` or `make test-rust`.
- Full local test target: `make test`.

Prefer the smallest check that covers the change. For frontend changes, run
typecheck and lint. For Rust behavior, run relevant `cargo test` targets.

## Engineering Conventions

- Follow existing local patterns before adding new abstractions.
- Keep changes scoped to the request. Avoid unrelated refactors.
- Do not revert user changes. If the working tree is dirty, inspect first and
  preserve unrelated edits.
- Use structured APIs and parsers when available instead of ad hoc string
  manipulation.
- Keep comments rare and useful. Explain non-obvious intent, not mechanics.
- Keep UI aligned with `design/runners-design.pen` when a node or frame is
  referenced by the user.
- Do not add repo conventions only to an agent-specific file. Update this file
  and leave tool-specific files as pointers if needed.

## Commit And PR Conventions

- Use focused commits with an imperative subject.
- Common scopes: `db`, `commands`, `ui`, `event-log`, `session`, `event-bus`,
  `router`, `cli`, `mission`, `docs`, `validation`.
- Example: `fix(session): preserve terminal geometry on tab switch`.
- For validation branches, keep PR descriptions current when scope changes.
- Do not add tool-specific co-author trailers unless the user explicitly asks.

## Notes For Agent Runtimes

This repository is intentionally agent-agnostic. Claude Code, Codex, or any
other assistant should read `AGENTS.md` as the shared guide. Tool-specific
instruction files may exist only as compatibility entrypoints and should point
back here.
