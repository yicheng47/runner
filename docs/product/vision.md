# Runner — Product Vision

> What we're building and why. The technical "how" lives in [`../arch/arch.md`](../arch/arch.md) — when this doc and the arch doc conflict on a protocol / schema / event-model question, the arch doc wins.

Current product description reconciled on 2026-10-22. The [roadmap](../roadmap.md) owns scheduling and distinguishes production releases, changes on `main` and remote work on its umbrella.

## 1. The problem

Coding agents — Claude Code, Codex, Copilot, pi, TRAE — are each powerful alone, and most people who use them seriously now hold subscriptions to more than one. Running several at once is already solved: give each its own terminal, window, or worktree and they work in parallel, isolated from one another. What is not solved is running several of them *together* — on one task, with different roles, a shared view of what's happening, and a sane way to pull the human in when they disagree or hit a wall.

Coordinating two agents by hand means juggling terminal windows, eyeballing logs, and relaying messages between them yourself. It breaks down past one agent, and it doesn't scale as people start combining specialists (coder + reviewer + tester + fixer).

The vendors are closing this gap, but only inside their own walls: a team or teammate mode spawns more of the same agent from the same provider. Nobody's coder can hand work to somebody else's reviewer. Filling that gap requires being neutral about whose agent sits in which seat, which is exactly what a vendor cannot be.

There is a second ceiling underneath the coordination one, and it is the person. Running more agents is cheap; holding more of them in your head is not. Past a handful of live sessions nobody can reliably track what each one is doing, which is blocked on which, and which one is waiting on an answer — so the binding constraint on this whole category is human attention, not compute and not how many subscriptions you hold. That is why the unit of work has to be the mission rather than the session: you supervise one task through a roster and a feed, and you drill into a terminal when something there needs you, instead of managing terminals as the primary act.

## 2. The vision

Runner is a **working environment for terminal agents**: the place several coding agents share while they work on one task. It provides roles, reusable crews, mission rosters, directed messages, pushed inbox notifications and a path to the human, while staying neutral about the provider. A saved crew has one lead; a mission can also start empty with no lead. Any provider, one workflow.

That neutrality is the product, and it is cheap to honor, because everything an agent needs in order to take part is something every CLI agent already has: a shell, a PTY, and a binary on PATH. Runner supplies the place (a mission), the identity (a handle in a roster), the channel (an append-only bus), and the escalation path (the lead, then the human). It does not wrap an editor around an agent — the agent brings its own tools. Planned file and diff review stays read-only, with comments returned to the agent that made the change. Runner is what sits *between* agents.

An agent can be the caller too: it starts a mission through the same CLI the person uses, while the person follows the real sessions in the app. The [agent-environment direction](agent-environment.md) explains why this matters and which delegation verbs remain planned.

A local desktop app where one person can:

1. Assemble a **crew** of CLI coding agents on their own machine, each slot filled by whichever provider suits it.
2. Define each **role** with a runtime and a brief (its system prompt).
3. **Launch a mission** from a saved crew or start it empty, add configured roles, and watch its members' live terminals and shared feed.
4. Let crew members **coordinate** through two channels: **signals** (typed, router-visible) and **messages** (prose, between crew members or addressed to the human).
5. Get pulled in through the **lead**, or directly in a mission without one, when a decision needs a human.

The same app also supports **direct chats** — one-on-one sessions with a single agent outside any mission — for quick "I just want to ask `@architect` something" loops without standing up a whole crew.

## 3. Vocabulary

These are the words the product surfaces to the user; they also map 1:1 to the architecture model (`arch.md` §3).

