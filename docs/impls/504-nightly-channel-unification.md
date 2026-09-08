# 504 — Nightly channel unification

Tracking issues: [#504](https://github.com/yicheng47/runner/issues/504) and [#505](https://github.com/yicheng47/runner/issues/505). Specs: [One nightly release for both platforms](../features/504-single-nightly-release.md) and [macOS nightly replaces Runner](../features/505-macos-nightly-replaces-runner.md). Baseline: `main` at `2e09fbb`, Runner 0.8.2, 2026-09-08. Status: planned; crew brief.

## Goal

Make the nightly channel behave the same on both platforms: one `nightly` release carries every nightly asset, and a nightly install is the same app as production with a different baked feed. Deliver a locally validated, reviewed working-tree diff. No UI design is needed: packaging, workflow, updater addresses, and documentation only.

## Checkout and authorization

- Work in the existing `/Users/jason/repos/yicheng47/runner` checkout. The coder creates `feat/504-nightly-channel-unification` from `main` before editing. Do not create another checkout or worktree.
- The uncommitted specs for #504 and #505, this brief, and the index edits in `docs/features/README.md` and `docs/impls/README.md` were prepared for this mission and belong to it. Preserve and include them in review.
- The two existing crew slots are the authorized workers. Do not spawn additional agents, crews, or review fan-outs.
- Implement, validate, and exchange review messages through Runner. The reviewer waits for the coder's handoff, critiques the diff, and does not edit it. Iterate until there are no must-fix findings, then broadcast the final handoff.
- Leave all changes uncommitted. No commit, push, PR, merge, tag, workflow dispatch, release mutation, app launch/restart, or installer execution. The nightly skill is a reference and an edit target, not a request to execute its `run` action.
- Follow `AGENTS.md`; keep background helpers scoped to their foreground command and do not leave polling loops running.

## Scope and implementation order

### 1. macOS bundle identity (#505)

In `script/bundle-mac`, the nightly arm uses `BUNDLE_ID="com.wycstudios.runner"` and `APP_NAME="Runner"`, keeping `SHORT_VERSION="$BUILD_SHA"`, `MARKETING_VERSION="Nightly"`, the nightly `FEED_URL`, and the `Runner-Nightly-$BUILD_SHA.$BUILD_STAMP-arm64.dmg` name. Check the rest of the script for anything keyed on the nightly bundle id or app name (icon handling, zip name, volume name) and let it follow `APP_NAME`. The production arm is unchanged.

Update `crates/runner-app/tests/bundle_mac.rs`: the nightly plist test asserts the production bundle id and name with the nightly feed, stamp, and short sha; keep the production test and the feed-isolation assertion. Rename the test if its name no longer describes it.

### 2. Windows nightly address (#504)

In `crates/runner-app/src/updater/windows.rs`, `release_urls`'s nightly arm returns `releases/tags/nightly` and `releases/tag/nightly`. Update the `UpdatesUrl` default in `script/windows/runner.iss`, the nightly `$updatesUrl` in `script/bundle-windows.ps1`, and the expected value in `script/windows/test-installer.ps1`. Add a Windows updater test that a release holding the DMG, appcast, installer, and signature together selects the newest installer with its signature; `installer_version` and `available_update` do not otherwise change.

### 3. One release in the workflow (#504)

In `.github/workflows/nightly.yml`, `publish` targets `nightly` for both platforms:

- One release-edit-or-create step with combined notes from a single notes file. Move `script/windows/release-notes.md` to a location that names both platforms (for example `script/nightly-release-notes.md`), covering macOS Sparkle updates and the existing Windows install, update, and SmartScreen guidance. The release stays a public prerelease excluded from latest.
- Selected-platform uploads in the existing order: DMG then appcast, installer then signature, all to `nightly`.
- One public verification step reading `nightly` once and requiring every selected platform's assets, the appcast comparison, and the signature comparison.
- Two prune steps on `nightly`, one per platform pattern, each keeping ten by stamp; Windows deletion stays paired with the `.sig`; refuse to prune the current DMG or installer.
- Completed and incomplete summaries name `nightly`. Nothing in the workflow references `nightly-win` any more.

Extend `script/test-nightly.py` rather than rewriting it: four assets on `nightly` after `both`; single-platform cuts touch only their assets; per-platform retention on one release; upload or download failure reported incomplete and never pruning; no `gh` call ever names `nightly-win`. Keep `make test-nightly` as the entry point.

### 4. Documentation and skill

Update `docs/arch/arch.md` §14 (Versions, Nightly channels, Isolation), `docs/arch/windows.md` (Unified nightlies, Update behavior), `.agents/skills/nightly/SKILL.md` (`run` and `check` read one release), and the #497 spec's `nightly-win` reference. State the transition on both platforms: the PC's installed nightly and the Mac's `Runner Nightly.app` each need one manual install of the first unified build, after which `nightly-win` and `Runner Nightly.app` are deleted by hand. Do not rewrite historical logs or the archived #493 material. Leave `design/windows-updates.pen` alone.

## Validation and review

Use the smallest relevant checks and report exact commands and results: `cargo test -p runner-app --test bundle_mac`, `cargo test -p runner-app`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`, `cargo fmt --all --check`, `make test-nightly`, `bash -n script/bundle-mac`, `git diff --check`. The new Windows updater test is `cfg(windows)` and does not run on the Mac; say so, and rely on the PR's `Rust / Windows` job. `actionlint`, `shellcheck`, and `pwsh` are unavailable here; inspect PowerShell changes by hand and say that.

Review the publish path against this table:

| Input | `nightly` after publish |
|---|---|
| `both` | new DMG, appcast, installer, `.sig`; each platform pruned to ten |
| `macos` | new DMG and appcast; DMG retention only; installers untouched |
| `windows` | new installer and `.sig`; installer retention only; DMG and appcast untouched |
| any, an upload or download fails | whatever was uploaded stays; nothing pruned; summary says incomplete |

`nightly-win` is never read or written.

Also review: the appcast never references a pruned DMG; production addresses and `release.yml` untouched; the nightly bundle plist differs from production only in `SUFeedURL`, `CFBundleShortVersionString`, and stamp-derived values.

## Handoff

The mission ends with a broadcast naming the branch, the exact changed files, exact check results, the reviewer's clean-verdict message id, and the live checks that remain. The landing session commits, opens the PR, cuts a nightly from the branch to validate, and asks Jason to do the two manual hops: install the Mac DMG over `Runner.app` and delete `Runner Nightly.app`; install the Windows installer on the PC. `nightly-win` is deleted by hand after the PC hop.

## First live cut — 2026-09-08

Dispatched by the landing session from the PR branch to validate before merge: `gh workflow run nightly.yml --ref feat/504-nightly-channel-unification` with the default `both`. The first attempt, run 34186646820, built both platforms and then stopped at the CI gate because the PR's `Rust / macOS` job had failed on `poll_until_returns_the_first_observed_value` in `runner-backend`, a timing test unrelated to this change; nothing was uploaded. After a rerun turned CI green, run [34187805571](https://github.com/yicheng47/runner/actions/runs/34187805571) on `2bf743d` (PR [#506](https://github.com/yicheng47/runner/pull/506)) succeeded through `publish`. `Rust / Windows` passed on the PR, the first native execution of the two new updater tests and the installer test.

- `nightly` now holds `Runner-Nightly-2bf743d.20260908.0440-arm64.dmg`, `appcast.xml`, `Runner-Setup-nightly.2bf743d.20260908.0440-x64.exe`, and its `.sig`, all anonymously downloadable, beside the retained `ec86f28` DMG from the #502 cut. Public prerelease, not draft.
- The appcast's channel title is `Runner`, the item is `2bf743d`, `sparkle:version` is the stamp, the arm64 requirement is present, and the enclosure length matches the uploaded DMG.
- The DMG mounts as a `Runner` volume containing `Runner.app` with bundle id `com.wycstudios.runner`, short version `2bf743d`, and the nightly `SUFeedURL`. `codesign --verify --deep --strict` passes and Gatekeeper reports Notarized Developer ID.
- `nightly-win` is untouched with its `ec86f28` installer and signature. `releases/latest` still resolves to `v0.8.2`.

Still open: the two hand installs (the DMG over `Runner.app` on the Mac, the installer on the PC), the next-cut in-place updates that follow them, single-platform cuts, and the manual deletion of `nightly-win` and `Runner Nightly.app`.
