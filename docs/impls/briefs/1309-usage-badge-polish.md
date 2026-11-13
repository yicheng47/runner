# Mission Brief — #1309 Usage Badge Polish

- **Issue:** [#1309](https://github.com/yicheng47/runner/issues/1309) (P1, `agent-ready`) and [#1310](https://github.com/yicheng47/runner/issues/1310) (P2, `agent-ready`, folded scope addition). Their bodies are the specs: read both first.
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1309-usage-badge-polish`, branch `fix/1309-usage-badge-polish`, off `origin/main` `979772e5`. Work only in this worktree, with its own `target/`.

## Problem And Goal

The usage badges and **Settings → Usage** have three problems, plus one added **Settings → Agents** cleanup (#1310). Fix all four:

1. **Codex row off-center.** Settings → Usage gives every runtime other than Claude Code and Antigravity CLI an empty `""` subtitle (`crates/runner-app/src/surfaces/settings/usage.rs:107-114`), so the Codex row still renders an empty subtitle line and its title is not vertically centered against the toggle. Set a subtitle only when there is one.
2. **Antigravity badge uses the highest pool.** `weekly_usage_percent` (`crates/runner-app/src/surfaces/app_shell/usage.rs:145-160`) takes the `max` over every weekly pool for `usage_groups` runtimes. The Antigravity CLI badge, and the remote `weekly_percent` (`app_shell/mod.rs:474`), must always use the `Gemini Models · Week used` window, showing `—` when it is missing. The Usage popover and Settings still list both pools. Replace the subtitle "Shows the highest weekly pool." with the Gemini wording in `crates/runner-app/i18n/en.toml` and `zh-hans.toml`, renaming the key to match. Remove `is_weekly_pool_window` if nothing else uses it.
3. **Badges too tight.** The gap between sidebar usage entries is 5 px (`.gap(px(5. * zoom))` at `app_shell/usage.rs:333`), barely more than the 2 px gap between an icon and its own percentage. Widen the gap between entries to 8 px, or to the value `runner.pen`'s footer frame specifies if it has one (`~/repos/yeelinks/yeelinks_design/runner/desktop/runner.pen`, read-only via Pencil). Keep `BADGE_GAP` in `select_sidebar_usage_entries` in sync so the `+N` overflow math still fits. The Sidebar preview reuses `sidebar_usage_element`, so it follows.
4. **Settings → Agents update row repeats pill buttons (#1310).** In `crates/runner-app/src/surfaces/settings/agents/render.rs`, remove the `agent-task-output-{id}` button ("Show output" / "Answer") from the update task row, keeping only the status caption (and Retry / Reinstall on failed or broken tasks). Collapse the empty button row when not failed so the caption centers vertically in the row. Keep the background-task pill, the Start chat launcher buttons, and the `agent_updates.show_output` and `agent_updates.answer` i18n keys.

Out of scope: popover layout, other settings rows, Start chat launcher, background-task pill task list, and daemon usage parsing.

## Deliverables

- Code fixes for the four items above, all in `runner-app`.
- Tests: update `app_shell/tests.rs:1350` to expect the Gemini value (not the 73% Claude and GPT pool); add a case where Gemini is missing but Claude and GPT is present (expect `None`); cover the subtitle-less row and the new gap/overflow width wherever the existing settings/sidebar tests can; add a test in `settings/agents/tests.rs` asserting that neither Show output nor Answer renders and that the caption centers vertically when not failed while Retry/Reinstall still render on failed/broken tasks.
- Guide: `docs/guide/guides/chats-and-windows.md:91` says Antigravity CLI shows "its highest weekly pool percentage in the footer". Change it to the Gemini Models weekly pool, and update the zh-CN guide page too if it carries the same sentence.

## Validation And Handoffs

1. **coder:** pass `make fmt`, the `runner-app` tests (`cargo nextest run --locked -p runner-app --cargo-profile ci`) and workspace Clippy (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, plus `--features updater` on macOS). Do not run the app. Fold this brief into the fix commit (a separate commit for #1310 is fine). Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff only (#1309 and #1310 together). Pass the clean SHA to `qa` with the live checks below, plus any the diff adds.
3. **qa:** take `script/qa-lock acquire --timeout 540` before building, run `make run` in this worktree, leave pre-existing development sessions untouched, and release the lock once the app and daemon have exited. Live checks on macOS, with screenshots:
   - **Codex row:** in Settings → Usage, the Codex row's title is vertically centered against its toggle, matching single-line rows. This is item 1.
   - **Antigravity value:** the Antigravity CLI sidebar badge equals the Gemini Models weekly percentage in the Usage popover, not the Claude and GPT one. If both pools read the same, say so and rely on the unit test. This is item 2.
   - **Subtitle copy:** the Antigravity CLI subtitle shows the new wording in English. This is item 2.
   - **Spacing:** the footer badges and the Sidebar preview at default zoom show clearly separated entries. Narrow the sidebar until `+N` overflow appears, and check that nothing clips or wraps. This is item 3.
   - **Agents update row (#1310):** in Settings → Agents, an agent update shows only the caption while running and after success; Retry still appears on failure; and the pill still opens the output. Starting a real update changes the agent's installed binary, so run it only if an agent already has an update available, and never force a failure. Otherwise mark those checks as covered by unit tests in the verdict, with the reason.
   Record the run in `docs/tests/runs/1309-usage-badge-polish-mac.md` and in the PR description.

## Authorization

The `agy trio` is authorized for #1309 and #1310. Once review and QA are clean, the crew may commit to `fix/1309-usage-badge-polish`, push, and open a PR against `main` that closes #1309 and #1310. The PR carries the labels `frontend-only` and `risk-low`. It also carries `needs-live-test` until every QA check passes, and `reviewed` on the final head. The crew drives CI green on both platforms and stops at the open PR, without merging.
