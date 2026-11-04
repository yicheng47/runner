# 634 file preview — program record

Implementation record for [feature 634](../../features/634-file-preview.md) ([#634](https://github.com/yicheng47/runner/issues/634)). The spec says *what*; this directory says *how, in what order, and what has landed*. This file is the condensed state and the decisions that bind. [plan.md](plan.md) has the phases, each mission's scope, checks and landing, and the gate before the umbrella reaches `main`; [impl_log.md](impl_log.md) is the dated log. Briefs go in [`../briefs/`](../briefs/) as `634-m{n}-{slug}.md`, written on each mission's branch when it starts.

## Status (2026-11-04)

The spec and design are complete: the design was signed off board by board on 2026-11-01 and 2026-11-03 in `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design`, and the spec was narrowed to the file preview on 2026-11-03, with the diff and review moved to [#1141](https://github.com/yicheng47/runner/issues/1141). This plan was written on 2026-11-04 after Jason settled where the work runs, where the chosen root is saved, the umbrella and the platforms. m1, m2 and m3 merged into the umbrella on 2026-11-04; m4 is next.

The umbrella is `feat/634-file-preview` in `.worktrees/feat-634-file-preview`, which already holds the spec. Six missions in four phases merge into it, and it reaches `main` as one pull request, the draft [#1157](https://github.com/yicheng47/runner/pull/1157), once the gate in the plan passes.

| Mission | Phase | Branch | Waits on | Pull request | State |
| --- | --- | --- | --- | --- | --- |
| m1 — runnerd file service | 1 | `feat/634-m1-files-service` | — | [#1164](https://github.com/yicheng47/runner/pull/1164) | Merged 2026-11-04 |
| m2 — side panel views | 2 | `feat/634-m2-panel-views` | — (runs beside m1) | [#1161](https://github.com/yicheng47/runner/pull/1161) | Merged 2026-11-04; its live checks ran in m3 |
| m3 — the Files view | 2 | `feat/634-m3-files-tree` | m1, m2 | [#1168](https://github.com/yicheng47/runner/pull/1168) | Merged 2026-11-04 |
| m4 — the file viewer | 3 | `feat/634-m4-file-viewer` | m3 | — | Not started |
| m5 — highlighting and Markdown | 3 | `feat/634-m5-highlight-markdown` | m4; its spike can run any time | — | Not started |
| m6 — Settings → Side panel | 4 | `feat/634-m6-side-panel-settings` | m5 | — | Not started |

## Decisions that bind

- **runnerd serves the files** (Jason, 2026-11-04). Resolving a session's root, listing folders, `git status`, the worktree list, reading a file and watching for changes all run in `runnerd` behind client-protocol requests and pushed events, so remote file access can later be served by the other computer's `runnerd` without a new model in the app. The app draws the tree and the file, highlights code and parses Markdown; `runnerd` carries no grammars and the protocol carries plain bytes.
- **The chosen root is saved by runnerd** (Jason, 2026-11-04): per chat, terminal and mission, in SQLite, so it survives restarts and follows the session's row. The recent-folder list is saved beside it.
- **One umbrella, merged as one PR** (Jason, 2026-11-04). Each mission branches from the umbrella and opens its PR against it, which merges as a rebase merge. The umbrella follows `main` by rebase, not by merging `main` in as AGENTS.md describes for umbrellas, and lands on `main` through #1157 with a merge commit.
- **Every platform** (Jason, 2026-11-04). macOS, Windows and Linux build and run every phase. A Linux piece that proves hard ships with the fallback named in the plan's Linux section and a follow-up issue rather than holding the umbrella.
- **The side panel's view contract is internal** (Jason on #1072, 2026-10-28). Files is the first view built on it, and Info and Sessions move onto it in m2. Third-party panel plugins stay deferred.
- **Ignored entries come from git.** `git status --porcelain=v2 -z --untracked-files=all --ignored=matching` reports each ignored folder once: in the Runner repository with `target/` built it lists 6 ignored entries in 17 ms, against 33,007 with `--ignored=traditional` (measured 2026-11-04). The same set tells the watcher which events to drop. No gitignore crate is added.
- **The operator drives the program** (Jason, 2026-11-04). The chat that runs the missions decides the crews' questions itself, merges each mission PR into the umbrella once review, CI and qa pass, rebases the umbrella onto `main` and starts the next mission, reporting each decision afterwards. The landing PR #1157 into `main` stays Jason's.
- **Live checks before the gate are scoped down** (Jason, 2026-11-04). Mission qa runs on macOS in the dark theme only; checks that need the real pointer (drags, resizing, hover-only states), remote access and the light theme are recorded as deferred, not run, and do not hold `needs-live-test`. The gate's own Verification run still covers them.
- **No CLI surface.** Agents already read files; the new requests are not exposed as `runner` commands.

## Open

- **Markdown beyond the spec's list.** The spec names headings, lists, code blocks, links and local images. Tables, task lists and inline HTML are common in READMEs: render the first two and show inline HTML as text, or keep strictly to the spec? Decide before m5.
- **Linux's reveal row.** Linux has no common call that selects a file in the file manager; the plan proposes "Open containing folder" through `xdg-open`, as the mission rail does today. The design draws only the macOS and Windows labels.
- **Daily use before landing.** Whether Jason runs an umbrella build day to day before the landing PR, as #645 planned, or the gate's live checks are enough.
