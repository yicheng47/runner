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

Planned when its development starts, after Phase 2's spec sections. It carries the PIN exchange and both Noise handshakes, the relay's client channel, the adapter that serves the companion protocol from `runnerd`'s state, terminals with the spec's terminal-size controller, the desktop pane's control card and `terminal_take_back`, and the new-device notification. [#838](https://github.com/yicheng47/runner/issues/838) is not part of it, since the controller treats every desktop window as one. Jason could not reproduce it on 2026-10-19, so Phase 3 reruns its two-window steps when it changes the pane's size push, without blocking on it.

## Phase 4 — Tailscale

Planned when its development starts. Its open questions are on [#954](https://github.com/yicheng47/runner/issues/954); the pairing and trust model are Phase 2's, unchanged.
