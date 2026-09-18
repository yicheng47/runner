# 539 mission 2 — pi hook smoke and evidence

Branch: `feat/539-pi-hooks`. Mission tip: `4e4dc72`; mainline base: `cbb1271`. Installed CLI: pi 0.85.1 at `/Users/jason/.nvm/versions/node/v24.19.0/bin/pi`. Implementation, verification and working-tree review remain uncommitted by mission authorization. The adapter is implemented for macOS and Windows; only macOS received live PTY evidence in this mission, and the native Windows smoke remains mission 3.

## Coverage boundary

Runner regenerates `<app data>/pi-hooks/runner-status.ts` atomically at startup and passes it as `-e <path>` on every pi spawn, resume and fork before `--append-system-prompt` and the goal. It sets a per-spawn feed path and generation, the same session key passed through `--session-id`, and the existing Claude rekey drop path. The extension is inert without the feed environment or outside `ctx.mode === "tui"`; it writes no pi configuration, extension, trust, authentication or session file under `~/.pi`. A nested pi process inherits the environment but not Runner's `-e`, so it cannot report into the parent feed.

The initial non-reload `session_start` resets local state without publishing Idle; after a hook observation establishes ownership, new, resume and fork starts publish Idle with no outcome and clear activity detail and waits. Working begins at `agent_start`; Using tools lasts while any `toolCallId` is open; Compacting context spans `session_before_compact` through `session_compact` or `session_compact_failed`; and ordinary turns become Idle only at `agent_settled`. The last assistant `message_end` before settling yields Completed for normal stops, Response failed for `error`, and Interrupted for `aborted`. `agent_end` remains intentionally unmapped, and a reload start is not a boundary. Confirm prompts raise Approval needed; select, input and editor raise Answer needed; custom raises Needs you; `ui_prompt_end` and `session_shutdown` clear the one coalesced wait. Only that unresolved wait holds crew delivery; Working and Idle do not.

The extension writes one complete inline JSON record with one `fs.appendFileSync` per handled event and silently ignores every write failure. It truncates an assistant `errorMessage` to 512 characters. Pi versions below 0.84.4 register no handlers; a package-version import failure counts as current. Removing the installed extension is bridge loss and releases status to the terminal baseline. No Runner control for pi prompts is added.

Pi's project-trust dialog remains unmapped because pi emits no `ui_prompt_start` for it; mission slots already receive `--approve` for one-run trust. `agent_end` also remains unmapped because pi can retry, compact or run queued work afterward. A short-session `/compact` was observed to emit only `session_compact_failed`, without a preceding `session_before_compact`; the watcher treats that as a no-op because no compacting span began. The documented select, input, editor and custom prompt kinds are implemented and unit-tested, but only confirm received a live extension prompt in this mission.

## Live PTY evidence

The probes ran pi 0.85.1 through `pty.fork` in this repository with `PI_SKIP_VERSION_CHECK=1`, `--session-dir` under `/tmp/runner-pi-hook-probe`, `--approve`, the Runner extension through `-e`, and the low-cost `deepseek/deepseek-flash` model. Each child exited through `/quit` or the driver's bounded master-close/SIGKILL fallback. The one interrupted probe was checked immediately and left no child; the final process check found no survivor. The probe did not write into `~/.pi/agent/extensions`, did not write user configuration by hand and did not delete anything under `~/.pi/agent/sessions`. The records below are sanitized to event names and mapping fields.

| Probe | Observed extension records | Adapter result |
| --- | --- | --- |
| Short prompt | `agent_start` → assistant `message_end(stop)` → `agent_settled` | Working → Idle/Completed, with no Idle transition at an earlier event. |
| Bash tool | `agent_start` → assistant `message_end(toolUse)` → `tool_execution_start(bash)` → `tool_execution_end(bash)` → assistant `message_end(stop)` → `agent_settled` | Working → Using tools → Working → Idle/Completed. |
| Manual `/compact` after a deliberately large two-turn context | `session_before_compact(reason=manual)` → `session_compact` | Compacting context for the exact observed span, then the prior Idle state resumes. |
| Escape during a long response | `agent_start` → assistant `message_end(aborted, "Request was aborted")` → `agent_settled` | Working → Idle/Interrupted. The native feed is sufficient, so Pi has no Runner input interrupt signal. |
| Invalid DeepSeek credential | `agent_start` → assistant `message_end(error, <sanitized 401 authentication error>)` → `agent_settled` | Working → Idle/Response failed. The credential and raw response are not retained here. |
| Scratch extension calling `ctx.ui.confirm` | `ui_prompt_start(confirm, "Runner status probe")` → Enter → `ui_prompt_end(confirm, "Runner status probe")` | Approval needed while the real prompt was open, then the wait cleared. |
| `/new` inside the TUI | `session_shutdown(reason=new)` → `session_start(reason=new, <new UUID>)`; the atomic drop file contained the same UUID | Idle with no outcome at the new boundary, with the row ready to rekey through the existing watcher. |
| Parent Bash tool launches nested `pi -p` | The parent emitted its normal tool span and settled; zero additional `session_start` records appeared | The nested pi did not attach to or write into the parent feed. |

