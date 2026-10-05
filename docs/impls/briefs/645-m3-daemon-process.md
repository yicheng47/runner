# 645 m3 — The daemon process (runnerd 1c)

Program [#645](https://github.com/yicheng47/runner/issues/645) (`runnerd`), phase 1, mission 1c. Jason requested this mission on 2026-10-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-645-m3-daemon-process`, on the existing branch `feat/645-m3-daemon-process`, created from the umbrella `origin/feat/645-runnerd` at `a1c2a300`. That commit is `main` plus the umbrella's CI commit, mission 1a (`625df01f`) and mission 1b (`a1c2a300`). The root checkout stays on `main`. Do not create another branch or worktree, and do not share a Cargo target directory. This lands on the umbrella, never on `main`.

## The goal

After 1a and 1b, the app reaches the whole core through `DaemonClient`, and each session's authoritative terminal (`TerminalModel`) lives in the session layer, but everything still runs inside the app's process over `InProcessTransport`. **This mission moves the core into its own process, `runnerd`, and makes the app a client of it over a local socket.** An app crash or force-quit then leaves `runnerd` and every session running, and the app reattaches when it starts again. The CLI starts `runnerd` when it is not running, so it works with the app closed. Quit itself is unchanged: quitting the app still stops the sessions and stamps them for resume, as today. The quit choices, the mismatch dialog and update handling are 1d.

## Read first

- `AGENTS.md`, especially Worktrees and Crew Missions.
- `docs/features/645-session-host.md`, all of it, in particular "The daemon owns the state", "The client protocol", "Starting, finding and stopping the daemon", "Quitting", "Updates and version skew", "When something dies", and "Windows".
- `docs/impls/645-runnerd/plan.md`: "Mission 1c" (items 1–15) and "Crates". The design is there; this brief wins on any detail.
- `docs/impls/645-runnerd/mission-1a.md`: the request surface, and "Main-thread requests", the 89 non-`fast` request sites that can run on the main thread.
- `docs/tests/645-m2-terminal-split.md`: the #709 audit table (its "Shutdown" row says 1c must define daemon shutdown) and the benchmark harness and baseline.
- Code at `a1c2a300`:
  - `crates/runner-app/src/bootstrap.rs` (`NativeMcpServer`, `boot_core`, `stop_running_sessions_on_quit`, `install_wake`) and `main.rs` `run()` (around 1317–1435, including `on_app_quit`);
  - `crates/runner-core/src/protocol/{client,api,terminal,mod}.rs` (`Transport`, `DaemonClient`, the `daemon_api!` table, `TerminalFrame`, `TerminalSnapshot`, `TerminalAttachment`, `TerminalLifecycle`);
  - `crates/runner-backend/src/daemon/{mod,resume,commands}.rs` (`dispatch`, `InProcessTransport`, `consume_resume_on_launch`);
  - `crates/runner-backend/src/{ipc.rs,cli_install.rs,mcp/}` (`IpcListener`, `install_runner_cli`, `locate_source`, the MCP server on `mcp.sock`);
  - `crates/runner-core/src/app_paths.rs` (`mcp_endpoint` and the data/log directories);
  - `crates/runner-cli/src/{main,command,client,ipc}.rs` (the CLI's socket client, `NotRunning`, `Blocked`);
  - `crates/runner-app/src/{logging.rs,app_settings.rs}` and `crates/runner-app/tests/request_surface.rs` (the guard test, whose last allowlist entry is this mission's).

## Deliverables

Plan items 1–15 are the deliverables. The points below add to them or settle what the plan leaves open.

1. **The binary.** One sidecar, installed as both `runner` and `runnerd` (plan item 1). `runner-cli`'s `main` dispatches on its file stem, case-insensitive with `.exe` removed. In the build tree the binary is still `target/debug/runner-agent-cli`. On Windows, `conpty.dll` and `OpenConsole.exe` are copied into `<app data>\bin` beside `runnerd.exe`; check how the app bundle carries them today (#492).
2. **What moves into `runnerd`.** `runner-backend::daemon::boot` takes over what `boot_core` and `NativeMcpServer` do today: the database, stale-row cleanup, the orphan sweep, router mounting, discovery, usage, `consume_resume_on_launch`, and the MCP server on `mcp.sock`, which the CLI keeps using unchanged. The app keeps the paths, the sidecar install, connect-or-spawn, the handshake and its UI settings.
3. **`cli_install` moves out of the backend.** The app installs the sidecar before it can start `runnerd`, and after item 13 it cannot call the backend. Move `install_runner_cli`, `locate_source` and what they need into `runner-core`; they copy files and need nothing from the daemon. It skips replacing `runnerd` (and on Windows the ConPTY files) while `runnerd.lock` is held.
4. **Settings have one owner each** (plan item 3). `runnerd` reads the enabled runtimes, resume on launch and the mission permission mode from `ui-settings.json` at start. The app pushes later changes as requests. The PR carries the inventory: setting, owner, how the other side learns of a change.
5. **The wire.** `runnerd.sock`, or `\\.\pipe\com.wycstudios.runnerd` (`-dev` for debug builds), through `IpcListener`, owner-only; on Windows with an explicit security descriptor, and the existing CLI pipe gets the same fix. Both keep rejecting remote clients.
   - A frame is a u32 length, a u8 kind and a payload. Control frames (`Hello`, `Welcome`, `Mismatch`, `Shutdown`, requests, responses, events) are JSON with optional fields, and unknown fields are ignored. Terminal frames (`Output`, `Resized`, `Resync`, `Input`, `Resize`, `Attach` and its snapshot) are binary. Split terminal payloads above 64 KiB so one burst cannot hold up a response.
   - Requests carry an id, and responses come back by id, so one connection carries calls from the main thread and from background tasks at once. `runnerd` runs each request off the IPC runtime's async threads (the ops are synchronous and use SQLite), so a slow request never blocks another.
   - Events: one subscription per connection, with a bounded queue. Overflow is reported to the client as `EventError::Lagged`, which the app already handles by refreshing. `TerminalLifecycle` notifications (spawn, exit, title) become events.
   - Terminal: `attach` returns the snapshot and then the frames for that session over the same connection; `input` and `resize` are one-way, as 1b made them. 1b's single-lock invariant and its `Resync` on a full queue stay as they are; only the transport changes.
   - `runnerd` serves on the existing tokio IPC runtime. The app uses one blocking reader thread and one writer per connection, as its other I/O does, feeding the event thread and the mirror registry.
6. **Synchronous calls keep their threads, with a deadline.** Keep 1a's rule: every call stays on the thread it runs on today. Add no request queues, generations or stale-completion gates. Each synchronous request has a client-side deadline (10 s by default); a call that passes it, or whose connection drops, returns a `ClientError` through its existing error path. The record lists the measured socket round trip for the common requests, and every main-thread request from mission-1a.md's table whose p99 exceeds 16 ms, for 1d to decide on.
7. **Connect or spawn, the handshake, and lifetime:** plan items 5, 7 and 8. In 1c, a mismatch stops the old daemon with `Shutdown { stop_sessions: true }` and starts the new one without asking. The order matters for `make run`: shut the old daemon down, wait for `runnerd.lock` to be released, install the new sidecar, then spawn. That is what restarts the development daemon after a rebuild.
8. **A clean environment:** plan item 6.
9. **Shutdown, defined.** `Shutdown { stop_sessions: true }`, `runner daemon stop`, SIGTERM and the Windows `CTRL_LOGOFF_EVENT`, `CTRL_SHUTDOWN_EVENT` and `CTRL_CLOSE_EVENT` (plan item 10) all run one path, in this order: stop accepting connections, stamp `resume_on_launch`, `kill_many`, drain and join the session workers with a deadline, stop the MCP server, remove both socket files, release `runnerd.lock`, exit. The record lists the deadline and what happens to a worker that misses it. This answers the 1b audit's Shutdown row.
10. **`runnerd` defends itself.** Every few seconds it checks that `mcp.sock` and `runnerd.sock` are still its own files (macOS; on Windows, that it still owns its pipes). If another process has replaced them, as a pre-`runnerd` build would, it runs the shutdown path and exits, so two processes never own one database.
11. **Quit is unchanged:** plan item 9. `on_app_quit` sends `Shutdown { stop_sessions: true }` and waits for `runnerd` to exit, with a deadline.
12. **Reattach and crash recovery:** plan items 11 and 12. At launch, after the handshake, the app loads `AppStore` through requests and attaches every live session (snapshot first, then the mirror). If the connection drops, the app shows a notice and reconnects or spawns a new daemon; after three restarts in five minutes it stops trying and points to `runnerd.log`.
13. **The app stops depending on the backend:** plan item 14. `runner-backend` moves to `runner-app`'s `[dev-dependencies]`; app tests keep building an in-process core through 1a's helper. Delete the guard test `crates/runner-app/tests/request_surface.rs`, because Cargo now enforces what it checked.
14. **The CLI:** plan item 13. On `NotRunning`, the CLI spawns `runnerd` from its own canonicalized directory, waits up to 10 s for `mcp.sock`, and continues. It never spawns when `SSH_CONNECTION` is set, and never on `Blocked`. `runner daemon status` and `runner daemon stop` use `runnerd.sock`. Exit codes stay as they are (3 not running, 5 blocked); the `--json` output of every existing command is unchanged.
15. **Logs:** plan item 15. `runnerd.log` beside `runner.log`, with the same rotation and panic hook, from a setup both binaries share.

## Out of scope

The quit dialog, the `quitBehavior` setting, the mismatch dialog and update handling (1d); the `runner-backend` → `runner-daemon` rename (1d); moving the CLI off MCP (after phase 1); the downgrade guard on `main` (done separately, before this mission's nightly); #797; any schema, event-log or CLI output change. No new runtime behaviour beyond what this brief names.

## Rules

- **The umbrella's rules:** no migration, no event-log shape change, no CLI output change (plan, "Branch and channel").
- **No test starts a daemon in real app data.** A test that spawns `runnerd` uses a temporary app-data directory and endpoint, passes every root in rather than resolving `$HOME`, and kills the daemon on drop.
- **Before every handoff, check the Windows build.** A local cross-check cannot run on this Mac (no Windows SDK; Jason waived installing one), so list every `cfg` gate and every moved `use` and verify each by hand, and gate imports used only by `cfg(unix)` tests. Windows CI is the real gate; it has failed on unused or missing imports in five missions.
- No new dependency without a reason in the handoff. `fs2` and `sha2` are already in `Cargo.lock`; check whether they are direct dependencies before adding anything.
- Stage by path, never `git add -A`.
- **Nobody runs the app.** The coder and the reviewer never launch it. The CLI in the build tree is `target/debug/runner-agent-cli`; never run `target/debug/runner`, which is the GUI. Integration tests start `runnerd` only as the rules above allow.
- No extra agents.

## QA

**QA does not run the app, any agent, or `runnerd` outside the tests until Jason authorizes it on the feed.** He will do so later. Until then QA may read the diff, the tests and the record, and prepare its live checklist in `docs/tests/645-m3-daemon-process.md`.

When Jason authorizes it, QA runs these on the branch's development build (`make run`, development data only), with Claude Code, Codex and pi chats, shell terminals and one disposable mission on a test crew it creates:
- a chat and a mission survive an app force-quit, and the app reattaches with intact screens and correct status;
- `ask_human` while the app is closed, answered from the CLI;
- quit still stops sessions, and relaunching resumes them;
- the CLI starting `runnerd` with the app closed;
- `runner-dev daemon status` and `stop`, with no orphan processes after any stop path, checked against the process list;
- killing `runnerd` (`kill -9`), the app's notice, and the restart cap;
- `runnerd` keeps running after the last session and client;
- a rebuild while sessions are live restarts the development daemon;
- the privacy check: an agent reads `~/Documents` after the app quits, with no new prompt;
- the terminal after a reattach: a long Claude Code transcript, Codex on the alternate screen, IME, paste, selection, link activation and the OSC 7 live cwd (the last two were left unchecked in 1b).

QA never logs Jason out of macOS: the logout case is the SIGTERM test, and the real logout check is Jason's. It never touches the production app, production data, or `~/.claude`, `~/.codex` or `~/.pi` settings, archives every chat and mission it created, deletes its test role and crew, and notes each live agent session's cost in the record.

## Verification

1. **Integration tests** in `crates/runner-cli/tests`, each against a temporary app-data directory and endpoint:
   - start `runnerd` and complete the handshake; a mismatched hash gets `Mismatch`;
   - spawn a `shell` session running `cat` and get the echo back;
   - disconnect and reattach, getting a snapshot identical to the model's;
   - two clients attached to one session get the same frames; a DA1 query still gets exactly one reply;
   - kill `runnerd` and confirm its children have gone, on both platforms;
   - SIGTERM a daemon with a live session and check the `resume_on_launch` stamp (unix; on Windows, the console control event if CI allows);
   - `runnerd` keeps running after the last session and client, with usage polling and discovery paused until a client connects;
   - a second starter loses the lock and exits 0;
   - the CLI starts the daemon on `NotRunning`, and does not with `SSH_CONNECTION` set;
   - replacing `runnerd.sock` makes the daemon shut down.
2. **Every existing test passes**, with 1b's snapshot and split-point tests unchanged. Record workspace test counts (`--profile ci`, with pipefail) at `a1c2a300` and after.
3. **The benchmark, now across the socket.** Run 1b's harness on macOS at `a1c2a300` for the baseline and on the branch. p99 echo is at most 1 ms above the baseline, and burst throughput at least the baseline's. Windows numbers are measured on the PC separately, not by the crew.
4. **The commands:** `make verify`, workspace clippy with `-D warnings`, `--features updater` clippy, `cargo fmt --all --check`, `git diff --check`, and the Windows audit above. Record each exit code.

Record everything in `docs/tests/645-m3-daemon-process.md`, together with the settings inventory (item 4), the request latencies (item 6) and the shutdown deadline (item 9).

## Review

The coder implements and hands off on the Runner feed. The reviewer reviews the full diff against this brief, must-fix first with file:line. It checks:
- that no request, event or terminal frame can be lost, duplicated or reordered across the socket, including on reconnect and `Resync`;
- that a slow or hung request cannot freeze another, and that every synchronous call has its deadline;
- the shutdown path and its order, from every trigger;
- connect-or-spawn races: two app windows, the app and the CLI at once, and a stale socket file;
- that no test can touch real app data;
- the Windows gates, the pipe security descriptor and the spawn flags;
- that nothing in Out of scope changed.

Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

## Authorization

After a clean review, Jason authorizes the following:
- Squash all work on this branch, this brief and the test record included, into one commit on top of current `origin/feat/645-runnerd`, with a subject that names the change (for example `feat(daemon): run the core in runnerd and make the app its client (#645 1c)`).
- If the umbrella has moved, rebase; never merge.
- Push the branch and open the PR against `feat/645-runnerd` **as a draft**. The body says `Refs #645` and carries the gates, the benchmark, the inventories and the review verdict, with no Claude session link, and says that QA's live checks are pending Jason's authorization.
- Review or CI fixes are amended into the same commit and pushed with `git push --force-with-lease`.
- Drive `Rust / macOS` and `Rust / Windows` green.

Do not mark the PR ready, merge it, delete the branch or worktree, or cut a nightly or release. After QA's live checks, which wait for Jason, any fixes follow the same amend rule, and QA's results go into the record and the PR body.

The handoff after CI is green carries the PR URL, what changed, tests and exit codes, the benchmark, CI and the reviewer's verdict. Then stand by for Jason.
