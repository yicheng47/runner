# #1324 — Antigravity usage without Windows console windows — QA

## 2026-11-15 — macOS host, Windows checks blocked

**NO REMAINING MUST-FIX QA ISSUES within the accessible scope; required Windows acceptance remains unresolved.** No native live check ran and no live behavior is claimed as Passed. Both Windows checks below are required and Blocked, left to Jason. Keep `risk-high` and `needs-live-test` on the PR until the Windows matrix passes; this verdict permits the disclosed handoff and does not establish Windows acceptance.

### Candidate and scope

The coder handed off local, unpushed commit `0ea5a497e1e58e9d179c3cd2b98c3c49ef59bf30` on `fix/1324-agy-usage-hidden-console`, in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1324-agy-usage-hidden-console`. `git rev-parse HEAD` matched that SHA and `git status --short --branch` showed no dirty files before this record was added. Reviewer reported NO REMAINING MUST-FIX ISSUES on the same SHA in Runner message `01M7DHBYJXC4GG67PSB6NZ4SMS`; coder's QA handoff is `01M7DHCFC96NR3KX09MJ68GC13` in mission `01M7D7Z0W5W0K0AQKTTJM32GCF`.

Jason's `direct-quota-api` decision supersedes the original hidden-console/grandchild approach: Windows usage reads Antigravity's saved sign-in and calls Google's OAuth/quota endpoints without launching agy. The Windows version/auth-check subprocesses and CONIN$ test are therefore replaced by direct-API and zero-spawn coverage. macOS and Linux retain their existing subprocess probe, version gate, stdin/session isolation and parser semantics. This record follows that approved scope and the reviewer's final check selection, rather than the superseded original Windows subprocess expectations.

Environment: macOS 26.6.2 (25G83), arm64, Asia/Shanghai. No Windows runtime was available in this QA task. QA did not acquire the screen lock, build, launch or operate the development app, inspect credentials, invoke agy, change settings, or create test chats/missions. No native screenshot exists because every live case was either not required or unavailable; source inspection and CI are not substitutes for screenshots or native acceptance.

### Selected matrix

| Check | Reason | Result | Evidence and remaining work |
| --- | --- | --- | --- |
| MAC-1324-01 — signed-in manual refresh in Settings → Usage and footer | Conditional check for changed non-Windows behavior | Skipped | Reviewer confirmed Unix arguments, inherited environment, isolation, timeout/error behavior and original snake_case parser semantics are preserved. Jason explicitly requires no macOS run for a Windows-only behavior change. No UI pass or screenshot claimed. |
| MAC-1324-02 — isolated signed-out SET-USAGE-05 | Conditional check for changes risking the Unix zero-login gate | Skipped | Same unchanged Unix boundary and final review decision; existing automated coverage was reported by coder/reviewer. No fixture or real account was operated. No UI pass or screenshot claimed. |
| WIN-1324-01 — signed-in scheduled/manual usage, no process/window, matching values | Directly exercises the reported Windows flash and the replacement quota source | Blocked | macOS host cannot execute the Windows path or observe Windows Terminal delegation. Follow the reproduction below on Windows; screenshot/process evidence pending. |
| WIN-1324-02 — isolated signed-out SET-USAGE-05 | Exercises the Windows zero-login guarantee with Credential Manager and file fallback | Blocked | Requires an isolated Windows user/profile without saved Antigravity credentials. Follow the reproduction below; screenshot/process evidence pending. |

### Windows reproduction left to Jason

Use the reviewed candidate, or record the exact later reviewed SHA and any dirty diff. Build and launch the development app from its branch worktree with `.\make.cmd run`. Verify the development namespace `%APPDATA%\com.wycstudios.runner-dev`; preserve installed Runner and pre-existing chats/missions. Use the shared QA lock for a QA-role run. Record Windows version, terminal default, agy version and candidate identity. Preserve any settings changed and restore them after the check.

**WIN-1324-01:**

1. On Windows, set Windows Terminal as the default terminal for this test, preserving its previous setting. Use an already signed-in agy account with a configured stdio MCP server launched through a shim, such as Puppeteer via Nodist's node.exe. Include agy 1.5.8 when available and a current supported version, recording each result separately.
2. In a separate, deliberate terminal run, capture `agy -p /usage --output-format json` as the comparison reading. This command is the reference action and may start its own children; finish it before monitoring Runner. Do not include credentials in evidence. Record Gemini and Claude/GPT pools, five-hour and weekly percentages/reset times, and timestamps.
3. Start a process-creation trace with parent IDs and timestamps, plus screen recording so a brief window cannot be missed. Record the idle baseline after agent discovery finishes; distinguish unrelated discovery, existing chats and the deliberate reference command from usage-triggered work. Monitor agy, shim/node descendants, WindowsTerminal.exe, OpenConsole.exe and browser processes.
4. Keep Antigravity and its usage display enabled. Open Settings → Usage and trigger manual refresh. Inspect both the full usage display and sidebar footer. Expect refreshed values consistent with the reference reading, both pools and their reset windows, and no process creation or window attributable to the Antigravity usage request.
5. Let a scheduled refresh occur without clicking refresh, opening an interactive chat or starting a mission. Record the configured usage interval (default 15 minutes), timestamps and refreshed data age so the schedule is actually observed. Apply the same no-process/no-window and value-matching assertions. If account activity changed quota during the interval, take another deliberate reference reading outside the monitored refresh and explain the timing difference.
6. Retain screenshots of usage/footer values, screen recording and the parent/timestamp process trace for each refresh. Restore changed settings. A build, automated zero-spawn test, or absence of a visible window alone does not establish this check's complete pass.

**WIN-1324-02 (SET-USAGE-05, adapted to direct API):**

1. Use a fresh disposable Windows user/profile or equivalent isolated Windows environment whose Credential Manager has no generic `gemini:antigravity` entry. Point HOME, USERPROFILE and ANTIGRAVITY_GEMINI_DIR at the fixture's empty directories before launching its development daemon/app. A HOME-only override does not isolate Windows Credential Manager. Never delete a real credential or sign a real account out to create this condition.
2. Confirm the fixture has no `antigravity-oauth-token` or `jetski-standalone-oauth-token` in the configured Gemini directory or its `antigravity-cli` child. Record existence/metadata only, never credential contents. Use fresh development data without a successful cached Antigravity reading that could mask the signed-out outcome. Preserve any effective executable override and settings; restore them afterward.
3. With screen/process monitoring active, enable Antigravity usage, open Usage and Settings → Usage, trigger manual refresh, and restart the development app in the same isolated environment. If needed, supply an isolated executable fixture for runtime discovery, but never let it launch OAuth. Verify `Sign in` and the explanation that Runner will not open login automatically; verify no usage-triggered agy, node/shim, browser OAuth, OpenConsole or Windows Terminal process/window and no credential or token-file changes.
4. Retain screenshots for sign-in status on both surfaces and after restart, plus process trace/screen recording and before/after fixture metadata. The old CLI unsupported/unknown-version fixture assertion remains a Unix gate: Windows no longer invokes agy for version/auth/usage and must not be reported as exercising that removed gate.
5. Stop and archive only owned QA objects, restore settings/overrides, and preserve evidence. Do not operate another user's or mission's sessions.

### Automated evidence and compatibility limits

QA did not rerun the coder's suite or re-review the implementation. The handoff reports scoped Antigravity usage tests (23 passed), direct-quota tests under a poisoned synthetic outer-home fixture (11 passed), formatting, workspace Clippy with and without updater, and the earlier daemon suite (1948 passed). Reviewer independently reran the formerly failing proxy test with an isolated synthetic outer home (1 passed). These are reported automated results, not QA native passes.

Windows compilation and the Windows-only `windows_probe_spawns_no_child_process` and `windows_credential_manager_roundtrip` tests remain pending Windows CI at this handoff. The zero-spawn test covers signed-out, unexpired-token quota and expired-token refresh paths with fixtures; the Credential Manager round trip uses a test-only target. Coder owns driving CI green and recording its actual result in the PR.

Only agy 1.5.9 was available to the coder for binary inspection. The handoff supplies 1.5.9 storage/marker, consumer/GCP OAuth client/host and schema evidence, plus consumer-endpoint User-Agent measurements. **agy 1.5.8 credential/API compatibility was not independently verified**, and no native Windows or live GCP acceptance is established here. Include 1.5.8 in the Windows validation when available and retain this limitation in the PR until checked.

### Cleanup and disposition

QA created only this Markdown record; there are no app processes, test objects, configuration changes or credentials from this run to clean up. No implementation code changed and QA made no commit, push, PR or label change. The PR did not yet exist. Coder must carry the matrix, reproduction steps or a link to this record, 1.5.8 compatibility limitation, pending Windows CI and required `risk-high` / `needs-live-test` labels into the PR. Windows checks remain unresolved until their native evidence is recorded.
