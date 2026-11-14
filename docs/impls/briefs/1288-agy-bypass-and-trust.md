# Mission Brief — #1288 Antigravity Bypass Toggle & Symlink Folder Trust

- **Issue:** [#1288](https://github.com/yicheng47/runner/issues/1288) (`feature`, `P2`, plus folded `agy` new-dir trust fix per Jason)
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1288-agy-bypass-and-trust`, branch `feat/1288-agy-bypass-and-trust`, off `origin/main` `d86af498`. Keep the root checkout on `main` and work strictly in this worktree with its own `target/` (never a shared `CARGO_TARGET_DIR`).
- **Parallel isolation:** sibling worktree `.worktrees/feat-1141-git-changes` is active concurrently; build purely against `origin/main` (`d86af498`) and never read or edit another worktree.

## Problem And Goal

1. **Antigravity Bypass permissions toggle (#1288):** In **Settings → Agents → Launch**, Runner exposes a chat-level `RuntimeLaunchOption::Bypass` switch only for Codex and Claude Code, even though Antigravity CLI (`agy`) already defines `PermissionMode::Bypass` (`--dangerously-skip-permissions`). Expose **Bypass permissions** on Antigravity's Launch card (`On: --dangerously-skip-permissions`) and apply it consistently across chat launches, resumes, and Command previews while keeping missions unchanged.
2. **Antigravity new-dir / symlink folder trust:** `Antigravity::seed_trust` (`agy_trust::seed_project_trust`) already adds spawned directories to `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`, but it only writes `std::fs::canonicalize(cwd)`. When a session opens via a symlinked directory (such as `/Users/jason/repos/<dir>` -> `/Volumes/samsung/repos/<dir>` or `/tmp/<dir>` -> `/private/tmp/<dir>`), `agy` checks the exact launch path without resolving symlinks and still opens `"Do you trust the contents of this project?"`. Seed both the normalized non-canonical `cwd` (when distinct from canonical) and `canonicalize(cwd)` in `agy_trust`, guarding both against broad trust roots (`is_broad_trust_root`), and verify no direct-chat, resume, fork, or mission spawn path skips `seed_trust` for `Runtime::Antigravity`.

## Read First

1. `crates/runner-core/src/protocol/runtime.rs` — `RuntimeLaunchOption::Bypass`, `RuntimeCommandNote` (`CodexBypass`/`ClaudeBypass` or new `AntigravityBypass`).
2. `crates/runner-daemon/src/runtimes/mod.rs` and `crates/runner-daemon/src/session/manager/spawn_spec.rs` — `apply_launch_options` and `command_lines` preview notes/options for `--dangerously-skip-permissions`.
3. `crates/runner-app/src/surfaces/settings/agents/launch.rs` and `crates/runner-app/i18n/{en,zh-hans}.toml` — Launch card switch copy (`ui.launch_antigravity_bypass*` or shared bypass wording) and `preview_note`.
4. `crates/runner-daemon/src/runtimes/antigravity/agy_trust.rs` and `crates/runner-daemon/src/session/manager/tests/antigravity.rs` — `trustedWorkspaces` preseed and existing spawn/trust tests.
5. `crates/runner-daemon/src/session/manager/tests/permissions.rs` and `crates/runner-app/src/surfaces/settings/agents/tests.rs` — launch-option persistence, stripping conflicting `--mode` flags, and Settings UI coverage.
6. `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md` — Launch options table and prose.

## Deliverables And Boundaries

- **Bypass launch option:** enable `RuntimeLaunchOption::Bypass` for `Runtime::Antigravity` across `runner-core`, `runner-daemon` (`apply_launch_options`, command preview), `runner-app` (`launch.rs` switch row, `preview_note`, i18n in `en.toml` and `zh-hans.toml`), and guide docs (`supported-agents.md` + `zh-CN`). Match Codex/Claude behavior: chat starts and resumes apply `--dangerously-skip-permissions` and strip conflicting role `--mode` flags when Bypass is on; missions always bypass; other runtimes and popover/daemon usage parsing are out of scope.
- **Symlink-safe `agy` trust:** update `agy_trust::seed_project_trust_at` to record both the normalized non-canonical `cwd` (when distinct) and `canonicalize(cwd)` in `trustedWorkspaces` while skipping broad roots (`/`, `$HOME` canonical or symlink) and preserving all unrelated bytes in `settings.json`. Check if `direct_spawn.rs` / `resume.rs` / `mission_spawn.rs` / `fork.rs` skip any `agy` new-dir spawn path (do not change `claude_code` or `codex` trust behavior).
- **Tests and goldens:** update golden expectations (`spawns.json` or command-preview expectations if affected), extend `permissions.rs` and `settings/agents/tests.rs` for `Runtime::Antigravity` Bypass toggle + preview + `--mode` stripping, and add unit tests in `agy_trust.rs` for symlinked `cwd` (both entries added, idempotent on repeat, broad root rejected).

## Validation And Handoffs

1. **coder:** pass `make fmt`, `cargo nextest run --locked --workspace --cargo-profile ci`, and workspace Clippy (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, plus `--features updater` on macOS). Do NOT run `make run`. Fold this brief (`docs/impls/briefs/1288-agy-bypass-and-trust.md`) into an implementation commit before final push (never leave a standalone brief commit in PR history). Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff only (`spawn_spec.rs` is a `core-review` path, `agy_trust.rs` symlink/broad-root safety, UI/i18n/bilingual guide parity, tests). Pass the clean candidate SHA to `qa` with the live checks below plus any the diff adds.
3. **qa:** take `script/qa-lock acquire --timeout 540` before building, run `make run` in this worktree, preserve any pre-existing development sessions/daemon untouched, clean up only QA-created sessions/missions, temp dirs, and QA entries added to `~/.gemini/antigravity-cli/settings.json`, and release `script/qa-lock release` once app and daemon exit (or follow manager release guidance if pre-existing user sessions keep the dev daemon alive). Live checks on macOS with screenshots:
   - **Settings UI & Command preview:** in Settings → Agents → Antigravity CLI, verify **Bypass permissions** (`On: --dangerously-skip-permissions`) renders cleanly, persists when toggled, toggles `--dangerously-skip-permissions` in the **Chat** command preview, and leaves **Mission** preview unchanged.
   - **Chat Bypass behavior & new-dir symlink trust:** start an Antigravity CLI chat in a fresh directory under `/tmp/...` (via the `/tmp` symlink, not `/private/tmp`) with Bypass off, verify no `"Do you trust the contents of this project?"` dialog blocks startup and `~/.gemini/antigravity-cli/settings.json` records both `/tmp/...` and `/private/tmp/...`; then enable Bypass, start a fresh Antigravity CLI chat in `/tmp/...`, and confirm the spawned process carries `--dangerously-skip-permissions` and runs a bounded tiny prompt cleanly.
   Record the run in `docs/tests/runs/1288-agy-bypass-and-trust-mac.md` and in the PR description.

## Authorization

The `agy trio` is authorized for `#1288`. Once review and macOS QA pass cleanly, fold the brief into the implementation commit, push `feat/1288-agy-bypass-and-trust`, and open a PR against `main` closing `#1288`. Apply `risk-medium` (keep automatic `core-review` if triggered by `spawn_spec.rs`, and `reviewed` on the final head SHA; leave `needs-live-test` off only if all required checks are satisfied, or keep it if Windows verification remains pending). Drive CI green on all platforms and stop at the open PR without merging.
