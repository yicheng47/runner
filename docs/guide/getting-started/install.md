# Install Runner

Install Runner and sign in to an agent CLI to start working. Runner provides the workspace and terminals; your agent CLI provides the model connection and uses its own account.

## Download the app

Download the latest stable build from [Runner downloads](https://runnersh.dev/downloads/) or [GitHub Releases](https://github.com/yicheng47/runner/releases/latest).

| Platform | Package | Requirements |
| --- | --- | --- |
| macOS | Signed and notarized `.dmg` | macOS 12 or later, Apple Silicon Mac |
| Windows | Signed `Runner-Setup-…-x64.exe` | Windows 10 version 1809 or later, x64 |
| Linux (beta) | Signed `Runner_…_linux-….deb` or `.tar.gz` | glibc 2.35 or later (Ubuntu 22.04 or later), kernel 5.3 or later, ARM64 or x86_64 |

Intel Macs and Windows ARM64 are not supported. Linux support is in beta: it has been tried on an Ubuntu 26.04 ARM64 virtual machine, while native hardware, x86_64 and other distributions are not yet verified. On Debian or Ubuntu, install the package with `sudo apt install ./Runner_<version>_linux-<arm64|x64>.deb`; on other distributions, extract the tarball and run its `install.sh`. See [Linux build/install instructions](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md) on `main` for prerequisites, local packages and current validation gaps.

On macOS, open the disk image, drag Runner into Applications, and launch it. On Windows, run the installer and launch Runner from the Start menu. A fresh Windows release may show a SmartScreen warning; after verifying that you downloaded the official installer, **More info → Run anyway** continues installation.

On Linux, local package installation registers Runner's application-menu entry and icon. Launch Runner from that entry; running windows use the same desktop identity so the dock can display the Runner name and icon.

## Set up an agent

1. Install an agent CLI using its provider's instructions. [Supported agents](../reference/supported-agents.md) lists the runtimes and setup links. The default pair-coding crew uses Codex for both roles.
2. Open your usual terminal and run the agent. Complete its sign-in or provider configuration and confirm it can answer a prompt.
3. Open **Settings → Agents** in Runner. Confirm the agent is enabled and that Runner shows its executable and version. Use **Refresh** after installing a CLI; use **Browse** to select its executable if it is outside `PATH`.

On Windows, agents run natively without WSL. Claude Code and pi's bash tool need Git for Windows; npm-installed CLIs also need Node.js. PowerShell 7 is optional.

Runner does not install the agent CLIs or supply subscriptions. Where supported, **Settings → Agents** offers **Update** when a newer CLI version is available. The update runs the agent's own updater in a terminal.

The dot on the Usage popover's **Agent settings** gear indicates an agent CLI update. Opening Usage or **Settings → Agents** refreshes the installed CLI versions, so the dot clears once every enabled agent is up to date. Updating Runner does not update these separately installed CLIs.

## Make the `runner` command available

Open **Settings → General → Command line**. This command is optional for a first chat, but useful when working from a shell or asking another agent to operate Runner.

- **macOS:** Runner automatically installs a link in `~/.local/bin`, or a writable `/usr/local/bin`, when the directory is already on the login `PATH`. If it is not installed, click **Install**. The settings page shows the target and whether an administrator password is needed.
- **Linux local packages:** The installer provides `runner`, `runnerd` and `runner-app` before the GUI is opened. Add `~/.local/bin` to PATH for a tarball install; system packages use `/usr/bin`. Settings recognizes these as package-managed commands; remove them with the package installer. The optional systemd user service is disabled until you enable it.
- **Windows:** Runner adds its command directory to your user `PATH`. Open a new terminal so it picks up the change.

Check the command in that terminal:

```sh
runner --version
runner help
```

The **Runner skill for agents** switch in the same settings section installs a small guide for detected agents. It points them to `runner help agents`, which matches the installed CLI version.

## Updates

Runner updates in place and preserves settings, chats and missions. When an update is available, an icon appears beside the Runner name in the sidebar. Click it to download the update, and a ring on the icon shows the progress. When the update is ready, the icon opens a small popover with **Restart to update**, **What's new** and **Later**; if you started the download, the popover opens by itself and keeps keyboard focus where it was. While the update downloads, verifies or installs, clicking the icon does nothing and its tooltip shows the progress. If the download failed or the update is manual-only, the icon opens **Settings → Updates**. That page shows the same state at every stage and has **Cancel**, **Retry**, **Skip this version** and **View downloads**. In the command palette, **Show Runner update** opens the popover when the update is ready and Settings → Updates otherwise; it doesn't start a download. **Runner → Check for Updates** opens Settings → Updates and starts a check.

On macOS, Runner checks in the background and nothing downloads until you click the icon or **Download** in Settings → Updates. On Windows and Linux, **Automatically download updates** is on by default; turn it off to be notified only. **Skip this version** in Settings → Updates hides the icon for that version until your next manual check.

**Restart to update** closes Runner and reopens it on the new version with no installer window. It restarts running agents so they can resume their saved conversations, and the popover and Settings → Updates show how many agents are working before you choose. **Later** only closes the popover, and the icon stays until the update is installed. On macOS, quitting with an update ready may also install it.

Linux builds use production or nightly package identities, with separate stable and nightly feeds. On Linux, **Settings → Updates** downloads and verifies the package for your architecture and installation format. **Restart to update** replaces a managed user tar installation atomically, or requests system authorization to install a deb with APT. System installation requires polkit (`pkexec`). If an external installer or APT has replaced the executable while Runner is open, restart Runner before checking for or installing another update; the old process shows a restart error and stays open. Cancelling authorization or an installation failure leaves Runner open and shows an error; **Settings → About** lists the installer log. User data is retained. Historical unsigned releases offer the download page instead of automatic installation. Over SSH, start `runnerd` explicitly with `systemctl --user start runnerd` before using the CLI. [Linux service lifetime and WSL setup](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md#optional-systemd-user-service) explain persistence and shutdown.

Preview Nightly builds are test builds of a feature still in development on its own branch. They are published for macOS, Windows and Linux to a separate [`preview-nightly` prerelease](https://github.com/yicheng47/runner/releases/tag/preview-nightly), display **Preview Nightly** with their short commit, and update only from that release. Every channel uses the same app and data, so settings, chats and missions carry over when you switch. To leave Preview Nightly, install a nightly or stable build by hand; on Linux, APT treats installing a nightly deb of the same release over a preview deb as a downgrade.

Next: [start your first chat](first-chat.md).
