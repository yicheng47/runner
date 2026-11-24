# Contributing to Runner

Pull requests written by AI coding agents are welcome. This page is addressed to the agent doing the work and to the person who sends the pull request. [AGENTS.md](AGENTS.md) is the repo's rulebook; read it before starting. A pull request that skips its rules or arrives untested is sent back.

## Before you start

- **A visual change needs an issue first.** Anything that changes what the app looks like (layout, spacing, colour, icons, on-screen copy, a new control) starts as an issue that describes the change with a screenshot or mockup, and the pull request links it with `Closes #<n>`. Runner's UI follows designs settled before code, so the design is decided in the issue rather than in review. A visual pull request without a linked issue is not reviewed.
- For other changes, link the issue when one exists.
- One change per pull request. No unrelated refactors, formatting sweeps or dependency bumps riding along.
- Runner's design files are kept by the maintainer in a separate, private design repo; a pull request does not change them.

## Which parts of AGENTS.md apply

All of it, except the maintainer's own workflow: Worktrees, Crew Missions, Post-mission cleanup, Docs Sweep At Release, and committing docs straight to `main`. Work on a branch of your fork, and send every change as a pull request, docs included.

## Fully tested

A pull request is ready for review when it proves the change works:

1. **The automated gates pass.** Run `make verify` (check, test, Clippy and format check), or the CI commands listed under Development Commands in AGENTS.md; on Windows use the commands in [Windows validation](docs/arch/windows.md#validation). CI must then be green on both macOS and Windows. Windows CI often fails on imports or helpers used only by `cfg(unix)` tests; gate them with `#[cfg(unix)]`.
2. **Changed behavior has tests.** A fix comes with a test that fails without it wherever the behavior can be tested automatically.
3. **Live checks follow [Test Scope](AGENTS.md#test-scope).** Any UI change, or any change whose behavior can be tested by clicking or operating the running app's UI (including large refactors and logic already covered by in-process tests), requires live checks in the development app (`make run`); only small refactors, test-only fixes, documentation, and non-UI changes with no app-UI surface skip them. List the behaviors your diff changes, choose the checks that cover them from the [live regression suite](docs/tests/regression/README.md) plus your own, give the reason for each, and run them.
4. **The pull request body carries the evidence:** each command with its exit code, each live check with its result, before and after screenshots of any UI change on every platform it touches, and anything you could not verify, with the reason. A check without evidence counts as not run.

## Pull request shape

- Branch from current `main`. To update, rebase onto `main` and push with `--force-with-lease`; never merge `main` into the branch.
- Focused commits with imperative subjects and the scopes listed in AGENTS.md, such as `fix(session): preserve terminal geometry on tab switch`. Fold review fixes into the commits they correct rather than adding "address review" commits. Pull requests land as merge commits, so your commits reach `main` as you wrote them.
- `README.md` and `README.zh-CN.md` change together.
- No agent session links or transcripts in the body.
