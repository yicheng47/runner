# 687 — Codex stays Working before its first hook

Fix [P1 #687](https://github.com/yicheng47/runner/issues/687). Jason requested the existing codex-crew to implement and review this bug fix. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-687-codex-pre-hook-idle`, branch `fix/687-codex-pre-hook-idle`, based on main `7a96277580162599a86b20f854f0501c21718c0d`. The branch and worktree already exist; do not create another checkout, change the root checkout, or share another worktree's target directory. Read AGENTS.md and issue #687 before implementation.

## Evidence and scope

An untouched Codex CLI prompt can remain Working forever in Runner, both after resuming a direct chat without a captured native key and after resuming a mission slot with a valid key. The missing key is not required. The issue records the screenshots, exact live session identifiers, and evidence collected before this mission.

The affected dev session's backend observation was Working from `baseline`; its hook feed was empty, despite injected hooks, reporter, path, and generation being present. Its resumed rollout contained no newly started turn. The raw PTY fallback treats every nonempty read as activity and resets its two-second silence timer. Repeated idle TUI redraw is the supported explanation, but the prior investigation did not capture the bytes; verify the mechanism with a fixture instead of claiming it was already proven.

Existing #659/#670 behavior correctly publishes Idle when SessionStart is received. Changing that reducer again does not fix an empty feed. A locally inspected upstream Codex source snapshot queued SessionStart until turn execution; verify the installed CLI behavior and distinguish source-snapshot evidence from the actual binary version. The user's screenshot and logs showed Codex 0.155.1.

## Deliverable

Make an untouched Codex prompt become and remain Idle after startup/resume, including continuous idle redraw before any hook arrives. Keep real submitted work Working. Prefer an existing reliable readiness/input signal or a narrow Codex fallback adjustment; choose the smallest fix supported by evidence. Do not treat a terminal substring, arbitrary elapsed time, missing native key, or silence alone as proof that a real turn has completed.

- Cover fresh direct launch, unkeyed direct resume, and keyed mission resume. The fix must not depend on submitting a dummy prompt or capturing a native key first.
- Preserve genuine first-turn work, including automatic initial mission prompts, later user submissions, and router-delivered work. A pending automatic prompt must not be lost or prevented from reaching the agent.
- Preserve hook ownership: once a healthy hook stream says Working, neither quiet tools nor idle-looking terminal output may mark it Idle. Preserve completion, interruption, unresolved human-input holds, typed-draft protection, stale generation/session/turn isolation, and child-event isolation.
- Keep missing/failed-hook behavior honest. State what remains baseline-derived and how the implementation transitions into and out of authoritative hook state. Do not claim a hook is armed unless actual hook evidence supports it.
- Keep the status vocabulary and UI unchanged. No agent-settings work, broad status redesign, or unrelated runtime refactor. Shared changes require regression coverage for the other runtimes.

Investigate these existing paths first: `crates/runner-backend/src/session/pty_runtime.rs` (IdleDetector, reader and monitor threads, hook watchers); `session/manager/{mod,output,spawn}.rs` (baseline arbitration, input submission, launch arguments); `session/{codex_status,hook_feed}.rs`; and `router/runtime.rs::codex_status_args`. Existing input/readiness parsing in runner-terminal and its consumer may help; inspect before introducing another parser. Follow actual callers and event ordering rather than relying on historical brief assumptions.

## Reproduction and validation

Capture a sanitized before/after reproduction of the pre-first-hook case using an isolated temporary fixture or a disposable test session. Do not type into, stop, restart, or alter Jason's existing direct chats, mission sessions, Runner apps, databases, native conversations, hooks, or agent configuration. Temporary fixture configuration is allowed; normal sessions must retain their native homes and settings. Do not create another source checkout. Avoid paid model probes; use a local canned endpoint or existing deterministic harness for submissions if needed.

Add meaningful regression tests for continuing idle redraw with an empty hook feed, true input submission/automatic mission startup, hook takeover, and completion/interruption. Tests must exercise the startup/input-to-manager integration that was missing from synthetic SessionStart-only tests. Demonstrate that the new regression fails before the fix. Reuse fixtures and test utilities where possible; do not add an elaborate test framework.

Run relevant backend tests, workspace Clippy with the CI profile, formatting and diff checks. If terminal or app consumers change, run their tests too. Use this worktree's own target directory; do not run concurrent Cargo jobs competing for it. Prefer `cargo test --locked -p runner-backend --profile ci --no-fail-fast`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and `cargo fmt --all --check`. Preserve Windows behavior; exercise portable logic and Windows-specific paths where affected, and identify any native Windows validation still needed. Do not repeat checks after they pass unless a later change warrants it.

## Crew handoff and authorization

The coder owns reproduction, implementation, tests and corrections. The reviewer waits for an explicit Runner handoff, then reviews the complete working-tree diff, including the proposed state ownership and test evidence. Iterate through Runner until the reviewer reports no remaining must-fix issues. Do not launch extra agents or crews. The coordinator follows the mission feed and handles questions already answered by the task.

Only the coordinator's preparation commit for this brief is authorized by the standing mission recipe. Leave implementation changes uncommitted. Do not push, open a PR, merge, delete branches/worktrees, or launch/restart the user's Runner apps without a new instruction. The prior publishing authorization concerned #684 and does not apply here.

Final handoff through Runner: exact root cause with measured evidence; fix and its status-ownership boundary; changed files; regression proving the old failure and new behavior; checks and limitations; minimal live smoke steps for Jason; and the reviewer's explicit no-remaining-must-fix verdict. Completion means the fix and review are done, not merely that a mission was started.
