# runnerd — implementation plan

Plan for [feature 645](../../features/645-session-host.md) ([#645](https://github.com/yicheng47/runner/issues/645), milestone 0.13, closed 2026-10-07). The spec says what and why. This file says how it lands: the phases, the missions, what each one touches, how each is verified, and what reading the code found. [README.md](README.md) holds the condensed state, [impl_log.md](impl_log.md) the dated log, and the briefs go in [`../briefs/`](../briefs/) as `645-m{n}-{slug}.md`.

## Status (2026-10-07)

Phase 1 shipped in 0.13.0 on 2026-10-06, tagged the day the umbrella landed instead of after the gate below (Jason: the app is pre-alpha, so fix forward), and #645 closed on 2026-10-07. The CLI's move to the client protocol ("After phase 1") is [#821](https://github.com/yicheng47/runner/issues/821), in [PR #822](https://github.com/yicheng47/runner/pull/822); phases 3 and 4 are [#808](https://github.com/yicheng47/runner/issues/808). The paragraph below is the record up to the landing.

The spec was drafted on 2026-10-04, reviewed with Jason on 2026-10-05, and landed on `main` with this plan the same day. In that review the process was renamed `runnerd`, the terminal design was checked against the tmux era and PR #157, #709 was closed, and the branch, channel, crate and no-downgrade plans below were settled. The umbrella branch `feat/645-runnerd` was cut the same day. Missions 1a ([#805](https://github.com/yicheng47/runner/pull/805)) and 1b ([#806](https://github.com/yicheng47/runner/pull/806)) merged into it on 2026-10-05, and its first nightly went to the regular `nightly` release (see the note under "The `nightly-runnerd` channel"). Mission 1c ([#807](https://github.com/yicheng47/runner/pull/807)) followed on 2026-10-06, and 1d ([#810](https://github.com/yicheng47/runner/pull/810)) the same day. The umbrella landed on `main` on 2026-10-06 ([#811](https://github.com/yicheng47/runner/pull/811)), and the gate now protects the 0.13.0 tag. Phase 2 and the downgrade guard were dropped (spec decision 13; Branch and channel).

## Phases

There are four phases, and only phase 1 gates 0.13.0.

| Phase | What the user gets | Missions | Ships in |
| --- | --- | --- | --- |
| 1. The local daemon | Quitting or crashing the app no longer stops agents; missions keep coordinating and the CLI works with the app closed | 4 (1a–1d) | 0.13.0, the gate |
| 2. Dropped (2026-10-06) | An update restarts every session (spec decision 13) | — | — |
| 3. Remote machines over ssh | A chat or a crew slot on another Mac | about 2 | later |
| 4. The Windows PC | A crew slot on the Windows PC, such as a tester | 1 | later |

**Why this order.** The 2026-08-18 record of this direction in the [GPUI rewrite plan](../archive/gpui-rewrite/plan.md) ("Post-cutover direction: session daemon") named updates as the motivator: "every update restarts the app and kills running agents; a daemon makes updates and crashes invisible to live missions." Phase 1 makes quit and crash invisible. Phase 2 was to make updates invisible too, with a second process holding the PTYs and a protocol kept stable across releases. Jason dropped it on 2026-10-06 (spec decision 13): agents resume their conversations after an update, the update dialog shows when agents are working before their turns are lost, and restarting carries every fix to the whole stack. Remote machines still need the stable session protocol, so phase 3 builds it.

## Branch and channel

Phase 1 does not land on `main` mission by mission (Jason, 2026-10-05). It is built on an umbrella branch, shipped to a separate nightly channel, and reaches `main` once. The GPUI rewrite did the same with `gpui-nightly`. On 2026-10-06 Jason moved the landing forward: the umbrella lands on `main` as soon as 1d merges into it, and the gate below protects the 0.13.0 tag instead, since nobody else runs nightlies and stable releases are cut deliberately.

### The umbrella branch

- **`feat/645-runnerd`, cut from `origin/main`** after the spec and this plan land on `main`, in the worktree `.worktrees/feat-645-runnerd`.
- **Each mission branches from the umbrella,** for example `feat/645-m2-terminal-split` in `.worktrees/feat-645-m2-terminal-split`, and opens its PR against the umbrella. It still lands as one commit. CI must be green on both platforms before it merges, as on `main`.
- **The umbrella follows `main` by rebase, never by merge** ([AGENTS.md](../../../AGENTS.md)). It is rebased onto `origin/main` between missions, never while a mission branch is open from it, and pushed with `--force-with-lease`. The next mission then branches from the rebased umbrella. It is also rebased before every channel cut, so each nightly carries `main`'s fixes.
- **Fixes to code the umbrella also touches go to `main` first.** A session or terminal bug found on `main` is fixed there, and reaches the umbrella at its next rebase. A fix for something only the umbrella has stays on the umbrella.
- **One rule keeps the two lines interchangeable: no schema change on the umbrella.** It adds no migration, changes no event-log shape and changes no CLI output. New settings are new keys that older builds ignore. If a migration becomes necessary, it lands on `main` first in a form both lines can read, as the GPUI rewrite required.
- **Docs stay on `main`.** The spec, this plan and the log are doc-only changes and go straight to `main`. A mission's brief goes on its mission branch, as AGENTS.md requires, and travels with its commit. Briefs and mission test records stay on the umbrella until it lands, because later missions read them; the landing's cleanup archives them.
- **Landing** (Jason, 2026-10-06). When 1d has merged into the umbrella, the umbrella reaches `main` as one PR, rebase-merged so the mission commits stay separate, as #800 was. The umbrella's two CI-only commits are dropped first. Nightlies are cut from `main` again from then on, and 0.13.0 is tagged from `main` when the gate below passes. Once the umbrella lands, `main` can no longer produce a 0.12.x patch; an urgent fix for 0.12 users goes on a branch from the `v0.12.9` tag.

**1a goes on the umbrella too (Jason, 2026-10-05).** He first chose `main` for 1a, because it changes no behaviour and touches the 82 app files every other app change would conflict with. He moved it to the umbrella the same day, so that no change reaches `main` before QA's regression on the umbrella covers it, and so 1a needs no separate smoke test. The cost: app changes that land on `main` while the umbrella lives must be converted to `DaemonClient` at each rebase. The 1a guard test flags every direct core call such a change brings, so none slips through.

### The `nightly-runnerd` channel

**Not built (Jason, 2026-10-05).** There are no nightly users besides Jason, so the umbrella's cuts go to the regular `nightly` release, dispatched with `gh workflow run nightly.yml --ref feat/645-runnerd`, and the channel infrastructure below is not built. While the umbrella lives, every nightly is cut from it: a cut from `main` would replace it, and from 1c on would be a downgrade for an install running `runnerd`. There is no downgrade guard (Jason, 2026-10-06): the answer to going back is 0.13.x being good enough that nobody wants to, and `runnerd` shuts down cleanly if an older app takes its sockets. The rest of this section is the original design, kept in case a separate channel is needed later.

- **A rolling public prerelease, tagged `nightly-runnerd`,** beside `nightly`. It is cut from the umbrella's head, keeps two builds per platform, and is never published as a normal release, so `releases/latest` and the production feeds never see it.
- **The same app identity as every other channel.** It is `com.wycstudios.runner`, `Runner.app` and the same app-data directory. Installing a `nightly-runnerd` build over Runner switches that install to its feed, and installing any other build switches back; this is the install-to-switch rule the unified nightly already follows. Jason daily-drives it on his real roles, crews and missions, which a separate data directory could not test.
- **No downgrade, fix forward (Jason, 2026-10-05).** Jason's Mac install takes every cut from the first one and does not go back. A problem found there is fixed on the umbrella and shipped in the next cut. Before the first install, his data directory is backed up, including `runner.db` with its `-wal` and `-shm` files: fixing forward repairs code but cannot bring back a damaged database.
- **No downgrade guard** (Jason, 2026-10-06). The original design added a guard to every build without `runnerd`, so it would refuse to start while `runnerd` runs. It was dropped: the answer is making 0.13.x good enough that nobody goes back, and `runnerd`'s self-defence below covers an older app that starts anyway.
- **`runnerd` defends itself as well.** Every few seconds it checks that `mcp.sock` and `runnerd.sock` are still its own files. If an older app has replaced them, it stops its sessions and exits, so two processes never own one database. This belongs to 1c.
- **The build says which channel it is.** About and Settings show `Nightly runnerd (<sha>)`.
- **Cuts happen after each mission merges into the umbrella** (1b, 1c, 1d) and after umbrella-only fixes. They are dispatched by hand, like `nightly`, and never cancel a `nightly` cut. Every cut goes to Jason's Mac and to the Windows PC. Development builds (`make run`), which have their own data directory and daemon, are still where each mission is tested before its PR merges into the umbrella.

**Channel infrastructure lands on `main` first,** before the first cut. It is a small change, done inline, not a mission:

- `nightly.yml` gains a `channel` input (`nightly` by default, or `nightly-runnerd`), which picks the release tag, the asset names, the feed URLs and its own concurrency group.
- `script/bundle-mac --channel nightly-runnerd` writes `SUFeedURL` as `…/releases/download/nightly-runnerd/appcast.xml` and names the DMG `Runner-Nightly-runnerd-<sha>.<stamp>-arm64.dmg`.
- `release_urls` in `updater/windows.rs` maps `RUNNER_RELEASE_CHANNEL=nightly-runnerd` to that tag.
- The publish job's checks and `script/verify-nightly-appcast.py` take the tag as a parameter.
- `ci.yaml` runs on pushes to the umbrella and on PRs against it, so mission PRs get both platforms and the channel's CI gate has a run to wait for. This trigger is already in place as the umbrella's own first commit (2026-10-05), and that commit is dropped when the umbrella lands.
- The `nightly` skill gains `run --channel runnerd`, which requires the umbrella's head instead of `main`'s.

### The checks before landing

They run while Jason daily-drives the channel, and all of them pass on both platforms before 0.13.0 is tagged. They are recorded in `docs/tests/645-runnerd-switch.md`, which is written when the first run starts, with each row's result, build and date. A failure is fixed on the umbrella and its row runs again on the next cut.

**A. Automated, in CI on both platforms.**
- The workspace tests, including the daemon integration tests from 1c.
- The snapshot round-trip and split-point tests over the fresh recordings.
- The benchmark against `main`, within the limits in 1b.

**B. A rehearsal on the development build** (`make run`, development data). The QA slot runs it under live-test authorization, and Jason runs the parts that need his eyes.
- **Lifecycle:**
  - quitting with Ask, Keep and Stop;
  - force-quitting the app;
  - killing `runnerd` (`kill -9` on macOS, ending its process tree on Windows);
  - the restart cap after repeated crashes;
  - keeps running after the last session and client, with usage polling paused;
  - `runner daemon status` and `stop`;
  - the CLI starting the daemon, and refusing to over ssh;
  - a rebuild while sessions are live, which restarts the development daemon.
- **Every runtime:** Claude Code, Codex, pi, Copilot, Antigravity and a shell, each through a turn with a tool, an approval, a cancel, `/clear`, resume, fork and restart. The status shown is right before and after a reattach.
- **Missions:**
  - start a mission and send crew messages;
  - `ask_human` with the app closed, answered from the CLI and, separately, from the card;
  - stop, resume and restart a slot;
  - archive.

  Nothing is delivered twice, checked against both the feed and the terminals.
- **Terminal after a reattach:**
  - a long Claude Code transcript and Codex on the alternate screen;
  - `vim` and `less` in a shell;
  - CJK and emoji widths, IME, paste, selection, scrollback;
  - drag-resizing, split panes, one session in two windows, drawer shells;
  - the agent-update modal.
- **No orphan processes** after any stop path, checked against the process list.
- **A 24-hour soak:** a mission and three chats running while the app quits and relaunches several times. `runnerd`'s and the app's memory and CPU are sampled every hour.

**C. A rehearsal on a copy of Jason's data.**
- Copy the production `runner.db` together with its `-wal` and `-shm` files, and the mission directories, into the development data directory.
- Run the development build on that copy.
- Open every recent chat and mission, resume a few chats and one mission, and confirm the database is unchanged apart from session state.

This catches problems that only real data shows, without touching the real data.

**D. Packaged builds.** Signing, notarization, the sidecar installing `runnerd`, the macOS privacy (TCC) attribution, and a Sparkle update from one `nightly-runnerd` build to the next while sessions are live.
- On the Mac, these run on Jason's own install. Jason, 2026-10-05: no separate account is needed, and his production build being affected is acceptable.
- On the Windows PC, the same checks run, plus the Windows list in 1c.

**E. Dropped:** the downgrade guard (Jason, 2026-10-06).

### The gate to tag 0.13.0

- Checks A to D (above) all passed on a build from `main` with 1d.
- At least one week of Jason daily-driving nightlies from `main` on the Mac, with no fallback to an older build.
- The phase 1 checks on the Windows PC.
- The spec's phase 1 verification list and the [full smoke test](../../tests/full-smoke-test.md) passed on the last cut.

Jason can shorten or lengthen the week. When the gate passes:

1. 0.13.0 is tagged from `main`.
2. Nightly installs move to the release by installing the 0.13.0 DMG or through the updater; there is no bridge build.

### Later phases

Phases 3 and 4 decide their branch and channel when they are planned.

## Crates

Jason asked on 2026-10-05 whether the backend crate should become `runner-daemon`. Yes, with a second step that matters more: the app should not be able to reach the daemon's code at all. A grep test can say "no `ops::` calls in the app", but Cargo can make them impossible.

| Crate | After phase 1 | Depends on |
| --- | --- | --- |
| `runner-core` | What the app, the CLI and the daemon share: the event log, app paths and the `Runtime` enum, as today, plus a new `protocol` module (1a). That module holds everything that crosses the socket: the row types now in `runner-backend/src/model.rs` (plain serde, no rusqlite), the request table with every argument and result type, `AppEvent`, the frames, the handshake, and `DaemonClient` with its transports. | — |
| `runner-terminal` | The model, the mirror and the snapshot; no backend dependency (1b) | — |
| `runner-daemon` (today's `runner-backend`, renamed in 1d) | `AppCore`, ops, the database, sessions, routers, buses, the client protocol, and the daemon's boot and server | `runner-core`, `runner-terminal` |
| `runner-cli` | The `runner` and `runnerd` binary | `runner-core`, `runner-daemon` |
| `runner-app` | The GPUI client | `runner-core`, `runner-terminal`; `runner-daemon` only as a dev-dependency, for tests |

The protocol is a module of `runner-core` rather than a crate of its own (Jason, 2026-10-05). What 1c needs is for the shared types and the client to live outside the daemon's crate, and `runner-core` is already the crate all three share, with the `schemars` feature the model types use. The CLI binary's size does not change, because it already links the whole backend.

Each step lands where it is cheapest:

- **1a moves the types and the table into `runner_core::protocol`,** while it is already touching every call site. The app keeps a normal dependency on `runner-backend` for one reason only, booting the in-process core. The guard test allows `runner_backend::` only in `bootstrap.rs`.
- **1c removes that dependency.** Once the app reaches the daemon over the socket, `runner-backend` moves to the app's `[dev-dependencies]` for tests that build an in-process core. From then on, an app change that calls daemon code does not compile, and the guard test can go.
- **1d renames `runner-backend` to `runner-daemon`.** By then only the CLI, the crate's own tests and the docs name it, so the rename is small. Renaming now would touch all 82 app files that 1a is about to rewrite anyway.

Anything the app needs that is not data from the daemon, such as runtime capability flags from `runtimes::for_key` or the command name in `cli_install`, either moves into `runner-core` as a constant or catalog type, or comes from a request at startup. 1a's brief lists each one.

## #709 does not go first

[#709](https://github.com/yicheng47/runner/issues/709) asks whether Runner should replace its hand-fed parser and its own PTY reader with alacritty's `tty` and `EventLoop`, as Zed does. The roadmap said to "decide the terminal engine before extracting it into the session host". The decision falls out of #645:

- **`runnerd` must forward raw bytes, and the upstream event loop does not expose them.** In the pinned `alacritty_terminal` 0.26.0, `EventLoop::pty_read` reads the PTY and calls `state.parser.advance` directly. Its only tap is the `ref_test` flag, which writes a hard-coded `./alacritty.recording` file. The app's mirror needs those bytes, and so do the fixture recorder, the idle detector's byte activity, the Codex title adapter, TUI readiness, and the OSC 7 and colour-scheme scans (vte drops OSC 7). #709's own draft forbids the workaround, a second reader or a second live parser. PR #157 found the same thing in July: "Its reader feeds bytes straight into `Processor::advance` and never exposes them externally."
- **Zed can use it because Zed renders in the same process.** Runner after #645 does not.
- **What motivated #709 is fixed.** The synchronized-update deadline from #647 shipped in [PR #710](https://github.com/yicheng47/runner/pull/710), and #647 closed on 2026-09-23.
- **Its audit is still worth doing, and 1b is the place.** 1b moves exactly this code, and a daemon raises the stakes on several items: final output drained before an exit is published, partial writes and backpressure, resize ordering, replies answered once, EOF and child exit, and shutdown. 1b's brief carries #709's checklist, and 1b's record answers it.

Jason closed #709 on 2026-10-05 as answered by #645 (not planned), with a comment giving these reasons; the audit lives on in 1b. Adopting the upstream engine later would mean a fork with a byte tap, which is not worth carrying. The unmerged branch `fix/647-terminal-black-sidebar` holds `docs/arch/process-model.md`, a map of today's per-session threads that 1d can update into the arch docs, and then the branch can go.

## Phase 1 — the local daemon

There are four missions, in order, each one PR with one commit, and each leaves both platforms working. 1a and 1b change no behaviour. 1c moves process ownership, but quit still stops sessions as it does today. 1d adds the choices. Nightlies carry 1c and 1d before 0.13.0.

### Sequencing with the rest of 0.13

1a touches 82 files in `runner-app`, and 1b rewrites `runner-terminal` and the app's terminal plumbing. Both conflict with any concurrent app work: [#793](https://github.com/yicheng47/runner/issues/793) (liquid glass), the UI halves of [#562](https://github.com/yicheng47/runner/issues/562) and [#704](https://github.com/yicheng47/runner/issues/704), [#782](https://github.com/yicheng47/runner/issues/782), [#552](https://github.com/yicheng47/runner/issues/552) and [#701](https://github.com/yicheng47/runner/issues/701). Run 1a and 1b back to back with no other `runner-app` mission in flight, and merge each within a day of its handoff.

Backend-only work can run alongside: #748's router notices, the [#723](https://github.com/yicheng47/runner/issues/723) runtimes, and #764. Any op such work adds after 1a lands goes into the request table. [#797](https://github.com/yicheng47/runner/issues/797) waits for 1c, because its listener belongs in `runnerd`.

### Mission 1a — one request surface

Brief `645-m1-request-surface.md`. Crew: codex duo (coder and reviewer), with no live sessions.

**Goal.** Every call from `runner-app` into the core goes through a `DaemonClient`, over an in-process transport that serializes every request and response. 1c then adds a socket and nothing else. Behaviour is identical.

**What the code has today** (`main` at `3d83d697`, 2026-10-05):

- 82 files reach `runner_backend`. They call 99 distinct `ops::` functions: session 26, mission 16, runtime 14, node 10, role 9, slot 7, window 7, crew 6 and project 4. They also call 16 `repo::` functions.
- Direct `AppCore` field use: `core.db` 18 times, `core.usage` 6, `core.events` 6, `core.runtime_discovery` 4, `core.runtime_shell_env` 3, `core.session_events` 3, `core.mcp` 3, `core.windows` 2, `core.sessions` 2, `core.broadcast_focus_map` 1. `core.app_data_dir` (11 uses) is a path the app can compute itself, and stays.
- About 88 call sites already run inside `cx.background_spawn`. About 100 outside test modules run on the main thread, mostly in event handlers and `update` callbacks; `role_list(this.core(cx))` after a create is typical. The most are in `start_chat.rs` (26), `app_store.rs` (10), `bootstrap.rs` (8), `chat.rs` (8), `main.rs` (6) and `windowing.rs` (6). These counts come from a script and are approximate.
- The app also drives core work itself: `bootstrap::boot_core` (the whole startup), `consume_resume_on_launch` (from startup and from Settings), `usage.set_enabled` (`bootstrap.rs` and `main.rs`), `start_background_discovery`, `NativeMcpServer`, and `AppStore::initialize_skill_defaults` and `command_default`, which make file changes in the user's home after discovery.

**Design.**

1. **One table, outside the daemon's crate.** `runner_core::protocol` (new; see Crates) holds the row types moved from `runner-backend/src/model.rs`, the argument and result types the app uses, and `api.rs`, which lists every request once: its name, argument type, result type, handler, and whether it is `fast`. A macro generates `Request`, `Response` and the typed `DaemonClient` methods in `runner-core`, and `runner-backend` implements `dispatch(&AppCore, Request) -> Response`. Adding an op is one line, and the table is what the app parity and CLI recorder tests check.
2. **A transport trait.** `DaemonClient` holds an `Arc<dyn Transport>`. 1a ships `InProcess(AppCore)`, which encodes each request to JSON, decodes it, dispatches, and does the same with the response. It does this always, not only in tests, so a type that cannot cross a socket fails now rather than in 1c. Handlers keep their `ops::` bodies; most arguments are already serde types, because the MCP tools call the same ops.
3. **Events.** `DaemonClient::subscribe()` yields `AppEvent`s. In-process it maps the broadcast receiver, and the app's `native-app-events` thread consumes it as before. On the client side, event names become `String`.
4. **What stays a library call:** model types, `runtimes::for_key`, `app_paths`, constants such as `cli_install::runner_command_name` and `SKILL_MARKER`, and pure formatting. The rule: a call that reads or writes the database, `SessionManager`, a router, a bus, usage, discovery or the window registry is a request.
5. **What stays in the app until later missions:** `boot_core` and `NativeMcpServer`, because the app still runs the core in-process in 1a (1c moves them); the terminal (`TerminalBridge`, `TerminalSession`, and the agent-update modal's `UpdateTerminalEvents`), which 1b moves; and the skill and command defaults, which are file changes in the user's home and read discovery results through requests.
6. **The thread rule, as corrected during the mission (2026-10-05).** Every call stays on the thread it ran on before, and a `fast` request replaces a direct read of in-memory state. The brief first moved non-`fast` calls into `background_spawn`. In-process that buys nothing, and the reviewer found it created ordering races (a drawer spawn persisting one mission's layout into another, runtime forms reading a lagging cache, out-of-order window writes), so the rule was relaxed. No request runs from `render`, `prepaint` or `paint`. The 87 non-`fast` requests still on the main thread are listed in [`mission-1a.md`](mission-1a.md), and 1c decides each one, with timeouts, when the socket arrives.
7. **A guard test.** It scans `runner-app/src` outside test modules and fails on any `runner_backend::` path outside `bootstrap.rs`, on `.db.get(`, and on the `AppCore` fields above. 1c replaces it with the Cargo dependency (see Crates). App tests build a client over an in-process core through one helper. The eight test helpers that build an `AppCore` by hand today use it: `bootstrap.rs`, `app_store/skill_defaults.rs`, `app_store/command_default.rs`, `terminal/element.rs`, `settings_page.rs`, `app_shell.rs`, `agent_update.rs` and `start_chat.rs`.

**Verification.**

- `make verify` passes, with the same test count plus the new tests.
- A serde round-trip test covers every request and response type with a sample value.
- The guard test passes.
- The reviewer confirms that no main-thread path gained blocking work.
- Jason runs a ten-minute smoke on `make run`: start a chat, start, stop and resume a mission, create and edit roles and crews, and open each Settings pane.

### Mission 1b — the terminal moves below the session seam

Brief `645-m2-terminal-split.md`. Crew: codex trio. The QA slot records fixtures under live-test authorization, unless Jason records them.

**Before it starts:**

- **Fresh recordings** with `RUNNER_RECORD_INPUT_FIXTURE`: Claude Code with a long transcript, `/clear` and a resize; Codex on the alternate screen with a resize; pi; and zsh running a full-screen program such as `less`. The corpus has 14 recordings today: the Claude session, the Codex title, the Antigravity and Copilot first turns, seven input-state recordings, `top-busy`, `width-torture` and the procedural glyphs.
- **A benchmark harness,** run against `main` for the baseline on both platforms. It measures keystroke-to-echo through a `shell` session running `cat` (10,000 keystrokes, p50 and p99) and a 50 MB burst through `cat` of a file (time until the last byte reaches the `Term`). The numbers go in the test record.

**What the code has today:**

- `runner-terminal` depends on `runner-backend`, and every `TerminalSession` holds an `AppCore`. Outside tests it uses it for:
  - input through `sessions.inject_direct_stdin`, inline and from the queued worker (`terminal.rs` around lines 440 and 831);
  - terminal replies through `sessions.inject_stdin` (around 537);
  - draft observations through `report_input_state`, from `publish_parsed`;
  - `ops::session::session_set_live_title` (around 616) and `ops::session::session_resize` inside `resize` (around 941);
  - a database read for the link cwd (around 1054 and 1189).

  `TerminalBridge` reads `session_last_size` (around 1542) and receives bytes as the `SessionEvents::output` observer.
- `TerminalSession::resize` resizes the PTY and the `Term` in one call. The terminal element's render path reads only the session: every backend import in `element.rs` is in its tests.
- `feed_output` runs in this order: fixture recording, the colour-scheme scan, the OSC 7 scan, sequence and readiness bookkeeping, parsing in pieces around colour-scheme sequences, the input observation, and scheduling the synchronized-update flush.

**Design.**

1. **The crate boundary inverts.** `runner-terminal` stops depending on `runner-backend`. The backend calls that `TerminalSession` makes today go through a small `TerminalHost` trait the owner implements: write input, write a reply, report the input state, set the live title, and resize the PTY. `runner-backend` depends on `runner-terminal`.
2. **Two types, one parse.** `TerminalModel`, on the daemon side, holds the `Term`, the parser, the synchronized-update flush, the colour-scheme and OSC 7 scans, the input tracker and the fixture recorder, and writes replies to the PTY. `TerminalMirror`, on the app side, holds the `Term`, the parser, the synchronized-update flush, viewers and the waker, selection, scrolling, links and key encoding. It drops its replies and has no input tracker. Both call one shared parse function, so they cannot parse differently.
3. **The model lives in the session layer.** `SessionManager` feeds it from the forwarder, in place of `SessionEvents::output`, so draft observations, replies, the live title and the live cwd all happen next to the delivery gate, with or without a client. `SessionEvents::output` goes away; the other callbacks remain as `AppEvent`s.
4. **Frames and attach.** For each session, one lock covers assigning the sequence number, pushing an `Output` frame onto every subscriber's bounded queue, and parsing into the model, in that order. `attach(session_id)` takes the same lock, serializes the snapshot and registers the subscriber. A full queue drops that subscriber and pushes `Resync`. `Resized` frames go to every subscriber except the one that resized.
5. **The snapshot.** It reads alacritty's private state (the inactive grid, scroll region, title and title stack, tab stops, keyboard mode stacks, active charset) through read-only accessors added to a vendored copy of the pinned crate: `vendor/alacritty_terminal/`, wired in with `[patch.crates-io]` and documented in `RUNNER_PATCH.md`. Jason chose this on 2026-10-05 over Runner tracking that state itself, which would be a second tracker that must agree with alacritty and still could not recover the inactive grid. It reverses the GPUI rewrite's "upstream only" rule for this one crate. The copy is vendored during phase 1; if the accessors are offered upstream, it moves to a fork pinned by commit, as Zed does. `runner_terminal::snapshot::serialize(&Term, unfinished: &[u8]) -> Vec<u8>` writes, in order:
   - a reset;
   - the primary scrollback and screen, as SGR runs with wide characters and zero-width marks;
   - the alternate screen, when active (`?1049h`, then its content);
   - the scroll region;
   - cursor position, style and visibility;
   - the title;
   - the modes: application cursor keys, bracketed paste (2004), mouse reporting (1000, 1002, 1003, 1006), focus reporting (1004), origin, autowrap, keypad and the kitty keyboard stack;
   - finally, `unfinished`.

   Unfinished input means two things. The first is the raw bytes since the model's parser was last at ground state; vte does not expose its state, so a small boundary scanner tracks ESC, CSI, OSC, DCS and SOS/PM/APC. The second is the bytes since an unmatched `ESC[?2026h`. Images and OSC 8 hyperlinks are not serialized, and the record lists them.
6. **The app's side.** `TerminalBridge` becomes the registry of mirrors, fed by `DaemonClient::attach` frames (a channel, in-process). Keys, pastes and named keys are encoded against the mirror, then sent with `client.input(id, bytes)`, one-way. Mission panes and chats both queue now, and the daemon applies `inject_direct_stdin` semantics in order. Resize: the mirror resizes at once, then `client.resize` goes one-way.
7. **The agent-update modal.** `UpdateTerminalEvents` goes away. The modal's unlisted session is attached by id like any other.
8. **#709's audit, answered in the record:** synchronized deadlines, partial writes and backpressure, read fairness, resize ordering, replies, EOF and child exit, draining final output before an exit is published, and shutdown. Each item is classified as an upstream guarantee, a Runner obligation with its test, or a known gap.

**Verification.**

- **The round-trip test:** every recording, replayed, serialized and replayed again, yields the same grid, scrollback, cursor and modes.
- **The split-point test:** for every recording, a snapshot at every chunk boundary and at 200 seeded random offsets per recording, then both copies are fed the rest. The final states must be identical.
- The input-state fixture goldens are unchanged; they move with the model.
- A DA1 query gets exactly one reply with zero, one and two mirrors attached.
- **The benchmark against the baseline:** p99 echo at most 1 ms above `main`, and burst throughput at least `main`'s.
- **Jason's smoke:**
  - Claude Code and Codex chats, and a mission;
  - IME, paste, selection and scrolling;
  - drag-resizing a busy pane;
  - the agent-update modal.

### Mission 1c — the daemon process

Brief `645-m3-daemon-process.md`. Crew: codex trio. The QA slot runs live checks against `make run` development data under live-test authorization.

**Before it starts:** measure named-pipe round trip and throughput on the Windows PC (`ssh pc`), and record the numbers beside the Mac's.

**Design.**

1. **The binary.** On Windows the sidecar install also copies `conpty.dll` and `OpenConsole.exe` into `<app data>\bin`, because `portable-pty` loads the DLL from beside the running executable; without it, sessions fall back to the inbox conhost (#492). Like `runnerd.exe`, they are replaced only while no daemon runs. `runner-cli`'s `main` dispatches on its file stem: `runnerd` (case-insensitive, `.exe` removed) runs `daemon::run`, and any other name is the CLI. `cli_install::install_runner_cli` installs the sidecar as both `runner` and `runnerd`, and skips `runnerd` while `runnerd.lock` is held. `fs2` and `sha2` are already in `Cargo.lock`; check whether they are direct dependencies before adding anything.
2. **Startup moves.** These move to `runner-backend::daemon::boot`:
   - `bootstrap::boot_core` and `NativeMcpServer`;
   - stale-row cleanup, the orphan sweep and router mounting;
   - starting discovery and usage;
   - `consume_resume_on_launch`, which sizes sessions from the persisted columns.

   The app's bootstrap keeps the paths, the sidecar install, connect-or-spawn and the handshake.
3. **Settings.** `runnerd` reads what it needs from `ui-settings.json` at start: the enabled runtimes (for usage and model discovery), and resume on launch. The mission permission mode setting is retired, so 1c keeps the session manager's default and reads no key for it. The app keeps pushing changes as requests; `usage_set_enabled` replaces `core.usage.set_enabled`, for example. Each setting has one owner, and the PR carries the inventory.
4. **Transport.** `runnerd.sock`, or `\\.\pipe\com.wycstudios.runnerd` (`-dev` for debug builds), through `IpcListener`, owner-only. On Windows that takes an explicit security descriptor, because a pipe created without one grants read access to Everyone; the existing CLI pipe gets the same fix, and both keep rejecting remote clients. A frame is a u32 length, a u8 kind and a payload: JSON for control frames, binary for terminal frames. `runnerd` serves on the existing tokio IPC runtime. The app uses one blocking reader thread and one writer per connection, as the rest of its I/O does, feeding the event thread and the mirror registry.
5. **Connect or spawn.** Connect with a 500 ms limit.
   - Nothing running: spawn `runnerd` detached (on macOS `setsid` in `pre_exec`, null stdio and the home directory as cwd; on Windows `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` and deliberately not `DETACHED_PROCESS`, so `runnerd` has a hidden console its console children inherit), then retry the connection for up to 10 s. On Windows, when breakaway is refused: the CLI refuses to start a daemon (it is inside another Runner session's job, which would take the daemon down with it) and tells the user to open Runner; the app retries without the flag and logs it.
   - Blocked (`EPERM`, as in `ClientError::Blocked`): show an error state and spawn nothing.

   `runnerd` takes `runnerd.lock` (`flock`, or `LockFileEx` on Windows) before it binds, and a second starter exits 0.
6. **A clean environment.** At start, `runnerd` removes `RUNNER_*` variables, and `PATH` entries under `<app data>/bin` and the mission shim tree, from its own environment.
7. **The handshake.** `Hello { exe_sha256, client }` gets `Welcome { exe_sha256, pid, started_at }` or `Mismatch`. `runnerd` hashes its own executable at start, and the app hashes its bundled sidecar (found through `locate_source`) once at launch, off the main thread. In 1c, a mismatch stops the old daemon with `Shutdown { stop_sessions: true }` and starts the new one without asking; the dialog comes in 1d.
8. **Lifetime.** `runnerd` keeps running once started, with no idle exit. It stops only on Stop Sessions, `runner daemon stop`, the OS ending it (item 10), or an update restart. With no client connected, it pauses usage polling and discovery refreshes, and resumes them on the next connection. Tests stop their daemon explicitly.
9. **Quit, unchanged.** `on_app_quit` sends `Shutdown { stop_sessions: true }`: stamp `resume_on_launch`, `kill_many`, exit. What is new is that an app crash or force-quit leaves `runnerd` and every session running.
10. **The OS ending `runnerd`.** On SIGTERM (macOS logout or restart), and on `CTRL_LOGOFF_EVENT`, `CTRL_SHUTDOWN_EVENT` and `CTRL_CLOSE_EVENT` (Windows, through its hidden console), `runnerd` runs the same Stop sessions path before exiting. Otherwise a reboot would bring sessions back stopped instead of resumed, which would be a regression: today macOS quits the app at logout, and its quit handler stamps the sessions. Test it by sending SIGTERM to a test daemon and checking the stamps, and add a logout-and-login check to QA's list.
11. **Reattach.** At launch the app restores its windows, loads `AppStore` through requests, and attaches every live session: snapshot first, then the mirror.
12. **Crash recovery.** If the connection drops, the app shows a notice and reconnects or spawns a new daemon. After three restarts in five minutes it stops and points to `runnerd.log`.
13. **The CLI.** When a socket command gets `NotRunning`, the CLI spawns `runnerd` from its own directory, canonicalized so that a `~/.local/bin` link resolves to the sidecar. It then waits up to 10 s for `runnerd.sock` and continues. It never spawns one when `SSH_CONNECTION` is set, and says to open Runner on that machine instead (see Phase 3). `runner daemon status` and `runner daemon stop` use `runnerd.sock`.
14. **The app stops depending on the backend.** `runner-backend` moves to `runner-app`'s `[dev-dependencies]`, and the 1a guard test is deleted (see Crates).
15. **Logs.** `runnerd.log` sits beside `runner.log`, with the same rotation and panic hook; the setup in `runner-app/src/logging.rs` moves somewhere both can share.

**Verification.**

- **Integration tests** in `runner-cli/tests`, each against a temporary app-data directory and endpoint:
  - start `runnerd` and complete the handshake;
  - spawn a `shell` session running `cat` and get the echo back;
  - disconnect and reattach, getting an identical snapshot;
  - kill `runnerd` and confirm the child has gone, on both platforms;
  - `runnerd` keeps running after the last session and client, with usage polling and discovery paused until a client connects;
  - a second starter loses the lock;
  - the CLI starts the daemon.
- The benchmark again, now across the socket.
- **QA live checks against `make run`:**
  - a chat and a mission survive an app force-quit;
  - `ask_human` while the app is closed;
  - `runner daemon status` and `stop`, with no orphan processes after a stop;
  - the privacy check: an agent reads `~/Documents` after the app quits, with no new prompt.
- **On the Windows PC:** detaching, breaking away from the app's job, and kill-on-close when `runnerd` dies; the startup log shows the bundled `conpty.dll` loaded, not the inbox conhost; a Codex redraw arrives whole; no console window flashes during discovery, version probes or usage reads; another account cannot open the pipe; and a `runner` command run inside a Runner session with no daemon running refuses to start one.

### Mission 1d — lifecycle

Brief `645-m4-lifecycle.md`. Design comes first: the spec's Design list, drawn in `design/specs/645-session-host.pen` and signed off by Jason. Crew: codex trio.

**Design.**

1. **Settings → General.** One "When Runner quits" row (`quitBehavior`: `ask`, `keep` or `stop`; default `ask`), a menu, with the live session count in its description. There is no `runnerd` status row and no Stop button (design review, 2026-10-06).
2. **The quit dialog.** Two choices, Keep them running and Stop them, the second naming any agent that is mid-turn, then Cancel and Quit. The last choice is preselected and Enter quits. It appears when sessions are live, on ⌘Q, on the Quit menu item and when the last window closes on Windows. "Don't ask again" writes the selected choice into the setting. ⌥⌘Q, and the Quit menu item with Option held, is Quit and Stop Sessions, without a dialog. A quit the OS starts (logout, restart) never asks and leaves sessions to `runnerd`; the daemon stamps and stops them when the OS ends it.
3. **A different build restarts without asking.** As 1c already does, the app stops the old daemon with stamping, installs the new sidecar, starts a new daemon and resumes. 1d adds the notice "Restarted N sessions in Runner <version>". There is no mismatch dialog. The notice that `runnerd` stopped gets proper wording, and after three crashes an Open log button.
4. **Updates.** The normal update dialog says the sessions restart with it; when agents are mid-turn, a small note beside the buttons says how many ("1 agent working"). Sparkle’s delegate and the Windows installer path mark an update quit, which never asks and leaves `runnerd` running. The new build’s mismatch restart stamps and stops sessions and resumes them. Sparkle’s window and flow are unchanged.
5. **The rename.** `runner-backend` becomes `runner-daemon`, in its directory, its package name and every `runner_backend::` path. By now those are only in the CLI, the crate's own tests and the docs.
6. **Docs.**
   - Arch §1, §5.5, §5.8, §11, and bets 3, 10 and 12.
   - `concurrency.md`, which now has three processes.
   - Vision §4.2 and `windows.md`.
   - The development notes in `AGENTS.md`: `make run` restarts the development daemon, and `runner-dev daemon stop` stops it.
   - The process map from the `fix/647-terminal-black-sidebar` branch, updated to the new shape.

**Verification:** the spec's phase 1 list, then the [full smoke test](../../tests/full-smoke-test.md), then 0.13.0.

## After phase 1 — the CLI moves to the client protocol

This and phases 3 and 4 are tracked in [#808](https://github.com/yicheng47/runner/issues/808) since 2026-10-06; #645 closes with phase 1.

Added on 2026-10-05 at Jason's request. One mission, on `main` after the umbrella lands, before phase 3 begins. It does not gate 0.13.0.

**Why.** Through phase 1, `runnerd` serves two protocols over one core: MCP on `mcp.sock` for the `runner` CLI, unchanged on purpose, and the client protocol on `runnerd.sock` for the app. Since 0.11 the CLI is MCP's only client, because no agent registers Runner's MCP server, so the MCP layer is an internal transport that can be swapped. Keeping both costs three things:
- Every operation is registered twice, as an MCP tool and as a request in the table.
- MCP's framing lacks what is coming: pushed events (`mission feed --follow`), the caller's identity on the connection, and a plain byte stream that can be forwarded over ssh. The 648 program record flagged this when it shipped.
- Phase 3 needs one protocol. A `runner` command run by an agent on a remote machine reaches its mission through that machine's daemon and the ssh link.

**What changes.**

- The CLI's socket commands become `DaemonClient` calls over `runnerd.sock`. `mission feed --follow` becomes a subscription.
- The MCP server, tool registry and all `rmcp` dependencies go. The old MCP endpoint remains only as an accept-and-close sentinel against a 0.12 app; its ownership check stays on Unix and Windows.
- The CLI's exhaustive recorder test checks against the request table, in place of the shared tool-name list.
- Inside a mission, `msg post`, `msg read`, `signal` and `ask` still append to the event log directly with no socket, as they do today.

**What must not change.** The CLI's output, including every `--json` shape, stays byte-identical, because agents' skills and scripts read it. Its exit codes stay the same: 3 for not running, 5 for blocked by a sandbox, and 4 reserved for #562's `wait`. So do the caller-identity rules (arch §9.3). The intended differences are that `runner status` reports `runnerd.sock` instead of `mcp.sock`, and the follow help describes pushed delivery. Golden tests capture the CLI's output on `main` before the change and compare it after.

**Crew:** codex duo. The #821 brief authorizes isolated daemon tests and no QA slot or live regression; Jason smoke-tests after implementation.

## Phase 2 — dropped

Phase 2 was to keep agents running through updates: a session daemon holding the PTYs below the `SessionRuntime` seam, which an update would not restart, speaking a session protocol kept compatible across releases. Jason dropped it on 2026-10-06 (spec decision 13). An update restarts every session, the agents resume their conversations, and the update dialog shows when agents are working before their turns are lost. The session daemon and its versioned protocol move to phase 3, which needs them for remote machines.

## Phase 3 — remote machines over ssh

This is an outline, detailed before its first mission. A session daemon runs on another machine and is reached through `ssh <host> runnerd --stdio`. This phase builds it, with what was outlined for phase 2:

- **The session daemon** holds everything below the `SessionRuntime` seam: the PTYs and children, the authoritative `TerminalModel`, the hook watchers and key capture.
- **The local `runnerd` talks to it over the session protocol:** spawn with a launch spec, stop, input, resize, status, terminal frames, agent events, draft observations and exits. On reattach the session daemon sends a status snapshot so the local reducer can rebuild.
- **The session protocol is versioned and compatible across releases,** because the remote's Runner can be a different build. Changes are additive, an incompatible change bumps the version, and a mismatch shows "Update Runner on <host>".

**ssh never starts the remote daemon** (2026-10-05, from Paseo). Paseo's ssh transport is `ssh -T -o BatchMode=yes -W 127.0.0.1:<daemonPort> <host>` (`packages/protocol/src/ssh-transport.ts`), a pipe to a daemon that is already running. Its connectivity docs say "It does not install, start, or configure Paseo on the remote host", and list "start the Paseo daemon on the remote host" as a prerequisite. Runner does the same. `runnerd --stdio` only connects, and the remote daemon runs in the user's own logon there: Runner open on that machine, `runner daemon start` from a terminal there, or an opt-in start at login (a LaunchAgent on macOS, a logon task on Windows). This phase designs that option. Agents then never run inside an ssh logon, so ssh session cleanup cannot kill them, and Keychain, DPAPI and Credential Manager behave as they do at the desk. The same rule applies on Macs, where a daemon started over ssh could leave Claude Code unable to read its credentials from a locked login keychain.

The spec's Remote machines section has the rest:
- the #795 launch steps move below the seam;
- the remote `runner` CLI reaches the mission through its daemon;
- the host picker is designed first;
- the session protocol's version check shows "Update Runner on <host>".

Supersedes [510](../../features/archive/510-remote-ssh-session.md).

## Phase 4 — the Windows PC

This is an outline: OpenSSH Server on the PC, `runnerd --stdio` on Windows with ConPTY and argv that needs no quoting layer, the `.cmd` first-turn paste and PowerShell hook reporters running on the PC, and a crew whose tester slot runs there.

The two Windows questions from 2026-10-05 no longer gate this phase, because of Phase 3's rule that ssh never starts the daemon:

- **Does a daemon outlive the ssh session?** It is never in one. The PC still confirms that the `--stdio` proxy dies with its connection while `runnerd` and its agents keep running. The `IsProcessInJob` probe written on 2026-10-05 shows what OpenSSH does to a session's processes; it could not run because the PC was off.
- **Can agents read their credentials?** They run in the user's desktop logon, as they do when Runner is opened there. The PC check is that every runtime the crew uses authenticates in a slot started from the Mac.

What remains specific to this phase: the user stays logged in on the PC (a test box can sign in automatically), and Runner or the start-at-login option runs there.

## Decisions that bind

- The spec's decisions 1–13.
- Every mission is one PR with one commit, and its brief's authorization section says so ([AGENTS.md](../../../AGENTS.md), Crew Missions).
- Phase 1 missions branch from `feat/645-runnerd`, open their PRs against it, and never merge into `main` on their own; only the umbrella lands on `main` (Branch and channel).
- **No test starts a daemon in real app data.** A test that spawns `runnerd` uses a temporary app-data directory and endpoint, passes every root in rather than resolving `$HOME`, and kills the daemon on drop.
- Imports used only by `cfg(unix)` tests are `cfg(unix)`-gated, after Windows clippy failed on four PRs.
- No new dependency without a reason in the handoff.
- **The CLI in the build tree is `target/debug/runner-agent-cli`.** `target/debug/runner` is the GUI app on a case-insensitive volume, and every brief says so.

## Risks

The spec lists the product risks. These belong to the plan:

- **1a conflicts with any app work running at the same time.** Sequencing above.
- **1b's snapshot is the hard part, and it is why #157 failed.** The split-point test and the fresh recordings are its gate, and nothing merges without them.
- **1c's privacy (TCC) attribution and Windows job breakaway** are unknown until they are tested live; both are in QA's list.
- **1d’s Sparkle hook.** The relaunch delegate must mark the update before application termination. It does not defer relaunch, prompt, or change Sparkle’s window; the new build owns the daemon restart.
