# Runner

Spawn a runner. Create your crew. Ship the feature.

Runner is a local desktop app for assembling a crew of CLI coding agents — Claude Code, Codex, and friends — giving each runner a role and a brief, and coordinating their work from one window.

> Status: pre-alpha. Crew + runner config, mission start/stop, event-log tailing, the signal router, the bundled `runner` CLI, and PTY-backed mission workspaces run end-to-end on macOS / Linux. See `docs/impls/0001-v0-mvp.md` for the current plan and status.

## What it does

- **Crews** — create a crew, pick which runners are on it.
- **Runners** — each runner is a local CLI runtime (claude, codex, ...) with its own role, system prompt, and working directory.
- **Event bus** — runners talk to each other through an append-only NDJSON log the router can read.
- **Signal router** — a deterministic parent-process bridge that routes built-in signals between runners and the human.

## Stack

- Tauri 2 + Rust backend
- React 19 + TypeScript + Tailwind 4 frontend
- SQLite for persistence
- PTY-based subprocess control (portable-pty) for running real CLI agents

## Develop

```sh
pnpm install
pnpm tauri dev
```

Repo-wide coding-agent and contributor conventions live in
[AGENTS.md](./AGENTS.md).

## License

MIT
