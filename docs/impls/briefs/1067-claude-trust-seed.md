# #1067 — Seed Claude Code folder trust before launch

Jason requested a codex trio mission to fix #1067 on 2026-11-03. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1067-claude-trust-seed` on `fix/1067-claude-trust-seed`, based on `origin/main` at `90940325b6b41a547bfd6ce74882b0e6ee68be64`. The root checkout stays on main.

## Problem

A Claude Code session that Runner starts in a folder Claude has not trusted stops at Claude's "Quick safety check: Is this a project you created or one you trust?" prompt. A mission's goal is never delivered and the slot looks hung; a direct chat waits silently too. #404 was closed by #405, which seeded only Codex; Copilot and Antigravity gained seeding later, while `runtimes/claude_code/` still uses the no-op `RuntimeAdapter::seed_trust`. It recurs in daily use because trust is stored per Claude account: on 2026-11-03 the active account's config (`~/.claude-work-3`) trusted 5 folders against 22 in `~/.claude.json`, so every new or switched account prompts once in every repo.

## Verified Claude behavior

Claude Code 2.1.317, read from the binary and checked live on 2026-11-03:

- Trust lives at `projects["<key>"].hasTrustDialogAccepted` in `$CLAUDE_CONFIG_DIR/.claude.json`, or in `~/.claude.json` when the variable is unset.
- The key is the cwd's canonical git root, and a linked worktree resolves to its main repository root: no worktree keys exist in any config on this machine although sessions run in worktrees daily. Outside git the key is the cwd's realpath, and any trusted ancestor also counts.
- Inside a repository the parent walk stops at the repository root, so trust on a parent folder does not cover a repo beneath it.
- Trust is never persisted for the home directory.
- Permission mode does not matter: `--permission-mode bypassPermissions` still shows the prompt, and no CLI flag or setting skips it for an interactive session.
- A `false` flag does not mean the user declined: configs here hold `false` entries for `~` and for a smoke folder. Treat `false` like a missing entry.
- A seeded entry in Claude's own shape suppresses the prompt (live probe, 2026-11-03). That shape, as Claude writes it for a newly trusted folder: `allowedTools: []`, `mcpContextUris: []`, `mcpServers: {}`, `enabledMcpjsonServers: []`, `disabledMcpjsonServers: []`, `hasTrustDialogAccepted: true`, `hasClaudeMdExternalIncludesApproved: false`, `hasClaudeMdExternalIncludesWarningShown: false`.

## Decisions for this mission

These settle #1067's open questions.

- Seed silently, as Runner does for Codex, Copilot and Antigravity: every Claude Code spawn path (mission, direct chat, respawn and resume), every permission mode, non-fatal on failure.
- Write into the configuration the child process will read: the `CLAUDE_CONFIG_DIR` of the session's effective launch environment (role env after `claude_accounts::launch_account`, then the shell env, then the daemon's env, as `config_pin` orders them), otherwise the native `~/.claude.json`. The native file is in scope here, unlike `onboarding::complete`.
- Key on the canonical git root with linked worktrees resolved to the main root, or on the cwd's realpath outside git. Reuse Codex's resolution (`resolve_project_trust_root`, `resolve_worktree_main_root`) by moving it into `runtimes/helpers.rs` instead of duplicating it, and refuse broad roots with `is_broad_trust_root`.
- A missing entry gets the full shape above. An existing entry only has `hasTrustDialogAccepted` set to `true`, and every other byte of the file is preserved; follow the byte-preserving approach in `claude_code/onboarding.rs` and write through `resolve_config_write_path` and `write_config_atomically`. Running Claude sessions rewrite this file, so keep the read-modify-write tight.

## Read first

- #1067 and PR #405.
- `crates/runner-daemon/src/runtimes/mod.rs` (`RuntimeAdapter::seed_trust`) and `session/manager/spawn_spec.rs` (`seed_runtime_project_trust`) with its callers.
- `runtimes/codex/codex_trust.rs` and its tests, `runtimes/copilot/copilot_trust.rs`, `runtimes/helpers.rs`.
- `claude_accounts.rs` (`config_pin`, `launch_account`) and `runtimes/claude_code/onboarding.rs`.
- [Agent configuration isolation](../../tests/full-smoke-test.md#agent-configuration-isolation) and RT-START-01 in [the runtime regression suite](../../tests/regression/runtimes.md).

## Deliverables

1. `ClaudeCode::seed_trust` implemented as decided, extending the seed seam only as far as needed to hand it the effective config location.
2. Unit tests mirroring Codex's: missing file and parent, empty config, a repo subdirectory seeds the repo root, a linked worktree seeds the main root, a forged gitdir does not widen trust, a symlinked cwd seeds its realpath, home and `/` are refused, an existing `true` is untouched, an existing `false` is flipped with every other key and byte preserved, a `CLAUDE_CONFIG_DIR` account writes only its own file and never `~/.claude.json`, a symlinked config stays a symlink, and concurrent seeds keep every entry.
3. Docs: the Claude Code trust paragraph in `docs/tests/full-smoke-test.md` says Runner now writes the entry, as the Codex paragraph does, and any guide page that tells people to accept Claude's trust prompt in a Runner-launched session is corrected.

Non-goals: a Needs-you cue for prompts seeding cannot prevent, a consent UI, behavior changes to other runtimes beyond moving the shared helpers, and editing a real Claude config outside QA's ledger.

## Validation and handoffs

The coder runs the new and affected daemon tests with locked dependencies in the worktree's own target directory, plus `cargo fmt --all --check`, workspace Clippy with warnings denied and `git diff --check`. The reviewer reviews the frozen snapshot. Live checks are required, because the change alters what a person sees when a Claude session starts. QA runs them in the development app under Test Scope, following the Claude trust ledger and cleanup rules in Agent configuration isolation:

- RT-START-01 for `claude-code`: a single-slot mission in a never-trusted folder under `/tmp` runs its goal with no trust prompt and no keystrokes.
- A direct Claude chat in a second never-trusted folder opens at the composer.
- If the development app has a second Claude account, one case repeated under it writes the entry into that account's `.claude.json` only; otherwise record that case as Blocked.
- A before-and-after diff of every touched `projects` map shows only the run's own entries, and cleanup removes them.

QA records the run in `docs/tests/runs/<date>-macos.md`.

## Authorization

Approved crew: codex trio (coder, reviewer, qa). The crew may commit, push this branch and open one PR against main that fixes #1067, labelled `needs-live-test` until QA's checks pass. Do not merge.
