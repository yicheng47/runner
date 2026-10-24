# Codex native input-attention adaptation

## Starting point and outcome

The user explicitly requested a suitable Runner crew to implement this fix on 2026-10-23. Use the existing **Peer coding** crew: `coder` implements and `reviewer` reviews and performs selected live validation. Work only in `/Users/lr/我的/runner/.worktrees/fix-codex-input-attention`, branch `fix/codex-input-attention`, based on `origin/main` at `0696c9e1bb163052ecd2f90f40b7e9f3f5782010`. Root stays on main. No product edits have been made yet; this committed brief is the handoff.

Codex currently reports Working/Idle/completion through hooks, but Runner intentionally does not map raw PermissionRequest or question tool attempts to human waits. The user sees a real Codex approval/question while Runner still shows Working. Fix Codex attention using a proven native signal and reuse existing Needs you/sidebar/header/background-notification surfaces. The user explicitly said existing status icons are sufficient: no sidebar pulse, no redesign, and do not reopen #983 (closed as not planned).

## New evidence to verify first

The local CLI reports `codex-cli 0.163.0`. The official tagged source at `openai/codex`, `rust-v0.163.0`, now emits OSC 7501 unconditionally when stdout is a terminal:

- `codex-rs/tui/src/terminal_program_status.rs`: emits `ESC ] 7501 ; state=idle|working|blocked:app=codex ESC \\`, and `state=clear` on cleanup. No capability probe is required by this version.
- `codex-rs/tui/src/chatwidget/status_surfaces.rs`, `desired_program_status`: chooses Blocked from `terminal_title_requires_action`, otherwise Working or Idle.
- `codex-rs/tui/src/bottom_pane/mod.rs`, `terminal_title_requires_action`: derives action-required state from unanswered questions or an active view requiring user action.

Inspect those exact official upstream files online and confirm their contract before relying on them. Do not inspect other local repositories. This is newer evidence than #861's audit of Codex 0.162.1, which found no support. The coarse signal does not distinguish approval from question, so use existing generic Needs you / WaitReason::Unknown unless stronger proven evidence is available. Never fabricate an Approval needed / Answer needed distinction from an attempted tool or a timer.

Related: #347's deliberate surfaced-interaction boundary, #861 (broader program-status work), #904 (terminal notifications). Do not implement or claim to close the full scope of #861/#904.

## Read first

- `AGENTS.md`, `CONTRIBUTING.md` and `docs/arch/runtime-integration.md`.
- `crates/runner-daemon/src/runtimes/codex/{codex_status.rs,terminal.rs,mod.rs}`.
- `crates/runner-daemon/src/session/{pty_runtime.rs,runtime.rs,state/}` and `session/manager/{output.rs,mod.rs}`.
- `crates/runner-app/src/notifications.rs`, `ui/agent_status.rs`, and existing sidebar rollups; these should normally be reused.
- `docs/guide/reference/supported-agents.md`, `docs/tests/regression/{runtimes,missions,settings}.md`.

## Acceptance and boundaries

1. A fresh native Codex blocked report produces one stable Needs you interaction visible through existing session status and sidebar/header consumers. Repeated blocked reports must not duplicate notifications or reset interaction identity/age.
2. Native working/idle/clear reports resolve the native attention correctly; approval, answer, rejection, cancellation, session switch, resume and process exit must not leave a stale wait. Do not infer resolution from a keystroke alone.
3. Preserve hook-owned work, completion, failures, compaction, cancellation and conversation identity. Clearing native attention must not synthesize successful completion or unread responses. Hooks and native reports may interleave: test stale/late events and retain a current native wait across unrelated hook updates.
4. Parse bounded structured OSC messages across arbitrary PTY chunk boundaries with BEL/ST terminators as appropriate. Reject malformed, unrelated-app, oversized and control-containing reports. Avoid a broad new protocol/framework; pick the smallest sound integration into the existing authoritative daemon path. Do not turn an app-side snapshot/replay into a fresh attention event.
5. Work without the relevant pane/client being mounted. Maintain coherent direct-chat and mission status publications and delivery readiness. Since human waits also affect crew delivery, review this interaction explicitly; do not silently expand baseline detection or claim protection without evidence.
6. Older Codex or customized sessions that emit no usable native report retain current behavior. Other runtimes' semantics remain unchanged. No config/auth/trust changes, no agent upgrades, and no switch to hosting Codex through app-server.
7. Update `docs/guide/reference/supported-agents.md` and relevant architecture/regression expectations with the exact supported version/signal and generic-state limitation. Audit of other adapters is informational only: Claude/Copilot have approvals/questions, pi extension prompts only, Grok permission prompts, Antigravity/Cursor/TRAE lack equivalent coverage, and OpenCode's current bridge maps working/ready/idle but no wait. Do not claim those agents have been live-tested by this mission.

## Validation and handoff

Before running checks, list changed behaviors and why each selected check covers them. Use this worktree's own Cargo target directory. Add meaningful parser, reducer and publication integration tests for native wait/clear, duplicate reports, chunking, hook interleaving, malformed input, older-session fallback, no synthetic completion and preservation of independent drafts. Run relevant runner-daemon/runner-terminal tests, formatting and diff checks, and required Clippy/workspace CI gates per the repository rules. Do not run the whole live regression suite.

Coder hands the working-tree diff and results to reviewer before implementation commits/push/PR. Reviewer does not edit product code. After review is clean, reviewer performs bounded live Codex approval and Plan-question checks, accept/reject/cancel and one background notification/withdrawal scenario, adapting RT-STATUS-06, RT-STATUS-11 and SET-NOTIF-01 to the supported native signal. A daemon/publication integration test should cover mission propagation; add a bounded live mission only if necessary for a changed path. Use the isolated development app and own test sessions under AGENTS.md's standing authorization. Coders do not run the app. Preserve evidence and state platform/runtime gaps honestly; unavailable prerequisites are not passes.

## Authorization

Exactly the existing two-member Peer coding crew is authorized. No extra missions, slots or internal subagents without asking. Authorized work: implementation, tests, reviewer-led selected development-app checks, focused commits, pushing this branch to `origin` (ruizer/runner), opening a PR against `yicheng47/runner:main`, and driving required macOS/Windows CI green. No new issue is required merely to reuse existing status visuals. Never operate installed-app chats/missions or user-owned development sessions, change global agent configuration, merge a PR, release, cut a nightly, archive this mission, delete branches or remove worktrees.

The operator commits this brief before launch as required; fold it into the final focused implementation commit rather than retain a brief-only commit. Verify explicit Git author name/email before each commit. Review and CI corrections are folded into the commits they fix; use force-with-lease after authorized history rewrites, never plain force. Do not add tool-specific coauthor trailers. Stop at an open PR with required CI green and report PR URL, final SHA, review verdict, test results, native evidence and remaining limitations through Runner.

Mission participants finish their turn when waiting and consume pushed inbox notifications with unfiltered `runner msg read`; they must not start inbox watchers or polling loops. This external operator has no documented host facility for automatic per-line event delivery while idle, so it must report that watch limitation honestly. Do not install tools or change agent configuration to create a watch facility.
