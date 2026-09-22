# #688: runtime-aware title hints in the status fallback

## Automated evidence

`crates/runner-terminal/fixtures/codex-title-working.ndjson` is the retained sanitized recording. Its recognized Codex activity title becomes Busy, the matching leading-spinner removal at 7.428 s becomes baseline Idle, and 46 later output events through 10.893 s do not wake it. The same replay runs with no accepted hook and after a simulated bridge failure. `claude-session.ndjson` remains on the unchanged byte fallback because this change accepts no Claude title format.

The accepted Codex grammar comes from upstream commit `7f01a84effccef40d4726c3ca12e6c839ec98d7a`: the [default title starts with the activity item and defines ten exact frames](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/status_surfaces.rs#L26-L36), [activity animates only during active progress](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/status_surfaces.rs#L1009-L1068), [thread-title progress adds its own spinner](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/thread_title_status.rs#L32-L49), and [titles are written as OSC 0 terminated by BEL](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/terminal_title.rs#L46-L90). The recorded fixture, rather than those source locations, is the evidence for the literal `renaming... ` progress prefix.

## macOS smoke

1. In a development build Jason chooses to launch, open a fresh Codex direct chat and a Codex mission with one slot left unopened in any pane. Leave both prompts untouched and confirm #687's startup behavior still reaches Idle.
2. Submit one real prompt in the direct chat and one through the mission router. Confirm each immediately reads Working, remains Working through active title animation, and reaches Idle when the leading activity spinner disappears even if the composer keeps repainting.
3. Create a disposable Codex runner with the explicit argument `--disable hooks`, start a new direct chat from it, repeat step 2, and confirm the labels remain estimated and no session-status feed is created. Start a separate chat from the ordinary Codex runner, begin another turn, and confirm accepted hook observations own Working/Idle regardless of title changes or quiet output.
4. With that hook-owned disposable turn running, delete its `<session-id>.ndjson` feed under `~/Library/Application Support/com.wycstudios.runner-dev/session-status/` (`com.wycstudios.runner` for an installed build). The bridge check runs at least once per second; confirm Runner reports the explicit failure, first returns to the ordinary baseline, and then lets fresh Codex spinner/rest titles refine Busy/Idle. Deleting the matching `.sh` reporter on macOS or `.ps1` reporter on Windows is equivalent. A merely quiet, empty, or unreadable already-open feed does not trigger failure.
5. After an Idle title, submit by normal Enter, paste, and one routed message. Confirm each changes to Working immediately and a stale resting title does not restore Idle; fresh title or hook evidence may do so. Resume the session and confirm it does not inherit title authority from the prior PTY.
6. Exercise a Codex configuration with terminal titles disabled or customized, plus a shell whose title contains `Working`, `Ready`, or a braille glyph in a path/topic. Confirm these use ordinary byte activity and never acquire title-derived Idle or a completed outcome.

## Windows smoke

Repeat the macOS steps with the npm `codex.cmd` launcher on Windows 11 and Windows 10 system conhost. Include an unopened mission slot, automatic first-turn paste, local paste, routed delivery, disabled/custom titles, and an explicit status-bridge failure. Confirm ConPTY split OSC sequences behave the same, hook-owned turns ignore titles, and no title transition changes approval/question holds, drafts, or delivery.

Native Windows execution is not available to this crew; these legs remain for Jason.
