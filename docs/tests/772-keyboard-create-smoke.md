# 772 — Keyboard creation smoke, 2026-10-02

Candidate: `ffa345780bad8bc8bb863475638ce7f0e6881a46`, branch `feat/772-keyboard-create`, base `5a310940`, [PR #776](https://github.com/yicheng47/runner/pull/776). The feature commit rebased cleanly onto latest main. [macOS and Windows CI](https://github.com/yicheng47/runner/actions/runs/36997833928) passed (4m11s / 8m8s). The four required local gates each exited 0: runner-app tests (617 passed; two existing real-agent credit-spending smoke tests ignored), workspace/all-target Clippy, formatting and `git diff --check`. Logs are `/tmp/runner-772-checks/{test,clippy,fmt,diff}-latest-main.log`; CI watch exited 0. These results are separate from live smoke evidence.

**Overall: Blocked — native computer-use window access was unavailable.** The #772 keyboard and visual checklist and the direct-chat lifecycle matrix remain unverified. CLI-only startup checks passed the once-only marker, captured-key and Idle/Completed components for Codex, Claude Code, Copilot and pi. Antigravity failed the requested-directory component. None of these CLI results proves terminal rendering, visible ACK, UI model/effort labels or keyboard behavior.

## Authorization and environment

Jason requested rebasing onto latest main and following the smoke-test instructions using computer use. That explicit live request supplies the bounded existing-account/conversation-history authorization in [the full smoke procedure](full-smoke-test.md), overriding the original mission's app-launch prohibition for this run. The run used development data only and did not open a Runner SQLite database directly or change authentication, global agent configuration or permission settings.

macOS 26.6.2 (25G83); Runner 0.12.7 (dev), built with `make run` from this worktree. The absolute development CLI reported `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/mcp.sock` and the development sidecar. Native Windows was unavailable. TRAE was Skipped: its CLI was not installed, and the prior smoke record reports no account access.

Computer use rejected the bare `target/debug/Runner` executable. A byte-identical copy and CLI sidecar were packaged in `/private/tmp/runner-772-smoke-2z2yhjax/Runner 772 Dev.app` with bundle ID `com.wycstudios.runner-772-dev`, as in the prior #777 run. SHA-256 comparison verified the copied executable; its hash is in `evidence/app-wrapper.json`. The original `make run` process was stopped before launching the wrapper. Development data selection remained unchanged. The wrapper was running and its development CLI endpoint responded, but the initial attempts to bind its native window returned `cgWindowNotFound`. Finder, the already-running installed Runner and Arc returned the same window error; Finder clipboard input also timed out. After Jason confirmed the test app was visible, one binding returned a minimal accessibility window before that turn was interrupted. After the session restart, inventory still showed the app running, but binding by bundle ID and exact path again returned `cgWindowNotFound`, including after resetting the computer-use session. No #772 UI action or agent terminal input was successfully performed. Live UI checks remain Blocked; bringing the app forward did not consistently restore native access.

A setup side effect must be distinguished from intentional smoke actions: the first development launch automatically resumed one pre-existing Claude development chat (`automatic=true` in the launch log). Switching to the CUA-compatible wrapper stopped that process with the original app. No input was sent to it; its stopped row remained resumable and was excluded from the cleanup ledger. No further operation targeted that chat. The installed production app received no input.

Scratch root: `/private/tmp/runner-772-smoke-2z2yhjax`; each startup mission used a different canonical child directory. Evidence: `/private/tmp/runner-772-smoke-2z2yhjax/evidence/` contains command results, per-session structured observations, markers, cleanup, environment and the object ledger. No screenshots could be captured. Existing settings and extensions were preserved; the tests used the application's existing mission permission policy without an override. Live permission labels were not observable.

| Runtime | CLI version | Requested model / effort |
| --- | --- | --- |
| Codex | 0.159.3 | `gpt-6.1-sol` / low |
| Claude Code | 2.1.287 | `sonnet` / low |
| Antigravity | 1.2.14 | `gemini-3.8-flash-low` / low |
| Copilot | 1.0.90 | `gpt-5-mini` / low |
| pi | 0.99.2 | `deepseek/deepseek-flash` / low |

## Live result matrix

| Check | Codex | Claude Code | Antigravity | Copilot | pi |
| --- | --- | --- | --- | --- | --- |
| Fresh direct chat, visible marker ACK and second-turn recall | Blocked: native input unavailable | Blocked | Blocked | Blocked | Blocked |
| Direct-chat Working/Idle, UI overrides, tool, interrupt and recovery | Blocked | Blocked | Blocked | Blocked | Blocked |
| Original and new-conversation stop/resume/context | Blocked | Blocked | Blocked | Blocked | Blocked |
| Startup marker once in requested fresh directory (CLI/file evidence) | Passed | Passed | Failed: written in hooks directory | Passed | Passed |
| Startup captured key and ready/hook/completed (CLI evidence) | Passed | Passed | Passed | Passed | Passed |
| Startup terminal ACK, displayed overrides and strict tool-count check | Blocked | Blocked | Blocked | Blocked | Blocked |
| Native Windows live smoke | Blocked: no Windows machine | Blocked | Blocked | Blocked | Blocked |

Each startup used a newly created single-lead crew and a bounded role prompt. The multi-line goal requested exactly one Python append of `SMOKE-772-AUTOSTART` in the current mission directory, followed by `ACK SMOKE-772-AUTOSTART`, with no other tools, messages, agents or settings changes. The Python command was validated in a separate scratch child before starting. No manual Return, trust acceptance or repeated goal submission was sent after starting a mission. Structured status captured non-empty session keys and ready/hook/completed for all five. Mission processes were idle before cleanup. CLI creation and hooks do not prove the visual ACK or strict number of tools.

Antigravity's expected `mission-agy/sm772-agy.txt` was absent, while its uniquely owned marker appeared in the development `antigravity-hooks/sm772-agy.txt`, with exactly one line. The session row's cwd remained the requested canonical mission directory. This repeats the working-directory discrepancy recorded in [#777](777-runtime-adapter-smoke.md); regression attribution to #772 is not established and no unrelated runtime code was changed.

This host has no qualifying background mission-event notification facility. Automatic watching was reported unavailable; exact foreground `mission feed <id> --follow --json` commands are retained in `evidence/foreground-feed-commands.txt`. One-shot show/session captures supplied bounded startup evidence. No feed follower was started or claimed as an active watch.

## #772 feature checklist

| Check | Result |
| --- | --- |
| ⌘N, ⌘2, choose role, ⌘↵ without mouse | Blocked: native window access |
| ⌘↵ from open select/model menu without selecting highlighted row | Blocked |
| ←/→ switch navigation and focus, mouse focus preservation | Blocked |
| Multi-line mission goal and ⌘↵ | Blocked |
| ⌘T into focused empty pane and new tab, correct project/cwd | Blocked |
| ⇧⌘M on chat/other routes and above Settings | Blocked |
| Create-key guards, + menu overrides/unbound hints, palette entries | Blocked |
| Footer/segment keycaps in both themes and IME behavior | Blocked |

The automated feature regressions pass; they do not change these live classifications.

## Cleanup and retained artifacts

All five test missions were stopped and archived. CLI show verified a non-empty archive timestamp and zero live sessions for each. Recents disappearance could not be inspected because native window access remained blocked. No direct smoke chats were created. Five test roles, five test crews, the scratch project, temporary app bundle, evidence and marker files were retained. The wrapper remained open for Jason to bring forward. No branch, worktree or conversation history was deleted. Cleanup addressed only IDs in the run's ledger.

| Runtime | Mission ID | Role ID | Crew ID |
| --- | --- | --- | --- |
| codex | `01M3Y4AC42J3P7Y87NV6FWP589` | `01M3Y45SY1CQ98WMZ8CF5AKPMR` | `01M3Y45SYCRK7PZK2QT8J95FG4` |
| claude | `01M3Y4APQS3YBDVNQWK20C0YM8` | `01M3Y45T0AGJBDRCB912ZCVSZK` | `01M3Y45T0Q8CFQ1B458KSY7D0G` |
| agy | `01M3Y4APSCZR4SCEQD5VP5S2EP` | `01M3Y45T2K768W73NE81YAA5ZT` | `01M3Y45T2YSDDN4K90MM6228YF` |
| copilot | `01M3Y4APTJ9A1ZBZ9P3ZARPERT` | `01M3Y45T4W630S5BE9Q1FA5XGM` | `01M3Y45T58SEA16QPW0X03PTHN` |
| pi | `01M3Y4APVK6K4P4V2K46F3W27M` | `01M3Y45T7DMR8RBTZQP13VAKHP` | `01M3Y45T7RVM1TEFPE7YMMEHC0` |

Scratch project: `01M3Y457JGM34K99A9WFWGSJNM`. Per-mission stop/archive/show evidence and the retained-ID ledger are under `/private/tmp/runner-772-smoke-2z2yhjax/evidence/`.

## Native follow-up and selector fix, 2026-10-02

Candidate: `3d7bf78fe6c165d72076efebad5de24d011cc453` plus the uncommitted `ui/select.rs` fix. Jason reported that Enter made a selector blink and Down did not scroll its highlighted option into view. The fix handles keyboard activation once on key-down, ignores GPUI's synthesized keyboard click on key-up, and scrolls to the highlighted option during navigation and after the menu's first layout. Three new regressions failed before the fix and passed afterward; all eight select tests passed. This is a correction to existing selector behavior, with no change to the settled #772 bindings or form behavior.

All four required gates exited 0 on this candidate: `cargo test --locked -p runner-app --profile ci --no-fail-fast` (620 passed, two existing real-agent smoke tests ignored), `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`. Each gate logged its own exit code in `/tmp/runner-772-checks/{test,clippy,fmt,diff}-selector-fix.log`. The focused regressions are in `select-before-fix-lib.log` (exit 101) and `select-after-fix.log` (exit 0). `cargo build --locked -p runner-app -p runner-cli` exited 0; its log is `build-selector-fix.log`. The already-committed candidate's [macOS and Windows CI](https://github.com/yicheng47/runner/actions/runs/36999554991) passed; it does not cover the uncommitted selector fix.

The follow-up used the same development namespace, OS, existing account settings and retained bounded smoke roles/crews. The fresh canonical scratch root is `/private/var/folders/9c/9pb565qs47v97w4flgys62kc0000gn/T/runner-772-selector-w3zhdpep`. A byte-identical executable and CLI sidecar were packaged as `Runner 772 Select Dev.app` (bundle ID `com.wycstudios.runner-772-selector-dev`) and launched through computer use, rather than another `make run`. Executable SHA-256: `b4c78a51cb08eba47387412f75929699d1f153d7bb2f0f7ac733fb904dd2b0e4`. `evidence/build.json` records the candidate, dirty file and build method. Screenshots were observed through computer use; screenshot files were not exported.

| Native check | Observed result |
| --- | --- |
| Enter/Space opens a selector and Enter chooses once | Passed: role and crew menus stayed open after key release; choosing closed the menu without reopening it. |
| Down scrolls highlighted options into view | Passed: role and crew highlights moved through long lists and the visible list/scrollbar followed. Reopening the far-selected role showed the current selection. |
| ⌘N from terminal focus, initial picker, Esc | Passed: opened Start chat from outside the modal; initial role picker received focus and Esc dismissed the menu before dismissing the form. After a computer-use reset, ⌘N again visibly opened Start chat. |
| ⌘N, ⌘2, choose bounded role, ⌘↵ without mouse | Passed after selecting the scratch project: keyboard-only form flow created the intended Codex role chat with the requested name and directory. |
| ⌘↵ from an open role selector | Passed: submitted the selected Codex role while a different role was highlighted, without choosing that highlighted row. |
| ←/→ switch and tab order | Passed: Shift-Tab reached the active segment; Left/Right changed mode and retained switch focus; Tab reached the picker, and Space opened it. ⌘2 focused the role picker without changing the tab behind the modal. |
| Create-key guards | Passed in both forms: ⌘N, ⌘T and ⇧⌘M left the existing form and underlying tabs unchanged. |
| Multi-line mission goal and ⌘↵ | Passed: plain Enter inserted goal lines; ⌘↵ from the textarea created one mission with the exact multi-line goal, title, directory and active project. |
| ⇧⌘M and Settings overlay | Passed on the chat route; Start mission was also visibly rendered above Settings and Esc returned to Settings. |
| ⌘T new tab and focused empty pane | Passed: from the mission route it switched to chat and opened a terminal tab in the active project's directory. In an empty pane beside the owned Codex chat, it filled that pane using the sibling chat's different directory. |
| Dark footer and segment keycaps | Passed: Cancel, submit and mode hints were readable on their respective fills. |
| Light keycaps, open model menu confirm, Reset/Browse/footer confirm, mouse focus preservation, IME, plain Enter in Chat name, both + menus and palette | Blocked for this follow-up's computer-use observation: these paths were not completed. Automated coverage remains separate. |

Dark was temporarily switched to Light for the mission's two-theme checklist, then restored when Jason asked why the theme changed. No Light modal screenshot was obtained, so it is not a visual pass. Computer-use input was intermittent after interruptions: a visible window sometimes ignored both synthetic shortcuts and coordinate clicks; native menu accessibility actions worked, and resetting computer use restored a visible ⌘N result. No shortcut regression is established by those inconclusive input attempts. On 2026-10-02 Jason ended the automated live continuation, reported that his own check showed the features working, and authorized updating the PR and merging. His confirmation is separate from the observed rows above; no complete runtime-suite pass is claimed.

The owned Codex role chat showed the original marker ACK, second-turn recall, requested `gpt-6.1-sol` / low labels, Working/tool activity, an interrupted response returning to Idle, and `RECOVERED` followed by Idle/Completed. Its bounded `sleep 30` was backgrounded by the agent; Escape interrupted the foreground response/wait, and does not prove the background wait stopped. This was a role chat, not the procedure's runtime-only baseline. Direct-chat stop/resume/new-conversation checks and the remaining runtimes' native lifecycle matrix were not completed before Jason's confirmation. Native Windows remains Blocked; TRAE remains Skipped because its CLI/access is unavailable.

The keyboard-created single-lead Codex mission executed its goal automatically, with no terminal input, trust acceptance or repeated submission. Its terminal showed one Python command, `ACK SMOKE-772-KEYBOARD` and Idle; `mission-keyboard/sm772-keyboard.txt` contained exactly one `SMOKE-772-KEYBOARD` line in the requested directory. Its captured session key was non-empty. It was stopped and archived; retained `keyboard-mission-cleanup.json` proves an archive timestamp, stopped sole session and zero live sessions. The mission row disappeared from Recents. Automatic background watching remained unavailable and the exact foreground feed command was provided, without claiming an active watcher.

Cleanup targeted only the follow-up ledger. The owned role chat's stop, archive and immediate show each exited 0; the immediate show verified stopped/archived. Later lookups reported it absent. The three owned shell IDs were already absent from the endpoint's recent-session list and returned `session not found` before cleanup could stop/archive them; their final archive timestamps and native disappearance were not verified. A later lookup also could not find the already-archived mission; its earlier zero-live-session cleanup evidence is retained. These lookup limits do not justify touching any other session. No pre-existing session received input or a cleanup command during this follow-up. The scratch project, app wrapper, evidence, markers and earlier smoke roles/crews remain retained.

| Owned object | ID |
| --- | --- |
| Scratch project | `01M3Y86Z9P5CG3ECWC7B58CCK6` |
| Codex role chat | `01M3Y89XMK8GYV8V4976W0G46T` |
| New-tab terminal | `01M3Y8TNCGEYNFQFSXV4R38V1M` |
| Split-terminal sibling | `01M3Y8VWEAH4V1XS4Y6Q24SJRJ` |
| Filled empty-pane terminal | `01M3Y8X9ET3T0928BX8BCBS7CX` |
| Keyboard mission | `01M3Y8S58ASZ880FN84K6RYFGH` |
| Mission's sole session | `01M3Y8S58HBDKJBJS2WH06F0F6` |

Evidence is under the follow-up scratch root's `evidence/`: build and ledger, selector/mission/session observations, marker, stop/archive commands and final lookup diagnostics. The historical blocked run above remains unchanged.
