# 648 — command install and Runner skill smoke

## Coverage boundary

The automated suite exercises command-target selection, link ownership, foreign entries, shadowing, default and explicit actions, shell-plus-AppleScript quoting, cancellation, the Windows user-PATH transform and value type, default initialization, settings migration, skill-root reconciliation, palette state, the Settings exception copy, the Skills badge, `runner status`, Runner-injected PATH filtering, and the two CLI output fixes. Every filesystem test receives a temporary home and temporary target folders; Windows registry access and macOS escalation are traits backed by fakes. The app's test entry point supplies no home or command-install integration, while the store-level default test receives temporary targets and the isolated registry/escalation implementations. The suite neither reads nor changes the real skill roots, `~/.local/bin`, `/usr/local/bin`, the Windows registry, nor invokes `osascript`.

The macOS UI, password sheet, launch-to-login-PATH interaction, and installed-package paths require the manual checks below. Windows registry and new-shell propagation require the JASONPC check in mission 5.

## Escalated command

For the development sidecar, the composed AppleScript argument is exactly:

```text
do shell script "mkdir -p '/usr/local/bin' && ln -sf '/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner' '/usr/local/bin/runner-dev'" with administrator privileges
```

The explicit action passes that string as the argument after `/usr/bin/osascript -e`. Shell paths are single-quoted first, including the canonical close-quote/escaped-quote/reopen sequence for a path containing a single quote, and the result is then escaped for the AppleScript string. `RUNNER_COMMAND_INSTALL_FORCE_ESCALATED=1` selects this path only in a debug build; release builds compile the environment lookup out.

## Jason's macOS checklist

Run these checks against `make run`. They intentionally change the development command link and development skill folders, so the automated suite does not perform them.

- On the first launch, confirm `~/.local/bin/runner-dev` links to `~/Library/Application Support/com.wycstudios.runner-dev/bin/runner` without a click.
- In a new plain Terminal.app shell, run `runner-dev status`; confirm it reports the development socket and the command as installed.
- Open Settings → General. Under Window, confirm the Command line section shows the installed path, then the Runner skill switch with no normal status line.
- Click Uninstall, restart the app, and confirm the link stays absent. Click Install and confirm the link returns.
- From a mission workspace, open the command palette and exercise both `Uninstall runner command` and `Install runner command` without navigating to Settings.
- Move the owned link aside, create a hand-made file at `~/.local/bin/runner-dev`, and confirm the row warns that it belongs to another program, offers no action, and leaves the file unchanged. Remove the canary and restore the owned state afterward.
- Relaunch with `RUNNER_COMMAND_INSTALL_FORCE_ESCALATED=1 make run`. With the development command absent, click `Install…`; confirm the row says `Waiting for your password…`, the button says `Installing…`, and the password prompt creates `/usr/local/bin/runner-dev`. Repeat after removing the link, cancel the prompt, and confirm the row stays not installed with no error. Install once more, then click Uninstall and confirm a second prompt removes the owned link.
- Switch `Runner skill for agents` off and confirm the three owned `runner-dev` folders are removed. Restart and confirm they stay absent. Switch it on and confirm detected-agent roots return.
- With the skill switch on, delete `~/.claude/skills/runner-dev`, restart, and confirm it returns. Confirm `~/.trae/skills/runner-dev` is installed although TRAE is off in Settings → Agents.
- Confirm the healthy skill row has no status line and Settings → Skills shows `Managed by Runner` on the managed folder's ordinary row.
- Stop a mission and confirm the human `runner-dev mission show <mission>` view prints `STATUS stopped`; confirm `mission stop`, `resume`, and `archive` print the crew name rather than its id.

## JASONPC checklist for mission 5

- From a fresh release install, confirm the sidecar directory appears exactly once in the current user's `Path` and retains the registry value's original type.
- Open a new PowerShell window and confirm `runner status` finds the release command.
- Uninstall from Settings and confirm only Runner's sidecar-directory entry is removed and a new PowerShell window no longer finds `runner`.

## Still unproven by automation

Automation does not prove the native macOS administrator sheet, Finder-launched login-shell timing, Windows registry APIs or `WM_SETTINGCHANGE` delivery, propagation into a newly opened terminal, final packaged sidecar locations, or palette dispatch from a live non-Settings route. The checklist above owns the macOS and palette-dispatch proof; mission 5 on JASONPC owns the Windows proof.
