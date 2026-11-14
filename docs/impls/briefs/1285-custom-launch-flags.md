# Mission Brief — #1285 Custom launch flags per agent

- **Issue:** [#1285](https://github.com/yicheng47/runner/issues/1285) (`feature`, `P2`).
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1285-custom-launch-flags`, branch `feat/1285-custom-launch-flags`, off `origin/main` `d08468fc`. Work only in this worktree, with its own `target/`.
- **Spec:** [`docs/features/1285-custom-launch-flags.md`](../../features/1285-custom-launch-flags.md), which wins on every detail. Design: `runner/desktop/specs/1285-custom-launch-flags.pen` in `YeelinksCo/yeelinks_design`, revision `cc9a437`; frames `k5Euq` (chat command), `t0WeZ6` (mission command), `Dhmkj` (anatomy and behavior), `PflQ5` (field states).

## Problem And Goal

Settings → Agents → Launch has a few switches and a read-only Command preview, with no way to add other flags. Add one **Custom flags** field per agent, shared by chats and missions, whose flags go into every session of that agent: plain and role chats, missions, resumes and forks. Runner refuses a value with an unclosed quote or a flag Runner already sets, and warns, while still saving, about a flag the agent's `--help` doesn't list. Only the first problem shows. The preview and Copy show the saved flags as one **Custom flags** line right after the executable.

## Read First

1. The spec, in full.
2. `crates/runner-daemon/src/session/manager/spawn_spec.rs` — `apply_runtime_args`, `compose_args`, `command_preview`, `command_lines`.
3. `crates/runner-core/src/protocol/runtime.rs` and `crates/runner-core/src/protocol/api.rs` — `RuntimeLaunchOptions`, `RuntimeCommandNote`, the launch-option request.
4. `crates/runner-daemon/src/db/app_state.rs`, `crates/runner-daemon/src/ops/runtime.rs`, `crates/runner-daemon/src/runtime_status/versions.rs` — persistence, the save and preview requests, the version probe that the `--help` read sits beside.
5. `crates/runner-app/src/surfaces/settings/agents/{launch,mod}.rs` — the Launch card and the Executable field's commit, Escape and `FieldValidation` handling.

## Deliverables And Boundaries

- Implementation steps 1–4 of the spec: storage and protocol (no migration), parsing with `shlex`, a reserved set derived from what Runner composes with a per-runtime test that fails when a composed flag is not reserved, the cached `--help` read, insertion in `compose_args` and `command_preview` from one composition, and the Launch card row with English and Chinese strings and the new caption.
- The spec's automated coverage, in the crates it names.
- Guide: Launch options in `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md`.
- Out of scope as the spec lists, including the role editor's Args field.

## Validation And Handoffs

1. **coder:** `make fmt`, `cargo nextest run --locked --workspace --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and the `--features updater` pass. Do not run the app. Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff. Look closely at the reserved-set derivation and its test, the insertion point on every launch path (Codex's prepended `resume` subcommand included), preview and argv coming from one composition, parser edge cases, Windows batch quoting of spaced values, serde defaults for old stored options and peers, and English/Chinese string and guide parity. Pass the clean SHA to `qa` with the checks below plus any the diff adds.
3. **qa:** live checks CLF-01 to CLF-07 from the spec on macOS, following Test Scope: take `script/qa-lock` before building, run `make run` in this worktree, inexpensive model, low effort, tiny prompts in fresh folders under `/tmp`. The development data has no custom flags before this feature; clear every Custom flags field qa set at the end, and archive only qa's own chats and missions. Record the run in `docs/tests/runs/1285-custom-launch-flags-mac.md` and in the PR description. CLF-08 needs Windows and stays pending in this mission.

## Authorization

The `agy trio` is authorized for #1285: it may commit, push `feat/1285-custom-launch-flags` and open a PR against `main` closing #1285. Label it `risk-medium`, keep any automatic `core-review`, add `reviewed` for the final head, and keep `needs-live-test` while CLF-08 has not passed. Stop at the open PR.