- **Role** — a configured CLI agent (binary + args + role brief). Top-level config; reusable across crews.
- **Crew** — a named set of *slots*, each filled by a role, with exactly one lead.
- **Slot** — a position inside a crew, filled by a role. The slot carries the per-crew handle (`@impl`, `@reviewer`, …) and the lead flag. Two slots in different crews can both be filled by the same role.
- **Mission** — a task with its own roster, working directory, goal and coordination bus. A crew seeds its roster or it starts empty; adding, removing or stopping one member does not stop the others.
- **Session** — the live PTY process for one slot inside a mission, one role-backed direct chat, or one runtime-only direct chat.
- **Signal** — a typed notification crew members emit for the router to handle. Verb grammar (`ask_human`, `mission_goal`, `session_status`).
- **Message** — prose posted to the mission, broadcast or addressed to one or more members with `--to` or leading `@handle` mentions.
- **Inbox** — the broadcasts and directed messages addressed to a member. Runner pushes a notification when delivery is safe; the recipient reads its contents with `runner msg read` and finishes its turn when waiting for another member.
- **Lead** — the crew's coordinator and default human-input gateway. A mission without a lead shares its goal with joining workers, who can ask the person directly.
- **Direct chat** — an off-bus, one-on-one PTY session with a single agent. No mission, no router, no inbox.

## 4. Product surfaces

The user-facing surfaces, described by the value they deliver, not by their implementation. Cross-references point at the arch doc for the "how."

### 4.1 Crew and role library

- Create, edit, delete roles (handle, display name, runtime, command + args, working dir, system prompt, env).
- Compose crews from those roles by adding slots. Exactly one slot per crew is the lead. The same role can sit in many crews simultaneously.
- Optional per-crew **team conventions** addendum (`crew.system_prompt_addendum`) — Layer 2 of the prompt stack, spliced between the platform-injected coordination preamble (Layer 1) and the role's system prompt (Layer 3). Lets a crew share house rules without editing every role. See [arch §6](../arch/arch.md#6-system-prompt-composition).

### 4.2 Missions

- **Start mission** with a crew or **No crew**. A crew starts its slots; an empty mission waits for roles to join. The app opens a mission it starts; a CLI start appears in the sidebar without taking focus.
- A mission states its own goal (a crew carries no default goal) and has its own working directory — the mission cwd is the authoritative working dir for every spawned slot, overriding the role's `working_dir`.
- Concurrent missions on the same crew are allowed; each has its own roster, sessions, event log and router state. Later crew edits do not change an existing mission. Workers can be added and removed during the run.
- **Stop Mission** kills the live PTYs but keeps the mission row running and resumable. Resume respawns stopped/crashed slots from their persisted session rows.
- **Archive** stops members and retains a read-only feed under Settings → Archived. **Restore** reopens the mission with members stopped; **Resume** continues their saved conversations.
- **Sessions outlive the app.** `runnerd` owns the PTYs, mission routers and state. Quitting offers Keep running or Stop sessions, with an Ask/Keep/Stop setting. Kept sessions continue coordinating while the app is closed; relaunch reattaches them. Stop sessions stamps them for launch resume. Updates restart the sessions into the new build and restore their conversations.

### 4.3 Live session terminals (with human takeover)

- One PTY per slot, rendered from an `alacritty_terminal` grid by GPUI for full TUI fidelity with first-class agent runtimes such as claude-code and codex.
- The terminal pane is a real terminal, not a log viewer. The human can type into any session's stdin at any time — answer a prompt, correct a bad plan, kill a tool call, or just chat mid-flight. Human and router share the same writer path, so they are symmetric.
- Native runtime hooks provide Working, Idle, surfaced waits and turn outcomes where the agent exposes them. Other sessions use an identified terminal-activity baseline; silence alone does not prove a healthy hook-driven turn finished. Agents do not report their own status with a coordination verb.

### 4.4 Coordination — signals and messages

- Crew members emit signals via `runner signal <type> [--payload <json>]` and post prose via `runner msg post [--to <handle>] "<text>"`.
- Both flow through one append-only NDJSON file per mission. The file is tailable with `tail -f` for debugging.
- Signals drive fixed handlers, such as surfacing a question from `ask_human`. The goal is delivered in the opening prompt; its `mission_goal` event records it without injecting it again. Messages create durable inbox backlogs; Runner sends pointer notifications and recipients read the actual messages with `runner msg read`. Drafts and surfaced interactions hold delivery.
- Messages stay flat. The product does not need separate thread or fact primitives; durable conclusions should land in the repo, docs, commits, or ordinary mission prose.

### 4.5 Human-in-the-loop

