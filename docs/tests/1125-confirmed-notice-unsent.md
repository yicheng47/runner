# #1125 confirmed inbox notice live QA

Verdict: **Passed for the selected required macOS live checks**, on the frozen reviewed candidate below. The exact original stale-history/composer-row alias was **not reproduced live**; it is not claimed as a passed reproduction. The in-process reproduction and automated gates are the coder’s separate evidence. This is feature QA, not a full regression pass.

## Scope, authorization and plan

Jason authorized the three-slot mission, bounded real-agent development-app checks, and this record. QA followed [full-smoke-test.md](full-smoke-test.md) for candidate identity, native control, isolation, evidence and cleanup, selecting the reviewer’s scoped [mission cases](regression/missions.md). No installed-app sessions, SQLite access, private PTY writes, product source edits, test expectation edits, QA commits or pushes were used.

The plan was recorded before execution in `/private/tmp/runner-1125-qa-q9jxvu17/plan.md`:

- Verify the exact copied app/CLI/daemon and development socket; prove native control by opening and dismissing New tab.
- Send several distinct messages to an idle Claude Code member while its pane stays hidden, then one more without opening it. Require a submitted notice, covering recipient read, ACK and completed turn for every marker, with no persistent tracker Unsent or router Unsent hold. Report the original screen-shape reproduction separately.
- Run MIS-MSG-01 for Codex Idle and Working: the idle notice delivers normally; a message posted during actual tool work stays queued until the hook reports Idle.
- Run the focused MIS-DRAFT-01 hold/clear subset for Claude and Codex: a real native human draft remains intact past typing grace, then clearing without Return releases exactly one submitted notice/read/ACK.
- Stop/archive only the owned mission, stop the owned app/daemon/helpers, remove only scratch trust entries, preserve evidence and verify candidate source identity.

Native Windows, pi, IME, ten-minute draft cap, rejected Return and unrelated lifecycle behavior were outside this selected scope.

The shared development environment was initially occupied by #897 QA. QA did not interrupt it. Jason answered to wait (`01M6DQ41GYR7Q25VT2S9H131AR`), then explicitly released it and requested this bounded run (`01M6DQ8GW1P0DSE2GAZS86TJ4S`). Before launch, process inventory showed no development app or daemon. The environment was released again at about 08:59 UTC, 2026-11-02.

## Candidate and environment

