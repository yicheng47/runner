---
name: release
description: Cut or check a Runner production release, write bilingual release notes, draft Chinese WeChat/linux.do and English Discord and X posts in the social repo, and sweep shipped docs. Publishing remains the user's action.
---

# Release

A production release is a `vX.Y.Z` tag on a commit whose workspace version is exactly `X.Y.Z`: a `main` commit, or for a [hotfix](#hotfix-version-pr) the tip of a `release/X.Y.Z` branch. `release.yml` builds the signed, notarized macOS DMGs for arm64 and x64, each with its own one-item Sparkle appcast (`appcast.xml` and `appcast-x64.xml`), and the Windows x64 installer with its minisign signature, plus Linux ARM64/x86_64 tar/deb packages, then attaches everything to a **draft** release. Publishing the draft is the user's switch: it moves `releases/latest` and the production Sparkle feed. Contract: `docs/arch/arch.md` §14; nightlies are a separate channel handled by the `nightly` skill.

An explicit request to `run` authorizes the bump commit, the push, the tag, and editing the draft's notes; after the user publishes, it also authorizes the sweep's docs commit and push. An explicit `hotfix` request authorizes the same, plus the release branch, its worktree, the cherry-picks and pushing the branch. An explicit `sweep` request authorizes that commit and push on its own. Nothing in this skill publishes the draft, deletes a release, moves an existing tag, or posts to a forum or social media. Record only observed results.

## Usage

`/release [run <version> | hotfix <version> <pr>… | notes <version> | post <version> | check <version> | sweep <version>]` — no action means `run`.

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
6. **Report.** The draft URL, every platform asset name, and the reminder that publishing is the user's action. A draft's `untagged-…` URL changes each time it is edited, so link the latest one. After the user publishes, confirm `gh release view v<version> --json isDraft` is `false` before anything that assumes the release is live, then verify `gh api repos/yicheng47/runner/releases/latest --jq .tag_name` is `v<version>` and that `releases/latest/download/appcast.xml` and `appcast-x64.xml` name the new arm64 and x64 DMGs.
7. **Posts.** Once the release is live, follow `post <version>` below to draft the WeChat/linux.do, Discord and X posts and record all three in the social repo.
8. **Sweep.** Then follow `sweep <version>` below.

## `hotfix <version> <pr>…`

The exception to releasing from `main`, for when something bad has shipped and `main` holds unreleased work that must not go out with the fix (Jason, 2026-11-13). Every other release is `run`. Each fix lands on `main` first through its own reviewed PR; the hotfix carries those PRs onto the release being patched. A change that only makes sense on the old release, such as a revert that `main` has already moved past, is committed on the branch instead and reviewed like any other fix before it is tagged.

1. **Preflight — stop on any failure and report it.** `<version>` passes the `run` checks, and `<previous>` is the current tag from `gh release list`, the release being patched. Each `<pr>` is merged (`gh pr view <pr> --json state,mergeCommit`) and CI on its merge commit is green (`gh run list --commit <merge> --workflow ci.yaml --json conclusion`). No release run is in flight.
2. **Branch.** From the root checkout, `git fetch origin && git worktree add .worktrees/release-<version> -b release/<version> v<previous>`. In the worktree, for each PR in merge order, `git cherry-pick -x <merge>^1..<merge>^2`, which brings over the PR's own commits. Stop and report a conflict: the fix depends on work being held back, and a hand-made backport needs its own review.
3. **Bump.** As `run` step 2, in the worktree, committing on the branch; the worktree builds into its own `target/`. Push with `git push -u origin release/<version>`. CI runs on `release/**` pushes, and `release.yml` waits for that run before it creates the draft.
4. **Tag and finish.** Tag the branch tip as `run` step 3, then follow `run` steps 4–8 unchanged; the sweep still runs on `main`.
5. **Afterwards.** `main` keeps its own workspace version, and the next `run` from `main` bumps past `<version>`. Once the release is published, the tag keeps the hotfix commits; ask before removing the worktree and the `release/<version>` branch. A second hotfix before `main` is shippable again branches from this hotfix's tag the same way.

## `notes <version>`

Release notes are **bilingual**: the full English text first, then the same content in 中文 inside a `<details>` block so the page reads as English-only until a reader expands it (GitHub has no tabs; a collapsed block is the nearest thing, and the in-app updaters open the release page in a browser where it renders). 0.8.3 used a horizontal rule instead; every release since 0.8.4 ships the collapsed form. A draft with only the workflow's one-line stub is not ready to publish.

Source the content from `git log v<previous>..v<version> --no-merges --format='- %s (%h)'` and the closed issues those commits reference. When `<previous>` was a hotfix, `main` reaches it only through the cherry-picks, so use `git log --cherry-pick --right-only --no-merges --format='- %s (%h)' v<previous>...v<version>` to leave out the fixes it already shipped. Write for users, not for the repo: what changed for them, in their words, one bullet per change with the issue number at the end. Skip internal work (tests, CI, docs, refactors) unless it changes what users see.

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
- **macOS:** download `Runner-<version>-arm64.dmg` for Apple Silicon or `Runner-<version>-x64.dmg` for Intel, or update through Sparkle. Both apps are signed and notarized.
- **Windows x64, Windows 10 version 1809 or later:** download `Runner-Setup-<version>.<stamp>-x64.exe`. The installer, app, and CLI sidecars are Authenticode-signed by Open Source Developer Yicheng Wang. You can also update through Settings → Updates → Update, then choose Install and restart. Settings, chats, and missions are retained.

- **Linux ARM64/x86_64:** use `Runner_<version>_linux-<arm64|x64>.tar.gz` or `.deb`; install/update through the same package installer. The opt-in user service is disabled on install, and uninstall retains user data. Each Linux package carries a matching minisign signature verified by the in-app updater.

Windows ARM64 is not supported. The Intel Mac build is cross-compiled and not yet verified on Intel hardware. Linux ARM64/x86_64 downloads use tar/deb packages; signed in-app updates support managed tar installations and system-authorized APT installation. State the actual build baseline and native validation gaps, and do not claim local artifacts have already been published.

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

Windows ARM64 暂不支持；Intel Mac 版本为交叉编译，尚未在 Intel 硬件上验证。Linux ARM64/x86_64 提供 tar/deb 包及对应 minisign 签名，支持应用内验证更新：用户 tar 安装原子替换，deb 包经授权由 APT 安装；说明实际构建基线与原生验证缺口，不把本地包写成已发布。

**完整变更记录：** https://github.com/yicheng47/runner/compare/v<previous>...v<version>

</details>
```

Keep a blank line after `<summary>` and before `</details>`, or GitHub renders the markdown inside as literal text.

Rules for the 中文 half: translate the meaning, not the words; keep product terms as they appear in the app (Runner, Sparkle, Nightly, ⌘, Settings → Updates); keep file names, issue numbers, and links identical to the English; use the same headings in the same order so a reader can line the two halves up. Headings inside the details block use `##` like the English half; they render collapsed until expanded. Read the exact Windows installer name from the draft's assets (`gh release view v<version> --json assets --jq '.assets[].name'`) rather than guessing the stamp.

## `post <version>`

Draft three posts: one Chinese plain-text post shared by Runner's WeChat user group and its linux.do thread (Jason, 2026-11-05: one post for both, no separate markdown version), one English post for Runner's Discord, and one English release post for X (Jason, 2026-11-12). Record all three in `~/repos/yicheng47/social`. Read that repo's `AGENTS.md`, relevant platform README files and voice notes first, including `x/AGENTS.md`, `x/playbook.md` and `x/voice.md` for X; check for existing drafts for the version and preserve Jason's edits.

Confirm the release is published (`gh release view v<version> --json isDraft` is `false`), then read its body with `gh release view v<version> --json body`. Build the Chinese post from the 中文 half and the Discord and X posts from the English half so all three agree with the release page. Keep the changes users notice; omit issue numbers, internal work and minor polish. Include Nightly changes only when the release is about that channel. Use the published release URL, which linux.do can render as a preview card.

### WeChat / linux.do

Use this plain-text shape, omitting empty sections:

```text
【Runner <version> 发布】
macOS / Windows / Linux 均已上线，应用内 Settings → Updates 即可更新，设置、聊天和 mission 都会保留。

✨ 新功能
1. 终端设置重新分组，新增行高、光标闪烁、回滚行数、选中即复制四项设置
2. ……

🛠 问题修复
1. Windows / Linux：Ctrl+D、Ctrl+K、Ctrl+S、Ctrl+M 在终端里不再被 Runner 占用……
2. ……

完整更新说明：
https://github.com/yicheng47/runner/releases/tag/v<version>

有问题欢迎在群里反馈 🙏
```

- Plain text only: WeChat renders no markdown, so no `#` headings, `**bold**` or `[links](…)`. Number the items under the ✨ 新功能 and 🛠 问题修复 lines; omit a section that is empty.
- Shorten each 中文 bullet to one line in plain words, keeping product terms as the app shows them.
- The release URL sits bare on its own line, which linux.do renders as a card and WeChat as a link.

### Discord

Write a concise English announcement for existing Runner users: version, supported platforms and update path, followed by the notable new features and fixes, the release URL on its own line, and a short invitation for feedback. Light Discord Markdown is fine: a bold title and short bullet lists. Keep the complete post within 2,000 characters so it fits in one ordinary message. Preserve product labels and relevant qualifications from the English release notes; do not add personal anecdotes or promotional claims.

### X

Write one English plain-text post within 280 weighted characters, counting each URL as 23 and CJK characters as 2. Lead with the version and a useful change, explaining what it lets a user do; include other highlights only if they fit comfortably. Put the published release URL on its own line. Keep the wording concrete and grounded in the release notes, without invented personal experience. Check and record the weighted length. This release workflow includes original-post drafting for X; publication remains the user's action.

### Save and hand off

- Canonical Chinese draft: `social/references/platforms/wechat/runner-<version>-user-group.md`, shared with linux.do by reference rather than a duplicate post file.
- Canonical Discord draft: `social/references/platforms/discord/runner-<version>-release.md`.
- Canonical X draft: `social/x/drafts/runner-<version>-release.md`. Follow the existing original-post format: draft date and status, agent authorship, weighted length, a copyable `text` fence, source release URL and space for Jason's response. Link it from `social/x/runner/README.md`.
- For WeChat/linux.do and Discord, follow the existing draft metadata pattern: date, `status: draft`, intended venue(s), language, agent authorship and source release URL, then the copyable post body. Update the platform README indexes and link the shared Chinese draft from `social/linux-do/README.md`. A draft is not evidence of publication; preserve published copy if one already exists.
- Also export the bodies without metadata to `/tmp/runner-v<version>-release-note.zh-CN.txt`, `/tmp/runner-v<version>-release-note.en.md` (Discord) and `/tmp/runner-v<version>-release-note.x.en.txt` for easy copying. The social repo is the durable record; `/tmp` is only a convenience copy.
- Review the social diff and follow its standing commit/push authorization, preserving unrelated edits. This does not authorize git writes in the Runner repo.
- Show the requested platform or language in the reply (all three posts when neither is specified), with `text` fences for Chinese and X, and link all three saved drafts. Do not post to any platform; the user does that.

## `sweep <version>`

Reviews and cleans up the docs of everything shipped since the previous sweep in one doc-only commit on `main`, so merges themselves add no docs commits (`AGENTS.md`, Docs Sweep At Release). Run it only once `v<version>` is published (`gh release view v<version> --json isDraft` is `false`): a `main` push while a release or nightly run is in flight cancels the CI run its publish job waits on.

1. **Preflight.** On `main`, clean tree, `git rev-parse main origin/main` equal after `git fetch origin`, and nothing in flight in `gh run list --workflow release.yml --limit 3 --json status` or the same for `nightly.yml`.
2. **Find what shipped.** Read `docs/README.md`'s Archive policy and the relevant archive README first; do not batch-read archived files for orientation. Walk the in-progress docs rather than a commit range, so anything an earlier sweep missed is caught: each brief in `docs/impls/briefs/`, each test record directly under `docs/tests/` (excluding `README.md` and `full-smoke-test.md`), and each spec in `docs/features/`. Check each one's issue with `gh issue view <n> --json state,stateReason,closedAt` and its PRs with `gh pr list --state merged --search <n>`. Anything whose work is still open stays put.
3. **Test records.** Move each shipped record to `docs/tests/archive/` and promote its lasting live checks into `docs/tests/regression/` under that README's promotion rules, reconciled with current code, or have the record state why none apply. Run results stay separate from maintained cases.
4. **Briefs.** Prune the brief of every merged mission; git history keeps it. Keep the ones `docs/impls/briefs/README.md` lists as references, and the brief of any mission still running.
5. **Specs and plans.** Once an issue closes (after the last PR for a multi-PR feature), delete completed small-feature, bug-fix, UI-polish and routine-refactor specs/plans. Retain only important features, architectural changes and consequential migration/platform/runtime/security contracts under `docs/README.md`'s Archive policy, with a concrete retention reason in the archive README. Avoid duplicate spec/plan coverage; a second record needs distinct lasting context. Preserve unique current decisions in `docs/arch/` and deferred scope in the issue; preserve embedded validation results and task history separately before pruning planning text. Do not prune test/run evidence or task/review logs. Repair references to current docs/code or fixed commit URLs, protect uncommitted user edits and update the indexes.
6. **Roadmap.** Record the release in `docs/roadmap.md`: latest release, the Releases table, milestone state and open-work counts, reconciled with GitHub, and move the snapshot date.
7. **Commit.** Repoint every link to a moved file and check the relative links with a script. Commit as `docs: record the <version> release and clean up its shipped docs`, push `main`, and report what was retained or pruned, the regression case IDs added, and anything left in place with the reason.

## `check <version>`

Read-only. `gh release view v<version> --json isDraft,isPrerelease,assets,body`: report draft state, that both DMGs, `appcast.xml`, `appcast-x64.xml`, installer, and `.sig` are all present, and whether the body has the English section and a `<details>` block containing the 中文 section. After publishing, also verify `releases/latest` resolves to the tag and each production appcast's enclosure names the new DMG of its architecture.

## Notes

- The workspace version stays at `X.Y.Z` after the release; nightlies do not bump it (see the `nightly` skill).
- `workflow_dispatch` of `release.yml` with `dry_run` builds the same artifacts into a throwaway draft named `dry-run-<stamp>` for inspection; delete that draft afterwards.
- Windows Authenticode signing shipped with #497: `release.yml` signs the installer, app and CLI sidecars through Certum SimplySign, so the notes carry the signed sentence above. If a release ever ships unsigned, say so in its notes and warn about SmartScreen.
- Apple credentials for local notarization checks live in `~/.zshrc`; `xcrun notarytool history` and `stapler validate <file>` are the tools.
