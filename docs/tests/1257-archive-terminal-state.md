# #1257 — Archive terminal state: macOS live QA

## Candidate and authorization

Jason approved the codex trio on 2026-11-12. QA tested the clean, independently reviewed local candidate `c6e45f3b41d8b1f0f200a865144ccb5afe2389f0` on `fix/1257-archive-terminal-state`, based on `c2fbafd9b41979d5ff18e9f752672584baa4a955`, in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1257-archive-terminal-state`. The candidate remained unchanged through execution. The initial uncommitted diff was empty, SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The selected checks below passed on macOS with Codex. This is neither a full regression pass nor a Windows live pass. The reviewer named no additional live checks; deterministic worker, natural-exit, late-detach and lock-interleaving coverage remains separate automated evidence. The coder reported 2,078 daemon/terminal tests passed, one pre-existing skip, formatting and workspace/updater Clippy passed; QA did not rerun those checks.

## Environment and build identity

- Run: 2026-11-12, approximately 12:53–13:02 Asia/Shanghai, macOS 26.6.2 (25G83), Apple silicon.
- Toolchain: Rust/Cargo versions recorded in `evidence/candidate.json`; Codex CLI and native header both `0.167.2`; Runner app/CLI `0.19.7 (dev)` / `0.19.7`.
- Runtime: `gpt-6-luna`, explicit `low` effort, existing ChatGPT sign-in. Native footer confirmed GPT-6-Luna low. Existing development permission policy displayed YOLO mode; it was not changed. Existing runtime extensions remained enabled; the native TUI displayed two startup warnings without blocking the checks.
- Scratch/evidence root: `/private/tmp/1257-qa-tRxqFR` (canonical path). Test cwd: `/private/tmp/1257-qa-tRxqFR/codex`.
- Build: `make run` from the candidate, with `NO_COLOR` and implementation mission identity removed from child environments. The initial build app PID 29864 was terminated before launching the wrapper.
- Wrapper: `/private/tmp/1257-qa-tRxqFR/Runner 1257 QA.app`, bundle ID `com.wycstudios.runner.qa1257`, app PID 49416. App and CLI copies were verified byte-identical to this worktree’s `target/debug` outputs.
- App SHA-256: `d81a31c8c13d62c424c022eef538cbb100acc0423c5808299dbd27c16483b676`.
- CLI/daemon SHA-256: `7d46d1602be508cfd5c6282a808fa8f4a6cc2af3fbb13afe63cd5e473b716198`. The installed development CLI and daemon matched the copied candidate CLI.
- Endpoint: `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`; CLI status confirmed outside-mission mode. Product commands used the absolute development CLI with `RUNNER_SESSION_CLI` empty and `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG` removed.
- Daemon PID 43937, started 12:55:53, remained unchanged throughout both archive cycles and the ordinary control. Installed Runner was not operated.

Build, executable/hash and endpoint evidence: `build-launch.log`, `evidence/candidate.json`, `evidence/binaries.json`, `evidence/status.json`, and the `*-daemon-continuity.txt` captures beneath the scratch root.

## Plan and method

QA acquired `script/qa-lock` before the build and wrapper launch. The full-smoke procedure supplied identity, isolation, native control and cleanup rules; only the mission’s three checks were selected. First archive/restore tests the exact reported lifecycle and startup color consequence (relevant `TERM-RENDER-01` / `RT-LIFE-07` expectations). Repeating it after further output tests the next terminal generation. Ordinary stop/resume protects the neighboring preserved-model lifecycle.

Native computer use was verified by opening and dismissing New tab. The owned chat was created through the candidate development CLI in the canonical scratch directory, then all prompts and both archive/restore cycles were driven through the native UI. No private PTY writes or SQLite access were used. Runner archive used the owned sidebar row’s Archive action while its process was running; restore used Settings → Archived → Restore, then the owned row’s Resume button. Ordinary stop/resume used the documented development CLI commands followed by native input and observation.

The initial prompt taught `QA1257-TURQUOISE-ORBIT-83`, followed by completed SECOND-TURN and THIRD-TURN prompts before the first archive. After each resume QA asked “What marker did I ask you to remember? Reply with the marker only. Do not use tools.” without supplying the marker. A completed FOURTH-TURN prompt added output before the second archive. Each typed prompt was observed in the native composer and its completed response verified.

## Result matrix

All screenshot paths below are relative to `/private/tmp/1257-qa-tRxqFR/evidence/`.

| Check | Platform/runtime | Result | Observed outcome | Evidence |
| --- | --- | --- | --- | --- |
| Archive/restore after history | macOS / Codex 0.167.2 | Passed | Running chat archived, restored through Settings and resumed without daemon restart. Original conversation visible; same Runner ID/native key; blind recall correct. First resumed composer shaded normally; newly submitted recall prompt shaded normally; input usable and turn completed. | `02-before-archive.png`, `03-settings-archived.png`, `04-resume-first-observation.png`, `04-restored-history-composer.png`, `05-first-recall-and-colors.png`; `02-pre-archive.json`, `03-archived.json`, `04-resumed.json`, `05-first-recall.json` |
| Repeat archive/restore | macOS / Codex 0.167.2 | Passed | After additional output, second archive/restore again preserved identity, visible history and blind recall. First resumed screen had normal composer shading; the first new prompt had normal shading and completed with usable input. Same daemon throughout. | `06-before-second-archive.png`, `07-second-archived.png`, `08-second-resume-first-observation.png`, `08-second-restored-history.png`, `09-second-recall-and-colors.png`; matching `06`–`09` JSON captures |
| Ordinary stop/resume control (`RT-LIFE-07`) | macOS / Codex 0.167.2 | Passed | Stop exited PID 61918; resume created PID 65637. Same Runner ID/native key; visible history and correct blind recall; normal composer and new-prompt shading; input usable and completed turn. | `10-ordinary-stopped.png`, `11-ordinary-resumed-history.png`, `12-control-recall-and-colors.png`; `10-ordinary-stop.json`, `11-ordinary-resume.json`, `11-ordinary-processes.txt`, `12-control-recall.json` |

Runner ID throughout: `01M771V1VR9T2GKH4SMRM0M1Z3`. Native conversation key throughout: `01a1ce1d-8e3e-7772-a5d0-c8896ec40164`. Codex processes progressed from initial PID 51123 to first archive-resume PID 57577, second archive-resume PID 61918 and ordinary-resume PID 65637, all under the same candidate daemon. Each prior process was confirmed absent after its stop/archive. All three blind recall turns reported hook `ready` / `completed` in structured session captures and displayed the correct marker natively.

## Findings and limits

No failure or blocker was observed in the selected checks. Both resumed composers were shaded on the first captured native screen, before sending a new turn. Newly submitted prompts retained the same normal gray shading as the baseline; the reported missing-composer/bright-black-prompt symptom did not occur. Screenshots establish the live visual result; deterministic startup OSC timing is covered by the coder’s automated regression rather than inferred from a later eventual reply.

## Cleanup

QA stopped and archived only the owned chat; `13-cleanup-state.json` records stopped status and archive timestamp `2026-11-12T05:01:23.264767Z`. `13-cleanup-recents.png` shows its row absent from Recents. No roles or crews were created. Conversation history and scratch evidence were retained.

The authorized scratch-folder Codex trust entry was removed after cleanup. The configuration matched the before-run copy both semantically and byte-for-byte (`evidence/config-cleanup.diff` is empty). No theme, authentication or permission setting was changed.

The wrapper exited, the candidate development daemon was stopped, and the wrapper was unregistered from LaunchServices. `ps` confirmed app PIDs 29864/49416, daemon PID 43937 and all four owned Codex PIDs absent; process inventory and daemon-stop output are retained in `evidence/14-process-inventory.txt` and `evidence/14-daemon-stop.json`. The shared QA lock was released and verified free. Pre-existing development chats were not manually operated; saved agents resumed automatically during authorized development startup. Installed Runner and its mission sessions were untouched.
