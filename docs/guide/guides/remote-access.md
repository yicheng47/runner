# Remote access

Connect this computer to your account so Runner on your other computers can reach its chats and missions. Remote access is a private beta, invite-only and available only in mainland China for now. Signing in to the hosted service needs an account created from an invite code. Sessions are encrypted end to end; the service relays them.

## Turn on Remote access

Open **Settings → Remote access** and turn on **Remote access**. It is off by default during the private beta. Turning it on adds Account in Settings, the sidebar’s computer picker, and pairing; it does not start sign-in. While off, Runner shows the plain Settings gear in its footer and keeps Remote access in Settings so you can find it. The choice is saved and applies to every window.

A computer that is already signed in, including through `runner remote login`, always shows Remote access as on. Signing out from Account or the avatar menu keeps the Remote access switch on. Turning the switch off while signed in asks for Sign out confirmation first. Cancel keeps everything visible; confirm signs this computer out and hides the remote surfaces once it is signed out. Its identity, PIN and paired computers stay. A failed sign-out keeps remote surfaces visible and shows the error. Windows viewing another computer return to this computer when Remote access becomes off. PIN and Reset dialogs in other windows close too.

## Sign in from Runner

1. Turn on **Settings → Remote access → Remote access**, then open **Settings → Account** and choose **Sign in**, or open **Settings → Remote access** and choose **Sign in to Runner** under **Through Runner’s relay**.
2. Runner shows a short code, a fingerprint and the time left, and opens the approval page in your browser. You can also copy the link or open the short link on another device and enter the code.
3. Sign in, check that the page shows the same fingerprint as Runner, name this computer and approve it.
4. Account shows your account, service, this computer and its fingerprint. If you started from Remote access, that pane shows the relay page once approval completes.

While Remote access is on, the round avatar in the sidebar footer shows your account’s initial when signed in and a person icon otherwise. Its menu opens Account, Settings and **Sign out…**. Signed out, it offers Settings and **Sign in…**, which opens Account, where you can start approval. Account shows its status card and **Sign out on this computer** when signed in.

If Runner cannot reach the service, it tries for up to 30 seconds, then returns to Not signed in with an error. Check the service address and your network, then choose **Sign in** again.

## Through Runner’s relay

With its switch on, Remote access opens directly on the relay’s sign-in state while signed out. Runner’s relay is the only route in this release; Tailscale is planned separately.

The relay page shows the connection, **Allow remote access**, the PIN and paired computers. Its Route section identifies the relay and links to Account. **Reset remote access** makes a new identity, unpairs every computer, clears the PIN, signs out and returns to relay sign-in with Remote access still on.

## Connect from a terminal

Run these commands in your own interactive terminal, including over SSH:

```sh
runner remote login
runner remote status
runner remote status --json
runner remote logout
```

`login` displays the approval link, code and fingerprint. Open the link yourself; it does not open a browser on the computer. After browser approval, confirm the account in the terminal. The default answer is No. Use `runner remote login --server https://runner.example` to choose another service.

Enrollment needs both input and output attached to a terminal, and is refused inside Runner agent sessions. Agents can read status, list or revoke paired devices, or disconnect the computer. They cannot enroll it, set or change its PIN, unlock PIN entry or reset its identity through `runner call`. Closing the CLI during account confirmation refuses the claim.

## Reach another computer from the CLI

Sign both computers in to the same account. On the computer you want to reach, allow remote access and set its PIN. On this computer, run:

```sh
runner remote computers
runner remote computers --json
runner remote pair <computer-id>
runner remote overview <computer-id> --json
runner remote forget <computer-id>
```

`computers` lists the account's computers, including this one, with their IDs, online status, last-seen time and whether this computer has paired with each. Use an ID from that list. `pair` prompts for the other computer's PIN without echoing it; it requires your own interactive terminal and is refused inside an agent session. A failed pairing is one attempt, and is never retried automatically. `overview` shows the other computer's projects, tabs, missions and sessions.

