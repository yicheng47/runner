# #836 remote control — release live checks

Every live check still pending for Jason before the umbrella reaches `main`, gathered on 2026-10-24 from the program's 13 test records and ordered for one session with two computers. The plan's gate is that the relay route passes Jason's testing ([program record](../impls/836-web-remote-control/README.md)); this list is that testing. Each item names the record it comes from, which keeps the detail and the reason it was selected. Superseded checks are listed at the end and are not run.

## Setup

- **Candidate.** The `remote-nightly` cut after m9 PR 2 merges, installed on both computers. Until then, everything except section 1's Local only items and section 7 can run with a development app built from `.worktrees/nightly-remote` at `0270ad64` as the client and the installed `remote-nightly` `39d957c` as the host, since m9 PR 1 changes only the client.
- **Computers.** One macOS and one Windows computer, signed in to the same account on the hosted service. Run each section with the Mac as client and the Windows computer as host, then swap roles for the items marked *both directions*.
- **Route.** The hosted service only. The local `runner_server` passes m7b and m7c planned were for development before the service had its relay and client enrollment, both deployed on 2026-10-22; they are not required for the release.
- **Recording.** Tick each item and note the platform. A failure goes to Claude Code as a live fix, with what was seen and on which computer; a fix gets its own recheck here.

Already passed: Phase 1's connect checks ([`836-connect-host.md`](836-connect-host.md), Jason, 2026-10-22), and signing in to the hosted service on the current candidate (Jason, 2026-10-24).

## 1. Upgrade, account and Local only

