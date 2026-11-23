# #1433 — Terminal locale and Git pager live QA

## 2026-11-23

**Passed on macOS. Blocked on native Windows.** Runner's development app supplied `LC_CTYPE=UTF-8` after a locale-free launch, and its integrated terminal rendered `样式` correctly in both Git pagers. Native Windows remains a required check, so the PR retains `needs-live-test`.

### Candidate and setup

- Tested `40fd9f28832b0467ceafccea62c5208126451da4` on `fix/1433-windows-lc-ctype-locale` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1433-windows-lc-ctype-locale`. The worktree had no uncommitted diff when built or tested. The QA record was added afterward.
- macOS 26.6.2 (25G83), arm64, Runner and CLI 0.20.3 development build, zsh 5.9, Apple Git 2.50.1, and system `less` 668. The run used one shell and no agent accounts.
- Acquired `script/qa-lock acquire --timeout 540` and ran `make run` from the candidate worktree. The Cargo launch wrapper used `env -i` with an allowlist and a blank private `HOME` so the app and daemon started without `LANG`, `LC_ALL`, `LC_CTYPE` or pager overrides. Relaunched byte-identical build binaries in the native QA app wrapper. The development CLI confirmed the private `com.wycstudios.runner-dev/runnerd.sock` and the daemon binary matched the built sidecar. Evidence includes `source.txt`, `build-and-bare-launch.txt`, `binary-hashes.txt`, `status.json`, `daemon.json` and `process-launch-environment.txt`.
- Opened a fresh terminal with `⌘T` through native computer use, then changed directory to the fixture at `/private/tmp/r1433.6xdav6/repo`. The fixture contained branch `feature_样式` and one commit, `Runner QA 1433 样式`. Coverage was limited to the affected locale fallback and the pager portion of [TERM-RENDER-02](../regression/terminal.md).

### Live results

Evidence directory is `/private/tmp/r1433.6xdav6/evidence/`.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| macOS locale after a locale-free app and daemon launch | Passed | The integrated shell had `LC_CTYPE=UTF-8` and `locale charmap` returned `UTF-8`. `LC_ALL` and `LESSCHARSET` were absent. The host's `/etc/zprofile` adds `LANG=C.UTF-8` during login-shell startup. A control login zsh launched with the same environment supplied `LANG=C.UTF-8` alone. See `02-native-locale.png`, `native-env.txt`, `system-zprofile-locale.txt` and `zsh-startup-locale.txt`. |
| macOS `git branch` and `git log` through `less` | Passed | The branch and commit subject displayed the exact Chinese characters without `<XX>` byte escapes or mojibake. Both pagers opened interactively, and the shell accepted a marker command after exiting them. See `03-git-branch-less.png`, `04-git-log-less.png`, `05-pager-exited.png` and `native-pager-processes.txt`. |
| Native Windows integrated `pwsh` with Git for Windows MSYS2 `less` | Blocked | This run had a macOS host only. On Windows, launch Runner with `LC_CTYPE` and `LESSCHARSET` unset, confirm `$env:LC_CTYPE` is empty in a new integrated `pwsh` terminal, then verify `git branch` and `git log` display `样式` through `less`. Windows screenshot evidence remains pending. |

The pager commands kept short output open with `LESS=R` and forced the system pager without overriding its charset.

```sh
LESS=R git -c core.pager=/usr/bin/less --paginate branch
LESS=R git -c core.pager=/usr/bin/less --paginate log -1 --oneline --decorate
printf '%s\n' RUNNER_QA_1433_PAGER_EXIT_OK
```

### Cleanup

Closed the sole QA terminal (`01M843VAM6G6AD3GVYEPYTPW56`) through its pane close button. The private CLI session list returned to `[]`, and the shell PID exited. Quit the app, stopped its private daemon, unregistered the temporary bundle, moved the test repo, bundle and private home to Trash, and released the QA lock. Evidence retained includes `06-terminal-closed.png`, `session-after.json`, `daemon-stop.json`, `processes-final.txt`, `fixture-cleanup.txt` and `lock-release.txt`. Existing Runner sessions were isolated by the private home.
