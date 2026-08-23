---
name: nightly
description: Cut, check, or version-bump the Runner nightly — dispatch nightly.yml on main, watch it, verify the rolling prerelease, record the stamp
---

# Nightly

Cut a build on the nightly channel. Nightlies are invisible to production installs by construction: the nightly bundle (`com.wycstudios.runner.nightly`, "Runner Nightly") polls `releases/download/nightly/appcast.xml`, the production bundle polls `releases/latest/download/appcast.xml`, and the `nightly` GitHub release is always a prerelease so the `releases/latest` alias never points at it. `nightly.yml` verifies both after publishing. Contract: `docs/arch/arch.md` §14 (full history: `docs/impls/archive/gpui-rewrite/plan.md` §Release channels).

Invoking this skill authorizes exactly two outward actions: the workflow dispatch and the one docs commit + push that records the stamp. Nothing else is pushed; run one outward git/gh action per command.

## Usage

`/nightly [run | status | bump <version>]` — no action means `run`.

### `run`

1. **Preflight — stop on any failure, say which.**
   - `git fetch origin`; on `main`, clean tree (`git status --short` empty), and `git rev-parse main origin/main` equal. The workflow builds `origin/main`'s head; unpushed local commits are not in the build — ask before pushing them.
   - `./script/bundle-mac --print-version` must end in `-nightly`. A production crate version (`0.6.0`) labels the build `0.6.0.<stamp>`, indistinguishable from a patch release; run `bump` first.
   - No nightly run already in progress: `gh run list --workflow nightly.yml --status in_progress --json databaseId`. `concurrency: cancel-in-progress` would kill it.
   - CI for the sha: `gh run list --commit $(git rev-parse origin/main) --workflow ci.yaml --limit 1 --json status,conclusion`. Missing or failed → stop; in progress is fine, the workflow's "Require CI green" step waits for it.
   - Live missions: if the `runner` MCP is reachable, `mission_list_summary` — report any running mission. Dispatching is safe (Sparkle only lights the sidebar hint; the user chooses when to install), but installing the nightly restarts the app and kills live PTYs, so say so.
2. **Dispatch:** `gh workflow run nightly.yml --ref main`. Wait ~10 s, then find it: `gh run list --workflow nightly.yml --limit 1 --json databaseId,status,headSha` and confirm `headSha` is the sha from preflight.
3. **Watch:** `gh run watch <id> --exit-status` (warm cache ~10 min; cold up to ~20; `timeout-minutes: 90`). On failure: `gh run view <id> --log-failed`, report the failing step, stop. A failure in "Verify updater channel isolation" or "Reject Tauri updater assets" means the production feed is at risk — report it verbatim and do not retry.
4. **Verify** the rolling prerelease, not just the run:
   - `gh release view nightly --json isPrerelease,assets --jq '{pre:.isPrerelease, assets:[.assets[].name]}'` — `pre` true; the new `Runner-Nightly-<version>.<stamp>-universal.dmg` and `appcast.xml` present; at most 10 DMGs (the workflow prunes).
   - `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` starts with `v` (the production alias did not move).
   - The stamp is `YYYYMMDD.HHMM` UTC from the run's "Compute build identity" step; the short version is `<crate version>.<stamp>`, e.g. `0.7.0-nightly.20260824.0312`.
5. **Record:** in `docs/impls/gpui-rewrite/README.md` §Nightlies, the `- **Current nightly**:` line — move the current entry into a trailing `Previous:` clause (keep the last three) and put the new stamp first with one clause on what it carries (`git log <previous nightly sha>..<sha> --oneline`, the landed M6 items by number) and `run <id> from <sha>`. Commit `docs(gpui-rewrite): nightly <stamp>` on `main`, push.
6. **Report:** short version, run id, sha, what it carries, and that the user's nightly app will show the update hint on its next silent check (Settings → Updates → Check for Updates forces it).

### `status`

- Current crate version (`./script/bundle-mac --print-version`) and whether it is a nightly version.
- Newest nightly: `gh release view nightly --json assets --jq '[.assets[].name | select(endswith(".dmg"))] | sort | last'`, plus the last nightly run (`gh run list --workflow nightly.yml --limit 3 --json databaseId,status,conclusion,headSha,createdAt`).
- Is `origin/main` ahead of the newest nightly's sha? `git log <nightly sha>..origin/main --oneline` — what a new cut would pick up.
- Channel isolation: `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` starts with `v`.

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
