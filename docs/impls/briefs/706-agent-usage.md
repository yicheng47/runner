# 706 — Plan usage for Claude Code and Codex

Implement [P2 #706](https://github.com/yicheng47/runner/issues/706), milestone 0.11. Jason asked for a codex pair crew mission on 2026-09-23 that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-706-agent-usage`, on branch `feat/706-agent-usage`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `26b3b23`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- **`docs/features/706-agent-usage.md`**: the spec. It wins over the issue and over this brief on any detail, except the one deviation below. Its design, `design/specs/706-agent-usage.pen`, is Jason's reference; do not open or edit `.pen` files, because the spec describes everything you need.
- `crates/runner-app/src/surfaces/app_shell.rs`: `settings_button` and `update_hint` (around line 370), the Settings row the icon joins.
- `crates/runner-app/src/updater/windows.rs`: the `zed-reqwest` client the backend will reuse.
- `crates/runner-backend/src/runtime_status.rs`: the effective executable per runtime and discovery completion.
- `crates/runner-backend/src/shell_path.rs`: the captured login-shell environment, including the proxy variables.
- `crates/runner-backend/src/events.rs`: `emit`, and how the app listens to backend events such as `session/status`.
- `crates/runner-app/src/ui/`: the existing popover, tooltip and icon-button components to build on.
- `crates/runner-app/src/theme.rs`: the type scale and colour tokens (`warn`, `danger`, and the sidebar tokens).

## Deliverable

1. **Backend `usage` module** in `runner-backend`, as the spec's Data section describes:
   - the Codex probe: the effective `codex` with Orca's `app-server` arguments, JSON-RPC `initialize`, `initialized`, `account/rateLimits/read`, parsed by window duration;
   - the Claude fetch: read `claudeAiOauth.accessToken` from the Keychain item `Claude Code-credentials` on macOS (the `security-framework` crate is already in `Cargo.lock`; use it, not the `security` command, so the system prompt names Runner) and from `~/.claude/.credentials.json` elsewhere, then the usage request;
   - the unavailable reasons as typed values the app turns into the spec's lines;
   - the in-memory cache, the refresh rules (launch after discovery, every 15 minutes, the refresh button, opening a popover older than 5 minutes), and a `usage/updated` event;
   - the same `zed-reqwest` dependency the app uses, and the login-shell proxy environment on every request.
2. **Never write** Claude Code's credential store or Codex's, never refresh a token, never log a token, and never persist usage.
3. **App**: the gauge icon at the rightmost slot of the Settings row with the update icon to its left; its four states (grey, amber at 80% or more, red at 100%, highlighted while open); the hover tooltip; the popover (header with age and refresh, one section per available agent with a row per window, the unavailable line, "Agent settings…" opening Settings → Agents); dismiss on click outside and Esc; hidden when no agent is installed and enabled.
4. **Deviation from the spec: no version line.** The installed version needs the version probe #533 owns, so this mission ships the popover without it. Update the spec's version-line bullet in this branch to say the line arrives with 533, and say so in the handoff.
5. **Tests**, per the spec's phases:
   - recorded answers for both agents, including missing fields, an unknown window duration, a rejected token and a timeout;
   - a test that nothing is written to Claude Code's credential store;
   - the colour thresholds and the reset formatting ("3h 10m", "6d 20h", under a minute);
   - the refresh rules without sleeping (inject the clock).

   No test reaches the network or the real Keychain.
6. **Docs, same diff**: a paragraph in `docs/arch/arch.md` on the usage module and its data sources, and the spec correction from item 4. If implementation forces any other deviation, update the spec in this branch and say why in the handoff.

Out of scope: the version line and Update button (533), other agents, account switching, history, notifications, a `runner usage` command, `.pen` files, README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app, and do not start, stop, restart or type into Jason's Runner apps, chats or missions; Jason smoke-tests the UI. Do not change agent configuration, and do not run `claude` or `codex` against Jason's accounts to capture fixtures: write the recorded answers from the shapes in the spec. Native Windows is unavailable; say what is unverified there (the credentials file path, `codex app-server` through a batch wrapper).

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff. Then it reviews the whole working-tree diff against the spec and this brief, must-fix findings first with file:line pointers. It checks in particular:

- that no path writes, refreshes, logs or persists a credential or a token;
- that a failed or unavailable fetch never blanks the last good numbers and never raises an error dialog;
- that the gauge keeps its slot whether or not the update icon shows;
- that the network and the Codex process never run on the UI thread.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `usage`, `ui` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/706-agent-usage`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #706`, a summary, the key changes, test evidence, the version-line deviation, unverified platforms, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
