# #1452 Terminal file-link detection on macOS

**Verdict: Passed.** TFL-01 through TFL-04 passed on reviewed candidate `cdeb787b51c5e0df85d3e686417b0084f4d9e906`.

## Candidate and environment

- Tested 2026-11-24 on macOS 26.6.2 (25G83), arm64. Repository `yicheng47/runner`, branch `feat/1452-terminal-file-link-detection`, base `a084538bd99d03661f0c63d2228e6f40b2c14464`. The worktree was clean at build and after cleanup. The empty tracked diff SHA-256 was `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- The neon trio handoff and standing development-app authorization cover this run. Scope follows the [feature spec](https://github.com/yicheng47/runner/blob/v0.20.4/docs/features/1452-terminal-file-link-detection.md#live-checks-macos-development-app), including the applicable terminal hover and Files preview actions from TERM-LINK-01 and WS-FILES-04. Native Windows and Linux and the full regression suite were outside this run. Automated checks are recorded by the coder.
- Built with `make run` using the worktree's own `target/` under the QA lock, with the installed mission environment and `NO_COLOR` removed. Rust and Cargo 1.97.1, Runner 0.20.3 (dev), development CLI and daemon 0.20.3, Codex 0.171.0 (GPT-6 Luna, low), Claude Code 2.1.331 (Haiku 5.5, low), and zsh. Retained the agents' existing permission modes and extensions.
- The temporary wrapper, `/private/tmp/r1452.F93bD4/Runner QA1452.app`, matched the built executable byte for byte (SHA-256 `5bb0b3f87cfd066ff0657365fceb4aa1cfb744bc5bc98e9c780b7093f8cc49ae`). The bundled CLI, installed private development CLI and private daemon all matched the built sidecar (SHA-256 `8ce809516a9747b1307f4f50f2fb185f25f11d1b870accea4457994547fc8b73`). The running process used the wrapper path. Its absolute CLI reported outside-mission mode and the scratch profile's `com.wycstudios.runner-dev/runnerd.sock`. See `artifact-hashes.txt`, `bare-executable.txt` and `dev-status.json`.

## Method and results

The scratch Git repository and fixture scripts are under `/private/tmp/r1452.F93bD4`. Screenshot names below are in its `evidence/` directory. Verified native control by operating the New tab launcher, then used native clicks, paste, keystrokes and ⌘-click for the tested actions. The documented real-cursor helper supplied hover and one sidebar resize. Calibration captures `18-hover-calibration-sidebar.jpg`, `tfl03-narrow-shell-before.jpg` and `tfl03-narrow-shell-resized.jpg` show a project hover and the sidebar width change.

| Check | Result | Evidence |
| --- | --- | --- |
| TFL-01, Codex | Passed | At 53 columns, GPT-6 Luna emitted the absolute path across two rows. Hovering either fragment underlined the path through `:3`, with leading indentation, the bullet and padding outside the underline. ⌘-click on either row opened `unique_file.rs` with line 3 selected. `tfl01-codex-row1.jpg`, `tfl01-codex-row2.jpg`, `tfl01-codex-preview-row1.jpg` and `tfl01-codex-preview.jpg`. |
| TFL-01, Claude Code | Passed | At 53 columns, Haiku split the same path at a different cell. Both rows showed the complete two-row underline. ⌘-click on either row opened line 3. `tfl01-claude-row1.jpg`, `tfl01-claude-row2.jpg` and `tfl01-claude-preview-row{1,2}.jpg`. |
| TFL-02, shell | Passed | All nine displayed shapes underlined and opened the expected destination. Individual captures and destinations appear below. |
| TFL-03, narrow shell | Passed | At 51 columns, `printf` emitted `src/a.rs` and `src/b.rs` on consecutive rows. Hovering each highlighted that row alone. Each ⌘-click opened the corresponding file, with its distinct fixture text visible. `tfl03-{a,b}-{hover,preview}.jpg`. |
| TFL-04, shell with different Files root | Passed | The shell started at the scratch repo root, confirmed by `pwd` in `22-shell-cwd.jpg`. `src/deep_module/unique_file.rs:3` stayed unlinked with the original root. After using the Files overflow menu and native folder chooser to select `crates/demo`, the same terminal output underlined and ⌘-click opened `unique_file.rs` at line 3. `tfl04-before-root-hover.jpg`, `tfl04-demo-root-selected.jpg`, `tfl04-extra-root-hover.jpg` and `tfl04-demo-line3-preview.jpg`. |

### TFL-02 path shapes

Each screenshot stem has separate `-hover.jpg` and `-preview.jpg` captures.

| Input | Observed highlight and destination | Screenshot stem |
| --- | --- | --- |
| `@crates/demo/src/deep_module/unique_file.rs:3-5` | Excluded `@` and included the complete range. Opened `unique_file.rs` at line 3. | `tfl02-01-range` |
| `b/crates/demo/src/deep_module/unique_file.rs` | Excluded `b/`. Opened `unique_file.rs`. | `tfl02-02-diff-prefix` |
| `查看 crates/demo/src/deep_module/unique_file.rs，测试` | Underlined the path alone and opened `unique_file.rs`. The delimiter had no hover target. See also `tfl02-03-cjk-path-end.jpg` and `tfl02-03-cjk-delimiter.jpg`. | `tfl02-03-cjk` |
| `app/users/[id]/page.tsx#L2C1` | Included the bracketed segment and location suffix. Opened `page.tsx` with line 2 selected. | `tfl02-04-brackets` |
| `Read(app/(marketing)/layout.tsx)` | Underlined the path with its internal parentheses. Opened `layout.tsx` under `app/(marketing)`. | `tfl02-05-parentheses` |
| Absolute `docs/my plan.md:2` | Continuous underline through the interior space in `tfl02-06-space-full.jpg`. Clicking the space opened Markdown source with line 2 selected. | `tfl02-06-space` |
| OSC 8 `vscode://file` target with a path label | Underlined the path label. Opened `unique_file.rs` at line 3 inside Runner. | `tfl02-07-osc-editor` |
| Plain `cursor://file` URI with `:3` | Underlined the whole URI, including its scheme, consistent with the reviewed terminal range assertions for editor URIs. Opened `unique_file.rs` at line 3 inside Runner. | `tfl02-08-cursor-editor` |
| OSC 8 target `docs/guide.md` | Underlined the label and opened the rendered `docs/guide.md` preview. | `tfl02-09-osc-relative` |

