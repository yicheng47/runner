# Preview nightly — an auto-cut channel for umbrella branches

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/ci-preview-nightly`, on branch `ci/preview-nightly`, cut from `origin/main` at `314286e4`. This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow `main` by rebase.

## Problem and outcome

#634's file preview is built on the umbrella `feat/634-file-preview` and reaches `main` only at the end, so today it gets no installable test builds. Cutting `nightly` from it would put unfinished work into every pre-release build of `main`. Jason asked on 2026-11-04 for a separate channel cut from the umbrella automatically on every change: the `remote-nightly` channel that #950 (`afa4fd11`) built for #836's `nightly-remote` umbrella and `19b3ba5f` retired on 2026-10-25, with two differences:

1. **It is cut automatically**, on every push to a listed umbrella branch that changes more than docs. Missions land on the umbrella by fast-forward, so there is no merge commit to test as `nightly-on-merge.yml` does on `main`.
2. **It covers Linux**, whose packages and in-app updater (`37772275`) came after `remote-nightly` was retired.

It is named `preview-nightly`, shown in the app as **Preview Nightly (<sha>)**, so the next umbrella reuses it by changing the branch list. The `nightly` and production channels must behave exactly as today.

## Read first

- `git show afa4fd11` and `git show 19b3ba5f`: the whole previous channel and its removal, across `nightly.yml`, `script/bundle-mac`, `script/bundle-windows.ps1`, `crates/runner-app/src/updater.rs`, `updater/windows.rs`, `tests/bundle_mac.rs`, `script/test-nightly.py`, `script/verify-nightly-appcast.py`, the release-notes file, the nightly skill and the docs. Re-apply it under the new name rather than redesigning it.
- `.github/workflows/nightly-on-merge.yml` and `.github/workflows/nightly.yml` as they are now (Linux jobs included).
- `script/bundle-linux` (`--channel`, `package_identity`, `release-info.json`), `crates/runner-app/src/updater/linux.rs` (`release_urls`) and `script/linux/`.
- `.agents/skills/nightly/SKILL.md`, `docs/arch/linux.md`, `docs/arch/windows.md`.

## Deliverables

1. **`nightly.yml`** gains a `channel` input (`nightly` by default, or `preview-nightly`). The prepare job cuts `nightly` only from `main` and `preview-nightly` only from a branch in the preview list; each channel has its own concurrency group, rolling release, tag, notes, Sparkle appcast, Windows feed and Linux feed, so neither cut cancels or overwrites the other. Retention as today.
2. **The auto cut**: on a push to a branch in the preview list, unless the push changes only `docs/**` or `**/*.md`, dispatch `nightly.yml` on that branch with `channel=preview-nightly` and `platform=all`. Keep the list in one place, starting with `feat/634-file-preview`, so adding or removing an umbrella is a one-line change. A push event runs the workflow file of the pushed commit, so the umbrella picks this up when it rebases onto `main`; note that in the skill.
3. **Bundles and updaters** for macOS, Windows and Linux: the `preview-nightly` channel in each bundle script; the updaters' Preview Nightly label and their release and feed URLs, so an installed preview build updates only from the preview feed. Installing a nightly or production build over a preview build moves it to that channel, as before.
4. **Tests**: `test-nightly.py`, `verify-nightly-appcast.py`, `bundle_mac.rs` and the updater unit tests cover both channels, including Linux, and prove `nightly` unchanged.
5. **Docs**: the nightly skill gains a Preview nightly section (what it is, the list, how an umbrella opts in and out, that the landing PR removes its branch from the list); `docs/arch/linux.md` and `docs/arch/windows.md` where they describe feeds; release notes for the preview release.

Non-goals: changing `nightly` or production behaviour, the release workflow, or anything on the umbrella. Do not dispatch any workflow, create releases or tags, or cut a build: the first real cut happens after this merges and the umbrella rebases. If a design question arises that this brief does not settle, ask the operator through Runner.

## Validation

`script/test-nightly.py` and `verify-nightly-appcast.py` run locally; `cargo test -p runner-app` for the updater and bundle tests; workspace clippy and fmt with the CI profile, logged with exit codes. `actionlint` on the changed workflows if available. CI on the PR covers Windows and Linux builds of the changed scripts and updaters.

Live checks: the reviewer names them. Expected, after merge and a real preview cut, which the operator arranges: installing a macOS preview build shows Preview Nightly and updates from the preview feed; a `nightly` cut from `main` still works and is unaffected. None can run before merge.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole working-tree diff against #950, its retirement and this brief, must-fix first with file:line pointers, checking in particular: that `nightly` and production are byte-for-byte unaffected, the branch guard in prepare, the docs-only skip, concurrency groups, and the Linux feed's naming.

## Authorization

Approved crew: the claude duo. Once review is clean the crew may commit, push this branch and open a PR against `main`, titled `ci(nightly): auto-cut a preview-nightly channel from umbrella branches`, labelled `risk-medium` and `needs-live-test`, and drive CI green on every job. Jason merges it.
