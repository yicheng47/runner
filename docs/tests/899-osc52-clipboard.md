# #899 — OSC 52 clipboard live QA

Verdict: **Incomplete**. The five local spec checks and both mission drawer checks passed. The owning-member and secondary-window member checks are **Blocked** because the available member runtime does not emit raw OSC 52. No #899 product failure was established. Keep `needs-live-test` while these required cases remain blocked.

## Candidate and environment

- Repository: Runner; branch `feat/899-osc52-clipboard`.
- Reviewed candidate: `c470e4dc3c40ac980e4d10110fd40fc25d7de0e7`; base: `a06bd8a89b6dfeef466eb7644e850ccdd1019ff5`.
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-899-osc52-clipboard`; clean at build time. Empty uncommitted-diff SHA-256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- macOS 26.6.2 (25G83), Apple Silicon; Rust/Cargo 1.97.1, zsh 5.9, Neovim 0.12.4, Codex CLI 0.166.0.
- Built with `make run` after the previous development QA app exited on its own. No pre-existing app or session was stopped.
- App: Runner 0.19.3 (dev), copied byte-for-byte to `/private/tmp/runner-899-qa.c732jsev/Runner QA 899.app/Contents/MacOS/Runner`; SHA-256 `0f9a12f294958eb8bb70052ccaf2038f14e60bd084587958a12e9702d93d4864`.
- CLI sidecar SHA-256: `4b0ade88a78f775bdd383fe9f49cb222bfa7e4989828e08c2199ab093799165b`. Absolute development CLI reports `com.wycstudios.runner-dev/runnerd.sock`; installed Runner was used only for crew coordination.
- Evidence root: `/private/tmp/runner-899-qa.c732jsev`. Build identity, initial plan, versions, process/session ledger and pasteboard observations are retained there.

## Authorization, scope and method

Jason authorized macOS development-app QA in this mission. He removed tmux from live scope, directed QA to leave the earlier QA 1177 app untouched until it exited, and confirmed there is no second paired computer. This is feature validation, not a full regression run.

Selected checks are the spec's five local checks plus the reviewer's mission visibility cases, limited to the relevant visibility portions of WS-WINDOW-01 and WS-DRAWER-02. Each must demonstrate the real pasteboard acceptance or rejection for a terminal the app actually draws. Windows/Linux native checks and the conditional remote viewer are not run.

Native controls use computer use bound to the distinct QA app bundle. Opening New tab verified the binding before terminal input. Commands were entered through visible terminal panes, with screenshots and real `pbpaste` reads; no private PTY input, fake events or direct database access was used. The app and development CLI omit implementation-mission environment variables and `NO_COLOR` on app launch. Test data lives under the canonical scratch root. The app is a copied fixed build and is not rebuilt from subsequent working-tree changes.

## Results

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Shell copy | Passed | Visible shell emitted the spec's BEL-terminated OSC 52; real pasteboard read `runner-osc52`, with no toast/status feedback. `evidence/01-shell-copy.png`, `clipboard-results.jsonl`. |
| Neovim yank | Passed | Clean Neovim with `g:clipboard="osc52"` opened the one-line fixture. Native `"+yy` followed by ⌘V in TextEdit pasted `runner-osc52-neovim-899` and its newline intact. `evidence/05-neovim-pasted-textedit.png`. |
| Read refused | Passed | UI-launched raw-terminal probe emitted the spec's `?` sequence and listened for 1.5 seconds: zero reply bytes. Clipboard stayed `runner-osc52`. `evidence/02-read-refused.png`, `clipboard-results.jsonl`. |
| Oversize ignored | Passed | Visible shell emitted a base64 payload decoding to 1,100,000 bytes; clipboard stayed `runner-osc52`. `evidence/03-oversize-ignored.png`, `clipboard-results.jsonl`. |
| Hidden chat tab dropped | Passed | Started delayed OSC 52, immediately opened another terminal tab, then confirmed `HIDDEN_899_COMPLETE` in the original after it fired. Pasteboard still contained the Neovim line. `evidence/06-hidden-tab-other-terminal.png`, `evidence/07-hidden-writer-completed.png`, `clipboard-results.jsonl`. |
| Visible mission drawer | Passed | Selected shell in an open drawer emitted OSC 52; `pbpaste` read `drawer-visible-899`. `evidence/09-mission-drawer-visible.png`. |
| Feed with drawer closed | Passed | First attempt inconclusive from external pasteboard interference. During Jason’s reservation, visible OSC 52 set `drawer-retry-sentinel-899`; Feed was selected and the drawer closed before a delayed distinct write. Reads before and six seconds after matched the sentinel; reopening showed `DRAWER_RETRY_COMPLETE`. `evidence/11-drawer-retry-hidden.png`, `evidence/13-drawer-retry-complete.png`, `clipboard-results.jsonl`. |
| Owning member copies | Blocked | Roles reject shell runtimes. A native Codex `!printf` command completed in the member terminal but filtered the OSC sequence; clipboard retained the drawer sentinel. No supported raw member emitter established. `evidence/12-member-shell-probe.png`, `clipboard-results.jsonl`. |
| Secondary remembered member tab | Blocked | Same missing raw emitter: a negative result cannot prove a non-owning window drops the event. No second window created. Coder reports separate in-process coverage in `mission_workspace::tests::osc52_visibility_follows_the_mission_terminals_a_window_draws`; that is not a live pass. |
| Remote viewer | Skipped | Not run: no second paired computer, confirmed by Jason. |
| Windows and Linux | Skipped | Not live-tested on this macOS host; CI/unit validation is separate. |
| tmux | Skipped | Removed from the spec and live scope by Jason; not installed. |

