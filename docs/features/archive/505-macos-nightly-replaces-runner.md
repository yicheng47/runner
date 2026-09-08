# macOS nightly replaces Runner

Tracking issue: [#505](https://github.com/yicheng47/runner/issues/505). Status: shipped 2026-09-08 in PR [#506](https://github.com/yicheng47/runner/pull/506); archived 2026-09-08. Priority P2. Bundled with [#504](./504-single-nightly-release.md) in one crew mission; brief: [504 — Nightly channel unification](../../impls/archive/504-nightly-channel-unification.md).

## Motivation

On Windows a nightly installer replaces the installed Runner and switches that install to the nightly channel; installing a production build switches it back. On macOS the nightly is a second bundle, `com.wycstudios.runner.nightly` named `Runner Nightly`, that sits beside the production app. The two Mac bundles already share one data directory (`arch.md` §10.2) and cannot run at the same time, so the separate identity only adds a second app to manage and a second Dock icon.

The separate bundle was chosen on 2026-08-24 to keep nightlies away from production installs. That isolation is carried by the feed address, not the bundle id: production Sparkle installs read only `releases/latest/download/appcast.xml`, which never lists a nightly, and `releases/latest` never resolves to a prerelease.

## Behavior

### Same app, different feed

`script/bundle-mac --channel nightly` produces `com.wycstudios.runner`, `CFBundleName` `Runner`, and a `Runner.app` bundle, the same as production. Only the baked `SUFeedURL` differs: `releases/download/nightly/appcast.xml` on nightly, `releases/latest/download/appcast.xml` on production. `CFBundleVersion` stays the UTC stamp and `CFBundleShortVersionString` stays the short sha, so the app and Sparkle's alert still show which channel and commit are installed. The DMG keeps its `Runner-Nightly-<sha>.<stamp>-arm64.dmg` name so the file is recognizable; its volume name follows the app name.

Installing a nightly DMG over `Runner.app` switches that install to the nightly channel. It then follows the nightly feed until a stable DMG is installed by hand, which switches it back. That is the Windows rule.

### Transition

Sparkle refuses an update whose bundle identifier differs from the host's, so the installed `Runner Nightly.app` does not hop by itself; its next check reports an installer error. The first unified nightly is a drag-to-Applications install over `Runner.app`, and `Runner Nightly.app` is deleted. One-time, on the Mac only.

## Non-goals

- Channel switching inside the app.
- Keeping a side-by-side stable fallback. Rolling back is a stable DMG install.
- Any change to the production bundle, `release.yml`, or the production feed.

## Verification

### Before handoff

- [x] `bundle_mac` tests: the nightly plist carries the production bundle id and name, the nightly feed, the stamp, and the short sha; the production plist is unchanged and still carries the stable feed.
- [x] Reviewer reports no remaining must-fix findings on the working-tree diff.

### After landing and an authorized cut

- [x] The cut's DMG opens as a `Runner` volume containing `Runner.app`, installs over the production `Runner.app`, and the app shows `Nightly (<sha>)`. `2bf743d` on 2026-09-08; `Runner Nightly.app` deleted.
- [x] That install's Sparkle check offers the next nightly and installs it in place. `d3838a5` on 2026-09-08, missions intact.
- [x] A production install's Sparkle check still offers nothing. The production appcast still serves 0.8.2 and `releases/latest` is unchanged; no production install remains on the Mac for a live check.
