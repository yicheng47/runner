---
name: nightly
description: Cut or check Runner nightlies — one workflow for macOS and Windows, public prereleases, shared stamp, and separate stable feeds
---

# Nightly

One `nightly.yml` dispatch builds both platforms by default from one SHA and UTC stamp. `platform=macos` or `platform=windows` selects one. The rolling `nightly` release serves macOS DMGs and a signed Sparkle appcast; `nightly-win` serves Windows x64 installers and matching minisign signatures at the existing updater URL. Both are public prereleases and keep ten builds by stamp. Production feeds and `releases/latest` stay separate. Contract: `docs/arch/arch.md` §14 and `docs/arch/windows.md`.

An explicit request to `run` authorizes that workflow dispatch. Reading or editing this skill and `check` do not authorize dispatch. Nightly is a rolling development channel identified by its commit; official version tags are separate, and no nightly crate-version bump is needed. Commit, push, merge, tagging, direct release mutation, app launch/restart, and installation require the user's explicit instruction. Record only observed results; never invent a successful cut or a PC smoke result.

## Usage

`/nightly [run [both|macos|windows] | check [both|macos|windows]]` — no action means `run`; omitted platform means `both`.

## `run [platform]`

1. **Preflight — stop on any failure and report it.**
   - `git fetch origin`; require branch `main`, a clean tree (`git status --short` empty), and `git rev-parse main origin/main` equal. Capture the full `origin/main` SHA. Do not push local commits without instruction.
   - Check for **any** queued or running nightly, including other platforms: `gh run list --workflow nightly.yml --limit 30 --json databaseId,status,displayTitle,headSha`. Stop if a run is not completed; all platforms share concurrency group `nightly`, so a dispatch cancels the entire older run.
   - CI for the SHA: `gh run list --commit <sha> --workflow ci.yaml --limit 1 --json status,conclusion`. Missing or completed without `success` → stop. Queued/in-progress is fine: the single publish job waits for CI and verifies its successful conclusion. Both macOS and Windows CI must pass even for a single-platform cut.
   - If the Runner MCP is reachable, report running missions from `mission_list_summary`. A successful cut can produce a nightly update offer and a Windows background download. Installation/restart remains a separate user action and interrupts live PTYs.
2. **Dispatch:** `gh workflow run nightly.yml --ref main` for both, or append `-f platform=macos` / `-f platform=windows`. Find the new run with `gh run list --workflow nightly.yml --branch main --limit 5 --json databaseId,status,headSha,displayTitle,createdAt`; require the selected `Nightly (<platform>)` title, creation after dispatch, and `headSha` equal to preflight. If the source changed during dispatch, report the mismatch and do not claim the intended cut succeeded.
3. **Watch:** `gh run watch <id> --exit-status`. On failure, inspect `gh run view <id> --log-failed` and report the failing step. A failed/cancelled selected build prevents publication of both channels on a `both` run. Failure or cancellation during publication can leave some files uploaded; report the cut incomplete and do not automatically retry.
4. **Verify:** run `check` for the selected platform(s), using this run's exact commit-based identity, stamp, and SHA. The shared identity is in `prepare`'s build-identity outputs/log and the successful publication job summary. Require the whole run to have succeeded, not just one build job.
5. **Record:** after successful verification, add a dated cut record to `docs/impls/502-unified-nightly.md` (or its archived location after completion): selected platforms, commit-based identity, UTC stamp, run id, source SHA, exact asset filenames/URLs, and a short change summary from `git log <previous nightly sha>..<sha> --oneline`. Keep prior records intact. Record installed upgrade/PC results only after observing them or receiving them from Jason. Follow `AGENTS.md` branch rules before editing; leave the record uncommitted unless the user explicitly requests commit/push.
6. **Report:** shared identity, run result, download links, changes included, and any installed-upgrade checks still pending. macOS nightlies can update through Sparkle; Windows nightlies use the in-app updater. Do not install or restart either app from this skill without explicit instruction.

## `check [platform]`

This is read-only and does not dispatch. Read recent `nightly.yml` runs with `gh run list --workflow nightly.yml --branch main --limit 10 --json databaseId,status,conclusion,headSha,createdAt,displayTitle`. A `both` run applies to both channels; a later single-platform cut may mean the channels now have different stamps. Associate each release with its actual successful run, and use `git log <nightly sha>..origin/main --oneline` to describe what a new cut would pick up. If the run/SHA cannot be established, report it unknown.

For each selected channel, use `gh release view <tag> --json isPrerelease,isDraft,assets`; require `isPrerelease=true`, `isDraft=false`, the expected filenames, and at most ten timestamped installers/DMGs. Sort by trailing `YYYYMMDD.HHMM` stamp, not by the version prefix. Both apps display `Nightly (<sha>)`, independently of the crate version. The UTC stamp orders artifacts and updates; the short commit identifies their source. For example, a Windows artifact identity is `nightly.abc1234.20260908.0100`.

- **macOS / `nightly`:** require `Runner-Nightly-<sha>.<stamp>-arm64.dmg` and `appcast.xml`. Check anonymous DMG access with `curl --silent --show-error --fail --head --location <asset-url>`; download and parse the appcast. Its single signed enclosure must point to that DMG at `https://github.com/yicheng47/runner/releases/download/nightly/`, with the expected stamp, seven-character commit as its short version, and `arm64` hardware requirement. Sparkle supplies the app name in its native alert; Runner’s own update display is `Nightly (<sha>)`. The release page is `https://github.com/yicheng47/runner/releases/tag/nightly`.
- **Windows / `nightly-win`:** require `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` and its exact `.sig`; anonymous access must succeed for both at `https://github.com/yicheng47/runner/releases/download/nightly-win/`. Installers/signatures prune together. Portable ZIPs are retired. The release page is `https://github.com/yicheng47/runner/releases/tag/nightly-win`.
- **Stable isolation:** verify `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` still identifies the current production release (0.8.2 when #502 was implemented), not a nightly. Do not report a real installed update or production Sparkle check as tested merely because release metadata is correct.

## Notes

- Nightlies are dispatch-only: a push to `main` runs CI alone. Nightlies need no crate-version bump or relationship to the next official version. One `publish` job validates selected artifacts, gates on CI once, uploads the DMG before its appcast and the installer before its signature, checks public downloads, then prunes. A single-platform cut leaves the other release/feed untouched.
- Nightly and production macOS apps share `~/Library/Application Support/com.wycstudios.runner/`; run one instance at a time. Public prereleases can be found by browsing, while production installs read their separate stable feed.
- Production releases remain a different path: tag `vX.Y.Z` at a bare `X.Y.Z` crate version → `release.yml` builds a draft; publishing is the human's switch. Do not tag from this skill.
