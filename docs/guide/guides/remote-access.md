# Remote access

Connect this computer to your account so the web panel can show it online. Remote access is a private beta, invite-only and available only in mainland China for now. Signing in to the hosted service needs an account created from an invite code. Browser access to chats, missions and terminals will arrive in a later update, with end-to-end encryption.

## Connect from Runner

1. Open **Settings → Remote access** and choose **Connect this computer**.
2. Runner shows a short code, a fingerprint and the time left, and opens the approval page in your browser. You can also copy the link or open the short link on another device and enter the code.
3. Sign in, check that the page shows the same fingerprint as Runner, name this computer and approve it.
4. Runner shows **Connected as @username**. The computer appears online in the web panel.

If Runner cannot reach the service, it tries for up to 30 seconds, then returns to Not connected with an error. Check the service address and your network, then choose **Connect this computer** again.

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

## Pause, retry or disconnect

Turn off **Allow remote access** to close the connection while keeping this computer on the account. The web panel shows it offline. Turn the switch on to reconnect with the same key and fingerprint.

**Disconnect**, or `runner remote logout`, deletes this computer's enrollment credential first and asks the service to remove it once. If the service cannot be reached, Runner reports that the computer is still listed offline; remove that entry in the web panel. Restarting Runner does not reconnect a disconnected computer. Connecting again needs a new code and keeps the same identity key and fingerprint, PIN and paired devices.

If the computer was removed in the web panel or the account was deactivated, Runner stops the connection. **Try again** uses the existing credential, which can work after an administrator reactivates the account. **Connect again** clears the old enrollment and starts fresh. Other stop reasons explain whether to retry, approve a new code or update Runner.

Remote access never stops your agents or sessions. Closing the desktop app leaves the connection with the background daemon; stopping the daemon closes it. Keys and credentials stay in the operating system's secret store on macOS and Windows, with a private local file as the current Linux fallback. Development builds use separate storage from the installed app.

## Who can reach this computer

The service routes connections to computers on your account. Access to terminals will also require a paired device: signing in to the account alone is not enough. This update prepares the host’s PIN and trust settings; device pairing and terminal access arrive in a later update.

Set a PIN in **Settings → Remote access → Set PIN**, or run `runner remote pin` in your own interactive terminal outside a Runner agent session. Choose a 6-digit PIN, then enter it again. Leading zeroes are kept. The CLI hides both entries; Runner never shows the PIN again. **Change PIN** replaces it without unpairing existing devices.

Runner’s service never sees the PIN and cannot pair a device on its own. When pairing becomes available, a device will enter the PIN once and use its own key on later connections. **Devices** lists the name, fingerprint, pairing date and last-seen time of each paired device. **Revoke**, or `runner remote revoke <fingerprint>`, removes that device’s trust. Use `runner remote devices` or `runner remote devices --json` to read the list from a terminal. The CLI also reports `Connected`, true while that device has an authenticated channel open; JSON uses `connected`. Revoke closes all of that device's channels.

Five failed pairing confirmations in a row lock PIN entry. A wrong PIN and a damaged encrypted confirmation both count; malformed frames do not. Runner shows a warning and **Unlock** in Settings; no new device can pair until you unlock it. Already-paired devices keep working. Changing the PIN does not unlock entry. The lock survives a restart.

A browser runs code supplied by the service. A malicious version of that code could capture a PIN as you type it, or use a paired browser’s key while the page is open, though it could not copy the key out. Pairing prevents the service from silently accessing hosts on its own; it cannot protect a browser from the site serving its code. A native client whose code does not come from the service does not have this browser limit.

If you suspect this computer’s identity key has leaked, choose **Reset identity**, or run `runner remote reset-identity` in your own interactive terminal. Confirming disconnects the computer from the account, creates a new key and fingerprint, clears the PIN and all paired devices, and unlocks PIN entry. Connecting again needs a new code. Your agents and sessions keep running.
