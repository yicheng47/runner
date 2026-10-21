# Linux updater validation

## Scope selected before tests

- Linux release selection: architecture/format, semantic stable versions,
  independent nightly stamps, missing signatures and stale cleanup.
- Download: valid/tampered signature, cancellation and partial cleanup. Reuse
  existing Windows downloader tests on Windows; add Linux transport fixtures.
- Installer boundary: refuse foreign/unmanaged installs, links/special files,
  traversal/oversized archives, wrong identity/ELF and mismatched sidecars.
  Verify atomic replacement, failed extraction preserving old bytes and user data.
- deb/rpm: package metadata and manager command/authorization outcome handling;
  no system installation over the user's real VM installation during fixtures.
- UI/lifecycle: adapt SET-UPDATE-02/03/04 from regression/settings.md to Linux:
  entry points, cancellation/retry, working-session caption and restart semantics.
  Native authorization, real version-to-version relaunch and RPM distro checks
  require disposable installed packages and are recorded separately from unit tests.
- Cross-platform gates: runner-app tests plus strict workspace Clippy on Mac and
  Linux, Mac updater Clippy, formatting. Windows CI is not inferred from Mac/Linux.
- Packaging/publishing: installer fixtures and workflow shell/syntax tests cover
  release identity, signature presence/upload/download/pruning. No release dispatch.

No unrelated IME, agent authentication or full workspace regression is selected.

## Candidate and results, 2026-10-20–21

Implementation remains local and uncommitted on `feat/823-linux-support`, HEAD
`dfb097666680209b7cb11edb0629100f4d999cba`, based on upstream `8e523b5e`.
No PR push, release/nightly dispatch or production signing secret was used.

The complete application and package checks use snapshot25:1539 verified source
files, archive SHA256
`9f6e4c800896208afca4929a56fbf54c19604a7de8db82d2c7ed988e96c2f970`,
source identity `922b1e936c8f51ee8dc29de17ec1c6e4abcda8ca3f1d68025d1006f8f6717a76`.
Snapshot26 adds an ignored actual-package extraction test, explicitly executed
against the built tar. Snapshot27 adds negative archive identity/ELF/path/size
cases, with production installer source unchanged from snapshot25. Its archive
SHA256 is `3510cc6d9f974f9a839f775ab0f270ba682c6dd2b0f3a0ce478d13246563edb0`,
source identity `649b84a2d1a056b18daf27125512927e0fa16f7c13b9f198d6e4bcc2777bc448`.
Later edits to this record do not change the tested production source.

