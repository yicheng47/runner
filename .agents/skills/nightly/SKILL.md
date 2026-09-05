---
name: nightly
description: Cut, check, or version-bump the Runner nightly — dispatch nightly.yml on main, watch it, verify the rolling draft release, record the stamp
---

# Nightly

Cut a build on the nightly channel. The `nightly` GitHub release is a rolling **draft** (since 2026-08-24): invisible to anyone without push access — the repo is public and general users must not find nightlies — and its assets are not anonymously downloadable, so the nightly has no Sparkle feed and can never touch production installs. The nightly bundle (`com.wycstudios.runner.nightly`, "Runner Nightly") still bakes `SUFeedURL` `releases/download/nightly/appcast.xml`, which 404s by design. Installs are manual: `gh release download nightly` or a local `./script/bundle-mac --channel nightly` build (the `../runner-nightly` worktree exists for that). `nightly.yml` verifies the release stays hidden after publishing. Contract: `docs/arch/arch.md` §14 (full history: `docs/impls/archive/gpui-rewrite/plan.md` §Release channels).

Invoking this skill authorizes exactly two outward actions: the workflow dispatch and the one docs commit + push that records the stamp. Nothing else is pushed; run one outward git/gh action per command.

## Usage

`/nightly [run | status | bump <version>]` — no action means `run`.

### `run`

1. **Preflight — stop on any failure, say which.**
   - `git fetch origin`; on `main`, clean tree (`git status --short` empty), and `git rev-parse main origin/main` equal. The workflow builds `origin/main`'s head; unpushed local commits are not in the build — ask before pushing them.
   - `./script/bundle-mac --print-version` must end in `-nightly`. A production crate version (`0.6.0`) labels the build `0.6.0.<stamp>`, indistinguishable from a patch release; run `bump` first.
   - No nightly run already in progress: `gh run list --workflow nightly.yml --status in_progress --json databaseId`. `concurrency: cancel-in-progress` would kill it.
   - CI for the sha: `gh run list --commit $(git rev-parse origin/main) --workflow ci.yaml --limit 1 --json status,conclusion`. Missing or failed → stop; in progress is fine, the workflow's "Require CI green" step waits for it.
   - Live missions: if the `runner` MCP is reachable, `mission_list_summary` — report any running mission. Dispatching is safe (nothing auto-updates; nightly installs are manual), but installing the nightly restarts the app and kills live PTYs, so say so.
2. **Dispatch:** `gh workflow run nightly.yml --ref main`. Wait ~10 s, then find it: `gh run list --workflow nightly.yml --limit 1 --json databaseId,status,headSha` and confirm `headSha` is the sha from preflight.
3. **Watch:** `gh run watch <id> --exit-status` (warm cache ~10 min; cold up to ~20; `timeout-minutes: 90`). On failure: `gh run view <id> --log-failed`, report the failing step, stop. A failure in "Verify the nightly release stays hidden" means the nightly leaked onto the public releases page — report it verbatim and do not retry.
4. **Verify** the rolling draft, not just the run:
   - `gh release view nightly --json isDraft,assets --jq '{draft:.isDraft, assets:[.assets[].name]}'` — `draft` true; the new `Runner-Nightly-<version>.<stamp>-arm64.dmg` present; at most 10 DMGs (the workflow prunes). `gh` resolves the draft by tag only with push access — that is the point.
   - The stamp is `YYYYMMDD.HHMM` UTC from the run's "Compute build identity" step; the short version is `<crate version>.<stamp>`, e.g. `0.7.0-nightly.20260824.0312`.
5. **Record:** in `docs/impls/gpui-rewrite/README.md` §Nightlies, the `- **Current nightly**:` line — move the current entry into a trailing `Previous:` clause (keep the last three) and put the new stamp first with one clause on what it carries (`git log <previous nightly sha>..<sha> --oneline`, the landed M6 items by number) and `run <id> from <sha>`. Commit `docs(gpui-rewrite): nightly <stamp>` on `main`, push.
6. **Report:** short version, run id, sha, what it carries, and how to install: `gh release download nightly --pattern '*<stamp>*'`, then open the DMG. No update hint appears in the app — the nightly feed is dead by design.

### `status`

- Current crate version (`./script/bundle-mac --print-version`) and whether it is a nightly version.
- Newest nightly: `gh release view nightly --json assets --jq '[.assets[].name | select(endswith(".dmg"))] | sort | last'`, plus the last nightly run (`gh run list --workflow nightly.yml --limit 3 --json databaseId,status,conclusion,headSha,createdAt`).
- Is `origin/main` ahead of the newest nightly's sha? `git log <nightly sha>..origin/main --oneline` — what a new cut would pick up.
- Hidden: `gh release view nightly --json isDraft --jq .isDraft` is `true`.

### `bump <version>`

