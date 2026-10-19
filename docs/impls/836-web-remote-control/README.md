# 836 — web remote control: program record

Program record for [feature 836](../../features/836-web-remote-control.md) ([#836](https://github.com/yicheng47/runner/issues/836)). The spec says *what*; this directory says *how, in what order, and what has landed*. This file is the condensed state and the decisions that bind every mission. [plan.md](plan.md) has each phase in detail, and [impl_log.md](impl_log.md) is the dated log. Briefs go in [`../briefs/`](../briefs/) as `836-m{n}-{slug}.md`; Phase 1's predates this directory as `836-connect-host.md`. The design is `design/specs/836-web-remote-control.pen` on `main`.

## Status (2026-10-19)

Remote control is the focus of 0.17. #836 is the parent issue, with one sub-issue per phase.

| Mission | Scope | State |
| --- | --- | --- |
| m1 | Phase 1 ([#951](https://github.com/yicheng47/runner/issues/951)): connect a host to an account | Merged into `nightly-remote` on 2026-10-18 as [#945](https://github.com/yicheng47/runner/pull/945); #951 closed on 2026-10-19. The hosted-service pass on macOS covered connect, approve, pause, resume and disconnect; the other live checks are pending with Jason in [`836-connect-host.md`](../../tests/836-connect-host.md) and gate the umbrella's landing |
| m2 | Phase 2 ([#952](https://github.com/yicheng47/runner/issues/952)): host trust, the stable identity key, the PIN and its lock, and the device list | Running as a codex duo on `feat/836-m2-host-trust` since 2026-10-19; its Settings commit waits on the Pencil design |
| m3 | Phase 2 ([#952](https://github.com/yicheng47/runner/issues/952)): the companion protocol v1 | Waits on the spec's protocol section |
| — | Phase 3 ([#953](https://github.com/yicheng47/runner/issues/953)): the end-to-end channel, pairing, the adapter and live terminals, with [#838](https://github.com/yicheng47/runner/issues/838) | Planned when its development starts |
| — | Phase 4 ([#954](https://github.com/yicheng47/runner/issues/954)): reach a host over Tailscale | Planned when its development starts |

## Decisions that bind

- **Work lands on `nightly-remote`.** Each mission's worktree is cut from `origin/nightly-remote`, and its PR targets `nightly-remote`, which `ci.yaml` gates like `main`.
- **The umbrella follows `main` by rebase** (Jason, 2026-10-18), not by merging `main` in as `AGENTS.md`'s umbrella rule says: `git rebase origin/main`, then `git push --force-with-lease`, only while no mission branch is open against it.
- **Each phase lands as one commit** (Jason, 2026-10-19). A mission's pull request into `nightly-remote` is squash-merged, so a rebase replays one commit per mission, and a mission stays one unit that `git revert` undoes. Phase 1's [#945](https://github.com/yicheng47/runner/pull/945) was squashed in place on 2026-10-19; its five commits remain on the pull request.
- **Installed builds come from a separate release** (Jason, 2026-10-18; [#950](https://github.com/yicheng47/runner/pull/950)). `/nightly run remote` cuts `remote-nightly` from the umbrella, and `nightly` stays on `main`. Every channel shares the app's bundle id and data directory, so the umbrella adds no database migration: an install must be able to go back to `nightly` or production.
- **Both routes are released together** (Jason, 2026-10-18): Tailscale for users worldwide and the invite-only hosted service for mainland China. The phases are ordered by how easily each part is built and tested, so the relay comes first and Tailscale follows Phase 3.
- **The person decides who reaches their terminals** (Jason, 2026-10-19). The host trusts a device only after it has been paired with a PIN set at the host; the service decides whom it routes, and only the host decides whom it trusts. The spec's Client trust section has the model.
- **It reaches `main` as one PR** once both routes pass Jason's testing. Its briefs and test records stay on it until then. Landing retires the umbrella's CI trigger and the `remote-nightly` channel.

## Open

- Phase 1's remaining live checks, pending with Jason.
- m2: the Pencil frames of the PIN, its lock, the device list and Reset identity, which its Settings commit waits on.
- m3: the spec's protocol section.
