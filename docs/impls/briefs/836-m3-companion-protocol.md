# 836 m3 — companion protocol v1 (Phase 2)

Implement m3 of Phase 2 of [#836](https://github.com/yicheng47/runner/issues/836) ([#952](https://github.com/yicheng47/runner/issues/952)): the companion protocol's types and golden JSON files in `runner-core`. Jason asked for this codex duo mission on 2026-10-20, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m3-companion-protocol`, on `feat/836-m3-companion-protocol`, cut from `origin/nightly-remote` at `3dcc86c1`. No other branch, worktree or target directory. The branch has no upstream: push with `git push -u origin feat/836-m3-companion-protocol`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

Nothing serves the protocol yet: no channel, nothing in `runnerd`, the CLI or the app. Phase 3 does that. This mission builds the contract both repos build against.

## What matters most

- The golden files are the contract with the web panel, which decodes the same files in TypeScript. Every message the spec names has one, with the spec's names and shapes, and the types parse and produce each file unchanged.
- The compatibility rules hold in code and each is proven by a fixture: unknown fields ignored, fields added later defaulted, each enum's fallback, an unknown push decoded as unknown, and an unknown request answered with `unknownRequest` and its `id`.
- `runner-core` only, in `src/companion/`. No new dependency (`serde_json` and `base64` are already there), no change to `runnerd`, the CLI, the app or the client protocol, and no database migration.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`, Phase 2's Companion protocol section and Terminal size's Messages: the contract.** The spec wins over this brief on any detail.
- **`docs/impls/836-web-remote-control/plan.md`, "m3 — companion protocol v1": the commits, the checks and Easy to get wrong.**
- Code: `crates/runner-core/src/remote/mod.rs`'s `round_trip` test and `remote/fixtures/`, the pattern to follow; `protocol/status.rs` (`AgentStatus`, which status flattens from); `protocol/terminal.rs` and `wire.rs` (the local protocol's snapshot, `seq` and `TERMINAL_CHUNK`, for comparison only).

## Deliverables

The plan's commits, each building and passing its tests, and the validation record `docs/tests/836-m3-companion-protocol.md`. Fold the commit already on the branch (this brief) into the first commit.

Where the spec leaves a detail open, such as `hello`'s exact field names, the `result` of a request that returns nothing, `closed`'s reasons or an ask without choices, pick the plainest form, write it into the spec's Companion protocol section in the same PR, and list each choice in the PR body for Jason. If a choice would change something the spec states, ask the human through Runner instead of guessing.

Easy to get wrong:

- Decode a request in two steps, its `type` and `id` first, so an unknown type can still be answered with `unknownRequest` and that `id`.
- `serde(other)` covers only unit variants: an unknown push or status needs its own fallback variant, tested by its fixture.
- camelCase everywhere, enum values included: `needsYou`, `ifFree`, `takeControl`.
- An empty optional field is written as `null`, as the spec's examples show (`"before":null`, `"reason":null`). Apply it everywhere; the round trip pins it.
- Base64 is the standard alphabet with padding.
- Status flattening reads `lifecycle` first, then pending interactions, then activity: one test per row of the spec's table.
- The 1 MiB message limit and the 32 KiB chunk size are constants, with a chunking helper tested on empty input, exactly 32 KiB and one byte over.
- Tests do not sleep and reach no network. Gate anything used only by `#[cfg(unix)]` tests with `#[cfg(unix)]`.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`. m3 has no live check: nothing in it can be run by a person.

## Boundaries

The coder and reviewer never run the app, development or installed. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any other service. Do not touch other worktrees or the `runner_server` repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan and this brief, must-fix first with file:line pointers. Must-fix: a message or field missing, or shaped unlike the spec; a fixture the types do not reproduce exactly; a compatibility rule without a fixture; an unknown request that cannot be answered with its `id`; a change outside `runner-core` and the docs, or a new dependency; a test that sleeps or reaches a network. Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff.

After clean review, Jason authorizes: the commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers; `git push -u origin feat/836-m3-companion-protocol`, later `--force-with-lease`; `gh pr create --base nightly-remote` whose body says `Part of #952 (Phase 2, m3)`, not Closes, with the summary, checks with exit codes, the spec details chosen, review follow-ups and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, the review verdict, the spec details chosen, follow-ups and what is left for Jason. Then all slots stand by.
