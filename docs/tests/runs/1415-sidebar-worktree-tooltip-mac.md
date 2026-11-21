# #1415 Sidebar worktree tooltip live QA

## 2026-11-21 macOS

**Verdict: Passed.** All required macOS live checks passed on the reviewed candidate.

The approved neon trio's QA role ran the three checks from the #1415 mission handoff under the standing development-app authorization and held the shared QA lock through cleanup. The run started at 17:26 CST (UTC+08:00).

- Repository `yicheng47/runner`, branch `feat/1415-sidebar-worktree-tooltip`, reviewed candidate `fa4eb2f47d3a70dffa009fdc067fcf40e5a8987e`, base `e0dfbe829731e8b3407c690b837e4b68fd1e018a`. The working tree was clean at build time. The empty diff's SHA-256 was `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- macOS 26.6.2 (25G83), arm64. Runner 0.20.2 (dev), development CLI 0.20.2, Rust and Cargo 1.97.1, Apple Git 2.54.0. Two pi 1.6.0 chats ran at `deepseek/deepseek-flash`, low effort, with the existing account, permissions and extensions. No prompts were sent.
- Built using `make run` and launched the byte-identical `Runner 1415 QA.app` wrapper under the scratch root, bundle ID `com.wycstudios.runner.qa1415`. App SHA-256 was `da011d65ac3579ff1c88fecbdd10cfe16560ba27420c07fc2273e3fe5ce22744`. The built CLI, wrapper CLI, installed development CLI and running daemon all matched SHA-256 `056a24fd8517046aca463a7ad13418d65381c56b792c4e770394b3f8355544bd`. Process paths and candidate CLI status verified the wrapper and the `com.wycstudios.runner-dev/runnerd.sock` endpoint. Crew and session environment variables were cleared for the development app and CLI.

### Method and evidence

Bound native computer use to the wrapper path and verified control by opening and dismissing New tab. Started two runtime-only chats through the absolute development CLI in a disposable Git main checkout and its linked worktree. Candidate CLI session state and `git worktree list --porcelain` confirmed their paths and branch. In Projects, the unassigned chats appeared in Recents. In Active, they appeared under Agents. Positioned the pointer over each row body with the documented macOS move-only helper and captured the resulting native window. The main-row samples waited at least one second, and the row highlight confirmed that the hover landed.

Evidence root is `/private/tmp/runner-1415.6RWYaJ/evidence`. It contains the screenshots below, pointer coordinates, Git and session identity, binary hashes, daemon status and cleanup records. Automated validation is tracked by the implementer. This run covers macOS sidebar tooltips on direct chat rows.

| Check | Platform / runtime / variant | Result | Evidence |
| --- | --- | --- | --- |
| Linked worktree in Projects | mac / pi / Recents | Passed | `projects-linked.jpg` shows the full `qa/full-sidebar-tooltip-branch` on one line with no checkout path below it. |
| Linked worktree in Active | mac / pi / Agents | Passed | `active-linked.jpg` shows the same single-line branch tooltip, even though the row's location subtitle is truncated. |
| Main checkout | mac / pi / Projects and Active | Passed | `projects-main.jpg` and `active-main.jpg` show the hovered main row without a worktree tooltip. |

### Cleanup

Archived the two owned chats through the candidate CLI, which confirmed both were stopped and archived. Restored the starting Projects view, quit the wrapper and stopped the development daemon started for this run. The development daemon had resumed existing saved sessions at launch, and QA did not send input to, stop individually or archive any of them. Verified the bare app, wrapper, daemon and both owned pi processes had exited, unregistered the wrapper and released the QA lock. The scratch repository, build copies, evidence and conversation history are retained. No authentication or agent permission settings were edited.