- [ ] An installed host enrolled before the upgrade keeps its fingerprint without enrolling again; Disconnect then Connect keeps it. ([m2](836-m2-host-trust.md#review-and-live-checks))
- [ ] Reset remote access asks for confirmation, changes the fingerprint, removes the old entry from the account's computers, clears the PIN and pairings, and returns to relay sign-in with no route choice or Tailscale card. Local only is unchanged. ([m2](836-m2-host-trust.md#review-and-live-checks), [switch 2](836-remote-access-switch.md#jasons-live-rechecks), spec)
- [ ] macOS: another program reading the identity item raises a Keychain prompt; rebuilding and starting a development build needs no Keychain password. ([m2](836-m2-host-trust.md#review-and-live-checks), [m7b](836-m7b-client-core.md#live-checks-pending-for-jason))
- [ ] Sign in from Account and from Remote access: both show the fingerprint reminder and the same waiting card, and approval returns to the relay page. ([m7a](836-m7a-account.md#live-checks-pending-for-jason))
- [ ] Signed in, Account holds only the account, service, computer and fingerprint card and Sign out, in English and Chinese. ([switch 1](836-remote-access-switch.md#jasons-live-rechecks))
- [ ] The avatar and its menu signed in and signed out (Sign in… opens Account); Sign out confirm and cancel; Pause and Resume. ([m7a](836-m7a-account.md#live-checks-pending-for-jason))
- [ ] In Settings → Remote access, Refresh beside the Computers heading reloads the list without leaving the page, shows loading and prevents another refresh while pending. A failed refresh preserves the rows and warning; a successful retry updates the list and clears the warning. Check English and Chinese.
- [ ] *PR 2.* A fresh install has Local only off in Settings → General; Account and Remote access are listed, the footer has the avatar, and signed out there is no computer picker. (spec, Local only)
- [ ] *PR 2.* Turning Local only on while signed in asks with the Sign out confirmation: cancel changes nothing; confirm signs out, then hides Account, Remote access, the picker and the avatar in every window and shows the plain Settings gear. Open remote windows return to this computer and remote dialogs close. A failed sign-out leaves it off. (spec, Local only)
- [ ] *PR 2.* Local only survives a restart; turning it off restores everything without starting sign-in; `runner remote login` while it is on shows everything until signed out again. (spec, Local only)

## 2. PIN and pairing — both directions

- [ ] Set a PIN: the weak-PIN warning appears in both languages and from `runner remote pin`; declining keeps the old PIN, accepting keeps the weak one; a PIN with leading zeroes is accepted. ([live fixes 5](836-remote-live-fixes.md#problem-5-revision-warn-and-allow))
- [ ] Pair from the computers list and from the picker: a wrong PIN, a locked host and a host with no PIN each show their state; Checking prevents a second submit; a changed fingerprint is shown before pairing again. ([m7c 1](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] The host shows the new-pair system notification, or the toast when notifications are off. ([m6](836-m6-host-desktop.md#live-checks-pending-for-jason))
- [ ] The computers list shows ↙, ↗, ⇄ and Not paired with the legend, and ⋯ has Revoke and Forget. (spec, Remote access)
- [ ] Changing the PIN keeps existing pairings and open terminals; Pair again and Forget remove the saved trust after the fingerprint is confirmed. ([live fixes 7](836-remote-live-fixes.md#problem-7-pending-pairing-survives-a-pin-change))

## 3. Picker and windows

- [ ] The picker is opaque in Glass and Solid, dark and light, including its directory-error state, and dismisses with Escape, an outside click and its trigger. ([live fixes 1](836-remote-live-fixes.md#problem-1-transparent-computer-picker), WS-MENU-01)
- [ ] Rows show status first, offline with last seen, then Not paired; the window button and ⋯ stay in place. ([switch 6](836-remote-access-switch.md#jasons-live-rechecks))
- [ ] An accent dot, with no count, lights on the host's row and the trigger when a host session waits for approval or input or a host mission has a question, and clears once answered. Unread output alone does not light it; offline, unpaired and locked computers have none. ([m9](836-m9-front-page.md#jasons-pending-live-checks))
- [ ] Open the host in the same window, with Shift-click and with the window button. Returning to this computer keeps its tabs, panes and mission selection, its terminals take input and its feed has caught up; after a restart every window opens on this computer. ([m7c 2](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] The remote sidebar has PINNED, PROJECTS and RECENTS, project indentation and collapse, provider, split and mission icons, no blank titles, and follows the host's order and title changes; the local sidebar's state is unchanged on return. ([live fixes 3](836-remote-live-fixes.md#problem-3-blank-remote-sidebar-tab-labels))
- [ ] With the host on this build, two pi role chats show their `@handles` and a renamed chat keeps its name in pinned, project, recent, Active and headers; an older host keeps its runtime-name fallback.
- [ ] Remote rows match local ones for working, unread, approval, answer, input, error and dimmed stopped, including tooltips and pushed changes. ([live fixes, sidebar status](836-remote-live-fixes.md#shared-sidebar-status-presentation))

## 4. Terminals — both directions

- [ ] TERM-INPUT-01 in a remote terminal: typed text and pasted Chinese, combining accents and emoji arrive once and in order. ([m5](836-m5-adapter.md#qa-matrix), [m7c 3](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] TERM-RESIZE-01 and TERM-RESIZE-02 on each computer's own desktop, then on a remote terminal: the host follows the controller's size. ([m5](836-m5-adapter.md#qa-matrix), [m6](836-m6-host-desktop.md#live-checks-pending-for-jason))
- [ ] The host shows the control card on the pane, one card over a split tab and over a mission; Take back on the host, then Take control on the client; each terminal's size is restored. ([m6](836-m6-host-desktop.md#live-checks-pending-for-jason), [m7c 3](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] Leaving the host releases its terminals while its agents keep running and the size stays; the host's list shows Connected to this computer while connected, through a reconnect, and not after disconnecting. ([m7c 3, 5](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] Drop the route while typing: reconnecting, then the unreachable card; on reconnect the output is fresh and nothing typed while disconnected arrives. Revoke on the host shows the revoked card and needs pairing again. ([m7c 4](836-m7c-client-app.md#live-checks-pending-for-jason))
- [ ] #838's two-window steps on the host: resizing, switching and taking back from either window leaves one owner and no duplicated prompt fragments. ([m6](836-m6-host-desktop.md#live-checks-pending-for-jason))
- [ ] A mission message to a member with an unsent composer draft is held and delivered after the draft is sent or cleared. ([m5](836-m5-adapter.md#qa-matrix))

## 5. Chats on the other computer

- [ ] New chat, Direct and Role, offers the host's agents, roles, models, effort, speed and Claude account, and starts on the host. (WS-CREATE-01/02/03, [m9](836-m9-front-page.md#jasons-pending-live-checks))
- [ ] Working directory → Browse: Projects and Recent, Browse folders… through home, a subfolder and Parent; an empty folder can be used; a failed listing offers Retry; Escape or an outside click keeps the field. A typed host path is accepted or shows its error under the field; a blank field uses the host's default. New project uses the same menu. ([m9](836-m9-front-page.md#jasons-pending-live-checks), frames `k3GCC`–`m1P4o`)
- [ ] Open a stopped/crashed remote chat or shell: only its paused/exited card appears, without a terminal-open failure or connection spinner. Resume/Restart attaches after the host pushes running status; a real attachment failure on a running chat remains visible and retryable.
- [ ] Stop, resume, restart a shell, fork, rename and archive a chat; the host's desktop shows each change at once. ([m9](836-m9-front-page.md#jasons-pending-live-checks))

## 6. Tabs, panes, drawer and sidebar on the other computer

- [ ] New tab; split, fill, resize and close panes; close a tab. (WS-PANE-01/02/03/04)
- [ ] Add and close drawer shells. (WS-DRAWER-01/02)
- [ ] Pin and unpin, move between projects, reorder and rename; create, rename and delete a project, including the running-member confirmation. (WS-SIDEBAR-01)
- [ ] Search, the Active filter and the command palette show only the host's chats and missions; Settings, role/crew management and Archived are not reachable from the remote window. Role and Crew show only the host’s read-only catalog: search, paging, Refresh/error recovery and chat/mission forms preselect the clicked item without exposing create/edit/delete. ([m9](836-m9-front-page.md#pr-1-selected-checks))

## 7. Missions on the other computer — PR 2

- [ ] Start mission from an existing crew with a goal and a host folder from the menu; the host shows it at once. (MIS-LIFE-01)
- [ ] The mission's feed, composer posts and answer cards; stop, resume and archive it. (MIS-LIFE-02/03, MIS-ASK-02)
- [ ] Members keep the mission's pane arrangement through stop and resume.

## 8. Older host

- [ ] A host on a build from before m8 (`remote-nightly` `864bc82` or earlier) hides the actions it cannot serve, while its terminals stay usable. ([m9](836-m9-front-page.md#jasons-pending-live-checks))

## Superseded — not run

- The Remote access switch's rechecks 3, 4, 5 and 7 ([switch](836-remote-access-switch.md#jasons-live-rechecks)): Local only replaced the switch (Jason, 2026-10-24); its checks are in section 1.
- m7a's "Reset returns to route choice": Reset returns to relay sign-in since 2026-10-23.
- m7c 2's "dimmed creation actions": creation works on the other computer since m9.
- The live fixes' signed-out picker (Problem 2): signed out, there is no picker (Jason, 2026-10-24). Their "no amber count" check (Problem 4) is replaced by the accent dots in section 3.
- m2's "removes the host from the web panel" and m5's browser adapter row: clients are Runner apps, with no web panel (Jason, 2026-10-21).
