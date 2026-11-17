# #1356 / PR #1367 — macOS tab-cell rendering QA

## 2026-11-17 — candidate 882d015d

**Verdict: Passed — all three selected live checks passed.** This is scoped terminal-renderer QA, not a full regression pass. Authorized by the #1367 live-QA mission handoff (mission `01M7KWXA1JVBB2WX1HX2RQH7V4`); no product code, tests, settings, commits or pushes changed.

### Candidate and environment

- Repository/worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1356-tab-cell-render`; branch `fix/1356-tab-cell-render`.
- Candidate: `882d015ddf7d5c17918a6c6e60275fdb0a792f83`; parent/base: `67f5f01e0cb52f8302c6b69c7f22644fe9fdcd00`. Source was clean before build and through the live checks. Empty uncommitted diff SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. This record is the only subsequent worktree addition.
- macOS 26.6.2 (25G83), arm64; zsh 5.9 (arm64-apple-darwin25.0). Rust 1.97.1, Cargo 1.97.1, GitHub CLI 2.97.0; app/development CLI 0.20.0 (dev). No agent runtime was started for these checks; model, effort and agent permission/extension configuration are not applicable.
- Built using `make run` in this worktree, omitting `NO_COLOR` and the inherited mission/session identity variables. The initial app PID was 27336; the candidate development daemon was PID 31090. The standard development namespace had no app/daemon at baseline; installed Runner and an unrelated private development daemon were left untouched.
- Temporary wrapper: `/private/tmp/1367-qa-uo1ma4za/Runner 1367 QA.app`, bundle ID `com.wycstudios.runner.qa1367`. App SHA256, matching `target/debug/Runner`: `611e0771814312c1b3ee974663c2875bc069c086e016ea70b70051176024c6d7`. Sidecar SHA256, matching `target/debug/runner-agent-cli`, the wrapper CLI, and the installed development `runner`/`runnerd`: `38ec36a93c9fe5efa71911edc3c94048e7b8215c3db5a279005106592e94df42`.
- The absolute development CLI reported `0.20.0 (dev)`, `outside a mission`, and `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`. Wrapper PID 54627 ran from the above path. Identity evidence: `evidence/hashes.json`, `status.json`, `processes-running.txt`, `source.txt`, `diff-sha256.txt`, `os.txt`, `tools.txt`.
- Automated evidence is separate: existing macOS, Windows and both Linux CI jobs passed for this PR, verified before live testing; see [CI run](https://github.com/yicheng47/runner/actions/runs/44012231154). No automated tests were rerun by QA.

### Plan and method

Followed `docs/tests/full-smoke-test.md` candidate identity, native-control, isolation and cleanup rules and the regression suite's feature-scope guidance. Required platform/runtime: macOS / zsh only. The three task-specific cases cover the plain ASCII span, colored decoration and combining-mark paths changed by the fix; unchanged agent lifecycle, IME, selection and other terminal cases were outside scope. The plan was written before execution at `/private/tmp/1367-qa-uo1ma4za/plan.md`.

Held the shared QA lock throughout building and live use. Native computer use bound to the wrapper; opening New tab visibly verified control. All test commands were pasted through the native terminal and submitted with Return, never through private PTY writes. One owned zsh tab (`01M7KX41V204JGDC7R6MBD0Y44`, shell PID 56949) was created. It initially inherited the launcher's cwd; native `cd` moved it to `/private/tmp/1367-qa-uo1ma4za/shell`, verified using `pwd -P` and the visible title. The shell's local `PS1`/`RPS1` were simplified for evidence; no saved development setting or global configuration was changed. Native colors were visible. Structured session identity and shell cwd/history were captured alongside screenshots.

### Results

Evidence root: `/private/tmp/1367-qa-uo1ma4za/evidence/`. Each zoom is a crop enlarged 5× from its full native screenshot; the originals are retained.

| Case | Platform / runtime | Result | Observation and evidence |
| --- | --- | --- | --- |
| TAB-01: surviving plain tab | mac / zsh | Passed | Output is `ab cdefg`; the next line from plain `echo 'ab cdefg'` aligns column for column, with `cdefg` beginning after the single blank cell. `01-plain-full.png`, `01-plain-zoom.png`. |
| TAB-02: colored underline | mac / zsh | Passed | The red underline covers only the blank cell between `b` and `c`; all letters align with the reference rows above. `02-underline-full.png`, `02-underline-zoom.png`. |
| TAB-03: combining acute | mac / zsh | Passed | Acute accent sits above the blank cell between `b` and `c`; `cdefg` remains aligned with the preceding rows. `03-accent-full.png`, `03-accent-zoom.png`. |

Executed commands:

```sh
printf 'ab\t\rab\033[Ccdefg\n'
echo 'ab cdefg'
printf 'ab\033[4;58;5;1m \033[0m\r\033[2C\t\r\033[3Ccdefg\n'
printf 'ab\t\rab\033[C\xcc\x81cdefg\n'
```

No selected check failed or needed a retry; no candidate/source change occurred. No baseline comparison was run, so this record establishes candidate behavior without independently attributing the original defect.

### Setup deviations and cleanup

The native macOS folder picker did not reliably advance under computer use. Before any selected check, only the run-owned wrapper was stopped and relaunched byte-identically; the shell was then created normally and changed directory through native input. This did not block a rendering check. Evidence preparation initially found Pillow unavailable; the crops were produced with built-in `sips` instead. These are tooling/setup incidents, not rendering failures.

`session archive` correctly refused the owned shell because terminals close rather than archive. The owned terminal was closed using its native pane close button, visibly disappeared, and its CLI lookup then returned `session not found`; shell PID 56949 exited. No agent chats, test missions, roles or crews were created. Existing saved development sessions resumed automatically with daemon startup under the documented standing authorization and were not typed into, archived or otherwise operated.

Quit the wrapper through its native Quit dialog with “Keep them running”, then stopped only this run's development daemon using the absolute development CLI (`{"stopped":true}`). Unregistered the wrapper from LaunchServices and confirmed no run-owned app, daemon or shell PID survived. The installed app and unrelated private daemon remained intact. Released the QA lock. Scratch artifacts, wrapper and evidence are retained; no files, branches or worktrees were removed. Ledger: `/private/tmp/1367-qa-uo1ma4za/ledger.json`; cleanup evidence: `shell-final.json`, `daemon-stop.json`, `cleanup.txt`.
