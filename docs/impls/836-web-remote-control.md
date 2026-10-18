# 836 — web remote control: program record

Program record for [feature 836](../features/836-web-remote-control.md) ([#836](https://github.com/yicheng47/runner/issues/836)). The spec says *what*; this file says how the work lands on the `nightly-remote` umbrella and how it is tested. Mission briefs go in [`briefs/`](./briefs/) as `836-{slug}.md`.

## Status (2026-10-18)

Phase 1's desktop side, connecting this computer to an account, merged into the umbrella on 2026-10-18 ([#945](https://github.com/yicheng47/runner/pull/945)). Its hosted-service pass on macOS covered connect, approve, pause, resume and disconnect. Still pending with Jason: Windows app enrollment, third-machine CLI enrollment, offline disconnect and its still-listed message, no reconnect after a daemon restart, fresh-key re-enrollment, removal from the web panel, and Keychain behavior across updates of a signed build. The encrypted channel, the companion protocol and terminals follow as later phases of the spec.

## The umbrella

- **Work lands on `nightly-remote`.** Each mission's worktree is cut from `origin/nightly-remote`, and its PR targets `nightly-remote`, which `ci.yaml` gates like `main`.
- **The umbrella follows `main` by rebase** (Jason, 2026-10-18), not by merging `main` in as `AGENTS.md`'s umbrella rule says: `git rebase --rebase-merges origin/main`, then `git push --force-with-lease`, only while no mission branch is open against it. `--rebase-merges` keeps each mission's merge commit, so a mission stays one unit that `git revert -m 1` undoes.
- **Installed builds come from a separate release** (Jason, 2026-10-18; [#950](https://github.com/yicheng47/runner/pull/950)). `/nightly run remote` cuts `remote-nightly` from the umbrella, and `nightly` stays on `main`. Every channel shares the app's bundle id and data directory, so the umbrella adds no database migration: an install must be able to go back to `nightly` or production.
- **It reaches `main` as one PR** when Jason decides the remote testing has passed. Its briefs and test records stay on it until then. Landing retires the umbrella's CI trigger and the `remote-nightly` channel.
