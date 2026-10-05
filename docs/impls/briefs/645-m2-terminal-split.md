# 645 m2 — The terminal moves below the session seam (runnerd 1b)

Program [#645](https://github.com/yicheng47/runner/issues/645) (`runnerd`), phase 1, mission 1b. Jason requested this mission on 2026-10-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-645-m2-terminal-split`, on the existing branch `refactor/645-m2-terminal-split`, created from the umbrella `origin/feat/645-runnerd` at `24e1e78b`. That commit is `main` plus the umbrella's CI commit plus mission 1a. The root checkout stays on `main`. Do not create another branch or worktree, and do not share a Cargo target directory. This lands on the umbrella, never on `main`.

## The goal

Today each live session's terminal (`TerminalSession`, an alacritty `Term`) lives in the app. It answers terminal queries, persists the live title, scans OSC 7 for the shell's live cwd, and derives the draft state that the delivery gate reads. In phase 1c the core moves into a daemon, so all of that must already sit beside the session layer, with the app keeping only a mirror it paints. **This mission splits the terminal into a daemon-side `TerminalModel` and an app-side `TerminalMirror`, connects them with sequenced frames and a VT snapshot on attach, and inverts the crate dependency.** Everything still runs in one process, and behaviour stays identical. This is the mission PR #157 failed at in July, so its tests are the gate (see Verification).

## Read first

- `AGENTS.md`, especially Worktrees and Crew Missions.
- `docs/features/645-session-host.md`: "The terminal: one authoritative copy in the daemon, a mirror in the app", including "This was tried before", "The client protocol", and the Windows section.
- `docs/impls/645-runnerd/plan.md`: "Mission 1b" and "#709 does not go first". The design is there; this brief wins on any detail.
- `docs/impls/645-runnerd/mission-1a.md`, 1a's record of the request surface you build on.
- `crates/runner-terminal/src/terminal.rs` at `24e1e78b`:
  - `TerminalSession` (335);
  - `attach_with_input_mode` (420–645): the input worker at about 431, the database reads at about 467, the event worker with its replies and live-title persistence at about 534–631, and the sync flusher at about 637;
  - `feed_output` (702);
  - the input methods (822–925);
  - `resize` (928);
  - the link-cwd database read (1188);
  - `TerminalBridge` and its `SessionEvents` impl (1445–1600).
- `crates/runner-terminal/src/{input_state,replay,fixtures,mappings,palette}.rs` and `fixtures/`.
- `crates/runner-backend/src/session/manager/output.rs`: `ingest_output_chunk` and `record_output` (887–918), where sequence numbers are assigned under the per-session lock. Also `manager/mod.rs`: `SessionEvents`, `OutputEvent`, `report_input_state`, `inject_stdin` and `inject_direct_stdin`.
- The app's terminal users: `main.rs`, `terminal_ime.rs`, `ui/scrollbar.rs`, `terminal/element.rs`, `surfaces/{agent_update,app_shell,start_chat,chat}.rs` and `surfaces/mission_workspace/{attach,feed}.rs`.
- `crates/runner-app/tests/request_surface.rs`, whose allowlist entries marked 1b this mission removes.
- `docs/tests/archive/791-session-state-reducer.md`, for how QA ran the development app and recorded fixtures last time.

## Stage 0, before any code change

1. **QA records fresh fixtures** on the development build at `24e1e78b`, with `RUNNER_RECORD_INPUT_FIXTURE` set in the app's environment (see 791's record):
   - Claude Code with a long transcript, `/clear`, and a window resize;
   - Codex on the alternate screen, with a resize;
   - pi, including Shift+Enter;
   - zsh running `less` on a long file, then `vim`.

   Add them to `crates/runner-terminal/fixtures/` with snapshots, in the existing format. Keep each recording under 2 MB, trimming idle tails. The corpus has 14 recordings today.
