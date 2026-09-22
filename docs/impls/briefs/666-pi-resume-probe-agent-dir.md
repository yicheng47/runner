# 666 — pi resume probe honours the role's agent directory

Implement [#666](https://github.com/yicheng47/runner/issues/666). Jason asked for a codex-crew mission that ends in an open PR, not a merge; he reviews it the next day. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-666-pi-resume-probe-agent-dir`, branch `feat/666-pi-resume-probe-agent-dir`, whose tip is this brief on top of main `5c5de00`. Build on it; do not rebase, squash, create another branch or checkout, touch the root checkout, or share another worktree's target directory. Another crew is working in `.worktrees/fix-688-title-status-fallback` at the same time; treat it as another machine.

Read first: AGENTS.md; issue #666 (`gh issue view 666`), which is the spec; `docs/features/archive/539-pi-runtime.md` (non-goals line 118, decision 2 at line 124); `docs/impls/archive/539-pi-runtime/README.md`; `docs/arch/arch.md` lines 398–400.

## Current behaviour

`router/runtime.rs::pi_conversation_exists` globs `<home>/.pi/agent/sessions/<pi_project_slug(cwd)>/*_<key>.jsonl` under the real home. The spawn-side caller in `session/manager/spawn.rs` (the `conversation_missing` match, `Runtime::Pi` arm) passes no env, while the Copilot arm beside it passes `role.env["COPILOT_HOME"]` into `copilot_conversation_exists_with_home`, and `session/copilot_trust.rs::copilot_home` falls back to Runner's own process env before the default. A role that relocates pi therefore always looks missing and takes the degraded resume path.

## Deliverable

1. **Resolve pi's sessions root the way pi does.** Verify it from the installed pi's own code before writing any path logic: `readlink -f "$(which pi)"` leads to its package (`dist/config.js` and the session manager). Establish and cite in the handoff: the precedence of `PI_CODING_AGENT_SESSION_DIR` over `PI_CODING_AGENT_DIR/sessions` over `~/.pi/agent/sessions`; whether a session-dir override still nests the per-cwd slug directory or holds files flat; whether pi expands a leading `~` or resolves relative paths; and the exact file naming. The issue's precedence is a hypothesis; the installed code wins. Name the pi version you read.
2. **Where the variables come from.** Mirror what the pi child actually sees: the role's env first, then Runner's own process env, then the default, the same shape as `copilot_home`. Confirm in the spawn code that the role env overrides the inherited env for the child; if it does not, match what it does instead.
3. **Plumbing.** `pi_conversation_exists` takes the role env (or the resolved overrides); the spawn arm passes `role.env`, mirroring the Copilot arm. Keep the `#[cfg(test)]` `CONVERSATION_HOME` seam working, and keep a role without either variable byte-for-byte on today's path. The other runtimes' probes do not change.
4. **Tests.** Unit tests in `router/runtime.rs` over a temp directory, each failing before the change where meaningful: `PI_CODING_AGENT_DIR` finds a session under it; `PI_CODING_AGENT_SESSION_DIR` wins over it with the layout you verified; an unset env behaves as today; a set variable pointing at an empty directory reports missing rather than falling back to `~/.pi`; and a manager-level resume through `SessionManager::resume` with a pi role whose env relocates pi resolves `resuming` true (use the existing fake-runtime harness in `session/manager/tests.rs`).
5. **`crates/runner-app/tests/pi_runtime_smoke.rs`** asserts the non-degraded resume leg (`resuming` true or its observable effect) now that the probe honours its temporary `PI_CODING_AGENT_DIR`. It stays `#[ignore]` and gated on `RUNNER_PI_SMOKE_BINARY`.
6. **Docs, same diff.** `docs/arch/arch.md` line 400 describes the new resolution; the 539 spec's non-goal line (118) and decision 2's "Documented, not handled" sentence point to #666 for the env variables and leave `sessionDir` and `--session-dir` as the remaining non-goals; the 539 record's line 18 notes the fix.

Out of scope, per the issue: `sessionDir` in pi's `settings.json`, a role's own `--session-dir` argument, and any other runtime.

## Validation

Run `cargo test --locked -p runner-backend --profile ci --no-fail-fast`, `cargo test --locked -p runner-app --test pi_runtime_smoke --profile ci` (compiles the ignored test), `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`; report each exit code. Keep path handling portable: pi on Windows uses `%USERPROFILE%`, and paths and slugs must not assume `/`. If any code ends up behind `#[cfg(windows)]`, macOS clippy will not see an import only that code uses. Native Windows is unavailable to this crew; say what is unverified there.

The real-binary smoke, once, on the same terms as #648 mission 5: only if `~/.pi/agent/auth.json` exists, run `RUNNER_PI_SMOKE_BINARY="$(which pi)" RUNNER_PI_SMOKE_AUTH=~/.pi/agent/auth.json cargo test --locked -p runner-app --test pi_runtime_smoke --profile ci -- --ignored --nocapture` with the test's default model (leave `RUNNER_PI_SMOKE_MODEL` unset; it already runs thinking off), and record the duration and model in the handoff. The test copies the auth file into its temporary directory and never writes to the real `~/.pi`. If the credential is absent or the run fails for a reason outside this change, report it and do not retry more than once.

Do not start, stop, restart or type into Jason's Runner apps, direct chats or mission sessions, and do not run the dev app; Jason smoke-tests. Do not change agent configuration.

## Crew handoff and authorization

The lead slot owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against this brief and the issue, must-fix findings first with file:line pointers, and verifies item 1's claims against the installed pi code itself. Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes: commit the work on this branch in focused commits (imperative subject, scope `session` or `docs`, no co-author trailers), `git push -u origin feat/666-pi-resume-probe-agent-dir`, and `gh pr create --base main` with `Closes #666`, a summary, the verified pi layout with its source, test evidence, the smoke result and unverified platforms; no agent session links in the body. Then `gh pr checks <n> --watch` and poll until nothing is pending; if CI fails, fix it on the branch, have the reviewer check the fix, and push again. **Do not merge**, do not delete the branch or worktree, do not cut a nightly or release.

Final handoff to everyone through Runner: PR URL and CI result, the verified resolution order and layout, changed files, checks with exit codes, the smoke result, unverified platforms, and the reviewer's verdict. Then both slots stand by.
