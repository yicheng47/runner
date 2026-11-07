# Archived implementation records

Follow the shared [archive policy](../../README.md#archive-policy): keep important architectural or migration records, delete completed small-feature and bug-fix plans, and preserve test evidence and task history. Read this index first, then only the relevant record. These documents describe past implementations and never grant permission to repeat their commands or operate their old worktrees.

## Major programs

Each program's README is its entry point. Open a plan for its design/sequence or a log for a specific dated event; do not read the whole program directory by default.

| Program | Why it stays |
| --- | --- |
| [GPUI rewrite](gpui-rewrite/README.md) | Native cutover, retained product differences, renderer/IME decisions and release-channel history. |
| [Windows port](windows-nightly/README.md) | Platform/process and packaging decisions, with native test and review history. |
| [runnerd](645-runnerd/README.md) | The app/daemon split, terminal snapshots and protocol/lifecycle migration. |
| [Mission containers](562-mission-spawn/README.md) | Immutable rosters, independent sessions, dynamic membership and empty-start migration. |
| [General CLI](648-runner-cli/README.md) | External control, caller identity, agent discovery and removal of the former workspace MCP server. |
| [Status hooks](347-hook-status/README.md) | Native event correlation, authority and UI semantics, with dated investigations. |
| [pi runtime](539-pi-runtime/README.md) | Native prompt, conversation and hook integration, plus its implementation history. |
| [Role rename](604-role-rename/README.md) | Persisted/wire read compatibility during the runner → role migration. |
| [Remote control](836-web-remote-control/README.md) | The `nightly-remote` umbrella's phase and mission sequence, the decisions binding every mission, and the dated log through the 0.18.0 landing. |
| [File preview](634-file-preview/README.md) | The `feat/634-file-preview` umbrella's eleven missions, the decisions binding them (the file service in `runnerd`, the saved root, every platform), and the dated log through the 0.19.5 landing. |
| [Skills](local-skills/README.md) | Native config ownership and the view-before-control decision; deferred work remains in issues. |

## Distinct implementation and compatibility records

| Record | Why it stays |
| --- | --- |
| [PTY cutover](0011-pty-host-terminal-runtime.md) | Why tmux replay failed and the runtime moved to a local PTY; historical predecessor to runnerd. |
| [TRAE integration](0036-trae-runtime.md) | Original versioned CLI/session-file probes for a still-supported runtime. |
| [Windows updater](493-windows-auto-update.md) | Installer handoff, rollback and signed-asset staging beyond the feature's behavior contract. |
| [Unified nightly](502-unified-nightly.md) and [channel cutover](504-nightly-channel-unification.md) | Separate job/identity transitions and their actual first-cut verification records. |
| [Copilot implementation record](540-copilot-cli-runtime.md) | The short mission/merge log and its dated native evidence. |
| [Windows hooks](610-windows-hook-status.md) | Native PC measurements and platform-specific hook constraints. |
| [GPUI dependency migration](733-gpui-pre.md) | API/ABI changes, binary-size probes and compatibility fixes for the pinned dependency. |
| [Recorded launch definitions](878-session-launch-definition.md) | Forward-only persistence conversion and the Resume/fork contract. |
| [Reliable delivery](907-reliable-delivery.md) | Failure analysis and the durable queue/submission-confirmation design. |
| [Native fork](fork-chat.md) | Copy-on-write conversation probes and why fork must materialize a new key. |
| [Linux/remote integration](982-linux-remote-compat.md) | The 0.18.0 compatibility rebase, private-file credential decision and unified signed publication contract, with its dated automated results and native gaps. The earlier local handoff is preserved separately as [validation history](../../tests/archive/823-linux-delivery-history.md). |
| [Maintenance history](maintenance-history.md) | Actual outcomes and task history extracted from deleted small-change plans; it is not a plan to implement. |

The 2026-10-22 cleanup retained 41 of 94 existing implementation records and consolidated embedded history separately. Validation runs and dedicated implementation/review logs are preserved. A former plan is not retained merely because its issue closed or another archived document links to it.
