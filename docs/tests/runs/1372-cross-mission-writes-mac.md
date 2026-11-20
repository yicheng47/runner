# #1372 cross-mission writes, macOS live QA

November 21, 2026, Asia/Shanghai. **All required live behavior checks passed. No remaining must-fix QA issues.**

## Candidate and environment

- Reviewed commit `8648909ac8876e80a95c0a0e3d4c42bcc1919f4e` on `fix/1372-cross-mission-writes`. The worktree resolved to `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1372-cross-mission-writes`. The candidate worktree was clean at build time. Runner handoff event `01M7WVM2VGWZSPXDY2805VF781`.
- macOS 26.6.2 (25G83), arm64. Runner 0.20.2 (dev), development CLI 0.20.2, pi 1.5.2 with `deepseek/deepseek-flash`, low effort, and default permissions.
- Built with `make run` under the QA lock with inherited mission variables and `NO_COLOR` cleared. Launched a temporary native wrapper with bundle ID `com.wycstudios.runner.qa1372`. The wrapper app and sidecar matched the built files byte for byte. App SHA-256 `6a059e3bcbdb10306ff70a7bd38244d2020417e86ba4cb6dc4f70db9fc553774`. Sidecar and development daemon SHA-256 `c071874dc7752fcaed06d8df6e25ddfda1953656f1d51fdd7172e85ad9833e53`.
- The development CLI reported `com.wycstudios.runner-dev/runnerd.sock` and outside-mission mode before fixture creation. Native Computer Use opened and dismissed New tab on the verified wrapper. `dev-status-wrapper.json`, `shasums.txt`, and `new-tab.png` record setup.
- Evidence root `/private/tmp/runner-1372-qa.1achRB/evidence/`. The development CLI started two unlinked single-member missions in separate canonical scratch folders. Runner will issue no reports to the implementation chat.

| Fixture | Mission ID | Handle and session ID |
|---|---|---|
| QA1372 A 1achRB (target) | `01M7WVV67ESP8JGDYMDPTS9VRX` | `probe-a`, `01M7WVV68YMG9P9J2C9FA46SND` |
| QA1372 B 1achRB (sender) | `01M7WVV6APHC0T189ATH2HW6WW` | `probe-b`, `01M7WVV6C97T5SXTBSSZV98R0R` |

## Live results

Submitted `./probe.sh 2>&1 | tee ../evidence/probe.txt` through B’s native pi terminal. The probe first verified that bare `runner` resolved to the verified development endpoint and reported B’s mission ID and `probe-b` identity. It then ran the commands below and recorded each exit code and terminal output. The explicit `ask --human` form was added because the fix covers that syntax and it was absent from the handoff’s live command list.

| Check | Result | Observation and evidence |
|---|---|---|
| Fixture preflight, first attempt | Failed | The QA script expected a direct sidecar path in `RUNNER_SESSION_CLI`. The session supplied a per-member shim. The guard exited before any scored command ran. `B-probe-preflight.png`, `probe-preflight.txt`, `A-after-preflight.json`. The scored run verified the endpoint, session mode and sidecar hash instead. |
| `runner msg post --mission <A> "hello …"` | Passed | Exit 2. The command named `QA1372 B 1achRB` and said posting to another mission is disallowed. `B-refusals.png`, `probe.txt`. |
| `runner msg post --mission <A> --as probe-a "hello …"` | Passed | Exit 2 with the same message. `B-refusals.png`, `probe.txt`. |
| `runner signal ask_human --mission <A> --payload '<prompt and choices>'` | Passed | Exit 2. The command named B and said signaling another mission is disallowed. `B-refusals.png`, `probe.txt`. |
| `runner ask --mission <A> "<prompt>"` | Passed | Exit 2. The command named B and said asking in another mission is disallowed. `B-refusals.png`, `probe.txt`. |
| `runner ask --mission <A> --human "<prompt>" --choices yes,no` | Passed | Exit 2 with the same message. `B-refusals.png`, `probe.txt`. |
| A receives no cross-mission event | Passed | Full CLI feeds with `--all` matched byte for byte before and after all five refusals. Both had 11 events and offset 3716. The native feed showed the same final READY message. `A-before.json`, `A-after-cross.json`, `A-before.png`, `A-after-cross.png`. |
| `runner msg post "hello …"` inside B | Passed | Exit 0. The marker appeared exactly once, from `probe-b`, event `01M7WVZNVXBKSH1B59CS1V4RA4`. `B-own-feed.png`, `B-after-probe.json`. |
| `runner msg post --mission <A> "hello …"` outside a mission | Passed | With inherited `RUNNER_*` values removed, the development CLI posted the marker exactly once as `human` (shown as **you**), event `01M7WW0XP1DZHW326Y8E27J8HM`. `A-off-bus-feed.png`, `A-after-off-bus.json`, `off-bus-post.json`. |

The live scope was macOS with pi. The coder’s automated checks separately cover explicit own-mission ID and prefix references, off-bus `--as`, and the mission-title lookup fallback.

## Cleanup

Stopped and archived both QA missions. Each has an archive timestamp and one stopped session. Both agent processes exited, and the missions disappeared from the active sidebar. See `A-cleanup.json`, `B-cleanup.json`, and `fixtures-archived.png`. Quit the temporary wrapper, stopped the development daemon started for this run, verified the owned processes exited, unregistered the wrapper, and released the QA lock. Retained the temporary role, archived conversation history, scripts and screenshots. The run changed no development settings or global agent configuration.