- Questions appear as cards in the mission feed with their prompt, choices and optional `on_behalf_of` attribution. Pending questions remain visible until answered.
- Choosing an answer records the response and delivers a durable message to the asker through the same inbox path. A lead can then relay it to a worker.
- Sidebar and workspace indicators show attention in the app. Background native notifications are shipped: chat events default on, the four mission switches default off, and clicking a notification opens its target.

### 4.6 Mission workspace UI

- **Sessions rail** — the mission's current members, session status and add/remove controls. Open a member to see its terminal.
- **Focused terminal** — the selected slot's terminal pane.
- **Event feed** — chronological view of messages plus user-visible signals for the mission. Router-internal signals (`inbox_read`, agent-source `session_status`) are filtered.
- **HITL cards** — questions and answers retained in the feed, with pending prompts available for a response.
- **Mission header and info** — title, goal, cwd, start time and lifecycle controls. A crew is an optional starting template, rather than the mission's live membership.

### 4.7 Direct chats

- Start a chat with a role or directly with a runtime (`claude-code`, `codex`) plus a working directory. No mission, no router, no bus — it's just a PTY between the human and the agent CLI. Useful for quick one-shots.
- Direct chat rows and daemon-owned live PTYs survive an app restart. Relaunch reattaches live terminals; stopped rows resume their saved conversations where supported. Each session records its launch definition so later role edits do not change Resume or fork.

### 4.8 App life

- **Updates** — Settings → Updates and the sidebar indicator expose available downloads and installation. macOS uses Sparkle; Windows verifies a signed installer before Install and restart. Production and nightly have separate feeds and share the installed app and user data.
- **Logging + crash reporting** — `tracing` writes a rotating log to the OS log dir for the bundle; a panic hook captures backtraces; Help → Reveal Logs in Finder.
- **Theming** — Settings exposes Auto / Light / Dark appearance, light variants (Runner Light, Catppuccin Latte), dark variants (Runner, Catppuccin Mocha), and separate terminal themes.

### 4.9 External control

