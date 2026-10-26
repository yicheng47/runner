# #823 — Linux remote sign-in never starts

Jason requested a codex duo mission (one coder, one reviewer) on 2026-10-26. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-823-linux-remote-sign-in` on `fix/823-linux-remote-sign-in`, branched from the Linux umbrella `feat/823-linux-support` at `f2bc95662e608bbce052e23736ecf7b3c22a20a8` ([#1027](https://github.com/yicheng47/runner/pull/1027)). The root checkout stays on main. Open the PR against `feat/823-linux-support`, not main.

## Problem

On Jason's Ubuntu 26 ARM64 UTM VM, `linux-nightly` build `f2bc956` installed from the deb: Settings → Remote access → **Sign in to Runner** stays disabled with **Cancel** visible, no browser opens, and pressing Cancel does not bring the button back. `xdg-open https://runner.yeelinks.com.cn/` works on that machine, and the server answers. `runnerd.log` shows only `ERROR runner_daemon::remote: remote task ended unexpectedly; restarting independently of sessions`, three times in two minutes, with no reason.

Likely root cause, from reading the code (Jason has not yet collected `stat` output): on Linux, `OsStore` uses `FileStore` rooted at the app data directory (`~/.local/share/com.wycstudios.runner`). `FileStore::validate_root` refuses a root with any group or other write bit (`mode & 0o022`). Ubuntu's default umask is `002`, so `create_dir_all` in daemon boot leaves that directory `0775`. `store::restore` → `Identity::load` then fails inside `run`'s preparation, before the command loop ever reads the inbox. The supervisor in `spawn_with_clock` restarts it with backoff and logs neither the `&'static str` error nor a `JoinError`. Queued `Envelope`s (Start, then Cancel's logout) are never answered, so the client waits until `REMOTE_REQUEST_TIMEOUT` (45 s), and the daemon keeps the stale envelopes queued. macOS release builds use the Keychain and Windows uses Credential Manager, which is why only Linux (and macOS debug builds, which use `FileStore` too) can hit it.

Confirm the cause with a test before fixing it. Rule out `transport::client()` failing on Linux (`rustls-tls-native-roots` with no usable system roots) as an alternative explanation.

## Read first

- `crates/runner-daemon/src/remote/store.rs`: `OsStore::new`/`named`, `FileStore` (`validate_root`, `get`, `put`), `restore`.
- `crates/runner-daemon/src/remote/mod.rs`: `spawn_with_clock` (supervisor), `run` (preparation, command loop, `pending_start`), `Inbox`.
- `crates/runner-daemon/src/daemon/boot.rs` (`create_dir_all` of the app data dir) and `server.rs` (`remote::spawn` root).
- `crates/runner-app/src/surfaces/settings/remote.rs` (`remote_request`, `busy`/`opening`) and `remote/account.rs` (`cancel_enrollment_start`).
- `docs/arch/linux.md`, the remote identity and secrets paragraph.

## Deliverables

1. **Sign-in works with a default Ubuntu umask.** A user-owned Runner data directory that is group-writable because of the umask must not stop remote access. Choose and justify the approach, for example creating and repairing Runner's own secrets location without group/other write, or a dedicated `0700` secrets directory with migration of existing files. Keep the protection the check exists for: never use a directory or file another user can write or replace. Existing Linux installs and macOS debug identities keep working without a forced re-sign-in.
2. **The real error is logged.** When the remote task ends unexpectedly, the log line carries the error message or the panic/join failure, redacted the same way other remote messages are.
3. **Queued requests fail fast.** While the remote task cannot start, pending and newly queued commands are answered with the actual error instead of waiting for the 45 s client timeout, and stale envelopes are not executed later. The Remote access card shows that error.
4. **Cancel always restores the button.** Pressing Cancel during **Sign in** makes Sign in usable again immediately, whatever the daemon's state, and abandons the in-flight start.
5. Update `docs/arch/linux.md` if the secrets location or permission behavior changes, and add a short Linux note to `docs/guide/guides/remote-access.md` or `troubleshooting.md` only if users need to act.

Non-goals: the UTM Virtio-GPU Venus no-window hang (worked around with software rendering, out of scope); changes to the server, enrollment protocol, macOS Keychain or Windows store; UI redesign beyond the error and button states above.

## Validation

Before running checks, name each changed behavior and the check that covers it. Expected: `FileStore` unit tests for a user-owned `0775` root (accepted or repaired as designed) and for a root another user could write (still refused); a remote-task test where preparation fails, proving the error is logged, queued requests are answered with it promptly, and nothing stale runs afterwards; an app settings test that Cancel clears `busy` while the daemon request is still pending. `FileStore` is `cfg(unix)`, so these run on macOS too. Run the affected `runner-daemon` and `runner-app` tests with `--locked` and the `ci` profile in this worktree's own target directory, `cargo fmt --all --check`, `git diff --check`, and workspace Clippy with warnings denied. Drive the PR's Linux (x86_64, aarch64), macOS and Windows CI green.

Coders and reviewers do not run the app. The live check stays with Jason on the UTM VM after a later `linux-nightly` cut: with a `0775` data directory, Sign in opens the browser and completes, and Cancel restores the button. List it in the PR.

## Authorization and handoff

Jason authorized this codex duo mission: the coder implements, commits, pushes `fix/823-linux-remote-sign-in` and opens one PR against `feat/823-linux-support`; the reviewer reviews the full diff and the tests before the PR is final. Fold this brief into the implementation commit; split commits only where each stands on its own. Fold review and CI fixes into the commits they correct and push with `git push --force-with-lease`; never plain force. Do not add tool-specific co-author trailers.

Stop at an open PR with green CI and report through Runner with the PR URL, final SHA, root-cause evidence, changes and validation results. Do not merge, cut a nightly or release, archive the mission, delete branches or remove worktrees. When waiting for a teammate or human response, finish the turn and rely on the pushed inbox notification.
