# #982: Linux integration with Runner 0.18.0

This integration rebases the contributor's Linux desktop, packaging and updater work onto Runner 0.18.0. It preserves current remote and Windows/macOS behavior, resolves platform conflicts, fixes Linux notifications/remote shortcuts, hardens file-backed credentials and ports Linux publishing onto the single-nightly workflow. Remote control uses Runner clients; browsers handle enrollment/accounts. macOS and Ubuntu 22.04 ARM64 automated checks pass; fresh native Linux acceptance remains outstanding.

## Changes

File names include context where useful; app source paths omit `crates/runner-app/src/`. Each row records one finding.

| Finding | Change | Files |
| --- | --- | --- |
| A1: Rebase conflicts | Combined current remote/window/clipboard behavior, dependencies, 0.18.0 versions and locale catalogs with Linux support. | `Cargo.lock`, `crates/runner-app/Cargo.toml`, `crates/runner-app/i18n/*.toml`, `main.rs`, `surfaces/app_shell.rs`, `surfaces/chat.rs`, `surfaces/mission_workspace/{input,rail}.rs`, `surfaces/settings_page.rs`; workflow/docs files below |
| A2: Pairing build | Linux attempts desktop delivery and always shows an actionable toast; permission remains Unknown. Added a Linux regression. | `notifications.rs`, `notifications/tests.rs` |
| A3: Store Clippy | Compile the namespace field only where used; Linux isolation remains based on the data root. | `crates/runner-daemon/src/remote/store.rs` |
| A4: Nightly fixtures | Ported Linux fixtures to current setup, covering selection, signatures, failures and retention. | `script/test-nightly.py` |
| B1: PIN shortcuts | Use Control on Linux; test paste, blocked export/edit shortcuts, masking and disabled input. | `surfaces/settings/remote/pin.rs` |
| B2: Remote copy | Propagate no-selection Copy so Ctrl+C reaches Linux terminals; test copy/interrupt/paste and retain disconnected/read-only cases. | `surfaces/remote_client/workspace.rs`, `surfaces/remote_client/tests.rs` |
| B3: Credential storage | Retained 0600 files; reject unsafe/symlink roots, create missing roots as 0700 and test permissions without rewriting existing modes. Document plaintext storage. | `crates/runner-daemon/src/remote/store.rs`, `docs/guide/guides/remote-access.md`, `docs/arch/linux.md` |
| C1: Identity/metadata | Retained key-based identity/Linux OS reporting; added ARM64 normalization coverage. | `crates/runner-core/src/remote/identity.rs`, `crates/runner-daemon/src/remote/store.rs` |
| C2: Service roots | Test HOME/XDG and release/dev identity isolation; document service environments, startup and daemon-replacement reconnect. | `crates/runner-core/src/app_paths.rs`, `crates/runner-daemon/src/remote/store.rs`, `docs/arch/linux.md`, `docs/guide/guides/remote-access.md` |
| C3: Daemon/CLI | Retained Linux IPC, process discovery, shell resolution and remote commands. | `protocol/socket.rs`, `session/process/unix.rs`, `shell_path.rs`, `runner-cli/src/remote.rs` |
| C4: Client integration | Preserved Linux chrome/fonts/Control and shared remote-window/link/clipboard APIs; retained macOS Option and Windows singleton gates. | `platform_ui/`, `keymap.rs`, `surfaces/app_shell.rs`, `docs/guide/reference/keyboard-shortcuts.md` |
| C5: Browser contract | Documented Runner apps as remote clients and browsers as enrollment/account surfaces; no browser terminal client was added. | `docs/arch/linux.md`, `docs/guide/guides/remote-access.md` |
| D1: Publishing | Default to all; retain both and add Linux-only selection. Verify/retain both Linux architectures and signatures. Nightly skips main CI; production keeps its CI gate/draft handoff. | `.github/workflows/nightly.yml`, `.github/workflows/release.yml`, `.agents/skills/nightly/SKILL.md` |
| D2: Retired channel | Reject remote-nightly in new Linux bundling/updater paths; correct signed-update/release instructions in both language templates. | `script/bundle-linux`, `script/linux/test-packaging.py`, `updater/linux.rs`, `.agents/skills/release/SKILL.md`, `docs/arch/arch.md`, `docs/guide/getting-started/install.md` |
| D3: Documentation | Updated Linux guides; preserved current remote content and historical validation. | `docs/arch/linux.md`, `docs/guide/guides/{remote-access,troubleshooting}.md`, `docs/features/823-linux-support.md`, `docs/tests/823-linux-support.md`, both READMEs |
| D4: Linux coverage | Added notification, PIN, clipboard, root-isolation and identity fixtures; retained platform-neutral remote tests and both architecture CI jobs. | Test files above, `.github/workflows/ci.yaml` |
| D5: Tar icon ownership | Use an owned icon link that follows app swaps; migrate matching legacy copies and preserve foreign assets. Test changed-icon replacement/reinstall/uninstall. | `script/linux/install.sh`, `script/linux/test-packaging.py`, `docs/arch/linux.md` |

