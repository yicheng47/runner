# 624 — Status details smoke checklist

Scope: [feature spec](../features/624-status-details.md), [implementation plan](../impls/624-status-details.md), and the uncommitted mission-1 working tree on `feat/624-status-details`. These are manual checks; none is marked passed by automated tests.

## Coverage boundary

Automated tests cover elapsed formatting and every specified presentation string, failure rollup priority/count/target behavior, transient `failed_since` and unread lifecycle, hook-only detail normalization, synthetic tool and compaction transitions in all three adapters, and Claude Code's macOS and Windows hook-registration forms. The shared Rust/GPUI implementation is the same on macOS and Windows.

The tests do not prove live CLI event emission, visual truncation, OS-specific rendering, or redraw cadence. In particular, the one-second tooltip refresh exists only while its tooltip view is alive and the runner-card refresh is minute-aligned only while a wait is open, but their visible cadence remains a manual smoke item. The real Copilot CLI integration test remains ignored because it consumes live credit; its adapter behavior is covered with synthetic reports and transcripts.

## Automated verification — 2026-09-17

- [x] `cargo test --locked -p runner-app -p runner-backend --no-fail-fast --profile ci` — passed, including 101 `runner-app` library tests, 268 app tests, and 812 `runner-backend` tests.
- [x] `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` — passed.
- [x] `cargo fmt --all --check` — passed after the implementation and documentation updates.
- [x] `git diff --check` — passed after this smoke record and the implementation-plan log were final.

## Jason's macOS and Windows checklist

Use fresh direct-chat and mission sessions for Claude Code, Codex, and Copilot. Keep hooks enabled except for the baseline row. Record the branch/commit, OS, CLI versions, theme/text size, and a concrete reproduction for any failure.

| macOS | Windows | Check | Exercise | Expected result |
| --- | --- | --- | --- | --- |
| [ ] | [ ] | Interrupted outcome | In each runtime, press Escape during an active response in a direct chat and a mission runner. | The pane-header and single-pane tab label say `Idle · Interrupted` with no hover tooltip; the mission card says `Idle · Interrupted`; the Idle ring is unchanged; the sidebar row has no unread dot or rollup count. A new prompt clears the interrupted detail. |
| [ ] | [ ] | Wait timers | Leave a Claude Code or Copilot approval open for more than a minute, including in a single-pane row and a multi-pane or collapsed rollup. Hover the approval's single-pane sidebar row glyph for more than two seconds while another pane remains Working. | The single-pane-row tooltip rises from `· Ns` to `· Nm` once per second while shown, including through redraws from the other pane. The header/tab label and card have no tooltip; they stay plain under one minute, then show `· 1m` and advance by whole minutes. Narrow panes drop the time before falling back to the icon-only header. The rollup tooltip remains an untimed count. |
| [ ] | [ ] | Failure attention | Force Claude Code `StopFailure` while its pane is not viewed; inspect the single-pane row, collapsed project row, and section header, then open the failed pane. | Attention uses the red `circle-alert`; the pane tooltip says `Response failed · Agent is still connected`; aggregate tooltips say `1 response failed` separately from process errors. Opening the pane clears the sidebar/project/section attention while the pane header and runner card remain `Response failed` until the next prompt. |
| [ ] | [ ] | Tool detail | Run a tool-heavy prompt in each runtime and inspect a pane header, single-pane tab label, single-pane sidebar row, mission card, multi-pane rollup, and mission tab strip. | The pane/header label, single-pane row tooltip, and mission card say `Working · Using tools` while any tool is in flight and plain `Working` between tools. A second in-flight tool keeps the detail after the first completes. Narrow panes drop `Using tools` before falling back to an icon; aggregate rollups and the mission tab strip remain glyph/count only; long card detail truncates instead of growing the card. |
| [ ] | [ ] | Compaction detail | Run `/compact` while idle in each runtime, then trigger automatic compaction while possible tool work remains. | The header/tab label and mission card show `Working · Compacting context` while compaction runs. Manual compaction returns to Idle with the previous outcome when it ends. In-turn compaction returns to Working and restores `Using tools` when a tool remains. Claude Code and Codex end at `PostCompact`; Copilot ends at its next prompt/tool work event or turn end. Stop, failure, or interruption clears the detail. |
| [ ] | [ ] | Baseline isolation | Disable status hooks and run a prompt in each applicable runtime. | Estimated Working/Idle remains unchanged and never shows `Using tools`, `Compacting context`, an interrupted claim, a wait timer, or response-failure attention sourced from baseline activity. |

## Recording a live pass

Replace only the applicable platform boxes after running the checks and append the tested OS version, Runner commit, CLI versions, and any skipped runtime or unavailable failure fixture. A synthetic `StopFailure` proves rendering and acknowledgement; distinguish it from a real CLI-emitted fatal response in the record.
