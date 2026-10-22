# Role avatar feature validation — 2026-10-22

## Candidate and scope

- Base: `8462b519`, branch `codex/role-initials-ai-avatars`.
- Changed behavior: default initials, optional Codex-generated image preview/adoption, reset/cancel/failure ownership, SQLite image reference persistence, and saved-avatar propagation through role-backed views.
- This is feature validation, not a full regression or release pass.
- Existing root checkout stays on `main` and unchanged. No installed Runner data, existing chats/missions, or agent global configuration is modified.

## Selected checks

| Check | Reason | Result |
| --- | --- | --- |
| Initial extraction: lowercase, whitespace, Unicode, numeric and empty handles | Replaces the pixel grid with one deterministic character | Extracted production-function harness: 2 tests passed. Mutating the implementation to return the whole handle makes both fail. The same cases also passed in the runner-app suite. |
| Avatar draft failure/reset/replacement/cancel/stale-result ownership | A generation must never adopt an image before save or destroy the saved image on failure | Passed in runner-app automated tests |
| Create/edit generation save guard and restoration dirty state | Buttons add new async work to existing save/cancel behavior | Passed in runner-app GPUI tests |
| Old database migration and preserve/clear/replace semantics | Adds a nullable persisted image reference and protocol patch field | Passed: migration 0029 and role-avatar persistence/patch tests |
| Fake Codex output/error/timeout/path tests | Exercises process/output boundary without network usage | Passed: fake Codex success, invalid/missing output, failure, deadline and process cleanup |
| Role-backed avatar identity and renamed-slot mapping | Role and crew handles differ, and saved avatars must be shared | Passed in mission identity/current-role avatar unit tests |
| Native view inspection: role create/edit, role picker and crew avatar | Relevant identity/config portion of `MIS-CREW-01` and `WS-CREATE-01`; feature-specific preview/reset/cancel/failure checks supplement them | Not run; coders/reviewers do not launch the app. Do not count either full suite case as passed. |
| Windows native/compile check | Shared widget and subprocess code must remain portable | Pending/host unavailable |

IME, authentication, terminal input, unrelated chat lifecycle and mission message delivery are outside this feature's selected live scope.

## Real Codex CLI capability probe

One isolated, ephemeral capability probe used the installed `codex-cli 0.163.0` and existing authentication/configuration:

```text
codex exec --ephemeral --skip-git-repo-check --enable image_generation \
  --sandbox workspace-write -c 'approval_policy="never"' --json -C <private-tempdir> <native-image-only-prompt>
```

The prompt requested one illustrated square avatar using only the native image tool, with no drawing-by-code, package installation or alternate image API. The CLI exited **0** after **29.8 seconds**, but returned:

> Native image generation is unavailable in this session. I’ve stopped without using a substitute method; no `avatar.png` was created.

No PNG was generated. The local CLI feature list advertises `image_generation`, which does **not** establish native tool availability in the selected account/provider session. The existing configuration uses API-key rather than ChatGPT-token authentication. This is a capability limitation, not a successful end-to-end image-generation check. No provider/login/global-settings changes or alternate API calls were made.

The implementation must treat missing/invalid image output as an error even when Codex exits zero. Success-path coverage uses a fake executable; actual successful generation still needs a native-image-capable Codex session.

## Other completed checks

- Both English and Simplified Chinese TOML catalogs parse and contain all six new avatar keys.
- `git diff --check` passed during implementation; final gate pending.

## Final automated gates

- `cargo test --locked -p runner-core --profile ci`: exit **0**, **66 passed**, including avatar serialization/patch semantics and request-specific timeout checks.
- `cargo check --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders`: exit **0**, application production code compiles. The dependency's existing runtime-shader feature is a validation-only choice; no manifest/build configuration was changed for it.
- `cargo test --locked -p runner-app --profile ci`: blocked before project test compilation by the existing `gpui-pre-apple` build step: `xcrun: error: unable to find utility "metal"`. This host has Command Line Tools but no full Xcode/Metal compiler. The normal build gate is not passed.

- `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders`: exit **0**, **807 passed**, **0 failed**, **3 ignored** (existing paid Copilot/Pi smokes and manual terminal benchmark). Includes GPUI missing-image fallback/size/presence, form generation save/cancel/restore/new-role adoption, stale-source rejection, keyboard restoration, and mission role/alias/current-image mapping.
- The first new-role integration test attempt failed because the fixture typed into a form in the same update that created it, before its observer subscription activated. Setup now renders the form before typing, matching the existing harness and real user sequence. The product did not need a workaround for that test setup failure.

