# #1122 — Show a paired phone as a phone in Remote access

Jason requested a claude duo mission (coder, reviewer) on 2026-11-02, and added a qa slot (codex, handle `qa`) for live checks the same day. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1122-paired-phone-row` on `fix/1122-paired-phone-row`, based on `origin/main` at `a9395c3ef9fa596ebd4a385851e19cafb686d116`. The root checkout stays on main. Use this worktree's own `target/`. Other worktrees (#897, #1111) are another machine's checkout.

## Problem

[#1122](https://github.com/yicheng47/runner/issues/1122) has the screenshots-in-words. In short: **Settings → Remote access** shows a paired phone as the leftover unmatched-device row, which is built for computers: `{fingerprint} · {name}` truncates the phone's name, the icon is `monitor.svg`, the status is a raw `last_seen_at` timestamp instead of `Offline · seen 5 days ago`, and the table header, intro, PIN, lock, empty-state and pairing-hint copy all assume the other side is a computer. The host cannot tell a phone apart: `PairPayload` carries only a name, `PairedDevice` stores no kind, and the `client` in `Hello` is not saved.

## Approach (decided by the operator)

Clients already identify themselves in the companion `Hello`: the iOS app sends `client: "ios"` (runner_ios `Runner/Models/Companion/Messages.swift`) and the desktop remote client sends `client: "runner"` (`runner-daemon/src/remote/client/route.rs`, `welcome`). Record the kind from the `Hello` on an authenticated, paired channel instead of changing `PairPayload`: no companion protocol change, and iOS builds already in testers' hands work as they are.

- Store a mapped kind, not the raw string: `ios` → phone, `runner` → computer, anything else → unknown. The value is self-reported and display-only; never use it for authorization or matching trust.
- `PairedDevice` gains the field with a serde default, so existing trust data loads as unknown. It updates on each authenticated connect and is persisted with the trust data, so an already-paired phone becomes a phone on its next connection.
- If `Hello` turns out not to be tied to a device fingerprint on the host, or recording it needs a wire change, stop and ask the operator before touching `PairPayload` or the companion protocol.

## Read first

The issue; `runner-core/src/remote/status.rs` (`PairedDevice`, `RemoteTrust`); `runner-daemon/src/remote/{trust.rs,channels.rs}` (`record_paired`, the `Hello` read in the channel loop, the per-channel fingerprint); `runner-core/src/companion/messages.rs` (`Hello`); `runner-app/src/surfaces/remote_client/{model.rs,mod.rs}` (`computer_rows`, `incoming_devices`, `render_computers`, `computer_menu`, `status_label`); `runner-app/src/surfaces/settings/remote.rs` and its tests; `runner-app/i18n/{en,zh-hans}.toml` `remote.*`.

## Deliverables

Two phases. Phase 1 needs no design; start it now. Phase 2 is blocked until the operator sends the approved design.

**Phase 1 — record the kind.** Persist the mapped kind on `PairedDevice` from the paired channel's `Hello`, expose it through `remote_trust` to the app, and keep phones out of the name-based match in `computer_rows`/`incoming_devices` so a phone named like an account computer never merges into that computer's row. Tests: `Hello` with `ios` on a paired channel stores phone and `runner` stores computer; an unrecognized client stores unknown; trust data written before this change loads as unknown; the kind survives a daemon restart; a phone never matches an account computer by name. Hand off to `reviewer` when green.

**Phase 2 — row and copy (blocked on design).** Wait for the operator's Runner message naming the approved frame in `yeelinks_design` `runner/desktop/specs/1122-paired-phone-row.pen` and the chosen copy. Do not invent the layout, icon or wording before it arrives. The issue's expected behavior is the baseline, and the frame wins on detail: a phone icon (add the Lucide `smartphone` icon to `assets.rs` if it is not there), the pairing name first with the fingerprint secondary or in the menu, last-seen in the same relative format as computers, access `Can open this computer`, Revoke in its menu, and the `remote.*` header, intro, PIN, lock, empty-state and hint copy in both `en.toml` and `zh-hans.toml` reading correctly for phones and computers. Unknown-kind leftover rows follow whatever the frame specifies for them. Update the `docs/guide/` remote-access pages (English and Chinese) where they describe this table or pairing from a computer only.

Non-goals: no `PairPayload` or companion wire change, no runner_ios change, no change to how computers are matched to account hosts beyond excluding phones, no new settings.

## Validation

In-process: the Phase 1 tests above; a settings render test for a phone row covering name order, icon, relative status and the Revoke menu; the existing remote settings and model tests still pass. Run the affected `runner-core`, `runner-daemon` and `runner-app` tests with locked dependencies, `cargo fmt --all --check`, workspace Clippy with warnings denied, and `git diff --check`. Drive the required CI green on both platforms.

Live checks by `qa`, after Phase 2's code review is clean: `coder` hands the frozen candidate to `qa` with the branch, worktree, HEAD SHA, the approved frame and option, and what changed. `qa` builds that revision in `CARGO_TARGET_DIR=target/qa` and tells `coder` when the build has finished. While `qa` holds a candidate, `coder` edits source only after that confirmation and from then on builds in `CARGO_TARGET_DIR=target/coder`. `qa` seeds `remote-trust.json` in the development data with a phone and an unknown-kind device (and a computer if the table needs one), then checks the row, its menu, Revoke and the changed copy in English and 简体中文, with a screenshot for each visual check. `qa` findings go back through `reviewer` as a delta review. The PR description carries `qa`'s verdict as reported.

Left to Jason, because it needs a real iPhone: pair the iOS app with the development app's PIN, open Settings → Remote access and confirm the phone row matches the frame; reconnect an iPhone paired before this change and confirm it turns into a phone row; confirm computer rows look unchanged. List these in the PR as not run.

## Authorization and handoff

Jason authorized this claude duo mission, with a `qa` slot added for live checks: implementation, review, live checks, commits, pushing this branch, and opening one PR against main. `coder` implements and owns all commits; `reviewer` reviews each handed-off snapshot; `qa` runs the live checks on the frozen candidate and does not edit source or commit. Fold this brief into an implementation commit, and fold review and CI fixes into the commits they correct, using `git push --force-with-lease` when rewriting pushed history. Do not add tool-specific co-author trailers. The trust-data change may draw the `core-review` label; keep it. The PR may close #1122.

Stop at an open PR with green CI. Report through Runner with the PR URL, final SHA, changes, validation and the live checks left to Jason. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. When waiting for a teammate or the operator, finish your turn and rely on the pushed inbox notification.