| Platform | Exact command | Result |
| --- | --- | --- |
| Mac | `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | exit0; app tests passed,3 pre-existing ignored cases |
| Mac | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | exit0 |
| Mac | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | exit0 |
| Ubuntu26 ARM64 | `cargo test --locked -p runner-app --profile ci` | exit0 /68.97s |
| Ubuntu26 ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | snapshot25 exit0 /21.77s; snapshot26 exit0 /20.44s |
| Mac | `python3 script/linux/test-packaging.py` | exit0;17 tests,2 skipped (dpkg/rpm host prerequisites) |
| Ubuntu26 ARM64 | `python3 script/linux/test-packaging.py` | exit0 /0.55s;17 tests,none skipped |
| Mac | `uv run --no-project --with pyyaml python script/test-nightly.py -v` | exit0;18 tests |
| Ubuntu26 ARM64 | `./script/bundle-linux --channel production --output-dir target/linux-packages/linux-updates25` | exit0 /348.43s;tar/deb/rpm built once |
| Ubuntu26 ARM64 | `python3 script/linux/test-packaging.py --packages target/linux-packages/linux-updates25` | exit0 /3.48s |
| Ubuntu26 ARM64 | `RUNNER_TEST_LINUX_PACKAGE=<built-tar> cargo test --locked -p runner-app --lib --profile ci updater::native::linux::tests::actual_packaged_tar_passes_the_updater_extractor -- --ignored --exact` | exit0 /29.66s;one real-package test |
| Ubuntu26 ARM64 | `RUNNER_TEST_LINUX_PACKAGE=<built-tar> cargo test --locked -p runner-app --lib --profile ci updater::native::linux::tests -- --include-ignored` | snapshot27 exit0 /17.84s;12 cases passed,none ignored |
| Ubuntu26 ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | snapshot27 exit0 /14.57s |
| Mac | `bash -n script/linux/sign-updates`; `cargo fmt --all --check`; `git diff --check` | exit0 |

Raw Mac evidence uses `target/linux-validation/linux-updates-final-mac-`,
`linux-updates-nightly-signatures.log` and
`linux-updates-package-signing-fixtures.log`. Ubuntu raw logs/result JSON use
`target/linux-validation/linux-updates-25-`, `-26-` and `-27-`; result copies
and package identity are kept locally in `linux-updates-linux-evidence/`.
The publishing and signer-wrapper fixtures stub GitHub/HTTP and npx signing;
they verify required artifacts, trust-key matching, failure gates, channel
isolation and paired package/signature retention, not an actual release signing.
Mac retains the pre-existing block0.1.6 future-compatibility warning.

## Real package identity and installer entry

VM packages are under `target/linux-packages/linux-updates25/`:

| Artifact | SHA256 |
| --- | --- |
| Runner-0.16.7-linux-aarch64.tar.gz | `3352e2084e3d5fb810a3b94dfa2ab1512100de2bc7de5fe514a5c2a3f52e6557` |
| Runner-0.16.7-linux-aarch64.deb | `c88a14c3c3ea3a8c62b0028939b3cee7ec42c0afa046655847c4554668d62763` |
| Runner-0.16.7-linux-aarch64.rpm | `e28f50936c8e3ea144f6fad07a2e332d6eb93937a7750e6b9c53f750e723beba` |

`release-info.json` identifies production0.16.7, aarch64 and stamp20261020.0926.
Actual Runner payload SHA256 is
`c55a0aaeaa387bfb6ee475d3ff3019fc1529f4ae83e32f37ea91108aaf7428c6`;
CLI/runnerd are identical, SHA256
`943e488c8d90075fdd548a9da07eff960c3402a4ee0047f24b2e4b5a6736d620`.
These are local Ubuntu26 artifacts, not Ubuntu22 compatibility evidence or
publicly signed releases; VM Git HEAD is not the synchronized source identity.

Running the extracted real Runner with `--runner-install-update` as an ordinary
user exits1 with `System update installation requires authorization`. An isolated
HOME/XDG root remains absent: this entry does not initialize UI/user data before
checking authorization. The built tar also passes the actual Rust updater
extractor, including identity, ELF and same-byte sidecar checks. Metadata/layout
and isolated tar/deb lifecycle passed; RPM metadata/layout is not a native RPM
installation. No existing user Runner installation or session was operated.

## Original failures and corrections

- Initial Mac dependency resolution failed inside the network sandbox; the
  unchanged command succeeded with network access and locked tar0.4.46/xattr1.6.1.
- Initial uv execution exited2 because the sandbox blocked its cache; the
  unchanged nightly test command passed outside that sandbox.
- Snapshot19 Linux check exited101: ambiguous File Read/Write `by_ref`; made the
  Read operation explicit and removed an unused import.
- Snapshot20 app build exited101: Windows-only auto-download setting and working
  count were not compiled on Linux. Snapshot21 additionally exposed the count's
  Windows-only mission cache. All related cfgs and test constructors now include
  Linux; no agent adapter, daemon, CLI or client-protocol behavior changed.
- Snapshot23 app tests exited101:581 passed and one older assertion still required
  Linux to hide the update icon. The assertion now checks the newly supported
  icon's presence/layout. Snapshot25 application tests passed.

These original logs/results remain retained separately from successful retries.

## Native verification still required

Selected automated checks are separate from live acceptance. The following have
not been exercised: a desktop polkit authorization prompt and cancellation, a
successful signed tar/deb/rpm version-to-version install/relaunch with live test
sessions, and the native Linux update UI before/after screenshots. They require
disposable managed installations and signed newer packages; the local packages
above are unsigned, and no production signing key was accessed. Existing agents,
credentials, installed Runner and user sessions must remain untouched.

Windows execution/CI, native x86_64 desktop, an RPM-based distro's real DNF
transaction and WSL/WSLg are also unverified. Unit tests validate deb metadata and
APT command construction without running a system transaction. The Linux UI
reuses existing desktop dialog/working-count tests; virtual GPUI bounds are not
native GUI evidence. No full regression or complete live-update pass is claimed.


## Independent platform modules, 2026-10-21

The results above record the earlier shared implementation. Linux now owns its
backend (`updater/linux.rs`), installer (`updater/linux/install.rs`), update dialog
and settings pane. Windows updater, dialog, settings pane and desktop update
helpers are restored byte-for-byte from HEAD. Public entry points select by cfg.

Selected checks before this follow-up validation:

- Linux updater fixtures cover moved release/transport/installer code and fixture
  paths. Reuse the earlier tar only as extraction input, not as a new binary.
- runner-app tests on Linux cover platform module wiring, update dialog actions,
  working-session captions and settings availability. Mac runner-app tests and
  workspace Clippy cover shared entry points; updater Clippy covers Sparkle cfg.
- Formatting and diff checks plus Windows file equality establish the intended
  split. No package rebuild, system upgrade, unrelated runtime smoke, release
  dispatch, commit or push is selected for this module separation.

Snapshot28 contains1542 verified source files, archive SHA256
`0d8c0ddad717f86b0abf3236387f699b09b8216862c5b3462b6db2508a2a9d48`,
source identity `fdd69ec1ce70a7a2e5b1a9db3d17365853532f1bb58a79a5a9532fc3cacbd7c9`.
The final change to this record only adds these results.

| Platform | Check | Result |
| --- | --- | --- |
| macOS | runner-app tests, runtime shaders | exit0 /28.88s |
| macOS | strict workspace Clippy, runtime shaders | exit0 /6.81s |
| macOS | strict runner-app updater Clippy, runtime shaders | exit0 /9.19s |
| Ubuntu26 ARM64 | runner-app tests, unchanged retry | exit0 /68.21s |
| Ubuntu26 ARM64 | strict workspace Clippy | exit0 /30.98s |
| Ubuntu26 ARM64 | `updater::linux::install::tests -- --include-ignored`, earlier tar as fixture | exit0 /1.45s;12 cases passed |
| Source | Windows updater/dialog/settings/desktop helpers equal HEAD bytes | 4/4 |
| Source | `cargo fmt --all --check`, `git diff --check` | exit0 |

The first Ubuntu app check exited101 after69.95s because the VM filesystem ran
out of space during linking. Its result/log is retained under
`target/linux-validation/linux-updates-28-app*`. Removing this worktree's
regenerable `target/ci/incremental` cache (24GB) allowed the identical-source retry
to pass. No source fix or scope change was needed.

Mac logs/results use `target/linux-validation/linux-updates-28-mac-*`;
Ubuntu logs/results use `target/linux-validation/linux-updates-28-*`, with result
copies under the Mac worktree's `linux-updates-linux-evidence` directory. No new
release package was built. Native authorization/relaunch and Windows CI gaps
above remain.


The user subsequently requested a local commit. The implementation is unchanged
from snapshot28; only this validation record and the feature authorization note
have changed since that candidate. No push or release dispatch is requested.
