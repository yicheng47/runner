# Host usage in the remote footer, link status in the host popup

Tracking issue: [#1229](https://github.com/yicheng47/runner/issues/1229). Design: `runner/desktop/specs/1229-remote-link-status.pen` in the design repo, frame `BGIfa`, options C and P1 with the footer's Slow state, signed off by Jason on 2026-11-09. It supersedes option B in frame `kLrRu`, which put the link status in the footer.

## Motivation

A window showing another computer keeps the footer's usage percentages beside the green host button. Those numbers are this computer's usage, while everything else in the window belongs to the other computer, so they read as the host's and say nothing about the computer being controlled.

Jason, 2026-11-09: show the remote host's account usage in that slot instead, and put the signal strength in the host popup. The footer then reads the same way in every window, as the usage of the computer the window shows, and the link's health sits beside the route it describes.

Runner tracks the connection state but has never measured latency: the daemon answers WebSocket pings and ignores pongs, and nothing times a round trip. The companion protocol also carries no usage, although every host already measures its own.

## Behavior

### Footer

While a window shows another computer, the footer's usage slot shows that computer's usage, with the same marks, figures and colours as this computer's: the weekly figure for each agent. For Claude Code with several accounts, the figure is the host's active account, as in the host's own footer. The agents shown are the ones the host's own footer would show, its installed agents that are enabled for usage there, in this computer's usual order. An agent this Runner does not know is left out.

A window showing this computer keeps its own usage, and switching back brings it back. The slot appears under the same condition as today, so a collapsed sidebar shows neither. The gear behaves as it does today. The host button keeps today's colours and also turns amber while the link is [slow](#a-slow-link-turns-amber). There is no separate link status in the footer: the host button's colour is the at-a-glance signal, and the popup has the detail.

| State | Usage slot | Host button |
|---|---|---|
| Connected, figures received | The host's figures | Green |
| Connected, slow link | The host's figures, at full strength | Amber |
| Connected, an agent has no figures yet | A dash for that agent, as on this computer | Green, or amber while slow |
| Opening, before the host's first reply | Empty | Green |
| Reconnecting | The last figures, at 45% opacity | Amber |
| Unreachable | The last figures, at 45% opacity | Red |
| Revoked | The last figures, at 45% opacity | Red |
| Forgotten | The last figures, at 45% opacity | Grey |
| Host cannot send usage | Empty | Follows the state |

Slow and Reconnecting share amber, and the usage slot tells them apart: the figures stay at full strength while the link is slow and dim while it reconnects.

The app keeps the last reply for each computer until the app quits or the computer is forgotten here, so a window that returns to a computer shows its figures at once.

### Usage popover

Clicking the host's usage opens the usage popover with the host's figures. It is read-only:

- The title is **Usage on {computer}**, so the figures cannot be mistaken for this computer's. The update age stays.
- There is no refresh button and no agent settings button, since both act on the host. The gear still opens this computer's Settings.
- With several Claude accounts, the host's active account is named as plain text where this computer shows the account menu. Switching it stays on the host.
- Errors and missing figures use this computer's wording for the host's agents. While the host reports a refresh in progress, the age reads **Updating**, as it does here.

Opening the popover asks the host for its latest figures, and the host refreshes them if they are more than 5 minutes old, as its own popover does.

### Host popup

The connected computer's row leads its status line with a mark: bars while connected, an icon in every other state. The bars are 18 by 9 px, the icons 10 px, and the text follows 5 px after the mark. The text is 11 px.

| State | Mark | Text | Mark and text colour |
|---|---|---|---|
| Opening, or connected before the first sample | `loader-circle` | `Connecting through the relay…`, or `Connected · relay · encrypted` once connected | `text-mid` mark, `text-low` text |
| Connected, under 100 ms | 4 bars | `42 ms · relay · encrypted` | Accent bars, `text-low` text |
| Connected, 100 to 199 ms | 3 bars | `150 ms · relay · encrypted` | Accent bars, `text-low` text |
| Connected, 200 to 499 ms | 2 bars | `320 ms · relay · encrypted` | Amber |
| Connected, 500 ms or more | 1 bar | `800 ms · relay · encrypted` | Amber |
| Reconnecting | `refresh-cw` | `Reconnecting to {computer}…` | Amber |
| Unreachable | `unplug` | `Can’t reach {computer}` | Red |
| Revoked | None | `Access not recognized · pair again` | Red |
| Connected, host cannot answer | None | `Connected · relay · encrypted` | `text-low` |

Bars that are not lit use `line-strong`. Other rows of the popup are unchanged.

### A slow link turns amber

A link is slow from 200 ms, the point where typing visibly trails the keys (Jason, 2026-11-09). While it is slow, the host button, the row's bars and its text are amber. They return to their normal colours only once the reading falls below 180 ms, so a link that hovers around 200 ms does not flicker between the two. The bar count always follows the boundaries in the table, so a reading of 190 ms on its way down shows 3 bars, still amber.

Slow describes the link, not the connection: the route stays Connected and the host's figures stay at full strength. The host button's colour for Connected therefore depends on the latency, which `host_tone` takes into account.

### Unreachable turns red

Unreachable changes from amber to red in the host button and the popup row, because Reconnecting is still trying and Unreachable has failed. This changes `host_tone` and `connection_subtitle`. The reconnecting banner and the connection card in the remote workspace are unchanged.

### What latency measures

Latency is the round trip to the host through the end-to-end encrypted channel, so it includes the relay and reflects what typing feels like. Timing WebSocket pings would measure only this computer to the relay. The measurement uses a purpose-built `ping` and `pong` rather than timing another request's reply (Jason, 2026-11-09), so a host shows a number once it has updated.

Each open route sends one ping every 5 seconds. The displayed value is the median of the last 3 samples, so the number does not flicker. A ping with no reply within 10 seconds counts as a 10,000 ms sample, so the reading falls to one bar and the host button turns amber, rather than freezing until the route itself reports the failure. Samples are discarded when the route leaves Connected, so a reconnect starts again from Connecting.

The ping never decides the connection state, and an unanswered or refused ping never disconnects a route. The route's existing failure handling stays the only source of Reconnecting and Unreachable.

### How the host's usage arrives

The app asks the host for its usage when a window enters the computer or its route becomes Connected, every 60 seconds while a window shows it, and when the usage popover opens. While the popover is open and the host reports a refresh in progress, it asks again every 2 seconds until the refresh finishes. Windows showing the same computer share these requests. A failed request keeps the last figures.

The host answers from the figures it already has, without waiting for a fetch. A request also starts a refresh on the host when one is due, by the host's own rules: the periodic requests use its scheduled interval of 15 minutes, and opening the popover uses the popover's 5 minutes. A host with its app closed keeps its figures current while someone is looking at it, and no host fetches more often than it would with its own app open.

## Non-goals

- Changing anything about the host's usage from a remote window: refreshing on demand, switching its Claude account or opening its agent settings.
- Throughput or real signal strength. Throughput is near zero on an idle terminal, and a route through a relay has no signal to read.
- Link status or the host's usage from a CLI command, or in the iPhone app.

## Implementation Phases

### 1. Protocol and host

- Add a `ping` request to `ClientMessage` and a `pong` reply to `HostMessage` in `crates/runner-core/src/companion/messages.rs`, the `pong` carrying the request's id, with decoder arms and fixtures beside the existing ones. The host answers `ping` with `pong` straight from the channel loop, without waiting on overview or feed work.
- Add a `usage` action beside `catalog` in `crates/runner-core/src/companion/actions.rs`, with fixtures `usage.json` and `result_usage.json`. The request says whether the popover opened it. The result lists, for each agent the host's footer would show, the agent's name as a string, each window's name, used percentage and reset time, when the figures were updated and the unavailable reason if any; whether a refresh is in progress; and the active Claude account's name when the host has more than one Claude account. It carries no account folders, commands or other configuration.
- Answer `usage` in `crates/runner-daemon/src/companion/actions.rs` from `UsageService::snapshot`, filtered to the agents that are enabled for usage and detected, as `UsageService::fetch` filters them, and start a refresh with `RefreshReason::Schedule`, or `RefreshReason::Open` when the popover opened the request.
- Add `ping` and `usage` to `FEATURES` in `crates/runner-daemon/src/companion/mod.rs` and to `fixtures/welcome.json`. The features are negotiated through the welcome's feature list, so the companion protocol `VERSION` stays 1. An older host is never sent either request. The iPhone app ignores both.

### 2. Daemon client

- In `crates/runner-daemon/src/remote/client/route.rs`, when the welcome lists `ping`, send the ping every 5 seconds for as long as the route is open and time its `pong`.
- Compute the median of the last 3 samples, apply the 10-second rule and clear the samples when the route closes.
- Emit a `remote/latency` event carrying the computer and the milliseconds, beside the existing `remote/connection` event.
- Forward the `usage` action like `catalog`.

### 3. App

- `crates/runner-app/src/surfaces/remote_client/model.rs` keeps the latest latency per computer from `remote/latency` and whether the link is slow, applying the 180 ms margin, both cleared when the connection leaves Connected, and the last usage reply per computer, kept until the app quits or the computer is forgotten. It requests usage on the schedule above for computers a window shows, when the host lists `usage`.
- `crates/runner-app/src/surfaces/app_shell/sidebar.rs` renders the host's usage in the usage slot when `remote_client.is_active()`, through `sidebar_usage_element` with the host's figures and agents, at 45% opacity when the route is not Connected, and an empty slot for a host without `usage`. It no longer renders a footer link status.
- `crates/runner-app/src/surfaces/app_shell/usage.rs` gives the usage popover a read-only remote form: the computer in the title, no refresh or agent settings button, and the active account's name in place of the account menu.
- `crates/runner-app/src/surfaces/remote_client/picker.rs` leads the connected row's status line with the mark, maps Unreachable to `theme::danger()` in `host_tone` and `connection_subtitle`, and gives a slow Connected link `theme::warning()` in `host_tone` and the row. Remove `render_link_status` and the footer-only strings.
- Add **Usage on {computer}** and the status line strings to `crates/runner-app/i18n/en.toml` and `zh-hans.toml`.

### 4. Docs

Update `docs/guide/guides/remote-access.md` and `docs/guide/zh-CN/guides/remote-access.md`: the remote footer shows the other computer's usage, its popover is read-only, the popup row shows latency bars, and the host button is red when the computer cannot be reached. Add `ping` and `usage` to the protocol description in `docs/tech/remote-control.md`, and update regression row `REMOTE-WIN-01` in `docs/tests/regression/remote.md`.

## Verification

### Automated

- The `ping` request, the `pong` reply, the `usage` request and result, and the updated welcome round-trip through the companion fixtures.
- A host answers `ping` with a `pong` carrying the same id, and a route to a host without the feature sends none and reports no latency.
- The median of the last 3 samples, the 10-second rule, and clearing samples when the route leaves Connected.
- A host answers `usage` from its snapshot without waiting for a fetch, lists only its enabled and detected agents, names the active Claude account only when it has several, and leaks no account folder. A periodic request starts a refresh only after 15 minutes, a popover request after 5.
- A remote window renders the host's figures and not this computer's; a local window renders this computer's. A host without `usage` leaves the slot empty, an agent this Runner does not know is skipped, and a route that is not Connected dims the last figures.
- The remote usage popover has the computer in its title, no refresh or agent settings button, and the account name without the account menu.
- Bar and colour boundaries at 99, 100, 199, 200, 499 and 500 ms in the popup row, and the row's mark and text in each state.
- The amber margin: a link turns slow at 200 ms and stays slow at 190 ms, returns to normal at 179 ms, and the host button and the row agree throughout.
- `every_connection_state_has_a_host_tone_and_a_row_label` updated for red Unreachable and an amber slow link.

### Live checks

These change what a person sees in the running app, so the change runs in a Claude trio. Each needs a second computer paired with the development build, because latency and the host's usage only exist across a real route.

| Check | Why |
|---|---|
| Connected: a window on the other computer shows that computer's usage, matching its own footer, and the popup row shows bars and a plausible latency | The main behavior, and the first real end-to-end measurement |
| Switching between the other computer and this one swaps the two computers' usage in the same window | The slot must follow the computer the window shows |
| Clicking the host's usage opens the read-only popover with the host's figures and its name in the title; with several Claude accounts on the host, its active account is named | The new popover form, and the account the figures belong to |
| The host's app is closed while its daemon keeps running: its figures still arrive and refresh | A host with no app open does not poll usage on its own |
| Slow: add delay to this computer's network, for example with Network Link Conditioner, until the reading passes 200 ms, then see the host button, bars and text turn amber with the figures at full strength, and green again once the delay is removed | The new amber state, and the margin against flicker on a real link |
| Reconnecting: interrupt the route, then see the figures dim, the host button and the row's icon and text turn amber, and the bars return once it reconnects | A real state transition, and samples must reset |
| Unreachable: stop the host's remote access, then see the red icon and text in the row, the red host button and the dimmed figures | The colour change from today |
| A host on 0.19.5, which has neither `ping` nor `usage`: it stays connected, the usage slot stays empty, the row shows `Connected · relay · encrypted`, and no errors are logged | Compatibility with hosts that have not updated |
| Light and dark appearance, and Glass on macOS: the popup's bars and the dimmed figures stay legible | New marks, and reduced opacity on the translucent sidebar |
