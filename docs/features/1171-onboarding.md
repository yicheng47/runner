# 1171 — First-run setup

Tracking issue: [#1171](https://github.com/yicheng47/runner/issues/1171). Builds on [#1165](https://github.com/yicheng47/runner/issues/1165)'s background tasks ([archived spec](archive/1165-agent-cli-version-gate.md)).

Design: `runner/desktop/specs/1171-onboarding.pen` in `yeelinks_design` (`49934b9`). Option A, setup in the launcher's place, was chosen on 2026-11-15 over a full-window wizard and a card above the launcher. Frames:

- `WBlrk` agents found, `e52th` expanded agent list with bounded scroll, `e839DM` no agents found.
- `t0ki9c` installing an agent from setup, `SoC3f` the install row's states.
- `GVeqR` Settings → Agents with Open setup and Install.
- `I0YpU` the starter crew card's states.

The frames win on layout and copy; this spec wins on behavior.

## Motivation

A fresh install opens on the launcher with no guidance. The only first-run help is the seeded **Pair coding** crew, whose `@coder` and `@reviewer` both run Codex (`crates/runner-daemon/src/db/seed.rs`). The daemon seeds it when it creates the database, before the login-shell probe has found any agent, so a new user without Codex has a starter crew that cannot start. They have to find **Settings → Agents** on their own to learn which agents Runner detected, and leave the app to install them. The getting-started guide covers these steps in prose; the app does not.

## Scope

### 1. The setup page

- On first launch the setup page takes the launcher's place in the window, beside the sidebar (`WBlrk`). It holds, in order: a header with one sentence each for chat, crew and mission; **Your agents** with Recheck; **Starter crew**; and a footer with **Skip setup** and **Start your first chat**.
- **Start your first chat** creates the starter crew unless the person opted out (section 4), then opens the normal launcher with the default agent selected and a safe, read-only sample prompt filled in. It does not start a mission.
- **Skip setup** creates nothing and opens the normal launcher. Its hint says setup reopens from Settings → Agents.
- Setup is per window surface, not a modal: the sidebar, other windows and existing tabs keep working while it shows.

### 2. Agents

The list uses the runtime status behind Settings → Agents, in catalog order, with the four most common agents shown and the rest behind "N more agents Runner supports". Expanding the list caps the agent rows in a bounded vertical scroll viewport (`216 px`, ~4.8 rows visible with row 5 peeking) with an inner scrollbar and a pinned `Show fewer agents` footer row (showing `Scroll for N more · M total` when overflowing) below a hairline divider; collapsing resets the inner list scroll offset.

- **Ready:** installed, with its version and "Signed in" where Runner can tell (see Open questions).
- **Sign in needed:** installed but signed out, with the sign-in command as the detail and **Open terminal**, which opens a terminal tab in the home folder.
- **Not installed:** the install command as the detail, Copy, and **Install** when section 3 applies.
- **Recheck** reruns detection, as Refresh does in Settings → Agents.

### 3. Install from setup

- An agent whose catalog entry has `npm_package` (Claude Code, Codex, GitHub Copilot CLI, pi) shows **Install** when the login shell's `PATH` has `npm`. Install starts an install task in runnerd that runs `npm install -g <package>` in its own PTY, in the home folder, with the login-shell environment.
- An install task is #1165's update task with a kind: `RuntimeUpdateTask` gains `kind` (`install` or `update`), and an install has no starting version. It shares the task list, the pill, the `runtime/task-changed` event, joining a running task, may need input, stop, dismiss, failure with Retry and the 5-second clear after success. A runtime has at most one active task of either kind.
- The row shows the task inline (`SoC3f`):
  - **Installing:** a spinner and "Installing…" in place of the status dot, the installer's latest output line as the detail, and × to stop it.
  - **May need input:** an amber dot, "May need input" and **Answer**, which opens the update dialog on the task's terminal.
  - **Failed:** a red dot, "Install failed", the exit code and a plain reason as the detail, and **Retry**.
  - The full output is viewed from the pill's task list, not the row.
- On success runnerd rechecks that runtime at once. The row moves on to sign-in or ready, and the starter crew card recomputes.
- Stopping an install interrupts npm and leaves the agent not installed. #1165's "broken after a stop" check and Reinstall do not apply to installs.
- **No npm:** the row's detail says installing needs Node.js and npm, and **Node.js** opens nodejs.org. Recheck after installing Node picks npm up.
- **No package** (Cursor Agent, OpenCode, Grok Build, TRAE CLI, Antigravity CLI): Copy and the install guide, as today. Runner never runs a vendor's `curl … | bash` script.
- Settings → Agents' Not installed rows get the same Install action and spinner (`GVeqR`).

### 4. The starter crew

The coder and reviewer loop, built from the detected agents and created by default (`I0YpU`).

- **Eligible runtimes:** installed, enabled and not below its minimum version. A signed-in runtime ranks above one that needs sign-in, where Runner can tell.
- **Ranking:** the user's default agent first, then catalog order (Claude Code, Codex, then the rest).
- **Coder:** the top-ranked eligible runtime.
- **Reviewer:** the top-ranked eligible runtime that differs from the coder's. Runner pairs different runtimes whenever two or more are eligible. With exactly one eligible runtime both slots use it, and the card's note says that installing a second agent lets them review each other.
- **None eligible:** the card says it needs an agent ("Install Claude Code or Codex. Runner builds a coder and reviewer from whatever you have."), with no status, and nothing is created.
- **Will be created** (default): the card names the crew, `@coder` and `@reviewer` with each runtime, and says **Will be created** with a quiet **Don't create** under it.
- **Won't be created:** after Don't create, the same card stays in place, muted, and says **Won't be created**, with **Create it** where Don't create was.
- The card recomputes when detection, sign-in or a task changes, until setup finishes.
- Created means one crew named Pair coding with two roles, using today's seed prompts and crew addendum (`SEED_CODER_PROMPT`, `SEED_REVIEWER_PROMPT`, `SEED_CREW_ADDENDUM`) with each slot's chosen runtime and that runtime's default model, effort and permission arguments. A name collision with an existing crew or role takes the next free suffix.

### 5. No seeded crew for new databases

- `seed_defaults` stops creating Pair coding. It still writes `default_crew_seeded`, so the seed step stays idempotent.
- Existing databases keep the crew they were seeded with.
- With no crew, the launcher's Mission entry says "No crews yet" and offers **Create a crew** (Crews) and **Open setup**; `runner mission start` without `--crew` and with no crews prints how to create one.

### 6. When setup shows

- A new `_app_state` key, `onboarding_completed`, holds the time setup finished. The seed marker cannot decide this, because runnerd writes it when it creates the database, before any app opens.
- A window's launcher shows setup while the key is missing.
- **Start your first chat**, **Skip setup**, or any chat or mission started another way (another window, the CLI, a remote window) sets the key. Every window then shows the normal launcher.
- A migration sets the key on existing databases that already have any session, mission, or a role or crew other than the seeded ones, so upgrades never see setup.
- Which agents are detected does not decide whether setup shows, only what it shows.
- **Open setup** in Settings → Agents (`GVeqR`) shows the page in the current window without clearing the key. Finishing or skipping it again behaves as above; the starter crew card is offered again only when no crew exists.

### Storage and protocol

- `_app_state` gains `onboarding_completed`. One data migration sets it for existing databases (section 6). No schema change.
- The protocol gains: read and set onboarding completion; start a runtime's install (the update start request with a kind, or a sibling request); create the starter crew from two runtimes; and the starter crew proposal (coder, reviewer, eligible count) computed in runnerd from runtime status, so the app, the CLI and a remote window agree.
- `RuntimeUpdateTask` gains `kind`; `from_version` is empty for an install.

## Out of scope

- Running provider sign-in flows inside Runner. Open terminal only starts the agent's own sign-in.
- Vendor install scripts and package managers other than npm.
- Interactive product tours.
- Starting a mission from setup.

## Guide

- `docs/guide/getting-started/install.md`: setup on first launch, installing agents from Runner.
- `docs/guide/getting-started/first-chat.md`: Start your first chat from setup.
- `docs/guide/getting-started/first-mission.md`, `guides/build-a-crew.md`, `guides/drive-runner-from-agents.md`, `concepts/projects.md`: the starter crew from setup replaces the seeded Pair coding crew.
- `docs/guide/reference/supported-agents.md`: Install for npm-packaged agents.
- The `zh-CN` pages that mirror them.

## Verification

### Automated

- Starter crew selection: default first, catalog order, different runtimes when two or more are eligible, one runtime, none eligible, below-minimum and disabled runtimes excluded, signed-in preferred.
- Starter crew creation: two roles and a crew from the seed prompts on the chosen runtimes; name collision suffixes.
- `seed_defaults` on a new database creates no crew and writes the marker.
- The onboarding migration: set for a database with sessions, missions, or non-seed roles or crews; left unset for a fresh database and for one with only the seed.
- The task state machine with the install kind: start, join, success with a recheck, failure, stop leaving not installed, no Reinstall.
- Install availability: npm package and `npm` on the login-shell `PATH`; no npm; no package.

### Live checks

The change alters what a person sees and operates, so it needs live checks, run by the `qa` slot of a claude trio. Each check uses a fresh development database, so setup shows, and a scratch npm prefix and package where it installs, so QA never changes the machine's real agent installs.

1. **Setup on a fresh database.** The launcher's place shows setup with the detected agents and a starter crew card that says Will be created. Reason: new first-run surface.
2. **Start your first chat.** Creates the crew with the coder and reviewer on different runtimes, opens the launcher with the sample prompt, and setup does not return in a new tab or window. Reason: completion, crew creation and the flag.
3. **Don't create, Create it, Skip setup.** Don't create mutes the card; Create it restores it; finishing after Don't create, or Skip setup, creates no crew. Reason: the opt-out states.
4. **Install from setup.** Install on an npm-packaged agent shows the spinner and output line in the row and the pill; on success the row moves on and the crew card recomputes; a failing install shows Retry; × leaves it not installed. Reason: install as a background task.
5. **No npm.** With `npm` absent from the login shell, the row shows the Node.js link instead of Install. Reason: install availability.
6. **Upgrade.** A development database copied from an existing install opens on the normal launcher, with its seeded crew intact. Reason: the migration.
7. **Open setup.** Settings → Agents' Open setup shows the page; Skip setup returns to the launcher. Reason: reopening.

## Open questions

- **Sign-in detection.** Which runtimes can runnerd tell are signed in without starting the agent? Claude Code's accounts are known; for the others setup may only say "Installed" and leave sign-in to the agent's first run.
- **Windows and Linux.** Which agents does setup offer where the capability table marks them unverified, and does `npm install -g` need elevation there?
- **Sample prompt.** The read-only first-chat prompt's wording, and whether it names the folder the launcher starts in.