- `cargo test --locked -p runner-daemon --profile ci role_avatar -- --nocapture`: exit **0**, **18 passed**. Covers migration 0029; valid/invalid/missing/externally saved native PNG; typed-event enforcement; failed process; bounded prompt/output pipes; timeout, detached-descendant reaping and process-birth identity checks before cleanup; reference-safe and concurrent discard; role patch/replacement/clear/delete; adoption/discard transaction race.
- `cargo test --locked -p runner-daemon --profile ci ops::role::tests`: exit **0**, **38 passed**, validating that existing role create/update/delete behavior still works with the new transaction boundaries.
- `cargo test --locked -p runner-daemon --profile ci shell_path::tests`: exit **0**, **13 passed**, covering the existing discovery/environment mechanism that now captures `CODEX_HOME`.
- `cargo clippy --locked --workspace --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings`: exit **0**.
- `cargo fmt --all -- --check`: exit **0**.

- `cargo test --locked -p runner-cli --profile ci`: the unit suites and daemon-client integration suite passed, then the golden suite reported **77 passed / 11 failed**. Ten failures were introduced by serializing absent `avatar_path` as null (an extra `avatar_path -` text row); the optional field now omits absent values. The strengthened core serialization test failed before this fix and passed after it.
- Rebuilt `runner-cli` and the golden harness, then reran the **11 exact failing golden cases**: **10 passed / 1 failed**. The remaining `chat_start_runtime` failure is the same error-precedence/environment mismatch both before and after the serialization fix: this host discovers an installed Codex and reports `working directory does not exist`, while the fixture expects `Codex executable was not found`. The CLI golden fixture and runtime-start/error-resolution code are unchanged by this feature (the synthetic Role only gains `avatar_path: None`). Do not claim a full CLI-suite pass; this unrelated runtime-start golden difference remains recorded.
- `cargo test --locked -p runner-daemon --profile ci schema_snapshot_matches_migrations`: exit **0**, **1 passed**.
- `cargo test --locked -p runner-daemon --profile ci migration_0028_preserves_every_row_column_and_mission_link`: exit **0**, **1 passed**, proving the preceding migration preservation contract still holds.
- After the serialization fix, reran core tests (**66 passed**), the full runner-app suite (**807 passed / 3 ignored**), all-target workspace Clippy with updater/runtime shaders (**exit 0**), and formatting (**exit 0**).

Native QA and real successful image generation remain separate gaps. Normal Metal compilation and Windows native testing are unverified on this host.


## Follow-up: selectable initials / pixel art

User requested both base styles as persistent choices. Scope adds style selection/preview, create/update/reopen/cancel behavior, AI-to-base transitions, and propagation through existing role-backed views. Pixel rendering reuses the original algorithm; a missing AI image falls back to the selected base style. Migration 0030 upgrades both old role databases and databases that already applied avatar-path migration 0029. Initials remains the default and is omitted from serialized default Role objects to preserve existing CLI output.

Selected verification: mirrored pixel determinism; both base-style image fallbacks and presence geometry; create/edit style save/reopen/cancel and AI preview switching; missing-field/default serialization and omitted-update preservation; migration 0029-to-0030 and schema snapshot; current-role style mapping across crew aliases and missions. Use the existing runtime-shader test option on this host; no new native image probe, unrelated CLI runtime smoke, or global agent-configuration change is part of this follow-up.

Follow-up results (all commands exited **0**):

