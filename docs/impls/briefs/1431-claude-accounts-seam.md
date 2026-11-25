# #1431 — Claude accounts under `runtimes/claude_code` behind an account seam

## Starting point

- Issue: [#1431](https://github.com/yicheng47/runner/issues/1431), item 5 of the [#1248](https://github.com/yicheng47/runner/issues/1248) cleanup tracker.
- Branch: `refactor/1431-claude-accounts-seam`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1431-claude-accounts-seam` (base `origin/main` `5869f774`, own `target/`).
- Other live worktrees: `chore-1477-gpui-pre-0.3.14` (running neon trio), `fix-818-terminal-round-trips` and `refactor-1429-start-chat-methods` (neon trio started alongside this one, app-only). Leave them alone.
- Fold this brief into the implementation commit before the final push.

## Problem and outcome

Claude Code accounts are a Claude Code-only concept, but `crates/runner-daemon/src/claude_accounts.rs` (1,943 lines) and `claude_accounts/sharing.rs` (1,623) sit at the daemon crate root, and shared modules name them directly. The session manager's spawn paths (`session/manager/{direct_spawn,mission_spawn,resume,fork}.rs`) call `claude_accounts::launch_account`, `config_pin`, `last_permission`, `resume_args` and `sharing::launch`, and `usage.rs` special-cases `Runtime::ClaudeCode` to swap in the selected account's snapshot.

Outcome: the module lives under `runtimes/claude_code/accounts/`, and the session manager and the usage service reach it through a runtime-neutral account / config-dir seam that the runtime adapter fills, as #977 did for trust and skills. Shared code stops naming Claude accounts, so a second runtime could get accounts without touching the session manager. No behaviour changes.

## Read first

- `crates/runner-daemon/src/claude_accounts.rs`, `claude_accounts/sharing.rs`
- `crates/runner-daemon/src/session/manager/{direct_spawn,mission_spawn,resume,fork}.rs`, `session/launch_definition.rs`
- `crates/runner-daemon/src/usage.rs` (the `Runtime::ClaudeCode` account swaps)
- `crates/runner-daemon/src/runtimes/mod.rs` (`RuntimeAdapter`), `runtimes/claude_code/mod.rs`, `runtimes/helpers.rs` (the #977 trust seam to copy)
- `crates/runner-core/src/protocol/api.rs` (request table naming `crate::claude_accounts::…`), `crates/runner-app/src/test_support.rs`
- `docs/features/archive/955-claude-accounts.md` and `docs/guide/guides/claude-accounts.md` for the behaviour that must not change

## Deliverables

1. **Move.** `claude_accounts.rs` and `claude_accounts/sharing.rs` become `runtimes/claude_code/accounts/{mod,sharing}.rs`, with their tests. Update every path, including the request table in `runner-core/src/protocol/api.rs` and `runner-app/src/test_support.rs`; no crate-root re-export shim.
2. **Seam.** Give the session manager's direct, mission, resume and fork spawn paths and the usage service a runtime-neutral hook (an adapter method or an accounts trait the Claude Code adapter implements and other runtimes leave at a no-op default) for: choosing the launch account, applying its config dir and sharing to the spawn spec, carrying the permission mode and resume args across resume and fork, and swapping usage to the active account. After the change `session/manager/` and `usage.rs` contain no `claude_accounts` path and no `Runtime::ClaudeCode` account branch.
3. **Other call sites.** Ops handlers that exist only for Claude account requests (`ops/runtime.rs`, `ops/mcp.rs`, `ops/skills.rs`, `companion/actions.rs`, `runtime_status/models.rs`, `repo/session.rs`) may call the moved module by its new path; route them through the seam only where it falls out of the same change.
4. **Unchanged.** The client protocol (`ClaudeAccounts`, `ClaudeSessionAccount`, request names and fields), the SQLite schema, persisted JSON (including the serialized `claude_account` field of the launch definition) and every golden. Moved tests keep their assertions.
5. **Docs.** Update module paths in `docs/arch/` if any name the old location. No guide change, since nothing user-visible changes.

Non-goals: #1363 (switching the host's account from a remote window), accounts for another runtime, renaming protocol or storage fields.

## Validation and handoffs

- **@coder:** `cargo test -p runner-daemon`, `cargo test -p runner-core`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`. Hand `@reviewer` an unpushed commit SHA with a summary and the before/after reference counts of `claude_account` in `session/manager/` and `usage.rs`.
- **@reviewer:** Confirm the seam carries no Claude naming into shared code, the default implementation is a true no-op for other runtimes, the protocol, schema and persisted names are unchanged, and no test or golden was weakened. This touches `core-review` spawn wiring, so the clean review names the live checks below (adjusting them if the diff warrants) and the reviewed SHA.
- **@qa:** Take `script/qa-lock acquire --timeout 540` before building or launching the macOS development app; another mission may hold it, so wait as Test Scope describes. Use only your own `/tmp` folders and test chats, an inexpensive Claude model at low effort, and tiny prompts. Do not add, sign in, sign out or remove Claude accounts.
  1. **Single-account launch, resume and fork (SET-ACCOUNT-01 shape):** start a Claude Code chat with a unique marker, stop and resume it, then fork it. Resume recalls the marker, the fork gets a distinct conversation ID, and the pane and Settings → Agents → Accounts look as on `main`. Reason: the direct, resume and fork spawn paths now go through the seam.
  2. **Usage on the active account:** open the sidebar usage popover and confirm Claude's usage shows for the active account, as on `main`. Reason: `usage.rs`'s account swap moved behind the seam.
  3. **Mission launch:** start a one-slot bounded mission whose role runs Claude Code and confirm it launches on the active account. Reason: `mission_spawn.rs` moved to the seam.
  4. **Two accounts (only if the development data already has a second signed-in Claude account):** the SET-ACCOUNT-03/04 subset — switch the active account with Settings Use, start a new chat while an older one runs (new chat uses the new account, the old one keeps its own), resume a chat created under the other account and fork it across accounts. Restore the original active account afterwards. If there is no second account, record this check as not run with the reason; it stays covered by unit tests.

  Record pass/fail, candidate SHA and screenshots in `docs/tests/runs/1431-claude-accounts-seam-mac.md`, archive only your own chats and missions, clean `/tmp`, run `script/qa-lock release`, and report the verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). After `@reviewer` passes and `@qa` passes, `@coder` may push the branch and open a PR against `main` with `Closes #1431`, labelled `no-behavior-change` and `risk-medium` (keep the workflow's `core-review`), and drive CI green on both platforms. Do not merge.
