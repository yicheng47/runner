# Claude Code accounts

Open **Settings → Agents → Claude Code → Accounts** to manage separate Claude Code sign-ins. Your existing native sign-in appears as **Personal**. Runner stores account names, folders and selections; Claude Code keeps each sign-in in its own configuration folder and credential store.

## Add an account

Choose **Add account**, enter a name and leave the folder on Auto, or use **Browse** to select an existing account folder. Auto derives a folder such as `~/.claude-work` from the name. **Use Personal’s settings** is selected by default; clear it to keep an independent setup. Choose **Add and log in** and complete Claude Code’s browser sign-in. An existing signed-in folder is recognized without signing in again. Runner completes initial setup for an added signed-in account so its first interactive chat opens past Claude Code’s onboarding. During sign-in, **Open sign-in page** reopens the page and **Cancel** stops the attempt.

Each account shows its sign-in status, email and any available organization or plan. An account can be signed in without a plan label.

## Switch the active account

Choose **Use** beside a signed-in account, or select it from the Claude Code dropdown in the usage popover. The dropdown lists signed-in accounts and their weekly usage. **Manage accounts…** opens the Claude Code settings page. The popover, footer and **Settings → Usage** show the active account’s usage. Usage settings control sidebar badges, the check interval and the 5-hour window. Near-the-limit notifications cover the active account only.

The selected account shows **Active**, including when it is also **main**; both badges appear together.

The active account applies to new direct chats, role chats, crew members, resumed chats and recovery after a restart. Running sessions keep their original account. With several accounts configured, a Claude Code pane shows its account after the title, and its side panel shows **Account** and the `config` folder.

A role or launch environment that explicitly sets `CLAUDE_CONFIG_DIR` keeps that pin. An explicit `~/.claude` pin is distinct from the original native sign-in, which leaves the variable unset.

## Resume and fork

A paused chat says which account it ran on and which active account will continue it. **Resume** continues the same saved conversation; switching accounts does not copy or move its history. A fork starts a separate conversation in the active account’s folder. Existing chats retain their recorded history location even after an inactive account is forgotten.

Starting a new conversation with Claude Code’s `/clear` uses the account running that process. After resuming a Personal conversation with Work, `/clear` starts a new conversation in Work’s folder, and subsequent Resume follows that new history. Selecting another active account while the process is running does not change this location.

A setup that has never added another account keeps the existing native launch, resume, fork and usage behavior, without an account chip or dropdown.

## Share the main account’s configuration

Choose **Use Personal’s settings** on another account’s row, then confirm. The name follows your current main account. Runner shares global memory, settings, skills, agents, commands and enabled installed plugins, plus user MCP servers for new sessions. The account’s `settings.json` links to main’s, so the account starts with main’s model, effort, permission and hook settings, and a setting changed from that account, such as with `/model`, changes main’s file. It adds individual skill links inside the account’s own `skills/` directory, preserving same-named local skills, synced skills and dot-folders. New main skills become available on the next shared launch. Each account keeps its own plugin directory, cache and installed/synced state; shared plugins load from main’s install directories at launch. The row shows shared skill/plugin counts and retained local state. Existing files and links stay in place; an account with its own settings file keeps it, and a role’s explicit settings override still takes precedence.

Choose **Stop using Personal’s settings** in the account’s **⋯** menu to disconnect. Runner removes only links it created whose targets still match its records, including individual skill links. The real skill directory, local/synced skills and plugin state stay in place. Removing an account uses the same cleanup. Your folder, own files, sign-in and conversation history stay on disk. On Windows, sharing uses directory junctions and a memory import, and links `settings.json` with a symbolic link when Developer Mode is on. Without Developer Mode, Runner passes main’s settings to each new session at launch instead; Claude Code may ignore some settings delivered that way, so turn on Developer Mode if the account doesn’t start like main.

**Settings → Agents → Claude Code → Skills / MCP** opens on the main account; choose another account from the dropdown to inspect and edit its own configuration. Sharing accounts show a note explaining which settings are shared. This selection does not change the active account and resets on each visit. **Make main** changes the default configuration owner, model discovery and update-channel lookup to the selected account. The promoted account’s managed links are removed and other linked accounts follow the new owner. Its own files become the shared configuration; Runner does not copy the previous owner’s files. If Runner cannot finish because a folder is inaccessible, the previous main stays selected. Restore access and choose **Retry** in Accounts. Accounts involved in that change wait to launch until it finishes; the pending change survives restarting Runner.

## Rename, make main, sign out or remove

An account’s **⋯** menu offers **Rename**, **Make main**, **Reveal in Finder** (**Show in Explorer** on Windows), **Sign out** and **Remove**. Renaming leaves its folder unchanged. Main identifies the owner of shared configuration; changing main never changes an account’s sign-in target.

**Sign out** appears for signed-in accounts and always asks for confirmation. Claude Code removes the account’s sign-in; its folder, settings and conversation history stay. **Log in** signs it in again. Signing out keeps main, active and sharing selections unchanged. An active account stays **Active**, and new Claude Code chats start signed out until you log in or switch accounts. Signing out the original native account also signs out `claude` in your terminal and other apps using `~/.claude`. Running chats and mission sessions keep their sign-in until Claude Code asks them to log in again; the confirmation shows their count.

Menu labels stay on one line; long account names are shortened to fit. The main account, active account and original native account cannot be removed; hover the disabled Remove item for the reason. Removing another account is immediate and only forgets it in Runner, preserving its folder, files and sign-in on disk. Sign out first to remove its sign-in too. Switch away from an active account before removing it.