- Repository/worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1125-confirmed-notice-unsent`; branch `fix/1125-confirmed-notice-unsent`.
- Base: `51c91c7e1836c04b817a762ebd96cdd95dd8dd6d`; HEAD: `6fb218905ebf3c7a43eedc6a20092e88920e5b74`.
- SHA256 of `git diff HEAD`: `726901bffc54dbbc2d6168071b819f5ab89cb52e5feed9a2f2e29a0392e71aca`, matched the clean-review handoff before build, before launch and after live checks. The later addition of this record is documentation only.
- macOS 26.6.2 (25G83), arm64; Rust/Cargo 1.97.1; GNU Make 3.81; Claude Code 2.1.316; Codex CLI 0.166.0.
- Models: Claude Haiku 5.5 (`haiku`), low effort; Codex `gpt-6-luna`, low effort (inherited Fast). Roles were created with permission `default`; existing application/runtime permission configuration was preserved. Claude displayed inherited bypass permissions. Existing sign-ins and extensions were used without modification.
- QA built with `cargo build --locked -p runner-cli -p runner-app`, then normal `env -u NO_COLOR make run`. Both succeeded in the worktree’s own `target/`. The latter builds CLI and app separately; final copied identities below are from that normal build. No source edits or builds occurred during live execution.
- Wrapper: `/private/tmp/runner-1125-qa-q9jxvu17/Runner 1125 QA.app`; bundle ID `com.wycstudios.runner.qa1125`; wrapper PID `90942`. App SHA256: `81754acda5028c89701e3592ff8f175bd3f259f5e7812f1307242df3539b5236`.
- CLI/daemon SHA256: `8f501557a7613797fb9bcf56b63468c7ae27d5e2c7e533365ec023b0fffeac5c`; source sidecar, wrapper sidecar, installed development CLI and daemon were byte-identical. Public daemon status independently reported this hash and PID `87770`.
- Absolute development CLI: `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner`; public status reported `0.19.1 (dev)`, outside-mission mode and socket `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`.

All external product commands used that absolute CLI with `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE` and `RUNNER_EVENT_LOG` removed. Launch also omitted `NO_COLOR`. The ordinary input recorder used an absolute scratch prefix. The installed `runner` CLI was used only to coordinate QA with this implementation crew. Native control was bound to the distinct QA wrapper and verified by New tab, then dismissal; screenshots show terminal colors. Large native controls were driven from screenshots because the terminal has no useful accessibility text.

## Fixture and method

Canonical scratch/evidence root: `/private/tmp/runner-1125-qa-q9jxvu17/`. Relative paths below resolve there. `candidate.json`, `plan.md`, `evidence/binary-hashes.json`, `evidence/environment.txt`, timestamped `evidence/cli-*.json`, build logs and process inventories preserve identity and setup.

Owned mission `01M6DQCRKBGYXY43C51YN8064E`, title `QA1125 bounded delivery`, cwd `mission/`, used only two bounded fixtures:

| Handle | Role ID | Session ID |
| --- | --- | --- |
| q1125-cc | `01M6DQCJ229T523QHMKTXWZRBA` | `01M6DQCZXAP4MZGFNN0B5Z3VFZ` |
| q1125-cx | `01M6DQCJ363HA1ZEZ8S4A3S8D4` | `01M6DQCZYP186VRGE1ZPN4F50W` |

Each fixture replied briefly, then finished. On each inbox notice, the recipient itself ran the scratch `mission/q.py read` helper, which invoked the ordinary unfiltered default read twice and saved both outputs. The recipient ACKed its exact markers in its native terminal. QA never impersonated a recipient to mark a message read. Every marker’s second read was empty, and all helper commands returned zero. The bounded work helper sleeps for 25 seconds and records start/end times; no agent delegation or watchers were requested.

This host has no delivering background-watch facility. That limitation and the foreground `runner-dev mission feed 01M6DQCRKBGYXY43C51YN8064E --follow --json` follow-up were disclosed at test-mission creation. QA inspected public state during active execution and started no feed watcher. Default feed snapshots contain only the first event window; the complete owned append-only event log was copied after archive as `evidence/events-final.ndjson`, without opening SQLite. This complete log supplies the read IDs and state transitions below.

## Results

| Check | Classification | Evidence and reason |
| --- | --- | --- |
| Candidate identity and native control | Passed | Matching final binary hashes, public development endpoint/daemon hash, source diff unchanged, native New tab screenshot `evidence/01-new-tab.png`. |
| #1125 hidden-Claude repeated-turn sequence | Passed | Four separate markers each produced one Confirmed attempt, exact covering read/ACK and completed turn while Codex remained displayed. Claude was first opened after the final ACK had already completed. No router Unsent hold occurred. |
| Original stale-history/composer-row alias live reproduction | Skipped — not reproduced | Submitted notices remained in history with the prompt prefix, but the exact tracker-row alias and stale idle hold were not induced or proven. No live reproduction pass is claimed; use the coder’s failing-before/fixed-after in-process reproduction. |
| MIS-MSG-01 Codex Idle | Passed | One confirmed submitted notice, exact covering read and native ACK; second read empty. |
| MIS-MSG-01 Codex Working | Passed | Actual hook-reported tool work at post time; router held `Ready { idle: false }` until hook Idle, then submitted/read/ACKed once. |
| MIS-DRAFT-01 Codex hold/clear subset | Passed | Native human draft held for about 60 seconds after post, with no covering read; Ctrl-E/Ctrl-U without Return released one Confirmed notice and exact read/ACK. |
| MIS-DRAFT-01 Claude hold/clear subset | Passed | Native human draft held for about 35 seconds after post, with no covering read; Ctrl-E/Ctrl-U without Return released one Confirmed notice and exact read/ACK. |
| Info-level Unsent transitions | Passed | During hidden-Claude attempts, row-match Unsent transitioned to Idle with `ownership cleared` at confirmation; transient Unsent never outlived its attempt. Logs identify session and reason without screen contents. |
| Cleanup and candidate preservation | Passed | Owned mission stopped/archived, own wrapper and daemon stopped, scratch-only trust removed, wrapper unregistered, no owned processes survived, source hash unchanged. |

All selected required delivery checks passed. There were no failed delivery checks or unresolved required prerequisites after the environment handoff. The exact original live reproduction is explicitly separate from those checks.

### Exact delivery/read evidence

All times are UTC on 2026-11-02; every read below has `up_to` equal to the addressed event ID. `evidence/delivery-summary.json` and `evidence/events-final.ndjson` retain full timestamps. Recipient helper results are in `mission/evidence/*-read-*.json`.

| Marker suffix | Addressed event | Covering inbox_read | Posted → read UTC |
| --- | --- | --- | --- |
| HIDDEN-1 | `01M6DQH5D8Y73QEVCJEB0BH239` | `01M6DQH80GW1T6MR0S3ND8KRZA` | 08:55:33.544 → 08:55:36.208 |
| CODEX-IDLE | `01M6DQHFZ3K9X28ZJ8RT5KHB4Z` | `01M6DQHKTT1MZ4QV9NYCGE2TV2` | 08:55:44.355 → 08:55:48.314 |
| HIDDEN-2 | `01M6DQHG0VAQQ08F279HFZ37HT` | `01M6DQHJ0RJW6VQQJCHSCHJ6R6` | 08:55:44.411 → 08:55:46.456 |
| HIDDEN-3 | `01M6DQHWR2W7G47DMW96QRV7BX` | `01M6DQHYYY5E9G6E5P7H4500H8` | 08:55:57.442 → 08:55:59.710 |
| CODEX-WORKING | `01M6DQK04FT8Y54C22XGJFB2DY` | `01M6DQK7RY4ZT8JVYVDP4S2MT2` | 08:56:33.679 → 08:56:41.502 |
| HIDDEN-4-FINAL | `01M6DQK05F09C7TNC074J98HJ6` | `01M6DQK2QRVH6494C8G93TS3AS` | 08:56:33.711 → 08:56:36.344 |
| CODEX-DRAFT-CLEAR | `01M6DQMAYX6Q5TWWB3BC7DBFTH` | `01M6DQPAB88BZW8EY9KKFDFNJG` | 08:57:17.533 → 08:58:22.440 |
| CLAUDE-DRAFT-CLEAR | `01M6DQNRSFVA70ZV4NVSBJ1G1J` | `01M6DQPWSD2BPBQ4YMZW5CQXJX` | 08:58:04.463 → 08:58:41.325 |

### Hidden Claude and logging

While Codex stayed selected, Claude’s HIDDEN-1, HIDDEN-2, HIDDEN-3 and HIDDEN-4-FINAL messages were posted only after its previous turn completed. Read times were 08:55:36.208, 08:55:46.456, 08:55:59.710 and 08:56:36.344; the final turn completed at 08:56:37.620, well before opening Claude at about 08:57:46. `evidence/04-hidden-claude-codex-idle.png` and `evidence/06-codex-working-queued.png` show the other pane selected. `evidence/10-claude-hidden-final-ack.png` shows the third/final ACKs and empty Claude composer after the sequence. Claude’s raw recorded output preserves the earlier ACKs too.

`evidence/router-hidden.log` and `evidence/router-final.log` show no `router delivery held … reason=Unsent` for either session. For example, hidden-final logged row-match Unsent at 08:56:33.814951, Confirmed at 08:56:35.310215, and `stopped reporting unsent … reason=ownership cleared state=Idle` at 08:56:35.310248. Its tracker Unsent lasted only within the attempt. Each of the four hidden messages had one Confirmed attempt; Claude had six total including startup and draft-clear. Codex had four including startup, idle, working and draft-clear. `evidence/routing-proof.json` records those counts.

### Codex Working and native drafts

Codex’s work helper started at 08:56:26.585. At the 08:56:33.679 post, public state was Working with hook detail `using_tools`. The router held the message at 08:56:33.690668, then released only after hook Idle at 08:56:38.350963. Confirmation followed at 08:56:39.941181 and read at 08:56:41.502474. The bounded helper had yielded a background terminal in Codex’s tool UI, so the agent finished its turn before the helper’s 08:56:51.588 end. This check proves the documented idle-only routing boundary, not queuing until an already-finished turn’s helper exits. The helper completed naturally and none survived cleanup. Screenshots `evidence/05-codex-work-submit.png`, `06-codex-working-queued.png` and `07-codex-working-ack.png` preserve the native sequence.

Codex’s human draft stayed visible with Inbox waiting from the 08:57:17.533 post through native clear at about 08:58:17. The log held Drafting at 08:57:17.562487 and released at 08:58:17.608543, with one Confirmed attempt at 08:58:19.196357. Claude’s post at 08:58:04.463 stayed held Drafting until clear/release at 08:58:39.339507, then Confirmed at 08:58:40.927624. Neither marker had a covering read before clear. Screenshots `evidence/08-codex-draft-entered.png`, `12-codex-draft-held-before-clear.png`, `13-codex-draft-cleared.png`, `17-codex-clear-ack.png`, `11-claude-draft-entered.png`, `14-claude-draft-held.png`, `15-claude-draft-cleared.png` and `16-claude-clear-ack.png` show intact drafts, hold banners, empty composers and final ACKs. No Return was used for either clear.

## Preserved setup observations and limits

Codex’s first startup failed at 08:53:32 with exit 1: `account/read failed during TUI bootstrap: … workspace routing discovery timed out (code -32603)`. QA made one native Resume, which succeeded at 08:55:02 without account/configuration changes. The first failure remains in `evidence/02-codex-startup-error.png` and `evidence/codex-startup-output.txt`; subsequent delivery checks passed on that resumed session. No regression attribution to this patch is made. The Codex input recorder retained only the first failed startup; Codex delivery evidence therefore uses native screenshots, public state, recipient reads and the complete mission event log, rather than claiming a resumed raw recording.

One work-prompt typing attempt had delayed/unobserved native echo; a subsequent paste caused duplicated prompt text. QA submitted the visible bounded request once, and the fixture ran exactly one finite work helper. This was a QA input sequencing deviation, preserved in `evidence/05-codex-work-submit.png`; it did not affect marker content or the measured Working→Idle delivery boundary. Later draft inputs used native paste and were verified before posting. A user window resize interrupted one computer-use action; QA refreshed the screenshot and recalculated coordinates before continuing. No check is credited from an unverified click or command success alone.

The original 64-minute incident’s exact cursor/history alias was not reproduced live, so these checks do not establish the original runtime trigger or a 64-minute soak result. They establish prompt delivery without opening the Claude pane, confirmation ownership cleanup, idle-only Codex delivery and retained person-draft protection on this candidate. No synthetic fixture was substituted for a real native check.

## Cleanup

Owned test mission `01M6DQCRKBGYXY43C51YN8064E` was stopped and archived at 08:59:09.095 UTC. Claude PID `94564`, initial Codex PID `94569`, resumed Codex PID `2658`, initial bare app PID `85490`, wrapper PID `90942` and candidate daemon PID `87770` were all absent in the final process inventory. Native Quit was interrupted by an external window change, so QA sent SIGTERM only to its verified wrapper, then used the public development `daemon stop`. The wrapper was unregistered with LaunchServices and was not queried again afterward. No feed watchers or persistent helpers were started. The installed Runner and its daemon remained untouched.

Codex’s own scratch-folder trust entry was removed; the entire config matched the prelaunch capture afterward (`evidence/codex-cleanup.diff`, empty). QA initially captured the default Claude profile, then found that the existing runtime used the work profile. This evidence-capture correction is preserved: the active Claude profile’s automatic pre-trust backup from 08:54:02.811 contained no scratch key and supplied the comparison baseline. Cleanup removed only this run’s canonical `mission/` project entry; the resulting projects map exactly matched that recovered pre-trust snapshot. `evidence/claude-work-cleanup-proof.json` and `claude-work-scratch-cleanup.diff` document this, without restoring unrelated keys or changing authentication. The default Claude profile’s project map also remained unchanged.

The two QA role definitions remain in development data because role deletion was not authorized. No crews were created. The wrapper, bounded scratch files, conversation history and raw evidence were retained. No branches, worktrees or evidence files were removed. Final `git diff HEAD` still matched the frozen hash before writing this record. Automated test/Clippy/fmt and CI status belong to the coder’s validation and PR; QA did not rerun or represent them as live evidence.
