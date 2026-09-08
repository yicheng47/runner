Rolling nightly builds for macOS Apple Silicon and Windows x64, identified by commit and UTC build stamp. Both platforms share this public prerelease; a single-platform cut leaves the other platform's files in place. Nightly installs follow this channel, while production installs use separate stable feeds. Install a stable build by hand to switch back to production.

## macOS

Download the newest **Runner-Nightly-…-arm64.dmg** below and drag **Runner.app** into Applications, replacing Runner. The app checks this release's signed Sparkle appcast for nightly updates. The DMG is signed and notarized.

If you still have **Runner Nightly.app**, install this build over **Runner.app** by hand, then delete **Runner Nightly.app**. Its old bundle identifier prevents Sparkle from installing the new bundle in place.

## Windows

Download the newest **Runner-Setup-…-x64.exe** installer below. It installs Runner for your Windows account without administrator access and adds a Start Menu shortcut. Windows 10 version 1809 or later is required.

Runner checks for newer builds at startup and every six hours, downloads them in the background by default, and verifies their minisign signatures. Open the update icon beside Settings or **Settings → Updates → Update**, then choose **Install and restart**. Runner closes, the installer runs, and Runner reopens with settings, chats, and missions retained. Turn off **Automatically download updates** to download only when you choose. Uninstalling removes the application and shortcut but keeps your data for a later reinstall.

The installer, app, and CLI sidecars are **Authenticode-signed**; Windows names the publisher as **Open Source Developer Yicheng Wang**. SmartScreen may still warn while the certificate builds reputation; where your security settings allow it, choose **More info → Run anyway**. In-app updates verify the separate minisign signature.

Older Windows nightlies read the retired `nightly-win` release. Install this build by hand once to switch to `nightly`; subsequent builds update in-app from this release.

Install the agent CLIs you want to use: Claude Code and/or Codex. Claude Code also needs Git for Windows for Git Bash. npm-based CLI installations need Node.js; native CLI installations do not. PowerShell 7 is optional.
