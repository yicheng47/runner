# #1374 — Appearance theme controls — macOS QA

**Current verdict: Passed on `3c690ee5fab90c2342202bd724702ff2fcbafb58`.** Attempt 2 passed the affected focus checks. Attempt 1 supplies the remaining selected macOS checks.

## Attempt 1 — 2026-11-21

**Verdict: Failed.** The theme gallery's keyboard focus outline touches the selected preview border on the same card (QA-1374-01). Both layout sizes and languages, live theme pairing, segmented selection, Archived filtering, and two-window material changes passed their respective checks.

### Candidate and method

- Repository `yicheng47/runner`, branch `feat/1374-appearance-theme-controls`, worktree `.worktrees/feat-1374-appearance-theme-controls`. Reviewed candidate `eb259205e5048011b171cd152c4eb7bb9ab61027`, base `8012ecb7dea011bf99811fd7450500afeee5a45c`. The worktree was clean before build and at launch. Empty tracked diff SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. Later coder edits never entered the frozen wrapper.
- macOS 26.6.2 (25G83), arm64, Rust/Cargo 1.97.1, GNU Make 3.81, zsh 5.9. Coordination CLI and candidate CLI 0.20.2. The app reported 0.20.2 (dev). The only runtime QA started was the native zsh terminal.
- The approved #1374 mission and clean review authorized the selected macOS cases. QA acquired the shared lock before building with `make run`, stopped its own bare app, then copied the app and CLI byte-for-byte to a uniquely identified scratch bundle. Native New tab and Settings actions confirmed the input target. The absolute development sidecar reported outside-mission mode and the `com.wycstudios.runner-dev/runnerd.sock` endpoint. No account setup was needed.
- Scratch root `/private/tmp/runner-1374-qa.rppbYB`. Screenshot names in the matrix are under its `evidence/` directory. `dev-status.json`, `identity.txt`, `plan.md`, and the resource records remain in the scratch root. The tested app path was `Runner 1374 QA.app/Contents/MacOS/Runner` within that root (PID 71458, daemon PID 37499). App SHA-256 `25347b32754a3ae2063e7b21f060c2654e557d842f294b35d33798b1826896e1`; CLI SHA-256 `fe5626cc9e02782584f6a15cc9103b329c5189042699417536908372e82998dd`. Both hashes were verified again after teardown.
- Seeded a 960-point native window before launch using the backed-up development window state. Moved it to the main display and widened to 1440 points using the documented real-cursor fallback after native drag hit the secondary display's width. Retina screenshots measure 1920 and 2880 pixels respectively. Zoom and language were changed through native controls and confirmed in the saved settings. Native input ran the bounded ANSI palette fixture in an owned canonical scratch shell.

### Result matrix

| Check | Result | Observations and evidence |
| --- | --- | --- |
| SET-THEME-01 — EN and zh-CN layout | **Passed** | The caption, localized Mode and Window material segments, icons, both gallery headings/meta and exactly one active pill fit at 960 points/100% and 1440 points/130% in both languages. At 130%, the lower theme cards were reachable through normal scrolling and their labels remained readable. Evidence: `en-960-100-*.jpg`, `zh-960-100-*.jpg`, `{en,zh}-1440-130-*.jpg`, `zh-960-settings.json`, `zh-1440-settings.json` and the scratch `window-1440.json`. |
| SET-THEME-01 — Keyboard and mouse activation | **Passed** | At 960 points/100% in English, mouse selection, Tab/Shift-Tab, Enter and Space changed Mode, Window material and gallery selection. Ctrl+Tab left the focused gallery card unchanged, and the following Enter selected it. Evidence: `keyboard-mode.png`, `keyboard-material.png`, `keyboard-gallery.png` and `after-keyboard-settings.json`. |
| SET-THEME-01 — Selected gallery card with keyboard focus | **Failed** | A selected card's outer focus outline and preview selection border use the same accent and touch along the top and sides. The label and check remain readable. QA-1374-01 below. |
| SET-THEME-02 — Live app and shell pairing | **Passed** | All six dark pairings plus Tokyo Night Day, Gruvbox Light, Solarized Light and Runner Light recoloured the same open shell and app chrome. All captures retained the fixture output and shell PID. Light/Dark moved the active pill, and Auto showed a single pill for the OS-resolved mode. Evidence: `terminal-palette-contact.png`, `{dark,light}-*-{gallery,terminal}.jpg` and `mode-*.jpg`. |
| SET-THEME-02 — Segmented selected state | **Passed** | In Runner Light, Carbon, Gruvbox Light, Solarized Light and Tokyo Night the selected Mode and Window material segments showed a lifted fill, distinct border and stronger label. The shadow was subtle in dark palettes. Evidence: `segmented-theme-contact.png` and the theme gallery originals. |
| Settings → Archived | **Passed** | In Runner Light, mouse selection, Shift-Tab + Enter and Tab + Space changed the selected segment. All showed the owned archive fixture and chat rows, Missions showed the fixture and other mission rows, and Chats hid the fixture and retained chat rows. Existing archived rows were observed only. Evidence: `archived-*.jpg` and `archived-contact.png`. |
| SET-PREF-02 — macOS, two windows | **Passed** | Glass and Solid selections and the window chrome updated live in both windows in Light and Dark, including changes made from either window. Screenshot samples confirmed the sidebar and frame changed while the workspace sample stayed identical within each mode. Evidence: `material-{a,b}-{light,dark}-{glass,solid}.jpg`, `two-window-material-contact.png`, `material-pixel-samples.json` and `material-dark-glass-settings.json`. |
| Terminal Ctrl keys — Windows/Linux | **Skipped in this macOS run** | The row is platform-gated and uses the shared segmented control. Its platform-gated automated checks and CI are separate from this live verdict. |

