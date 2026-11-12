# #1257 — Restore Codex colors after archive

Jason approved the codex trio on 2026-11-12 to fix https://github.com/yicheng47/runner/issues/1257. Work in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1257-archive-terminal-state` on `fix/1257-archive-terminal-state`, cut from `origin/main` at `c2fbafd9b41979d5ff18e9f752672584baa4a955`.

## Problem and confirmed cause

After Runner archives a Codex chat, restoring and resuming it leaves the composer without its shaded background and gives submitted prompts ANSI bright-black backgrounds. Runner archives its own row; the native Codex conversation remains intact. The decisive path is archive → restore → resume, not ordinary stop/resume. The issue was updated with the investigation and supersedes its earlier timing hypothesis.

`ops::session::session_archive_direct` stops a running chat, then `session_archive` calls `SessionManager::forget_session_state`. That helper resets `state.output_seq` to zero but retains the terminal model and input/subscriber state. `SessionState::is_empty` requires the terminal to be absent, so pruning does not remove that state. `install_handle` later reuses the retained terminal when no handle exists. Its `TerminalModel::feed_output` guard rejects every chunk whose sequence is no newer than `sequence.last`, including the resumed process's startup OSC 10/11 queries. Codex's 250 ms probe gets no color replies and caches the fallback result.

An isolated harness using the real session manager/database/archive/unarchive/resume/forwarder/parser and a fake process reproduced this. Fresh launch and ordinary stop/resume each received 2 color replies. Archive reset daemon sequence 22 → 0 while retaining the identical terminal Arc; archive/restore/resume received 0 replies at sequence 1. Replies returned at sequence 23. The completed harness exited 0. It used a Trae adapter with a fake runtime and existing local debug libraries; it was not a live Codex/UI pass. Optional local reference: `/tmp/runner-1257-archive-o6s8maez/probe.rs` and `result.txt`; write durable regression coverage rather than depending on these temporary artifacts.

Observed macOS occurrence: Codex 0.167.2, Runner build `1f5f159` / bundle `20261111.1230`. The same native conversation was resumed after archive; no upstream Codex archive action is implicated.

## Implementation scope

Make archive cleanup and terminal stream lifetime consistent: release the daemon-owned terminal and its associated input/subscriber resources when forgetting/resetting the stream, so the restored process starts with a clean model. Inspect all callers of `forget_session_state`, stop/natural-exit behavior, and the distinction between a deliberately prepared terminal and one left by a terminated process before choosing the smallest correct fix. Preserve conversation IDs, saved launch definitions, window ownership and valid live attachments. Ensure cleanup does not leak workers, deadlock their joins, or send old-generation replies into a new process.

Do not remove the sequence duplicate guard, mask the failure with colors, extend Codex's timeout, or make an unrelated runtime/protocol/storage refactor. There is no new screen or layout to design; restore the existing behavior. Keep any additional cleanup strictly necessary for this lifecycle contract and explain it in review.

Primary code: `crates/runner-daemon/src/ops/session.rs`; `session/manager/{output,attach,lifecycle,bookkeeping,mod,terminal}.rs`; `crates/runner-terminal/src/terminal/model.rs`. The app's mirror removal on archive does not clear the daemon model.

Update the archive/restore explanation in `docs/guide/concepts/sessions.md` and its `docs/guide/zh-CN/concepts/sessions.md` counterpart to describe preserved agent conversation history and a fresh terminal on resume. Keep this concise and user-facing. QA owns `docs/tests/1257-archive-terminal-state.md`; coder folds its record into the PR.

## Automated validation

Before testing, list changed behaviors and checks with reasons. Add a regression through the actual archive/unarchive/resume operations after nonzero output history, proving the first resumed output and OSC 10/11 queries are processed without waiting for the old counter to catch up. Include ordinary stop/resume as the passing control. Cover old model/mode and subscription/input-worker cleanup as the implementation requires, including any other affected forget callers. Use fake runtimes or fixtures for deterministic tests; a five-second eventual reply alone cannot establish startup correctness.

Run relevant `runner-daemon` tests and `runner-terminal` tests if its code changes, formatting, and workspace Clippy with warnings denied. Follow repository platform/feature gates and drive all configured CI green. No whole live regression run is required.

## Required live validation — qa, macOS development app

Read `AGENTS.md` Test Scope and the full-smoke procedure for candidate identity, isolation and cleanup, selecting only the checks below. Acquire the shared QA lock before building a wrapper or launching the app; wait if another mission owns it. Use existing sign-ins, an inexpensive available Codex model at low effort, tiny prompts, and a fresh `/tmp/1257-qa-*` directory. Record the exact candidate and Codex version. Only operate chats created for this test. Coder and reviewer do not launch the app.

1. **Archive/restore after history:** Create a bounded Codex chat, teach a unique marker, and exchange several tiny turns to build output history. Record Runner ID/native key and the normal composer/submitted-prompt colors. Archive the running chat through Runner; restore through Settings → Archived and resume without restarting the daemon. Verify the same Runner ID/native key, correct blind marker recall, visible restored conversation, normal composer background, correctly shaded newly submitted prompt, and usable input. Capture screenshots before and after. This exercises the exact failing lifecycle and its color-query consequence, using the relevant `TERM-RENDER-01` and `RT-LIFE-07` expectations.
2. **Repeat archive/restore:** Repeat on that owned chat after further output. Verify the first restored screen/input and colors again. This catches retained old generations and counters after a second cleanup.
3. **Ordinary stop/resume control:** Run Codex's `RT-LIFE-07` on the owned chat and verify identity, blind recall, colors and input. This protects the neighboring lifecycle while archive cleanup changes.

These are selected macOS checks, not a full regression or a Windows live pass. Shared lifecycle behavior is covered by automated tests/CI on the configured platforms; report any diff-driven need for additional live checks explicitly. No theme/settings change is required. Record each result and screenshot path in the QA record and PR description. Keep `needs-live-test` while any selected required check is Failed or Blocked.

## Authorization

Approved crew: codex trio (`coder`, `reviewer`, `qa`). The crew may commit, push this branch, and open one PR against `main`, referencing #1257, then drive CI green. The operator's initial brief commit is to be folded into the implementation history. Follow repository review, QA, risk-label and core-review requirements; this touches core session lifecycle and is expected to need `core-review` and `risk-medium`. The mission ends in an open PR; merge and release are not authorized.