- **The `runner` CLI** — one bundled binary is the external surface for agents, scripts, and people at a terminal: projects, roles, crews and slots, mission and chat lifecycle, the feed (with `--follow`), signals and messages. Inside a mission the caller's mission and handle come from the environment; outside, `--mission` is a flag and the caller acts for the person unless `--as <handle>` names a seat they hold. `--json` is what agents read; the default output is shaped for people.
- **Discovery through a skill, not configuration.** No agent is set up to reach Runner. Each detected runtime finds the CLI through a managed `runner` skill in its native skills root; the development build has a separate `runner-dev` skill. Installation respects foreign folders and the General setting. The [architecture](../arch/arch.md#96-agent-discovery-skill) owns the current roots. A provider Runner has never heard of only needs the binary on PATH.
- **One way in.** The MCP integration — the `runner-mcp` bridge and the entries Runner wrote into each agent's config — was removed in 0.11.0, the release that shipped the CLI; mission coordination has one CLI surface. The internal optional Codex status helper is separate from this retired workspace API. The CLI uses the same `runnerd.sock` client protocol as the app, with typed requests and pushed mission events. The old MCP endpoint remains only as an accept-and-close guard against a 0.12 app replacing the daemon. Settings → MCP remains as the catalog of the user's own servers.
- `runnerd` owns state; the app and CLI are local clients. Planned [web remote control](https://github.com/yicheng47/runner/issues/836) adds host enrollment, an outbound relay connection and a versioned encrypted client protocol. A person first reaches existing sessions from a desktop or phone browser; the [iOS companion](https://github.com/yicheng47/runner/issues/912) reuses that foundation, followed by [desktop-to-desktop SSH](https://github.com/yicheng47/runner/issues/808). The relay cannot read session traffic, and rejecting a remote client must leave the host's agents running.

## 5. The demo loop

The concrete user flow the product must support end-to-end:

1. User creates a crew called *Feature Ship*.
2. User adds a coder lead and a reviewer, using whichever supported providers suit the task, with role prompts and shared team conventions.
3. User starts the mission with a cwd and goal. Both PTYs spawn; the composed opening prompt gives the lead its task.
4. Coder implements and runs the selected checks, then posts `runner msg post --to reviewer "Ready for review; inspect the working-tree diff and recorded checks."`
5. Reviewer receives an inbox notification, runs `runner msg read`, reviews and sends specific findings directly to the coder. While waiting with no independent work, each participant finishes its turn.
6. They iterate through messages until review is clean. Decisions and durable conclusions go into the repo and the handoff.
7. A worker needing a human decision messages the lead. The lead answers or uses `runner ask --human "Decision needed" --choices "Proceed,Pause"`. The person answers the feed card; the reply reaches the asker's inbox. In a no-lead mission, workers ask the person directly.
8. The crew finishes the authorized work and reports its result. Commits, pushes, PRs and merges follow the task's explicit permissions; Runner does not grant them.
9. The person stops the mission to pause it or archives it when done. Its feed remains available; restoring it allows a later resume.

If this loop doesn't work end-to-end without the user touching a terminal outside the app, the product hasn't shipped.

## 6. Non-goals (long view)

These are intentionally out of scope — they belong to a different product or a later phase.

- **An agent development environment.** Editing, staging, committing and pushing belong to the agent's own tools, git or an IDE. Runner is the environment agents work *in*, not a workbench for building them. It does not create git worktrees: a crew that needs its own checkout makes one from its brief, and worktree isolation ([#403](https://github.com/yicheng47/runner/issues/403)) was closed as not planned on 2026-09-22. Reading the work is the exception: a read-only file and diff viewer whose review comments go back to the agent ([#634](https://github.com/yicheng47/runner/issues/634)), closed on 2026-09-22 and reopened on 2026-10-08, because reading an agent's changes was still the most common reason to leave Runner.
- Cross-mission memory / persistent crew brain.
- A distributed coordination bus. The shipped daemon owns local sessions. Browser access, the iOS companion and desktop-to-desktop SSH are separate planned remote tracks; the mission bus remains host-local.
- Sandboxing beyond the child process's own permissions.
- Observability dashboards. The unscheduled token ledger ([#630](https://github.com/yicheng47/runner/issues/630)) proposes per-session tokens and subscription quota; it has not shipped.
- Marketplace of roles.
- Multi-human collaboration on the same mission.
- Thread/fact primitives for mission coordination.
- A general-purpose secrets vault for role environments. Remote-access credentials have their own OS secret-store contract in the remote design.
- LLM-based signal routing (the router is a flat dispatcher by design — the lead owns coordination judgment).
- Additional platforms before their own validation. macOS and Windows ship today; Linux ARM64/x86_64 source/package support is in development, with native and WSL validation tracked separately in [#823](https://github.com/yicheng47/runner/issues/823).

## 7. Open product questions

The issues own unresolved scope; these are the product decisions still needed:

1. How should a starting chat stay connected to the missions it launches? [#748](https://github.com/yicheng47/runner/issues/748) needs a new design; Runner typing updates into a normal chat was rejected on 2026-10-17. External watches and a participant's own inbox are separate existing workflows.
2. How should optional mission leads and the coordinator's `spawn`, `ps`, `wait`, `stop` and `done` verbs work together? Empty missions already work; [#873](https://github.com/yicheng47/runner/issues/873) and [#916](https://github.com/yicheng47/runner/issues/916) own the follow-through.
3. What is the read-only file and diff review surface? [#634](https://github.com/yicheng47/runner/issues/634) requires design before implementation; editing and git writes stay with the agent's tools.

## 8. Risks

- **PTY and process-lifecycle edge cases.** Orphan reaping, resume, and geometry are the recurring trouble spots. Windows runs a second lifecycle implementation on ConPTY and Job Objects, so every PTY change is checked on both platforms.
- **TUI rendering edge cases in the terminal renderer.** Claude / codex use rich TUIs (alt-screen, OSC 8 hyperlinks, OSC 52 clipboard); every new TUI quirk is a tuning loop.
- **Agents that don't know the `runner signal` / `runner msg` conventions.** We ship sensible default briefs per runtime so even an untuned agent participates correctly.
