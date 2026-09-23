# 644 — Antigravity CLI runtime

Implement [P1 #644](https://github.com/yicheng47/runner/issues/644), the last 0.11 feature. Jason asked for a claude pair crew mission on 2026-09-23 that ends in an open PR, not a merge; he reviews it the next day.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-644-antigravity-runtime`, on branch `feat/644-antigravity-runtime`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `d7f060b`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- **`docs/features/644-antigravity-runtime.md`**: the spec, probed against agy 1.2.8/1.2.9. It wins over the issue and over this brief on any detail, except the deviations below. "What a new runtime touches" is your site list; "Decisions" and "Verification" are binding.
- `docs/features/archive/540-copilot-cli-runtime.md` and the Copilot code: the closest runtime (first turn on `-i`, `--add-dir`, trust preseed). Follow its shapes.
- `docs/impls/archive/539-pi-runtime/plan.md`: how the last runtime was split and what it deferred.
- `crates/runner-backend/src/session/copilot_trust.rs`, `session/codex_capture.rs`, `session/pi_status.rs` and `session/hook_feed.rs`: the patterns for `agy_trust.rs`, `agy_capture.rs` and `agy_status.rs`.
- `crates/runner-backend/src/router/runtime.rs` `RuntimeDefinition`: since #533 it carries `update_args` and `npm_package`.

## Deliverable

Spec phases 1, 2 and 3 in one branch:

1. **Phase 1, the adapter**: every spawn-path site in the spec's table. That covers the enum, the definition, model and effort from the static catalog (decision 3: only accepted pairs, never `--effort` without `--model`), the first turn on `-i`, permission flags and the four-spelling strip, the `--log-file` key capture, resume by `--conversation`, the conversation probe, the trust preseed, `--add-dir <mission dir>` for slots, the catalog entry, and every enumerating test.
2. **Phase 2, settings and identity**: the MCP client (`~/.gemini/config/mcp_config.json`, 0-byte file reads as `{}`), default registration, the Agents row, the Skills pane's two roots, permission copy, `README.md` and `README.zh-CN.md` together, and `docs/arch/arch.md` plus `docs/arch/windows.md`. For #533: `agy --version` feeds the row's version; agy is not on npm and updates itself, so give it no `npm_package` and no Update button.
3. **Phase 3, hook status on macOS**: the Runner-owned `<app data>/antigravity-hooks` folder loaded with `--add-dir`, holding `.agents/hooks.json` and the reporter; `agy_status.rs`; `HookStatusWatcher::Antigravity`; env injection; and decision 5's mapping. It registers **no `PreToolUse`**, answers `{}` to every event, and writes nothing into `~/.gemini`. Response failed needs a live failure's `Stop` shape; if the spec has none, map what it documents and list the gap.
4. **Tests** for every Verification item that does not need a live agy.
5. **A smoke checklist** at `docs/tests/644-antigravity-smoke.md`, covering spec phase 4 and the hook checks. The spec sends phase 3's checklist to `docs/impls/347-hook-status/`, which is now archived; correct that line to point at the new file.

## Deviations from the spec

- **The mark is not designed yet** (phase 0). Give `chat_icon.rs` and `assets.rs` an Antigravity arm with a neutral placeholder glyph already in the asset set, and keep the icon tests exhaustive. Do not draw or source a logo, and do not open or edit `.pen` files. Jason designs the mark next, and it replaces the placeholder in a follow-up commit.
- **No live agy sessions.** Do not start `agy` interactively, with `-p`, or against any account: a session uses Jason's Google sign-in, writes under `~/.gemini`, and agy updated itself during the probe. `agy --help` and `agy --version` are fine. Take every payload shape from the spec. The fixture `crates/runner-terminal/fixtures/agy-first-turn.ndjson` and the wheel check need a live session, so they stay with Jason's smoke and are listed in the checklist.
- **Windows**: `default_enabled` stays off there, per the spec, until its smoke.

Out of scope: everything in the spec's Out of scope and Phase 5, the mark itself, the fixture recording, `.pen` files, and README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-terminal --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions: Jason smoke-tests the UI. Do not edit `~/.gemini`, `~/.claude`, `~/.codex` or any agent configuration outside tests' temp dirs. Native Windows is unavailable, so say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the spec and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that every `(runtime, mode)` arm and every enumerating test covers `Antigravity`, and that the other runtimes' argv are unchanged;
- that no path emits `--effort` without `--model` or a model/effort pair outside the catalog;
- that the trust preseed keeps every other key in agy's settings file byte-for-byte;
- that the hooks folder registers no `PreToolUse` and nothing writes into `~/.gemini` outside the trust and MCP files the spec names.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `runtime`, `session`, `ui`, `mcp` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/644-antigravity-runtime`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #644`, a summary, the key changes, test evidence, the placeholder mark, what waits for Jason's smoke (fixture, wheel, live hooks, Orca's `PreToolUse` question, Windows), and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, any spec correction, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
