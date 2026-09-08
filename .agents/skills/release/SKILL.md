---
name: release
description: Cut a Runner production release — bump the lockstep crates, tag vX.Y.Z, let release.yml build the draft for both platforms, write bilingual release notes (English, with 中文 collapsed in a details block), and hand the publish switch to the user
---

# Release

A production release is a `vX.Y.Z` tag on a `main` commit whose lockstep crate version is exactly `X.Y.Z`. `release.yml` builds the signed, notarized macOS DMG with its one-item Sparkle appcast and the Windows x64 installer with its minisign signature, then attaches everything to a **draft** release. Publishing the draft is the user's switch: it moves `releases/latest` and the production Sparkle feed. Contract: `docs/arch/arch.md` §14; nightlies are a separate channel handled by the `nightly` skill.

An explicit request to `run` authorizes the bump commit, the push, the tag, and editing the draft's notes. Nothing in this skill publishes the draft, deletes a release, or moves an existing tag. Record only observed results.

## Usage

`/release [run <version> | notes <version> | check <version>]` — no action means `run`.

## `run <version>`

1. **Preflight — stop on any failure and report it.**
   - `<version>` must match `^[0-9]+\.[0-9]+\.[0-9]+$` and be greater than the current tag from `gh release list`.
   - `git fetch origin`; require branch `main`, a clean tree, and `git rev-parse main origin/main` equal. Everything meant for the release is already merged.
   - CI on `origin/main` is green: `gh run list --branch main --workflow ci.yaml --limit 1 --json conclusion`.
   - No release run is in flight: `gh run list --workflow release.yml --limit 3 --json status`.
2. **Bump.** Set `[package].version` to `<version>` in `crates/runner-app/Cargo.toml`, `crates/runner-backend/Cargo.toml`, and `crates/runner-terminal/Cargo.toml`. Leave `runner-core` and the CLI alone. Run `cargo check --workspace` to refresh `Cargo.lock`, then `cargo test -p runner-app --test bundle_mac`. Commit the four files as `chore: bump version to <version>` and push `main`.
3. **Tag.** `git tag -a v<version> -m "Runner <version>" && git push origin v<version>`. The push starts `release.yml`.
4. **Watch.** Find the run with `gh run list --workflow release.yml --limit 1 --json databaseId,status,headSha` and `gh run watch <id> --exit-status`. Both build jobs and the publish job must succeed. On failure, `gh run view <id> --log-failed`, report the failing step, and stop; the tag stays, the draft may be partial.
5. **Notes.** Follow `notes <version>` below and set them on the draft: `gh release edit v<version> --notes-file <file>`. Do not pass `--draft=false`.
6. **Report.** The draft URL, both asset names, and the reminder that publishing is the user's action. After the user publishes, verify `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` is `v<version>` and that `releases/latest/download/appcast.xml` names the new DMG.

## `notes <version>`

Release notes are **bilingual**: the full English text first, then the same content in 中文 inside a `<details>` block so the page reads as English-only until a reader expands it (GitHub has no tabs; a collapsed block is the nearest thing, and both in-app updaters open the release page in a browser where it renders). 0.8.3 used a horizontal rule instead; every release since 0.8.4 ships the collapsed form. A draft with only the workflow's one-line stub is not ready to publish.

Source the content from `git log v<previous>..v<version> --no-merges --format='- %s (%h)'` and the closed issues those commits reference. Write for users, not for the repo: what changed for them, in their words, one bullet per change with the issue number at the end. Skip internal work (tests, CI, docs, refactors) unless it changes what users see.

Structure, both languages:

```
Runner <version> is a <bug-fix | feature> release for macOS and Windows on top of <previous>.

## New                      (omit if empty)
- **Short bold lead.** One or two sentences. (#123)

## Bug fixes                (omit if empty)
- ...

## Nightly channel          (only when the nightly channel changed; nightly users read these)
- ...

## Download and upgrade
- **macOS, Apple Silicon:** download `Runner-<version>-arm64.dmg`, or update through Sparkle. The app is signed and notarized.
- **Windows x64, Windows 10 version 1809 or later:** download the `Runner-Setup-<version>.<stamp>-x64.exe` installer. The installer is currently **unsigned**, so Windows may show a SmartScreen warning. Settings, chats, and missions are retained.

Windows ARM64, Intel Macs, and Linux are not supported.

**Full changelog:** https://github.com/yicheng47/runner/compare/v<previous>...v<version>

<details>
<summary>中文</summary>

Runner <version> 是 macOS 和 Windows 上基于 <previous> 的<修复版本 | 功能版本>。

## 新功能
...
## 问题修复
...
## Nightly 渠道
...
## 下载与升级
...

Windows ARM64、Intel Mac 和 Linux 暂不支持。

**完整变更记录：** https://github.com/yicheng47/runner/compare/v<previous>...v<version>

</details>
```

Keep a blank line after `<summary>` and before `</details>`, or GitHub renders the markdown inside as literal text.

Rules for the 中文 half: translate the meaning, not the words; keep product terms as they appear in the app (Runner, Sparkle, Nightly, ⌘, Settings → Updates); keep file names, issue numbers, and links identical to the English; use the same headings in the same order so a reader can line the two halves up. Headings inside the details block use `##` like the English half; they render collapsed until expanded. Read the exact Windows installer name from the draft's assets (`gh release view v<version> --json assets --jq '.assets[].name'`) rather than guessing the stamp.

## `check <version>`

Read-only. `gh release view v<version> --json isDraft,isPrerelease,assets,body`: report draft state, that the DMG, `appcast.xml`, installer, and `.sig` are all present, and whether the body has the English section and a `<details>` block containing the 中文 section. After publishing, also verify `releases/latest` resolves to the tag and the production appcast's enclosure names the new DMG.

## Notes

- The crate version stays at `X.Y.Z` after the release; nightlies do not bump it (see the `nightly` skill).
- `workflow_dispatch` of `release.yml` with `dry_run` builds the same artifacts into a throwaway draft named `dry-run-<stamp>` for inspection; delete that draft afterwards.
- Windows Authenticode signing is tracked in #497; until it lands the notes carry the SmartScreen sentence.
- Apple credentials for local notarization checks live in `~/.zshrc`; `xcrun notarytool history` and `stapler validate <file>` are the tools.
