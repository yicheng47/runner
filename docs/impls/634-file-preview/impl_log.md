# 634 — implementation log

Dated entries for [the file preview program](README.md), newest last. Mission handoffs, decisions and measurements go here; the plan changes only when a decision changes how the work lands.

## 2026-11-04

- Jason settled the four questions left after the design was finished: `runnerd` serves the files so remote file access can follow; the chosen root is saved; the work runs as a phased program on an umbrella branch with its plan in this directory; and every platform is supported, with any Linux piece that proves hard postponed rather than holding the program.
- The umbrella is the existing `feat/634-file-preview`, which holds the spec. The `ci.yaml` trigger is its own commit, and the draft PR #1157 takes it into `main`. Jason chose to keep it current by rebase rather than by merging `main` in, and it was rebased onto `main` the same day, from 25 commits behind.
- Measured in the Runner repository with `target/` built: `git status --porcelain=v2 -z --untracked-files=all --ignored=matching` lists 6 ignored entries in 17 ms; `--ignored=traditional` lists 33,007.
- The plan splits the work into six missions in four phases. m1 and m2 can run in parallel.
- m2 (#1161) and m1 (#1164) merged into the umbrella. m2's live checks were moved into m3's.
- m3 (#1168) was reviewed clean, then live-tested by `qa` in a claude-duo mission. qa found one failure: a double-click on the panel's collapse control zoomed the window. The coder fixed it, the reviewer passed the fix, and qa passed it on the chat, terminal-only, mixed and mission surfaces. Jason narrowed the live scope during the run: dark theme only, and no checks needing the real pointer or remote access. The hosted macOS job twice failed to get a runner (GitHub capacity), so m3 merged on local macOS fmt, clippy and nextest results, recorded on the PR. The repository allows only merge commits, so m3 reached the umbrella as a fast-forward; the plan's Shape now says so.
- Jason handed the program's day-to-day decisions and the umbrella merges to the operator chat. The umbrella was rebased onto `main` with no conflicts, picking up cb800c08, which keeps the Claude launch tests off the shell's `CLAUDE_CONFIG_DIR`.

