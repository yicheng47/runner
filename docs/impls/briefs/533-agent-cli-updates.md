# 533 — Update agent CLIs from Settings → Agents

Implement [P2 #533](https://github.com/yicheng47/runner/issues/533). Jason asked for a claude pair crew mission on 2026-09-23 that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-533-agent-cli-updates`, on branch `feat/533-agent-cli-updates`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `44d4780`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- **`docs/features/533-agent-cli-updates.md`**: the spec, rewritten 2026-09-23. It wins over the issue body and over this brief on any detail. The issue body still describes an older design (a pane, a badge, a version line in the usage popover); ignore those parts. The design is `design/specs/533-agent-cli-updates.pen`, and the spec names its frames. Do not open or edit `.pen` files: the spec describes everything you need.
- `crates/runner-backend/src/runtime_status.rs`: `status_list`, `apply_discovery_result`, the effective executable per runtime.
- `crates/runner-backend/src/router/runtime.rs`: `RuntimeDefinition` (add the update arguments here) and `trailing_runtime_args` (the #475 flags, which stay).
- `crates/runner-backend/src/session/manager/spawn.rs`: `base_spawn_spec`, the agent spawn environment the update reuses without the runner-specific layers.
- `crates/runner-backend/src/usage.rs`: `http_client`, the proxy-aware client to reuse for the npm check, and the cache and refresh shape to follow.
- `crates/runner-backend/src/ops/session.rs`: `session_activity_snapshot`, the source of the live per-runtime session count.
- `crates/runner-app/src/surfaces/settings/agents.rs`: the Installed cards, their captions and Refresh.
- `crates/runner-app/src/surfaces/update_dialog.rs`: the existing centered dialog and scrim to model the update modal on.
- `crates/runner-terminal/src/terminal.rs` and `crates/runner-app/src/terminal/element.rs`: `TerminalSession` and the terminal element the panes render, which the modal embeds.
- `crates/runner-app/src/surfaces/app_shell.rs`: `render_usage_popover`, whose Agent settings gear gets the dot.

## Deliverable

1. **Version probe**: `<effective command> --version`, off the UI thread with a five-second timeout, at discovery completion, on Refresh, and when an update process exits; the first semver token, stored as `installed_version`. Not-found rows never probe.
2. **Update available**: the npm `latest` dist-tag for `@anthropic-ai/claude-code`, `@openai/codex` and `@github/copilot` through `usage.rs`'s client and the login-shell proxy environment, cached per app run for six hours, fetched when the Agents pane opens, on Refresh, and when the usage popover opens. Every failure degrades silently to version-only.
3. **Rows**: the caption reads `2.1.266 · Detected: …`, or `0.153.4 → 0.155.0 · Detected: …` when an update is available. An **Update** button (plain label, no version, no badge) sits beside the enabled toggle only when an update is available. TRAE never gets one. pi is out of scope for this mission.
4. **Guard**: macOS keeps Update enabled and captions running sessions ("3 running Codex sessions keep 0.153.4 until they relaunch."). Windows disables it while any session of that runtime is alive ("Stop the 3 running Codex sessions first."), with the count live while Settings is open. Split by platform at compile time like the rest of the platform chrome.
5. **Update modal**: Update opens a centered modal over Settings with a real terminal inside, whose PTY child is the update command (the effective executable plus the runtime's update argument, `base_spawn_spec`'s environment without the runner-specific layers, cwd = home). It has four states: running, waiting for input, succeeded (Done) and failed (Close, with the exit code). While running it has no button, and Esc and a scrim click do nothing. It must never appear as a chat: not in the sidebar, a tab, the archive, relaunch, the CLI's session lists or the MCP. `TerminalSession` attaches through `AppCore` by session id, so an ephemeral, unlisted session is acceptable if that is the smaller path. Either way, correct the spec's "it is not a session" sentence in this branch to match what you built, and say which you chose in the handoff.
6. **Usage popover dot**: the Agent settings gear shows an accent dot while any enabled, installed agent has an update available. Its tooltip then reads "Agent settings · Update available". The sidebar Settings row gets no dot.
7. **Tests**: version parsing against real `claude`, `codex` and `copilot` `--version` shapes and garbage; the npm comparison, cache and degrade paths with a stubbed fetch; the update spawn spec's argv, env and cwd; the guard count across direct, mission and fork sessions; the row presentation mapping (caption, button visibility, disabled state); the popover dot's rule. No test reaches the network or runs a real CLI.
8. **Docs, same diff**: the line beside #475's decision in `docs/arch/arch.md` from the spec's Docs phase, plus any spec correction the implementation forces, explained in the handoff.

Out of scope: pi and OpenCode updates, installing a missing CLI, background polling, an MCP or CLI command for updating, `.pen` files, README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions: Jason smoke-tests the UI. Do not run `claude update`, `codex update`, `copilot update` or `npm install -g` on this machine, and do not change agent configuration; running `--version` on the installed CLIs to capture their output shape is fine. Native Windows is unavailable, so say what is unverified there: the guard, `.cmd` wrappers, the modal's PTY.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the spec and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that no network request, `--version` probe or PTY spawn runs on the UI thread;
- that the update session never shows up in the sidebar, tabs, archive, relaunch or session lists;
- that the #475 flags are still on every agent session's argv and env;
- that a failed npm check never shows an error and never hides the version.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch, each with an imperative subject and scope `runtime`, `ui`, `session` or `docs`, and no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/533-agent-cli-updates`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #533`, a summary, the key changes, test evidence, the session-or-standalone choice, unverified platforms, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, any spec correction, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
