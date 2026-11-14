# #1288 Antigravity bypass and symlink trust — macOS live QA

## Candidate and scope

Tested 2026-11-14 00:35–00:42 CST on candidate `8edbe0d272671a8f9ff4089574264b444237e9db`, branch `feat/1288-agy-bypass-and-trust`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1288-agy-bypass-and-trust`. The reviewer handed off this exact SHA with no remaining must-fix issues. The source tree was clean before building and remained unchanged during QA; this record is the only QA-owned repository edit.

Selected checks follow the brief and clean review: the new Launch switch, Chat/Mission command previews, persistence across app restart, fresh Bypass-off startup through `/tmp`, two-spelling trust seeding, and fresh Bypass-on argv plus a bounded prompt. These exercise the changed user control and actual Antigravity launch behavior. This is feature QA, not a full smoke or regression run. Native Windows and Linux were unavailable and were not part of the selected macOS matrix; no native pass is claimed for them. Resume and mission process launches were not separately exercised live; the brief selects direct-chat launches and the Mission command preview. Antigravity native fork is unsupported.

Environment: macOS 26.6.2 (25G83), Apple Silicon, Runner 0.19.8 (dev), Antigravity CLI 1.5.8. Native control used `@oai/sky` through node_repl. Original screenshots, state backups, process argv and status evidence are retained under `/private/tmp/runner-1288-qa`.

## Build and isolation

Acquired `script/qa-lock acquire --timeout 540` before building. Preflight found a pre-existing development daemon and running pi session `01M6TC8FEWDFD2FMXT5DG7KZ4V`, with no development app open. Paused the first `make run` before launch and reported the conflict. The operator explicitly directed normal development launch, preserving that session without input, stop or archive, and leaving the daemon running at cleanup. No installed Runner sessions were operated.

Built successfully with `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`, using this worktree's own `target/`. An initial build-only `cargo build -p runner-cli -p runner-app` also passed. Automated tests and Clippy were not rerun by QA; those gates belong to the coder's reviewed handoff.

Stopped only this run's bare development app, then used `script/screenshot-app bundle /private/tmp/runner-1288-qa` and `script/screenshot-app launch /private/tmp/runner-1288-qa`. The wrapper `/private/tmp/runner-1288-qa/Runner Screenshot Dev.app`, bundle ID `com.wycstudios.runner.screenshot-dev`, contains byte-identical app and CLI copies. App SHA-256: `2b64c989ce712da1f87d50e40b7d545c9acb4aae7ed6978c0fe91ffd18ac3fbd`; CLI SHA-256: `dda2d1fe00d0c5f309afb770ccdefbe18dc098b11d6d7d28af553418ce90817f`. The copied candidate remained frozen throughout QA. The coder was notified when its build finished.

All development CLI commands used the explicit `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner`, empty `RUNNER_SESSION_CLI`, and cleared implementation mission identity variables. `status --json` confirmed outside-mission identity and `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`. The original development UI settings and Antigravity settings were backed up before modification. Antigravity was initially disabled in Runner and its Bypass option was off; both were restored after testing. No global authentication or model setting was changed.

## Results

Evidence paths in the table are relative to `/private/tmp/runner-1288-qa`.

| Case | Result | Observation and evidence |
| --- | --- | --- |
| Settings Launch row | Passed | Settings → Agents → Antigravity CLI shows **Bypass permissions**, the Chats scope, explanatory copy and `On: --dangerously-skip-permissions` without clipping or overlap. `agy-settings-baseline.png`, `chat-bypass-on.jpg`. |
| Chat command preview | Passed | Enabling Bypass adds `--dangerously-skip-permissions` with the Bypass permissions annotation; disabling it removes the flag. `chat-bypass-on.jpg`, `chat-bypass-off.jpg`. |
| Mission command preview | Passed | The preview includes `--dangerously-skip-permissions`, annotated “always in missions,” while the chat toggle is off and on. Other displayed mission arguments remain unchanged. `mission-bypass-off.jpg`, `mission-bypass-on-enabled.jpg`. |
| Toggle persistence | Passed | Enabled Bypass, quit only the QA wrapper, relaunched the same frozen copy, reopened Settings → Agents → Antigravity CLI, and observed the enabled switch and flagged Chat preview. `bypass-persisted-after-restart.jpg`. Persistence across a daemon restart was not selected. |
| Fresh `/tmp` chat, Bypass off | Passed | Chat `01M7AWARB41FADCR94100B1MEG` opened in `/tmp/runner-1288-qa/chat-off` directly at the normal Antigravity composer, without a project-trust dialog. Process argv contains no bypass flag. `chat-off-native.jpg`, `chat-off-process.txt`. No account-backed prompt was sent in this chat. |
| Symlink trust entries and settings preservation | Passed | After the off launch, settings contain both `/tmp/runner-1288-qa/chat-off` and `/private/tmp/runner-1288-qa/chat-off`; the on launch adds the corresponding two `chat-on` entries. Comparing the raw prefix/suffix outside the `trustedWorkspaces` JSON value showed every unrelated byte preserved. `trust-off-settings.json`, `trust-on-settings.json`, `trust-validation.txt`, with native startup evidence in `chat-off-native.jpg` and `chat-on-native.jpg`. |
| Fresh `/tmp` chat, Bypass on | Passed | Chat `01M7AWCWZ1B2PYD1QYEPGWSYDT` opened in `/tmp/runner-1288-qa/chat-on`; its actual process argv includes `--dangerously-skip-permissions --model gemini-3.8-flash-low`. Native UI shows Gemini 3.8 Flash (Low). Typed `Reply exactly QA1288_OK. Do not use tools.` through native input; the agent returned `QA1288_OK` and became idle without a permission or trust dialog. `chat-on-process.txt`, `chat-on-native.jpg`, `tiny-prompt-typed.jpg`, `tiny-prompt-result.jpg`, `chat-on-final-status.json`. |

No failed selected check. **NO REMAINING MUST-FIX QA ISSUES** for the selected macOS matrix. Native Windows/Linux and additional lifecycle scenarios have no live result in this run. A future PR must disclose those limits and retain `needs-live-test` if any additional required platform check remains pending.

## Cleanup and handoff

Archived both owned chats through development CLI `session archive <id> --json`, then verified each is stopped with a non-null `archived_at`. No roles, crews, missions or projects were created. Removed the two empty scratch chat directories. Removed exactly the four QA trust entries, restoring the original trust-array bytes; the complete Antigravity settings file then matched its baseline byte-for-byte. Restored Bypass off and Antigravity disabled through native Settings; JSON comparison of all development UI setting keys against the backup showed no differences before and after quit. Evidence: `settings-restored.jpg`, `cleanup-trust.txt`, `ui-settings-restored.json`, and the two `<session-id>-archived.json` files.

Quit natively with **Keep them running** selected (`quit-dialog.jpg`), verified no QA wrapper process remained, unregistered the wrapper with `lsregister -u`, and released `script/qa-lock release`; `script/qa-lock status` reports free. Preserved the development daemon per operator direction. The final unarchived session list matches the preflight list, including the pre-existing pi session still running. No pre-existing chat received input or a stop/archive action. Wrapper, screenshots and evidence backups remain available for review.

No PR existed during this QA run. The coder receives this record and the passed candidate SHA to include the selected checks, evidence and platform limits in the PR description. QA made no commits, pushes, implementation edits or PR changes.
