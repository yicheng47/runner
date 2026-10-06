# runnerd — program record

Implementation record for [feature 645](../../features/645-session-host.md) ([#645](https://github.com/yicheng47/runner/issues/645)). The spec says *what*; this directory says *how, in what order, and what has landed*. This file is the condensed state and the decisions that bind. [plan.md](plan.md) has the phases, the four phase 1 missions in detail and the #709 evaluation; [impl_log.md](impl_log.md) is the dated log. Briefs go in [`../briefs/`](../briefs/) as `645-m{n}-{slug}.md`.

## Status (2026-10-06)

1a, 1b and 1c are on the umbrella: 1a ([#805](https://github.com/yicheng47/runner/pull/805)) and 1b ([#806](https://github.com/yicheng47/runner/pull/806)) merged on 2026-10-05, and 1c ([#807](https://github.com/yicheng47/runner/pull/807)) on 2026-10-06 after review, CI on both platforms and QA's live pass on the Mac. 1c's open checks are Jason's: IME, clipboard paste and ⌘-click links on the Mac, and the Windows list and named-pipe benchmark on the PC. The 1d design was drawn on 2026-10-06. Next: the 1c nightly, then the 1d brief. There is no downgrade guard (Jason, 2026-10-06).

The phases: (1) the local daemon, four missions, which gates 0.13.0; (2) dropped on 2026-10-06, because an update restarts every session; (3) remote machines over ssh, which builds the session daemon and its versioned protocol; (4) the Windows PC.

## Decisions that bind

- **The state owner runs without the UI.** `runnerd` runs `AppCore`, and the app, the CLI and any later client are its clients (spec decision 1).
- **The live terminal path is unchanged.** The agent's bytes reach the app untouched, the mirror parses them with the same `alacritty_terminal` code the daemon uses, and everything except input and output stays local to the app. Snapshots happen only on reconnect.
- **The upstream alacritty event loop is not the engine**, because a daemon must forward raw bytes and it does not expose them (plan, "#709 does not go first").
- **Alacritty gets a read-only accessor patch** (Jason, 2026-10-05), on a vendored copy during phase 1, so the snapshot reads the exact state. A fork comes only if the accessors go upstream.
- **`runnerd` keeps running once started.** There is no idle exit, and with no client connected it pauses UI-only background work.
- **An update restarts every session** (Jason, 2026-10-06; spec decision 13). The update dialog shows when agents are working ("1 agent working" beside its buttons), and a manually installed build restarts the sessions with a notice instead of a dialog. Phase 2 is dropped.
- **Quitting** (1d design, 2026-10-06): one "When Runner quits" setting, a dialog with two choices (Keep them running, Stop them) plus "Don't ask again", and ⌥⌘Q as Quit and Stop Sessions.
- **The boundary is a crate boundary.** A new `protocol` module in `runner-core` holds everything that crosses the socket (1a). The app loses its normal dependency on the backend (1c), and the backend crate becomes `runner-daemon` (1d).
- **The four phase 1 missions run in order.** 1a and 1b run with no other `runner-app` mission in flight.
- **No downgrade, fix forward.** Jason's Mac takes every umbrella cut from the first one, with his data directory backed up first. Problems are fixed on the umbrella. There is no downgrade guard: 0.13.x has to be good enough that nobody goes back, and `runnerd` shuts down cleanly if an older app takes its sockets. Checks A to D (plan, "The checks before landing") gate the landing on `main`, not the install.
- **Phase 1 lives on `feat/645-runnerd` and ships on the regular `nightly`.** Missions merge into the umbrella; the umbrella follows `main` by rebase, adds no schema change, and lands on `main` once, after the daily-driving gate. There are no other nightly users, so the umbrella's cuts go to `nightly` and no separate channel is built (Jason, 2026-10-05); while the umbrella lives, every nightly is cut from it. Its mission briefs and test records stay on it until it lands.

## Open

- How long the daily-driving gate runs (proposed: one week).
- Whether to close #795 as covered by the spec. #709 was closed on 2026-10-05.
- The issue body's Shape section and title predate `runnerd`. The spec file is still named `645-session-host.md`.
