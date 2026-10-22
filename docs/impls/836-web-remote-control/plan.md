# 836 — plan

The phases of [feature 836](../../features/836-web-remote-control.md) in detail: how each lands, what it touches, what reading the code changed, and how it is checked. The program's state and its missions are in [README.md](README.md), and the dated log is [impl_log.md](impl_log.md).

## Phase 1 — connect a host (mission 1, merged)

Device-code enrollment from Settings → Remote access or `runner remote login`, the secrets in the OS secret store, the outbound connection to the service and its states, specified in the spec's Phase 1 and its Implementation section. Briefed as [`836-connect-host.md`](../briefs/836-connect-host.md) for a codex duo; merged into `nightly-remote` on 2026-10-18 as [#945](https://github.com/yicheng47/runner/pull/945), and squashed there into one commit on 2026-10-19. Its record is [`836-connect-host.md`](../../tests/836-connect-host.md), with the live checks still pending with Jason.

## Phase 2 — client trust and the companion protocol

Two missions, each one pull request into `nightly-remote`:

| Mission | What | Waits on |
| --- | --- | --- |
| m2 — host trust | The stable identity key, the PIN and its lock, the device list and Reset identity, with their client requests, CLI and Settings | The spec additions below; the Pencil design of the PIN and device states, for its last commit |
| m3 — companion protocol v1 | The versioned companion types in `runner-core`, with a golden JSON file for every message | The spec's protocol section |

The two touch different code and could run in either order. m2 goes first because its spec is settled.

Phase 3 builds on both: the PIN exchange and the Noise handshakes, the relay's client channel, the adapter that serves the protocol from `runnerd`'s state, and terminals. Phase 2 opens no connection to a client, so nothing in it can pair a device yet. Until Phase 3, m2's device list and PIN lock are driven by tests.

### m2 — host trust

#### Commits

1. **The host identity key**, in `runner-core/src/remote/` and `runner-daemon/src/remote/`.
   - A secret-store item of its own, named `host-identity`. An origin always starts with its scheme, so the name cannot collide with an enrollment's item. macOS: service `com.wycstudios.runner.remote`, account `host-identity`. Windows: target `com.wycstudios.runner.remote/host-identity`. Development builds use the `runner-dev` names. Its content is `{"v": 1, "key": "<base64 private key>", "pin": null}`.
   - The Linux file store keys nothing today: `OsStore::path` in `store.rs` ignores the name and returns one file. It gains one file per name, `remote-secrets.json` for enrollments as now and `remote-identity.json` for the identity, both created `0600`.
   - `start_owned` in `remote/mod.rs` loads the identity, creating it on first use, instead of calling `HostKey::generate()`. The enrollment item becomes `{"v": 2, "credential": "rhc_…"}`. Logout, removal and the expired cleanup delete only the enrollment item and `remote.json`.
   - Upgrade: restoring a `v: 1` enrollment item when no identity exists adopts its `key` as the identity and rewrites the item as `v: 2`. A host enrolled from `remote-nightly` keeps its fingerprint and stays enrolled.
   - Reset identity runs the sign-out cleanup when enrolled, then creates a new identity with no PIN, empties the device list and ends in Off.
2. **The PIN, its lock and the device list**, in `runner-daemon/src/remote/`.
   - Set and change: exactly six ASCII digits. The PIN is stored in the identity item as entered, so Phase 3's choice between SPAKE2 and CPace does not change what is stored. Changing it keeps the devices.
   - `remote-trust.json` beside `remote.json`, written the same way (temporary file and rename, `0600`, a file that fails to parse set aside as `.bad`): `{"version": 1, "devices": [], "failed_pins": 0, "pin_locked": false}`. Each device has its fingerprint, name, public key, `paired_at` and `last_seen_at`. It holds no secret.
   - The lock: `record_failed_pin` and `record_paired` for Phase 3 to call. Five failures in a row lock PIN entry, a success resets the count, and Unlock clears both.
   - Revoke removes the device from the list; Phase 3 adds closing its channel.