Each pairing keeps a separate private key and pins the other computer's identity. Long Unicode computer names use a shortened pairing label of at most 64 UTF-8 bytes; the full account name stays unchanged. Later connections use that key. **Forget** deletes this computer's key for the other computer and closes the channel; it does not revoke the other computer's access to this one. **Revoke** on the host removes an incoming pairing. If a connection reports that this computer is not recognized, Runner keeps its saved pairing key: a relay can forge that unauthenticated rejection. Verify the host's fingerprint before explicitly choosing **Forget** and pairing again from the CLI, or submitting **Pair again** in the app. An identity mismatch fails without silently pairing again.

Turning off this computer's **Allow remote access** stops incoming access while it can still reach other computers. Signing out keeps paired keys, and **Reset remote access** deletes them. The app’s computer picker uses the same pairing and terminal connection as these CLI commands.

## Pair and open another computer in the app

Sign both computers in to the same account. On the computer you want to reach, enable **Allow remote access** and set a PIN. On this computer, choose **Pair** beside it in **Settings → Remote access**, or select it in the sidebar’s computer picker. Enter the other computer’s six-digit PIN. Each submit makes one attempt; Runner never retries a PIN automatically. A wrong PIN lets you try again. If entry is locked or no PIN is set, follow the dialog’s instructions on the other computer first. Check a changed fingerprint in that computer’s Settings before pairing again. If the listed identity changes while the dialog is open, Runner clears the PIN and shows the new fingerprint for you to verify.

The picker lists this computer and the account’s other computers, including offline and unpaired ones. Each remote row shows online or offline status first, including when it was last seen when available; an unpaired computer adds **Not paired**, such as **Online · Not paired**. A green dot on a remote row means a session there needs input or approval, or a mission has a question. The trigger’s dot means another paired computer needs you. Dots carry no count, and unread output alone does not light them. Runner follows online paired computers while Remote access is on, sharing the follow with open windows. Offline, unpaired and locked computers have no attention follow. Click a computer to switch this window; hold Shift while clicking, or use its window button, to open another window. Choose this computer to return to your local tabs and panes. A local mission keeps its layout and selection; its terminals resume and its feed catches up when you return. Each window chooses independently, and every window opens on this computer after restarting Runner. Your local notification settings continue to apply while a remote window is active; clicking a local chat or mission alert returns a window to this computer and opens that item.

With Remote access on and this computer signed out, the picker shows **This computer** and **Sign in to see your computers…**. That action opens Settings → Account; start approval there when ready. Tab and Shift-Tab move through the picker’s controls; moving past either end closes it. Escape closes it while a control inside is focused. Signed out is a normal state: the picker does not fetch account computers, show old remote rows or display a refresh warning.

If the computer list cannot refresh while signed in, the picker shows a warning below its available rows. Check Account and your connection, then reopen the picker to try again.

A remote window shows only that computer’s projects, tabs, missions and terminals. Unnamed tabs show their first populated pane’s session title; a name you give the tab takes precedence. Empty tabs show the same start surface as local tabs. Titles follow updates from the other computer. Pinned items, projects and recents use the same sidebar structure; projects and sections can be collapsed. Update the host as well as the client for matching mixed tab/mission and pin order. Opening a remote view keeps its scroll and collapse choices separate from your local sidebar. Sidebar rows use the same status indicators as local rows: working, unread, approval or answer needed, and errors. When several sessions need attention, the row uses the local priority; stopped rows are dimmed. A pending mission question keeps its answer-needed indicator. New chat uses the host’s agents and roles, model/effort/speed choices and active Claude account. Settings, Roles, Crews and Archived remain available only on this computer. Choose a tab or mission to open its terminals; the first available tab or mission opens when you enter. Remote mission feeds, replies and posting are planned for later. Shells already attached to a remote tab or mission can also be opened.

