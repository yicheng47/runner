# Issue 753: Windows inbox delivery

Validated on 2026-09-29 with Windows 10.0.26200, Codex CLI 0.158.0, Antigravity CLI 1.2.12, and Runner 0.12.3. The development patch is based on `24f68c99`; the original reproduction used installed Runner `6f6d6d2`.

## Findings

| Delivery | Result |
| --- | --- |
| Original raw text, Enter after 80 ms, default Codex paste detection | Six writer notices and one reconciliation notice accumulated in the editable composer; no writer message was read. |
| Original Runner, disposable Codex role with `-c tui.disable_paste_burst=true` | Nine of nine notices submitted and their message ids were acknowledged. |
| Negotiated bracketed paste, Enter after 80 ms, default paste detection | Six of six writer messages remained unread. A separate traced idle probe also remained in the composer. |
| Negotiated bracketed paste, Enter after 300 ms on Windows, default paste detection | Six of six notices became submitted Codex user messages, and all six event ids were read and acknowledged. |
| Original raw-text delivery, Enter after 300 ms on Windows, default paste detection | Six of six notices submitted, and all six event ids were read and acknowledged. This is the final implementation. |

Both successful tests sent three messages during four 20-second shell work windows and three after the receiver wrote its idle marker. Inbox reads were allowed only after an actual notice arrived as a user message. Success was checked against the Codex transcript, Runner `inbox_read` events, and the receiver's acknowledgement file, rather than the synthetic busy indicator. The first three submitted messages appear together after a tool window completed. The later notices also submitted, and all six event ids were acknowledged in each run. The test sessions were stopped after collecting the evidence. The final raw-text run confirms that the framing changes were unnecessary for this reproduction.

The traced 80 ms failure recorded bracketed-paste mode enabled, a 66-byte notice body, and a separate one-byte Enter 80.7 ms later. Replaying the captured terminal output showed the notice still in the composer. This disproves the assumption that negotiated bracketed paste alone bypasses Codex's Windows paste detector.

Codex's Windows TUI selects console input-record mode. Its pinned Crossterm Windows event source reads individual console key records and does not produce a paste event. The final patch changes only the Windows submit delay to 300 ms. It keeps the existing raw-text delivery, separate Enter, and delivery reservation. Codex's paste-detection configuration stays unchanged. The proposed bracketed-paste implementation was removed because it added session state and changed shared input handling without demonstrating a benefit for this bug.

## Evidence

- Original mission: `01M3NYQRN87XXV2PDJF5YHD1AB`.
- Paste-detection-disabled control: `01M3NZ9FS524X5W7BE5EB1JGXV`.
- Bracketed paste at 80 ms: `01M3P0PWTVW2PYDX3G3F7613XZ`.
- Traced idle failure: `01M3P10B7T58NF63XFK683XRYY`.
- Successful bracketed-paste/300 ms mission: `01M3P1CGEF6HXQV7K8NKRDM5WB`; Codex conversation: `01a0ec16-4d74-75d0-8449-905430468b9e`.
- Successful final raw-text/300 ms mission: `01M3P3H6CB2B61FFWY9P3QNCPG`; Codex conversation: `01a0ec38-a61d-7670-b929-c7ae0497ab64`.

Local evidence is under `%TEMP%\runner-issue753-repro-20260929`: `findings.md`, `delay300-*` and `raw300-*` event/submission/acknowledgement files, and terminal recordings. These machine-local artifacts are not checked in.

## Automated coverage and limits

Existing backend tests cover queued human keystrokes, reservation cancellation, coalescing, ordering, reconciliation, and draft protection. Router tests that wait for Enter completion now include the platform's submit delay and scheduling margin in their deadlines. The discarded bracketed-paste implementation and its new tests are absent from the final patch.

Validation commands:

- `cargo build --locked --workspace --profile ci -j 12`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets --profile ci -j 12 -- -D warnings`: passed.
- `cargo test --locked -p runner-backend --profile ci --no-fail-fast -j 12`: all 997 tests passed on the final patch, with Git's Unix tools on the test process PATH. A reconciliation test initially exceeded its 600 ms deadline under load; waits for completion now allow the submit delay plus one second of scheduling margin.

Earlier testing exposed a separate environment issue: `session::claude_status::tests::git_sh_runs_the_injected_commands_with_forward_slash_feeds` failed with Windows error 109 (`BrokenPipe`), also reproduced in the baseline binary. The test finds Git's outer `sh.exe` by absolute path, but its generated hook invokes bare `sh` and `cat`; neither was available on the agent tool shell's PATH. Adding `C:\Program Files\Git\usr\bin` to PATH for the test process resolved it without a source change.

The live validation establishes this Windows workload; it does not establish a maximum input-processing delay under arbitrary load or a semantic submission acknowledgement. macOS and other live agent runtimes were not exercised. The earlier disabled-detector control had a different workload schedule, so it is supporting evidence for the mechanism rather than a timing comparison.

## Sources

- [Runner issue 753](https://github.com/yicheng47/runner/issues/753).
- [Codex 0.158.0 paste-burst handling](https://github.com/openai/codex/blob/rust-v0.158.0/codex-rs/tui/src/bottom_pane/paste_burst.rs).
- [Codex 0.158.0 Windows input mode](https://github.com/openai/codex/blob/rust-v0.158.0/codex-rs/tui/src/tui/windows_console.rs).
- [Codex's pinned Crossterm Windows event source](https://github.com/openai-oss-forks/crossterm/blob/efa177859fd9623d57b9fe7ae9bf491ae1ac6ec4/src/event/source/windows.rs).
