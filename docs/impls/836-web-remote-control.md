# 836 — web remote control: program record

Program record for [feature 836](../features/836-web-remote-control.md) ([#836](https://github.com/yicheng47/runner/issues/836)). The spec says *what*; this file says how the work lands on the `nightly-remote` umbrella and how it is tested. Mission briefs go in [`briefs/`](./briefs/) as `836-{slug}.md`.

## Status (2026-10-18)

Remote control is the focus of 0.17. [#836](https://github.com/yicheng47/runner/issues/836) is the parent issue, with one sub-issue per phase: [#951](https://github.com/yicheng47/runner/issues/951) connects a host, [#952](https://github.com/yicheng47/runner/issues/952) is the companion protocol and client trust, [#953](https://github.com/yicheng47/runner/issues/953) the end-to-end channel and live terminals (with [#838](https://github.com/yicheng47/runner/issues/838)), and [#954](https://github.com/yicheng47/runner/issues/954) the Tailscale route.

Phase 1's desktop side, connecting this computer to an account, merged into the umbrella on 2026-10-18 ([#945](https://github.com/yicheng47/runner/pull/945)). Its hosted-service pass on macOS covered connect, approve, pause, resume and disconnect. Still pending with Jason: Windows app enrollment, third-machine CLI enrollment, offline disconnect and its still-listed message, no reconnect after a daemon restart, fresh-key re-enrollment, removal from the web panel, and Keychain behavior across updates of a signed build. The companion protocol, the encrypted channel with terminals, and the Tailscale route follow as Phases 2 to 4 of the spec.

## The umbrella

- **Work lands on `nightly-remote`.** Each mission's worktree is cut from `origin/nightly-remote`, and its PR targets `nightly-remote`, which `ci.yaml` gates like `main`.
- **The umbrella follows `main` by rebase** (Jason, 2026-10-18), not by merging `main` in as `AGENTS.md`'s umbrella rule says: `git rebase --rebase-merges origin/main`, then `git push --force-with-lease`, only while no mission branch is open against it. `--rebase-merges` keeps each mission's merge commit, so a mission stays one unit that `git revert -m 1` undoes.
- **Installed builds come from a separate release** (Jason, 2026-10-18; [#950](https://github.com/yicheng47/runner/pull/950)). `/nightly run remote` cuts `remote-nightly` from the umbrella, and `nightly` stays on `main`. Every channel shares the app's bundle id and data directory, so the umbrella adds no database migration: an install must be able to go back to `nightly` or production.
- **Both routes are released together** (Jason, 2026-10-18): Tailscale for users worldwide and the invite-only hosted service for mainland China. The phases are ordered by how easily each part is built and tested, so the relay comes first and Tailscale follows Phase 3.
- **It reaches `main` as one PR** once both routes pass Jason's testing. Its briefs and test records stay on it until then. Landing retires the umbrella's CI trigger and the `remote-nightly` channel.
