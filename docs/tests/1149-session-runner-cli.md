# #1149 — Session CLI routing: focused macOS live QA

Run: 2026-11-04, approximately 03:56–04:05 UTC. Overall: **Incomplete — rollout prerequisite**. Functional checks 1(b), 2(b), 3 and 4 passed on the fixed candidate. Literal checks 1(a) and 2(a) remain **Blocked** because the installed Runner 0.19.2 CLI predates #1149. Keep `needs-live-test`; the staged fixture does not establish a pass for the installed CLI.

Scope decision (Jason, 2026-11-04, after this run): 1(b) and 2(b) stand as this PR's pass for checks 1 and 2, and `needs-live-test` was removed. 1(a) and 2(a) cannot pass before a release because the hand-off runs in the CLI the shell finds. Follow-up once the first release carrying #1149 is installed: in an owned dev Codex chat and an owned dev Codex mission slot, rerun the 1(a) and 2(a) probes (`command -v runner`, `runner status --json`, `runner mission list --json`). Each must reach the development daemon even though the command resolves to `~/.local/bin/runner`.

## Candidate and environment

| Item | Recorded identity |
| --- | --- |
| Repository/worktree | `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1149-session-runner-cli` |
| PR / branch | [#1153](https://github.com/yicheng47/runner/pull/1153) / `fix/1149-session-runner-cli` |
| Tested commit | `a1221ecbb984ef6da640798e2e0d73021dea4ecd` |
| Candidate parent/base | `976816c58e71ef5c0d1aa72979b62f17d1a7ace1` |
| Initial uncommitted diff | Empty; SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| OS | macOS 26.6.2 (25G83), arm64 |
| Toolchain | rustc 1.97.1; cargo 1.97.1; zsh 5.9 |
| Agent CLIs | Codex 0.166.0; Claude Code 2.1.318 |
| Models / effort | Codex `gpt-6-luna`, low; Claude Code `haiku` (displayed Haiku 5.5), low |
| App build | `make run` from this worktree, development build 0.19.2 (dev) |
| Dev endpoint | `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock` |
| Scratch/evidence root | `/private/tmp/runner-1149-vd7a1cse`; evidence filenames below are relative to `evidence/` there |

The worktree and PR head were checked again after the live checks and remained at the tested SHA, with no product changes. This record was then added uncommitted for the coder. Earlier results would require rerunning if the candidate's product code changed.

The initial `make run` executable was stopped, then byte-identical app and CLI copies were launched through `/private/tmp/runner-1149-vd7a1cse/Runner 1149 QA.app` with bundle ID `com.wycstudios.runner.qa1149`, following the documented macOS computer-use wrapper procedure. No development app or daemon was running before this run. Native control was verified by opening and dismissing New tab; all prompts were submitted through native computer use into the owned agents' visible terminals. The screenshots show their actual rendered results. CLI requests independently confirmed the mission link, event identity and cleanup. No SQLite database or private PTY input was used.

| Binary | SHA-256 |
| --- | --- |
| Worktree `target/debug/Runner` and wrapper app | `a5c120df5c8616e44315d658fddf33957d19fb546b9fa19c6b9f8d750dbd395c` |
| Worktree debug CLI, wrapper sidecar, installed dev `bin/runner` and `bin/runnerd` | `6322dfab67673a64614478db02863bed7b019c05b72b7fee5c7cbd8b09a89dec` |
| Worktree release CLI and staged scratch `bin/runner` | `8e8131fcfd5336f8a1e1827043a62151aadc0f1d43ce0b8a44f579361e20a708` |

The release fixture was built with `cargo build --release --locked -p runner-cli --bin runner-agent-cli` using this worktree's target directory and copied only to scratch `bin/runner`. With session variables removed, its read-only mission list included the installed coordination mission; from the dev sessions it instead returned dev missions. Production binaries and command links were not replaced.

## Authorization, plan and method

The mission operator explicitly assigned these four focused checks on this commit, including the staged release CLI fixture and owned test agents, after code review. The plan was recorded before launch in scratch `plan.md`. Scope follows `docs/tests/full-smoke-test.md`, `docs/arch/arch.md` §9.4/§9.6, and CLI-READ-01, MIS-LINK-01 and CLI-IDENT-01 subsets. This was not a full smoke or a full regression run. Windows and Linux live checks were explicitly outside this assignment; Windows automated regression remains separate CI evidence.

The task originally required `--version` to report the development build. Actual `--version` prints `runner 0.19.2` in both builds. Before any test-object creation, the operator corrected the task through Runner: use `status --json` showing `app_version: "0.19.2 (dev)"` and the dev socket, the candidate sidecar hash, and matching dev mission IDs. Those guards were checked through each agent's own shell before each staged-CLI write. The stub control still used `--version` because its marker and exit code were the evidence.

Development app launches and external dev CLI commands omitted the implementation mission's `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE` and `RUNNER_EVENT_LOG` (also session/hook routing variables where applicable). External product requests used the absolute development sidecar with session routing cleared. Installed Runner coordinated this QA role; its product probes stayed read-only. No shell startup file, authentication, global permission mode or development setting was changed. App-managed skill refresh and the documented temporary trust entries were the expected setup effects.

## Result matrix

| Check | Required proof / reason | Result | Evidence |
| --- | --- | --- | --- |
| 1(a), literal Codex chat | Login-shell PATH boundary; installed command must reach dev after rollout | **Blocked** — `command -v runner` was `~/.local/bin/runner`; installed 0.19.2 listed installed missions despite `RUNNER_SESSION_CLI` naming dev `bin/runner`. This is the documented rollout prerequisite, not a candidate regression. | `01-codex-probes.txt`, `01-codex-routing.png` |
| 1(b), staged CLI in Codex chat | Updated release CLI follows session target and preserves the chat-to-mission link | **Passed** — guarded staged command created dev mission `01M6JBEREE6KV7DHSP0P6NHJNJ`, title `qa 1149`. `STARTED BY` and `LINKED TO` both identify owned chat `01M6JBBCDFK7C8F38Y08TP7RT8`, reports on. UI shows it grouped beneath that chat. | `01-codex-create.raw.txt`, `01-mission-show.json`, `01-linked-mission.png` |
| 2(a), literal Codex slot | Login-shell resolution and exported slot-specific shim | **Blocked** — bare command still resolves to installed 0.19.2; `RUNNER_SESSION_CLI` correctly names dev `missions/01M6JBEREE6KV7DHSP0P6NHJNJ/shims/qa1149-slot/bin/runner`. No post was attempted through the old installed CLI. | `02-slot-probes.txt`, `02-slot-routing.png` |
| 2(b), staged CLI in Codex slot | Hand-off restores shim identity and posts once into the correct mission | **Passed** — staged status identifies the dev socket and `inside mission … as @qa1149-slot`; guarded `runner msg post "qa-1149 slot probe"` produced exactly one matching event, authored by `qa1149-slot`, in the dev mission. | `02-slot-post.raw.txt`, `02-probe-events.json`, `02-feed.json`, `02-slot-post.png`, `02-feed-probe.png` |
| 3, Claude Code chat | Runtime that already preserves PATH continues reaching dev | **Passed** — bare command resolves to dev `bin/runner`; status reports dev app/socket, mission IDs match dev data, and the exported target's hash matches the candidate. | `03-claude-probes.txt`, `03-claude-routing.png` |
| 4, development skill opt-out | Rendered skill clears routing only for its command, preserving parent session routing | **Passed** — refreshed installed skill contains `RUNNER_SESSION_CLI= "<dev sidecar>"` forms. Stub control prints `QA1149_STUB_CONTROL` and exits 42; the child-shell opt-out returns dev mission data with exit 0 and no stub marker. Parent target remains the original dev sidecar; subsequent bare `runner status --json` reports dev. | `04-installed-skill.txt`, `04-skill-optout.txt`, `04-skill-optout.png` |

Dev baseline mission IDs were compared without modifying the pre-existing baseline mission. The new mission was subsequently verified through the development sidecar. The slot's exact probe event is `01M6JBJK1Y2NZ46ZGQQ77HZ5N8`, timestamp `2026-11-04T04:02:49.534739Z`, mission `01M6JBEREE6KV7DHSP0P6NHJNJ`, author `qa1149-slot`. Feed selection included all events in this small owned mission; the exact-text match count was one.

## First attempts and limitations

- The first Codex read-only probe could not access either daemon socket inside its command sandbox. Preserve that first result in `01-codex-probes-sandbox.txt` and `01-codex-sandbox-first.png`. The same probe succeeded with authorized per-command escalation, without changing a permission setting. This was a fixture restriction; it did not establish a routing failure.
- The test slot automatically ran bare `runner mission show`, `runner --help` and `runner msg read` before the prescribed probes. The installed CLI returned `mission not found` for the dev mission lookup; `msg read` succeeded using the inherited dev event-log path and acknowledged only the owned dev inbox. These calls exceeded the task's permitted installed-CLI command list; the inbox acknowledgement is an extra owned-dev mutation, so this run does not claim strict procedure compliance. No installed mission was mutated. Subsequent instructions prohibited further bare calls, including inbox reads, and the only message post used the guarded staged CLI. Exact commands/results are retained in `02-slot-command-records.private.json` and the dev feed capture.
- The Codex chat wrote `01-codex-create.txt` as a reconstructed summary after executing the real command. Use the actual tool result preserved as `01-codex-create.raw.txt`, its native screenshot, and `01-codex-command-records.private.json` for audit. Owned Codex transcripts are retained locally as supplemental evidence. Raw command results take precedence over agents' prose summaries.
- Claude's default config was captured before launch, but its existing work account was identified only afterwards. Its active account had no prelaunch snapshot in this run, so preservation of its entire project map across startup is not proven. Cleanup removed only the exact owned scratch-root entry, with semantic equality of every other current value verified. This limits configuration-preservation evidence, not the routing results.
- Automated suite results are recorded by the implementer in the PR. This QA run built both fixtures and did not rerun those suites. At the closing CI observation, macOS and both Linux jobs passed; Windows was still pending. This record does not claim a green Windows CI result or any Windows/Linux live result.

## Owned objects and cleanup

| Object | ID / identity | Final state |
| --- | --- | --- |
| Codex direct chat | `01M6JBBCDFK7C8F38Y08TP7RT8`; scratch `codex/` | Stopped and archived |
| Claude Code direct chat | `01M6JBCGY6CC5R0FYT5XQ5WPV0`; scratch `claude-code/` | Stopped and archived |
| Test mission | `01M6JBEREE6KV7DHSP0P6NHJNJ` | Stopped, then archived/completed; zero live sessions |
| Codex slot | `01M6JBGRWSXQMPKXJ5X452DA91`; handle `qa1149-slot` | Stopped with mission |
| Test role/crew definitions | None created; existing `coder` role used with model/runtime overrides | Unchanged |
| Initial app / wrapper / dev daemon | PIDs 91092 / 97876 / 93106 | Exited; daemon explicitly stopped |

The wrapper was quit and unregistered from LaunchServices. Final `ps` showed no QA wrapper or dev daemon, and the recorded slot PID 11549 was absent. All foreground build/launch commands finished; no watcher or background helper was left running. Installed Runner and its daemon remained running and were not stopped.

Cleanup removed only this run's Codex `projects.<scratch>/codex` and Claude `projects.<scratch>/claude-code` trust entries. Codex's other project entries matched its prelaunch snapshot; the active Claude account's other current values were preserved. No roles, crews, branches, worktrees or evidence files were deleted. Scratch evidence, wrapper binaries and conversation history remain. See `ledger.json`, `cleanup.txt`, `cleanup-*.json`, `trust-cleanup.json`, and `processes-after.txt`.

Identity/setup evidence is in `candidate.txt`, `environment.txt`, `binary-hashes.txt`, `dev-status.json`, `release-build.log` and `00-native-control.png`. Private account/config snapshots and installed-mission captures remain only in the scratch evidence directory and must not be committed. The public record contains only task-relevant results and disposable test identities.
