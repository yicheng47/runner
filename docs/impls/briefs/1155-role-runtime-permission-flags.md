# #1155 — A role's runtime change rewrites its permission flags

Jason approved a claude duo mission on 2026-11-04. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1155-role-runtime-permission-flags` on `fix/1155-role-runtime-permission-flags`, based on `origin/main` at `b156cdbd`. Issue: [#1155](https://github.com/yicheng47/runner/issues/1155), P2, milestone 0.19.

## Problem

`ops::role::update` only touches permission flags when the patch carries `permission_mode`. A runtime change without it keeps `input.args.unwrap_or(existing.args)` unchanged, so the old runtime's permission flags stay in the role's args. `runner role update demo-x --runtime claude-code` on a Codex role created with `--permission bypass` leaves `--ask-for-approval never --sandbox workspace-write`, `claude` rejects them, and every chat from the role stops at once with "The PTY exited with code 1" and no agent output.

The issue's comment adds a second case: passing the mode again does not repair a role that is already broken. `runner role update demo-release --runtime claude-code --permission bypass` on the already-switched role stores `--ask-for-approval never --sandbox workspace-write --permission-mode bypassPermissions`, because the runtime no longer changes, so only Claude Code's own flags are stripped. Today the only CLI workaround is to delete and recreate the role.

The Roles form always sends a permission mode, so the desktop editor is not affected; the CLI and any client that omits `permission_mode` are.

## Intended outcome

- A runtime change without `permission_mode` keeps the role's permission mode and rewrites its flags for the new runtime: read the mode under the prior runtime (`Permissions::infer`), strip the prior runtime's permission flags, apply that mode under the new runtime. When the new runtime does not offer that mode, choose the closest sensible fallback and say which in the PR. Unrelated custom arguments stay, in order.
- An update that carries `permission_mode` on a role already broken by this bug repairs it: the stale permission flags of another runtime are removed, and the role ends with only the current runtime's flags for that mode. Do not strip a custom arg that the current runtime itself accepts; check the `strip_flags` tables for name collisions across runtimes and handle them deliberately.
- An update that changes neither runtime nor permission mode leaves args exactly as today.

The coder decides the shape; the reviewer checks it. Keep the logic beside the existing `permission_mode` branch in `update`, reusing `Permissions` rather than parsing flags by hand.

## Read first

- `AGENTS.md`, including Test Scope and Crew Missions.
- `crates/runner-daemon/src/ops/role.rs`: `update`, `create`, and the existing update tests.
- `crates/runner-core/src/protocol/runtime_metadata.rs`: `Permissions` (`strip`, `apply`, `infer`, `offered`) and each runtime's `strip_flags`.
- `crates/runner-daemon/src/runtimes/mod.rs`: `permissions()`; `runtimes/tests.rs` for existing inference tests.
- `crates/runner-cli/src/command/role.rs`: how `role update` builds the patch.

## Deliverables and boundaries

- The fix in `ops::role::update`.
- Tests in `ops::role`: Codex bypass → Claude Code without a mode stores `--permission-mode bypassPermissions` and no Codex flags; the reverse direction; a non-default mode other than bypass; custom args survive; a runtime with no permission flags on either side; the already-broken role repaired by an update with `permission_mode`; an update that changes neither leaves args untouched.
- No guide change unless CLI help or documented role behaviour changes; then update `docs/guide/` and regenerate with `make guide-cli`.
- Non-goals: a migration that repairs stored roles automatically, changes to the Roles form, slot runtime overrides, or mission permission handling.

## Validation and handoffs

Name the changed behaviours and the check that covers each before running anything. Run the `runner-daemon` role tests (and `runner-core` or `runner-cli` tests if touched) with `--locked` and this worktree's own target directory, workspace Clippy with warnings denied, `cargo fmt --all --check` and `git diff --check`. Drive the required macOS and Windows CI checks green. No full smoke.

@coder implements and hands the diff to @reviewer. @reviewer reviews code only and decides from the diff whether live checks are needed. The fix is daemon logic that the in-process `ops::role` tests exercise, so none are expected; if the clean review names any, the mission's operator adds the codex `qa` role once the mission settles.

## Authorization

Jason approved this claude duo mission: commit, push this branch and open one PR against `main`, labelled per AGENTS.md. Stop at the open PR with green CI and report through Runner with the PR URL, final SHA, changes and validation.
