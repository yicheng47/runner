Download the newest **Runner-Setup-…-x64.exe** installer below. It installs Runner for your Windows account without administrator access and adds a Start Menu shortcut. Windows 10 version 1809 or later is required.

Runner checks for newer builds at startup and every six hours, downloads them in the background by default, and verifies their minisign signatures. Open the update icon beside Settings or **Settings → Updates → Update**, then choose **Install and restart**. Runner closes, the installer runs, and Runner reopens with settings, chats, and missions retained. Turn off **Automatically download updates** to download only when you choose. Uninstalling removes the application and shortcut but keeps your data for a later reinstall.

These testing builds are not yet **Authenticode-signed**. Windows may show a SmartScreen warning during the initial download and installation; where your security settings allow it, choose **More info → Run anyway**. Authenticode signing is planned separately from the minisign verification used for updates.

Install the agent CLIs you want to use: Claude Code and/or Codex. Claude Code also needs Git for Windows for Git Bash. npm-based CLI installations need Node.js; native CLI installations do not. PowerShell 7 is optional.
