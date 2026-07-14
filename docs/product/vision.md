# Runner — Product Vision

> What we're building and why. The technical "how" lives in [`../arch/arch.md`](../arch/arch.md) — when this doc and the arch doc conflict on a protocol / schema / event-model question, the arch doc wins.

## 1. The problem

Coding agents like Claude Code, Codex, and aider are each powerful alone, but there's no good way to run several of them *together* on one machine with different roles, a shared view of what's happening, and a sane way to pull the human in when they disagree or hit a wall.

Today, coordinating two agents means juggling terminal windows, eyeballing logs, and manually relaying messages. It breaks down past one agent, and doesn't scale as people start combining specialists (coder + reviewer + tester + fixer).

## 2. The vision

A local desktop app where one person can:

1. Assemble a **crew** of CLI coding agents on their own machine.
2. Give each **runner** a role and a brief (the role's system prompt).
3. **Launch a mission** — one activation of the whole crew — and watch every runner's live output in one window.
4. Let runners **coordinate** through two channels: **signals** (typed, router-visible) and **messages** (prose, runner-to-runner or runner-to-human).
5. Get pulled in through the **lead runner** when a decision needs a human.

The same app also supports **direct chats** — one-on-one sessions with a single runner outside any mission — for quick "I just want to ask `@architect` something" loops without standing up a whole crew.

## 3. Vocabulary

These are the words the product surfaces to the user; they also map 1:1 to the architecture model (`arch.md` §3).

- **Runner** — a configured CLI agent (binary + args + role brief). Top-level config; reusable across crews.
- **Crew** — a named set of runner *slots* with exactly one lead.
- **Slot** — a position inside a crew, filled by a runner template. The slot carries the per-crew handle (`@impl`, `@reviewer`, …) and the lead flag. Two slots in different crews can both be filled by the same runner template.
- **Mission** — one live activation of a crew. Everyone spawns together, shares a coordination bus, ends together.
- **Session** — the live PTY process for one slot inside a mission, one runner-backed direct chat, or one runtime-only direct chat.
- **Signal** — a typed notification runners emit for the router to handle. Verb grammar (`ask_lead`, `ask_human`, `mission_goal`, `runner_status`).
- **Message** — prose posted to the mission. Broadcast or directed (`--to <handle>`).
- **Inbox** — each runner's projection of the mission: broadcasts plus directs addressed to me. Pull-based — runners check it on convention; nothing auto-interrupts a working runner.
- **Lead runner** — the human's counterpart in the crew. The mission goal lands on the lead; the lead dispatches work; the lead is the default HITL gateway when workers need a human.
- **Direct chat** — an off-bus, one-on-one PTY session with a single runner. No mission, no router, no inbox.

## 4. Product surfaces

The user-facing surfaces, described by the value they deliver, not by their implementation. Cross-references point at the arch doc for the "how."

### 4.1 Crew and runner library

- Create, edit, delete runner templates (handle, display name, runtime, command + args, working dir, system prompt, env).
- Compose crews from those runners by adding slots. Exactly one slot per crew is the lead. The same runner template can sit in many crews simultaneously.
- Optional per-crew **team conventions** addendum (`crew.system_prompt_addendum`) — Layer 2 of the prompt stack, spliced between the platform-injected coordination preamble (Layer 1) and the runner's persona (Layer 3). Lets a crew share house rules without editing every runner. See [arch §6](../arch/arch.md#6-system-prompt-composition).

### 4.2 Missions

- One-click **Start Mission** on a crew. The mission spawns one session per slot and opens the mission workspace.
- A mission has its own goal (optional override of the crew default) and its own working directory — the mission cwd is the authoritative working dir for every spawned slot, overriding the runner template's `working_dir`.
- Concurrent missions on the same crew are allowed; each one is fully namespaced (its own session set, event log, router state).
- **Stop Mission** kills the live PTYs but keeps the mission row running and resumable. Resume respawns stopped/crashed slots from their persisted session rows.
- **Archive Mission** is the terminal end state: it appends `mission_stopped`, marks the mission completed, sets `archived_at`, hides it from active lists, and leaves the workspace read-only by direct URL.
- **Sessions outlive the UI window, not the app process.** Closing or navigating away from the mission workspace does not kill sessions. Quitting Runner kills the in-process PTYs; on next launch, stale running rows are demoted to stopped and the user resumes them explicitly.

### 4.3 Live per-runner terminals (with human takeover)

- One PTY per slot, rendered with xterm.js for full TUI fidelity with first-class agent runtimes such as claude-code and codex.
- The xterm pane is a real terminal, not a log viewer. The human can type into any runner's stdin at any time — answer a prompt, correct a bad plan, kill a tool call, or just chat mid-flight. Human and router share the same writer path, so they are symmetric.
- Per-runner busy/idle is inferred from PTY-byte silence — agents do not have to call a status verb. Works for any TUI.

### 4.4 Coordination — signals and messages

- Runners emit signals via `runner signal <type> [--payload <json>]` and post prose via `runner msg post [--to <handle>] "<text>"`.
- Both flow through one append-only NDJSON file per mission. The file is tailable with `tail -f` for debugging.
- Signals drive fixed router handlers (wake the lead on `mission_goal`, surface HITL cards on `ask_human`, etc.). Messages are pull-based — the recipient picks them up on the next `runner msg read`.
- Messages stay flat. The product does not need separate thread or fact primitives; durable conclusions should land in the repo, docs, commits, or ordinary mission prose.

### 4.5 Human-in-the-loop

- Right-rail panel showing all pending `ask_human` cards. Each card shows the triggering signal, the prompt, the choices, and optional `on_behalf_of` attribution when the lead is escalating on a worker's behalf.
- Click a choice → answer flows back to the asker's stdin.
- In-app attention is the current contract: pending cards, sidebar/workspace indicators, and feed updates. Native OS notifications are not part of the shipped surface.

### 4.6 Mission workspace UI

- **Runner rail** — every slot in the crew with a busy/idle dot. Click to focus its terminal.
- **Focused terminal** — xterm.js view of the selected slot.
- **Event feed** — chronological view of messages plus user-visible signals for the mission. Router-internal signals (`inbox_read`, agent-source `runner_status`) are filtered.
- **HITL cards** — pending `ask_human` prompts, always visible.
- **Mission header** — crew, goal, cwd, start time, controls.

### 4.7 Direct chats

- Start a chat with a runner template or directly with a runtime (`claude-code`, `codex`) plus a working directory. No mission, no router, no bus — it's just a PTY between the human and the agent CLI. Useful for quick one-shots.
- Direct chat rows persist across app restarts. The live PTY does not; stopped rows can be resumed, and runtime-only chats reconstruct their ephemeral runner config from `agent_runtime` / `agent_command`.

### 4.8 App life

- **Auto-update** — the native app menu owns manual "Check for Updates..."; the toast surfaces available/downloading/ready states, with an explicit Restart button once installed.
- **Logging + crash reporting** — `tauri-plugin-log` writes to the OS log dir for the bundle; a panic hook captures backtraces; Help → Reveal Logs in Finder.
- **Theming** — Settings exposes Auto / Light / Dark appearance, light variants (Codex, Catppuccin Latte), dark variants (Carbon, Catppuccin Mocha), and separate terminal themes.

### 4.9 External control

- **MCP** — external Claude Code / Codex sessions can inspect and operate Runner through the bundled `runner-mcp` bridge: project discovery, crew/runner/slot CRUD, project-aware mission/direct-chat creation, and mission lifecycle, feed, and status tools. Runner.app remains the state owner; MCP is a local control surface, not a remote server.

## 5. The demo loop

The concrete user flow the product must support end-to-end:

1. User creates a crew called *Feature Ship*.
2. User adds two slots:
   - `coder` (Coder) — runtime `claude-code`, brief "Implement feature X. When ready, signal `review_requested` and post a message explaining what changed."
   - `reviewer` (Reviewer) — runtime `claude-code`, brief "When review is requested, read `coder`'s messages and the diff, then signal `approved` or `changes_requested` and post messages with specific feedback."
3. User clicks **Start Mission**, picks a cwd, and writes a goal. Both PTYs spawn. The lead receives the composed launch prompt via stdin.
4. Coder writes code, runs tests, then:
   - `runner msg post "Branch feat/x is ready. I refactored auth and added session tests."`
   - `runner signal review_requested`
5. Reviewer reads its inbox, sees the broadcast, and reviews the diff.
6. Reviewer signals `changes_requested` and posts specific feedback as a directed message to `coder`.
7. Coder responds; cycle continues until Reviewer signals `approved`.
8. If at any point a worker needs human input, it emits `ask_lead`. The lead can answer from context or escalate via `ask_human` (carrying `on_behalf_of: "@coder"`). The HITL card pops; user clicks a choice; router injects the answer into the lead's stdin; lead forwards via a directed message.
9. User clicks **Stop** when pausing work, or **Archive** when the mission is done. Archive terminates the run, marks the mission `completed`, and removes it from active mission lists while keeping a read-only record.

If this loop doesn't work end-to-end without the user touching a terminal outside the app, the product hasn't shipped.

## 6. Non-goals (long view)

These are intentionally out of scope — they belong to a different product or a later phase.

- Cross-mission memory / persistent crew brain.
- Remote runners / SSH / multi-host coordination bus.
- Sandboxing beyond the child process's own permissions.
- Cost tracking / observability dashboards.
- Marketplace of runner templates.
- Multi-human collaboration on the same mission.
- Thread/fact primitives for mission coordination.
- Secrets management beyond plain env vars.
- LLM-based signal routing (the router is a flat dispatcher by design — the lead owns coordination judgment).
- Windows desktop support (macOS + Linux only for the foreseeable future).

## 7. Open product questions

Decisions we have not taken; revisit when the product surfaces them.

1. Should the router include recent messages as context when injecting stdin on a signal, or is the recipient expected to call `runner msg read` itself? *Today: the latter.*
2. Should `runner msg read` paginate, or always return everything sorted by ULID? *Today: returns everything; clients filter with `--since`.*
3. User-defined signal types — today the CLI's `KnownSignalType` enum is the closed set; whether to let crews extend it (and where the router gains handlers for new types) is open.
4. Auto-restart semantics on crash. *Today: no auto-restart.*

## 8. Risks

- **PTY flakiness across platforms.** Targets macOS + Linux; Windows is deferred.
- **TUI rendering edge cases in xterm.js.** Claude / codex use rich TUIs (alt-screen, OSC 8 hyperlinks, OSC 52 clipboard); every new TUI quirk is a tuning loop.
- **Agents that don't know the `runner signal` / `runner msg` conventions.** We ship sensible default briefs per runtime so even an untuned agent participates correctly.