Use the same chat header, splits, pane controls, terminal drawer and sidebar menus to start, stop, resume, fork, rename or archive chats; organize tabs and projects with pin, move and reorder. Persisted changes appear when the host confirms them, and its desktop sees the same changes. A host that lacks an action hides its controls; its existing terminals remain available.

In New chat, **Working directory → Browse** opens the other computer’s folders. Choose a project or recent folder, or **Browse folders** to visit its home, enter subfolders and go to Parent. **Use folder** fills the field. Escape or clicking outside keeps the previous value. You can type a host path, including a Windows drive or network path; an invalid path keeps its text and shows an error below the field. A blank field uses the host’s default. **New project** uses the same folder menu. A failed listing offers **Retry**. Starting missions and operating their feed are the next stage; existing mission terminals and sidebar Stop/Resume/Archive are available now.

Opening a terminal takes control at its pane’s size. Type, paste text, select, copy and resize as in a local terminal. If the host desktop or another client takes control, the terminal remains visible with typing off and **Take control** above it. Leaving the computer releases its terminal attachments and keeps the last size on the host.

The sidebar footer shows the computer’s encrypted relay connection. During reconnection, typing is dropped; it is never replayed. After a connection to the local daemon is restored, terminal panes load fresh snapshots without taking control again. If a terminal cannot reattach, it shows an error; select its tab again to retry. If Runner cannot reach the computer, choose **Reconnect** or **Close Tab** to return to this computer. If access is not recognized, the saved pairing key stays in place. Verify the other computer's fingerprint before choosing **Pair again**. Submitting it explicitly forgets that saved key before making one PIN attempt. A failed **Take control** shows an error in the pane; use **Take control** again to retry. Choosing the computer this window already shows keeps its current terminals unless the connection has failed. The picker’s **⋯ → Forget** deletes this computer’s outgoing key and shows that it was forgotten here; pair with its PIN to open it again. **Revoke** removes incoming access. Neither action stops the remote agents.

## Pause, retry or sign out

Turn off **Allow remote access** to close the connection while keeping this computer on the account. The web panel shows it offline. Turn the switch on to reconnect with the same key and fingerprint.

**Sign out on this computer** in Account, **Sign out…** in the avatar menu, or `runner remote logout`, deletes this computer's enrollment credential first and asks the service to remove it once. If the service cannot be reached, Runner reports that the computer is still listed offline; remove that entry in the web panel. Runner asks for confirmation before signing out in the app. Restarting Runner does not reconnect a signed-out computer. Signing in again needs a new code and keeps the same identity key and fingerprint, PIN and paired devices.

If the computer was removed in the web panel or the account was deactivated, Runner stops the connection. Account shows the reason even when an old enrollment remains; Remote access returns to relay sign-in with a link to Account. **Try again** uses the existing credential, which can work after an administrator reactivates the account. **Sign in again** clears the old enrollment and starts fresh. Other stop reasons explain whether to retry, approve a new code or update Runner.

Remote access never stops your agents or sessions. Closing the desktop app leaves the connection with the background daemon; stopping the daemon closes it. Release builds keep keys and credentials in the macOS Keychain or Windows Credential Manager. macOS development builds use separate files with mode 0600 under the development data directory (`com.wycstudios.runner-dev`): `remote-identity.json`, `remote-client-keys.json`, and one `remote-credential-<hash>.json` per service. Rebuilding the development daemon therefore does not prompt for the login Keychain password. Existing development Keychain secrets are not copied into these files; sign in and pair again when first using this storage. Linux also uses private files. Development storage is separate from the installed app.

## Who can reach this computer

The service routes connections to computers on your account. Terminal access requires a paired computer: signing in to the account alone is not enough. Treat paired computers as trusted to operate your chats, missions and projects as well as type into terminals. New chats use the host’s active Claude account. Settings, account selection, editing roles and crews, and restoring or deleting archived items stay on the host.