3. **Client protocol and CLI**, in `runner-core/src/protocol/api.rs` and `runner-cli/src/remote.rs`.
   - Requests beside the existing `remote_*` ones: `remote_trust()` (fast) returning `RemoteTrust { fingerprint, pin_set, pin_locked, devices }`; `remote_set_pin(pin)`; `remote_unlock_pin()`; `remote_revoke_device(fingerprint)`; `remote_reset_identity()`. A pushed `remote/trust` event carries `RemoteTrust` whenever it changes. The PIN appears in one request and in no response, event or log, and that request's `Debug` redacts it. None of them is added to `call_daemon`.
   - `runner remote pin`, behind `login_allowed`'s terminal and `RUNNER_HANDLE` guard, reads the PIN twice without echo. `runner-cli` has no input without echo today; `rpassword` is the proposed dependency.
   - `runner remote reset-identity`, behind the same guard, asks `[y/N]` first.
   - `runner remote devices [--json]` and `runner remote revoke <fingerprint>` are allowed inside agent sessions.
   - `runner remote status` gains a PIN line and a device count. The CLI reference is regenerated with `make guide-cli`.
4. **Settings**, in `runner-app/src/surfaces/settings/remote.rs`, built from the signed-off frames in `runner/desktop/specs/836-web-remote-control.pen` in `yeelinks_design`: the PIN row with Set and Change, the locked row with Unlock, the device list with Revoke, and Reset identity behind the existing confirmation dialog. A warning toast when PIN entry locks while the app runs. English and Simplified Chinese strings. `docs/guide/guides/remote-access.md` gains Who can reach this computer.

The new-device notification belongs to Phase 3, since only a pairing raises it.

#### Checks

- **The state,** through the existing `step` tests where the machine is involved: re-enrolling after logout and after removal sends the same `hostKey`; a `v: 1` item is adopted and rewritten; Reset identity signs out, changes the fingerprint, empties the devices and clears the PIN; PIN validation; five failures lock and a success resets the count; Unlock; the device list's round trip and Revoke; `remote-trust.json` written atomically with `0600`, and a bad file set aside.
- **Secrets:** the PIN never appears in `Debug`, a log line, `RemoteTrust`, the pushed event or `remote-trust.json`. The identity item's round trip on macOS and Windows CI, with test names.
- **Agent rules:** `runner remote pin` and `runner remote reset-identity` refused without a terminal and with `RUNNER_HANDLE` set; `runner call remote_set_pin` and `runner call remote_reset_identity` fail as unknown tools; neither command is in the agent guide.
- **The app:** each new row, dialog and the toast rendered from a snapshot, in English and Chinese and in both themes.
- **Gates:** the required workspace gates, each logged to a file with its exit code captured directly.
- **Live,** with Jason on a `remote-nightly` build: a host enrolled before the upgrade keeps its fingerprint; Disconnect then Connect again keeps it, which replaces #951's fresh-fingerprint check; Reset identity changes it and the host leaves the web panel; on macOS, another program's read of the identity item prompts.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m2-host-trust` on `feat/836-m2-host-trust`, with its brief as [`../briefs/836-m2-host-trust.md`](../briefs/), and a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason squash-merges. No database migration. A `remote-nightly` follows the merge when Jason wants an installed build.

### m3 — companion protocol v1

The types and golden files of the spec's Companion protocol section, in `runner-core` alone. Nothing serves them yet: no channel, nothing in `runnerd`, the CLI or the app, so nothing a person can see changes and no guide page does either.

#### Commits