## Approved decisions

Jason approved all five recommendations on 2026-10-25.

1. Use private-file credentials for the first Linux release, with permission/isolation tests and documentation. This supports headless/SSH/linger operation but stores plaintext rather than using a desktop vault.
2. Carry compatibility fixes in #982. This slightly increases the contributor diff while keeping build-breaking integration together; no remote protocol rewrite is required.
3. Default nightly dispatch to `all`, retain `both` as macOS+Windows and publish only to `nightly`. Default cuts add two Linux jobs and six signed packages; explicit `both` callers keep their scope.
4. Keep the Runner-to-Runner remote model. Browser terminal control requires a separate product/security design.
5. Require both Linux CI jobs when accepting Linux support and obtain native remote acceptance on ARM64 and x86_64 before claiming full Linux support. Hosted checks establish portability, with native desktop/service acceptance outstanding.

Jason also authorized the two UUID-only macOS Keychain fixtures used by `make verify`; both passed. Other Keychain data and real Runner credentials remain outside scope.

## Follow-ups outside #982

- Maintainers must configure required Linux CI checks in repository settings.
- Obtain fresh Linux X11/Wayland input, chrome, browser-link, notification, systemd/SSH/linger, package/update and remote host/client acceptance, including cross-platform clients and x86_64.
- Consider optional Secret Service storage with migration and locked/unavailable-service behavior while preserving headless identity continuity.
- Consider Linux second-launch routing separately from remote compatibility.

## Verification

| Check | Platform | Result |
| --- | --- | --- |
| `make verify` | macOS ARM64 | Passed with no exclusions, including both Keychain fixtures, workspace check/tests, updater-feature Clippy and formatting. Existing opt-in tests remain ignored. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Ubuntu 22.04 ARM64 | Passed. |
| `cargo test --locked --workspace --profile ci --no-fail-fast` | Ubuntu 22.04 ARM64 | Passed, including remote tests and new Linux regressions. |
| `script/test-nightly.py -v` | macOS ARM64 | 14/14 passed with stubbed publishing/download operations. |
| `script/linux/test-packaging.py` | Ubuntu 22.04 ARM64 | 19/19 passed. |
| `script/linux/test-packaging.py`; `sh -n script/linux/install.sh` | macOS ARM64 | Packaging: 17 passed, 2 Linux-tool prerequisite skips; shell syntax passed. |

Not verified:

- Windows or x86_64 Linux builds/execution locally.
- Fresh Linux desktop, systemd/SSH/linger, package installation or in-app update acceptance; earlier contributor results retain their dated candidate scope.
- Real remote enrollment, pairing, host/client operation or hosted-service behavior on this candidate.
- Workflow dispatch, signed production publication, public downloads or branch-protection configuration.
- Native macOS app behavior; reviewer deemed live checks unnecessary for preserved paths and in-process-tested storage.
