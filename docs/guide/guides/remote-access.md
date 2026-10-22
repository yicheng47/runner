# Remote access

Connect this computer to your account so Runner on your other computers can reach its chats and missions. Remote access is a private beta, invite-only and available only in mainland China for now. Signing in to the hosted service needs an account created from an invite code. Sessions are encrypted end to end; the service relays them.

## Sign in from Runner

1. Open **Settings → Account** and choose **Sign in**, or open **Settings → Remote access** and choose **Sign in to Runner** under **Through Runner’s relay**.
2. Runner shows a short code, a fingerprint and the time left, and opens the approval page in your browser. You can also copy the link or open the short link on another device and enter the code.
3. Sign in, check that the page shows the same fingerprint as Runner, name this computer and approve it.
4. Account shows your account, service, this computer and its fingerprint. If you started from Remote access, that pane shows the relay page once approval completes.

The round avatar in the sidebar footer shows your account’s initial when signed in and a person icon otherwise. Its menu opens Account, Settings and **Sign out…**. Signed out, it offers Settings and **Sign in…**, which opens Account, where you can start approval. Account also shows what the account is used for: remote access through the relay, with syncing roles and crews planned for later.

If Runner cannot reach the service, it tries for up to 30 seconds, then returns to Not signed in with an error. Check the service address and your network, then choose **Sign in** again.

## Choose a route

Remote access shows two choices while signed out: **Through Runner’s relay**, using your Runner account in the private beta, and **Over Tailscale**, marked **Later** with its button disabled until Phase 4. Tailscale will connect computers directly on the same tailnet without an account or Runner’s service.

Signing in chooses the relay. The relay page shows the connection, **Allow remote access**, the PIN and paired computers. Its Route section identifies the relay and links to Account. A computer uses one route at a time; to use the other route later, choose **Reset remote access**. Reset makes a new identity, unpairs every computer, clears the PIN, signs out and returns to choosing a route. Nothing carries over.

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

## Pause, retry or sign out

Turn off **Allow remote access** to close the connection while keeping this computer on the account. The web panel shows it offline. Turn the switch on to reconnect with the same key and fingerprint.

**Sign out on this computer** in Account, **Sign out…** in the avatar menu, or `runner remote logout`, deletes this computer's enrollment credential first and asks the service to remove it once. If the service cannot be reached, Runner reports that the computer is still listed offline; remove that entry in the web panel. Runner asks for confirmation before signing out in the app. Restarting Runner does not reconnect a signed-out computer. Signing in again needs a new code and keeps the same identity key and fingerprint, PIN and paired devices.

If the computer was removed in the web panel or the account was deactivated, Runner stops the connection. Account shows the reason even when an old enrollment remains; Remote access returns to the route choice with a link to Account. **Try again** uses the existing credential, which can work after an administrator reactivates the account. **Sign in again** clears the old enrollment and starts fresh. Other stop reasons explain whether to retry, approve a new code or update Runner.

Remote access never stops your agents or sessions. Closing the desktop app leaves the connection with the background daemon; stopping the daemon closes it. Keys and credentials stay in the operating system's secret store on macOS and Windows, with a private local file as the current Linux fallback. Development builds use separate storage from the installed app.

## Who can reach this computer

The service routes connections to computers on your account. Terminal access requires a paired computer: signing in to the account alone is not enough.

Set a PIN in **Settings → Remote access → Set PIN**, or run `runner remote pin` in your own interactive terminal outside a Runner agent session. Choose a 6-digit PIN, then enter it again. Leading zeroes are kept. The CLI hides both entries; Runner never shows the PIN again. **Change PIN** replaces it without unpairing existing devices.

Runner’s service never sees the PIN and cannot pair a device on its own. Another Runner enters the PIN once to pair and uses its own key on later connections. The computers section currently lists paired computers with their name, fingerprint, pairing date and last-seen time. A green dot and **Connected now** replace the last-seen time while that computer has an authenticated connection open. When its last connection closes, the last-seen time returns. **Revoke**, or `runner remote revoke <fingerprint>`, removes that device’s trust. Use `runner remote devices` or `runner remote devices --json` to read the list from a terminal. The CLI also reports `Connected`, true while that device has an authenticated channel open; JSON uses `connected`. Revoke closes all of that device's channels.

Five failed pairing confirmations in a row lock PIN entry. A wrong PIN and a damaged encrypted confirmation both count; malformed frames do not. Runner shows a warning and **Unlock** in Settings; no new device can pair until you unlock it. Already-paired devices keep working. Changing the PIN does not unlock entry. The lock survives a restart.

Runner shows **{computer} paired** as a system notification when another computer pairs. Clicking it opens **Settings → Remote access**. If system notifications are unavailable or off, an in-app toast says that the computer can now reach this one and offers **Open Remote access**. A reconnection does not raise another pairing notification.

If you suspect this computer’s identity key has leaked, choose **Reset remote access**, or run `runner remote reset-identity` in your own interactive terminal. Confirming disconnects the computer from the account, creates a new key and fingerprint, clears the PIN and all paired devices, and unlocks PIN entry. The pane returns to the route choice, and signing in again needs a new code. Your agents and sessions keep running.


## Take a terminal back

While another Runner controls a terminal, its pane is covered by **Controlled from {computer}** and **Take back**. The remote computer sets the terminal’s size. The covered pane sends neither its own size nor terminal input, including typing, paste, file drops and IME composition. A mission composer submission addressed to a covered terminal keeps its draft until you take the terminal back.

Choose **Take back**, or press Enter on the card, to return control to this desktop and fit the terminal to its pane. If one computer controls all the terminals in a split tab or mission, one card covers the group; taking it back fits each terminal to its own pane.

When the remote computer leaves, the terminal keeps its last size. The card changes to **{computer} has left** with **Continue here**. Choose it, or press Enter on the card, to fit the terminal here and continue. If Runner did not see the computer’s name, it says **Another computer has left**.

Several desktop windows can show the card, and you can take control back from any of them. With no remote computer using the terminal, desktop input and resizing work as usual. Instructions for pairing and opening another computer from Runner are added with the client.
