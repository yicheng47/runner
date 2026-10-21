---
name: release
description: Cut a Runner production release — bump the workspace version, tag vX.Y.Z, let release.yml build the draft for macOS, Windows and Linux, write bilingual release notes (English, with 中文 collapsed in a details block), hand the publish switch to the user, and sweep the shipped work's docs once the release is live
---

# Release

A production release is a `vX.Y.Z` tag on a `main` commit whose workspace version is exactly `X.Y.Z`. `release.yml` builds the signed, notarized macOS DMG with its one-item Sparkle appcast and the Windows x64 installer with its minisign signature, plus Linux ARM64/x86_64 tar/deb/rpm packages, then attaches everything to a **draft** release. Publishing the draft is the user's switch: it moves `releases/latest` and the production Sparkle feed. Contract: `docs/arch/arch.md` §14; nightlies are a separate channel handled by the `nightly` skill.

An explicit request to `run` authorizes the bump commit, the push, the tag, and editing the draft's notes; after the user publishes, it also authorizes the sweep's docs commit and push. An explicit `sweep` request authorizes that commit and push on its own. Nothing in this skill publishes the draft, deletes a release, or moves an existing tag. Record only observed results.

## Usage

`/release [run <version> | notes <version> | check <version> | sweep <version>]` — no action means `run`.

## `run <version>`

1. **Preflight — stop on any failure and report it.**
   - `<version>` must match `^[0-9]+\.[0-9]+\.[0-9]+$` and be greater than the current tag from `gh release list`.
   - `git fetch origin`; require branch `main`, a clean tree, and `git rev-parse main origin/main` equal. Everything meant for the release is already merged.
   - CI on `origin/main` is green: `gh run list --branch main --workflow ci.yaml --limit 1 --json conclusion`.
   - No release run is in flight: `gh run list --workflow release.yml --limit 3 --json status`.