| Command / check | Result |
| --- | --- |
| `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | **815 passed, 0 failed, 3 existing ignored** |
| `cargo test --locked -p runner-core --profile ci` | **66 passed**; defaults, omitted fields and style patch serialization covered |
| `cargo test --locked -p runner-daemon --profile ci role_avatar` | **17 passed**; includes style-only patches preserving an AI image and explicit switch-back to initials |
| `cargo test --locked -p runner-daemon --profile ci ops::role::tests` | **38 passed** |
| `cargo test --locked -p runner-daemon --profile ci repo::role::tests` | **6 passed**, including pixel/default persistence round trips |
| `cargo test --locked -p runner-daemon --profile ci migration_0029_to_0030` | **1 passed**; old image path preserved, old roles default to initials, style and image reference round-trip |
| `cargo test --locked -p runner-daemon --profile ci schema_snapshot_matches_migrations` | **1 passed**; snapshot updated to migration 0030 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

The first form-test run incorrectly looked for test-only library render selectors from the Runner binary's non-test dependency; even the default initials selector was absent there. Form tests now assert the draft/save state after real mouse/keyboard interactions; library tests still directly verify both rendered fallback tiles and presence/size. No product workaround was added for this harness issue. The new enabled style buttons appear before Name, so backward keyboard traversal from an empty Name now reaches Pixel, Initials, then the breadcrumb; this order is explicitly tested.

No new native-image request or unrelated CLI golden/runtime sweep was run. Native-window/Windows acceptance and the existing real image-generation limitation remain unverified as described above.


## Follow-up: role theme colors and editable image-prompt dialog

### Changed behavior and selected checks

The screenshot is used only as a palette/layout reference. The eight named role colors, Custom hex and Automatic are form-local choices until save. They affect role avatars, fallback tiles and existing role identity colors, not the global app theme; existing image pixels are not recolored. The existing generation button now opens a Modal containing a localized default image brief from the role identity, responsibilities and selected color. Reset and Cancel perform no generation. Only Generate sends the edited brief and role context to Codex. The brief is separate from the role system prompt and remains in the current form only. Unedited defaults refresh with current metadata; a confirmed custom brief is retained for regeneration. Late-result matching includes both brief and color.

Selected tests cover preset/custom/invalid/automatic color, save/reopen/cancel, both style color fallbacks, alias/current-role propagation, modal open/edit/reset/cancel/empty/Enter/Escape, exact edited brief and color captured by a fake transport, and unchanged persisted role system prompt. Backend checks cover canonical hex storage, omitted/null/value updates, invalid color and brief rejection before process launch, creative-brief JSON, and migration 0030-to-0031 without changing prior images or styles. Existing fake-Codex process checks remain in scope. The old generation-button behavior was observed failing the new modal-presence test before the UI change; no real image request was issued.

### Automated results

All commands below exited **0**:

| Command / check | Result |
| --- | --- |
| `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | **823 passed, 0 failed, 3 existing ignored** |
| `cargo test --locked -p runner-core --profile ci` | **67 passed** |
| `cargo test --locked -p runner-daemon --profile ci role_avatar` | **19 passed** |
| `cargo test --locked -p runner-daemon --profile ci ops::role::tests` | **38 passed** |
| `cargo test --locked -p runner-daemon --profile ci repo::role::tests` | **6 passed** |
| `cargo test --locked -p runner-daemon --profile ci migration_0029_to_0031` | **1 passed** |
| `cargo test --locked -p runner-daemon --profile ci schema_snapshot_matches_migrations` | **1 passed**, snapshot at 0031 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | Passed after removing a redundant String-to-String conversion in a test selector |

- `cargo fmt --all -- --check`: exit **0** after formatting the newly added backend fixture fields; `git diff --check` also passed.
- `cargo build --locked -p runner-cli -p runner-app --profile ci --features gpui_platform/runtime_shaders`: exit **0**, producing the development candidate binaries in this worktree's `target/ci/`.

The dev app was not automatically restarted during this edit to avoid discarding the user's current form draft. A restart is required before native user acceptance of this refinement. No real image-generation request, global provider/login change, unrelated CLI golden sweep, native-window or Windows acceptance is claimed. The previously recorded native-image capability and Metal-toolchain limitations remain separate.

## Timeout report and Runner-owned wait-loop fix

### Investigation evidence

- The user-facing dev process was still the candidate started on **October 22, 2026 at 13:21 Asia/Shanghai**. Its installed daemon hash still matched that launch record, rather than the newly built palette/prompt candidate. It was not restarted during diagnosis, preserving the user's current editor.
- Existing dev logs contained no generation events. Consequently the precise upstream reason for the user's already-completed timeout cannot be reconstructed from those logs.
- Installed CLI: `codex-cli 0.163.0`; current auth was API-key/custom-provider. These are local observations, not a claim that all API-key sessions lack native image generation.
- No-model-turn app-server probe: initialization **0.36 s**, ephemeral thread initialization **0.18 s**, node_repl inventory **0.14 s**; this did not reproduce a bootstrap/MCP startup stall.
- A separate no-model-turn capability read took **0.18 s** and returned `imageGeneration: true`. A local HTTP mock (no real inference endpoint) captured the CLI request's tool inventory: it did not advertise `image_gen/imagegen`, and deferred discovery listed multi-agent and node_repl sources but no native image source. Therefore the advertised flag alone is not sufficient proof of a usable native image tool. The user's real provider's successful image-generation behavior remains unverified.
- `codex doctor --json` exceeded its bounded 25-second diagnostic window; that observation does not establish an inference or image timeout cause. No real image request or paid model turn was submitted, and no provider/login/global config was changed.

