# Full Windows PC regression — 2026-10-06

## Goal and checkout

Run the complete Windows regression on JASONPC against a fresh worktree build. The installed nightly hosts the crew only; it is not the test target. Deliver evidence and reproducible findings. Do not fix product bugs or weaken tests.

Branch: `qa/2026-10-06-pc-regression`. Worktree: `C:\Users\ROG\repos\yicheng47\runner\.worktrees\qa-2026-10-06-pc-regression`. Frozen product baseline: `ca2ec282263adae1d799523bc9c929c51cafc94a` (`origin/main`), before this docs-only brief commit. Every slot uses this worktree despite role-default cwd. Preserve the root checkout's `fix/742-windows-file-paste` branch and other worktrees. Keep a separate local `target/`; no shared `CARGO_TARGET_DIR`.

## Directions and scope

Read this worktree's `AGENTS.md`, `docs/tests/full-smoke-test.md`, `docs/arch/windows.md` and `docs/arch/process-model.md`. Discover current and archived test plans. Current Cargo/specs govern historical instructions: the core is now `runner-daemon`, not `runner-backend`; Tauri/React commands are retired.

Use full-smoke as the runtime procedure, then add the UI/installer coverage it omits. Include the applicable #645 terminal/daemon/lifecycle, #777 catalog/runtime, #791 state reducer, #783 cancellation, #766 Codex idle, #799 pi draft, #753 inbox, Antigravity, #610 Windows hooks, #624 details, #687/#688 fallback/startup, #64 terminal, M6.1 input/IME, M6.6 resize, #648 CLI/install/skills, #686 watch and #772 keyboard plans. Honor documented unsupported behavior and platform exclusions.

## Candidate and isolation

Create a unique canonical scratch/evidence root outside the repo, separate runtime/startup folders, and an ID/PID ledger for all test objects. Record OS, tools/runtime versions, source SHA, branch and dirty status. Use native Windows paths and UTF-8.

Build here using `.\make.cmd build`, with CLI/daemon sidecars and ConPTY. Launch the exact `<worktree>\target\debug\Runner.exe`. Use private child-only `APPDATA=<scratch>\appdata`, producing `<scratch>\appdata\com.wycstudios.runner-dev`. Omit `NO_COLOR` and inherited crew identity (`RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG`) from external test app/CLI children. Never modify the crew's own environment or global settings. Inspect startup PATH/skill auto-install and disable it using supported private test settings if necessary; surface a blocker if global writes cannot be isolated.

Dev pipes remain fixed despite private APPDATA: `\\.\pipe\com.wycstudios.runner-dev` and `\\.\pipe\com.wycstudios.runnerd-dev`. Check for pre-existing owners before launch; do not stop or attach to another dev instance. Keep installed Runner/runnerd and this crew untouched.

Before live checks, prove the target using executable path, binary SHA-256, compiled build identity, selected window, daemon executable and absolute development CLI `status --json` endpoint. All binaries must come from this candidate/private sidecars. Use `<worktree>\target\debug\runner-agent-cli.exe` or the private installed dev sidecar for external test operations with clean child identity. Never use bare `runner` for product tests. Installed CLI commands coordinate this crew only. Never open SQLite directly.

## Roles and execution

Coder leads prerequisite inventory, coverage planning, candidate preparation, automated evidence and the report. Reviewer audits coverage/isolation before testing and final evidence afterward. QA owns full regression execution and the result matrix. This is validation, not product implementation. Workers wait for directed handoffs. One slot owns Cargo at a time; keep the candidate stable and reuse valid logs.