2. **Bump.** Set `version` under `[workspace.package]` in the root `Cargo.toml` to `<version>`. Every crate inherits it with `version.workspace = true` (since #635), so there is no per-crate edit. Run `cargo check --workspace` to refresh `Cargo.lock`, which updates each crate's entry, then `cargo test -p runner-app --test bundle_mac`. Commit `Cargo.toml` and `Cargo.lock` as `chore: bump version to <version>` and push `main`.
3. **Tag.** `git tag -a v<version> -m "Runner <version>" && git push origin v<version>`. The push starts `release.yml`.
4. **Watch.** Find the run with `gh run list --workflow release.yml --limit 1 --json databaseId,status,headSha` and `gh run watch <id> --exit-status`. All platform build jobs (including both Linux matrix instances) and the publish job must succeed. On failure, `gh run view <id> --log-failed`, report the failing step, and stop; the tag stays, the draft may be partial.
5. **Notes.** Follow `notes <version>` below and set them on the draft: `gh release edit v<version> --notes-file <file>`. Do not pass `--draft=false`.
6. **Report.** The draft URL, every platform asset name, and the reminder that publishing is the user's action. A draft's `untagged-…` URL changes each time it is edited, so link the latest one. After the user publishes, confirm `gh release view v<version> --json isDraft` is `false` before anything that assumes the release is live, then verify `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` is `v<version>` and that `releases/latest/download/appcast.xml` names the new DMG.
7. **Sweep.** Once the release is live, follow `sweep <version>` below.

## `notes <version>`

Release notes are **bilingual**: the full English text first, then the same content in 中文 inside a `<details>` block so the page reads as English-only until a reader expands it (GitHub has no tabs; a collapsed block is the nearest thing, and both in-app updaters open the release page in a browser where it renders). 0.8.3 used a horizontal rule instead; every release since 0.8.4 ships the collapsed form. A draft with only the workflow's one-line stub is not ready to publish.

Source the content from `git log v<previous>..v<version> --no-merges --format='- %s (%h)'` and the closed issues those commits reference. Write for users, not for the repo: what changed for them, in their words, one bullet per change with the issue number at the end. Skip internal work (tests, CI, docs, refactors) unless it changes what users see.

Structure, both languages:

```
Runner <version> is a <bug-fix | feature> release for macOS, Windows and Linux on top of <previous>.

## New                      (omit if empty)
- **Short bold lead.** One or two sentences. (#123)

## Bug fixes                (omit if empty)
- ...

## Nightly channel          (only when the nightly channel changed; nightly users read these)
- ...

## Download and upgrade
- **macOS, Apple Silicon:** download `Runner-<version>-arm64.dmg`, or update through Sparkle. The app is signed and notarized.
- **Windows x64, Windows 10 version 1809 or later:** download `Runner-Setup-<version>.<stamp>-x64.exe`. The installer, app, and CLI sidecars are Authenticode-signed by Open Source Developer Yicheng Wang. You can also update through Settings → Updates → Update, then choose Install and restart. Settings, chats, and missions are retained.

- **Linux ARM64/x86_64:** use `Runner-<version>-linux-<aarch64|x86_64>.tar.gz`, `.deb` or `.rpm`; install/update through the same package installer. The opt-in user service is disabled on install, and uninstall retains user data. Linux artifacts are unsigned in the initial implementation.

Windows ARM64 and Intel Macs are not supported. Linux ARM64/x86_64 downloads use tar/deb/rpm packages; there is no in-app Linux updater. State the actual build baseline and native validation gaps, and do not claim local artifacts have already been published.

**Full changelog:** https://github.com/yicheng47/runner/compare/v<previous>...v<version>

<details>
<summary>中文</summary>

Runner <version> 是 macOS、Windows 和 Linux 上基于 <previous> 的<修复版本 | 功能版本>。

## 新功能
...
## 问题修复
...
## Nightly 渠道
...
## 下载与升级
...

Windows ARM64 和 Intel Mac 暂不支持。Linux ARM64/x86_64 提供 tar/deb/rpm 包，不含应用内更新；说明实际构建基线与原生验证缺口，不把本地包写成已发布。

**完整变更记录：** https://github.com/yicheng47/runner/compare/v<previous>...v<version>

</details>
```

Keep a blank line after `<summary>` and before `</details>`, or GitHub renders the markdown inside as literal text.

Rules for the 中文 half: translate the meaning, not the words; keep product terms as they appear in the app (Runner, Sparkle, Nightly, ⌘, Settings → Updates); keep file names, issue numbers, and links identical to the English; use the same headings in the same order so a reader can line the two halves up. Headings inside the details block use `##` like the English half; they render collapsed until expanded. Read the exact Windows installer name from the draft's assets (`gh release view v<version> --json assets --jq '.assets[].name'`) rather than guessing the stamp.

## `sweep <version>`

Reviews and cleans up the docs of everything shipped since the previous sweep in one doc-only commit on `main`, so merges themselves add no docs commits (`AGENTS.md`, Docs Sweep At Release). Run it only once `v<version>` is published (`gh release view v<version> --json isDraft` is `false`): a `main` push while a release or nightly run is in flight cancels the CI run its publish job waits on.

1. **Preflight.** On `main`, clean tree, `git rev-parse main origin/main` equal after `git fetch origin`, and nothing in flight in `gh run list --workflow release.yml --limit 3 --json status` or the same for `nightly.yml`.
2. **Find what shipped.** Walk the in-progress docs rather than a commit range, so anything an earlier sweep missed is caught: each brief in `docs/impls/briefs/`, each test record directly under `docs/tests/` (not `full-smoke-test.md`), and each spec in `docs/features/`. Check each one's issue with `gh issue view <n> --json state,stateReason,closedAt` and its PRs with `gh pr list --state merged --search <n>`. Anything whose work is still open stays put.
3. **Test records.** Move each shipped record to `docs/tests/archive/` and promote its lasting live checks into `docs/tests/regression/` under that README's promotion rules, reconciled with current code, or have the record state why none apply. Run results stay separate from maintained cases.
4. **Briefs.** Prune the brief of every merged mission; git history keeps it. Keep the ones `docs/impls/briefs/README.md` lists as references, and the brief of any mission still running.
5. **Specs and plans.** Once an issue closes (after the last PR for a multi-PR feature), retain its spec or plan in `archive/` only if it explains lasting design decisions, migrations, compatibility constraints or useful implementation context. Preserve unique current decisions in `docs/arch/` and deferred scope in the issue, then prune obsolete or duplicate proposals after checking references and protecting uncommitted edits. Do not prune test evidence or task history under this rule. Update the relevant index to link to the retained record, implementation or issue; an archive entry is not required for every closed feature.
6. **Roadmap.** Record the release in `docs/roadmap.md`: latest release, the Releases table, milestone state and open-work counts, reconciled with GitHub, and move the snapshot date.
7. **Commit.** Repoint every link to a moved file and check the relative links with a script. Commit as `docs: record the <version> release and clean up its shipped docs`, push `main`, and report what was retained or pruned, the regression case IDs added, and anything left in place with the reason.

## `check <version>`

Read-only. `gh release view v<version> --json isDraft,isPrerelease,assets,body`: report draft state, that the DMG, `appcast.xml`, installer, and `.sig` are all present, and whether the body has the English section and a `<details>` block containing the 中文 section. After publishing, also verify `releases/latest` resolves to the tag and the production appcast's enclosure names the new DMG.

## Notes

- The workspace version stays at `X.Y.Z` after the release; nightlies do not bump it (see the `nightly` skill).
- `workflow_dispatch` of `release.yml` with `dry_run` builds the same artifacts into a throwaway draft named `dry-run-<stamp>` for inspection; delete that draft afterwards.
- Windows Authenticode signing shipped with #497: `release.yml` signs the installer, app and CLI sidecars through Certum SimplySign, so the notes carry the signed sentence above. If a release ever ships unsigned, say so in its notes and warn about SmartScreen.
- Apple credentials for local notarization checks live in `~/.zshrc`; `xcrun notarytool history` and `stapler validate <file>` are the tools.
