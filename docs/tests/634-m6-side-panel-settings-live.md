# #634 m6 — Settings → Side panel live QA

Date: 2026-11-05. Verdict: **Incomplete**. The r3 visual fix passes; the required cross-window synchronization check remains **Blocked** by computer-use window selection. Keep `needs-live-test`. This is scoped feature validation, not a full regression pass.

## Candidate and environment

Repository: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m6-side-panel-settings`; branch `feat/634-m6-side-panel-settings`; base commit `6f516770aa854f6b09d6b8e9eebf260cb559155c`. Review and QA used frozen patch snapshots on that base:

| Snapshot | SHA-256 of uncommitted `git diff HEAD` | App binary SHA-256 |
| --- | --- | --- |
| r2 | `1110352518d829e52327c728d0441d4ca7645af53befd4c454893bdacd06db02` | `86e00a7be0321706a13ddab0f719cf303fe84b4aae3533bdf9b2781365489658` |
| r3 | `f2005a875d38c2a0b94af80d7bb3c715aac8e8efd22d0f77f149a8989b889404` | `563d932197b72bf33a292f9734d9b00e90f7833dad2d76a2b27d5c9f9bf5e816` |

macOS 26.6.2 (25G83), arm64; dark theme, Solid material; English and Simplified Chinese; 100% and 120% app zoom for the selector retest. Cargo 1.97.1, rustc 1.97.1, installed coordination CLI 0.19.2, Codex CLI 0.166.0. Product app/CLI reported 0.19.2 development mode and the development endpoint. CLI sidecar SHA-256: `659278937b3c2bd6c79959f775c29ae16c6cfc8bd02e45d1f16120f29f948a7d`.

Built with `CARGO_TARGET_DIR=target/qa CARGO_BUILD_JOBS=8 cargo build --locked -p runner-cli -p runner-app`: r2 exit 0 (2m26s), r3 exit 0 (10.63s). Byte-identical app/CLI copies ran from registered scratch QA bundles. Executable paths, process IDs and hashes were checked before trusting results. Source changed from r2 to r3 while QA continued against the frozen r2 copy; r3 changes only the icon selector width and its three visual samples. Unaffected r2 results carry forward; the affected visual portion was rerun on r3 after its clean review.

## Authorization and method

The mission and reviewer required six live groups: new-page visuals/localization, panel gates on each subject, editor independence, tree/font/size effects, viewer modes, and cross-window synchronization/persistence. The plan was recorded before building in scratch `plan.md`. Followed `docs/tests/full-smoke-test.md` for identity, isolation, native control and cleanup; selected SET-LANG-01/05 only for the new pages. The feature spec, Phase 4 plan and exported design PNGs were the comparison authority. No unrelated runtime, authentication or terminal regression suite is claimed.

All UI actions used native computer use. Harmless Settings navigation established control. Settings-file readback corroborated visible state; file edits were limited to owned fixtures and backed-up development settings for missing-font startup. Four disposable shell terminals, one idle Codex chat and one empty mission supplied subject fixtures. No prompts or commands were sent to pre-existing sessions. Product CLI calls used the absolute frozen development sidecar with mission identity variables removed; installed `runner` coordinated this crew only. No database was opened.

Evidence root: `/private/tmp/runner-634-m6-qa.tn2nbidi`. Screenshot references below are filenames under its `evidence/`; identities, patch copies, hashes, build/launch logs, settings backups, fixture files, detailed notes and resource ledger are alongside. Evidence is retained outside the repository.

## Result matrix

| Check | Result | Observed evidence |
| --- | --- | --- |
| Navigation after App, Panel/Files grouping, Panel on/off visuals and live header preview, no grips | Passed, r2 | `01-panel-en-on.png`, `02-panel-en-off.png`, `03-preview-info-off.png`; design comparison |
| Disabled Info/Files switches retain values and ignore clicks | Passed, r2 | `02-panel-disabled-readback.json`, `02-panel-en-off.png` |
| Both pages and option lists in English and Chinese; searchable font groups/default choices | Passed, r2, with r3 icon correction below | `04-files-en-default.png`, `07-font-options-en.png` through `15-markdown-options-zh.png`; full page strings and checked Rendered/Same as terminal options |
| Icon selector full label and folder/Rust/Markdown samples, both languages, 100%/120%, all three themes selectable | Failed r2; Passed r3 retest | Initial `05-files-en-100-truncated.png`/`06-icon-options-en.png`; r3 screenshots `72-r3-en100-closed.png` through `81-r3-en120-list.png` |
| Master off removes expanded panel and reopen control in chat, terminal, mixed tab, mission and archived chat | Passed, r2 | `44-terminal-master-off.png`, `48-mission-master-off.png`, `49-chat-master-off.png`, `51-chat-collapsed.png`, `52-chat-collapsed-master-off.png`, `58-mixed-master-off.png`, `59-archived-chat-master-off.png` |
| Every applicable optional view off removes chat/terminal panel; mission retains Sessions before Info | Passed, r2 | `42-terminal-files-off-no-panel.png`, `53-chat-every-view-off.png`, `54-mission-only-sessions.png`, `55-mission-sessions-before-info.png`; Files off also removes the expanded viewer |
| Files off and master off still allow Cmd-click to chosen editor | Passed, r2 | `43-editor-files-disabled.png`, `47-editor-master-fresh.png`; Zed visibly showed the exact owned fixture; fresh file stayed 63 bytes |
| Actual Catppuccin, Material and Minimal tree icons | Passed, r2 | `16-tree-catppuccin.png`, `18-tree-material.png`, `20-tree-minimal-code-menlo-confirmed.png` |
| Shared installed monospace font choices; bundled/installed font changes code and Markdown blocks | Passed, r2 | `20-tree-minimal-code-menlo-confirmed.png`, `21-markdown-menlo-highlight.png`, `28-markdown-bundled17.png`, `29-terminal-font-options.png`; visible installed families match Terminal |
| Font size follows Terminal, explicit override stays fixed, Reset follows again; rendered Markdown scales | Passed, r2 | `22-files-override-reset.png`, `23-markdown17-no-highlight.png`, `30-size-override-stays17.png`, `31-reset-follows14.png`, `32-inherited-follows15.png`, size readback JSON |
| Highlighting off affects already-open code/Markdown blocks, reload remains plain, re-enable recolors | Passed, r2 | `23-markdown17-no-highlight.png` through `27-markdown-reenabled.png`; appended Python line visibly reloaded while off |
| Single-click off promotes visible and cached previews; later clicks kept; re-enable retains kept tabs and permits a new preview | Passed, r2 | `33-alpha-preview-before-cache.png` through `40-preview-replaced-kept-survive.png` |
| New Markdown opens as Source/Rendered setting; manual toggle works | Passed, r2 | `37-new-markdown-source-kept.png`, `38-markdown-manual-render.png`, `39-new-rendered-preview.png` |
| Representative nondefaults survive startup | Passed, r2 | Native-saved checkpoint `nondefaults-stopped.json` restored after the operator pause; `64-persisted-nondefaults-r2.png` shows Material, bundled font, explicit 16/Reset, highlighting off, single-click off and Source; `65-persisted-master-off-r2.png` shows master off with retained view values |
| Saved missing font falls back to bundled while keeping the choice | Passed, r2 | `66-missing-font-choice.png`, `missing-font-retained.json`, `68-missing-font-code.png`; glyph rendering matches explicit bundled comparison `69-bundled-font-comparison.png` at 16 px |
| All Files values synchronize in two already-open windows; size step uses latest value and Reset synchronizes | **Blocked** | Window B was open on Files while A changed all six values. CUA could not select B through window cycling. Operator restored it after one minimize attempt; selection still failed. Retry `62-cross-window-retry-blocked.png` remained on A. No cross-window result is inferred from disk readback. Operator instructed no further rearranging. |
| Side-panel shortcut no-op | Skipped, baseline gap | Reviewer established no side-panel shortcut binding exists in the base keymap; hiding controls passed, but an unbound shortcut is not counted as a tested no-op |
| Light theme, real-pointer drags/resize/hover-only states, remote access | Skipped | Explicit mission deferral by Jason, 2026-11-04 |

Screenshots 19, 63, 67, 70 and 71 represent ineffective navigation/setup attempts and are excluded from pass evidence. In the r3 startup navigation miss, only a pre-existing stopped subject was selected; no input, resume or mutation occurred. Those images remain private scratch evidence.

## Findings and setup incidents

F1, P2, resolved on r3: the closed File icons selector truncated Catppuccin at both 100% and 120% English zoom. The open list fit. Coder widened the control and corrected its samples; reviewer approved the delta before QA built r3. The full label and ordinary folder/Rust/Markdown samples now fit at all four locale/zoom combinations. The initial failure remains recorded.

Unattributed observation: `editor-open.txt` became empty during the first editor check. A fresh `editor-master.txt` opened correctly with the master switch off and remained 63 bytes before opening, after closing and after later checks. This was not reproduced, and no regression attribution is established.

Shared-app scheduling caused two failed startup attempts while another mission held the development app. QA's orchestration incorrectly issued launch even after its guard failed; Runner rejected both without taking the other app's daemon. No results were trusted from those attempts and settings were not changed by the failed guards. The operator and coder were informed. The launcher was corrected to one foreground script that exits before settings changes or launch on guard failure. Subsequent launches were conditional, including a successful guard rejection while our own prior daemon still existed. Other missions' apps were not stopped.

The human paused this run to release the development app. At that boundary QA restored its original settings backup and unregistered the wrapper; frozen binaries/evidence were retained. After a later operator release, QA backed up the then-current settings before restoring test checkpoints. The first checkpoint selected on resume had already been replaced by pause cleanup; this was detected from UI plus readback, excluded, and corrected using the preserved native-saved checkpoint.

## Automated checks

Coder reported runner-app nextest: 1212 Passed, 3 Skipped; fmt, workspace CI-profile clippy with and without `runner-app/updater`, and guide translation check all exit 0. These are coder-reported gates, not QA reruns. QA's two app/CLI builds both exited 0. CI remains the implementer's responsibility.

## Cleanup

All four owned terminals were stopped through the candidate CLI; normal product terminal closure removed their rows, and final `session show` returned not found. The owned Codex chat is stopped/archived and the empty mission completed/archived; their histories remain. No roles or crews were created. The Codex scratch trust entry was removed under the procedure's standing authorization; a before/after diff proves only the owned entry was removed and the resulting configuration matched the prelaunch configuration semantically.

Both QA bundles were quit and unregistered (exit 0); the owned development daemon was stopped (exit 0). Final `ps` showed no scratch app or development daemon. The pre-resume development settings backup was restored byte-for-byte, SHA-256 `1d411ee5dc2c881904f4c4677bf7cefc5bcac5d4273a4f689202367d5bae9951`, preserving the settings left by intervening QA. Installed Runner was left running. Scratch builds, fixtures, evidence and conversation history are retained. `cleanup-final.json`, `owned-session-show-final.json`, `owned-mission-final.json` and `codex-own-trust-removal.diff` contain the cleanup evidence.
