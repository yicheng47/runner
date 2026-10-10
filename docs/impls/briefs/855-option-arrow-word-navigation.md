# 855 — Option+arrow moves by word in shell prompts

Fix [#855](https://github.com/yicheng47/runner/issues/855). Jason requested the `codex duo` crew on 2026-10-10. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-855-option-arrow-word-navigation`, on the existing branch `fix/855-option-arrow-word-navigation`, created from `origin/main` at `c169ac8a`. The root checkout stays on main. Do not create another branch or worktree, touch another crew's checkout, or share a Cargo target directory.

## The bug and required behavior

In a macOS shell chat, typing several words and pressing Option+Right four times inserts `CCCC`; Option+Left inserts `D`. The encoder sends xterm's modified arrow sequences (`ESC [1;3C` and `ESC [1;3D`), which default zsh/bash line editors do not bind. Expected: Option+Left/Right move backward/forward by word at a shell prompt without inserting text, and word movement in Codex and Claude Code continues to work.

Commit `1453c6bd` replaced double-Escape arrows because crossterm interpreted them as Escape followed by literal `[C`/`[D` in Codex. Do not restore that encoding. The issue proposes either Meta-f/Meta-b for plain Option+Right/Left or binding the current sequences in Runner's shell integration. Investigate and select the smallest sound fix; document the choice and any limitation, including shells without Runner integration if that approach is chosen. Keep platform scope deliberate.

## Read first

- `AGENTS.md`, including Worktrees, Test Scope, and Crew Missions, and issue #855.
- `crates/runner-terminal/src/mappings.rs`: `encode_key`, `encode_kitty_key`, `encode_modified_special_key`, and the modified-special-key and kitty test tables. Kitty negotiation takes precedence today; preserve protocol-correct handling for apps that enable it, including pi.
- `crates/runner-app/src/terminal/element.rs`: the native key handler and its calls to the encoder, only as needed to understand platform behavior.
- `crates/runner-daemon/src/shell_integration.rs` and `crates/runner-daemon/shell-integration/`: zsh/bash injection, startup preservation and existing real-shell PTY tests if considering shell bindings.
- `docs/tests/regression/README.md` and `docs/tests/regression/terminal.md`: maintained terminal cases and evidence rules. Select checks connected to this diff; the existing suite has no exact Option+arrow shell case, so add a focused feature-specific check.

## Deliverables

1. Fix plain Option+Left/Right word navigation at macOS zsh and bash prompts. Preserve unrelated key semantics: plain arrows and application-cursor mode, Ctrl/Shift combinations, other Alt-modified special keys, and negotiated kitty behavior. Avoid runtime-name-dependent special cases unless evidence makes one necessary.
2. Add meaningful regression coverage for both directions and repeated navigation that would expose literal C/D or [C/[D] insertion. Exercise actual line-editor behavior in isolated temporary shells where feasible, rather than relying only on byte assertions. Cover the affected encoding/protocol modes and modifier boundaries. If the wire encoding changes, substantiate Codex/crossterm and Claude Code/Ink compatibility using available parser tests or primary source evidence; clearly separate that evidence from native live validation.
3. Keep documentation proportional to a small input bug. Record the fix choice, selected checks, exact commands/results, and remaining native checks in `docs/tests/855-option-arrow-word-navigation.md`. No new UI, layout, settings, feature spec, or Pencil change is needed. Do not rewrite historical test results. README changes, if actually necessary, must update both languages.

## Review and verification

Before testing, list changed behaviors and chosen checks with the reason each covers this diff. Run `cargo test --locked -p runner-terminal --profile ci`; run relevant daemon tests if shell integration changes, and runner-app tests if app code changes. Run workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings`), `cargo fmt --all --check`, and `git diff --check`. Gate imports/helpers used only by Unix tests with `#[cfg(unix)]` so Windows CI builds. Do not run the entire live regression suite for this fix.

The reviewer waits for the coder's explicit Runner handoff and then reviews the complete branch diff against #855. Focus on shell semantics, agent-parser compatibility, modifier/platform boundaries, kitty negotiation, and whether tests expose the original failure. Iterate on the uncommitted implementation until the reviewer reports `NO REMAINING MUST-FIX ISSUES`. The preparation brief commit already exists; do not commit implementation or publish before clean review.

Crews never run the dev app or drive Jason's Runner for UI checks. This mission authorizes bounded automated shell/PTY tests with temporary fixtures, but does not authorize extra agents, crews, subagents, agent account probes, configuration changes, or live agent/UI testing. Jason's native check is a draft with several words in zsh, bash, Codex, and Claude Code: repeated Option+Left/Right moves by word without inserting characters or submitting the draft. Report those checks as pending, not passed by unit tests or CI; list kitty/native checks if the chosen diff affects them.

## Authorization and final handoff

After clean review, Jason authorizes folding this brief and the implementation into one focused commit on this branch, pushing `fix/855-option-arrow-word-navigation`, opening a PR against main with `Fixes #855`, and driving CI green on macOS and Windows. Rebase onto current `origin/main` when needed; never merge main into this branch. Fold review/CI fixes into the change commit and push with `git push --force-with-lease`. Do not merge, archive the mission, delete branches/worktrees, or cut a nightly/release.

Final Runner handoff: PR URL, selected approach and resulting behavior, exact validation results, CI status for both platforms, reviewer verdict, and the native checks Jason should run. Then stand by. Use Runner messages for handoffs; do not poll or busy-wait for the other slot.
