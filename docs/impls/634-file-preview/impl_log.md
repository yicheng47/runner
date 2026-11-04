# 634 — implementation log

Dated entries for [the file preview program](README.md), newest last. Mission handoffs, decisions and measurements go here; the plan changes only when a decision changes how the work lands.

## 2026-11-04

- Jason settled the four questions left after the design was finished: `runnerd` serves the files so remote file access can follow; the chosen root is saved; the work runs as a phased program on an umbrella branch with its plan in this directory; and every platform is supported, with any Linux piece that proves hard postponed rather than holding the program.
- The umbrella is the existing `feat/634-file-preview`, which holds the spec. The `ci.yaml` trigger is its own commit, and the draft PR #1157 takes it into `main`. Jason chose to keep it current by rebase rather than by merging `main` in, and it was rebased onto `main` the same day, from 25 commits behind.
- Measured in the Runner repository with `target/` built: `git status --porcelain=v2 -z --untracked-files=all --ignored=matching` lists 6 ignored entries in 17 ms; `--ignored=traditional` lists 33,007.
- The plan splits the work into six missions in four phases. m1 and m2 can run in parallel.
