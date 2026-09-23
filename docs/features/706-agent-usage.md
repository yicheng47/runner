# 706 — Plan usage for Claude Code and Codex

> Tracking issue: [#706](https://github.com/yicheng47/runner/issues/706)
> Priority: P2, milestone 0.11 (moved from 0.12 on 2026-09-23: Jason wants to use it), so it ships in a 0.11.x patch. Platforms: macOS and Windows.
> Status: spec, waiting for Jason's comments. Design settled 2026-09-23.
> Design: `design/specs/706-agent-usage.pen`: `706 — Usage icon in the Settings row, popover open` (`z08zW`) and `706 — Usage icon states` (`aFl82`).
> Related: [533](./533-agent-cli-updates.md) (agent CLI updates: the popover is where its "Update available" shows on the main window), [562](./562-mission-spawn.md) (a lead choosing a runtime per spawn).
> Prior art: Orca's status-bar usage meters and popover (`~/repos/ai/orca/src/main/rate-limits/`, `src/renderer/src/components/status-bar/`), read 2026-09-23.

## Motivation

Runner runs agents on the person's own subscriptions: Claude Pro or Max through Claude Code, ChatGPT through Codex. Each has limits that reset. Claude has a 5-hour window, a weekly window and per-model weekly limits (Fable); Codex has a 5-hour and a weekly window. Nothing in Runner shows how close either is. The person finds out when a session stops mid-task, then opens each CLI's `/usage` or `/status` to learn when it resets.

Once 562 lets a lead pick a runtime for each piece of work, which subscription still has room this week is exactly what that choice needs. The person needs it first.

## Placement

Decided with Jason on 2026-09-23 after five candidates in the design file's history: meters in the sidebar footer, chips in the Settings row, a status bar under the panes, a Usage sidebar section, and Settings moved to the header. The sidebar is already crowded, and every visible meter took room from the tab list or the panes.

**A usage icon in the Settings row that opens a popover.**

- **The icon.** A lucide `gauge` icon at the right end of the sidebar footer's Settings row (`crates/runner-app/src/surfaces/app_shell.rs`, `settings_button`). It always holds the rightmost slot. The Runner update icon, shown only while an update is available, appears to its left, so the gauge never moves.
- **Its colour is the signal** (frame `aFl82`):
  - grey (`text-mid`) while every agent is under 80% of every window;
  - amber (`warn`) when any window of any agent is at 80% or more;
  - red (`danger`) when any window is at 100%;
  - highlighted (the `sidebar-sel` fill) while the popover is open.
- **Hover** shows a tooltip with the headline numbers: "Claude Code 4% · Codex 27%". This is the glyph-only case where a tooltip is right.
- **Click** opens the popover upward from the row, above the footer. Clicking outside or Esc closes it.
- **Hidden** when neither agent is installed and enabled in Settings → Agents, and with the sidebar collapsed. A collapsed sidebar is a deliberate choice to hide chrome, and the popover is one ⌘S away.

## The popover

One design for every state (frame `z08zW`), 340px wide.

- **Header.** "Usage", "updated 2m ago", and a refresh button.
- **One section per agent,** in the order Claude Code, Codex, each with its mark and name. Under it, one row per window: its name (5 hours, Week, Fable · week), a bar, the percent used, and "resets 3h 10m". The bar fills in `text-mid`, `warn` at 80% and `danger` at 100%, matching the icon.
- **A version line per agent,** from 533: the installed version, then either "up to date" or "0.153.4 → 0.155.0 · Update available" with an **Update** button that opens 533's update pane. Until 533 lands, the line shows the installed version only.
- **Unavailable.** An agent whose usage cannot be read keeps its section with one line saying why, instead of window rows: "Sign in to Claude Code to see usage.", "Runner was not allowed to read Claude Code's sign-in from the Keychain.", "Couldn't reach Anthropic." or "Codex didn't answer.". The icon ignores unavailable agents when it picks its colour.
- **Footer.** "Agent settings…", which opens Settings → Agents.

## Data

Both fetches run off the UI thread with a 10-second timeout, through the login-shell proxy environment Settings → Agents already captures, and against the effective executable Settings → Agents resolves.

### Codex

- Spawn the effective `codex` with `-c approval_policy=never -c features.plugins=false -s read-only -a never app-server` (Orca's arguments). Speak JSON-RPC over stdio: `initialize`, the `initialized` notification, then `account/rateLimits/read`. Kill the process after the answer or the timeout.
- The answer's `rateLimits` has `primary` and `secondary` windows, each with `usedPercent`, `windowDurationMins` and `resetsAt`. Name a window by its duration (300 minutes is "5 hours", 10080 is "Week"), not by its position.
- `app-server` is Codex's own interface for editor integrations, and Codex authenticates itself. Runner never reads Codex's credentials.

### Claude Code

- `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer <token>` and `anthropic-beta: oauth-2025-04-20`. The answer carries `five_hour` and `seven_day` windows and per-model weekly limits (the `limits` list, `kind: weekly_scoped`, Fable today), each with a percent used and `resets_at`.
- **The token is the one Claude Code stores.** On macOS it is the Keychain generic password `Claude Code-credentials`. On Windows and Linux it is `~/.claude/.credentials.json`. Both hold JSON with `claudeAiOauth.accessToken`.
- **Runner only reads the token.** It never refreshes, writes or caches it. A refresh can rotate the refresh token and sign Claude Code out. A rejected token shows "Sign in to Claude Code to see usage." and recovers on the next fetch after Claude Code has refreshed its own login.
- **The first Keychain read on macOS shows a system prompt** asking to let Runner read Claude Code's item. "Always Allow" makes it silent. "Deny" shows the Keychain line above, and Runner does not ask again in that app run.
- **The endpoint is undocumented.** It is what Claude Code's `/usage` calls, and it can change without notice. The parser takes known fields and ignores the rest, and a shape it cannot read shows the agent as unavailable. There is no fallback that reads `/usage` from a hidden terminal.

### When it refreshes

- Once at launch, after agent discovery completes.
- Every 15 minutes while Runner runs.
- From the popover's refresh button.
- When the popover opens and the last fetch is older than 5 minutes.

The last result is kept in memory for the app run and nowhere else. A failed refresh keeps the last good numbers with their "updated … ago" time.

### Where it lives

A `usage` module in `runner-backend`: the two fetchers, the cache, the schedule, and a `usage/updated` event the app listens to. It stays in the backend rather than the app so that a later `runner usage` command can reach it; a lead choosing a runtime under 562 is the obvious reader. That command is out of scope here. The backend gains the same `zed-reqwest` client the app's updater uses.

## Out of scope

- Other agents: TRAE, pi, and GitHub Copilot's premium-request allowance. They are added when their CLIs expose usage.
- More than one account per agent, and switching accounts (Orca's "Manage Accounts…").
- Usage history, charts, and per-session token accounting.
- Notifications at a threshold. The icon's colour is the signal.
- A `runner usage` command or socket tool.
- Refreshing Claude Code's token, or reading `/usage` or `/status` from a hidden terminal.

## Implementation phases

1. **Design.** Done: the two frames above.
2. **Backend.** The `usage` module:
   - the Codex `app-server` probe;
   - the Claude credential read (Keychain on macOS, the file elsewhere) and the usage request;
   - window parsing by duration;
   - the cache, the 15-minute schedule and `usage/updated`.

   Tests against recorded answers (both agents, missing fields, unknown windows, a rejected token, a timeout), and the rule that nothing is written to Claude Code's credential store.
3. **App.** The icon in the Settings row with its four states and tooltip, the popover with its unavailable lines, the refresh button, and "Agent settings…". Runner-app tests for the colour thresholds and the reset formatting.
4. **Smoke** (Jason, macOS and Windows):
   - the numbers match `/usage` in Claude Code and `/status` in Codex;
   - the Keychain prompt appears once, and Deny shows the Keychain line;
   - signing out of Claude Code shows the sign-in line;
   - a Codex account near a limit turns the icon amber;
   - the Runner update icon appearing leaves the gauge where it was.

## Verification

- [ ] The popover shows every window each agent reports, with the right percent and reset time, and names windows by duration.
- [ ] The icon is grey under 80%, amber at 80% or more, and red at 100%, across every window of every available agent.
- [ ] The gauge keeps the rightmost slot whether or not the Runner update icon is showing.
- [ ] Runner never writes, refreshes or stores Claude Code's token or Codex's credentials, and a rejected token shows the sign-in line.
- [ ] Denying the Keychain prompt shows the Keychain line and does not prompt again in that run.
- [ ] A failed refresh keeps the last numbers with their age, and an agent that has never answered shows its unavailable line.
- [ ] Refreshes happen at launch, every 15 minutes, on the button, and on opening a popover older than 5 minutes, and never more often.
- [ ] With 533 landed, the Update button opens the update pane for that agent.