Set a PIN in **Settings → Remote access → Set PIN**, or run `runner remote pin` in your own interactive terminal outside a Runner agent session. Choose a 6-digit PIN, then enter it again. Any six digits are allowed, including leading zeroes. If you choose repeated single digits, an ascending or descending sequence, or a common PIN, Runner warns that it could be guessed within five tries. **Use this PIN** keeps your choice; **Choose another** clears the entries so you can pick again. The CLI shows the warning in English and Chinese and asks for a separate `y` or `yes`; Enter alone cancels without changing the PIN. The CLI hides both entries; Runner never shows the PIN again. **Change PIN** replaces it without unpairing existing devices. Changing the PIN, turning off **Allow remote access**, signing out or resetting remote access invalidates pairings still in progress, including confirmations waiting for their trust save.

Runner does not send the PIN to its service. Pairing requires the PIN; account credentials alone do not grant terminal access. The PIN exchange prevents offline guessing, while the lock below limits online guesses. Another Runner enters the PIN once to pair and uses its own key on later connections. The computers section lists every computer on your account, with this computer first. Each row shows whether it is online, offline with its last-seen time, or connected to this computer. The access column distinguishes **↙ Can open this computer**, **↗ This computer can open it**, **⇄ Both ways**, and **Not paired**. Pairing grants access in one direction; both ways requires pairing from each computer. The incoming label matches a paired device’s name to the account’s computer name; it does not grant trust. Paired devices with no matching account computer, or with a pairing label shared by multiple account computers, remain listed as separate rows below the computers, each with its own **Revoke**. An ambiguous label attributes incoming access to neither computer. The row’s **⋯ → Revoke**, or `runner remote revoke <fingerprint>`, removes incoming trust. Use `runner remote devices` or `runner remote devices --json` to read the list from a terminal. The CLI also reports `Connected`, true while that device has an authenticated channel open; JSON uses `connected`. Revoke closes all of that device's channels.

Five failed pairing confirmations in a row lock PIN entry. A wrong PIN and a damaged encrypted confirmation both count; malformed frames do not. Runner shows a warning and **Unlock** in Settings; no new device can pair until you unlock it. Already-paired devices keep working. Changing the PIN does not unlock entry. The lock survives a restart.

Runner shows **{computer} paired** as a system notification when another computer pairs. Clicking it opens **Settings → Remote access**. If system notifications are unavailable or off, an in-app toast says that the computer can now reach this one and offers **Open Remote access**. A reconnection does not raise another pairing notification.

If you suspect this computer’s identity key has leaked, choose **Reset remote access**, or run `runner remote reset-identity` in your own interactive terminal. Confirming disconnects the computer from the account, creates a new key and fingerprint, clears the PIN and all paired devices, and unlocks PIN entry. The pane returns to relay sign-in, and signing in again needs a new code. Your agents and sessions keep running.


## Take a terminal back

While another Runner controls a terminal, its pane is covered by **Controlled from {computer}** and **Take back**. The remote computer sets the terminal’s size. The covered pane sends neither its own size nor terminal input, including typing, paste, file drops and IME composition. A mission composer submission addressed to a covered terminal keeps its draft until you take the terminal back.

Choose **Take back**, or press Enter on the card, to return control to this desktop and fit the terminal to its pane. If one computer controls all the terminals in a split tab or mission, one card covers the group; taking it back fits each terminal to its own pane.

When the remote computer leaves, the terminal keeps its last size. The card changes to **{computer} has left** with **Continue here**. Choose it, or press Enter on the card, to fit the terminal here and continue. If Runner did not see the computer’s name, it says **Another computer has left**.

Several desktop windows can show the card, and you can take control back from any of them. With no remote computer using the terminal, desktop input and resizing work as usual. Use the CLI above to pair and read another computer’s overview; opening its terminals in the desktop follows with the computer picker.