1. **The envelope, handshake and versions**, in `runner-core/src/companion/`, a module of its own beside the build-locked `protocol/`.
   - Two message enums, one per direction, tagged by `type`, with camelCase names. A request carries its `id`; `result` and `error` answer it; a push has none.
   - `hello` and `welcome`, the version choice (the highest both speak, else `unsupportedVersion` with the host's range), `features`, and the error codes.
   - A base64 bytes type for terminal data, the 1 MiB message limit and the 32 KiB chunk size as constants, and a helper that splits terminal bytes into chunks.
   - The compatibility rules as code: unknown fields ignored; fields added later defaulted; each enum's fallback; an unknown push decoded as unknown rather than an error.
2. **The messages**: the overview and its split tree, status with its flattening from `AgentStatus`, the feed and its entries, `follow`, `answer` and `post`, and the terminal messages (`attach`, `snapshot`, `output`, `resized`, `input`, `takeControl`, `viewport`, `detach`, `closed`).

The crew may fold them into one commit if the first does not stand alone.

#### Checks

- **Fixtures** under `runner-core/src/companion/fixtures/`, one per message, read with the round trip Phase 1's `remote/fixtures` uses: each ordinary file parses, and serializing it gives the same JSON. Per Jason's PR #966 follow-up, unknown-field compatibility files instead decode and compare to the known-field form, without changing their wire meaning. The compatibility files from the spec: the handshake both ways and a version mismatch, an unknown field, a version 1 message without later fields, an unknown status and an unknown push.
- **Status flattening**, one test per row of the spec's table, from `AgentStatus` values.
- **The version choice**: overlapping ranges, no overlap, and a client offering versions out of order.
- **Chunking**: empty input, exactly 32 KiB, and one byte over.
- **Gates**: the required workspace gates, each logged to a file with its exit code captured directly.

There is no live check: m3 changes nothing a person can run.

#### Easy to get wrong

- Answering an unknown request with `unknownRequest` needs its `id`, so a request is decoded in two steps: its `type` and `id` first, then the body.
- `serde(other)` covers only unit variants: an unknown push or status needs its own fallback variant, tested by its fixture.
- Base64 is the standard alphabet with padding, which the browser's `atob` and `btoa` read and write.
- The golden files are the web panel's contract too: their paths and names are part of it, and a change to one is a protocol change.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m3-companion-protocol` on `feat/836-m3-companion-protocol`, with its brief as `836-m3-companion-protocol.md`, and a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason merges. No database migration.

### Spec additions before m2

Planning m2 surfaced details the spec did not state. Jason approved them with the plan on 2026-10-19, and they went into the spec with m2's brief:

- Reset identity also clears the PIN: the two share a secret-store item, so a leak of one is a leak of both.
- `runner remote reset-identity`, for a host without the app, behind the same guard as `runner remote pin`.
- The trust requests and the `remote/trust` event, `runner remote devices` and `revoke`, and the new `status` lines.
- The PIN is exactly six ASCII digits, stored as entered.
- `runner remote revoke` names a device by its fingerprint, as `runner remote devices` prints it.

## Phase 3 — the end-to-end channel and live terminals

Proposed on 2026-10-20 with the spec's Phase 3 section. Jason approved starting it the same day with the spec's proposals as drafted, without reviewing every detail, so a crew raises any detail it finds wrong through Runner rather than working around it. Four missions, each one pull request into `nightly-remote`:

| Mission | What | Waits on |
| --- | --- | --- |
| m4 — channel | The route's frames on `/relay/host` and `hello`'s `features`; the channel in `runner-core/src/companion/channel/`, CPace, both Noise handshakes, records and credit, with its vectors; pairing and connecting in `runnerd` against `remote-trust.json`, Revoke and Reset identity closing channels; the `remote/paired` event and `connected` in `RemoteTrust` and `runner remote devices`; the adapter's `hello` and `welcome` alone. No change to the app | — |
| m5 — adapter | The overview, status and terminal requests (`attach`, `input`, `takeControl`, `viewport`, `detach`) with their pushes; the snapshot's history limit; the controller in `runnerd` and its client-protocol changes, without the card. `feed`, `follow`, `unfollow`, `answer` and `post` are left out of `welcome`'s `features` for a later mission, which needs no protocol version (Jason, 2026-10-20) | m4 |
| m6 — host desktop | The control card over panes, tabs and missions, Take back and Continue here, and the pane's size push; the pairing notification and the Connected now row; #838's two-window rerun; the guide | m5; the frames for the notification and the row |
| m7a — Account | Settings → Account and the sidebar's avatar menu, with signing in and out moved out of Remote access; Remote access's relay pages, the route choice with Tailscale Later and Reset remote access, on today's device list | m6 |
| m7b — the client core | The channel's client side in the client computer's `runnerd`, its keys per paired computer, and the companion protocol's client; `ListAccountHosts` and `/relay/client` with the computer's credential; the CLI's view of other computers | m5; `runner_server`'s R2 |
| m7c — the client app | The computer picker, the PIN dialog, another computer's window and terminals in the desktop's terminal view, and Remote access's computers list | m7a, m7b |

**Revised on 2026-10-21 (Jason).** The client is a Runner desktop, not a browser, and the service only relays; the spec's [Clients are Runner apps](../../features/836-web-remote-control.md#clients-are-runner-apps) has why. m4 and m5 stand as built. m6 keeps its scope with the client's device name in its card and notification. m7 is new. A fix from the first live pass, [`fix/836-mirror-size`](https://github.com/yicheng47/runner/tree/fix/836-mirror-size), makes the desktop's terminal mirror follow the size `runnerd` applied, since the desktop ignored a browser's control while drawing; it lands before m6. The order is m6 and m7 in parallel, since m7 waits on the service and its own frames, then Phase 4.

**Revised on 2026-10-22 (Jason).** m7's frames are signed off: a computer picker rather than a sidebar section, one route per computer, chosen once, and the account in its own Settings pane, with an avatar menu of Account, Settings and Sign out, as the spec's [Runner as a client](../../features/836-web-remote-control.md#runner-as-a-client) describes. m7 builds Remote access's route choice with Tailscale marked Later, and the relay's pages; Tailscale's pages and finding computers on the tailnet come with Phase 4.

### m4 — the channel

#### Commits

1. **The channel**, in `runner-core/src/companion/channel/`, with no I/O.
   - CPace, CPACE-RISTR255-SHA512 from draft 21, initiator-responder, on `curve25519-dalek`'s ristretto255 (`RistrettoPoint::from_uniform_bytes` for the generator) and `sha2`'s SHA-512, with the identity check on `K`. Both crates are already in `Cargo.lock` through `snow`; they become direct dependencies at the same versions, with no new crate in the lockfile.
   - The frame kinds and their parsing, then the handshakes as pure state machines that take frames in and give frames and outcomes out: the host's (`pair` → `pairReply` → `pairFinish`, `connect` → `accept`, `reject`) and a browser's, used by the tests and the vectors. `snow`'s `XXpsk3` with the PSK set at position 3 from the ISK's first 32 bytes, and `IK`.
   - Records: encoding, a message's assembly with the 1 MiB limit, and credit, which is the sender's remaining allowance and the receiver's owed bytes with the 64 KiB threshold.
   - Vectors in `vectors/`, JSON with hex bytes: appendix B.3's CPace vectors; a pairing, a connect, a wrong PIN and `unknownDevice`, frame by frame, from fixed static keys, ephemeral keys (`snow`'s `fixed_ephemeral_key_for_testing_only`), CPace scalars and `sid`; and a record sequence with `credit`.
2. **The route and pairing in `runnerd`**, in `runner-daemon/src/remote/`.
   - `hello` gains `"features":["channels"]`, with its golden file. `channelOpen` and `channelClosed` are read, `closeChannel` is sent, and binary frames are routed by channel number. `tokio-tungstenite`'s size limit applies to every message, so it becomes 65,540 bytes and the 64 KiB text limit is kept in code.
   - One task per channel, fed by the remote task, whose writer sends every channel's frames; at most 16 channels; the 30-second handshake deadline; a frame for a channel that is not open is dropped.
   - Pairing against the identity item's PIN and `Trust`: `noPin` and `pinLocked` at `pair`, the lock checked again at `pairFinish`, `pairFinish` frames read one at a time, `record_failed_pin` and `record_paired`. Connecting against the device list, updating `last_seen_at`.
   - Revoke closes that device's channels; Reset identity, Pause, Disconnect and the connection dropping close all of them. `RemoteTrust`'s devices gain `connected`, and a pairing pushes `remote/paired` with the name and fingerprint.
   - After the handshake, records and credit, and the companion handshake alone: `hello` is answered with `welcome`, whose `features` is empty, and every request with `unknownRequest`. m5 serves the rest.
3. **The CLI**: `runner remote devices` and its `--json` show `connected`. Regenerate the CLI reference if help text changes.

#### Checks

- **Vectors**: CPace's appendix B.3; each channel vector reproduced from the browser's side and from the host's side.
- **The channel**: each reject reason; a frame out of order, of the wrong length or failing decryption; a non-canonical or identity ristretto point; a message over 1 MiB; parts of two messages interleaved; credit, so a sender stops at zero and resumes on `credit`, and `credit` is owed at 64 KiB.
- **`runnerd`**, with the existing fake service and a test browser over it: pairing and connecting end to end to `welcome`; PIN entry locking at the fifth wrong PIN, including five `pairFinish` frames sent at once; Unlock; Revoke closing the device's open channel; Reset identity, Pause and Disconnect closing all; the 17th channel rejected with `busy`; the handshake deadline on a fake clock; a frame for an unknown channel dropped; a reader that grants no credit holding up only its own channel; `connected` and `remote/paired`.
- **Secrets**: no PIN, ISK, key, frame or record body in a log line or `Debug` output.
- **Gates**: the required workspace gates, each logged to a file with its exit code captured directly.

There is no live check: until the service's R1 and R2 exist, nothing can open a channel to a real host.

#### Easy to get wrong

- In `XXpsk3` the host must set the PSK before reading the third message, and the PSK is the ISK's first 32 bytes, not the ISK.
- CPace rejects a point that does not decode as canonical ristretto255 and a `K` that is the identity; both are `badHandshake`, never a counted wrong PIN.
- Count every in-order, structurally valid `pairFinish` authentication failure, including ciphertext or tag tampering: it is indistinguishable from a wrong PIN (Jason, 2026-10-20). Invalid kinds, order, lengths or CPace points, and successfully decrypted invalid payloads, are `badHandshake` without counting.
- The relay's writer is shared by every channel: one channel without credit must not block the socket, the heartbeat or the other channels.
- A channel's task failing ends that channel and is logged; it never ends the remote task or `runnerd`.
- Tests do not sleep and reach no network.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m4-channel` on `feat/836-m4-channel`, with its brief as [`../briefs/836-m4-channel.md`](../briefs/836-m4-channel.md), and a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason merges. No database migration.

m4 ends at the companion handshake so that the service's browser channel can be tested live against a real host before the adapter exists. The service's side runs alongside in `runner_server`: R1, `/relay/client` and the host's channel frames, and R2, the channel in TypeScript against m4's vectors, can start from the approved spec; R3 and R4, the host and terminal pages, are designed first. The first live terminal needs m4, m5, R1, R2 and a bare R4; m6 makes the desktop side right and can follow.

[#838](https://github.com/yicheng47/runner/issues/838) is not part of it, since the controller treats every desktop window as one. Jason could not reproduce it on 2026-10-19, so m6 reruns its two-window steps when it changes the pane's size push, without blocking on it.

### m5 — the adapter, terminals first

m4 merged as [#975](https://github.com/yicheng47/runner/pull/975): a paired browser gets `welcome` with empty `features`, and every request after it gets `unknownRequest`, from `serve` in `runner-daemon/src/remote/channels.rs`. m5 replaces that loop's request handling with the adapter.

#### Commits

1. **The controller and the snapshot's history**, in the session manager and `runner-terminal`.
   - Each live terminal gains its controller in memory: the desktop, a browser with its channel and device name, or none; the desktop when `runnerd` starts. `resize_with_origin` applies a client-protocol size only while the desktop controls. Without a browser nothing changes: the desktop always controls, and TERM-RESIZE-01 and TERM-RESIZE-02 must pass unchanged.
   - Browser-side operations for the adapter: attach with `take` or `ifFree`, take control, a viewport from the controller, and release on detach or channel close, each following Terminal size's table, with the 20×8 minimum.
   - The client protocol carries the controller with the size, in the snapshot, the metadata and `Resized`, and gains `terminal_take_back(session_id, cols, rows)`. The app compiles against it and ignores the controller until m6; it changes nowhere else.
   - `TerminalModel` gains a snapshot whose scrollback is cut to a number of lines; the client protocol's own snapshot is unchanged.
2. **The adapter**, in `runner-daemon/src/companion/`.
   - `AppCore` owns the remote handle, so the adapter reaches `AppCore` without a reference cycle: a narrow trait the daemon implements over a `Weak<AppCore>`, which the tests implement over a fake.
   - `overview` from the sidebar's nodes, projects, sessions, `session_status_snapshot` and the nodes' viewed and completed marks; its push follows each `AppEvent` that changes any of them, at most once every 250 ms. `status` pushes for one session's status or unread change. The status flattening is m3's.
   - `attach`, `input`, `takeControl`, `viewport` and `detach`: the snapshot in 32 KiB chunks with `seq`, then `output` and `resized` from the subscription, `closed` when the terminal ends, and a fresh snapshot for the subscription's resync. An attached stream owns a history-limited final snapshot across process exit, without retaining the terminal model or allowing stopped-session attaches. Snapshot chunks also carry REP and unfinished-parser continuation metadata for the xterm decoder. `take` marks the session's node viewed.
   - `input` reaches the terminal's input queue and the input observation the desktop's typing feeds, so draft-aware delivery and status detection see a browser's typing. The adapter classifies the bytes: a bracketed paste as a paste, a lone carriage return as submit, printable text as content, and the rest as navigation.
   - The send order from the spec's Records section: results, errors and other pushes first; terminal streams then share the credit in turn, a chunk each; a terminal's queue past 1 MiB is dropped for a fresh snapshot.
   - `welcome`'s `features` lists `overview`, `attach`, `input`, `takeControl`, `viewport` and `detach`. `feed`, `follow`, `unfollow`, `answer` and `post` keep answering `unknownRequest`.

#### Checks

- **The controller**: every row of Terminal size's table, a desktop size ignored while a browser controls, the minimum size, and a browser's channel closing; the existing terminal-size tests unchanged.
- **The adapter**, over m4's test browser and a fake core: the overview's shape against m3's fixtures, its 250 ms coalescing on a fake clock, `status`; `attach` with the history limit, chunking and `seq`, `output`, `resized`, `closed` and resync; `input` reaching the queue and its observation; the send order, a busy terminal not holding up another, and the 1 MiB queue becoming a snapshot; subscriber overflow and adapter overflow followed by exit both deliver final snapshot/resized/closed and release state; shared REP fixtures and the xterm 6.0.0 headless decoder check cover attach/resync and unfinished parser bytes; requests outside `features` answered `unknownRequest`.
- **Secrets and logs**: no message body or terminal byte in a log line.
- **Gates**: the required workspace gates, each logged to a file with its exit code captured directly.
- **Live, pending for Jason on macOS and Windows**: TERM-RESIZE-01, TERM-RESIZE-02, TERM-INPUT-01 and the draft-held mission message check. Jason removed QA from m5 on 2026-10-21; after clean review the mission proceeds directly to commits, push, PR and CI. A browser cannot reach the host until the service's R1 and R2 exist, so the adapter has no live check of its own.

#### Easy to get wrong

- The desktop's size push is unchanged without a browser: the controller check must not delay, reorder or drop a desktop resize while the desktop controls.
- A resize from a browser must reach the desktop app as `Resized`, as a resize from another desktop window does today, so the app redraws at the new grid.
- The overview push is coalesced but never lost: the last change inside a 250 ms window is always sent.
- `seq` continues across a resync's fresh snapshot; the browser drops everything before it.
- The adapter holds no `Arc<AppCore>`; a channel outliving `runnerd`'s shutdown must not keep the core alive.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m5-adapter` on `feat/836-m5-adapter`, with its brief as [`../briefs/836-m5-adapter.md`](../briefs/836-m5-adapter.md), run by coder and reviewer after Jason removed the qa slot on 2026-10-21; a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason merges. No database migration. Merged into `nightly-remote` on 2026-10-21 as [#976](https://github.com/yicheng47/runner/pull/976), after a follow-up mission fixed the two problems Jason's review found.

### m6 — the host desktop

m5 and [#986](https://github.com/yicheng47/runner/pull/986) left `runnerd` keeping each terminal's controller, the client protocol carrying it as `TerminalController` in the snapshot, the metadata and `Resized`, with `terminal_take_back`, and the app's mirrors adopting a size only from the snapshot and ordered `Resized` frames; m4 left `remote/paired` and `RemoteTrust`'s `connected`. The app still ignores the controller. m6 is the desktop's side of another Runner using this computer, drawn on 2026-10-22 in `836-web-remote-control.pen`: steps 2.8 to 2.10 and 3.6 to 3.10 of its journey, and Connected now in today's device list. The client protocol's `Browser` variant now means a remote Runner client; its `name` is the computer's name from pairing.

#### Commits

1. **The control card**, in the app.
   - A pane whose terminal the desktop does not control stops pushing its size, from every path that pushes it today, and draws the card over the terminal: the two-window overlay's scrim and centered card from `ui/duplicate_subject_overlay.rs`, reading Controlled from {name} with Take back while a remote client controls it, and {name} has left with Continue here when nobody does. For has left the app uses the last name it saw for that terminal, and Another computer has left when it saw none.
   - Take back, Continue here, or Enter on the card sends `terminal_take_back` with the pane's size; once the desktop controls again the pane pushes its size as before. While the card shows, keys, pastes, IME composition, file drops and every other input path of that pane reach the card or nothing, never the terminal.
   - A tab or mission whose terminals one remote client controls draws one card below its tab bar, naming how many terminals it uses; Take back sends `terminal_take_back` for each terminal at its own pane's size. A terminal controlled on its own gets its own pane's card. Several windows showing the same tab each draw the card, and Take back works from any of them.
   - Its strings in `en.toml` and `zh-hans.toml`, from the frames; the light theme follows step 3.9.
2. **The pairing notification and Connected now**, in the app.
   - The app follows `remote/paired` as it follows `remote/status` and `remote/trust`, and shows a system notification through `notifications.rs`, {name} paired with {name} can now reach this computer., whose click opens Settings → Remote access. When system notifications are off, it shows an in-app toast with the same sentence and Open Remote access, as other notifications fall back today.
   - Remote access's device list reads Connected now, with a green dot, in place of a device's last-seen time while `connected` is true. The copy loses the browser and the later update: the intro, the empty list and the hint say other computers pair from their own sidebar with this computer's PIN.
3. **The guide.** `docs/guide/guides/remote-access.md` gains the desktop's side: the notification, Connected now, the card, Take back and Continue here. m7 adds pairing and using another computer.

#### Checks

- **The app's tests**: the card's state through each controller change (the desktop, a remote client, none, the desktop again) for a pane and for a tab or mission; no size pushed while the desktop does not control, from any path; Take back, Continue here and Enter sending `terminal_take_back` for each terminal at its pane's size; no input reaching a covered terminal; `remote/paired` showing the system notification, or the toast when notifications are off; Connected now following `connected`.
- **Gates**: the required workspace gates, each logged to a file with its exit code captured directly.
- **Live, pending for Jason**: TERM-RESIZE-01, TERM-RESIZE-02 and #838's two-window steps, since the pane's size push changed, on a `remote-nightly` build on macOS and Windows. The card, Take back, Continue here, the notification and Connected now need another Runner to connect, which arrives with m7; they run then. Jason chose a duo for m6 on 2026-10-22 for this reason.

#### Easy to get wrong

- Without another Runner nothing changes: the desktop always controls, and the pane pushes its size exactly as today.
- A covered pane pushes no size on any path: the window, a split or drawer drag, zoom, a second window, switching tabs.
- After Take back the pane pushes again at once; the suppression does not outlive the card.
- The mirrors' rule from #986 stays: a size is adopted only from the snapshot and `Resized`.
- No input leaks into a covered terminal, including paste, drag and drop and the composer paths that send to the terminal.
- One notification per pairing; a device that pairs again after `unknownDevice` is a new pairing.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m6-host-desktop` on `feat/836-m6-host-desktop`, with its brief as [`../briefs/836-m6-host-desktop.md`](../briefs/836-m6-host-desktop.md), run by a codex coder and reviewer; a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason merges. No database migration, no new crate.

### m7a — Account

R2 merged in `runner_server` on 2026-10-22 as #21: one enrollment per computer, its host credential also listing the account's computers and opening `/relay/client`. So signing in needs no new service call. m7a is the desktop's side of the design's Account band and Remote access's relay pages, on the client protocol's existing `remote_*` requests: steps 1.1, 1.6, 1.7 and 5.1 to 5.4, and section 6, of `836-web-remote-control.pen`. Nothing in it reaches another computer; the computers list and "Sign in to Runner" starting the client are m7c's.

#### Commits

1. **Settings → Account.**
   - A pane with the route key `account` and the lucide `user-round` icon, first in Settings' App group, for the hosted service's sign-in.
   - Its states are Phase 1's enrollment states, moved from Remote access:
     - signed out, with the private-beta note and Sign in;
     - waiting for approval, with the code, fingerprint, link and time left;
     - signed in, with the account, service, this computer and its fingerprint, and what the account is for (remote access through the relay, and Sync roles and crews marked Later);
     - each stop reason's card, with Sign in again where Phase 1 had Connect again.
   - Sign out on this computer replaces Disconnect, with its confirmation retitled. The CLI's `runner remote login` and `logout` are unchanged.
2. **The avatar menu.**
   - The sidebar footer's Settings gear becomes a round avatar: the account's initial, from its display name, when signed in, and a local-user icon otherwise.
   - It opens a menu: the account and the service; Account; Settings (⌘,); and Sign out…, which asks first. Signed out, the header reads Not signed in with Private beta · mainland China, then Settings, then Sign in….
   - The footer's height and its usage values keep their place.
3. **Remote access.**
   - Signed out, the pane shows the route choice: Runner's relay first, with Sign in to Runner, which starts the same enrollment and shows its waiting card in the pane; and Tailscale, marked Later with its button off.
   - Signed in, it shows the relay page: the status card with the account, computer, fingerprint and service; Allow remote access; the PIN; Reset remote access; today's device list with m6's Connected now; and a Route section naming the relay, chosen once, with Signed in as @account and a link to Account.
   - Reset remote access is Reset identity renamed. It already signs the computer out, so the pane returns to the route choice.
   - Paused, reconnecting, PIN entry locked and its toast keep their behaviour in this layout.
4. **The guide.** `docs/guide/guides/remote-access.md` gains signing in from Account, the avatar menu, and the route choice with Tailscale coming later.

#### Checks

- **The app's tests**:
  - Account's states from `RemoteStatus`, including each stop reason;
  - Sign in and Sign out calling the existing requests, with Sign out confirmed first;
  - the avatar's initial and its signed-out icon, and the menu's items in both states;
  - Remote access showing the route choice while signed out and the relay page while signed in;
  - Tailscale's button disabled;
  - Reset remote access returning to the route choice;
  - the sidebar footer's layout.
- **Gates**: the required workspace gates, each logged to a file with its exit code captured directly.
- **Live, pending for Jason** on a `remote-nightly` build, macOS and Windows: sign in from Account against the hosted service, and from Remote access's route choice; the avatar and its menu; sign out; pause and resume; Reset remote access.

#### Easy to get wrong

- Sign in, Sign out and Reset reuse `remote_enroll_start`, `remote_enroll_cancel`, `remote_logout`, `remote_retry` and `remote_reset_identity` as they are; no `runnerd` or client-protocol change.
- Signed out but enrolled-and-stopped is not signed in: a stop reason shows on Account, and Remote access shows the route choice with the reason linked to Account.
- Every string in `en.toml` and `zh-hans.toml`. The account's name, never a credential or code, appears in the avatar menu.
- The sidebar footer is shared by macOS and Windows; both keep their layout.

#### Landing

A worktree cut from `origin/nightly-remote`, `.worktrees/feat-836-m7a-account` on `feat/836-m7a-account`, with its brief as [`../briefs/836-m7a-account.md`](../briefs/836-m7a-account.md), run by a codex coder and reviewer; a pull request against `nightly-remote`, driven green on macOS and Windows, which Jason merges. No database migration, no new crate.

## Phase 4 — Tailscale

Planned when its development starts. Its open questions are on [#954](https://github.com/yicheng47/runner/issues/954); the pairing and trust model are Phase 2's, unchanged. Since 2026-10-21 its client is m7's: one Runner reaches another directly over the tailnet, with no service involved, so Phase 4 adds the host's listener and the client's direct route.
