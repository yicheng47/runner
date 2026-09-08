# One nightly for both platforms

Tracking issue: [#502](https://github.com/yicheng47/runner/issues/502). Status: commit-only nightly identity implemented, locally validated, and reviewed clean, 2026-09-08; post-landing live verification pending. Priority P1. Crew brief: [502 — Unified nightly implementation](../impls/502-unified-nightly.md).

## Motivation

Before #502, `nightly.yml` builds one platform per dispatch, each run computes its own stamp and waits on CI separately, and the two runs have separate concurrency groups. Windows already self-updates from `nightly-win`; macOS still publishes a hidden draft with no working Sparkle feed.

Jason runs the nightly channel on both machines. One dispatch should produce both platforms from one commit and stamp, and both should self-update. Nightly is a rolling development channel identified by its commit, independent of an official `X.Y.Z` release. The initial proposal tied nightlies to `0.8.3-nightly`; Jason explicitly replaced that rule on 2026-09-08: version tags belong to official releases only.

## Behavior

### One dispatch, one stamp

`gh workflow run nightly.yml --ref main` builds both platforms. The `platform` input stays for a single-platform cut and gains a default:

| `platform` | Builds |
|---|---|
| `both` (default) | macOS and Windows, in parallel, from one stamp |
| `macos` | macOS only |
| `windows` | Windows only |

The workflow takes the shape `release.yml` already has: a `prepare` job computes the stamp, the sha, and the commit-based nightly identity once, without reading or gating on the crate version; `build-macos` and `build-windows` run in parallel, each gated on the input; one `publish` job waits for CI on the sha once, uploads, prunes, and verifies. One concurrency group, `nightly`, with `cancel-in-progress`, so a new dispatch cancels a whole in-flight nightly rather than one half of it.

The source is the dispatch commit throughout, including both checkouts, artifact identity, and the CI lookup. Builds pass artifacts from this workflow run to `publish`; they do not publish independently. For `both`, both builds must succeed before publication. For a single-platform dispatch, the unselected build is skipped and must not prevent `publish` from running; failed or cancelled selected builds do prevent publication. A single-platform cut updates only that platform's release and feed. Missing or failed CI prevents publication.

### Two rolling releases

Assets keep going to two releases, both public prereleases:

- `nightly` receives `Runner-Nightly-<sha>.<stamp>-arm64.dmg` and a signed `appcast.xml`.
- `nightly-win` receives `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` and its `.sig`.

They are not merged. Every installed Windows nightly reads `nightly-win` by a URL baked at build time (`crates/runner-app/src/updater/windows.rs`), so a single release would strand the build on Jason's PC until a manual reinstall. Both prune to the newest ten by stamp, deleting installer and signature together on Windows.

Upload the DMG before replacing the appcast, and the installer before its matching signature. Verify the expected files and public downloads before pruning older builds; the appcast must never reference an asset that pruning removes. Existing releases are converted in place, and missing releases are created as public prereleases without becoming GitHub's latest stable release. Publishing across two releases is not transactional: cancellation or an upload failure can leave some files uploaded, and that run must be reported incomplete rather than claiming both channels are ready.

### macOS nightlies self-update

The `nightly` release stops being a draft. The macOS job generates the appcast with `generate_appcast` from the same Sparkle key `release.yml` uses, with `--download-url-prefix https://github.com/yicheng47/runner/releases/download/nightly/`, and `publish` uploads it beside the DMG. The nightly bundle already bakes `SUFeedURL` `releases/download/nightly/appcast.xml` (`script/bundle-mac`), which starts resolving; nothing changes in the app. The "Verify the nightly release stays hidden" step is replaced by the check the Windows job already runs: prerelease true, draft false, the new asset downloads anonymously.

This flips the 2026-08-24 policy that general users must not find nightlies. Windows already flipped it on 2026-09-06 so a friend could download, and the safety argument is unchanged: GitHub's `releases/latest` never resolves to a prerelease, the nightly bundle has its own bundle id (`com.wycstudios.runner.nightly`) and feed URL, and production Sparkle installs read only `releases/latest/download/appcast.xml`. A production user can find the nightly page by browsing; they cannot be updated onto it.

### Nightly identity and official versions

Nightly is its own rolling development channel, shown in the app and update offers as `Nightly (<short-sha>)`. It is not a prerelease of a particular official version. `vX.Y.Z` tags are for official releases; the existing `nightly` and `nightly-win` tags are rolling channel addresses needed by installed updaters.

`prepare` computes one full source SHA, seven-character display SHA, and UTC stamp. macOS emits `Runner-Nightly-<sha>.<stamp>-arm64.dmg`; Windows emits `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` and `.sig`. For example, the Windows machine identity is `nightly.abc1234.20260908.0100`; the app displays `Nightly (abc1234)` on both platforms. The stamp orders builds and retention; the commit identifies their source.

`CFBundleVersion` remains the UTC stamp, while the macOS short version is the seven-character commit. Sparkle already includes the app name `Runner Nightly` in its native alert; using only the commit avoids a repeated “Nightly”. Runner’s own app and update displays format that identity as `Nightly (<sha>)`. The existing Windows installer parser already supports a `nightly.<sha>` base and compares the trailing stamp, so both older bare-version and `X.Y.Z-nightly` installers remain compatible. Windows update offers format the new machine identity as `Nightly (<sha>)`.

There is no nightly crate-version bump or bare-version rejection. The three lockstep crates and lockfile retain the existing `0.8.2` package version as internal Rust/Windows numeric metadata until an official release changes it. Leave the independently versioned `runner-core` and CLI alone. Official packaging and `release.yml` retain their existing version/tag contract.

## Non-goals

- Merging `nightly` and `nightly-win` into one release. Possible later with a one-cycle overlap; not worth stranding an installed build for.
- Changing the production `release.yml` path, tags, or the Sparkle production feed.
- Channel switching in the app. Nightly and stable stay separate installs.
- Windows Authenticode ([#497](./497-windows-code-signing.md)).

## Implementation Phases

1. **Shared identity and workflow restructure.** Restructure `nightly.yml` into `prepare`, the two parallel build jobs gated on `platform` with `both` as the default, and one `publish`; single concurrency group; CI gate once. Nightly identity comes from the commit and stamp, independently of crate versions. Validate the workflow locally, including skipped and failed build outcomes. The crew leaves the changes uncommitted.
2. **Packaging and self-update.** Give both nightly packagers the commit-based identity and app display, retaining production packaging and updater compatibility. Generate the macOS appcast, publish the existing public prereleases, and verify the signed feed and downloads. Reuse the pinned Sparkle tools, checksum, signing key, and Apple Silicon requirement from `release.yml`.
3. **Docs and skill.** `/nightly` exposes one `run` and one `check` with an optional platform; remove its nightly `bump` path. Update architecture and current instructions to describe the rolling development channel, shared workflow, public macOS nightly, and official-only version tags.

This mission implements and reviews the working-tree change. Commits, pushes, PRs, workflow dispatches, release mutations, and app installation are separate actions requiring Jason's subsequent instruction. Live verification below happens after landing; it is not a prerequisite for the crew to deliver its reviewed implementation. The old nightly skill's dispatch/push permissions do not authorize those actions in this implementation mission.

## Verification

### Before handoff

- [x] Workflow validation covers the default, both single-platform inputs, failed/cancelled selected builds, and identity independent of the crate version.
- [x] The three package versions and lockfile remain at the baseline; macOS bundle and app tests pass, including commit-only nightly display and the separate production feed.
- [x] Appcast URLs point only at `nightly`, with a signed enclosure for the expected DMG and the existing `arm64` requirement; Windows installer/signature naming and stamp comparison support the new version convention.
- [x] Reviewer reports no remaining must-fix findings on the working-tree diff. Local checks and live checks still pending are reported separately.

### After landing and an authorized cut

- [ ] `gh workflow run nightly.yml --ref main` with no input builds both platforms with one stamp; `-f platform=macos` and `-f platform=windows` each build only that platform.
- [ ] A second dispatch while one is running cancels the whole first run.
- [ ] `nightly` is a public prerelease with a DMG and `appcast.xml` that download anonymously; `nightly-win` unchanged in shape.
- [ ] An installed macOS nightly finds the next nightly through Sparkle and installs it; an installed Windows nightly does the same through the in-app updater.
- [ ] Both DMG and installer names contain the same short commit and stamp, and both apps display `Nightly (<sha>)` without an official version.
- [ ] `releases/latest` still resolves to `v0.8.2`; a production install's Sparkle check offers nothing.
- [ ] The workflow builds nightlies from the existing bare crate version without a version bump.
