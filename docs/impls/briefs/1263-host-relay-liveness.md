# Mission Brief: Harden host `/relay/host` socket liveness (#1263 item 21)

**Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1263-host-relay-liveness`
**Branch:** `fix/1263-host-relay-liveness` (from `origin/main`)
**Tracking issue:** [#1263](https://github.com/yicheng47/runner/issues/1263) Addendum item 21

## Authorization & Workflow

- Approved crew: `carbon trio` (`coder`, `reviewer`, `qa`).
- You are authorized to commit, push `fix/1263-host-relay-liveness`, open a **non-draft** PR targeting `main`, and drive CI green on macOS, Windows, and Linux. Do **not** merge the PR.
- **CRITICAL issue-link rule:** Use `Refs #1263` / `Part of #1263` in the PR title/body — **NEVER** write `Closes #1263` or `Fixes #1263`, because `#1263` still tracks wire-changing Phase 3 (`earlyHello`/`resume`) and Phase 4.
- **Commit hygiene:** Before opening the PR (and after any review/QA fixes), fold this brief commit into clean feature/fix commit(s) so there is no standalone brief commit (`git rebase -i` / `--amend` + `--force-with-lease`).
- **PR labels:** Ensure `core-review` (required by `docs/features/1263-remote-route-hardening.md`; add manually if GitHub Actions doesn't auto-apply it), `risk-medium`, `reviewed` (added by `@reviewer` on the exact final head SHA), and `needs-live-test` (kept because Windows/Linux native wake & network-change live checks cannot run on macOS, or if any macOS relay check is blocked).

## Problem Statement

Today in `crates/runner-daemon/src/remote/machine.rs` (`Input::Welcome`, `Input::Frame`, `Input::Deadline`) and `crates/runner-daemon/src/remote/mod.rs`, the host's `/relay/host` WebSocket sets a passive 75-second read deadline and waits for inbound frames or relay pings without sending outbound WebSocket pings when idle or probing on system wake / network change. If a NAT mapping silently drops or the host changes networks, the host stays `Connected` for up to 75 seconds while incoming client handshakes stall on the dead socket.

This optimization changes **no wire/companion protocol format** and intentionally lands ahead of Phase 3's wire changes (`earlyHello`, `resume`).

## Requirements

1. **Outbound idle WebSocket pings & tighter pong deadline (no wire change):**
   - Treat all valid inbound WebSocket frames (`Text`, `Binary`, `Ping`, `Pong`) as inbound liveness activity.
   - When `Connected`, if no inbound frame has arrived for an idle window (e.g. `10s`), send an outbound RFC 6455 `Message::Ping` through `Relay`'s writer (`tx.try_send(Write::Frame(Message::Ping(...)))`, preserving `CONTROL_HEADROOM` for channel traffic and handling write failure as network failure) and enforce a tight pong/silence deadline (`5s` pong timeout / `15s` max silent bound instead of `75s`, aligned with `remote/client/route.rs`'s `ROUTE_TIMEOUT = 15s` and Relay's 25s ping / 60s idle drop).
   - Confirm `tokio-tungstenite` surfaces `Message::Pong` on `stream.next()` (and test this with `TestSocket`!). Avoid spurious `Ping` spam while traffic is actively arriving.

2. **Immediate probe & fast recovery on system wake or network change:**
   - Detect sleep/wake and network-interface/address/route changes via:
     - **Cross-platform clock-jump / timer-overshoot detection** in `remote::run` (detecting suspend gaps across wall clock vs monotonic `Instant` without OS dependencies).
     - **Native wake / network-change listeners** cleanly isolated per target OS (must compile cleanly on macOS, Windows, and Linux!):
       - macOS: lightweight `SystemConfiguration` (`SCDynamicStore` interface/IP change) + power wake (`IORegisterForSystemPower` or equivalent CoreFoundation wake notification) cleanly torn down on daemon shutdown / drop.
       - Windows: lightweight `NotifyIpInterfaceChange` / `NotifyUnicastIpAddressChange` (`windows-sys`, checking Cargo features!) + wake notification or sleep-gap fallback.
       - Linux: lightweight nonblocking `AF_NETLINK` (`NETLINK_ROUTE` link/addr groups) + sleep-gap fallback.
     - Listener failures in sandboxes/containers/tests must log and degrade gracefully to ping + timer detection — never panic or abort `runnerd`.
     - Debounce rapid bursts (e.g. `250ms`–`500ms`) so interface transitions do not double-reconnect. Also ensure threads/FDs/watchers terminate promptly when `RemoteHandle::shutdown()` / `cancel` fires (no leaked threads in tests!).
   - Action on signal:
     - If `Connected`: immediately probe by sending a `Message::Ping` and tightening the pong deadline (e.g. `2s`–`3s`). If a pong arrives in time, stay `Connected` and restore normal idle schedule; if it times out or write fails, immediately drop via `Input::Failure(Failure::network())` (`Effect::Close`) and reconnect.
     - If `Reconnecting`: wake up immediately (`retry` / immediate connect instead of sleeping through stale backoff).

3. **Docs updates:**
   - Update `docs/tech/remote-control.md` (§11 "Noticing a dead route" and constants table) for the new host idle ping, tight deadline, and wake/network probe behavior.
   - Update `docs/features/1263-remote-route-hardening.md` to note item 21 landing independently ahead of Phase 3's wire changes (`earlyHello`/`resume`).

## Key Files

- `crates/runner-daemon/src/remote/machine.rs` (pure state transitions, `Command`/`Input`/`Effect`, `deadline`, `last_frame`, tests)
- `crates/runner-daemon/src/remote/mod.rs` (`run()`, `Relay`, `Write`, `Clock`, signal wiring, shutdown lifecycle)
- `crates/runner-daemon/src/remote/tests.rs` (`TestSocket` mock relay tests)
- `crates/runner-daemon/Cargo.toml` (check existing workspace deps/`windows-sys` features before touching)
- `docs/tech/remote-control.md`, `docs/features/1263-remote-route-hardening.md`

## Validation & Crew Handoff

- **@coder:** Keep changes small, well-factored, and free of cross-platform compile breakages. Add unit tests in `machine.rs` and async mock-socket tests in `remote/tests.rs` covering idle ping emission, pong refresh, missed-pong fast disconnect (`<= 15s`), wake/network probe success and timeout-fail, `Reconnecting` immediate wakeup, debounce, and clean watcher shutdown. Verify locally with `cargo test -p runner-daemon remote`, `cargo fmt --check`, and `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`.
- **@reviewer:** Review state transitions, writer backpressure (`CONTROL_HEADROOM`), thread/FD shutdown lifecycle, cross-platform `#[cfg]` code (Windows `windows-sys` features, Linux netlink, macOS CoreFoundation safety), and `docs/` updates. Name exact live checks (or blockers) for `@qa`. After all fixes, verify the exact final head SHA and add the `reviewed` label with a PR comment naming that commit.
- **@qa:**
  - **Lock rule:** Wait for and acquire `script/qa-lock acquire --timeout 540` BEFORE building wrappers or launching the dev app; release with `script/qa-lock release` once finished.
  - Use the Dev Runner + same-Mac two-host relay fixture (if signed-in Chrome/relay credentials exist; if sign-in/relay is unavailable in this environment, do NOT bypass or probe interactively — record `Blocked` honestly with exact blocker details).
  - Verify real `/relay/host` behavior: steady `Connected` health, fast host disconnect/reconnect when host interface/network drops or wakes (vs old 75s stall), and recovered remote access, logging results in `docs/tests/runs/1263-host-relay-liveness-mac.md`.

**STOP condition:** Report status to operator when PR is open, `reviewed` is stamped on final head SHA, QA record is committed (folded cleanly), required labels (`core-review`, `risk-medium`, `reviewed`, `needs-live-test`) are applied, and all CI checks pass.
