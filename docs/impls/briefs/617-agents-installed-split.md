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

Out of this mission: segmented Enabled/Disabled and the chevron expander; `runtime_status.rs` detection; the catalog's runtime list; `#533`; unrelated `docs/`. The per-card default action is now in scope through Jason's follow-up below.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff with one question in front: **does a card land in the right section for every one of the six row states, including while a probe is in flight?** Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree. Do not launch or restart the Runner app — `target/debug/runner` is the GUI binary — and touch no real Runner data directory or database.

**After the reviewer's clean verdict: commit, push `feat/617-agents-installed-split`, and open a PR against `main` that closes #617.** Then drive CI green with `gh pr checks <pr> --watch`, both the macOS and the Windows job. **Do not merge.** Jason does the quality check and the final merge himself.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`.

Unit tests for the partition over all six `RuntimeRowState` values, and for both empty states. The pane itself is judged with eyes, so the PR body must list what Jason should look at: a machine with some agents missing showing both sections with the toggle only in the first; setting a valid override on a not-found agent moving its card up without a restart, and clearing it moving it back; Refresh re-sorting after an agent is installed while Runner is open.

## Handoff

Final Runner handoff on the feed: branch and base commit; every file and function changed; the partition function and where it is called; the install-hint field, its values and the source of each; how the empty states render; the tests and what each would catch; checks with results; the reviewer's no-remaining-must-fix verdict; and the PR number.

## Follow-up: per-card default action (2026-09-21)

Jason requested this during live UI testing and selected the existing design frame `n1krgH`, then called out too many status pills on Codex card `N67UH`. This section supersedes the earlier instruction to keep Installed cards exactly unchanged and the earlier authorization to commit this iteration. Preserve existing PR #684 and its commits. Implement and review this follow-up as uncommitted working-tree changes; wait for Jason to confirm the UI works before committing or pushing the implementation. Jason separately authorized the coordinator to commit and push this documentation update; that does not authorize committing the UI changes. No merge.

The updated design is in the root checkout's `design/runner.pen`, frame `n1krgH`; use this written handoff as the implementation contract. The exported reference cards are `/tmp/runner-617-default-agent/N67UH.png` and `/tmp/runner-617-default-agent/AConK.png`. Do not copy or modify the root checkout or its design file. Existing model, path and install-link sample values on the canvas are illustrative; keep actual runtime values and the already verified install links.

Remove the standalone Default agent selector card and dropdown. Below the Installed header's shell caption, add a compact summary line: `Default for new chats: Codex` when explicitly selected, using the actual display name. At the right of this line, a quiet `Use first available` action clears the explicit preference through the existing settings update path. This is a caption row, not another settings card.

On an eligible Installed card, show a small `Set as default` button at the right of the header, immediately before the enable toggle. The explicitly selected card replaces that button with a plain check icon and `Default` label: no pill background, border or clickable appearance. Selection must persist and immediately refresh all card actions, the summary, and any open Start Chat modal using existing synchronization. Keep the enable toggle separate from default selection.

Jason's latest correction preserves the existing `Detected` and `Override` pills beside the agent name, including their dot and color. Default is a separate selection, not a third detection state: an agent can be both Override and Default. Reduce competing decoration by rendering `Default` as plain secondary text with a small accent check, without a pill. Remove the visible redundant Enabled/Disabled label; keep the toggle and its accessible name/tooltip so its purpose remains clear. Preserve informative error/timeout messaging and the Checking spinner; detection states and partitioning remain unchanged. The Not installed cards gain no default action or enable toggle.

Preserve current selection semantics: eligibility is enabled plus effective source Detected or Override, not mere membership in Installed. Checking and ProbeTimedOut remain in Installed but must not become newly selectable defaults. Keep the current reconciliation behavior, including preserving a configured default while the shell check is in flight and returning to automatic mode when the configured default becomes invalid or is disabled. Do not add new settings or alter crew/role runtime selection.

Jason clarified that `Set as default` stays visible but disabled while its row is Checking; the existing explicit Default marker remains. ProbeTimedOut continues to use the existing source-based eligibility rule.