After a production release, move the crates to the next nightly version (`0.6.0` → `0.7.0-nightly`; the minor never rolls during the nightly period). The one hand bump the other way (`X.Y.0-nightly` → `X.Y.0`) happens at cutover and is not this skill.

1. On `main`, clean tree, in sync with `origin/main`.
2. `<version>` must match `^\d+\.\d+\.\d+$`; the target is `<version>-nightly`.
3. The three crates carry the version in lockstep: `crates/runner-app/Cargo.toml`, `crates/runner-backend/Cargo.toml`, `crates/runner-terminal/Cargo.toml`. Read each, then Edit the `version = "…"` line under `[package]` — not `sed`, and not any dependency line.
4. `cargo check --workspace` to refresh `Cargo.lock` (CI fails on an uncommitted lockfile change).
5. `cargo test -p runner-app --test bundle_mac` — the plist tests derive their expectations from `bundle-mac --print-version`, so they must stay green.
6. Commit the three manifests plus `Cargo.lock`: `chore: bump version to <version>-nightly`. Push `main`.

## Notes

- Nightlies are dispatch-only (decision 12, amended 2026-08-21): a push to `main` runs CI alone, so a landing never restarts the user's app by accident.
- The nightly and production apps share one data directory (`~/Library/Application Support/com.wycstudios.runner/`). One instance at a time.
- Production releases are a different path: tag `vX.Y.Z` on a commit whose crate version is exactly `X.Y.Z` → `release.yml` builds a draft; publishing is the human's switch. Do not tag from this skill.

## Windows variant

`/nightly windows [cut | check | stamp]` — no action means `cut`. This variant uses the existing `runner-windows` checkout and the long-lived `nightly-windows` branch. Its rolling `nightly-win` release is a public prerelease; the macOS commands and draft policy above stay as is. Never run the app or install anything on the PC from this skill.

### `windows cut`

1. **Preflight — stop on any failure, say which.** Run `git fetch origin`; require branch `nightly-windows`, a clean tree, and `git rev-parse nightly-windows origin/nightly-windows` equal. Read the version with `cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "runner-app") | .version'`. No version bump is required for this separate prerelease channel.
2. Check that no Windows nightly run is in progress with `gh run list --workflow nightly.yml --branch nightly-windows --status in_progress --json databaseId`; another Windows dispatch would cancel it. Windows and macOS use separate concurrency groups. Check CI for the selected sha with `gh run list --commit <sha> --workflow ci.yaml --limit 1 --json status,conclusion`. Missing or failed → stop; in progress is fine, the workflow waits for it. Both `Rust / macOS` and `Rust / Windows` must pass.
3. **Dispatch:** `gh workflow run nightly.yml --ref nightly-windows -f platform=windows`. Find the new run with `gh run list --workflow nightly.yml --branch nightly-windows --limit 1 --json databaseId,status,headSha`; confirm its `headSha` matches preflight.
4. **Watch:** `gh run watch <id> --exit-status`. On failure, inspect `gh run view <id> --log-failed`, report the failing step, and stop. A failure in the public-release verification means the friend cannot use the download link; do not report the build as ready.
5. Run `windows check`, then `windows stamp`. Report the version, UTC stamp, run id, sha, and public x64 zip URL. Jason extracts the zip and runs the Phase 1 PC checklist in `docs/impls/437-windows-nightly.md`.

### `windows check`

- Read `gh release view nightly-win --json isPrerelease,isDraft,assets --jq '{prerelease:.isPrerelease, draft:.isDraft, assets:[.assets[].name]}'`; require `prerelease` true and `draft` false.
- Confirm the expected `Runner-Nightly-<version>.<stamp>-x64.zip` is present and there are at most ten timestamped x64 zips. The stamp is `YYYYMMDD.HHMM` UTC from the run's build-identity step.
- Require an anonymous download check to succeed: `curl --silent --fail --head --location "https://github.com/yicheng47/runner/releases/download/nightly-win/<zip>"`. The release page is `https://github.com/yicheng47/runner/releases/tag/nightly-win`.
- Inspect recent runs with `gh run list --workflow nightly.yml --branch nightly-windows --limit 3 --json databaseId,status,conclusion,headSha,createdAt`; use `git log <nightly sha>..origin/nightly-windows --oneline` to show what a new cut would pick up.

### `windows stamp`

After a successful cut and public-download check, record the x64 zip filename and URL, UTC stamp, run id, source sha, and a short description of what it carries under `## Windows nightlies` in `docs/impls/437-windows-nightly.md` (create the section on the first cut). Keep the current entry first and the last three previous entries. Record PC results only after Jason supplies them. Commit only that document as `docs: Windows nightly <stamp>` on `nightly-windows`, then push that branch. This uses the skill's existing docs-commit authorization; it does not authorize code pushes, merging, or a macOS dispatch.
