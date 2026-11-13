# #1300 Windows file links — macOS live QA

## Candidate and scope — 2026-11-13

Candidate `f26d79bae5b02d120a10950f25a786a7598110a4` on `fix/1300-windows-file-links`, reviewed clean before QA. Worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1300-windows-file-links`; source tree clean before building. The QA record is the only QA-owned repository edit.

Authorized by the mission's agy trio brief and development-app standing authorization. Selected checks are the TERM-LINK-01 subset named by the reviewer: plain local file URIs (case and encoding included), OSC 8 local-host URI, relative source/HTTPS regression routes, and rejected foreign-host OSC 8 label fallback. Native Windows drive-letter and `rundll32.exe` activation remains required but unavailable on this macOS host. This is feature QA, not a full regression run.

Environment: macOS 26.6.2 (25G83), Apple Silicon. Native control uses `@oai/sky` via node_repl. Scratch fixtures and screenshots: `/private/tmp/runner-1300-v_begq1u`. No agent runtime or account is needed; use one owned shell terminal. Development settings backed up to `/private/tmp/runner-1300-v_begq1u/ui-settings-before.json`; original `fileLinkEditor` is `zed` and will be restored after testing DefaultApp.

Build: `env -u NO_COLOR make run` from this worktree, using its own `target/`. Shared lock acquired with `script/qa-lock acquire --timeout 540` before building/launching. Automated gates belong to the coder's handoff and are not rerun by QA.

## Results

Tested 2026-11-13 20:49–20:55 CST. Build succeeded, then only this run's bare development app was terminated and replaced with `/private/tmp/runner-1300-v_begq1u/Runner QA1300.app`, bundle ID `com.wycstudios.runner.qa1300`. App and CLI copies were checked byte-for-byte against this worktree's build. Runner SHA-256: `9d3dbaf233ef75bf7549829923096f23a99d64cd0ae61c1c711bb419011c6590`. CLI SHA-256: `7653013b093628810edfd4bb6403b3e75cb8988337075ac7bbec311ecff76df9`. App and CLI reported version `0.19.8 (dev)` / `0.19.8`; `status --json` confirmed the endpoint under `com.wycstudios.runner-dev/runnerd.sock` and outside-mission identity. The copied binary stayed frozen throughout QA; the coder was notified that the build had finished.

| Case ID | Platform / variant | Result | Evidence and reason |
| --- | --- | --- | --- |
| QA1300-URI-01 | mac / plain `file:///.../external/report.html` outside Files root, DefaultApp | Passed | Native Cmd-click opened the intended HTML in the existing default browser, Zen, with heading `QA1300 PLAIN FILE URI` and the exact fixture path in the browser AX tree. `/private/tmp/runner-1300-v_begq1u/evidence/plain-open.jpg`; terminal fixtures in `fixtures.jpg`. Covers plain URI detection through the external launcher. |
| QA1300-URI-02 | mac / uppercase `FILE://localhost` and percent-encoded space | Passed | Native Cmd-click of `FILE://localhost/private/tmp/runner-1300-v_begq1u/external/space%20report.html` opened `QA1300 ENCODED SPACE`; browser AX URL identified `space%20report.html`. `/private/tmp/runner-1300-v_begq1u/evidence/encoded-open.jpg` and `after-encoded-click.jpg`. Covers scheme case, local authority and decoding. |
| QA1300-OSC-01 | mac / OSC 8 with local machine authority and descriptive label | Passed | Fixture target `file://Jasons-Mac-Studio.local/private/tmp/runner-1300-v_begq1u/external/osc.html`, label `OSC LOCAL HOST REPORT`. Native Cmd-click opened `QA1300 OSC LOCAL HOST` at the intended external file. `/private/tmp/runner-1300-v_begq1u/evidence/osc-click.jpg`, `osc-open.jpg`. Covers the shared hostname acceptance fix through native activation. |
| TERM-LINK-01 subset | mac / existing relative `source.rs:23` inside Files root | Passed | Native Cmd-click opened `source.rs` in the same terminal tab's Files side-panel preview with line 23 selected. `/private/tmp/runner-1300-v_begq1u/evidence/relative-preview.jpg`. Covers unchanged in-root routing and line location. |
| TERM-LINK-01 subset | mac / HTTPS | Passed | Native Cmd-click of `https://example.com/?runner-1300` opened Example Domain in the browser; AX URL retained `?runner-1300`. `/private/tmp/runner-1300-v_begq1u/evidence/https-click.jpg`, `https-open.jpg`. Covers existing web URL activation. |
| QA1300-REJECT-01 | mac / foreign-host OSC 8 with existing relative-path label | Passed | Target `file://foreign-host.invalid/private/tmp/runner-1300-v_begq1u/external/report.html` with label `source.rs:31`. Native Cmd-click did not underline or activate the label, did not open an external file, and left the existing preview selection at line 23 rather than 31. Browser remained on Example Domain. `/private/tmp/runner-1300-v_begq1u/evidence/foreign-host-click.jpg`, `foreign-browser-unchanged.jpg`. Covers the rejected-target label fallback branch. |
| QA1300-WIN-01 | Windows / drive-letter paths and file URI native activation | Blocked | Native Windows unavailable on this macOS host. Still required: Ctrl-click `C:\...\report.html`, `C:/.../report.html`, and `file:///C:/.../report.html` through DefaultApp / `rundll32.exe`, including line/column variants. Cross-platform unit coverage and Windows CI are separate evidence and do not establish a native pass. Retain `needs-live-test` on the PR. |