Automatic mode still stores the existing empty preference. Its summary reads `Default for new chats: First available (currently Codex)`, deriving the current agent from the same eligible catalog order as Start Chat. No card carries the explicit `Default` marker in automatic mode; all eligible cards offer `Set as default`, including the agent currently chosen automatically, so it can be pinned. Hide `Use first available` when already automatic. If no agent is selectable, show `Default for new chats: No available agent` and no default buttons. Preserve the existing empty-section copy and Refresh access.

Verify selecting another agent, pinning the automatic choice, returning to first available, disabling or invalidating the explicit default, a refresh in flight, zero available agents, and an already-open Start Chat modal. Reuse existing coverage where it verifies these behaviors; add focused tests only for meaningful new derived state. Run runner-app tests, workspace clippy, formatting and diff checks for this follow-up; do not repeat broader checks unless failures or changed scope justify them. Check that the header does not overlap or clip at supported window widths. Do not launch or restart Jason's app. Coder hands the uncommitted diff to reviewer through Runner; reviewer checks these selection and visual-state requirements and reports remaining must-fix findings.

Jason subsequently authorized publishing this follow-up so he can continue on another computer. After checks and a clean working-tree review, commit and push the iteration to existing PR #684 and drive both macOS and Windows CI green. This supersedes the earlier requirement to wait for live UI confirmation before committing or pushing. Do not merge.

## Follow-up: remove the default summary (2026-09-21)

Jason's latest screenshot review removes the entire `Default for new chats` summary row and its `Use first available` action. Keep default selection and the `Default` marker on the Installed cards. Automatic fallback when no explicit preference exists, or when the selected agent becomes unavailable or disabled, remains unchanged. This supersedes the summary/reset UI requirements above. Leave this iteration uncommitted for live review.

## Approved design: inline implementation (2026-09-21)

Jason approved the updated Agents design and requested implementation inline, without a mission or subagents. Work in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-617-agents-installed-split` on the existing PR #684 branch. Preserve the current uncommitted edits in `crates/runner-app/src/surfaces/settings/agents.rs` and this brief. This section supersedes the earlier conflicting design, mission, review-agent and publishing instructions. Do not commit, push, open another PR, merge, or create another checkout.

The approved Pencil source is `/Users/jason/repos/yicheng47/runner/design/runner.pen`, frame `n1krgH` (Settings — Agents). Inspect it through Pencil MCP only; the branch's design file is older. Do not copy, overwrite or edit the root checkout's design. The written requirements below are sufficient if Pencil is unavailable.

- Remove the whole `Default for new chats` / `Use first available` summary row (`rcKyA`, now deleted from the design).
- Remove normal `Detected` and `Override` pills from Installed card headers. Preserve the underlying detection states, source/path details, validation errors, timeout messaging and Checking indication.
- Show the explicitly selected agent's `Default` marker as the sole status pill beside its name on the left. Use a small accent check and accent text on a subtle accent background, matching design node `YoIyw` (11px label/check, 5px gap, 3px vertical and 9px horizontal padding, 10px corner radius).
- Put each eligible agent's `Set as default` button beside its name on the left, in the same position as the Default pill. Design nodes `t34HgG` and `y0RXl` show this for Claude Code and GitHub Copilot CLI. The enable toggle stays on the far right.
- Keep existing persistence, source-based eligibility, disabled actions during Checking, automatic fallback and open Start Chat synchronization. Automatic mode has no Default pill until an agent is explicitly selected. Keep Not installed cards unchanged.

Keep implementation scoped to this settings design; the separate P1 Codex Working-status report #687 is not part of this chat. Update existing tests only where behavior or assertions changed. Run runner-app tests, workspace clippy, formatting and diff checks, using the existing CI profile where practical. Report the uncommitted changes and check results, plus anything needing visual verification. Do not launch or restart Jason's app without a new instruction.

Validation: `cargo test --locked -p runner-app --profile ci --no-fail-fast --quiet` passed 435 tests, with the two existing paid runtime smoke tests ignored; workspace Clippy with `--profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check` passed. Existing layout checks cover widths of 320, 480 and 760 px at 16 and 20.8 px rem sizes, and selection tests verify both the left-side Default marker and persisted preference. Jason confirmed the live UI is correct and authorized publishing and merging PR #684. This supersedes the uncommitted-only restriction above. Commit the approved design separately on main after the branch merges.