## Findings and test conditions

The first fixture launch requested `gpt-6.1-luna`, as older development fixtures had used. The provider rejected that model for this account. Its error is retained in `evidence/08-fixture-model-unavailable.png`. The native model catalog identified `gpt-6-luna`; QA dismissed the catalog without changing global model settings, updated only the disposable role, and restarted it successfully. This fixture setup issue is not attributed to #899.

An initial native key-name attempt at the Neovim yank did not establish clipboard delivery. Using explicit Shift+apostrophe, Shift+equal, y, y produced the real successful yank and TextEdit paste. The earlier attempt remains in the evidence; it is not a product failure.

The first mission negative-check attempt was contaminated by unrelated pasteboard content. That attempt remains inconclusive; the isolated retry passed. The final config audit matches the pre-run configuration after removing only the run’s own mission trust entry.

## Cleanup

Completed on 2026-11-05. Only the run’s resources were operated:

- Mission `01M6N4ZRK3GP3AP8CJW43SMXJZ` is completed and archived at `2026-11-05T06:10:00.910150Z`, with zero live members. Its archive closed drawer shell `01M6N51BFBNKQVQRMWDJTNK0Q3`. Original member `01M6N4ZYYVXJSZKF46H41PAE8A` and its restarted conversation remain in preserved history.
- Standalone shells `01M6N4T5BSF5FHP5ZWN90MMW11` and `01M6N4XDRM1GBNNZZTVRY45QM9` were stopped through the development CLI. Terminals close rather than archive; an archive attempt was rejected before successful stops.
- The candidate and test TextEdit app were quit. TextEdit initially saved the evidence document as a literal path-like filename in its default iCloud directory; QA moved that owned file into `neovim-paste.rtf` under the scratch root, leaving no owned document in that cloud directory.
- The test-started development daemon was stopped. Final `ps` verification by executable identity found no surviving owned app, daemon or scratch-path process. A numeric PID reused by a later tool process was excluded by verifying its executable identity.
- Only the run’s mission trust entry was removed from Codex configuration. Final parsed config matches the backup: no remaining project or non-project differences. The QA wrapper was unregistered from LaunchServices; its files remain with the evidence.
- Disposable role `qa-899` (`01M6N4ZR3TBR20YVHC5ZCJQH2Z`) is retained for history; no crew was created. No branches, worktrees, credentials, existing sessions or installed-app resources were changed.
- The clipboard/UI reservation was explicitly released through Runner after execution. Evidence and conversation history are retained.

Cleanup evidence under the scratch root: `ledger.json`, `mission-before-cleanup.json`, `mission-after-cleanup.json`, `own-sessions-before-cleanup.json`, `config-audit.json`, `config-cleanup-audit.json` and `process-cleanup.json`.

Automated checks and CI are owned and reported by the coder. These results do not claim an automated or full-regression pass.
