# Windows code signing

Tracking issue: [#497](https://github.com/yicheng47/runner/issues/497). Status: **shipped 2026-09-08** as commit `100f1c3` on `main`; 0.8.4 was the first Certum-signed stable release. Priority P1.

## Motivation

Every Windows download of Runner 0.8.0 and 0.8.1 is an unsigned Inno Setup installer. Windows SmartScreen stops it with **Windows protected your PC**, and the user has to find **More info → Run anyway** before the installer will run. The README, the GitHub release notes, and `script/nightly-release-notes.md` all tell people to click through that wall. That is acceptable for a nightly tested on one PC and not for a stable release: it reads as untrustworthy, some managed machines have the override disabled, and [#493](https://github.com/yicheng47/runner/issues/493) cannot verify a downloaded installer without a signature to check. Stage 4b of the [archived Windows port plan](../impls/archive/windows-nightly/plan.md#phase-4--windows-installer-and-upgrades-remaining) deferred signing; this spec is that stage.

## Scope

- **What gets signed.** `Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` after the release build and before packaging; then the installer and the uninstaller. Inno Setup signs both when `[Setup]` carries `SignTool=` and `SignedUninstaller=yes`, so the uninstaller needs no separate step. SHA-256 digests with an RFC 3161 timestamp so signatures stay valid after the certificate expires.
- **Where it runs.** Both Windows packaging paths: the `build-windows` job in `release.yml` and the Windows job in `nightly.yml`, through `script/bundle-windows.ps1`. Nightlies are signed with the same certificate so `nightly` testers stop hitting SmartScreen and #493 can be exercised on nightlies. Local builds stay unsigned; the script signs only when the certificate thumbprint is present, and both workflows require the credentials the way `release.yml` already requires the Apple and Sparkle keys.
- **Verification in CI.** `script/windows/test-installer.ps1` checks that all five binaries carry a valid, timestamped Authenticode signature from the expected certificate whenever signing is enabled. A release with an unsigned or invalid binary fails before upload.
- **Publisher identity.** The certificate subject is what Windows shows in SmartScreen and the Properties → Digital Signatures tab. `AppPublisher` in `script/windows/runner.iss` stays `wyc studios`: Certum's open-source product fixes the subject to the developer's name, and the installer's publisher field is product branding, not the signing identity.
- **Wording.** Replace the unsigned / Run anyway text in `README.md`, `docs/arch/windows.md`, `script/nightly-release-notes.md`, and the release notes string in `release.yml`.
- **Out.** Microsoft Store submission, macOS signing (Developer ID and notarization already ship), the updater's own download verification (#493 consumes this), Windows ARM64.

## Provider decision

Since 2023-06-01 the CA/Browser Forum requires code-signing private keys on certified hardware, so every provider delivers either a USB token or a cloud HSM. GitHub-hosted runners can only use the cloud form.

**Chosen on 2026-09-08: Certum Open Source Code Signing in the Cloud (SimplySign).** Jason bought and activated it on 2026-09-08; the certificate is issued to `CN=Open Source Developer Yicheng Wang, O=Open Source Developer, L=Shanghai, S=Shanghai, C=CN` by `Certum Code Signing 2021 CA`, valid until 2027-09-08, and must be renewed and its thumbprint secret updated before then. The private key sits in Certum's SimplySign cloud HSM. SimplySign has no headless API: signing needs SimplySign Desktop logged in with the account name and a one-time code, after which the certificate is a virtual smart card for about two hours. `script/windows/simplysign.ps1` automates that login on a GitHub-hosted Windows runner, the approach several open-source projects already ship with. Certum allows 5000 cloud signatures a month; a build uses nine.

Repository secrets: `CERTUM_USERNAME` (the SimplySign login), `CERTUM_OTP_URI` (the full `otpauth://` URI from the SimplySign enrollment QR code, SHA-256, six digits, 30 seconds), and `CERTUM_CERTIFICATE_SHA1` (the certificate thumbprint). The SimplySign Desktop MSI is pinned by version and SHA-256 in `simplysign.ps1`.

Rejected:

1. **SignPath Foundation OSS program.** Free, but the certificate belongs to SignPath, so Windows would name SignPath Foundation as the publisher; every signing request needs a manual approval in their dashboard, which makes unattended nightlies impractical; and the OSS plan signs uploaded artifacts rather than exposing a key to `signtool`, so Inno Setup could not sign the uninstaller. An application was drafted on 2026-09-07 and not pursued.
2. **Azure Artifact Signing.** The simplest integration, but as checked on 2026-09-06 the public-trust regions do not include mainland China.
3. **Commercial OV certificate with cloud signing** (SSL.com eSigner, DigiCert KeyLocker, GlobalSign). A few hundred US dollars a year for the same SmartScreen reputation ramp as the Certum certificate.

## Implementation

- `script/windows/simplysign.ps1` installs SimplySign Desktop, pre-sets its registry so the login dialog opens on launch, derives the one-time code from the secret, types the credentials, and waits for the certificate with a private key to appear in `Cert:\CurrentUser\My`, retrying with a fresh code up to three times.
- `script/windows/signtool.ps1` returns the newest Windows SDK `signtool.exe`.
- `script/bundle-windows.ps1` takes `-SigningThumbprint` (default `CERTUM_CERTIFICATE_SHA1`), signs the three executables with `signtool sign /sha1 <thumbprint> /fd sha256 /tr http://time.certum.pl /td sha256`, and compiles `runner.iss` with `/DSign` plus a `/Sauthenticode=…` sign-tool definition.
- `script/windows/runner.iss` adds `SignTool=authenticode` and `SignedUninstaller=yes` under `#ifdef Sign`.
- `script/windows/test-installer.ps1` compiles its test installers with the same definition in payload mode and asserts a `Valid`, timestamped signature from the expected thumbprint on the installer, the three installed binaries, and `unins000.exe`.
- `release.yml` and `nightly.yml` require the three secrets, run `simplysign.ps1` after the Rust cache step, pass the thumbprint to the build and installer-test steps, and keep the minisign step for in-app updates.

Verified locally on 2026-09-08 on JASONPC with SimplySign Desktop connected: `signtool` signed and timestamped a binary with the full Certum chain, and `test-installer.ps1` in payload mode passed its signature assertions on all five files.

## Implementation Phases

1. Choose the provider and validate the publisher identity; record the decision, the certificate subject, and the CI secret names in this spec. Done 2026-09-08.
2. Wire signing into `bundle-windows.ps1` and both workflows behind the credential check; add the signature verification to `test-installer.ps1`; cut a signed nightly and verify it on `nightly`. Code done 2026-09-08; the first signed nightly is pending.
3. Ship a signed stable release; update the docs and notes; check SmartScreen on a fresh PC and record the result.

## Verification

- `Get-AuthenticodeSignature` reports `Valid` with a timestamp on all three executables, the installer, and `unins000.exe` from a published release; the Properties → Digital Signatures tab names the publisher.
- A fresh Windows 11 PC downloads the stable installer from GitHub in Edge and runs it without the **Windows protected your PC** interstitial, or, for an OV certificate before reputation accrues, with the publisher named instead of "Unknown publisher".
- Nightly and stable installers are both signed; a release with an unsigned binary fails the workflow before upload.
- Existing installer tests, Windows CI, and macOS release checks pass; macOS signing and notarization are untouched.
