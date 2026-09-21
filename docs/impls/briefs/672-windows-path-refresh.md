# 672 — Refresh must rediscover a CLI installed since launch, on Windows

[#672](https://github.com/yicheng47/runner/issues/672), P2, milestone 0.11. Work in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-672-windows-path-refresh`, a linked worktree of the repository. Stay inside it; other worktrees hold other people's work. You are on branch `fix/672-windows-path-refresh`, cut from `main` at `32a653d`; its first commit carries this brief. Build on it; do not rebase, squash or create another branch.

Read first: this brief; the issue end to end; `crates/runner-backend/src/{runtime_status.rs, shell_path.rs, cli_install.rs}`; `docs/arch/windows.md`; `AGENTS.md` for the conventions.

## The bug, and the seam that fixes it

Discovery finds an agent by walking a PATH string. `direct_chat_path` (`runtime_status.rs:314`) builds that string with `launch::compose_path` from three inputs, and on Windows only one of them ever has anything in it: `std::env::var("PATH")`, the process PATH inherited from Explorer at launch. A running process never sees a later edit to the user PATH, which is exactly what an agent's installer writes. So pressing Refresh re-runs detection against a string that cannot have changed.

The second input is empty because Windows has no probe. `start_background_discovery` (`runtime_status.rs:407`) spawns a thread that calls `shell_path::resolve_login_shell_env()` at `:419` and feeds the result through `apply_discovery_result`; on macOS that is a real `zsh -ilc` which re-reads the user's profile, and that is the only reason Refresh works there. The `#[cfg(not(unix))]` implementation (`shell_path.rs:257`) returns `DiscoveryOutcome::NoShell` with `LoginShellEnv::default()` — nothing.

**So the fix belongs at that one seam.** Give Windows a real environment source in `resolve_login_shell_env`, and everything downstream — `compose_path`, `direct_chat_path`, `find_executable`, the last-known-good store — works unchanged, along the same path macOS already takes.

## Deliverable

The `#[cfg(not(unix))]` `resolve_login_shell_env` returns a `LoginShellEnv` whose `path` is read from the registry rather than left empty:

- The user PATH from `HKCU\Environment`, which is where Runner's own CLI install writes and where most agent installers write.
- The machine PATH from `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`, for installers that write there instead.
- Composed in Windows' own order, machine before user.

**Reuse the registry access that already exists; do not write a second one.** `cli_install.rs` has the whole apparatus, added for #648 mission 4 after `runner status` reported the command installed following an uninstall because it had read the process PATH — the same class of bug as this one. `UserPathRegistry` is the trait (`:287`), `SystemUserPathRegistry` the real implementation (`:700`), the private `windows_registry` module the `windows_sys` calls (`:715`), and `FakeRegistry` the test double (`:854`). A machine-PATH read is the one thing missing; add it beside the user read rather than in a new module.

`resolve_login_shell_env` takes no arguments and is called from a background thread with no context, so do not thread a registry parameter through it and do not change the unix signature. Construct the system registry inside the `cfg` branch, and put the composition — registry values in, PATH string out — in a small pure function that takes the values as arguments. That function is the part a macOS machine can test.

### The traps

- **`REG_EXPAND_SZ` is not a literal string.** A PATH stored with that kind contains unexpanded `%SystemRoot%`-style references, and `find_executable` will simply not find anything under an unexpanded prefix. `RegistryPathValue` already carries `kind` (`cli_install.rs:282`); expand before the string reaches discovery, and cover an unexpanded value in the pure function's tests.
- **`DiscoveryOutcome::NoShell` becomes a lie.** Windows would now be reporting a successful environment read under a variant that means "there is no shell to probe". Decide between adding a variant and returning `Ok`, then check every reader of the outcome — the Settings surface copy and the LKG store in `db/app_state.rs` — so nothing starts claiming a shell probe happened that did not. Say what you chose and what reads it.
- **A Windows machine will write `login_shell_env_lkg` for the first time.** That store has only ever held Unix probes. Confirm a registry-derived env survives the round trip through `LoginShellEnvLkg` and that a stale entry cannot pin discovery to an old PATH.
- **`compose_path` merges shell PATH, home and process PATH.** The registry PATH will overlap heavily with the process PATH, since one was inherited from the other. Check the ordering and de-duplication so the searched string does not double in length on every refresh.
- **A registry read must never take down discovery.** It runs on a background thread at startup and on every Refresh. A missing key, a denied read or an unexpected value kind returns the previous empty-env behaviour with a logged warning, not an error that aborts the discovery pass and not a panic.
- **You cannot compile the code you are writing.** `#[cfg(not(unix))]` is not built on macOS, so a typo in the `windows_sys` calls will pass every local check and fail in CI. The Windows job on the PR is the first real compile. Expect that round trip and budget for it rather than being surprised; keep as much logic as possible in the platform-independent pure function where local tests do reach it.

Out of this mission: listening for `WM_SETTINGCHANGE` so Runner refreshes without the button — a reasonable follow-up, named in the issue, deliberately not built here so the diff stays reviewable. Also out: the macOS probe, the Settings → Agents layout, `cli_install`'s write path and its broadcast, and `docs/`.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff with one question in front: **is the untestable surface as small as it can be, and is everything above it actually covered?** Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree; do not touch any other worktree under `.worktrees/` or the checkout at the repository root. Do not launch or restart the Runner app — `target/debug/runner` is the GUI binary — and **do not read or write the real Windows registry or any real Runner data directory**; every test uses the existing fake.

**After the reviewer's clean verdict: commit, push `fix/672-windows-path-refresh`, and open a PR against `main` that closes #672.** Then drive CI green with `gh pr checks <pr> --watch`, both the macOS and the Windows job — the Windows job matters more than usual here, because it is the only thing that compiles the code you wrote. **Do not merge.** Jason does the quality check and the final merge himself.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`.

Be honest about what green means here: a macOS pass proves the composition and the unix path, and proves nothing about the registry read. The PR body must carry the Windows smoke steps for Jason — install an agent CLI while Runner is open, press Refresh, expect the row to flip to installed without a restart — and must state plainly which lines no test covers.

## Handoff

Final Runner handoff, posted on the feed: branch and base commit; every file and function changed; the `DiscoveryOutcome` decision and what reads it; how `REG_EXPAND_SZ` is expanded and where that is tested; what `compose_path` does with the overlap between the registry PATH and the process PATH; the exact lines with no test coverage and why they could not be covered; checks with results; the reviewer's explicit no-remaining-must-fix verdict; and the PR number.