After clean method review, run all Windows workspace gates with real exit codes, counts/ignored tests and full logs: `cargo check --locked --workspace --all-targets --profile ci`; `cargo test --locked --workspace --no-fail-fast --profile ci --timings`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`; `cargo fmt --all --check`; `git diff --check`. Run documented installer fixture tests after verifying their temporary identity, plus supplemental documented targets absent from the suite. Retain failures even when a retry passes; explain baseline/environment/flaky attribution with evidence.

QA independently verifies Computer Use in its own session: initialize bundled `@oai/sky`, discover/capture the candidate window, then harmless open/dismiss. The parent just successfully captured installed Runner and opened/dismissed New chat: capability here only, not candidate QA or proof of worker access. Read the installed skill and required guidance, use supported APIs and obey exclusions. Do not automate ChatGPT/authentication/prohibited terminal or agent UI. Mark denied/inaccessible required checks Blocked and request exact assistance; never substitute private PTY/DB writes or CLI success for live UI proof.

Execute full-smoke's entire Windows matrix for Codex, Claude Code, Antigravity, Copilot and pi, including fresh/second turn, overrides, tools, interrupt/recovery, stop/resume/key continuity, new conversation/resume and single-lead startup in fresh canonical folders. TRAE is optional when inaccessible. Use existing accounts, bounded smoke history and inexpensive supported overrides; preserve global auth/permissions/extensions. Startup must produce exactly one marker in the requested cwd, visible ACK, captured key and Idle without manual submission or trust intervention.

Also cover tabs/panes/focus/multi-window, shells/drawer, scrollback/reflow/resize, paste/multiline/drafts/IME, status/hooks/fallback/cancellation, routing/inbox/questions, lifecycle/persistence, settings/keyboard, isolated command/skill fixtures, and #645 daemon keep/stop/reconnect/recovery/notices. Fault-inject/relaunch only test-owned candidate processes. Mac-only checks are outside PC scope; unavailable required Windows legs remain Blocked. Reproduce findings minimally and preserve evidence; do not rewrite expectations or edit source.

## Record and cleanup

Write `docs/tests/2026-10-06-pc-full-regression.md`: source/build/isolation, authorization, automated commands/exits/counts, runtime and UI-family matrices, evidence paths, severity/repro/expected-vs-actual findings, cleanup and exact remaining steps. Rows are Passed, Failed, Blocked or Skipped with reasons. Overall Pass requires all required Windows checks. Separate CI, fixtures, CLI proof and live UI evidence. Keep large/sensitive logs/screenshots/history in scratch; commit only sanitized small evidence. Markdown paragraphs occupy one line.

Reviewer audits the report and spot-checks evidence; QA confirms reported results. An accurate Fail/Incomplete record is valid evidence, not product-readiness approval. If blocked, surface the gap, finish unaffected checks and retain an Incomplete record rather than silently reducing this to unit tests.

Use the ledger to stop/archive test chats/startup missions and stop owned followers/candidate app/daemon only. Verify no test-owned live sessions remain. Preserve history, scratch evidence and roles/crews (crew deletion destroys archived metadata); list retained objects. Never disrupt pre-existing sessions or the coordinating mission.

## Authorization

Jason requested this full PC regression mission/worktree and confirmed the target is the worktree build. Authorized: these three slots, bounded real-agent chats/startup smoke missions from the procedure, existing-account smoke history, private test settings/data, candidate build/launch, isolated installer fixtures and test-owned cleanup above. No extra coding/review agents, product fixes or unrelated workstreams. Destructive commands and real global/security/auth changes remain gated by applicable instructions.

Commit the brief before start. Deliver one branch commit including brief/report: amend the brief commit, push, open a PR against `main` and drive configured CI green on macOS and Windows. Label Fail/Incomplete honestly; never claim a product pass from gaps. Stop at the open PR. Never merge, release, delete branches/worktrees or edit product source. Later report/CI-document fixes amend the same commit and use force-with-lease after push. Ask through the mission if required checks need human/host help.

## Human scope amendment — 2026-10-06, 14:05 UTC

The original frozen-baseline phase reached reviewed report PR #812 with an honest Incomplete Windows verdict; that report-only head eventually received green configured macOS and Windows CI. At 14:05 UTC Jason requested fixing the bugs found and retesting in this same mission (Runner event `01M48RHKY3Q9TND34B7HPT2CFJ`). This amendment supersedes the original prohibition on scoped product fixes; retain the original baseline, failures and evidence rather than rewriting them as passes. Continue in the assigned branch/worktree with the same three slots and publication boundary: one reviewed commit in the open PR, configured CI green, no merge, release, extra checkout/agent or global configuration restoration.

Address F1's test scheduling budget without changing router ordering, F2's Git-for-Windows hook utility lookup and F4's ConPTY reader/master teardown. Correct F5's unsafe pi account-directory test method rather than attributing an unproven settings field/writer to Runner. Diagnose F3's supplementary socket latency and the external-console observation before asserting a cause or fix. Jason excluded the full 15-minute idle check; keep it Skipped. Required native, platform and account checks that cannot run remain Blocked, and the final product verdict remains Incomplete unless every required Windows check passes.

Jason's 14:17 UTC clarification (`01M48S7NA2JNDMSVRRBAWM9ZVS`) narrows the Escape/status investigation to Codex: Claude feels instant, Codex appears delayed. Before changing that behavior, report one session's actual input-write, hook append, rollout `turn_aborted` and Runner status-transition timestamps, distinguishing PowerShell startup cost from native interruption and status evidence. No Escape fix is authorized before that breakdown. Computer Use exclusions still apply; request human input where required.

At 14:26 UTC Jason superseded that diagnostic plan with his A/B evidence (`01M48SPYZ64C4S8N86C5FKKJR1`): in his normal Runner, the same Codex terminal responds instantly with `--disable hooks` and visibly stalls with hooks enabled; Codex in plain PowerShell responds instantly. Record this as F6, explicitly human-reported evidence rather than QA's fresh-candidate measurement. Drop the four-timestamp trace and private Codex login/manual-session plan for F6. Research exactly how Codex 0.159.3 launches Windows hook commands and return two or three fix options with expected cost and tradeoffs. Do not implement F6 in this mission or open its separate issue/spec before Jason selects an option. Fresh source review of F1/F2/F4 and QA's native Stop re-check remain required.

Jason temporarily brought F6 implementation into scope at 14:29 UTC (`01M48SWS1DYJ79539SATB8ERNH`), requiring options before code, a veto window, method review and then fresh source/QA validation. At 14:31 UTC (`01M48T154JY4X9474EVYSD5K1P`) he assigned the native F6 A/B to himself and removed the heavy timing/private-login protocol. No F6 product code was written: method review rejected a session-id-only projection and a required reporter with a 100 ms fatal startup deadline.

The final scope instruction at 14:46 UTC (`01M48TWJPZNJBE22SK6HZAM8MG`) supersedes that temporary expansion: **F6 is out of scope again; stop its method and implementation work.** Finish F1/F2/F4, the F5 procedure correction, QA's fresh-build native Stop re-check, the reviewed Incomplete report and the single-commit PR with configured CI green, then stop. Record F6's human A/B, PS5.1 cost table, exact 0.159.3 shell/MCP identity/startup findings and pwsh selection caveat; retain and name scratch research files for a follow-up mission tied to [#797](https://github.com/yicheng47/runner/issues/797). Jason's native F6 A/B is no longer an acceptance criterion. His response to the restricted-check assistance request was to record those checks Blocked and finish the Incomplete report PR; do not wait for or invent a restricted-check pass.

Coder owns implementation and Cargo. Freeze source after relevant checks, hand the complete diff and evidence to reviewer, then hand the same reviewed revision and rebuilt candidate to QA. Any follow-up source change repeats the affected review and regression scope. Existing baseline report review, QA and CI do not approve these fixes. Record final source/build hashes, test exit codes/counts, fresh review/QA verdicts and exact remaining gaps before amending the PR commit.
