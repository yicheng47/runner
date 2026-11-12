# 1249 — Usage settings, and Skills and MCP inside each agent

Tracking issue: [#1249](https://github.com/yicheng47/runner/issues/1249).

Design: `runner/desktop/specs/1249-agent-integrations.pen` in `yeelinks_design`, signed off on 2026-11-11. Frames: `R5FECK` Claude Code · General, `a7mUO` Claude Code · Skills, `hEc4H` Claude Code · MCP, `UDYTT` Skills with the account menu open, `DdgTC` Skills for an account that uses the main account's settings, `tIls6` Settings · Usage, `fK9QE` Usage with Codex hidden, `KeftC` Usage with the sidebar badges off, `z2dw5B` Usage with the 5-hour limit off, and `L06ZsA` Settings · Notifications with the Usage group. The bottom row (`LOEzL`, `VvFIf`, `s8mYyG`, `S4ZUkV`) is today's UI, for comparison only. The frames win on layout and copy; this spec wins on behavior.

## Motivation

Usage has no settings. The sidebar footer always shows weekly Codex and Claude Code usage while those agents are installed and enabled, refreshed every 15 minutes and every 5 while the usage popover is open (`runner-daemon/src/usage.rs`). The popover's Settings actions only open Settings. People cannot choose what the footer shows, get told before they hit a limit, or control how often Runner asks.

Skills and MCP are separate Settings pages, each with its own runtime dropdown, although both are per-agent configuration. They belong on the agent's own page. They also read only the main Claude Code account's folder (`claude_accounts::sharing::config_env`), so an account with its own settings has no place where its skills and MCP servers can be seen or changed.

## Scope

### Settings navigation

Settings › Integrations lists **Agents** and **Usage**. The Skills and MCP panes leave the sidebar. `SettingsPane::from_route(Some("skills"))` and `Some("mcp")`, and Settings search hits for them, open the matching tab of an agent page: the agent the old pane last showed, else the default agent, else the first agent that supports it.

### Agent page tabs

An agent's page gets tabs under the title and version line: **General · Skills · MCP**, with the item count after Skills and MCP. General holds today's sections (Executable, Defaults, Launch and, for Claude Code, Accounts). Skills and MCP are today's panes scoped to that agent: search, Refresh, config path and counts, caption, one toggle per row, click for detail, edit in the file's own syntax. Their runtime dropdown goes away. An agent that supports neither skills nor MCP shows no tabs; one that supports only one shows General and that tab. The open tab is per page and resets to General when another agent's page opens.

### Claude Code account on Skills and MCP

With more than one Claude Code account, the Skills and MCP tabs show an account dropdown at the start of the search row (`UDYTT`). It opens on the main account, which is what both panes read today. Its menu lists every account with its name, `main` and `active` tags, a second line with "Uses {main}'s settings" or "Own settings" and its folder (or "not signed in"), and a check on the shown account. The config path, counts and list follow the chosen account, and toggles and edits write to that account's files.

For an account that uses the main account's settings (`DdgTC`), a note above the path line says "{account} uses {main}'s settings, so it gets {main}'s skills and these toggles change them for both accounts." with a **Show {main}** button that switches the dropdown. On MCP the note says the same of servers; the list is the main account's servers, which load at launch, plus the account's own.

With one account there is no dropdown. The choice is not persisted: each visit opens on the main account.

### Usage page

Settings › Usage, icon `gauge`, after Agents. Its header has the title, the last fetch time ("Updated 2m ago") and **Refresh**. Only Codex and Claude Code appear; other agents are not listed.

- **Sidebar.** A compact preview row inside the Sidebar card shows weekly usage, the host button and the Settings gear beside its label, following the controls below as they change and sharing the real footer's elements. [#1271's approved option A](1271-usage-preview.md) supersedes this feature's original `MYiIi` faded recent-row preview: the sample is 240px wide including 12px padding, has no recent chat content, and stacks below the label when necessary. Its host and Settings buttons are inert. A **Show usage in the sidebar** switch (on by default), then one switch per usage-capable agent that is installed and enabled (on by default; Claude Code's subtitle "Shows the active account."). Turning the first off dims and disables the agent rows and hides only the badges, keeping host and Settings anchored. Badges always show the weekly limit, as today.
- **Limits.** **Check every** sets the scheduled refresh interval: 5 minutes, 15 minutes (default), 30 minutes or 1 hour. The popover's 5-minute refresh while open stays. A **5-hour limit** switch, on by default, shows or hides the 5-hour window in the usage popover, on this page and in alerts; the weekly limit always shows. Below it, a card per agent with each window's bar, percentage and reset time, the same data as the popover, including Claude Code's per-model weekly windows. Claude Code's card has an account dropdown that shows one account at a time, as the popover's picker does. The caption under the cards states the interval.
- The usage popover's Settings actions open this page.

