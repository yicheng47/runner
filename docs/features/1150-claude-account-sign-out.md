# 1150 — Sign out of a Claude Code account

Tracking issue: [#1150](https://github.com/yicheng47/runner/issues/1150)

Design: `runner/desktop/specs/1150-claude-account-sign-out.pen` in `yeelinks_design`, signed off by Jason on 2026-11-09 with option A for each open decision. The chosen frames are the menu `fe8N5`, the confirmation `O1LZRk`, the signed-out row `l4fAo9`, the active account `zBJWs` and `m88dN`, the native account `Dkwxx` and Remove `gOUC6`; the B frames beside them were not chosen. Where this spec's copy differs from the canvas, the spec wins.

## Motivation

Runner can sign a Claude Code account in but not out. **Settings → Agents → Claude Code → Accounts** offers **Log in**, and each account's **⋯** menu has Make main, Rename, Reveal in Finder and Remove, but no Sign out. Remove only forgets the account in Runner and leaves its folder and sign-in on disk. To move an account folder to a different Anthropic login, or to revoke a sign-in on a shared machine, you have to leave Runner and run `CLAUDE_CONFIG_DIR=<folder> claude auth logout` in a terminal, or `/logout` inside a chat on that account.

## Scope

### Menu

Each signed-in account's **⋯** menu gains **Sign out** (Lucide `log-out`), under the separator and directly above **Remove**. It appears only while `claude auth status` reports the account signed in: not while it is signed out, signing in, being checked or unavailable. It is enabled for every signed-in account, including the main, the active and the native account.

### Confirmation

Sign out always asks first, with the app's `ConfirmDialog` and its default Danger button.

| Part | Copy | Shown |
|---|---|---|
| Title | Sign out of {account}? | Always |
| Body | Claude Code removes {account}'s sign-in for {email} from {folder}. The folder, its settings and conversation history stay, and Log in signs it in again. | Always; without "for {email}" when no email is known |
| Active note | {account} is the active account. New Claude Code chats start signed out until you log in again or switch to another account. | The account is active |
| Native note | {account} is Claude Code's own sign-in. Signing out here also signs out claude in your terminal and every other app on this computer that uses ~/.claude. | The account is the native one |
| Running line | 1 running session on {account} keeps its sign-in until Claude Code asks it to log in again. / {count} running sessions on {account} keep their sign-in until Claude Code asks them to log in again. | At least one running session uses the account |
| Buttons | Cancel · Sign out (busy: Signing out…) | Always |

`{folder}` is shown as the row shows it (`~/.claude` for the native account). A note is the design's raised callout with a warning icon, between the body and the running line; an account that is both active and native shows both notes. If `ConfirmDialog` has no place for a note, add an optional one to it rather than a new dialog.

The running count covers live Claude Code sessions, chats and mission slots alike, whose recorded account (`ClaudeSessionAccount`) resolves to this account, the same way resume and the chat side panel resolve it: a session with no account id resolves to the native account.

### Signing out

A new `ClaudeAccountAction::SignOut { id }`. Under the accounts mutation lock, the daemon stops and forgets any login job for the account, as Cancel does, then runs `claude auth logout` through `command`, with the account's `CLAUDE_CONFIG_DIR` (none for the native account) and the session `PATH`, and waits for it with a bounded timeout. It then refreshes that account's status and emits `claude_accounts/changed`. A non-zero exit or a timeout returns an error, which the app shows through `account_error` like other account actions; the refreshed status shows whatever Claude Code reports. Runner deletes nothing itself: Claude Code owns its credential store, which is the Keychain on macOS.

Nothing else changes: the active account stays active, main stays main and sharing links stay as they are. Sessions already running keep going until Claude Code rejects their token.

### After signing out

The row shows **Not signed in** with **Log in**, without the email and plan line, as an account that never signed in does; an active account keeps its **Active** badge. The account leaves the usage popover's account dropdown, as unsigned accounts do today.

### Remove

Remove stays as it is: immediate, with no dialog, forgetting the account in Runner only. The note under the account list (`accounts.keep_folder`) becomes: "Signing out keeps an account's folder and history. Removing an account only forgets it here; sign out first to remove its sign-in too."

### Non-goals

No sign-out from the usage popover or the chat side panel, no Remove dialog, no change to Log in, sharing or Make main, and no deletion of account folders.

### Guide

[Claude Code accounts](../guide/guides/claude-accounts.md) and its `zh-CN` counterpart: the "Rename, make main or remove" section adds Sign out, the notes for the active and native accounts, and the new Remove wording. Fix any link to that section's anchor if its heading changes.

## Implementation Phases

1. **Protocol and daemon.** `SignOut` in `runner-core`'s `ClaudeAccountAction`, handled in `runner-daemon`'s `claude_accounts::action` with the logout job above.
2. **App.** The menu item and its dispatch in `surfaces/settings/claude_accounts.rs` (the handler maps item indices today, and Sign out is conditional, so the mapping must hold with and without it and with Stop using {main}'s settings), the confirmation overlay beside `AccountSharingOverlay`, the running count, the `accounts.keep_folder` copy, and new strings in `i18n/en.toml` and `i18n/zh-hans.toml`.
3. **Guide**, as above.

## Verification

Automated tests: the daemon runs `auth logout` with the account's folder (and without `CLAUDE_CONFIG_DIR` for the native account), stops a pending login first, refreshes the status and emits the change, and returns an error when logout fails; the app shows Sign out only for signed-in accounts, above Remove, for the active, main and native accounts too, dispatches every item correctly with and without the conditional items, and builds the confirmation's notes and running line for each case.

Live checks run in the macOS development app against a stub `claude`, never a real sign-in; the [mission brief](../impls/briefs/1150-claude-account-sign-out.md) lists them.
