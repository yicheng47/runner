# #1477 GPUI 0.3.14 macOS live QA

The macOS matrix is incomplete. No product failure was confirmed in the checks that could run. Required input-driven checks remain blocked because the native Computer Use service rejected the Runner-hosted Codex process. Windows and Linux checks 1–7 remain pending. Keep `needs-live-test` on the PR.

## Candidate and environment

On 2026-11-25, Runner mission `01M8966M190075X3CP2X4JCDJK` handed QA the clean, reviewed commit `ace6d29458fae52493ac436f31ae33074ef9ca9a` on `chore/1477-gpui-pre-0.3.14`. QA compared clean base `678da3ec` in the same worktree. The host ran macOS 26.6.2 (25G83), arm64, with three connected displays, on AC power with Low Power Mode off. Both builds reported Runner 0.20.4 (dev) and CLI 0.20.4. Rust was 1.98.1 for the candidate and 1.97.1 for the base. Xcode was 26.3 (17C529).

QA acquired `script/qa-lock acquire --timeout 540` before `make run`. Both builds used the worktree's own `target/`, with `NO_COLOR` and inherited Runner mission variables removed. The normal development daemon was already running, so both builds used one disposable home at `/private/tmp/r1477.JEVjKD` and its `Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`. Each development wrapper and its CLI copy matched the corresponding worktree artifacts byte for byte, and the development CLI verified the isolated endpoint before testing. Only the disposable project `QA1477` was created. QA started no agent session or test mission and made no global configuration edits.

Evidence is outside Git in `/private/tmp/r1477.JEVjKD/evidence/`. The two `*-sha.txt`, `*-dirty.txt`, `*-artifacts.sha256`, `*-make-run.log`, `*-status.json` and `*-daemon.json` sets identify the candidates, artifacts and daemon. All screenshots are native captures of the named window. The two wrappers had distinct bundle IDs.

## macOS results

| Check | Result | Observation and evidence |
| --- | --- | --- |
| 1. Terminal rendering and selection, plus trackpad and mouse-wheel scrolling in the terminal, mission feed, Markdown preview and long list | Blocked | The native control service rejected the session before it could open or drive these surfaces. |
| 2. Unfocused repaint during streaming agent output | Blocked | The idle sampling proved an unfocused window remained present. Streaming repaint was unobserved because QA could not drive a test chat. |
| 3. Cursor blink, marked text and committed Chinese in terminal and text field, dynamic shortcut | Blocked | Simplified Pinyin was present and the host started on U.S. input. QA could not send targeted native keystrokes. |
| 4a. Resize, titlebar drag and display move | Passed | Titlebar drag moved the window by the requested 130 × 60 points. Right edge resize changed width from 1440 to 1242 points and back. A native drag moved the same window from x=690 on the main display to x=2960 on the second landscape display and back. The content rendered at each observed size and position. See `04-titlebar-drag.png`, `04-resize.png`, `04-second-display.png`, and the matching `window-*.json` drag captures. |
| 4b. Display sleep and wake | Passed | `pmset displaysleepnow` succeeded. The macOS power log recorded display off at 20:38:45 and on at 20:38:47 local time. The same candidate window rendered after wake. See `04-before-display-sleep.png`, `04-after-display-wake.png`, `window-sleep-wake.log` and `window-sleep-power-events.log`. |
| 4c. Native close and reopen; delivered Notification Center item withdraws on quit | Blocked | Native close and reopen, notification delivery, and notification withdrawal required targeted input. |
| 5a. Tooltip | Passed | Native hover showed the Add project tooltip next to its sidebar anchor. Moving away dismissed it. See `05-project-tooltip-hover.png` and `05-project-tooltip-dismissed.png`. |
| 5b. Menus and popovers | Blocked | Opening, positioning and dismissal required targeted native input. |
| 6. English and Chinese Markdown wrapping in the mission feed and chat file viewer | Blocked | The bilingual test text is in `fixture.md`. Both native views remained untested. |
| 7. File drop into terminal and tab or pane drag | Blocked | Real-cursor window drags worked. QA could not set up the terminal and tab/pane targets or verify their application behavior. |
| 8. Focused and unfocused idle wakeups | Passed | See the sample table below. |

The `@oai/sky` tool returned `Sky Computer Use native pipe startup failed` before a UI action. The macOS helper log reported `Sender process is not authenticated` after retries with both the verified app path and bundle ID and a fresh tool kernel. `native-control-error.log` records the host-side errors. The candidate launch screenshot is `00-candidate-window.png`. QA did not change the system's automation or authentication configuration.

## #1135 idle measurements

