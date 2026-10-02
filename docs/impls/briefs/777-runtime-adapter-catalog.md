# 777 PR 3 — Runtime adapter: catalog surfaces and the app

Implement PR 3 of 3 for [P1 #777](https://github.com/yicheng47/runner/issues/777): phases 3 and 4 of the spec. PR 1 ([#779](https://github.com/yicheng47/runner/pull/779)) moved identity and argv into `runtimes/`, and PR 2 ([#780](https://github.com/yicheng47/runner/pull/780)) moved spawn hooks, trust, key capture and status watchers. This PR moves what is left: the catalog surfaces in the backend and the per-runtime branches in the app. When it lands, adding a runtime touches only the files the spec lists under "After the change". This is a refactor, and behavior stays byte-identical.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-777-runtime-adapter-catalog`, on branch `refactor/777-runtime-adapter-catalog`. The mission's directory is this worktree. Its tip is this brief, on top of main `bc4ac945` (PR [#776](https://github.com/yicheng47/runner/pull/776), the keyboard create feature, which reworked `start_chat.rs`). Do not create another worktree, touch the root checkout or another worktree, or share another worktree's target directory. If main moves, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/777-runtime-adapter.md`**: Proposal, the "Where each concern moves" table, Rules, Decisions, phases 3 and 4, and Verification. The spec wins over the issue and over this brief on any detail.
- `runtimes/mod.rs`: the trait as PR 2 left it, `adapter`, `for_key`, `NoAgent` and `StatusHooks`; `runtimes/catalog.rs`; `runtimes/*/mod.rs`.
- The goldens from PR 1 and PR 2: `session/manager/tests/golden.rs` and `expectations/`.
- The remaining sites. These counts are production matches of `Runtime::<agent>` outside `runtimes/`, not counting tests.
  - **Backend:** `runtime_defaults.rs` `runtime_defaults` (6); `runtime_status/models.rs` `DISCOVERY_RUNTIMES`, `request` and `source` (12), plus `models/{antigravity,claude,codex,pi}.rs`; `runtime_status/versions.rs` `dist_tag` (Claude's channel); `usage.rs` `UsageSnapshot` and the fetch `match` at about line 189; `skills.rs` `skill_catalog` and `read_entry`; `ops/skills.rs` `set_global_enabled_at`, `set_copilot_enabled_at` and `set_codex_enabled_at`; `ops/mcp.rs` `McpClientId` (`parse`, `for_runtime`, `runtime`, and the per-client status and write functions).
  - **Codex Speed:** `session/manager/spawn.rs` at about lines 806, 925 and 1509; `ops/role.rs` `create` and `update`; `ops/slot.rs` `update`, plus the `runtime == "codex"` checks at about 1194; `ops/session.rs` `session_start_runtime_with_speed`; `runner-cli/src/output.rs` at about 214.
  - **App:** `surfaces/app_shell.rs` usage functions (23: `usage_installed`, `unavailable_line`, `weekly_usage_percent`, `visible_usage_runtimes`, `usage_pill_element`, `usage_section` and `render_usage_popover`); `settings/skills.rs` (`catalog_caption`, `supports_global_skill_toggle`, `render_modal`); `app_store/skill_defaults.rs` `root_runtimes`; `chat_icon.rs` `for_runtime`; `start_chat.rs` `effort_needs_launch_model`.
  - **App Codex Speed, string comparisons:** `runtime == "codex"` in `crews/popup.rs`, `crews/logic.rs`, `start_chat.rs` and `roles/detail.rs`. The spec's grep misses these, but they are the same per-runtime branching.

## Deliverable

### Step 1: more goldens, before any code moves

1. **Record goldens on untouched `bc4ac945`** for this PR's surfaces, in new expectation files; never edit PR 1's or PR 2's. At minimum:
   - **Native defaults:** what `runtime_defaults` reads for each runtime from a temp home seeded with representative config files, plus an empty one.
   - **Model discovery:** the command, args, env and config home each runtime's discovery would use, and which runtimes discover at all.
   - **Versions:** the npm package and dist-tag per runtime, including both of Claude's channels.
   - **Usage:** which runtimes fetch usage, and with which command and env.
   - **Skills:** the catalog roots, and toggle support per runtime. Also what each global toggle writes into a temp home (Codex config, Copilot `disabledSkills`, Claude overrides).
   - **MCP:** each client's config path and format, its status read and write against a temp home, `McpClientId`'s serde form and `parse` accept and error strings, including `claude_code`.
   - **Codex Speed:** the launch args for each speed and role/slot override, and the speed fields persisted on role, slot and session rows.
2. **Coverage map.** Every per-runtime decision this PR moves, app ones included, must be covered by a golden or an existing test before it moves. App-only facts (icon, tint, captions, usage labels, the effort rule, which runtimes show Speed) need app tests that pin today's values. Put a table in the handoff mapping each moved site to its test.
3. **Commit this step alone, locally,** green on untouched `bc4ac945`. From then on every expectation file stays byte-identical. If one has to change, stop and ask through Runner.

### Step 2: the move (phases 3 and 4)

4. **New adapter members, per the spec table:** `native_defaults`, `model_discovery()`, the npm dist-tag (a `catalog()` field or default), `usage()`, `skills()`, `mcp()`, and Codex Speed as a catalog capability with its args in `launch_args`. Every default means "not supported", and `NoAgent` takes them all. Discovery, usage and skill-toggle code moves into the owning `runtimes/<name>/` module. `runtime_status/models/<name>.rs` moves into its module with `git mv`, and its tests move with it unchanged. Mechanisms shared by several runtimes stay shared helpers, and no adapter calls another adapter.
5. **`UsageSnapshot` becomes a map keyed by `Runtime`,** for both the value and the error. It is not serialized, so only the app changes; the pill, the popover order and the copy stay identical.
6. **`McpClientId` becomes a thin wrapper over `Runtime`,** taking its wire name from the adapter's `mcp()`. Its serialized form and its `parse` strings stay byte-identical, `claude_code` included, so persisted app settings and CLI output do not change.
7. **App, phase 4.** Add one `runtime_ui(Runtime) -> RuntimeUi` table in `runner-app` for icon, tint, the skills caption and usage labels, and drive `chat_icon`, `settings/skills.rs`, `skill_defaults` and the usage pill and popover from it plus the catalog. Everything that is behavior comes from the `RuntimeCatalogEntry` the app already receives: usage support, toggle support, Speed support, and `effort_needs_launch_model` as a catalog capability. Permission modes already read the adapter; keep them that way.
8. **The CLI.** `runner-cli/src/output.rs`'s Speed column reads the catalog capability instead of comparing keys.
9. **Remove the leftovers.** Remove any temporary `Option<Runtime>` shim PR 1 or PR 2 left. `Runtime::Shell` checks become `runtime.is_shell()` where the spec says so. At the end, `rg 'Runtime::(Codex|ClaudeCode|Antigravity|Pi|Copilot|Trae)\b' crates -g '*.rs'` outside tests matches only `runtimes/`, `runner-core`'s identity table and the app's `runtime_ui`. `rg '== "codex"'` and the other keys outside tests match nothing per-runtime.
10. **Docs, same diff.** Rewrite "Where the implementation lives" in `docs/arch/runtime-integration.md` around `runtimes/<name>/`. Point `docs/arch/arch.md`'s references to the adapter in `router/runtime.rs` at `runtimes/`, about lines 164, 389 and 458. Add a short "Adding a runtime" checklist matching the spec's "After the change". If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.
11. **The stub runtime check, from the spec's Verification.** After the review is clean, create a local throwaway branch `scratch/777-example-runtime` from the branch tip, in this same worktree with a clean tree. Add a minimal `Example` runtime there: a `runtimes/example/` module, one variant with its identity row, one adapter arm, one `runtime_ui` row, an icon and a test. Confirm it compiles, passes workspace tests and appears in the catalog the app reads. Save `git diff --stat refactor/777-runtime-adapter-catalog` and the full patch to `/private/tmp/777-example-runtime.{stat,patch}`. Switch back, and leave the scratch branch unpushed and undeleted. Every file in the stat must be on the spec's list; anything else is a finding to fix on the real branch.

Out of scope: renamed items, fields, columns or wire names (`codex_speed`, `claude_code` and so on keep theirs), new runtime features, and any behavior change, including bugs found on the way. List those in the handoff. Design files and both READMEs stay untouched.

On 2026-10-03, Jason explicitly authorized folding the pre-existing New Chat model-chooser Enter bug into PR #792. This narrow exception makes Enter open or select model suggestions without submitting the chat; Cmd+Enter (Ctrl+Enter on Windows) still submits. Record the deviation, regression test and affected-surface recheck in the handoff. Every expectation file remains byte-identical, and the rest of this brief's scope and workflow still apply.

## Validation

Run each of these and report its exit code:

- `cargo test --locked --workspace --profile ci --no-fail-fast`, recording the passed and ignored counts at `bc4ac945` and at the end.
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and the same for `-p runner-app --features updater`.
- `cargo fmt --all --check` and `git diff --check`.
- `git diff <step-1 commit> -- <all expectation files>` prints nothing, under both `TMPDIR=/tmp` and `TMPDIR=/private/tmp`.
- The two greps in item 9, with their output pasted in the handoff.
- `git log --follow` on two moved files shows their history.

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`. Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`, because Windows clippy fails on unused ones. Watch `cfg(windows)` paths in moved code.

The coder and reviewer never run the dev app; only QA does, under Live check below. Nobody starts, stops or types into Jason's Runner apps, chats or missions. Never open a Runner database under `~/Library/Application Support/com.wycstudios.runner*`, and tests use temp homes for every config read or write. Tests never launch real agent CLIs; discovery and usage goldens record the planned command, they do not run it. Native Windows is unavailable; say what is unverified there.

## Live check (QA)

Jason authorized QA to run the dev app for a view-only before and after comparison of the surfaces this PR moves. The goldens and app tests pin values; this pass catches a screen that renders differently.

- **Setup.** QA builds this worktree and runs its development app with `make run`, using development data only, and drives it with computer use. If a development app is already running when QA starts, ask Jason through Runner instead of quitting it. Evidence goes to `/private/tmp/777-qa/before/` and `/private/tmp/777-qa/after/`, never into git.
- **Surfaces.** Screenshot each at the same window size: the sidebar's chat icons and tints, with one existing chat per runtime where the dev data has one; the usage pill and its popover; Settings → Agents, Skills and MCP, scrolled through; the Speed control on a Codex role's page and its absence on a non-Codex role's page; Start a chat with a Codex role picked and then a non-Codex role (cancel, never start); a crew slot popup for a Codex slot and a non-Codex slot (cancel, never save). If the dev data lacks a role or crew needed here, QA may create ones named `qa-777-…` and deletes them at the end.
- **Before.** On the untouched tip (this brief on `bc4ac945`), before the coder edits anything. The coder reads and plans meanwhile and starts step 1 when QA reports the baseline.
- **After.** On the branch tip, after the clean review and item 11. Compare per surface: the same runtimes, order, icons, tints, labels, captions, toggles shown, and Speed shown or hidden. Usage numbers and model lists may differ between the runs; their presence and layout may not. A difference is a must-fix for the coder unless it comes from data that changed between the runs, in which case say so.
- **Limits.** View only: no skill toggles, no MCP register or unregister, no saved settings, no agent chats or missions, and no changes to pre-existing chats, missions, roles or crews. Toggles and MCP writes go to the real `~/.claude`, `~/.codex` and `~/.copilot` configs and stay with Jason. Do not touch the installed Runner app, authentication or permission settings, and do not open Runner's SQLite database. Quit the dev app QA started after each pass.
- **Record.** Write `docs/tests/777-runtime-adapter-catalog.md`: per surface, the before and after result with the evidence file names, plus what was not checked and why. If computer use cannot reach the dev app after two tries, QA says so through Runner and records what it could, and the mission continues; the unchecked surfaces go to Jason's manual check.
- If a fix after the after pass touches app UI code, QA re-checks only the affected surfaces.

## Crew handoff and authorization

The coder owns implementation, tests and fixes, and hands off to QA at the two points in Live check. The reviewer waits for an explicit Runner handoff, then reviews the whole branch diff against the spec and this brief, posting must-fix findings first with file:line pointers. It checks in particular that:

- the goldens were committed first, pass on untouched `bc4ac945`, and no expectation changed afterwards;
- the coverage map leaves no moved site uncovered, app sites included;
- usage, skills, MCP, discovery, defaults, versions and Codex Speed behave exactly as today for every runtime, Shell and unknown keys included;
- `McpClientId`'s wire form and the persisted rows are unchanged;
- no `cfg` attribute was lost, and no moved test changed or disappeared;
- the greps in item 9 are clean.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`, then run item 11 and send its stat to the reviewer, then hand the branch tip to QA for the after pass. No extra agents, crews or subagents.

After the clean review and QA's after pass, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, the goldens commit, this brief and QA's test record included, into a single commit on top of `main`. Use an imperative subject such as `refactor(runtime): move catalog surfaces and app runtime branches into runtime adapters`, with no co-author trailers.
- **Push** with `git push -u origin refactor/777-runtime-adapter-catalog`. Never push the scratch branch.
- **Open the PR** with `gh pr create --base main`. The body carries:
  - `Closes #777`;
  - a summary;
  - test evidence: the new golden matrix, the coverage map, before and after counts, and the grep output;
  - the stub runtime's file list;
  - QA's before and after table, and what it could not check;
  - a manual check for Jason, covering what QA does not. On Codex, Claude Code, Antigravity and pi: a chat and a mission slot each, a resume, and a model and effort override. Settings → Skills (each global toggle) and MCP (register and unregister per client). Any surface QA could not check. Nothing should look or behave differently;
  - what is unverified;
  - no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete any branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It gives the PR URL and CI result, QA's result, changed files, checks with exit codes, the stub runtime's file list, any spec deviation, bugs found but not fixed, what is untested or unverified, and the reviewer's verdict. Then the crew stands by.
