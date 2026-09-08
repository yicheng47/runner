# 502 — Unified nightly implementation

Tracking issue: [#502](https://github.com/yicheng47/runner/issues/502). Spec: [One nightly for both platforms](../features/502-unified-nightly.md). Baseline: `main` at `99e32ef`, Runner 0.8.2, 2026-09-08. Status: commit-only nightly identity implemented, locally validated, and reviewed clean per Jason’s 2026-09-08 clarification. All changes remain uncommitted; post-landing live verification is pending.

## Goal

Implement one nightly dispatch that builds macOS and Windows from one commit and stamp, publishes the existing two rolling prereleases, and gives macOS nightlies a working Sparkle feed. Deliver a locally validated, reviewed working-tree diff. No UI design is needed: this is packaging, workflow, and documentation work with no new app surface.

## Checkout and authorization

- Work in the existing `/Users/jason/repos/yicheng47/runner` checkout. The coder creates `feat/502-unified-nightly` before editing implementation files. Do not create another checkout or worktree.
- The uncommitted changes to `docs/features/502-unified-nightly.md`, this brief, and `docs/impls/README.md` were prepared for this mission and belong to it. Preserve and include them in review; they are not unrelated user edits.
- The two existing crew slots are the authorized workers. Do not spawn additional agents, crews, or review fan-outs.
- Implement, validate, and exchange review messages through Runner. The reviewer waits for the coder's handoff, critiques the diff, and does not edit it. Iterate until there are no must-fix findings, then report to Jason.
- Leave all changes uncommitted. No commit, push, PR, merge, tag, workflow dispatch, release mutation, app launch/restart, or installer execution against the real installation. The nightly skill is a reference and an edit target, not a request to execute its outward actions. Do not copy the commit authorization from the archived #493 brief.
- Follow `AGENTS.md`; keep background helpers scoped to their foreground command and do not leave polling loops running.

## Scope and implementation order

### 1. Shared identity and job graph

Jason’s clarification supersedes the initial `0.8.3-nightly` version chore: nightly is a rolling development channel identified by its commit; only official releases get `vX.Y.Z` version tags. Keep `runner-app`, `runner-backend`, `runner-terminal`, and `Cargo.lock` at the baseline `0.8.2` package version. Leave `runner-core` and the CLI alone. Run `cargo check --workspace` to verify the lockfile.

Restructure `.github/workflows/nightly.yml` following the existing shape in `.github/workflows/release.yml`, without changing the production workflow:

- `platform` accepts `both`, `macos`, and `windows`, defaulting to `both`; retain an informative run name and dispatch-only triggering.
- `prepare` computes one UTC stamp, captures the dispatch SHA, and exports the identities both build jobs consume. The machine identity is `nightly.<short-sha>.<stamp>`; the human display is `Nightly (<short-sha>)`. Do not read or gate on the crate version, and do not require a nightly version bump.
- Use one `nightly` concurrency group with cancellation of an older run. The platform must not appear in the group key.
- `build-macos` and `build-windows` depend on `prepare`, build the same source commit, and upload distinct artifacts from the current run. Preserve the existing build targets, cache setup, signing, notarization, Windows installer tests, and Windows signature key-id check.
- `publish` depends on all three jobs and explicitly handles skipped unselected builds. It runs only when prepare and every selected build succeeded. With `both`, either build failing or being cancelled prevents publishing either new build. The CI gate runs once here, tied to the dispatch SHA, before changing either release.
- Limit artifact downloads to the selected platforms and the current workflow run. Validate expected filenames from the prepared identity; do not silently use an older artifact when one is absent.

### 2. Feed and rolling publication

Update `bundle-mac` so nightly DMGs are `Runner-Nightly-<sha>.<stamp>-arm64.dmg`, `CFBundleVersion` stays the UTC stamp, and `CFBundleShortVersionString` is just the seven-character commit, since Sparkle supplies the `Runner Nightly` app name in its native alert. Supply `Nightly` as the app marketing version so the existing display formatter appends the commit once. Update `bundle-windows.ps1` to use `nightly.<sha>.<stamp>` for installer identity and `Nightly` for app marketing, retaining numeric package metadata for Windows resources. Format the macOS short-commit and new Windows update identities as `Nightly (<sha>)`, preserving legacy labels and the existing trailing-stamp comparison. Add relevant display and compatibility regression tests.

In the macOS build, reuse `release.yml`'s pinned Sparkle archive and checksum, `SPARKLE_ED_PRIVATE_KEY`, and `generate_appcast` invocation. The download prefix is `https://github.com/yicheng47/runner/releases/download/nightly/`; release-note/link URLs point at the `nightly` release. Preserve the signed DMG enclosure and `arm64` hardware requirement. Upload the DMG and appcast as the macOS artifact. No private key belongs in an artifact or log.

Publish only selected platforms: `nightly` receives the DMG followed by `appcast.xml`; `nightly-win` receives the installer followed by its `.sig`. Existing drafts are converted in place, both tags stay public prereleases, and neither becomes latest stable. Keep the Windows tag and baked updater URL unchanged. Reuse the existing Windows minisign verification; Authenticode remains #497.

Verify release flags and anonymous access to the expected files, including the appcast and Windows signature. Check the appcast enclosure URL and identity against the new DMG. Prune each selected channel to ten builds ordered by UTC stamp; Windows deletion is paired with the matching `.sig`. The retained appcast must not point at deleted files. Do not alter an unselected platform's release, feed, or retention. Preserve the latest stable release and production feed. A partial publish is a failed/incomplete run, not an atomic transaction to roll back.

### 3. Documentation and nightly skill

Update `.agents/skills/nightly/SKILL.md`, `docs/arch/arch.md` section 14, and `docs/arch/windows.md` to describe one workflow, both as the default, optional single-platform cuts, one shared CI gate/concurrency group, the commit-only nightly identity and official-only version tags, and public macOS nightlies that update through Sparkle. Update nightly release-note text in the workflow and any current nightly instructions that would otherwise contradict this behavior; do not rewrite historical logs.

The skill should expose one `run` and one `check` with an optional platform (`both` by default); remove nightly `bump`. Remove the separate Windows implementation path and the obsolete claims that all nightly installs are manual or that dispatch cannot lead to an update offer. Retain clean-main/source-SHA/CI preflight and check for any queued or running nightly because the concurrency group is shared. Record build results without inventing a successful cut or a PC smoke result. Keep commit/push and install actions subject to the user's explicit instructions.

## Validation and review

Use the smallest relevant checks, and report exact commands and results. Run `cargo check --workspace`, `cargo test -p runner-app --test bundle_mac`, and `cargo fmt --all --check` after the packaging changes. Because update display behavior changes, also run `cargo test -p runner-app` and workspace Clippy. Validate the workflow with `actionlint` if available and check changed shell/PowerShell syntax with available native tooling. If a tool is unavailable, report it and use focused inspection or existing tooling; do not claim an unrun check passed.

Review the publish predicate against this table; skipped jobs must not suppress a valid single-platform publish, and failure/cancellation must not open the gate:

| Input | macOS build | Windows build | Expected publication |
|---|---|---|---|
| default / `both` | success | success | Both channels |
| `macos` | success | skipped | macOS only |
| `windows` | skipped | success | Windows only |
| `both` | failure or cancelled | success | None |
| `both` | success | failure or cancelled | None |
| Any | prepare failed, or selected build failed/cancelled | As applicable | None |
| Any otherwise valid combination | CI missing or failed | As applicable | None |

Review shared stamp/SHA propagation, artifact selection, upload ordering, retention with old and new filename conventions, feed/signature URLs, and stable-channel isolation. Add focused executable coverage only for behavior introduced or an uncovered compatibility case; do not build a generic workflow test framework. `installer_version` and `available_update` in `crates/runner-app/src/updater/windows.rs` are the compatibility references; keep their filename parsing and timestamp comparison intact; the new commit-based update display is in scope per Jason’s clarification. Any Rust behavior change gets the relevant crate tests and workspace Clippy in addition to the packaging checks. Windows-only tests cannot be claimed as executed on this Mac.

## Handoff and subsequent live verification

The mission ends with the feature branch name, changed files, check results, the review verdict, and the remaining live checks. Keep the feature issue open and the spec unarchived until those checks are actually completed.

After Jason authorizes landing and cuts, verify default/both and each single-platform dispatch, cancellation of an older run, identical stamps across both artifacts, public downloads and ten-build retention, unchanged latest stable, and a real installed-nightly upgrade on both machines. Until then, leave the spec's live verification boxes unchecked. No test dispatch or release mutation is part of this mission.

## Revised identity — 2026-09-08

The initial semantic-nightly implementation passed working-tree review, but Jason then clarified that nightly is an independent rolling development channel identified by a commit, not a preview of `0.8.3` or any other official version. This brief and the feature spec now use that clarified contract. The renewed verdict recorded below supersedes the earlier review. The public rolling release addresses and shared workflow stay unchanged.

Local validation and the renewed clean review are recorded below. No live cut, release mutation, app launch/restart, or installation is authorized. The feature remains open and unarchived until the post-landing checks in the spec are actually completed.

## Local validation of the commit-identity revision

The app and Runner’s update indicators display `Nightly (<sha>)`. The macOS bundle/appcast short version is the seven-character commit, so Sparkle’s own alert can combine it with the `Runner Nightly` app name without repeating “Nightly”. Windows keeps numeric resource metadata from the crate version and uses the stamped `nightly.<sha>.<stamp>` installer identity. The SHA is validated before the new Windows seven-character slice. The existing Windows installer parser and update stamp comparison are unchanged; a Windows-only regression test covers upgrading from a legacy installer to a commit-based nightly.

| Exact check command | Result |
|---|---|
| `cargo check --workspace` | Passed. The three package versions and lockfile are back at the `0.8.2` baseline. |
| `CARGO_BUILD_JOBS=12 cargo test -p runner-app` | Passed: 298 tests (72 library, 192 binary, 3 bundle, 18 pane layout, 4 session integration, 3 IME, 6 text utilities); 0 failures; doc-tests 0. This includes both nightly/production bundle checks and the new app/update display regression. |
| `CARGO_BUILD_JOBS=12 cargo clippy --workspace --all-targets -- -D warnings` | Passed. |
| `CARGO_BUILD_JOBS=12 cargo clippy -p runner-app --features updater --all-targets -- -D warnings` | Passed. |
| `cargo fmt --all --check` | Passed after formatting the new tests. |
| `UV_CACHE_DIR=/tmp/runner-502-uv-cache UV_OFFLINE=1 make test-nightly` | Passed: 10 tests in 1.012 seconds, including 384 publish-predicate combinations, all 21 Bash run steps, commit-only identity without reading the crate version, source/artifact wiring, failure summaries, single-platform isolation, and paired old/new retention. |
| `bash -n script/bundle-mac` | Passed. |
| `git diff --exit-code -- Cargo.lock crates/runner-app/Cargo.toml crates/runner-backend/Cargo.toml crates/runner-terminal/Cargo.toml` | Passed: no differences from baseline. |
| `UV_CACHE_DIR=/tmp/runner-502-uv-cache uv run --offline --no-project --with pyyaml python /Users/jason/.codex/skills/.system/skill-creator/scripts/quick_validate.py .agents/skills/nightly` | Passed: `Skill is valid!`. |
| `git diff --check` | Passed. |

A standalone experiment extracted the unchanged `installer_version` function, compiled it with `rustc --test <temporary parser.rs> -o <temporary parser-test>`, and ran it: 1 passed. It covered legacy bare and semantic-nightly filenames, the new `nightly.<sha>.<stamp>` filename, stamp ordering across base-version differences, and rejecting `.sig` as an installer. This is separate from the Windows-only regression test, which was not executed on this Mac.

The first sandboxed full-app run failed only because an existing MCP socket test could not bind its temporary Unix socket (`Operation not permitted`); the full rerun with socket permission passed. Existing future-incompatibility warnings remain for `block 0.1.6` and `proc-macro-error2 2.0.1`. `actionlint`, `shellcheck`, and PowerShell are unavailable here; PowerShell changes were inspected, not executed. PyYAML 6.0.3 lives only in a temporary uv cache. No workflow dispatch, signing/notarization run, release mutation, real app launch/restart, or installation was performed. All live verification remains pending after landing and authorization.

## Changed files

| Area | Exact files |
|---|---|
| Workflow and validation | `.github/workflows/nightly.yml`, `Makefile`, `script/test-nightly.py` (new), `script/verify-nightly-appcast.py` (new) |
| Packaging and app display | `script/bundle-mac`, `script/bundle-windows.ps1`, `crates/runner-app/src/updater.rs`, `crates/runner-app/src/updater/windows.rs` (test only), `crates/runner-app/src/version.rs` (test only), `crates/runner-app/tests/bundle_mac.rs` |
| Skill and architecture | `.agents/skills/nightly/SKILL.md`, `docs/arch/arch.md`, `docs/arch/windows.md` |
| Spec/brief and live instructions | `docs/features/502-unified-nightly.md`, `docs/features/README.md`, `docs/impls/502-unified-nightly.md` (new), `docs/impls/README.md`, `docs/features/468-landing-page.md`, `docs/impls/gpui-rewrite/README.md`, `docs/impls/gpui-rewrite/m6-remainder.md` |

All 20 files are mission-owned working-tree changes on `feat/502-unified-nightly`, uncommitted and unstaged. The package manifests and `Cargo.lock` are no longer part of the diff.

## Renewed review verdict

Reviewer reported **CLEAN — no remaining must-fix issues** through Runner message `01M1ZDQDJF2M8VSES5ZY0SEMAB` at `2026-09-08T02:32:03.407803Z`, reviewing the 20-file working-tree diff at `99e32ef` (recorded diff hash `3607d3768a4c7cf758b2f06c888c5591ddee96a2`). Reviewer independently reran formatting, the full `runner-app` tests, workspace Clippy, updater-feature Clippy, and the nightly workflow tests; all passed. Only this verdict/status bookkeeping was edited afterward.

The Windows-only updater regression and PowerShell packager still require native Windows validation. Signing, notarization, actual GitHub job scheduling/publication, public downloads, retention, stable-feed isolation, and installed-nightly upgrades remain live checks after landing and explicit authorization.
