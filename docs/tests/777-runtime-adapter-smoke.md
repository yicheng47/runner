# 777 PR 2 — Live runtime smoke, 2026-10-02

Candidate: `86e66e8a7f47657e0a65d83fb216409a711ab3b2`, branch `refactor/777-runtime-adapter-spawn`, [PR #780](https://github.com/yicheng47/runner/pull/780), base `d8e5919e`. The implementation received `NO REMAINING MUST-FIX ISSUES` before the PR opened. [macOS and Windows CI](https://github.com/yicheng47/runner/actions/runs/36896877613) passed; the local workspace run passed 1,859 tests with 3 ignored. Those automated results are separate from the live matrix below.

**Overall: the live suite completed with findings, not a full pass.** All five accessible runtimes passed fresh turns, second-turn context recall and model/effort overrides. Codex, Antigravity, pi and Copilot passed original-conversation stop/resume; Claude passed in a separate canonical-cwd lifecycle retest. Claude, Antigravity and Copilot interruption status failed, pi classified cancellation as Response failed, Claude resume failed with the `/tmp` alias, Copilot failed to capture its post-`/clear` key, and the Antigravity sole-lead startup command ran in the hooks directory. Native Windows was unavailable; TRAE was skipped because Jason has no access.

## Authorization and environment

The mission brief originally prohibited launching the app or real agents. Jason explicitly overrode that restriction by answering **“Yes — authorize live smoke tests”** to launching real agents and testing Runner chats/missions with existing accounts and conversation-history writes. He then requested computer use, the reusable [procedure](full-smoke-test.md), and archiving all test chats/missions. No production chats or missions were modified and no Runner SQLite database was opened directly.

The test used macOS, Runner `0.12.6 (dev)`, this worktree's debug binary built through `make run`, and the absolute CLI under `~/Library/Application Support/com.wycstudios.runner-dev/bin/runner`. Its endpoint was in the same development namespace. Native computer use became available after the app restart; running inside a Runner mission did not prevent native control once that tool was enabled.

Computer use could not bind the bare debug executable, so the test wrapped a byte-identical copy in `/tmp/runner-pr780-ui-5iygtyo1/Runner PR780 Dev.app`, with distinct bundle ID `com.wycstudios.runner-pr780-dev` and the worktree's CLI sidecar. Byte comparison verified the executable matched `target/debug/Runner`. The original `make run` process was stopped before the wrapper launched. Debug app-data selection remained `com.wycstudios.runner-dev`. The installed `/Applications/Runner.app` was briefly inspected read-only during target identification; all test input went to the explicitly selected development wrapper.

Scratch root: `/private/tmp/runner-pr780-smoke-aswarqr9`. The initial direct chats used its `/tmp` spelling; subsequent Claude and mission checks used the canonical path. Evidence is local under `/tmp/runner-777-validation/`: per-boundary JSON status, mission start/feed/show records, stop/archive results and automated gate logs. UI replies were inspected through native computer-use screenshots. Raw screenshots with account information are not included here.

| Runtime | Installed version recorded | Requested model / effort |
| --- | --- | --- |
| Codex | Version not recorded in this run | `gpt-6.1-sol` / low |
| Claude Code | `2.1.287` | `sonnet` (UI: Sonnet 5.5) / low |
| Antigravity CLI | `1.2.14` | `gemini-3.8-flash-low` (UI: Gemini 3.8 Flash / low) |
| pi | `0.99.2` | `deepseek/deepseek-flash` / low |
| Copilot | `1.0.90` | `gpt-5-mini` / low |

Existing agent extensions/settings were left in place. pi loaded Runner's status extension alongside existing Orca/herdn extensions. Direct Claude chats used the existing auto mode; mission Claude showed bypass permissions. Direct Copilot showed Manual Approval, and each requested wait received one-time approval. Its mission showed Allow All under Runner's existing mission bypass setting. No permission setting was changed for these tests.

## Results

The initial direct chats for the original four runtimes received an `ORIGINAL` marker, recalled it in a second turn, ran a bounded `sleep 30`, received Escape during the tool, then answered `RECOVERED`. The supplemental canonical-cwd Claude chat covered lifecycle/rekey only. Stop/resume was checked by comparing captured keys and asking for the marker without putting it in the recall question. Codex/pi used `/new`; Claude/Antigravity used `/clear`; a different `NEW` marker was then recalled after another stop/resume. Copilot's later run used `/clear` and the bounded interruption retry described below; its new-marker recall failed. CLI JSON and terminal output were checked independently.

| Check | Codex | Claude Code | Antigravity | pi | Copilot |
| --- | --- | --- | --- | --- | --- |
| Fresh chat, second turn, correct marker | Passed | Passed | Passed | Passed | Passed |
| Working → Idle/Completed through hooks | Passed | Passed | Passed | Passed | Passed |
| Explicit model/effort reflected in UI | Passed | Passed | Passed | Passed | Passed |
| Working / tool activity | Passed | Passed | Passed | Passed | Passed |
| Escape cancellation and Runner Interrupted | Passed | Failed: remained Working | Failed: remained Working | Failed: Idle/Response failed | Failed: retained Approval needed/Working |
| Recovery reply and Idle/Completed | Passed | Passed | Passed | Passed | Passed |
| Stop/resume key and context | Passed | Passed on canonical cwd; failed on `/tmp` alias | Passed | Passed | Passed for original conversation |
| `/new` or `/clear` captures new key | Passed | Passed | Passed | Passed | Failed: old key retained |
| Resume new conversation and recall new marker | Passed | Passed on canonical cwd | Passed | Passed | Failed: old history and original marker |
| Fresh-folder mission executes marker once | Passed as sole lead | Passed as lead | Failed as sole lead: tool cwd was hooks directory; earlier worker passed | Passed as sole lead | Passed as sole lead |
| Native Windows interaction | Blocked: no Windows machine | Blocked | Blocked | Blocked | Blocked |

The initial four-slot startup mission used Claude as lead. Claude and Antigravity each appended exactly one `PR780-AUTOSTART` line and displayed the ACK without manual input. Codex finished its worker instructions without appending; pi's worker sat at an empty input. Separate single-lead Codex and pi missions both appended exactly one line and displayed the ACK. This is why the standard procedure uses a sole lead for per-runtime automatic execution. Antigravity also ran read-only Runner help/feed commands before the marker despite the bounded goal; its once-only marker and startup status passed, but strict one-tool instruction adherence did not.

## Findings left unfixed

1. **Claude interruption status:** Claude backgrounded `sleep 30` and waited on it. Escape cancelled the waiter and its terminal displayed “Interrupted,” while Runner remained Working with hook ownership. The background sleep finished and Claude later replied DONE. A recovery prompt completed normally. This proves a status discrepancy; it does not prove the background sleep was cancelled. Regression attribution remains unverified.
2. **Antigravity interruption status:** Escape cancelled the visible task and the terminal displayed interruption, while Runner's structured state remained Working. One Ctrl+C also left that status unchanged. A new recovery turn restored Idle/Completed. Regression attribution remains unverified.
3. **pi cancellation outcome:** Escape aborted the Bash wait after 14.3 seconds; the terminal showed “Command aborted” and “This operation was aborted.” Runner reported ready/hook/failed and displayed Response failed. Recovery succeeded. The adapter maps `stopReason: aborted` to Interrupted and `error` to Failed; the live raw stop reason was not retained, so the provider/extension cause remains unverified.
4. **Claude path-alias resume:** The initial row used `/tmp/.../claude-code`, while Claude stored its conversation beneath the encoded `/private/tmp/...` project path. The file probe looked under the literal row cwd, failed to find the old conversation and started fresh; recall returned UNKNOWN. File-existence checks confirmed the old JSONL existed only under the canonical project path. `runtimes/helpers.rs::conversation_file_exists_at` has the same literal-path lookup at base `d8e5919e`, so that mechanism predates PR 2. A new chat with canonical cwd preserved its original key/context and its post-`/clear` key/context.
5. **Copilot cancellation status:** The first `sleep 30` completed before interruption was confirmed, so that attempt was inconclusive. One bounded `sleep 60` retry was cancelled using Copilot's advertised two-Escape sequence. The terminal showed “Operation aborted by user” and “Operation cancelled by user,” but Runner retained Working/hook with the `copilot-2` approval interaction and no outcome, including a later settled capture. A recovery turn displayed RECOVERED and restored ready/hook/completed. Actual input bytes were not retained, so the interruption mechanism remains unverified.
6. **Copilot new-conversation resume:** `/clear` displayed fresh history and preserved Manual Approval. After a visibly completed `ACK PR780-COPILOT-NEW` turn, Runner still held the original key `2de68daf-ebb0-4fe1-b624-a85ece36c176`. Stop/resume reopened the old history; a blind recall question returned `PR780-COPILOT-ORIGINAL`. An earlier attempt was stopped before its new-marker ACK had been verified and is excluded from this finding; the bounded retry verified completion before stopping. Live baseline attribution remains unverified.
7. **Antigravity tool working directory:** A fresh sole-lead mission displayed the automatic ACK and ready/hook/completed, but its expected marker was absent from the requested mission directory. One read-only diagnostic printed the tool cwd as Runner's development `antigravity-hooks` directory and the expected test handle. The marker existed there with exactly one line. `lsof` independently showed the agent process cwd was the requested mission directory, matching the session row. The adapter injects the hooks directory with `--add-dir`; whether that option causes the CLI to select it as the tool cwd still needs a controlled reproduction. The earlier worker run's marker remains in its requested directory. This sole-lead startup check fails despite the ACK.

No runtime code, fixtures or frozen expectations changed in response to these findings. For the original four findings, reviewer comparison against `d8e5919e` found the input path, inherent watchers, pi outcome mapping and literal-cwd lookup unchanged, with the new trait wrappers preserving the interrupt Arc and drain calls. This supports separate follow-up investigation; live baseline reproduction, the raw pi stop reason and attribution of the supplemental findings remain unverified. App-wide release flows and native Windows remain outside this run's evidence. TRAE is Skipped because Jason has no access.

### Supplemental coverage

Jason identified the omitted Copilot coverage and requested testing every accessible runtime and fixing discovered issues. The development chat `01M3X8H89GF3250AYW92YSSDAS` used canonical cwd `/private/tmp/runner-pr780-smoke-aswarqr9/copilot`. Native control initially lost the closed window (`cgWindowNotFound`); opening the same wrapper through Finder restored control. Original-history resume, second-turn recall, recovery, the interruption retry and the completed new-conversation lifecycle check were then inspected through the UI and CLI. Its sole-lead mission automatically appended exactly one marker in the requested fresh directory, displayed the ACK and reached ready/hook/completed. Evidence: `copilot-first.json`, `copilot-resumed.json`, `copilot-retry-interrupted-settled.json`, `copilot-recovered.json`, `copilot-new-verified*.json`, `copilot-new-resumed-verified.json` and `copilot-mission-*.json` under the validation directory.

Antigravity then ran once as a sole lead to complete per-runtime startup coverage. Window clicks briefly failed with `noWindowsAvailable`; reopening the same wrapper through Finder restored control. The missing marker triggered the single read-only diagnostic described above. Evidence: `agy-lead-mission-*.json`, `agy-lead-actual-marker.json`, `agy-lead-directory.json` and `agy-lead-diagnostic-state.json`. Neither supplemental run erases the earlier findings or establishes a full pass.

## Cleanup and traceability

All six test direct chats and all eight mission-slot processes were stopped. The original five chats and three missions were archived at approximately `02:41:43 UTC` on 2026-10-02. The supplemental Copilot chat/mission were archived at approximately `05:17:27 UTC`, and the Antigravity sole-lead mission at `05:25:18 UTC`. CLI show verified archive timestamps and zero live mission sessions; native UI cleanup was checked separately. Existing development chats and missions were left alone. No feed followers remained active. Five test roles, five test crews, the temporary app bundle, scratch evidence and the Antigravity test marker in the development hooks directory were retained; no branch, worktree or conversation files were deleted.

| Object | ID |
| --- | --- |
| Codex direct chat | `01M3X5GTKJEMV9SY9KBZRFRW7D` |
| Claude initial direct chat | `01M3X5P04Q40QEF6BPY2G1NEGM` |
| pi direct chat | `01M3X5X1ZAQWKPV1VKKKKHV656` |
| Antigravity direct chat | `01M3X5YCSNCJG0JMSKD53HTYTD` |
| Claude canonical direct chat | `01M3X7H0S69BDJN1WVTSZKXVYA` |
| Four-slot startup mission | `01M3X6QKCY4G8ZMX3WP1FHBGM9` |
| Codex sole-lead startup mission | `01M3X6ZKFNTMY81BS7Y6YX3D4R` |
| pi sole-lead startup mission | `01M3X7EYPTF1D1Y5Z560QKQCPG` |
| Copilot direct chat | `01M3X8H89GF3250AYW92YSSDAS` |
| Copilot sole-lead startup mission | `01M3XGK2ZC0JP54097GG2RVP40` |
| Antigravity sole-lead startup mission | `01M3XGRYTKYMB8K78JHJDYX6DW` |

The development CLI supplied state/lifecycle operations; the `runner-dev` skill supplied the version-matched command and watch rules. This host had no qualifying background notification facility, so each mission start reported automatic watching unavailable and supplied its foreground `mission feed <id> --follow --json` command. One-shot feed/show captures were used during the active test.
