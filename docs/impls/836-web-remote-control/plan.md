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
4. **Settings**, in `runner-app/src/surfaces/settings/remote.rs`, built from the signed-off frames in `design/specs/836-web-remote-control.pen`: the PIN row with Set and Change, the locked row with Unlock, the device list with Revoke, and Reset identity behind the existing confirmation dialog. A warning toast when PIN entry locks while the app runs. English and Simplified Chinese strings. `docs/guide/guides/remote-access.md` gains Who can reach this computer.

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

Planned in full once the spec's protocol section exists. Already fixed:

- Its own module, `runner-core/src/companion/`, apart from the build-locked client protocol in `protocol/`, because clients update independently of the host.
- A golden JSON file for every message, handshake and error under the module's `fixtures/`, parsed and produced by the types, including missing default fields and unknown fields. The service's browser client decodes the same files.
- Types and fixtures only: serving them from `runnerd`'s state is Phase 3's adapter.

It lands like m2, on `feat/836-m3-companion-protocol` with its brief as `836-m3-companion-protocol.md`.

### Spec additions before m2

Planning m2 surfaced details the spec did not state. Jason approved them with the plan on 2026-10-19, and they went into the spec with m2's brief:

- Reset identity also clears the PIN: the two share a secret-store item, so a leak of one is a leak of both.
- `runner remote reset-identity`, for a host without the app, behind the same guard as `runner remote pin`.
- The trust requests and the `remote/trust` event, `runner remote devices` and `revoke`, and the new `status` lines.
- The PIN is exactly six ASCII digits, stored as entered.
- `runner remote revoke` names a device by its fingerprint, as `runner remote devices` prints it.

## Phase 3 — the end-to-end channel and live terminals

Planned when its development starts, after Phase 2's spec sections. It carries the PIN exchange and both Noise handshakes, the relay's client channel, the adapter that serves the companion protocol from `runnerd`'s state, terminals, the new-device notification, and the multi-client resize fix [#838](https://github.com/yicheng47/runner/issues/838).

## Phase 4 — Tailscale

Planned when its development starts. Its open questions are on [#954](https://github.com/yicheng47/runner/issues/954); the pairing and trust model are Phase 2's, unchanged.
