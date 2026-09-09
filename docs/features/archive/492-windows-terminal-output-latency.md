# Ship the Windows Terminal ConPTY and forward Windows output immediately

Tracking issue: [#492](https://github.com/yicheng47/runner/issues/492). Status: **shipped 2026-09-08 in [#522](https://github.com/yicheng47/runner/pull/522)**, released in 0.8.4. The first revision's coalescer rewrite is superseded and removed by this one. Priority P1. Windows only; the macOS forwarder is not touched.

## Motivation

On Windows, typing into a Codex chat and watching it redraw feels batched, while the same build on macOS feels continuous. The Windows forwarder (`coalesce_windows_output`, `crates/runner-backend/src/session/manager/output.rs`) was added on 2026-09-06 to hide a cursor jump: the inbox ConPTY splits every Codex frame at its cursor-style escape and re-paints the parked cursor about 15 ms later, so a forwarder that delivers chunks as they arrive shows the cursor on the wrong row between the two paints ([impl log](../impls/archive/windows-nightly/impl_log.md#2026-09-06--captured-cursor-redraw-and-sidebar-follow-up)). It hid the jump by holding output until 25 ms of quiet or a 100 ms cap, which delayed every byte and collapsed redraws into batches. That is the regression in #492.

The first revision of this spec measured the split with a raw ConPTY probe and replaced the hold with one that waited only for the restore paint. A Peer coding crew implemented it; it matched the reference simulation on the captures and passed every check, and in the dev app Jason judged it a little better but still not macOS. That residual sent the investigation to Zed, which runs the same GPUI plus alacritty_terminal stack on Windows.

## Findings

### The split belongs to the inbox conhost, and Zed does not use it

Zed does not coalesce terminal output. It ships Microsoft's ConPTY from the Windows Terminal project, `conpty.dll` and `OpenConsole.exe` from the `Microsoft.Windows.Console.ConPTY` package, next to `zed.exe` ([crates/zed/build.rs](https://github.com/zed-industries/zed/blob/main/crates/zed/build.rs)), and alacritty_terminal loads that in place of the inbox `conhost.exe`. Runner's PTY layer, `portable-pty` 0.9, has the same preference built in: `load_conpty` in `src/win/psuedocon.rs` opens `conpty.dll` through the normal DLL search, which starts in the executable's directory, and falls back to `kernel32.dll` only when it is absent. Runner ships nothing beside `Runner.exe`, so every session runs on the inbox conhost, `10.0.26100.8875` on JASONPC.

The standalone probe under the ignored `target/492-conpty-probe/` (`cargo run -- record|analyze|simulate`; 149 × 49 grid, the same terminal-query answers the runtime gives, `asdf` typed at 2.5 s and ` qwer` at 5 s, eight seconds per capture) was re-run with the two x64 files from `Microsoft.Windows.Console.ConPTY.1.24.260303001.nupkg` (Windows Terminal release `v1.24.10621.0`, MIT, both files Authenticode-signed by Microsoft Corporation) placed beside the probe binary. A repeat run, `codex4-wt.ndjson`, confirmed by executable path that the probe's console host was its own `target\debug\OpenConsole.exe` (654 chunks, 145 frames, 0 split, key echo 30 to 32 ms, the same picture as `codex3-wt`). The new captures are `codex3-wt.ndjson`, `codex4-wt.ndjson`, and `claude2-wt.ndjson`; the inbox captures from the first revision are `codex1`, `codex2`, and `claude1`.

| Codex, 8 s from spawn | inbox conhost, run 1 / run 2 | Windows Terminal ConPTY 1.24 |
| --- | --- | --- |
| raw chunks | 418 / 573 | 450 |
| frames ending at `ESC[?2026l` | 90 / 123 | 98 |
| frames whose cursor was parked, then restored by a later paint | 47 / 59 | 0 |

On the newer ConPTY every Codex frame arrives whole. The `DECSCUSR` no longer splits it and there is no second paint, so the parked cursor the forwarder exists to hide is never on the wire.

### With whole frames, any hold is pure delay

Replaying `codex3-wt.ndjson` through the probe's policies:

| Codex on the Windows Terminal ConPTY | immediate (the macOS forwarder) | first-revision coalescer (hold ≤ 25 ms after a sync end) | Windows before #492 |
| --- | --- | --- | --- |
| added delay per delivery, p50 / p90 / max | 0 / 0.1 / 0.4 ms | 0.4 / 25.1 / 52.2 ms | 25.2 / 28.1 / 58.4 ms |
| deliveries in the busiest second (200 raw chunks) | 83 | 46 | 21 |
| key echo, input → composer cursor advanced, p50 / max | 31 / 32 ms | 56 / 57 ms | 56 / 57 ms |
| parked cursor shown after the TUI has painted | never; one residual delivery at spawn on a blank screen, 0 ms | once, 47 ms | never |

The first-revision coalescer waits its full 25 ms after every frame because the restore paint it is waiting for never comes; on this ConPTY it is as slow as the forwarder it replaced. Immediate delivery gives a 31 ms key echo against 47 ms on the inbox conhost under the same policy: the 15 ms that was ConPTY's second paint is gone, and what remains is Codex's own redraw, which is the same on macOS.

Claude Code is unaffected either way: 22 deliveries, zero added delay, and a key echo of 3 to 7 ms once its TUI is up (the first four keys in `claude2-wt` landed 400 to 660 ms before the TUI finished starting and are not echo measurements). Codex will never feel like Claude Code on any platform: Claude Code echoes a key as one byte within about 3 ms and never uses synchronized updates, while Codex repaints its frame 30 to 45 ms after a key. 31 ms on Windows is Codex's macOS floor.

## Behavior

- **Runner ships the Windows Terminal ConPTY.** `conpty.dll` and `OpenConsole.exe` (x64) sit beside `Runner.exe` in the installer, in nightlies, and in local builds under `target\debug` and `target\release`. `portable-pty` picks them up with no Rust change. The package is pinned by version and SHA-256 the way the Inno Setup compiler and the SimplySign MSI are, and Runner's license notices gain the package's MIT license.
- **The Windows forwarder delivers output immediately.** `coalesce_windows_output` and its tests are deleted, the `cfg(windows)` branch of the forwarder loop goes away, and both platforms run the same `try_recv` drain up to the 1 MiB cap. No hold, no quiet period, no sync-end detection.
- **The installer treats the two files like the binaries.** `ignoreversion`, listed with the binaries the installer renames aside when they are in use, and expected by `test-installer.ps1` in payload mode. The Authenticode assertions stay on Runner's own five files; Microsoft's signatures are checked once, when the package is fetched.
- **A missing DLL is a broken install, not a mode.** `portable-pty` silently falls back to the inbox conhost, where the cursor jump would return. On the first PTY spawn Runner logs whether `conpty.dll` is present beside the executable so a report can be diagnosed from the log. Nothing else adapts.
- **macOS is unchanged.** The shared drain loop is the existing `cfg(not(windows))` loop verbatim.

Targets: the immediate column above on `codex3-wt.ndjson`, and Claude Code unchanged on `claude2-wt.ndjson`.

## Non-goals

- Keeping a coalescer as a fallback for the inbox conhost. Zed ships without one, and a fallback would keep a second timing model alive for a configuration Runner does not ship.
- Switching PTY crates. `portable-pty` already loads the sideloaded ConPTY; alacritty_terminal's `tty` is not needed for this.
- ARM64. The package carries an ARM64 build; Runner ships x64 only.
- Fixing the cursor in the renderer, working around Codex upstream, changing alacritty's sync-update timeout, the 4 ms UI wake batch, the read buffer size, or the first-turn readiness wait (`deliver_windows_batch_first_turn`), unchanged from the first revision.

## Implementation Phases

1. **Ship the ConPTY.** Add `script/windows/conpty.ps1`: download `https://github.com/microsoft/terminal/releases/download/v1.24.10621.0/Microsoft.Windows.Console.ConPTY.1.24.260303001.nupkg` (SHA-256 `2c57cb7da7e19fa06c86487c8d9b5c307d65695429fa15a854bf5f3cddca9e1d`) into `target/tools` unless already cached, verify the hash, extract `runtimes/win-x64/native/conpty.dll` and `build/native/runtimes/x64/OpenConsole.exe` (a `.nupkg` is a zip), and copy both into the output directory given on the command line. Call it from `make.cmd` after `cargo build` for the profile's output directory, and from `bundle-windows.ps1` for the release directory before the `dumpbin` checks. Add both files to `[Files]` in `runner.iss` and to the rename-aside names; add a `LICENSE.conpty` notice (the MIT text with Microsoft's copyright line) to the repository and the installer. `test-installer.ps1` payload mode expects both files to be installed, upgraded, and removed. Document in `docs/arch/windows.md`: development outputs, installer contents, and a short paragraph on why the ConPTY is shipped, linking this spec.
2. **Remove the coalescer.** Delete `coalesce_windows_output`, its constants, and the ten `windows_coalescer_*` tests plus their simulation helper; make the drain loop platform-independent; keep `forwarder_delivers_a_cursor_burst_without_waiting_for_eof` and the first-turn tests. Add the one log line on first PTY spawn.
3. **Re-measure and verify live.** Confirm in the dev app that `OpenConsole.exe` appears as a child process when a chat starts and that the log line reports the sideloaded ConPTY; then Codex startup typing, typing while streaming, a resize storm, and Claude Code typing. Record the result in the impl log.
4. **macOS check.** Build and run `cargo test -p runner-backend` on a Mac; the diff touches no macOS code path except the now-shared drain loop.

## Verification

- [x] `test-installer.ps1` passes in fixture and payload mode; a fresh install and an upgrade place `conpty.dll` and `OpenConsole.exe` beside `Runner.exe`, and uninstall removes them. 2026-09-08, payload mode against `target\debug`.
- [x] Starting a chat in the dev app spawns `OpenConsole.exe`; the log names the sideloaded ConPTY. 2026-09-08: the Codex chat's host was `target\debug\OpenConsole.exe`, while the installed app without the DLL showed `conhost.exe` hosts.
- [x] Codex on Windows: typing `asdf` during startup echoes each character with the cursor on the composer; no cursor visit to the first owned row. Jason, 2026-09-08 dev build: "much better now".
- [x] Codex on Windows during a streaming response: the spinner and text update at ConPTY's cadence, no 100 ms stepping. Same session.
- [x] Claude Code on Windows: keystroke echo is indistinguishable from macOS. Same session.
- [x] Resize storm on Windows: no regression against the existing resize-grace behavior. Same session.
- [ ] `cargo test -p runner-backend` green on Windows and macOS; workspace Clippy in the `ci` profile and `cargo fmt --all --check` clean; `bundle-windows.ps1` produces a nightly that installs on JASONPC. Windows side green on 2026-09-08 (590 tests); macOS and the nightly install remain.

## History

- 2026-09-06: the quiet-period coalescer was added to hide the cursor jump.
- 2026-09-08, first revision: the probe measured the ConPTY split (restore paint 0.5 to 23 ms after the frame, median 15 ms) and the coalescer's cost (key echo 72 ms against 47 ms immediate; 14 to 17 deliveries in the busiest second against 91 to 116). A Peer coding crew replaced the coalescer with a restore-paint hold, replayed it against the captures (198 / 298 / 22 deliveries against the reference 197 / 298 / 22), and passed tests, Clippy, and fmt. Jason's feel test: a little better, still not macOS. Superseded the same day by this revision after the Windows Terminal ConPTY measurement.
- 2026-09-08, this revision: a Peer coding crew shipped phases 1 and 2 in one review round (mission `01M20GAV08GRRMCH8B056THA44`). Verified on JASONPC: backend tests, CI-profile Clippy, fmt, fixture and payload installer tests, the sideloaded host in the dev app, and a fourth probe capture with the host confirmed by path. Jason's feel test on the dev build: much better.
