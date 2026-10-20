# 836 m4 — the channel (Phase 3)

Implement m4 of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): the end-to-end channel that pairs a browser with the host's PIN and encrypts everything after, carried over the host's relay connection. Jason asked for this codex duo mission on 2026-10-20, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m4-channel`, on `feat/836-m4-channel`, cut from `origin/nightly-remote` at `3b4eb45c`. No other branch, worktree or target directory. Push with `git push -u origin feat/836-m4-channel`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

The adapter that serves the companion protocol is m5, and the desktop's control card, the pairing notification and the Connected now row are m6. This mission ends at the companion handshake: a paired browser gets `welcome`, and every request after it gets `unknownRequest`.

## What matters most

- The channel is a contract with the web panel, which implements the browser's side in TypeScript from the spec and replays this mission's vectors. Frames, CPace inputs, Noise patterns, prologues, payloads, records and credit follow the spec exactly, and each is pinned by a vector.
- The service is untrusted. It must learn neither the PIN nor anything to test guesses against, and must get no more than five wrong PINs in a row, parallel attempts included. Jason clarified on 2026-10-20 that every structurally valid `pairFinish` authentication failure counts, including tampering, which cannot be distinguished from a wrong PIN. Malformed frames do not count.
- With remote access off, nothing changes: no new network, no new work at start-up, nothing in the session manager, the terminal or the app. A channel's failure ends that channel only, never the remote task, `runnerd` or its sessions.
- No production change to `runner-app`; Jason approved the single `connected: false` initializer needed by its existing test fixture on 2026-10-20. No new crate in `Cargo.lock`: `curve25519-dalek` and `sha2` become direct dependencies at the versions `snow` already brings in. No database migration.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`, Phase 3's Route, Channel and Records sections: the contract.** Phase 2's Client trust and Companion protocol for the context. The spec wins over this brief on any detail.
- **`docs/impls/836-web-remote-control/plan.md`, "m4 — the channel": the commits, the checks and Easy to get wrong.**
- CPace: [draft-irtf-cfrg-cpace-21](https://datatracker.ietf.org/doc/draft-irtf-cfrg-cpace/), section 8.3 (CPACE-RISTR255-SHA512) and appendix B.3's vectors.
- Code: `runner-daemon/src/remote/` (`mod.rs`'s socket loop, `transport.rs`, `trust.rs`, `store.rs` and the fake service in `tests.rs`); `runner-core/src/remote/` (`Control`, `HostKey`, the fixtures); `runner-core/src/companion/` (m3's messages, which the handshake's `hello` and `welcome` use).

## Deliverables

The plan's commits, each building and passing its tests, and the validation record `docs/tests/836-m4-channel.md`. Fold the commit already on the branch (this brief, with the spec's Phase 3 section and the plan) into the first commit.

Where the spec leaves a detail open, such as the vector files' layout, pick the plainest form, write it into the spec in the same PR, and list each choice in the PR body for Jason. Jason approved Phase 3 as drafted without reviewing every detail: if implementing it shows a part of the spec to be wrong, unsafe or unimplementable in a browser, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`; and `make guide-cli` if CLI help changed. m4 has no live check: until the service's browser channel exists, nothing can open a channel to a real host.

## Boundaries

The coder and reviewer never run the app, development or installed. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any other service; tests reach no network and do not sleep. Do not touch other worktrees or the `runner_server` repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan and this brief, must-fix first with file:line pointers. Must-fix: a frame, input, pattern or record unlike the spec; a vector the code does not reproduce from both sides; anything that lets the relay learn or test the PIN, or exceed five wrong PINs; a malformed frame counted as a wrong PIN; one channel able to block the socket, the heartbeat or another channel; a PIN, ISK, key or body in a log or `Debug`; a behavior change with remote access off; a change to `runner-app` beyond that approved test-fixture initializer, a new crate in the lockfile or a migration; a test that sleeps or reaches a network. Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff.

After clean review, Jason authorizes: the commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers; `git push -u origin feat/836-m4-channel`, later `--force-with-lease`; `gh pr create --base nightly-remote` whose body says `Part of #953 (Phase 3, m4)`, not Closes, with the summary, checks with exit codes, the spec details chosen, review follow-ups and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, the review verdict, the spec details chosen, follow-ups and what is left for Jason. Then all slots stand by.