2. **The coder adds a benchmark harness** that works before and after the change. It measures keystroke-to-echo through a `shell` session running `cat` (10,000 keystrokes, p50 and p99), and a 50 MB burst through `cat` of a file (wall time until the last byte reaches the app's `Term`). Mark it `#[ignore]` so CI skips it. Run it on `24e1e78b` on macOS for the baseline. Windows is measured before 1c, at Jason's decision of 2026-10-05.
3. **QA runs 1a's regression smoke list** on the `24e1e78b` development build, because 1a merged without a smoke test, by Jason's decision. The list is at the end of `docs/impls/briefs/645-m1-request-surface.md`:
   - a direct chat and a role chat: type, stop, resume, fork;
   - a mission: start, stop and resume a slot, archive;
   - create, edit and delete roles, crews and projects;
   - every Settings pane;
   - the sidebar: pin, rename, drag, a second window.

   Report failures to the lead at once, because a 1a regression is fixed on the umbrella before 1b builds on it.

Record all three in `docs/tests/645-m2-terminal-split.md`: what ran, on which commit, the numbers, and the results.

## Deliverables

1. **The crate boundary inverts.** `runner-terminal` drops its `runner-backend` dependency. Every backend call `TerminalSession` makes today goes through a small trait the owner implements: write input, write a reply, report the input state, set the live title, and resize the PTY. `runner-backend` depends on `runner-terminal`; the app depends on both. Add no new external dependencies.
2. **Two types share one parse.**
   - `TerminalModel`, on the daemon side, holds the `Term`, parser, synchronized-update flush, colour-scheme and OSC 7 scans, input tracker and fixture recorder. It writes query replies to the PTY. It persists the live title through the session layer and keeps the live cwd.
   - `TerminalMirror`, on the app side, holds the `Term`, parser, synchronized-update flush, viewers and waker, selection, scroll, links and key encoding. It drops its query replies and has no input tracker.

   Both call one shared parse function, so they cannot parse differently.
3. **The model lives in the session layer.**
   - `SessionManager` creates a session's model at spawn and feeds it from `ingest_output_chunk`.
   - `SessionEvents::output` goes away.
   - The app reads the live title, live cwd and link cwd through requests added to the 1a table; `fast` where they read in-memory state.
   - The input tracker's observations reach `report_input_state` without the app.
4. **Frames and attach.**
   - For each session, one lock covers four steps, in order: assign the sequence number, push an `Output` frame onto every subscriber's bounded queue, then parse into the model.
   - `attach(session_id)` takes the same lock, serializes the snapshot, and registers the subscriber.
   - A full queue drops that subscriber and pushes `Resync`, and the subscriber re-attaches.
   - `Resized` frames go to every subscriber except the one that resized.
   - The transport is the 1a in-process one: the frame channel and the attach call go through `DaemonClient`.
5. **The snapshot.** `runner_terminal::snapshot::serialize(&Term, unfinished: &[u8]) -> Vec<u8>` writes, in order:
   - a reset;
   - the primary scrollback and screen, as SGR runs with wide characters and zero-width marks;
   - the alternate screen when active, as `?1049h` then its content;
   - the scroll region;
   - cursor position, style and visibility;
   - the title;
   - the modes: DECCKM, 2004, 1000/1002/1003/1006, 1004, origin, autowrap, keypad, and the kitty keyboard stack;
   - finally, `unfinished`.

   `unfinished` is the raw bytes since the model's parser was last at ground state. vte does not expose that state, so a small boundary scanner tracks ESC, CSI, OSC, DCS and SOS/PM/APC. Add the bytes since an unmatched `ESC[?2026h`. Images and OSC 8 hyperlinks are not serialized; list them in the record.
6. **The app's side.**
   - `TerminalBridge` becomes the registry of mirrors, fed by attach frames.
   - Keys, pastes, named keys and IME text are encoded against the mirror and sent with `client.input(session_id, bytes)`, one-way and ordered.
   - Every session now queues its input the way mission panes do, applied by a per-session worker in the session layer with today's `inject_direct_stdin` semantics. Keep the `session/input-error` event for failures.
   - Resize: the mirror resizes at once in the same call, as today, then `client.resize` goes one-way.
   - Draw reads only the mirror.
7. **The agent-update modal.** `UpdateTerminalEvents` goes away. The modal's unlisted session is attached by id like any other.
8. **The guard test** loses its 1b entries: `terminal/`, `terminal_ime.rs`, `surfaces/agent_update.rs`, and the `TerminalBridge::new` construction in `app_store.rs`.
9. **#709's audit**, answered in the test record. Classify each item as an upstream guarantee, a Runner obligation with the test that covers it, or a known gap:
   - synchronized-update deadlines;
   - partial writes and backpressure;
   - read fairness;
   - resize ordering;
   - replies;
   - EOF and child exit;
   - draining final output before an exit is published;
   - shutdown.

## Rules

- **No behaviour change**, beyond the deliberate one: direct chats now queue their input, as mission panes already do. Keep every call on the thread it ran on before; the 1a correction stands. Add no new ordering queues, generations or stale-completion gates to make a moved call safe.
- **Only the model answers terminal queries.** A query is answered exactly once, whether zero, one or two mirrors are attached.
- **Before every handoff, check the Windows build.** Run `cargo check --locked --workspace --all-targets --target x86_64-pc-windows-msvc` if the toolchain allows. Otherwise, list every `cfg` gate and every moved `use` and verify each by hand. Windows CI has failed on unused or missing imports in five missions, including 1a's first push.
- Stage by path, never `git add -A`.
- **The coder and the reviewer do not run the app.** The CLI in the build tree is `target/debug/runner-agent-cli`; never run `target/debug/runner`, which is the GUI. Only QA runs the development app (next section).
- No extra agents.

## QA's authorization

Jason authorized live tests on 2026-10-05, for QA only, on the development app and development data:
- Run `make run`, with or without `RUNNER_RECORD_INPUT_FIXTURE`.
- Start Claude Code, Codex and pi chats and shell terminals, and one test mission on a test crew it creates.
- Never touch the production app, production data, other people's chats, or any `~/.claude`, `~/.codex` or `~/.pi` settings.
- Archive every chat and mission it created, and delete any test role or crew, at the end.
- Note the cost of each live agent session in the record.

**After implementation, QA runs these checks on the branch's development build.** Run each side by side with the `24e1e78b` build where feel matters:
- typing echo;
- a large output burst;
- drag-resizing a busy pane and split panes;
- IME (Pinyin) and paste;
- selection and copy, scrollback, links;
- OSC 7 live cwd in zsh;
- live titles;
- colour queries across a theme switch;
- pi Shift+Enter, through the kitty keyboard;
- the agent-update modal;
- mission panes' queued input;
- a typed draft holding a crew delivery (the #791 smoke);
- one session in two windows;
- relaunching the app (which still kills sessions until 1c; check that nothing regressed);
- 1a's smoke list again.

## Verification

1. **The round-trip test.** Every recording, replayed into a `Term`, serialized and replayed into a fresh `Term`, gives the same grid, scrollback, cursor and modes.
2. **The split-point test.** For every recording, take a snapshot at every chunk boundary and at 200 seeded random byte offsets, including inside escape sequences and synchronized updates. Feed the rest to both the original and the restored `Term`; the final grids, scrollback, cursor and modes must be identical. Validate the test with a negative control: drop one mode from the serializer in a scratch copy and confirm it fails.
3. **The one-reply test.** A DA1 query gets exactly one reply with zero, one and two mirrors attached.
4. **Existing tests.** The input-state fixture goldens are unchanged; they move with the model. Every existing test passes. Record workspace test counts (`--profile ci`, with pipefail) at `24e1e78b` and after.
5. **The benchmark on macOS against the baseline.** p99 echo is at most 1 ms above the baseline, and burst throughput is at least the baseline's.
6. **The commands:** `make verify`, workspace clippy with `-D warnings`, `--features updater` clippy, `cargo fmt --all --check`, `git diff --check`, and the Windows check above. Record each exit code.
7. **The size budget.** Snapshot goldens, if any, stay small; the 791 lesson is that its first goldens reached 57,650 lines.

## Review

The coder implements and hands off on the Runner feed. The reviewer reviews the full diff against this brief, must-fix first with file:line. It checks:
- the single-lock invariant, and that no subscriber can miss or repeat a chunk;
- that only the model replies;
- the snapshot's completeness against alacritty's mode set;
- the split-point test's coverage;
- no behaviour change beyond queued direct-chat input;
- the Windows gates.

Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. QA then runs its post-implementation checks. A QA failure goes back to the coder, and the reviewer re-reviews the fix.

## Authorization

After a clean review and QA's checks pass, Jason authorizes the following:
- Squash all work on this branch, this brief and the test record included, into one commit on top of current `origin/feat/645-runnerd`, with a subject that names the change (for example `refactor(terminal): split the terminal into a session-side model and an app mirror (#645 1b)`).
- If the umbrella has moved, rebase; never merge.
- Push the branch and open a PR against `feat/645-runnerd`. The body says `Refs #645` and carries the gates, the #709 audit table, the benchmark numbers and QA's results, with no Claude session link.
- Review or CI fixes are amended into the same commit and pushed with `git push --force-with-lease`.
- Drive `Rust / macOS` and `Rust / Windows` green.

Do not merge, delete the branch or worktree, or cut a nightly or release.

The final Runner handoff carries:
- the PR URL;
- what changed;
- tests and exit codes;
- the benchmark;
- CI;
- the reviewer's verdict;
- QA's summary, including the 1a regression results.

Then stand by.
