# One nightly for both platforms

Tracking issue: [#502](https://github.com/yicheng47/runner/issues/502). Status: planned. Priority P1.

## Motivation

`nightly.yml` builds one platform per dispatch. Its required `platform` input picks macOS or Windows, each run computes its own build stamp, each waits on CI separately, and the two runs live in separate concurrency groups. Since [#493](./archive/493-windows-auto-update.md) shipped in 0.8.2 the Windows nightly updates itself in-app from the public `nightly-win` prerelease, while the macOS nightly is still the hidden draft decided on 2026-08-24: no Sparkle feed, installs by `gh release download nightly`. The two channels also disagree on versioning. The macOS path refuses to cut unless the crate version ends in `-nightly`, so a nightly can never be mistaken for a patch release, while Windows nightlies have been cut from the bare version (`0.8.1.20260907.1531`).

Jason runs the nightly channel on both machines. One dispatch should produce both platforms from one stamp, both should self-update, and both should carry the same version string.

## Behavior

### One dispatch, one stamp

`gh workflow run nightly.yml --ref main` builds both platforms. The `platform` input stays for a single-platform cut and gains a default:

| `platform` | Builds |
|---|---|
| `both` (default) | macOS and Windows, in parallel, from one stamp |
| `macos` | macOS only |
| `windows` | Windows only |

The workflow takes the shape `release.yml` already has: a `prepare` job computes the stamp, the sha, and the version once and refuses to run unless the crate version ends in `-nightly`; `build-macos` and `build-windows` run in parallel, each gated on the input; one `publish` job waits for CI on the sha once, uploads, prunes, and verifies. One concurrency group, `nightly`, with `cancel-in-progress`, so a new dispatch cancels a whole in-flight nightly rather than one half of it.

### Two rolling releases

Assets keep going to two releases, both public prereleases:

- `nightly` receives `Runner-Nightly-<version>-arm64.dmg` and a signed `appcast.xml`.
- `nightly-win` receives `Runner-Setup-<version>-x64.exe` and its `.sig`.

They are not merged. Every installed Windows nightly reads `nightly-win` by a URL baked at build time (`crates/runner-app/src/updater/windows.rs`), so a single release would strand the build on Jason's PC until a manual reinstall. Both prune to the newest ten by stamp, deleting installer and signature together on Windows.

### macOS nightlies self-update

The `nightly` release stops being a draft. The macOS job generates the appcast with `generate_appcast` from the same Sparkle key `release.yml` uses, with `--download-url-prefix https://github.com/yicheng47/runner/releases/download/nightly/`, and `publish` uploads it beside the DMG. The nightly bundle already bakes `SUFeedURL` `releases/download/nightly/appcast.xml` (`script/bundle-mac`), which starts resolving; nothing changes in the app. The "Verify the nightly release stays hidden" step is replaced by the check the Windows job already runs: prerelease true, draft false, the new asset downloads anonymously.

This flips the 2026-08-24 policy that general users must not find nightlies. Windows already flipped it on 2026-09-06 so a friend could download, and the safety argument is unchanged: GitHub's `releases/latest` never resolves to a prerelease, the nightly bundle has its own bundle id (`com.wycstudios.runner.nightly`) and feed URL, and production Sparkle installs read only `releases/latest/download/appcast.xml`. A production user can find the nightly page by browsing; they cannot be updated onto it.

### One version convention

The macOS rule wins. After each production release the crates are bumped to the next `-nightly` version, the existing post-release chore; the first one is `0.8.3-nightly` after 0.8.2. Both platforms then label nightlies `<version>-nightly.<stamp>`: `bundle-mac --channel nightly` already does, and `bundle-windows.ps1` already uses `$version.$Stamp` on the nightly channel. The Windows updater's installer-name parsing (`installer_version`) splits the stamp off the end and handles a `-nightly` base.

## Non-goals

- Merging `nightly` and `nightly-win` into one release. Possible later with a one-cycle overlap; not worth stranding an installed build for.
- Changing the production `release.yml` path, tags, or the Sparkle production feed.
- Channel switching in the app. Nightly and stable stay separate installs.
- Windows Authenticode ([#497](./497-windows-code-signing.md)).

## Implementation Phases

1. **Version chore and workflow restructure.** Bump the three crates to `0.8.3-nightly` (the `/nightly bump` path). Restructure `nightly.yml` into `prepare`, the two parallel build jobs gated on `platform` with `both` as the default, and one `publish`; single concurrency group; CI gate once. Verify with one dispatch of each input value.
2. **macOS self-update.** Appcast generation in the macOS job, upload to `nightly`, flip the release to a public prerelease, replace the hidden-release verification with the public-prerelease check. Confirm on the Mac that an installed nightly offers the next one through Sparkle.
3. **Docs and skill.** `/nightly` collapses to one `run` with an optional platform and one `check`; `docs/arch/arch.md` §14 and `docs/arch/windows.md` describe the shared workflow, the public macOS nightly, and the version rule; the nightly release-notes strings stop saying "hidden".

## Verification

- [ ] `gh workflow run nightly.yml --ref main` with no input builds both platforms with one stamp; `-f platform=macos` and `-f platform=windows` each build only that platform.
- [ ] A second dispatch while one is running cancels the whole first run.
- [ ] `nightly` is a public prerelease with a DMG and `appcast.xml` that download anonymously; `nightly-win` unchanged in shape.
- [ ] An installed macOS nightly finds the next nightly through Sparkle and installs it; an installed Windows nightly does the same through the in-app updater.
- [ ] Both DMG and installer names read `0.8.3-nightly.<stamp>` with the same stamp.
- [ ] `releases/latest` still resolves to `v0.8.2`; a production install's Sparkle check offers nothing.
- [ ] The workflow refuses to run on a bare crate version.
