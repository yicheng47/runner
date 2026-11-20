# #1372 — refuse cross-mission posts, signals and asks from inside a mission

Jason approved `neon trio` on 2026-11-20. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1372-cross-mission-writes` on `fix/1372-cross-mission-writes` (base `origin/main` `12c867d2`, its own `target/`). Other worktrees under `.worktrees/` belong to other work; never inspect or edit them. A #1051 phase 4 mission may run in parallel in its own worktree; it touches runtime and session code, not `runner-cli`.

## Problem and outcome

From inside a mission, `runner msg post`, `runner signal` and `runner ask` accept `--mission` naming a different mission and deliver there. `run_local` (`crates/runner-cli/src/command.rs`) handles the command on the event log only when `same_mission` holds; any other `--mission` falls through to the daemon path. There, a missing `--as` makes the daemon record the sender as `human` (`post_message` in `crates/runner-daemon/src/ops/mission.rs`), so an agent in one mission can steer another mission as the person, and `--as <handle>` lets it post as any member of the other mission's roster. #1371 found an agent one command away from doing this to a running mission mid-handoff.

Outcome: when the caller is inside a mission (`BusContext::Mission`) and `--mission` names a different mission, these three commands fail with a usage error (exit 2) before anything is written. Off-bus callers (operator chats, scripts, people) keep `--mission` and `--as` unchanged.

## Read first

- Issue #1372 (expected behavior and reproduction).
- `crates/runner-cli/src/command.rs`: `run_local`, `same_mission`, `validate_remote` and its `add-role` / `remove-role` refusals, the existing pattern to follow.
- `crates/runner-cli/src/command/{msg.rs,ask.rs,shared.rs}` (`resolve_scoped_mission`), `crates/runner-cli/src/env.rs` (`BusContext`), `crates/runner-cli/src/command/cli.rs` (`--mission` help), `crates/runner-cli/src/help.rs` (agent guide CONTEXT block), and the refusal tests in `crates/runner-cli/src/command/tests.rs` (`role_change_flags_and_person_only_refusals`).

## Deliverables

1. **Refuse cross-mission writes inside a mission.** `msg post`, `signal` and `ask` (including `ask --human`) refuse a `--mission` that does not name the caller's mission, with or without `--as`, using the same test `run_local` uses (`same_mission`: the caller's mission ID starts with the given reference), so a reference to the caller's own mission keeps working exactly as today. The message names the command and the caller's mission by title, in the style of the add-role refusal, for example `runner msg post: you're in mission "<title>"; posting to another mission isn't allowed — ask the person`, with wording that fits `signal` and `ask`. Getting the title needs the daemon; if that lookup fails, name the mission by ID instead of failing differently. Nothing may reach the other mission's log or feed on the refusal path.
2. **Help and guide text.** Update the `--mission` help for the three commands (inside a mission, only the caller's own mission) and the agent guide's CONTEXT block in `help.rs` so agents learn the rule from `runner help agents`. Regenerate the CLI reference with `make guide-cli`, and update any `docs/guide/` page that describes posting with `--mission` from inside a mission, with its zh-CN page (`python3 script/check-guide-translations.py`).

Non-goals: mission lifecycle commands (`mission stop`, `archive`, `link` …) and `msg read` from inside a mission (the issue's open follow-up); daemon-side enforcement or a caller that deliberately unsets the `RUNNER_*` variables, since the CLI environment is the boundary today, as it is for add-role and remove-role.

## Validation and live checks

- **Automated:** unit tests beside the role refusals: inside a mission, each of `msg post`, `signal`, `ask` and `ask --human` with another mission's reference is refused with exit 2, with and without `--as`; the caller's own full ID and prefix still take the local path; off-bus calls with `--mission` and `--as` still pass validation. If the title lookup adds async code, cover its ID fallback too. Follow the no-race test rules; loop any new daemon-backed test 20 times with nextest stress mode and state the count. Run `cargo test -p runner-cli`, `make clippy`, `cargo fmt --all --check`, `git diff --check`, and confirm `make guide-cli` leaves no diff after regeneration.
- **Live QA (`@qa` after a clean review, macOS development app, QA lock):** use the development build's CLI with qa's own inherited `RUNNER_*` mission variables removed for off-bus commands. Start two bounded development missions, A and B, each with one cheap low-effort member in a fresh `/tmp` folder. From inside B's member session, have the member run: `runner msg post --mission <A> "hello"`, the same with `--as <A's handle>`, `runner signal --mission <A> …` and `runner ask --mission <A> …`; each is refused with the new message and A's feed shows nothing new. Then `runner msg post "hello"` from B still lands in B's feed from B's member. Off-bus, `runner msg post --mission <A> "hello"` still lands in A's feed from the person. Record results in `docs/tests/runs/1372-cross-mission-writes-mac.md`.

## Authorization

Approved crew: `neon trio`. Fold this brief into the fix commit, push and open a PR against `main` with `Fixes #1372`, `reviewed` from `@reviewer` on the final head, and the live-test label handled as AGENTS.md describes. Do not merge.