### Confirmed code defect and fix

The old subprocess wait path buffered CLI output until exit. The new regression emitted a terminal `turn.failed` authentication event and then kept the fake process alive; before the change Runner returned only a generic timeout. This RED result was observed.

The existing bounded stdout reader now observes bounded JSON progress lines. Terminal failures stop/reap the process promptly; nonterminal retry errors remain nonterminal. Deadline failures include a fixed last-stage and safe failure category (authentication/access, quota/rate limit, network/stream, missing native tool, or generic CLI failure). Raw error messages, tokens, prompts and provider responses are not included in those diagnostics. Failures are warn-logged for later inspection. The lifetime remains 180 seconds, and Runner does not add a retry or change providers.

Automated check results are recorded after the final run. The earlier 200-millisecond/four-second fake-startup assertion was too short under concurrent subprocess load, so the new diagnostic fixtures use the same 10-second startup budget as existing success fixtures while their child sleeps for 30 seconds. The terminal-failure assertion still proves exit before that budget rather than accepting a generic deadline error. Pure bounded-line/secret-handling tests avoid dependence on process scheduling.

Final timeout-fix gates:

- `cargo test --locked -p runner-daemon --profile ci role_avatar`: exit **0**, **23 passed**. Includes prompt/image/storage cases plus terminal-failure early return, retained retry diagnostics, bounded/secret-safe progress parsing, and lifecycle-stage tracking.
- `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders avatar`: exit **0**, **30 passed**, covering avatar UI state and fallbacks without a real CLI image request.
- All-target workspace Clippy with updater/runtime shaders: exit **0**.
- `cargo fmt --all -- --check` and `git diff --check`: exit **0**.
- Development app and CLI candidate build with runtime shaders: exit **0**.
- A read-only reviewer found no confirmed material issue in the progress parsing, privacy classification or terminal-failure cleanup.

The running acceptance window is deliberately not restarted. These results establish the Runner-owned wait/error-reporting fix, not successful live image generation or the exact upstream cause of the user's prior timeout. Save or deliberately discard the current role draft before restarting the dev app to load this candidate.

## Personal avatar / Appearance follow-up (2026-10-22)

Scope: add an Appearance-local My avatar subsection/tab, compatible personal settings, owned imported image files and human-author display. Theme controls and AI-role/event identity remain unchanged. No new image-provider API, synthesized stand-in assets, runtime configuration, daemon restart or production operation is in scope.

Selected automated checks and why:
- Personal settings default/roundtrip/normalization: existing settings must load unchanged and edited appearance must persist.
- Personal file import validation/ownership and shared avatar fallback: malformed files must fail without saving; imports must not depend on external files or role-preview lifetime; missing files retain selected local fallback.
- Appearance-local tab navigation and existing settings tests: the entry belongs inside Appearance and current theme settings remain reachable.
- Human versus role/system author tests: editing the operator's picture must not change event attribution or another role's appearance.
- Store shell snapshot/revision tests: other windows must redraw when the personal picture changes while unrelated settings retain their existing update behavior.
- Runner-app suite, all-target workspace Clippy and formatting: cover integration with the existing typed UI/settings surfaces without expanding into unrelated live runtimes.

Selected native acceptance (not executed by the coder): Appearance subset of `SET-PREF-01` plus feature-specific local-tab/preview/import/relaunch and cross-window Human-feed checks; author/avatar subset of `MIS-FEED-01`. Unchanged agent startup, IME, authentication, lifecycle and long-feed performance cases are outside this feature's live scope. Successful generation of the three preset images is still an unavailable-prerequisite gap, not a passed image test.

