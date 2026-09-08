# One nightly release for both platforms

Tracking issue: [#504](https://github.com/yicheng47/runner/issues/504). Status: planned, 2026-09-08. Priority P2. Bundled with [#505](./505-macos-nightly-replaces-runner.md) in one crew mission; brief: [504 — Nightly channel unification](../impls/504-nightly-channel-unification.md).

## Motivation

[#502](./502-unified-nightly.md) made one dispatch build both platforms but kept two rolling prereleases, `nightly` for macOS and `nightly-win` for Windows, because every installed Windows nightly reads `nightly-win` from an address baked at build time. Production already ships both platforms on one `vX.Y.Z` release. Two nightly addresses mean two pages, two sets of notes, two publish and prune paths, and a skill that has to explain both.

The updaters do not conflict on one release. Sparkle reads only `appcast.xml`. The Windows updater filters assets by the `Runner-Setup-…-x64.exe` name and ignores the DMG, which `production_release_ignores_macos_assets_and_requires_a_windows_installer` already pins.

## Behavior

### One release

`nightly` is the only rolling prerelease. A `both` cut leaves it holding `Runner-Nightly-<sha>.<stamp>-arm64.dmg`, `appcast.xml`, `Runner-Setup-nightly.<sha>.<stamp>-x64.exe`, and its `.sig`. Single-platform cuts replace only that platform's assets. Upload order within a cut is unchanged: DMG before appcast, installer before signature.

Retention prunes each platform independently to its newest ten by stamp on the same release: DMGs by the existing DMG pattern, installers by the existing installer pattern with paired signature deletion. The appcast's DMG is never pruned. The public-download check reads one release and requires every expected asset for the selected platforms.

One release-notes file covers both platforms. It replaces the inline macOS notes string and `script/windows/release-notes.md`, keeping the Windows install, update, and SmartScreen guidance.

### Windows reads `nightly`

The nightly channel's addresses in `release_urls` (`crates/runner-app/src/updater/windows.rs`) move from `releases/tags/nightly-win` to `releases/tags/nightly`, and the Inno Setup `UpdatesUrl` default plus `test-installer.ps1`'s expected value follow. Production addresses are untouched.

### Transition

Installed Windows nightlies still read `nightly-win`, and that address is compiled into the binary, so they cannot be redirected. Jason installs the first `nightly` build on the PC by hand, the same one-time hop the Mac needs for [#505](./505-macos-nightly-replaces-runner.md). Once the PC is on a build that reads `nightly`, the `nightly-win` release and tag are deleted by hand. Any other install still reading `nightly-win` sees a failed update check after that and reinstalls from `nightly`, the existing rule for dormant installs.

## Non-goals

- Deleting the old `nightly-win` release or tag through the workflow. They are deleted by hand after the PC has moved.
- A transition mirror to `nightly-win`. Decided against on 2026-09-08: one hand install beats a workflow step and its later removal.
- Any change to `release.yml`, production addresses, or the production Sparkle feed.
- Windows Authenticode ([#497](./497-windows-code-signing.md)).

## Verification

### Before handoff

- [ ] `make test-nightly` covers a `both` cut leaving four assets on `nightly`, single-platform cuts touching only their assets, per-platform retention on one release with paired signatures and appcast protection, and upload or download failure reported as incomplete without pruning. Nothing in the workflow references `nightly-win`.
- [ ] Windows updater tests cover the nightly channel addresses pointing at `nightly` and a release holding both platforms' assets still selecting the newest installer with its signature.
- [ ] Reviewer reports no remaining must-fix findings on the working-tree diff.

### After landing and an authorized cut

- [x] A `both` cut leaves `nightly` with the DMG, appcast, installer, and signature from one sha and stamp, all anonymously downloadable; `nightly-win` is untouched. Verified on run 34187805571 from the PR branch, 2026-09-08.
- [ ] `-f platform=windows` replaces only the installer and signature on `nightly`; `-f platform=macos` replaces only the DMG and appcast.
- [ ] The hand-installed build on the PC checks `nightly` and updates from it in-app on the next cut; `nightly-win` is then deleted.
- [x] `releases/latest` still resolves to the current production release. Verified `v0.8.2` after run 34187805571.
