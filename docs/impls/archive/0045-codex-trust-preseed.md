# Codex trust pre-seeding: mark the project trusted before spawn

## Status

Implemented (#404 closed 2026-08-17, `1b7ee92`, shipped in v0.5.2; native `crates/runner-backend/src/session/codex_trust.rs`). Fixes [#404](https://github.com/yicheng47/runner/issues/404) and absorbs the codex half of the #403 trust pre-seeding direction. Supersedes the boot-stall cue previously specced under this number (`BootWatch` byte-budget heuristic) — that mission was archived and its working-tree changes reverted; the general "agent needs you" tier remains future work under `docs/features/52-hook-based-session-status.md`. Approach validated against orca (`~/repos/ai/orca`): `src/main/agent-trust-presets.ts` documents the artifact and why CLI flags are not equivalent; the format is verified there against codex's own source (`codex-rs/tui/src/onboarding/trust_directory.rs`).

## Problem

A codex session spawned in a project codex has never seen boots into the "Do you trust this folder?" onboarding modal. The dialog paints one screen and goes quiet: the byte-flow `IdleDetector` reports Idle, the rail shows a dim presence dot, and the mission reads as stopped while the first-turn goal argv (`spawn.rs`, `first_turn_argv`) sits undelivered behind it. Stray or injected input can answer the modal wrong and kill the CLI. Live repro 2026-08-17: the mission created to fix this bug was itself stopped by it. Claude-code does not hit this in practice; trae and qoder are second-class runtimes — support on demand if an issue appears.

## Key Decisions

1. **Prevent, don't detect.** Before spawning codex, write the exact trust artifact codex writes after the user accepts the dialog, so the dialog never renders and the first turn delivers normally. No detection heuristic: the reverted `BootWatch` byte-budget cue is dropped, and the broader "waiting on something" state arrives later with spec 52's hook-based status.

2. **The artifact is a `trust_level` entry in `~/.codex/config.toml`.** Codex records per-project trust as `[projects."<absolute path>"]` with `trust_level = "trusted"`. Edit via `toml_edit::DocumentMut` — already a dependency, same doc-preserving pattern as `codex_write_at` in `commands/mcp.rs` — so the user's existing config survives byte-for-byte outside the added block. Match codex's own serialization: an explicit `[projects."<path>"]` table header, not an inline table.

3. **Insert-only, never overwrite.** If the project already has any `trust_level` entry — `trusted` or `untrusted` — leave it untouched. Starting a session in a folder is the operator's trust decision and runner materializes it, but an explicit `untrusted` marking is also an operator decision and outranks ours. No write also means no file churn on the common already-trusted path.

4. **Canonicalize, and resolve worktrees to the main repo root.** Codex realpaths before comparing, so seed the realpath of the resolved spawn cwd (macOS `/tmp` vs `/private/tmp`). If the cwd is a linked git worktree (`.git` is a file with `gitdir:` pointing into `<main>/.git/worktrees/<name>`), codex resolves trust at the **main repo root** — seed that instead. Validate the worktree backlink (`<gitdir>/gitdir` must point back to the cwd's `.git` file) before widening, so workspace-controlled `.git` contents can't trick us into trusting an arbitrary path — mirror orca's `resolveCodexProjectTrustRoot` checks. Harmless today; correct the day spec 61's worktree isolation lands.

5. **Seed on every codex spawn path, non-fatally.** Mission spawn, direct chat, and resume all seed, gated on effective runtime == `codex` — the same no-op-for-other-runtimes shape as `enter_claude_launch_gate`. Seeding failure (unreadable config, permissions) logs a WARN and the spawn proceeds: the worst case is the dialog appearing, which is exactly today's behavior. Fail-safe forward too — if a codex update changes the trust format, the dialog comes back; nothing breaks.

## Goals

- A codex mission session in a fresh project delivers its first turn without human intervention; the trust dialog never renders.
- The user's `~/.codex/config.toml` is preserved exactly outside the single added block; repeat spawns are no-ops.
- No schema changes, no migrations, no frontend work.

## Non-Goals

- Claude-code, trae, or qoder trust handling — codex only, others on demand.
- Boot-stall detection heuristics (superseded) or hook-based needs-you status (spec 52).
- Login prompts or other boot-time modals that cannot be pre-seeded.
- Flipping an existing `untrusted` entry.

## Implementation Notes

- New module `src-tauri/src/session/codex_trust.rs`: `seed_project_trust(cwd: &Path)` resolving config path from `$HOME`, with a path-injected `seed_project_trust_at(cwd, config_path)` seam for tests (same `_at` pattern as `IdleDetector`). Internals: realpath → worktree-root resolution (decision 4) → `toml_edit` insert-if-absent (decisions 2–3).
- `src-tauri/src/session/manager/spawn.rs`: call after cwd resolution, before `runtime.spawn`, on the mission-spawn, `spawn_direct*`, and resume paths, gated on the effective runtime.
- Constant for the config location beside `codex_path()` reuse — don't duplicate the `~/.codex/config.toml` literal; share or mirror `commands/mcp.rs::codex_path`.

## Validation

- Rust unit tests (`codex_trust.rs`, temp dirs): empty/missing config → block created; unrelated existing config preserved verbatim; existing `trusted` entry → file untouched; existing `untrusted` entry → file untouched; linked-worktree cwd → main root seeded; worktree with a forged `gitdir:` and no valid backlink → cwd seeded, not the forged target; symlinked cwd → realpath seeded.
- Manual: remove the project's `[projects."…"]` entry from `~/.codex/config.toml`, start a codex mission — no trust dialog, first turn delivers; re-check config.toml diff is exactly one block; restart the mission — file unchanged.
- `cargo test --workspace`.
