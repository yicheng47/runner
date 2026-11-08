Rolling preview builds of an in-development umbrella branch for macOS Apple Silicon and Intel, Windows x64 and Linux ARM64/x86_64: everything on `main` plus the umbrella's unfinished feature. Each push to the umbrella cuts a new build, identified by commit and UTC build stamp and displayed as **Preview Nightly** with its short commit, for example `Preview Nightly (abc1234)`. Preview installs update from this release only; the regular nightly and production builds keep their own feeds. Every channel uses the same app and data, so settings, chats and missions carry over when you switch. Install a nightly or stable build by hand to switch back.

## macOS

Download the newest **Runner-Preview-Nightly-…-arm64.dmg** for an Apple Silicon Mac or **Runner-Preview-Nightly-…-x64.dmg** for an Intel Mac below and drag **Runner.app** into Applications, replacing Runner. The app checks this release's signed Sparkle appcast for its architecture for updates. The DMG is signed and notarized.

## Windows

Download the newest **Runner-Setup-preview-nightly…-x64.exe** installer below and run it over your current Runner. It installs for your Windows account without administrator access. Runner checks this release for newer preview builds at startup and every six hours, verifies their minisign signatures, and installs them from **Settings → Updates → Update**, the same way as the regular nightly.

## Linux

Choose `Runner_preview-nightly.<sha>.<stamp>_linux-<arm64|x64>.tar.gz` or `.deb` for your architecture; each package has a matching minisign signature. Settings → Updates installs newer preview builds the same way as the regular nightly. A preview deb sorts above a nightly deb of the same release, so APT treats moving back to the regular nightly as a downgrade. See the repository's [Linux requirements and installation guide](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md).