### QA-1374-01 — Gallery focus outline merges with the selection border

Jason flagged the selected Runner Light theme card. On the reviewed build, open Appearance in Light mode at 960 points/100%. Click the selected Runner Light card, press Tab to focus Catppuccin Latte, then Shift-Tab back to Runner Light without changing the selection. The outer green focus outline meets the green preview border along its top and both sides, resembling one thick outline. The same effect occurs after using Enter to select the keyboard-focused Latte card, in purple. The selection check and card name stay readable. The review handoff requires focus and selection to be independently distinguishable on the same card. The introduction date of this appearance remains unverified.

Evidence: `finding-runner-light-keyboard-selected-detail.png` and `finding-runner-light-selection-focus-moved-detail.png`, their full native captures, and `keyboard-gallery.png`. The coder received the finding and screenshot paths. A reviewed fix requires an affected gallery keyboard and visual retry.

### Teardown

- Stopped the owned shell session `01M7YHDG82YMMP59BE0CXST849`. This nonresumable shell disappeared from the list on stop, so archive returned “session not found”; the shell process also exited. The empty owned fixture mission `01M7YHEWA3W79PEXG4MBKSJF5Y` was stopped and archived with zero sessions and a confirmed archive timestamp. No roles or crews were created.
- Closed the owned secondary window, quit the scratch wrapper, shut down only the development daemon started by this run through its absolute CLI, and unregistered the wrapper. The daemon's normal shutdown stamps resumable sessions. No input, direct session stop or archive was applied to pre-existing chats or missions. No prior installed or privately isolated daemon was operated.
- Restored `ui-settings.json`, `window-state.json` and `window-layout.json` byte-for-byte. Restored UI settings SHA-256 `5a21e7442308f6aaa521f07ec9ddf8d1a775a4bf91cc8d0ca89323d5985385a3`. Verified no owned build, app, shell or daemon process or development socket remained, then released the shared QA lock. Retained screenshots, backups and binaries outside Git.

## Attempt 2 — 2026-11-21 — Focus outline retry

**Verdict: Passed.** The panel-colored gap separates the focus and selection outlines on the Runner Light card. Selected and unselected Mode segments also show the focus gap and retain readable icons and labels.

### Candidate and method

- Reviewed candidate `3c690ee5fab90c2342202bd724702ff2fcbafb58`, on the same branch, worktree and base as Attempt 1. The only worktree edit was this untracked QA record. Empty source diff SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` before build and launch.
- The coder's reviewed handoff authorized the gallery and Mode focus retry. macOS, Rust, Make, zsh and coordination CLI versions matched Attempt 1. The first shared QA lock acquisition timed out at 540 seconds behind another QA run. The second acquired the lock. The normal development endpoint was free before build.
- Built with `make run` and verified a byte-identical copy in `/private/tmp/runner-1374-qa.rppbYB/retry/Runner 1374 Retry.app/Contents/MacOS/Runner` (wrapper PID 15041, daemon PID 12064). App SHA-256 `13bfbdf11a353f9a1583482339bd51157abbc68156f37eb92692622d9ee3e9b6`; CLI SHA-256 `fe5626cc9e02782584f6a15cc9103b329c5189042699417536908372e82998dd`. The absolute candidate CLI confirmed 0.20.2 / 0.20.2 (dev), outside-mission mode and the development socket. The New tab action verified native input before exercising Appearance.
- The native window measured 1920 Retina pixels wide, or 960 points. Saved zoom was 100%. Scratch evidence for this attempt is under `/private/tmp/runner-1374-qa.rppbYB/retry/evidence/`; the plan, executable identity, daemon state and resource ledger are in its parent directory.

### Result matrix

| Check | Result | Observations and evidence |
| --- | --- | --- |
| SET-THEME-01 — QA-1374-01 Runner Light | **Passed** | Clicked Runner Light, tabbed to Latte and pressed Shift-Tab back to the selected Runner Light card. A visible panel-colored gap separated the outer focus outline from the green selection border along the top and both sides. The check and label remained clear. Tab + Enter selected Latte with the same separation, and Shift-Tab + Space restored Runner Light. Evidence: `runner-light-selected-focused-detail.png`, `latte-selected-focused-enter-detail.png`, `runner-light-selected-focused-space.jpg` and `settings-after-focus-retry.json`. |
| Mode — selected and unselected segments | **Passed** | In Light mode, Tab focused unselected Dark, then Shift-Tab focused selected Light. Both outlines kept their track-colored gap, readable icon and label, and distinct selected fill. Repeated in Carbon with focused selected Dark and unselected Light. The selected Carbon gallery card also retained a clearly separate focus outline. Evidence: `mode-{light,carbon}-*-focused-detail.png`, `carbon-gallery-selected-focused-detail.png` and `settings-after-carbon.json`. |
| Other selected cases on this delta | **Skipped on retry** | The reviewer scoped the retry to the focus painting changes. Attempt 1 supplies the layout, live palette, material synchronization and Archived result evidence. |

### Teardown

Native Quit closed the retry window. The wrapper process remained with zero windows, so QA stopped its own PID by executable path, then shut down the owned development daemon through the absolute CLI. This retry created zero chats, missions, roles or crews. QA unregistered the wrapper, restored all three backed-up settings and window files byte-for-byte, rechecked both binary hashes and confirmed the owned processes and development socket were gone before releasing the lock.
