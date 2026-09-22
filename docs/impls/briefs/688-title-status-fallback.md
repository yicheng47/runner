# 688 — Runtime-aware title hints for the status fallback

Fix [P1 #688](https://github.com/yicheng47/runner/issues/688). Jason asked for a codex-crew mission that ends in an open PR, not a merge; he reviews it the next day. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-688-title-status-fallback`, branch `fix/688-title-status-fallback`, whose tip is this brief on top of main `5c5de00`. Build on it; do not rebase, squash, create another branch or checkout, touch the root checkout, or share another worktree's target directory.

Read first: AGENTS.md; issue #688 (`gh issue view 688`), which is the spec and wins on any detail; `docs/features/archive/584-title-status-detection.md` (the old detector, its evidence and limits); `docs/impls/briefs/625-m1-title-heuristic-removal.md` (what was removed and where it was wired); `docs/impls/briefs/687-codex-pre-hook-idle.md` and `docs/tests/687-codex-pre-hook-idle.md` (the startup fix this builds on); `docs/arch/arch.md` §5.10.

## Why

Byte activity is the baseline and hooks sit on top. #625 removed title classification on the assumption that an installed hook adapter means hooks report. #687 showed that assumption fails before the first hook, when hooks are disabled or unavailable, and after a bridge failure. In those windows an idle agent repainting its terminal reads Working. The retained `crates/runner-terminal/fixtures/codex-title-working.ndjson` records exactly that: the title shows rest at 7.428 s while 46 output events follow over 3.465 s with no gap over 153 ms.

## Deliverable

A title-aware layer inside the fallback, never above hooks:

1. **Hook authority stays.** Once a healthy hook stream owns the session, title hints are ignored. Quiet output or a lack of recent hook events alone must not demote a hook-owned turn into fallback. Title hints apply before the first accepted hook, when hooks are unavailable/disabled, and after an explicit `StatusBridgeFailed`.
2. **Within fallback, a recognized runtime title signal beats raw bytes;** with no supported signal, byte activity runs exactly as today. Everything title-derived stays estimated/baseline in the status source and UI vocabulary.
3. **Recognize only what source inspection plus a recorded fixture backs.** Per runtime, cite the upstream source location that produces each title shape you accept, and pin it with a fixture (reuse the existing `runner-terminal/fixtures/*.ndjson` recordings; a new fixture must be a sanitized recording, not hand-typed bytes). Arbitrary title text, a missing spinner, a blank or reset title, and words inside a topic or path must never establish Idle or completion. Do not restore the old arm-on-braille-then-any-nonempty-title-is-Idle classifier.
4. **Submissions invalidate.** A new local, routed, pasted or automatic submission (including the pending first turn) clears any earlier title-derived Idle hint. A stale title must not mask pending work. Restart and resume start with no title authority from the previous process.
5. **Unknown formats degrade to the ordinary fallback,** never a latched status. Do not read or rewrite users' title settings.
6. **No side effects beyond activity.** Title inference emits no Ready/completed outcome, sets or clears no human-interaction hold, and does not touch draft protection or message delivery. Direct chats and mission sessions reach the same decision through the same path.
7. **Keep #687's startup handling** (`CodexStartup` in `IdleDetector`) as the separate mechanism for an untouched launch before any spinner; integrate with it rather than replacing it.

Design questions to settle from the code, and to state in the handoff:

- **Where titles are observed.** Today the only OSC title parser is alacritty inside runner-terminal's event thread (`crates/runner-terminal/src/terminal.rs`, the `Event::Title | Event::ResetTitle` arm, which `continue`s on a display-rejected title before anything else sees it). runner-backend cannot depend on runner-terminal. Verify whether every session, including a mission slot never opened in a pane, has a live terminal model feeding that thread. If not, the observation must come from the backend's own output path so direct and mission sessions share it. Either way, activity classification lives apart from `session/title.rs::provider_title`, which stays display-only.
- **Ownership arbitration.** Where the title hint sits relative to `IdleDetector`, the manager's status sources (`forwarder`, `input-submit`, `hook`, `wake`, …) and `accept_hook` / `hooks_unavailable`. Prefer extending the detector the byte path already uses over a new source string with its own gating, unless the code shows otherwise.

Out of scope: generic shell command detection (#586), hook adapters themselves, the idle threshold and resize grace, title display and persistence, status UI copy.

## Validation

Tests that must exist, each failing or impossible before the change where that is meaningful:

- Fixture replay with no accepted hooks, and again with a simulated bridge failure: idle redraw after a recognized rest title reads Idle; known work reads Busy.
- Healthy hook-owned quiet work stays Working regardless of titles; hook takeover drops title authority.
- First launch and resume: no inherited title authority; #687's startup tests still pass unchanged.
- A new submission after an idle title returns to Busy and stays there until fresh evidence.
- Missing, reset, blank and custom titles; unrelated shell titles; topic text containing spinner-like glyphs or status words.
- Each supported runtime format, positive and negative.
- One test proving direct and mission sessions take the same decision.

Run `cargo test --locked -p runner-backend -p runner-terminal -p runner-app --profile ci --no-fail-fast`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`; report each exit code. Keep platform-neutral logic portable; if any path is cfg-gated, remember macOS clippy prunes imports used only under `#[cfg(windows)]`. Native Windows is not available to this crew: list what remains unverified there.

Docs in the same diff: `docs/arch/arch.md` §5.10 describes the title-aware fallback and its boundary with hooks; add `docs/tests/688-title-status-fallback.md` with Jason's minimal live smoke steps (macOS, and the Windows legs he runs himself).

Do not start, stop, restart or type into Jason's Runner apps, direct chats or mission sessions, and do not run the dev app; Jason smoke-tests. Do not change agent configuration or spend model quota on live probes.

## Crew handoff and authorization

The impl slot owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against this brief and the issue, must-fix findings first with file:line pointers. Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes: commit the work on this branch in focused commits (imperative subject, scope `session` or `docs`, no co-author trailers), `git push -u origin fix/688-title-status-fallback`, and `gh pr create --base main` with `Closes #688`, a summary, the root cause, the status-ownership boundary, test evidence and the unverified platforms; no agent session links in the body. Then `gh pr checks <n> --watch` and poll until nothing is pending; if CI fails, fix it on the branch, have the reviewer check the fix, and push again. **Do not merge**, do not delete the branch or worktree, do not cut a nightly or release.

Final handoff to everyone through Runner: PR URL and CI result, the title formats supported per runtime with their source citations, the ownership boundary, changed files, checks with exit codes, unverified platforms, and the reviewer's verdict. Then both slots stand by.