### Notifications

Settings › Notifications gets a **Usage** group after Missions with one row, **Near the limit**: a threshold dropdown (50%, 60%, 70%, 80%, 90%; 80% by default) and a switch, on by default. It notifies once per window per reset period when a Codex or Claude Code usage window reaches the threshold, naming the agent, the window and the percentage; clicking it opens Settings › Usage. For Claude Code it covers the active account only. With the 5-hour limit off, 5-hour windows never alert. It follows the System permission row like the other notifications. The caption below the group is "For Claude Code this covers the active account. Limits and the 5-hour window are set in Settings › Usage."

### Fetching

Hiding the badges, or an agent's badge, does not stop that agent's checks while the near-the-limit alert is on; an agent is checked when its badge is on or the alert is on, and the Usage page and popover still load on open. Checks stop for every agent only when the badges and the alert are both off.

### Storage and protocol

Badge switches, the 5-hour switch and the alert threshold and switch are app settings (`app_settings.rs`, the notification ones under `NotificationSettings`), with serde defaults so existing files load unchanged. The check interval belongs to the daemon's usage scheduler: one new client request sets it and the daemon keeps it in `_app_state`, no migration. Skills and MCP catalogs for a chosen Claude Code account need the account's config env in the existing catalog, toggle and edit requests (an optional account id; none means the main account, as today).

### Guide

Update `docs/guide/guides/mcp-and-skills.md` (new location, account dropdown), `docs/guide/guides/chats-and-windows.md` (the footer section and the notifications paragraph), `docs/guide/guides/claude-accounts.md` (usage and skills mentions), and their `zh-CN` copies.

## Non-goals

- A window choice for the badges, per-window thresholds, or a limit-reset alert.
- Alerts for Claude Code accounts other than the active one.
- Usage for agents other than Codex and Claude Code.
- Remote windows' footers (#1229) beyond keeping them working.

## Implementation phases

1. Agent page tabs and the Skills and MCP move, with route and search redirects.
2. The Claude Code account dropdown on Skills and MCP, and the catalog requests that take an account.
3. The Usage page: preview, badge switches, interval, 5-hour switch, Limits cards; popover Settings actions open it.
4. The Notifications Usage group and the near-the-limit alert; fetching rules.
5. Guide pages.

## Verification

Automated: the settings nav test lists Agents and Usage under Integrations; old `skills` and `mcp` routes resolve to agent tabs; tabs per runtime follow its skill and MCP support; catalogs for a chosen account read that account's folder and a sharing account reads the main account's; settings defaults and round-trip for the new fields; the scheduler honours the interval; the alert fires once per window per reset, not for 5-hour windows with the switch off, and only for the active Claude Code account; fetching stops only when the badges and the alert are both off.

Live checks in the development app, because the change moves Settings pages and adds controls people operate:

1. **Navigation:** Integrations shows Agents and Usage only; the Claude Code and Codex pages show General · Skills · MCP with counts, and Cursor (or another agent without skills and MCP support) shows no tabs.
2. **Agent tabs:** on Claude Code's Skills tab, search filters, a toggle persists across a Refresh, and the MCP tab lists the same servers as the old MCP pane did.
3. **Claude account dropdown:** with a second Claude Code account added in the development app (a fresh folder under `/tmp`, no sign-in needed), the dropdown appears, its menu shows both accounts, choosing the second changes the path line, and with one account it is gone.
4. **Usage page:** turning a badge switch off removes that badge from both the preview and the real sidebar footer; the 5-hour switch hides the 5-hour rows on the page and in the popover; the popover's Settings action opens the page.
5. **Notifications:** the Usage group shows Near the limit at 80% and on; the threshold change persists after reopening Settings.
