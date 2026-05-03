# v0 MVP Test Plan

Reviewer-facing test plan for the implemented MVP. The codebase has detailed
Rust coverage next to the modules it exercises; this document is the practical
plan for PR review and release readiness.

Use two lanes:

- **Agent-run integration/regression checks:** commands Codex can run locally
  before handing a PR back.
- **Human smoke checks:** UI and terminal behavior that must be verified in the
  Tauri app with real PTY sessions.

## Agent-run integration/regression checks

Run from the repository root.

```sh
pnpm exec tsc --noEmit
pnpm run lint
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For focused regressions, use these when the touched area is narrow:

```sh
# CLI/event-log contract used by spawned agents.
cargo test -p runner-cli --test roundtrip

# Router, mission lifecycle, and PTY/session behavior.
cargo test -p runner --lib router::tests
cargo test -p runner --lib commands::mission::tests
cargo test -p runner --lib session::manager::tests
```

What these checks cover:

- TypeScript route/component type safety.
- ESLint rules for the React surface.
- Rust formatting, linting, and full workspace regression coverage.
- CLI sidecar behavior: `runner signal`, `runner msg post`, `runner msg read`,
  `runner status`, allowlist validation, roster validation, and missing-env
  handling.
- Router behavior: mission bootstrap, `ask_lead`, `ask_human`,
  `human_response`, `human_said`, `runner_status`, warning paths, and
  reconstruction from the mission log.
- Mission/session behavior: start/stop, no-runner validation, pending ask
  counts, PTY spawn/inject/output snapshots, resize, kill, and late attachment.

## Human smoke checks

Start the app:

```sh
pnpm tauri dev
```

Optional clean-state reset on macOS. `pnpm tauri dev` writes to the
`-dev` sibling directory so a packaged install's data isn't touched —
only wipe the dev dir:

```sh
rm -rf "$HOME/Library/Application Support/com.wycstudios.runner-dev"
```

Prepare two crews:

- **Demo crew:** at least two runners, one lead and one worker. Use shell
  runners for deterministic manual CLI checks; use a real `claude-code` lead
  for the final demo path.
- **Empty crew:** no runners, used to verify Start Mission validation.

### 1. Missions entrypoint

1. Open **Missions** from the sidebar.
2. Confirm Active/Past tabs render and counts update.
3. Click **Start mission**.
4. Select the empty crew.
5. Confirm the modal warns that the crew has no runners and the Start button is
   disabled.

### 2. Start a mission

1. Open **Start mission** again.
2. Select the demo crew.
3. Enter a title, goal, and working directory.
4. Start the mission.

Expected:

- App routes to `/missions/:id`.
- Header shows the mission status as running.
- Feed shows opening mission events.
- Runner rail lists every crew member and marks the lead correctly.
- A PTY tab exists for each runner.

### 3. Terminal switching and scrollback

1. Open the lead PTY tab.
2. Type a simple command, for example `echo lead-smoke`.
3. Switch to the worker PTY tab and type `echo worker-smoke`.
4. Switch back and forth between Feed, lead, and worker tabs.
5. Route away from the workspace, then reopen the mission from the Missions
   list.

Expected:

- The visible terminal switches to the selected runner.
- Output remains attached to the correct runner.
- Scrollback is preserved while the app process is alive.
- Reopening the workspace reattaches live sessions and restores the current
  terminal snapshot.

### 4. Message and signal loop

In the lead PTY:

```sh
runner msg post --to <worker-handle> "start smoke task"
```

In the worker PTY:

```sh
runner msg read
runner signal ask_lead --payload '{"question":"Need direction?","context":"smoke"}'
```

Expected:

- Worker inbox includes the directed message.
- Feed shows the message and `ask_lead` signal.
- Lead PTY receives the ask-lead injection.
- Raw PTY output stays in terminal tabs, not in the event feed.

### 5. Human-in-the-loop card

In the lead PTY:

```sh
runner signal ask_human --payload '{"prompt":"Use A?","choices":["yes","no"],"on_behalf_of":"<worker-handle>"}'
```

Expected:

- Feed renders an ask-human card.
- The card attribution shows the worker/lead/user chain.
- Missions list shows a pending count for this mission.
- Reopening the mission keeps the pending card visible.

Click `yes` on the card.

Expected:

- Feed appends `human_response`.
- The card resolves.
- Pending count clears.
- The response is injected back to the runner that emitted the matching
  `ask_human`.

### 6. Human message input

From the Feed tab input:

1. Send a message with no explicit target.
2. Send a message targeted to the worker.
3. Clear the target and send a broadcast if the UI exposes that state.

Expected:

- Untargeted human input lands on the lead.
- Targeted human input lands on the selected runner.
- Feed records `human_said` events without confusing them with PTY output.

### 7. Runner status

In the worker PTY:

```sh
runner status busy --note "working"
runner status idle --note "ready"
```

Expected:

- Runner rail updates busy/idle state.
- Non-lead idle status injects a short availability notice into the lead PTY.
- Reopening the mission reconstructs latest runner status from the log.

### 8. Stop and reopen

1. Click **Stop** in the mission workspace.
2. Return to Missions.
3. Open the Past tab.
4. Reopen the stopped mission.

Expected:

- Mission leaves Active and appears in Past.
- Workspace no longer presents the mission as running.
- Feed replays historical mission events, including the stop event.
- Inputs that require a running mission are disabled or inert.

### 9. Direct chat regression

1. Open a runner detail page.
2. Start a direct chat.
3. Type a command and confirm output.
4. Start or open a second direct chat session for another runner.
5. Switch between sessions from the sidebar SESSION list.
6. Route away and back, or reload while the app process is still running.

Expected:

- Switching sessions switches the visible PTY.
- Each session keeps its own scrollback.
- Reattach restores the active session terminal snapshot.
- Direct-chat sessions do not require mission env vars; `runner` CLI commands
  no-op cleanly when there is no mission bus.

## Final demo path

Run this once before declaring v0 complete:

1. Create a crew with a real lead and worker.
2. Start a mission from the Missions page without DevTools.
3. Lead receives the goal, posts work to the worker, and worker reads it.
4. Worker emits `ask_lead`; lead escalates with `ask_human`; human clicks a
   choice; the response reaches the asker.
5. Send a human message from the workspace input and verify it lands on the lead
   by default.
6. Close and reopen the mission; feed, pending asks, runner status, and live PTY
   attachment reconstruct correctly.

## Scope notes

- There is no browser-driven UI E2E suite in v0; UI correctness is covered by
  the smoke checks above.
- macOS and Linux are the MVP targets. Windows behavior is out of scope.
- Production sidecar packaging is not part of the MVP smoke gate; dev mode must
  build and install the `runner` CLI into `$APPDATA/runner/bin`.
- Terminal scrollback is an in-process session snapshot, not a durable replay
  file. Durable mission history lives in `events.ndjson` and is shown in the
  feed.
