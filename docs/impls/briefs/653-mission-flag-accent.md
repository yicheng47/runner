# 653 — a live mission's flag draws in the accent

[#653](https://github.com/yicheng47/runner/issues/653), P2, milestone 0.11. Work in `/Users/jason/repos/yicheng47/runner-wt2` — a third checkout of the same repository, so other crews can work in `/Users/jason/repos/yicheng47/runner` and `/Users/jason/repos/yicheng47/runner-wt` at the same time. Stay inside `runner-wt2`. You are on branch `feat/653-mission-flag-accent`, cut from `main` at `509e493`; its first commit carries this brief. Build on it; do not rebase, squash or create another branch.

**The issue body is the spec** — read it end to end, including its Motivation, Scope, Tradeoffs and Verification. This brief only records the decisions that the issue leaves open and the boundaries that come from not being able to run the app. There is no separate feature doc and none is needed.

## Deliverable

`ChatIcon` (`crates/runner-app/src/chat_icon.rs`) gains a mission constructor whose tint is `theme::accent()`, alongside `generic` and `for_runtime`. It then follows the rule `ChatIcon::color` already applies to the Claude Code, TRAE and Copilot marks with no new colour rule: the tint while live, `theme::with_alpha(theme::text(), 0.45)` when not.

The two sidebar call sites use it in place of `ChatIcon::generic("flag.svg")` — the inline-rename row at `crates/runner-app/src/surfaces/sidebar/rows_render.rs:217` and the normal row at `:265`. Both already pass `summary.any_session_live` as the liveness flag, so nothing new has to be plumbed.

### Three decisions this brief settles

- **The terminal glyph stays neutral.** The issue floats the accent for terminals too and then recommends against it; take the recommendation. A shell is the quietest row, and an accent on every live row stops meaning anything. Jason can overrule it at the smoke — do not build a toggle for it.
- **Only surfaces that already carry a liveness flag change.** That is the sidebar. The workspace header (`crates/runner-app/src/surfaces/mission_workspace/view.rs:380`) and the command palette (`crates/runner-app/src/surfaces/command_palette.rs:45`) pass the bare path `"flag.svg"` and have no liveness input; the issue's "every surface that draws the mission glyph **with a liveness flag**" is exactly that qualifier. Do not plumb a new liveness signal into them for this change. Say in the handoff that they still draw the neutral flag, so Jason can decide whether that reads as a bug.
- **The flag glyph itself is unchanged.** Whether the outline flag reads too thin beside the solid provider logos is a judgement that needs the running app, and `FLAG` in `crates/runner-app/src/assets.rs:74` is one inline SVG whose `fill="none"` would become `fill="currentColor"` if Jason wants it. Leave it, and name it in the PR as the one thing he may want after looking.

Out of this mission: the provider marks, the label dimming, the tail states and their accent, `#613`'s rule anywhere it is not the mission flag, stopped rows of any kind, the `"New mission"` menu items in `crates/runner-app/src/surfaces/sidebar/menus.rs`, and `docs/`.

## Tests

`chat_icon.rs` already has `provider_marks_keep_their_tint_only_while_live`, which loops the runtimes over both Carbon and Runner Light under a `ThemeGuard`. Extend that shape to the mission flag: the accent while live, the 0.45 text colour when stopped, in both variants. `crates/runner-app/src/surfaces/sidebar/tests.rs:977` expects a generic flag today and changes with it — say in the handoff what that test was asserting and why the new expectation is right.

Note the tradeoff the issue calls out and do not try to solve it: in Carbon the accent `#00ff9c` sits close to TRAE's `#32f08c`, so a live mission and a live TRAE chat differ more by shape than by hue. That is Jason's call at the smoke, not a reason to pick a different colour here.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff, with particular attention to whether any colour rule was widened beyond the mission flag. Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree; do not touch `/Users/jason/repos/yicheng47/runner` or `/Users/jason/repos/yicheng47/runner-wt`, where other crews are working. Do not launch or restart the Runner app — `target/debug/runner` is the GUI binary — and touch no real Runner data directory or database.

**After the reviewer's clean verdict: commit, push `feat/653-mission-flag-accent`, and open a PR against `main` that closes #653.** Then drive CI green with `gh pr checks <pr> --watch`, both the macOS and the Windows job. **Do not merge.** Jason does the quality check and the final merge himself.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`. `main` is 1459 passed / 3 ignored on macOS at `509e493`.

This change is judged with eyes, so the PR body must list what Jason should look at, in both themes: a live and a stopped mission in the same project, a live mission beside a live TRAE chat and a live Claude Code chat, and a terminal row in the neutral treatment that was kept. Name the two open questions with it — the terminal glyph and the outline weight — so he can answer both in one pass.

## Handoff

Final Runner handoff, posted on the feed: branch and base commit; every file and function changed; the tests added and the existing expectation you changed, with why; confirmation that the header and palette were deliberately left neutral; checks with results; the reviewer's explicit no-remaining-must-fix verdict; and the PR number.
