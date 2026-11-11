# Harden remote control routes and cut typing, click and reconnect latency

Tracking issue: [#1263](https://github.com/yicheng47/runner/issues/1263). No design file: this changes no screen. The machinery it changes is walked through in the [remote control note](../tech/remote-control.md), whose section numbers this spec cites as "note §n".

Signed off by Jason on 2026-11-11, with the four [decisions](#decisions) below. The spec covers four pull requests, so it lives on `main` and not on one of their branches.

## Motivation

A review of the remote control code on 2026-11-11 found no hole in the channel's cryptography and twenty places where the route around it is fragile or slower than it needs to be. Three of them are felt today:

- **Clicks are slow and can fail while the channel is fine.** Every attach, take control, action and overview request re-reads secrets and may call the service before it uses the open route. When that call fails or is rate-limited, every one of them fails for up to a minute.
- **A dead route looks alive.** After sleep or a change of network the window stays "connected" at 10,000 ms until the operating system gives up on the connection, which takes minutes.
- **Every network blip repaints everything.** A reconnect resends a full snapshot of every open terminal.

The rest is overhead and fragility that shows under load: about six frames per keystroke, one slow request ending the whole route, typing dropped while the route writes, and the whole overview pushed ahead of terminal output whenever any agent changes status.

The iPhone client ([#912](https://github.com/yicheng47/runner/issues/912)) will be a second implementation of the protocol and a Tailscale route ([#954](https://github.com/yicheng47/runner/issues/954)) a second transport. Wire changes are cheapest before either exists.

## Findings

Numbers match the issue. "Client" is the client's `runnerd`, "host" the host's.

| # | Finding | Where | Phase |
| --- | --- | --- | --- |
| 1 | Every request re-resolves the computer (enrollment file, three to four secret-store reads, the service's `ListAccountHosts` once the 5-second cache expires) before it looks for the live route; a failed or rate-limited directory call fails requests on a healthy channel | `remote/client/mod.rs`, `Client::route`, `computers`, `context`, `keys` | 1 |
| 2 | The client's route has no liveness deadline, and a ping never ends a route | `remote/client/route.rs`, `run` | 1 |
| 3 | One request past its 30-second deadline ends the route and reattaches every terminal | `route.rs`, `run`, "Remote request timed out" | 1 |
| 4 | Client reconnects at exactly 1, 2, 4, 8, 16, 30 s, with no jitter | `route.rs`, `run` | 1 |
| 5 | Input goes through a 32-command queue with `try_send`, and the loop does not read it while it writes (up to 10 s per frame), so typing is dropped on a healthy route | `route.rs`, `Handle::input`, `flush` | 1 |
| 6 | Credit counts message bytes; the relay's 1 MiB queue and the host's inbound budget count each frame plus 64 bytes, so small messages can overflow them (close 4413) before credit stops the sender | `channel/records.rs`; `remote/channels.rs`, `feed` | 2 |
| 7 | An unmatched `result` is decoded speculatively as an `Overview` | `route.rs`, `run` | 1 |
| 8 | The host's relay loop reads, writes every channel's frames one at a time and runs the state machine in one task; a waiting write stops all reads; each frame recomputes the trust snapshot, an X25519 operation, up to twice | `remote/mod.rs`, `run`, `send_frames`, `publish` | 2 |
| 9 | `hello` and `welcome` cost a round trip after `IK`, whose payloads are empty | `channel/handshake.rs`; `route.rs`, `welcome` | 3 |
| 10 | No resume: every reconnect sends a full snapshot of every open terminal | `remote/client/terminal.rs`, `reattach` | 3 |
| 11 | A keystroke is about six frames: `input`, a discarded `result`, the echo, and a `credit` record for each | `companion/mod.rs`, `Adapter::request`; `records.rs`, `handled` | 2, 4 |
| 12 | Terminal bytes travel as base64 inside JSON | `runner-core/src/companion/mod.rs`, `TerminalBytes` | 4 |
| 13 | The send window is 512 KiB whatever the link carries, so one pane's flood delays another pane's echo and every reply | `channel/mod.rs`, `INITIAL_CREDIT` | 4 |
| 14 | The whole overview is rebuilt and pushed, ahead of terminal output, on any `session/`, `chat/`, `mission/`, `project/`, `crew/`, `slot/` or `event/appended` event, with no check that it changed | `companion/mod.rs`, `relevant`, `Adapter::changed` | 2 |
| 15 | Every size step of a window drag is its own `viewport` request, each a blocking resize on the host | `remote/client/mod.rs`, `queue_viewport` | 2 |
| 16 | Nothing measures keystroke-to-echo time; the ping skips the host's queues by design | `route.rs`, `Latency` | 1 |
| 17 | Every relay ping is answered twice: tungstenite queues a pong itself | `route.rs`, `read`; `remote/mod.rs`, `run` | 1 |
| 18 | Private keys, the PIN-derived key and credentials are not zeroized on drop | `runner-core/src/remote/mod.rs`, `HostKey`, `Secret`; `handshake.rs`, `HostPair` | 1 |
| 19 | One OS thread per attached terminal per channel | `companion/mod.rs`, `Stream::new` | Left as is |
| 20 | `NoiseKeys::fixed_ephemeral`, a test-vector hook, is a field of the production type | `handshake.rs` | Left as is |

## Scope

Four phases, each its own pull request, each useful without the next. Phases 1 and 2 change no wire format. Phases 3 and 4 add negotiated, additive wire changes; the companion protocol version stays 1.

Out of scope: a direct or Tailscale route (#954), QUIC, predictive local echo, compression (message lengths are visible to the relay, so compressing secrets beside attacker-influenced text leaks through size), any change to pairing or the Noise patterns' key agreement, any screen, and the relay's internals, which belong to the service's repository.

Left as is, with the reason:

- **19, a thread per stream.** `TerminalSubscription::recv` is blocking by contract and the threads sleep. Replace them only when the subscription gains an async receiver for another reason.
- **20, `fixed_ephemeral`.** The daemon's tests in another crate use it, so `cfg(test)` cannot hide it, and both production call sites pass `None`.

## Decisions

Jason, 2026-11-11:

1. **A silent route ends itself (finding 2).** #1229 decided that a ping never changes a route, so that a slow reading could not cause a reconnect. That stays true of the latency reading. A separate liveness rule is added: a route that has received nothing for 15 seconds is treated as lost and reconnects.
2. **Typing during a short write stall is kept, not dropped (finding 5).** Up to 1 MiB of input per route waits in order while a write is slow. It is still discarded, with today's error, when the route reconnects, so nothing is ever typed into a screen the person has not seen.
3. **Resume costs the host memory (finding 10).** The host keeps the last 256 KiB sent on each attached terminal, and keeps a dropped channel's terminals for 60 seconds so the same device can continue.
4. **Order and timing.** Phases 1 and 2 first, as they need no host and client to match. Phases 3 and 4 before the iPhone client's protocol work starts. Each phase goes to a claude trio, confirmed at that mission's start.

Decision 2 amended by Jason on 2026-11-11 after code review: the original 64 KiB budget collided with the client protocol's 64 KiB IPC paste chunking (reviewer, 2026-11-11). A later paste chunk could reach the route before the first chunk released its budget, dropping the paste's tail, including its closing bracketed-paste marker. The input budget is therefore `c::MAX_MESSAGE_SIZE` (1 MiB), shared with the existing message cap rather than a new constant. Input is still discarded across a reconnect.

## Security

The review found no weakness in the channel: pairing, the two Noise patterns, the PIN lock and what the relay can and cannot do (note §12) stay as they are. This work must keep it that way, and three of its changes touch what the channel protects:

- **Secrets cached in memory (phase 1).** The enrollment credential and the client keys stay in the daemon's memory after their first read. The daemon already holds the host identity and every channel's session keys there, so this adds no new place a secret can be read from; the cache is dropped on sign-out and the buffers are zeroized.
- **A payload in the first handshake message (phase 3).** Noise gives that payload no forward secrecy and no replay protection. The rules under [Hello inside the handshake](#hello-inside-the-handshake-9) are requirements, not suggestions: `hello` carries nothing secret, and the host changes no state for it until the first transport record decrypts. This is the change to review most closely.
- **Lingering terminals (phase 3).** A lingering stream is handed only to a channel authenticated with the same device key that attached it. A different paired device, or the same device after Revoke, gets a snapshot under the ordinary rules; Revoke, Reset identity and turning remote access off drop every lingering stream at once.

The raw envelope of phase 4 travels inside the Noise seal like any other message. Nothing is compressed, and no frame gains a field the relay can read.

## Phase 1: route robustness and measurement

Client `runnerd` only. Works against every host.

### Use the open route first (1)

`Client::route` looks for a live route by computer id before anything else and returns it. It resolves the directory, the credential and the keys only to open a route.

`Client` keeps the `Context` and the `ClientKeys` in memory after their first read. The cache is dropped when the remote status changes origin, host id or sign-in state, and updated by `pair`, `forget` and `reset`, which already write the keys. Nothing else reads the secret store on a request.

A live route is authoritative: the relay closes a channel whose client or account is removed, and the host closes one whose device is revoked, so a route needs no directory check to stay valid. `computers` keeps its cache for the Settings list and the host popup.

### Notice a dead route (2)

The route records when it last received any WebSocket message. With nothing received for 15 seconds it ends with `Remote route timed out`, which takes the existing reconnect path and shows Reconnecting.

Hosts that serve `ping` answer one every 5 seconds, so three missed answers end the route. For a host without `ping` the route sends a WebSocket ping to the relay every 5 seconds, which the relay answers; that covers the client's own leg, the one sleep and network changes break.

The latency reading is unchanged: an unanswered ping is still a 10,000 ms sample and still never ends a route by itself.

### A timed-out request fails alone (3)

A request past its deadline gets `Remote request timed out` and leaves `pending`; the route stays up. A reply that arrives later matches nothing and is dropped. An `attach` that times out is cleaned up as a refused one is. A link that has really died is caught by the liveness rule.

### Jitter (4)

Both reconnect delays become half the step plus a random amount up to the other half: 0.5–1 s, 1–2 s, up to 15–30 s. A `Retry-After` from the relay is used as given.

### Never block the loop on a write (5)

The route splits its socket. A writer task owns the sink and takes sealed frames from a bounded queue; the loop seals a frame only when the queue has room, so it keeps reading commands and incoming frames while a write is slow. Frames are sealed in queue order, which is the order Noise's nonces need.

Input moves from the 32-command queue to its own buffer on the route, in arrival order, at most `c::MAX_MESSAGE_SIZE` (1 MiB). Input past the cap, and everything buffered when the route's generation changes, is discarded with today's `session/input-error`.

### Track the route's own overview requests (7)

The route records the ids of the `overview` requests it sends itself. Only a `result` with one of those ids is decoded as an overview; any other unmatched result is dropped.

### Measure the echo (16)

The route times the first `output` on a terminal after an `input` to it, one sample per input burst, and keeps the median of the last five per route. The `remote/latency` event gains `echo_ms` beside `ms`, and the route logs both once a minute while there are samples. No screen changes; the popup keeps the ping.

This lands first so that phases 2 to 4 can be measured.

### Hygiene (17, 18)

Remove both explicit pong sends. `HostKey::private`, `HostPair::psk` and the intermediate session key are held in `Zeroizing`; `Secret` zeroizes on drop.

## Phase 2: host loop and flow control

Host `runnerd`, plus two sender-local changes that both ends make. No negotiation.

### Split the relay socket (8)

A writer task owns the host socket's sink. Channel tasks send their wrapped frames to it through one bounded queue, as do `closeChannel` and the pong, so a channel's last frame and its close keep their order. The writer feeds every frame that is ready and flushes once; a write that takes 10 seconds fails the connection as today.

The remote loop reads the socket and runs the state machine, and never awaits a write. A frame from a channel that has since closed is harmless: the relay drops frames for a closed channel number and never reuses a number on a connection.

`publish` runs when a command, a handshake, a trust write or a channel opening or closing changed something, not once per frame, and the machine caches its fingerprint.

### Bound in-flight frames by the relay's measure (6)

The sender keeps a first-in-first-out list of the frames it has sent and not yet had credit for, each with the relay's cost: the frame's length plus 64 bytes. Returning credit pops it. A sender holds its next frame while the list's cost would pass 768 KiB, so the relay's 1 MiB queue and the host's inbound budget cannot overflow from data. Both ends do this in `Records`; a peer that does not is no worse off than today.

### Grant credit in batches (11)

`Records::handled` stops forcing a grant after every message. Credit goes out once 64 KiB is owed, as it already does in the middle of a long message. A sender stalls only with no credit at all, when the receiver owes the whole window, so nothing can deadlock, and an older peer accepts any grant up to what it has spent. The test vectors in `channel/vectors/channel.json` are regenerated.

### Push an overview only when it changed (14)

The adapter hashes each overview it builds, leaving out the per-session status that `status` pushes already carry, and pushes it only when the hash differs from the last one pushed on that channel. Replies to `overview` requests are unaffected.

### Send the newest size only (15)

The client's route keeps one pending `viewport` per terminal and sends the newest when the writer has room, so a drag costs the host one resize per write, not one per step. `takeControl` and `attach` are not coalesced.

## Phase 3: faster connect and resume

Wire changes, negotiated.

### Hello inside the handshake (9)

`IK`'s first message carries the client's `hello` as its payload and the second carries the host's `welcome`, saving one round trip between the two computers on every connect.

- **Negotiation.** A host that reads payloads lists `earlyHello` in `welcome`. The client stores that with the host's key and uses the early form on later connects. An older host rejects a first message that is not exactly 96 bytes with `badHandshake`; the client then retries once in today's form and clears the flag.
- **What the payload may do.** Noise gives the first message's payload no forward secrecy and no replay protection, and `hello` holds only the client's versions, kind and build. The host therefore acts on it (the device's last-seen time, kind and Connected state) only after the client's first transport record decrypts, as it does today after `hello`.
- A forged `reject` still only causes a retry in the older, equally safe form.

### Resume a terminal after a reconnect (10)

- **Host.** Each stream keeps the last 256 KiB of the `output` and `resized` messages it sent, with their `seq`. When a channel drops, its streams linger for 60 seconds, still subscribed and queueing, keyed by the device's fingerprint and the session. Control is released at once, as today, so the host desktop is never held by a client that is gone.
- **Client.** After a reconnect, each open terminal's `attach` carries `resume` with the last `seq` it applied. It keeps its snapshot and replay instead of marking them invalid.
- **Reply.** If the lingering stream still holds everything after that `seq`, the host answers `{"resumed": true}`, replays from `seq + 1` and continues; the pane neither resyncs nor repaints, and keeps its scroll position. Otherwise, and on any host without `resume` in `features`, the host answers as today and sends a snapshot.
- A resumed terminal is attached with `ifFree`, as a reconnect attaches today, so control follows the existing rules.

## Phase 4: lean data path

### Input without a reply (11)

`hello` gains an optional `features` list. To a client that lists `quietInput`, the host answers an `input` only with an error. Older hosts ignore the field and older clients never send it.

### Raw terminal bytes (12)

A message may be a binary envelope instead of JSON: `0x01`, a 4-byte big-endian header length, the JSON header (today's message without `data`), then the raw bytes. JSON messages start with `{`, so the first byte tells them apart. It applies to `output`, `snapshot` and `input`. The host lists `rawTerminal` and sends envelopes to a client that lists it; the client sends raw `input` to a host that lists it. This removes the base64 third from terminal traffic and two copies from the path. It does little for a single keystroke, whose header dominates.

### Pace by delivery, not only by bytes (13)

The frame list from phase 2 also records when each frame was sent. While more than 128 KiB is unacknowledged and the oldest such frame is older than 500 ms, the host sends no terminal output on that channel; pongs, results and pushes still go. In-flight data then tracks what the link delivers in about half a second, so a flood in one pane delays another pane's echo by that much and not by a full window.

Build this last and only if phase 1's echo reading shows the problem: skip it if typing in one pane while another floods stays under 300 ms on a home connection.

## Compatibility

| Change | New client, older host | Older client, new host |
| --- | --- | --- |
| Phase 1 | Works; liveness uses WebSocket pings where the host lacks `ping` | Unaffected |
| In-flight bound, batched credit | Each end bounds what it sends; grants stay within what an older peer accepts | Same |
| `earlyHello` | First connect in today's form; early form after the host lists it; one fallback retry after a downgrade | Host accepts both forms |
| `resume` | Client sends no `resume`; snapshot as today | Host never receives `resume`; snapshot as today |
| `quietInput`, `rawTerminal` | Not offered by the host, so not used | Client lists neither, so the host uses neither |

## Guide and docs

- `docs/guide/guides/remote-access.md` and its `zh-CN` page: a route that hears nothing for 15 seconds reconnects; a reconnect keeps a terminal's contents and scroll position when the host can resume it; typing during a short stall is kept, and typing across a reconnect is still discarded.
- `docs/tech/remote-control.md`: §6 credit, §7 the adapter, §9 the client's route, §10 the keystroke and §11 with its table of constants, updated in each phase's pull request.
- `docs/arch/arch.md` where it describes the companion protocol's features.

## Verification

Automated, in each phase's pull request:

- **Phase 1.** A request on a live route reads no secret and calls no service, including while `ListAccountHosts` fails; a silent route ends after 15 seconds and reconnects, with and without the host's `ping`; a timed-out request fails alone and its late reply is dropped; reconnect delays stay within their jittered bounds; input queued during a stalled write arrives in order, a healthy 200 KiB paste spanning several IPC frames arrives whole and in order; input past 1 MiB and input across a generation change is discarded with the error; an unmatched `result` is never emitted as an overview; `echo_ms` is reported.
- **Phase 2.** Input from one channel is read while another channel's write is stalled; a stalled receiver with trickling small messages is paused before the relay's cap; a 1 MiB message still completes under batched credit against a peer running the old grant rule; an unchanged overview is not pushed; a burst of viewports sends the last one.
- **Phase 3.** Channel vectors for the early form; a replayed first message marks no device connected; fallback to today's form against a host that rejects the payload; resume within the buffer continues with no resync frame; resume past the buffer, after 60 seconds, or for a different device falls back to a snapshot; Revoke, Reset identity and turning remote access off drop lingering streams.
- **Phase 4.** Golden encodings for the envelope; mixed JSON and envelope traffic on one channel; no `result` for a quiet `input`, an error still delivered; pacing holds terminal output and lets pongs through.

Live checks, by the trio's `qa`, on two computers signed in to one account. Each is tied to what the phase changes:

| Phase | Check | Reason |
| --- | --- | --- |
| 1 | `REMOTE-TERM-01`, typing and paste in a remote terminal | The input path and the writer changed |
| 1 | `REMOTE-TERM-05`, drop and restore the client's network while typing | Liveness, the input buffer's discard and the reconnect path changed |
| 1 | New: sleep the client for a minute and wake it; the window shows Reconnecting within about 15 seconds and recovers | The liveness rule is new |
| 1 | New: with the service's directory call failing, switch tabs and run an action in an open remote window; both work | The route no longer depends on the directory |
| 2 | `REMOTE-TERM-02`, resize a remote terminal by dragging | Viewports are coalesced |
| 2 | `REMOTE-WIN-03`, session states in the remote sidebar | Overview pushes are deduplicated |
| 2 | New: flood one pane (`yes`) and type in another; typing stays responsive and `echo_ms` is logged | The writer split and credit changes |
| 3 | `REMOTE-TERM-06`, pair then open at once, and Reconnect | The handshake carries `hello` |
| 3 | New: drop the network for 10 seconds with a terminal scrolled up; on return its contents and scroll position are kept, with no repaint | Resume is new |
| 3 | New: the same against a host on the previous release; the terminal reloads from a snapshot as today | The fallback path |
| 4 | `REMOTE-TERM-01` and `REMOTE-TERM-08`, typing, paste and switching between long chats | The envelope carries input, output and snapshots |

Each phase's test record states `echo_ms` and the ping for the same link before and after the change.

## Implementation phases

One pull request per phase, in order, each by a claude trio since each has live checks. Each phase's branch carries its own brief and test record; this spec is updated on that branch when a phase changes a decision here. Phases 1 and 2 touch the client protocol's transport and the daemon's remote module, so their pull requests carry `core-review` and `risk-medium`; phase 3 changes the handshake and is `risk-high`.