Both builds displayed the same idle Welcome page and disposable project on the main display, with one window, no attached terminal and no active animation. For each condition QA collected two samples 30 seconds apart with `top -l 2 -s 30 -pid <pid> -stats pid,command,cpu,idlew`. CPU is the second sample. Wakeups are the difference between the displayed IDLEW values. Focus observations, sample timestamps and raw output are in the evidence directory. One early candidate focused attempt changed frontmost application at the end of the interval and was replaced by a continuously focused attempt. The interrupted raw sample was retained. [Apple top manual](https://github.com/apple-oss-distributions/top/blob/main/top.1)

| Build and focus | CPU | IDLEW counters | Package-idle wakeups / 30 s | Interrupt wakeups / 30 s |
| --- | ---: | ---: | ---: | ---: |
| Base `678da3ec`, focused | 0.9% | 13 → 71 | 58 | 3760 |
| Candidate `ace6d294`, focused | 0.0% | 0 → 0 | 0 | 52 |
| Base `678da3ec`, unfocused | 1.1% | 88 → 124 | 36 | 3747 |
| Candidate `ace6d294`, unfocused | 0.0% | 0 → 0 | 0 | 57 |

**#1135 looks fixed in the sampled configuration on macOS 26.6.2.** The candidate recorded zero package-idle wakeups and 0.0% CPU in both focus conditions. Per-process interrupt wakeups also fell sharply. The check covered an idle single-window Welcome page. Evidence includes `08-base-idle.png`, `08-candidate-idle.png`, and `*-rusage.txt`. The kernel-backed package-idle counter matched the raw `top` values for all four intervals.

## Cleanup

Stopped both private application wrappers and their private daemon processes. Unregistered both test bundles, restored the feature branch after the baseline measurement, and released the shared QA lock. The existing installed Runner and normal development daemon were left running. The scratch project, build wrappers and evidence remain under the disposable home. The pre-existing TextEdit process was used as the foreground app for the unfocused samples and left running. No QA chats or missions were created.

## Rerun on 2026-11-25

QA2 built the clean reviewed head `9936a7fe0fd80f134a1c6bb25a8e49aa902ee104` on `chore/1477-gpui-pre-0.3.14` with `make run` and Rust 1.98.1 on macOS 26.6.2 (25G83), arm64. The dev app and CLI reported 0.20.4. QA acquired the shared lock and verified the private development socket under `/private/tmp/q1477b.qB8eQm`. The temporary app and CLI initially matched the worktree binaries byte for byte. The native click preflight worked. For the notification retry, QA signed a separate wrapper ad hoc and verified its executable text and CLI matched the candidate.

Evidence is in `/private/tmp/q1477b.qB8eQm/evidence/`. The commit, clean diff, build log, hashes, and endpoint records identify the tested build. QA used a shell for numbered ANSI and Unicode output, a private empty mission for repeatable feed messages, and one private pi chat with DeepSeek Flash at low effort. Checks 4a, 4b, 5a, and 8 retain their first-run passes.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| 1. Terminal rendering, selection, and four-surface scroll | Blocked | ANSI colors and Chinese text rendered, and native dragging highlighted multiple terminal rows. Computer Use scroll moved the terminal, mission feed, Markdown preview, and the 80-file list. The driver exposes one scroll action, leaving separate precise trackpad and discrete mouse-wheel verification pending. See `01-terminal-rendered.jpg`, `01-terminal-selection.jpg`, `01-terminal-scroll.jpg`, `06-feed-at-end.jpg`, `06-feed-bilingual.jpg`, `01-markdown-scrolled.jpg`, and `01-files-list-scrolled.jpg`. |
| 2. Unfocused streaming repaint | Passed | While TextEdit remained frontmost, the pi terminal advanced from a partial line 24 to line 50 and settled to Idle. See `02-stream-0.jpg`, `02-stream-1.jpg`, `02-stream-7.jpg`, and `02-frontmost.txt`. |
| 3. Cursor blink, IME, Chinese entry, and dynamic shortcut | Passed | Blink samples 0, 1, and 3 show visible, hidden, and visible again. Simplified Pinyin showed marked text and committed 你好 in the chat terminal and 中文 in Find files. A live ⌥⌘9 override hid and restored the sidebar while the chat terminal had focus. See `03-blink-sample-*.jpg`, `03-terminal-ime-*.jpg`, `03-field-ime-*.jpg`, `03-shortcut-recorded.jpg`, and `03-terminal-sidebar-*.jpg`. |
| 4c. Close/reopen and Notification Center withdrawal on quit | Blocked | Native close removed the second window, and New Window reopened it with a fresh window ID. The unsigned wrapper's notification attempt returned `UNErrorDomain error 1`. The separate signed wrapper passed code-sign verification, but macOS still denied authorization and Runner showed “Notifications are off.” No delivered item was available for the quit check. See `04c-after-close.jpg`, `04c-reopened.jpg`, `04c-*-windows.json`, `04c-notification-service.log`, `04c-signed-authorization.log`, and `04c-signed-notification-permission.jpg`. |
| 5b. Menus and popovers | Passed | The layout menu opened below its control and closed on selection. The chat title popover opened at its header anchor and dismissed with Escape. The native File menu opened and its accessibility Cancel action dismissed it. See `07-layout-picker.jpg`, `07-split-before-fill.jpg`, and `05b-title-*.jpg`. |
| 6. English and Chinese Markdown wrapping | Passed | Both languages wrapped within the mission feed and the pi chat's rendered Markdown viewer. Widening the viewer reflowed both paragraphs. The sample showed no clipping or horizontal overflow. See `06-feed-bilingual.jpg` and `06-chat-markdown-{narrow,wide}.jpg`. |
| 7. File drop and pane drag | Passed | A Finder drop inserted the quoted path of a QA file containing spaces at the shell prompt without running it. Dragging the pane grip rearranged the two marked shells from side by side to stacked. See `07-file-dropped-settled.jpg`, `07-native-file-drag.json`, `07-pane-order-{before,after}.jpg`, and `07-pane-drag.json`. |

No product failure was confirmed. The device-specific scroll comparison and quit-time withdrawal of a delivered notification are still required on macOS. Windows and Linux checks 1–7 remain pending. Keep `needs-live-test`.

QA restored the original U.S. input source, verified the private UI settings matched the backup byte for byte, closed both shells and the scratch Finder window, and archived the QA chat and empty mission. QA quit both wrappers, unregistered both bundles, stopped the private daemon, verified no private process survived, and released the lock. Scratch data and evidence remain available.

During notification setup, a native state query still targeted the retired unsigned wrapper. Computer Use relaunched it with the default home, which automatically replaced the pre-existing normal development daemon and reported restarting one session. QA caught the unexpected window before sending input and stopped the stray wrapper. The normal development daemon remained running, and the installed Runner stayed untouched. See `native-app-selector-incident.txt`.