The embedded-source Node test separately runs a `.mjs` copy with a stub pi API and TUI context, checks the inline records consumed by the Rust watcher, the 512-character bound, atomic rekey behavior and matching-key exclusion, print-mode inertness, and the 0.84.4 version floor. It skips with an explicit message when Node is unavailable.

## Jason's manual macOS checklist

1. Open a pi role chat and submit a short prompt. Confirm Working appears without an estimated tooltip, a Bash tool changes the detail to Using tools, and only `agent_settled` returns the row to Idle/Completed.
2. Build enough context for `/compact`, run it, and confirm Compacting context appears until compaction completes or fails, then the prior state returns.
3. Escape during a response. Confirm pi returns to its prompt and Runner reaches Idle/Interrupted without needing a second keypress; the next prompt must restore Working.
4. Exercise a disposable provider-auth or unknown-model error. Confirm the row becomes Idle/Response failed at settlement and the next valid prompt still works.
5. Run `/new` inside the pane, then relaunch the chat. Confirm the sidebar row updates without an app restart and the relaunch resumes the new conversation rather than the original one. Repeat the identity check with `/resume` and `/fork` if convenient.
6. In a real crew with a pi slot, load a scratch extension that calls `ctx.ui.confirm`. While its prompt is visible, confirm Runner shows Approval needed and holds a queued delivery; answer it and confirm the hold releases. Post another crew message while the slot is merely Working and confirm Working itself does not hold delivery.
7. Spot-check the same status and outcome in a single-pane tab, a split-pane header, the collapsed sidebar, and the mission tab/card/row. Stop and relaunch the session once to confirm hooks return without persistent pi configuration.

## JASONPC mission 3 checklist

1. Confirm startup regenerates `%APPDATA%`'s Runner-owned `pi-hooks/runner-status.ts`, every pi invocation receives `-e` before its prompt/goal arguments, and all feed/rekey paths in the four environment variables use forward slashes acceptable to Node.
2. In a direct chat and a crew slot, verify prompt Working → Idle, Bash Using tools, manual compaction, Escape → Interrupted, and a disposable provider error → Response failed.
3. Use a scratch extension to open `ctx.ui.confirm`; verify one Approval needed wait holds crew delivery until answered, while ordinary Working and Idle never hold it.
4. Run `/new`, `/resume` and `/fork` inside a pane. For each, verify the sidebar row rekeys without restarting Runner and a subsequent relaunch opens the selected pi conversation.
5. Launch a nested `pi -p` from the parent Bash tool and verify it creates no parent-feed records. Remove the Runner-owned extension only in a disposable app-data copy and verify bridge loss falls back to estimated terminal status without ending the terminal session.
6. Stop each test session and verify its status feed is cleaned up, no shell or PowerShell reporter was created for pi, and no user file under `~/.pi/agent/extensions` was installed or changed by Runner.

## Automated verification and review

`make verify` passed after the real-PTY teardown test was aligned with the runtime's asynchronous output-channel close: workspace check and tests, workspace/all-target Clippy, app/updater Clippy, and formatting are green. The locked CI-profile package run passed 839 runner-backend tests and 408 runner-app tests; the credit-spending Copilot binary smoke remained intentionally ignored. `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` passed; only the existing third-party future-incompatibility notices remain. The final formatting and diff checks are run immediately before the reviewer handoff, and the review verdict remains in the Runner mission record. Native Windows interactive behavior and the rendered Runner UI checklist above remain unproven in this mission.
