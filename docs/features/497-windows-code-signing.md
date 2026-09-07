# Windows code signing

Tracking issue: [#497](https://github.com/yicheng47/runner/issues/497). Status: planned. Priority P1.

## Motivation

Every Windows download of Runner 0.8.0 and 0.8.1 is an unsigned Inno Setup installer. Windows SmartScreen stops it with **Windows protected your PC**, and the user has to find **More info → Run anyway** before the installer will run. The README, the GitHub release notes, and `script/windows/release-notes.md` all tell people to click through that wall. That is acceptable for a nightly tested on one PC and not for a stable release: it reads as untrustworthy, some managed machines have the override disabled, and [#493](https://github.com/yicheng47/runner/issues/493) cannot verify a downloaded installer without a signature to check. Stage 4b of the [archived Windows port plan](../impls/archive/windows-nightly/plan.md#phase-4--windows-installer-and-upgrades-remaining) deferred signing; this spec is that stage.

## Scope

- **What gets signed.** `Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` after the release build and before packaging; then the installer and the uninstaller. Inno Setup signs both when `[Setup]` carries `SignTool=` and `SignedUninstaller=yes`, so the uninstaller needs no separate step. SHA-256 digests with an RFC 3161 timestamp so signatures stay valid after the certificate expires.
- **Where it runs.** Both Windows packaging paths: the `build-windows` job in `release.yml` and the Windows job in `nightly.yml`, through `script/bundle-windows.ps1`. Nightlies are signed with the same certificate so `nightly-win` testers stop hitting SmartScreen and #493 can be exercised on nightlies. Local builds stay unsigned; the script signs only when the provider's credentials are present, and `release.yml` requires them the way it already requires the Apple and Sparkle keys.
- **Verification in CI.** `script/windows/test-installer.ps1` checks that all five binaries carry a valid, timestamped Authenticode signature whenever signing is enabled. A production release with an unsigned or invalid binary fails before upload.
- **Publisher identity.** The certificate subject is what Windows shows in SmartScreen and the Properties → Digital Signatures tab. `AppPublisher` in `script/windows/runner.iss` is `wyc studios` today; align it with the name the provider validates.
- **Wording.** Replace the unsigned / Run anyway text in `README.md`, `docs/arch/windows.md`, `script/windows/release-notes.md`, and the release notes string in `release.yml`.
- **Out.** Microsoft Store submission, macOS signing (Developer ID and notarization already ship), the updater's own download verification (#493 consumes this), Windows ARM64.

## Provider decision

Since 2023-06-01 the CA/Browser Forum requires code-signing private keys on certified hardware, so every provider delivers either a USB token or a cloud HSM. GitHub-hosted runners can only use the cloud form. Candidates, with what was already found:

1. **SignPath Foundation OSS program.** Free code signing for open-source projects; the certificate stays with SignPath and signing runs from GitHub Actions through their action, with the project name on the certificate. Runner is GPL-3.0 with public CI, so it should be eligible; approval is a review, not instant, and the shown publisher is SignPath's, not wyc studios. Try this first.
2. **Azure Artifact Signing.** About US$10 a month, short-lived Microsoft-issued certificates, the simplest GitHub Actions integration, and Microsoft documents a SmartScreen benefit. Identity validation requires an eligible region: as checked on 2026-09-06, the public-trust regions do not include mainland China, so this works only through an eligible entity.
3. **Commercial OV certificate with cloud signing** (SSL.com eSigner, DigiCert KeyLocker, Certum SimplySign, GlobalSign). A few hundred US dollars a year plus HSM fees, organization or individual validation, no Azure-style region limit. SmartScreen reputation accrues with download volume, so the interstitial may persist for a while after the first signed release. EV has historically bought immediate reputation; confirm that still holds before paying the premium.

Certum's open-source certificate is cheap but ships on a physical card, which rules out hosted runners without their cloud service. Whatever is chosen, credentials live only in GitHub Actions secrets and the secret names are recorded here.

## Implementation Phases

1. Choose the provider and validate the publisher identity; record the decision, the certificate subject, and the CI secret names in this spec.
2. Wire signing into `bundle-windows.ps1` and both workflows behind the credential check; add the signature verification to `test-installer.ps1`; cut a signed nightly and verify it on `nightly-win`.
3. Ship a signed stable release; update the docs and notes; check SmartScreen on a fresh PC and record the result.

## Verification

- `Get-AuthenticodeSignature` reports `Valid` with a timestamp on all three executables, the installer, and `unins000.exe` from a published release; the Properties → Digital Signatures tab names the publisher.
- A fresh Windows 11 PC downloads the stable installer from GitHub in Edge and runs it without the **Windows protected your PC** interstitial, or, for an OV certificate before reputation accrues, with the publisher named instead of "Unknown publisher".
- Nightly and stable installers are both signed; a release with an unsigned binary fails the workflow before upload.
- Existing installer tests, Windows CI, and macOS release checks pass; macOS signing and notarization are untouched.