Results: runner-app **834 passed, 3 existing ignored** on the final implementation, including the Appearance subtab click/preview/style/color integration case and personal-settings/file/epoch/author regressions. All-target workspace Clippy with `updater,gpui_platform/runtime_shaders` passed; formatting, diff whitespace and catalog key parity passed. The runtime-shader feature remains the previously documented local build accommodation for the missing Metal compiler. Native/Windows acceptance is not claimed. The earlier 15:09 development window has not been restarted by this follow-up.

Review follow-up: selected cross-window tests also cover a delayed import in window A after a local-style choice in window B (including choosing the already-selected style), plus successful adoption and save-failure cleanup. An in-memory shared edit epoch invalidates stale import results; this is transient task state, not a second persistence channel. Saved replacements/clears reclaim only non-symlink `avatar-*.png` files whose canonical parent is the personal managed directory. Tests preserve original/role/symlink paths and retained images on save failure.

Image-generation clarification: this session's direct native `image_gen.imagegen` calls returned empty tool results without an attachment, `saved_path`, or located output file. Thus neither call proves image generation succeeded. Tool discovery's omission alone was insufficient evidence that the native entry point did not exist. This is distinct from the saved CLI probe, which completed with no PNG and no native image tool in its captured request. The three requested default images remain unfinished; no separately billed API or credentials/provider changes were used.

The final development app and CLI candidate built successfully; identity is recorded in `/tmp/runner-personal-avatar-candidate.json`. This build does not restart the already-running 15:09 app/daemon.

## Authorized development restart and publication follow-up

On 2026-10-22 at 16:41:55 Asia/Shanghai the user explicitly requested a restart. The existing development app/daemon were stopped through their normal development paths and the verified final candidate launched (app PID 21964, daemon PID 21974). Pre-restart SQLite and UI-settings backups were retained; read-only role/session/mission counts remained 5/2/1. Daemon hash matched the candidate sidecar. These are startup/data-preservation checks only, not a native visual acceptance pass. The installed application was not operated.

Before PR publication, fresh local checks cover the app/core suite, avatar daemon supervision, role persistence, migrations/schema, all-target workspace Clippy, formatting and guide/catalog consistency. The pre-PR backend review found the Linux descendant-identity implementation missing: process-group cleanup alone could miss detached children. Add a `/proc/<pid>/stat` start-time identity parser, verify command names with spaces/parentheses and PID-reuse differences, and enable the existing detached/reused-PID tests for Linux. The parser is tested on this macOS host; real Linux execution remains unclaimed.

The contributor requirement for a visual-change issue is recorded as #996 with a text layout sketch. The PR remains draft pending native macOS/Windows screenshots, live successful image generation, and GitHub CI results; no generated default-image assets are claimed.

### Fresh pre-publication automated evidence

- `cargo test --locked -p runner-app -p runner-core --profile ci --features gpui_platform/runtime_shaders`: exit 0; app **834 passed / 3 existing ignored**, core **67 passed**.
- `cargo test --locked -p runner-daemon --profile ci role_avatar`: exit 0; **24 passed**, including the Linux identity parser. An earlier concurrent run failed before the detached-child fixture wrote its PID handshake file. The focused reproduction passed, and the fixture now uses the existing success fixtures' 10-second startup budget and explicitly rejects timeout so missing handshake cannot masquerade as successful cleanup. The final full avatar rerun passed. This does not extend the production 180-second generation deadline.
- Daemon focused role operations **38 passed**, role repository **6 passed**, migration-to-0031 **1 passed**, and schema snapshot **1 passed**, each exit 0.
- Workspace all-target Clippy with updater/runtime-shader features, formatting and diff whitespace are checked again for the final pre-push snapshot.
- Previous unrelated CLI-golden/environment mismatch and lack of native screenshots/live generated assets remain explicit gaps; no full-workspace test or native acceptance pass is claimed.

A second read-only review caught that the existing Unix `ProcessTree::snapshot_descendants` implementation returned no children on Linux. The fix now extends that owning mechanism to walk every `/proc/<pid>/task/<tid>/children` recursively, deduplicating descendants and rejecting root cycles/unsafe PIDs. A synthetic proc-tree regression first failed for the missing implementation and then passed on macOS; the native macOS branch is unchanged. This connects discovery with the per-PID Linux start-time validation rather than leaving that parser unused. The fork PR #997 is draft; GitHub CI currently reports `action_required` and no checks, requiring upstream workflow approval.
