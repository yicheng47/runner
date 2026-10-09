# Full smoke test procedure

Use this procedure when Jason asks for a complete live smoke test before a major refactor or important release. Select the cases from the [live regression suite](regression/README.md): active `smoke` rows for release smoke, or all active rows for a complete regression after a major refactor. Its runtime cases exercise every accessible runtime through real Runner terminals: Codex, Claude Code, Antigravity CLI, Copilot and pi. Test TRAE when access is available; otherwise record it as Skipped with the access limitation. Add feature-specific checks when the change affects them. List the required runtimes and platforms before starting; an unavailable required account, CLI or machine is a blocked check, not a pass. The suite includes terminal, workspace, mission, daemon, Settings and CLI cases as well as runtimes. Installation/update and other specialized checks need their stated authorized fixtures. Automated CI remains separate evidence.

Ordinary feature QA follows [Test Scope](../../AGENTS.md#test-scope): test changed behavior and directly affected regressions, with broader coverage for major changes or an explicit request. Feature plans may reuse this procedure's candidate identity, isolation, native control, evidence and cleanup sections without adopting its full runtime or mission matrix. Apply only setup steps needed by the selected checks and authorized fixtures; a shell-only layout check does not require agent accounts or runtime startup missions.

The first run is recorded in [#777 runtime adapter smoke](archive/777-runtime-adapter-smoke.md). Its findings informed the canonical-path setup and use of a single lead slot for startup checks below.

## Scope and setup

1. Confirm the live-test request covers using the existing agent accounts and writing test conversation history. Continue under authorization already given in the session. Without that authorization, prepare the checklist and automated evidence only. Do not change authentication, global agent configuration or permission settings to make a test pass.
2. Check repository status and branch, and record the candidate commit, dirty diff if any, app build, OS and CLI versions. Build from the authorized checkout. Run the normal relevant tests and CI first, and keep that evidence separate from live results.
3. Start the development app with `env -u NO_COLOR make run` on macOS or `.\make.cmd run` on Windows with `NO_COLOR` omitted from the child process environment. Agent hosts can set `NO_COLOR=1`; inheriting it makes child agent terminals monochrome even with `TERM=xterm-256color` and `COLORTERM=truecolor`. Remove it only for this launch, preserve global settings, and verify colors in a native terminal screenshot. Identify the exact app executable/window before sending input. Use the development CLI from its absolute path; a bare `runner` can address the installed app. Verify `status --json` reports the development namespace/daemon endpoint. Never open Runner's SQLite database directly or operate pre-existing chats or missions.
4. Bind computer use to the development app, then verify a harmless action such as opening and dismissing New chat. If native app control is unavailable, report that limitation and continue only the checks the CLI can prove. A CLI-created chat is not proof that its terminal rendered or accepted input.
5. Create a new scratch root and an evidence directory outside the repository. Use a different child directory for each runtime and each startup mission. Resolve paths before passing `--cwd`: on macOS `/tmp` resolves to `/private/tmp`, and different spellings can break conversation lookup. Keep a ledger of every test session, mission, role and crew ID.
6. Discover supported commands and model/effort choices through the version-matched CLI guide and `--help`. Choose an inexpensive available model and explicit effort where supported. Record existing permission mode and user extensions as test conditions; do not silently disable them.

### Agent configuration isolation

Private Runner app data and conversation directories do not isolate an agent's configuration. Audit the installed runtime's complete startup path, including migrations, changelog bookkeeping, settings saves, trust decisions and extensions, before reusing an account directory. Hash-only canaries detect a write after it happens; they cannot make a writable global directory safe. If supported account reuse cannot preserve real configuration, record the affected runtime as Blocked and request the exact private-account prerequisite.

For pi, set child-only `PI_CODING_AGENT_DIR` to a canonical private directory beneath the smoke scratch root, and keep `PI_CODING_AGENT_SESSION_DIR` private as well. Verify both resolved paths before launch and on resume. Never point `PI_CODING_AGENT_DIR` at the real `~/.pi/agent` merely to reuse authentication: pi 0.87.1 can save `lastChangelogVersion` in that directory's `settings.json` during interactive startup, and migrations can write before the first-time setup guard. An explicit model/thinking choice, `--approve`, a private session directory or suppressing setup does not prevent those writes. Do not copy credentials or change global authentication to bypass this prerequisite; use an already authorized private account or leave account-backed pi checks Blocked. Keep native startup results separate from configuration-preservation results. See the [2026-10-06 Windows regression record](archive/2026-10-06-pc-full-regression.md) for the failed global-state canary that established this boundary.

Example macOS setup, after verifying the development app:

```sh
smoke_cli="$HOME/Library/Application Support/com.wycstudios.runner-dev/bin/runner"
"$smoke_cli" status --json
"$smoke_cli" help agents
smoke_root=$(mktemp -d /tmp/runner-smoke.XXXXXX)
smoke_root=$(cd "$smoke_root" && pwd -P)
mkdir "$smoke_root/codex" "$smoke_root/claude-code" "$smoke_root/antigravity" "$smoke_root/copilot" "$smoke_root/pi"
```

On Windows, resolve the development CLI and scratch paths on that machine, use native PowerShell commands with exit-code checks, and follow [Local Windows development](../arch/windows.md#local-windows-development). Do not reuse a Mac path. Windows CI does not replace a native Windows smoke run.

### macOS native control of the debug executable

Computer use may reject the bare `target/debug/Runner` executable as an invalid app. The #777 run and #766 baseline used a temporary `.app` wrapper with a distinct bundle ID to make the exact development build selectable. Build with `make run` first, then stop only the development process started by this test before launching the wrapper. If a development app was already running before the test, obtain authorization before stopping it.

Create the wrapper under the run's canonical scratch root. Copy both the app executable and CLI sidecar from this worktree, and verify the app copy is byte-identical:

```sh
smoke_bundle="$smoke_root/Runner Smoke Dev.app"
mkdir -p "$smoke_bundle/Contents/MacOS"
cp target/debug/Runner "$smoke_bundle/Contents/MacOS/Runner"
cp target/debug/runner-agent-cli "$smoke_bundle/Contents/MacOS/runner-agent-cli"
cat > "$smoke_bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Runner</string>
<key>CFBundleIdentifier</key><string>com.wycstudios.runner.smoke-dev</string>
<key>CFBundleName</key><string>Runner Smoke Dev</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
cmp target/debug/Runner "$smoke_bundle/Contents/MacOS/Runner"
cmp target/debug/runner-agent-cli "$smoke_bundle/Contents/MacOS/runner-agent-cli"
shasum -a 256 target/debug/Runner "$smoke_bundle/Contents/MacOS/Runner"
env -u NO_COLOR "$smoke_bundle/Contents/MacOS/Runner"
```

The source CLI is `runner-agent-cli`; Runner installs it as `runner` in development app data. On a case-insensitive macOS filesystem, `target/debug/runner` resolves to the `Runner` app executable and must not be used as the CLI sidecar source.

Preserve any recording environment variables on the wrapper launch, such as `RUNNER_RECORD_INPUT_FIXTURE`, using an absolute scratch path prefix. Also omit `NO_COLOR` from a programmatically constructed child environment; changing `TERM` or `COLORTERM` alone does not restore colors. The #791 run confirmed native Claude colors returned after removing inherited `NO_COLOR=1`. When launching from a crew session, unset `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE` and `RUNNER_EVENT_LOG` for the wrapper and external development CLI commands so they do not inherit the implementation mission's identity. Bind computer use to the full `.app` path or its distinct bundle ID, verify its window, and confirm the absolute development CLI still reports `com.wycstudios.runner-dev/runnerd.sock`. Keep only one QA wrapper running: selecting or querying a closed macOS app through computer use can relaunch it, so retire old app bindings when switching builds and verify process inventory without querying a closed app. In the #766 baseline, selecting by bundle ID and clicking the composer before each keyboard action resolved missing terminal input; verify the typed text or recorder input events before proceeding. Debug app-data selection depends on the build, so the distinct bundle ID does not create another data namespace. Record the wrapper path, hash comparison and endpoint in the test evidence. Quit only this wrapper during cleanup and retain it with the evidence unless removal is authorized.

## Direct-chat execution

Run the selected [runtime cases](regression/runtimes.md), including `RT-LIFE-01` through `RT-LIFE-09` for the lifecycle core. Their actions, expected outcomes and required evidence are maintained in the suite. Complete each applicable case once per required runtime. Use a unique marker such as `SMOKE-<run>-<runtime>-ORIGINAL`, and a different `...-NEW` marker after clearing. Stop repeating passed checks unless the candidate changes or a specific failure needs a targeted retry.

Create a runtime-only chat with explicit overrides:

```sh
"$smoke_cli" chat start --runtime codex --model <available-model> --effort <supported-effort> --cwd "$smoke_root/codex" --json
"$smoke_cli" session show <session-id> --json
```

Open that exact chat in the development UI; chat creation can return before the process is spawned. Use computer use for terminal input and screenshots, and the CLI for state capture. After a process resume, click the visible terminal input before typing because focus can be lost. Inspect the fresh UI state after actions. If paste does not work, use the computer-use typing API; do not replace UI input with a private PTY or database write.

## Mission startup in a fresh folder

For `RT-START-01` in [runtimes.md](regression/runtimes.md#lifecycle-core), use one test role and a **single-slot crew whose slot is lead** for each runtime's startup check. Worker startup behavior can intentionally wait for lead instructions; a multi-runtime crew is therefore not a reliable test that every slot executes the mission goal automatically. Give the test role a bounded prompt: perform only the smoke goal, inspect no repositories, send no messages, launch no agents and change no settings.

Create the role/crew using the development CLI's (`"$smoke_cli"`) `role create`, `crew create`, `crew add` and `crew lead` commands, checking `--help` for their flags. `crew add` takes a role handle, not its opaque role ID. Use test-specific names and preserve the returned IDs. These are temporary smoke configurations, not implementation crews working on the repository.

Write a goal file outside the repository and start the mission in a newly created canonical directory that has never been trusted. The goal runs one harmless append command and then becomes idle:

```text
This is a development-only runtime startup smoke test. Perform this goal exactly once, without asking for further input. Use your terminal tool once to run the following command in the current working directory:

python3 -c "import os; from pathlib import Path; p=Path(os.environ['RUNNER_HANDLE']+'.txt'); f=p.open('a'); f.write('SMOKE-AUTOSTART\n'); f.close()"

Then reply exactly ACK SMOKE-AUTOSTART. Do not run any other tools, inspect files, send messages, launch agents, or stop the mission. Remain idle after replying.
```

Use an installed Python executable appropriate to the platform, or an equivalent append command supported by that shell. Verify the command before starting the agents; do not install a dependency just for this marker.

```sh
"$smoke_cli" mission start --crew <crew-id> --title <test-title> --cwd <fresh-canonical-directory> --goal-file <goal-file> --json
"$smoke_cli" mission feed <mission-id> --follow --json
"$smoke_cli" mission show <mission-id> --json
```

Start means start and watch: use the Runner skill's supported watch facility for that exact mission ID. If the host cannot deliver background events while idle, state that automatic watching is unavailable and give the foreground feed command. Do not claim an unread background log or manually polled PTY is an active notification watch.

Before typing anything into the mission, verify the marker file contains **exactly one line in the requested mission directory**, the agent's terminal shows the ACK, the session captured a key and Runner reached Idle. An ACK with a marker written elsewhere fails the working-directory check. No manual Return, trust acceptance or repeated goal submission may be counted as automatic startup. Record any prompt that prevented execution, unexpected extra tools and the existing permission mode. Do not switch to bypass mode to obtain a pass.

## Cleanup and report

Stop all processes created by this run, then archive its direct chats and missions. A request to run this procedure includes this cleanup unless Jason explicitly excludes it. Use only the ledger's IDs:

```sh
"$smoke_cli" session stop <test-chat-id> --json
"$smoke_cli" session archive <test-chat-id> --json
"$smoke_cli" mission stop <test-mission-id> --json
"$smoke_cli" mission archive <test-mission-id> --json
```

Verify chats are stopped/archived, missions have zero live sessions and a non-empty archive timestamp, and test rows disappear from Recents. Stop any feed followers. Preserve conversation history and evidence for review. List temporary roles/crews and scratch artifacts retained; do not delete them, branch/worktree data or agent conversation files without the applicable authorization. Leave pre-existing sessions untouched.

Deleting a crew also removes its mission and session metadata, including archived missions. Retain the test crew when preserving those records; archiving its missions is sufficient cleanup. If crew deletion is explicitly authorized, capture final stopped/archived status before deleting it and disclose the resulting loss of Runner metadata in the run record.

Write the run at `docs/tests/runs/YYYY-MM-DD-<platform>.md` using the [suite run-record format](regression/README.md#run-records-and-evidence): candidate and environment, then case ID/platform/runtime/variant → Passed, Failed, Blocked or Skipped with evidence, findings and cleanup. Repeated same-day runs use separate identified blocks in that file. Record authorization, exact method and evidence locations. Keep credentials, account identifiers and unrelated conversation content out of committed evidence. Link automated CI separately. Classify each check as Passed, Failed, Blocked or Skipped with a concrete reason; an overall full pass requires every required check to pass. Record regression attribution only when established. For a behavior-preserving refactor, list discovered bugs separately and do not change frozen expectations or fix unrelated behavior to make the smoke green.