## Setup retries and evidence

The first Codex fixture displayed a model retirement notice for GPT-5.4 Mini (`05-split-right.jpg)). The retry requested its indicated replacement, GPT-6 Luna, at low effort. The first Claude fixture reached the composer but displayed a sign-in requirement with the private process home (`07-claude-first-open.jpg`). The retry reused the existing signed-in managed account through environment overrides on the disposable role process (`13-claude-retry-open.jpg`). Both retries used the same built candidate. Codex's only trust acceptance was for the new scratch Git root. In TFL-04 the overflow item appeared as `ui.choose_another_folder`; it opened the native **Choose folder…** dialog.

The app and private daemon inherited the absolute `RUNNER_RECORD_INPUT_FIXTURE` prefix `/private/tmp/r1452.F93bD4/private/record`. The recordings for both initial chats, both retries and the terminal fixtures are retained by session ID. `recorder-summary.json` indexes the passing chat recordings, hashes and 53-column prompt submissions. The left shell recording preserves the raw CRLF between the two TFL-03 paths. `screenshot-sha256.txt` lists the native captures and their hashes.

## Cleanup

Exited both owned terminal processes through native input. Stopped and archived the four owned agent chats through the absolute development CLI. Public session listing returned zero current sessions, and the four archived records each reported stopped status and an archive timestamp. See `25-owned-sessions-archived.jpg` and the session records under `private/`.

Quit and unregistered the run's wrapper, stopped its private development daemon, verified that its socket and scratch processes had exited, and released the QA lock. Removed the scratch repository's entries from the two agent trust stores. Parsed Codex config and the Claude projects map matched their prelaunch baselines (`trust-cleanup.json`). Preserved the scratch fixture repository, private development state with archived chats, owned test project and roles, and evidence for audit. This record is the only QA edit in the source worktree.