No failed macOS check and no remaining must-fix QA issue observed. Native Windows remains unresolved and must be disclosed in the PR. No full smoke, account-backed runtime, lifecycle/resume, IME or unrelated regression checks were selected.

## Reproduction and cleanup

Scratch project root: `/private/tmp/runner-1300-v_begq1u/project`; HTML fixtures deliberately live in sibling `external/` outside its Files root. A single UI-created shell session (`01M7AFCB4SN7J2GGFBNNZGHB8G`) ran `python3 links.py` through native terminal input. The retained `links.py` emits the exact plain and OSC 8 sequences; `source.rs` has 40 numbered comment lines. No agent account, test role, crew or mission was created.

Opened and dismissed New tab to establish native control, chose the scratch folder using the native directory picker, then used Cmd+T for the owned shell. Changed only File Link Editor from Zed to Default app using the file preview's Open in menu. `restore-editor-menu.jpg` shows Default app selected before restoration. Restored Zed through the same menu, and JSON comparison against the backup showed no changed settings keys. No global agent configuration was edited.

Exited the owned shell through native input. Its pane disappeared, `session show` returned not found, and `session list --json` no longer included its ID; shell sessions are ephemeral, so there was no remaining chat to archive. Closed only this run's four browser tabs, preserving the user's other tabs. Quit the QA wrapper with Keep them running selected, verified its process exited, and unregistered it from LaunchServices. Wrapper, fixtures and screenshots are retained for review.

Cleanup deviation: the development daemon was preserved because a pre-existing development session was running. Stopping that daemon would terminate an unrelated session, contrary to the isolation rule. No development app remains; the shared QA lock was released after wrapper exit. `quit-dialog.jpg` records the preservation choice. Installed Runner and its sessions were never operated.

Commands used: `script/qa-lock acquire --timeout 540`; `env -u NO_COLOR make run`; copy app/CLI into the temporary wrapper and compare bytes; launch with `NO_COLOR` and implementation mission identity variables unset; development CLI `status --json`, `session list --json`, and owned `session show <id> --json`; unregister wrapper with `lsregister -u`; `script/qa-lock release`. The development CLI used its explicit absolute path and empty `RUNNER_SESSION_CLI`, with mission variables unset.

