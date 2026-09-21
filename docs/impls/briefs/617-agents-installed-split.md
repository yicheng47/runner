# 617 — Settings → Agents splits into Installed and Not installed

[#617](https://github.com/yicheng47/runner/issues/617), P2, milestone 0.11. Work in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-617-agents-installed-split`, a linked worktree. Stay inside it; other worktrees and the repository root hold other people's work. You are on branch `feat/617-agents-installed-split`, cut from `main` at `7d5c880`. Build on it; do not rebase, squash or create another branch.

Read first: this brief; the issue's Motivation and Scope; `crates/runner-app/src/surfaces/settings/agents.rs` end to end; `crates/runner-backend/src/ops/runtime.rs` for the catalog; `AGENTS.md`.

The design is `design/runner.pen`, frame `n1krgH`, signed off on 2026-09-21. **You cannot open it**, so this brief is the spec. Where the two disagree, this brief wins; if something here is ambiguous, ask rather than invent.

## What changes

Today `render` walks `status.runtimes` and emits one `SettingsCard` per runtime into a flat column (`agents.rs:249`, `:780`, `:812`), above a `Shell environment` card (`render_shell_card`, `:543`) that carries the Refresh button. A reader has to read every badge to learn which agents this machine can actually run.

It becomes two labelled sections, so layout answers that instead of badges.

**Installed** — header reads `Installed` with a count pill, and carries the **Refresh** button on its right. The shell-environment line becomes a caption directly under that header: same sentence as today, no longer a card. `render_shell_card` goes away, and with it a card from the pane.

**Not installed** — header reads `Not installed` with a count pill. No Refresh.

Catalog order is preserved inside each section; sections never reorder within themselves.

## The partition

Key on the existing `RuntimeRowState` (`agents.rs:957-978`), not on anything new:

- **Installed:** `Detected`, `Override`, `Checking`, `ProbeTimedOut`. A row being probed keeps its place rather than jumping, and a probe timeout means the binary exists, so it stays with its badge.
- **Not installed:** `NotFound`, `InvalidOverride`.

A card moves between sections whenever detection changes — after Refresh, and after an override is set or cleared — with no restart. This is a derived partition computed at render, not stored state.

## The two cards

**Installed keeps today's card exactly**: mark, name, badge, the `Enabled`/`Disabled` label with its `Toggle`, the override field with Browse and conditional Reset, the model and effort line, and the detected-path caption. Nothing about it changes.

**Not installed is lighter.** It carries the mark, the name, the catalog's one-line `description` of what the agent is, its install page, and the Browse override for a binary Runner did not find on PATH. It **drops** the badge, the Enabled label and toggle, the model and effort line, and Reset.

Dropping the badge is deliberate: the section header already states the row is not installed, and a red `Not found` on every card there is noise. `InvalidOverride` is the one case where the user needs more than the section header — keep its existing validation message visible on the card, since "your override is wrong" is not the same as "this is not installed".

An agent that is not on the machine has nothing to enable, which is why the toggle goes. Do not disable it — remove it.

## New catalog data

`RuntimeCatalogEntry` (`ops/runtime.rs:29`) already has `description`, which is the one-liner. It has **no install hint**, and the not-installed card needs one. Add a field for it and populate it per runtime with the official page — `docs.trae.cn/cli` for TRAE CLI and `github.com/earendil-works/pi` for pi are the two the design shows; find the equivalents for the others from their own documentation and say in the handoff where each came from. Do not invent package names: a wrong `npm i -g …` in a settings pane is worse than a URL.

Keep the per-card copy to *what the agent is* plus *where to get it*. The generic advice — that you can point Runner at a binary it did not find — belongs once, not repeated on every card; put it in the section caption or leave it out.

## Empty states

- **All installed:** the `Not installed` section collapses to a single line rather than an empty header with a zero pill.
- **None installed**, which is a fresh machine: the `Installed` section shows one line inviting the user to install one of these, and every card sits below.

## The traps

- **`runtime_defaults_visible` and `show_reset` already gate parts of the row.** The partition must not duplicate that logic in a second place; compute the section once and let the existing gates do their job inside the card.
- **`Checking` has a spinner** (`agents.rs:965-978` returns a bool for it). A row that starts `NotFound`, is re-probed, and becomes `Detected` must animate in place and then move, not flicker between sections mid-probe.
- **The Refresh button's label becomes `Checking…` while a probe runs** (`agents.rs:586`). That behaviour moves with the button to the section header.
- **The footnote at `agents.rs:818` stays as it is.** It talks about disabled agents and overrides, which is still true of the Installed section.
- **Windows and macOS share this pane.** Nothing here is platform-specific, and the Windows registry work in `#672` is untouched.

Out of this mission: the row redesign sketched in the issue's Reference section — segmented Enabled/Disabled, per-row `Set default`, the chevron expander — which was explicitly **not** chosen; `runtime_status.rs` detection; the catalog's runtime list; `#533`; `docs/`.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff with one question in front: **does a card land in the right section for every one of the six row states, including while a probe is in flight?** Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree. Do not launch or restart the Runner app — `target/debug/runner` is the GUI binary — and touch no real Runner data directory or database.

**After the reviewer's clean verdict: commit, push `feat/617-agents-installed-split`, and open a PR against `main` that closes #617.** Then drive CI green with `gh pr checks <pr> --watch`, both the macOS and the Windows job. **Do not merge.** Jason does the quality check and the final merge himself.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`.

Unit tests for the partition over all six `RuntimeRowState` values, and for both empty states. The pane itself is judged with eyes, so the PR body must list what Jason should look at: a machine with some agents missing showing both sections with the toggle only in the first; setting a valid override on a not-found agent moving its card up without a restart, and clearing it moving it back; Refresh re-sorting after an agent is installed while Runner is open.

## Handoff

Final Runner handoff on the feed: branch and base commit; every file and function changed; the partition function and where it is called; the install-hint field, its values and the source of each; how the empty states render; the tests and what each would catch; checks with results; the reviewer's no-remaining-must-fix verdict; and the PR number.
