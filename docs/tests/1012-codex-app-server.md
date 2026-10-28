# 1012 — Codex app-server test record

Hand test of the mechanism in the [spec](../features/1012-codex-app-server.md), outside Runner. No Runner code was changed or run.

## Environment

- 2026-10-28, macOS 26.6.2 (arm64), Codex CLI 0.165.0, signed in with ChatGPT.
- One `codex app-server --listen unix://<scratch>/app.sock` per run, on a scratch socket. The person's shared daemon was not touched.
- The TUI ran in a pseudo-terminal rendered with `pyte`, so its screen could be read. A second client spoke WebSocket over the same socket at `ws://localhost/rpc`, as Codex's own remote client does, standing in for Runner.
- Turns used `model_reasoning_effort="low"`. Threads ran in a scratch working directory.

## Results

| ID | Check | Result | Evidence |
| --- | --- | --- | --- |
| B1 | A client that has not subscribed sees a turn the TUI starts | Failed, as expected | No `turn/started` or `turn/completed` for the TUI's thread within 30 s. `thread/started` was received. |
| B2 | After `thread/resume` on the live thread, the client sees the TUI's next turn | Passed | `turn/completed` for the TUI-submitted turn. The TUI kept working. |
| C1 | `turn/start` from the client runs on the TUI's thread while the composer holds `DRAFT-123 keep me` | Passed | The turn completed. The TUI showed `› [inbox] 2 new messages. …` then `• INBOX-ACK`; the composer line still read `› DRAFT-123 keep me`. |
| C2 | Enter in the TUI afterwards submits only the draft | Passed | The next `userMessage` item was exactly `DRAFT-123 keep me`. |
| D1 | The TUI's `-c shell_environment_policy.set.RUNNER_SPIKE="tui-a"` reaches its thread's commands | Passed | `echo SPIKE=$RUNNER_SPIKE` printed `SPIKE=tui-a`. |
| D2 | A second thread on the same server, created with `config: {"shell_environment_policy.set.RUNNER_SPIKE": "client-b"}`, gets its own value | Passed | Printed `SPIKE=client-b`. |
| E1 | `codex resume --remote unix://… <id>` attaches to a thread the client created with no turns yet | Failed | "thread/resume failed during TUI bootstrap: no rollout found for thread id …". |
| F1 | Two TUIs started together each report their own thread id in the terminal title with `tui.terminal_title=["activity","thread-title","project-name","thread-id"]` | Passed | Each TUI's last OSC title began with its own thread id, matching `thread/loaded/list`. |

## Not tested

Codex's shared daemon (start, restart, Remote Control from a paired phone); `codex resume --remote` of a thread that has history; `thread/fork`; approvals with several clients attached; `runtimeWorkspaceRoots` in place of `--add-dir`; Windows and Linux.

The test threads remain in the tester's Codex history: `01a18147…`, `01a18148…`, `01a18152…` and `01a18154…` under `~/.codex/sessions/2026/10/28/`.
