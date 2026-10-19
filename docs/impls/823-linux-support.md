# Linux support implementation plan

1. Inventory app cfg/dependencies, daemon /proc and login-shell seams; record spec and
   selected validation before checks. Preserve current macOS/Windows branches.
2. Implement /proc descendants and exact rollout descriptors with tests, account shell
   fallback, Linux GPUI dependencies/fonts/platform selection, Ctrl/window/open behavior.
   Freeze an early source snapshot for coordinator ARM64 compilation.
3. Stage one package tree and produce ARM64/x86_64 tar/deb/rpm, preserving foreign links
   and data. Add opt-in Restart=no user service and reconcile packaged CLI ownership.
4. Add both Linux architectures to CI/release/nightly definitions without changing
   existing feed semantics, document prerequisites/WSL/known gaps and guide changes.
5. Run relevant narrow tests, package fixtures and macOS cfg checks with runtime shaders.
   Coordinator owns Linux synchronization, compilation, package/native checks.
6. Hand entire dirty diff to reviewer; fix must-fix findings to explicit clean verdict.
   Final coordinator Linux workspace Clippy/tests and retained native evidence.

During the Build squad implementation, impl was the sole production-code/docs
writer and work remained uncommitted. The user later authorized the local feature
commit and, on 2026-10-20, its rebase/review, push and a draft PR in the user's
fork for CI. No upstream PR or merge/release is authorized.

2026-10-19 resume: integrate the coordinator's rebase onto upstream 4a5e5ecf before
retrying earlier Linux failures. Resolve explicit conflicts and audit automatic
merges for i18n, file drops, multi-target crew messages and remote-nightly channel
isolation. Validate the integrated candidate with focused Mac compilation and
workflow/helper checks, obtain independent review and freeze a new manifest/archive.
Only then sync Ubuntu and diagnose/retry the four prior failures with private Node
on PATH. Earlier binaries and test results keep their original candidate identity.


## Current local status, 2026-10-20

Fork draft PR <https://github.com/ruizer/runner/pull/1> is authorized; no upstream
PR, merge or release is authorized. The latest completed hosted run for
`6ced77ff` passed macOS, Windows and both Linux architectures. PR CI runs checks
and tests; release/nightly alone build installation packages.

The user confirmed Chinese input, zoom, simultaneous OpenCode two-role startup,
package/menu launch and, after reinstalling the corrected package, the dock
icon/name. These are user-reported native passes, with original candidate
identities retained in `docs/tests/823-linux-support.md`.

The branch is now rebased without conflicts onto upstream `8e523b5e` (0.16.7),
including mission-feed virtualization and filtered-inbox feedback. The feature
patch is unchanged by the rebase. Scoped Mac/Linux app tests, workspace Clippy, CLI roundtrip/golden and spawn
prompt checks passed, as did Mac updater Clippy, CLI reference checks and
formatting. The new commit's hosted CI remains separate from the previous run. WSL/WSLg,
x86_64 native GUI, X11 and native RPM installation remain unverified; Linux
in-app updates are not implemented. The user's requested updater assessment is
separate from the Linux foundation delivered by this branch.

### Earlier 0.16.6 integration

The committed Linux feature was rebased onto upstream `a9d3b290` (0.16.6) without
text conflicts. Review preserved the new Active sidebar and the upstream design
repository migration. One upstream UI test needed Linux adaptation: simulated
sidebar selection now uses `keymap::platform_default("cmd-1")`, preserving the
same attention-target assertions on every platform. No runtime or CI behavior
changed in this follow-up. Mac/Linux application tests and strict workspace Clippy,
Mac updater Clippy and formatting passed; exact commands and the original failed
attempts are in the dated rebase section of the validation record.

Snapshot14 binds the tested source before this documentation update. The 0.16.5
full-workspace results and local installation packages below remain historical;
no 0.16.6 packages, fresh native GUI pass or hosted CI run is claimed. The earlier
handoff is retained below, and the PR draft includes the newer scoped results.

## Local delivery handoff, 2026-10-19

Implementation remains uncommitted on `feat/823-linux-support`, HEAD
`1fff68095aa665c1981aabc3b33c0f56559a31fc`, based on upstream `4a5e5ecf` (0.16.5).
The local brief remains the authorization: no commit, push, PR, merge or release.

Delivered source includes Linux GPUI X11/Wayland selection and font fallbacks,
shared Windows/Linux chrome with Linux native resizing and initial bounds,
Ctrl bindings, platform open/notification behavior and Linux updater gating.
Daemon/CLI changes cover `/proc` process identity/descendants, exact Codex rollout
capture, account-shell fallback and fresh headless CLI/daemon installation.
Packaging produces architecture-labelled tar/deb/rpm from one staging tree with a
same-byte real `runnerd`, ownership-aware install/upgrade/remove behavior and an
opt-in systemd user service. CI/release/nightly definitions cover ARM64/x86_64
Ubuntu 22.04 builds and preserve separate nightly channels. Relevant guides,
architecture docs and bilingual README/catalog changes accompany the code.

The spec is `docs/features/823-linux-support.md`; development/packaging contracts
are in `docs/arch/linux.md`. Exact commands, source/package hashes, original
failures and remaining native limits are in `docs/tests/823-linux-support.md`.
Local artifacts under `target/` are not public downloads.

| Evidence | Final local delivery status |
| --- | --- |
| Source11 automation | Linux and Mac workspace tests exited 0 with no FAILED result; existing ignored cases remain ignored. Both platforms passed fmt, strict workspace Clippy, CLI build/reference checks; Mac updater Clippy also passed. |
| Theme-test repair | Real-Root barrier regression failed before explicit test-scope isolation and passed after. Final Linux/Mac concurrent workspace runs passed this and the original mission theme test. Original source7 failures remain retained. |
| Source11 packages | Rebuilt tar/deb/rpm, with isolated tar/deb lifecycle and RPM metadata/layout passing. First lifecycle attempt failed because the coordinator PATH omitted sbin; unchanged-package retry passed after PATH correction. Local build requires glibc 2.43 on Ubuntu 26 ARM64. |
| Native/manual checks | User-reported window resize/restore, foreground interrupt/recovery and clipboard passes. Automated native control was blocked, screenshots/remaining native input/IME unverified, zoom unconfirmed. No new source11 GUI smoke is implied. |
| OpenCode on source10 | Initial concurrent startup failed with SQLite locking; resume enabled bounded real tools, PING/PONG/ACK, inbox reads and idle/completed. Two short independent CLI controls did not reproduce; cause remains unattributed and no fix is claimed. |

The frozen source is `target/linux-checkpoint-11.tar.gz`, SHA256
`393078239b6b98a3f9d9ba609e8cc9e2891648a96cdbdd90e50df6358a147b2b`,
with `target/linux-checkpoint-11.json`, 1557 source entries and tested dirty SHA256
`dd55df48b51107d65dfe7a4b87fccf642d7106d153129c3aae0901dabfc42ee9`.
Architect and reviewer independently verified the full snapshot; final complete
source review found NO REMAINING MUST-FIX ISSUES. The coordinator's final gates
and rebuilt packages use this identity. The final authorized documentation update
postdates the archived docs only; production/test bytes and archive remain frozen,
and there is no replacement snapshot.

Final local packages are in `target/linux-packages/final11-20261019/`, with
`package-identity.json`/`SHA256SUMS`; authoritative metadata and raw Linux evidence
are in `target/linux-validation/final11-evidence/`. Release Runner SHA256 is
`c2e6301d497bf31d54c7fa2eb94761b55728ce7dd991ea3c9c2ba49ac57ac1d4`;
release CLI and real runnerd both use
`2ab530ba3c0ccec8bb5f7d962fced667391520ac24607abb97213c561ca52a07`.
Old source9 packages omit the window repair and remain historical artifacts.

Ubuntu 22/24 compatibility, x86_64 execution, Windows regression execution, hosted
CI, WSL and native RPM installation remain unverified. The earlier native systemd
fixture keeps its source4 identity. Local automation and the bounded user reports
do not establish complete platform acceptance or release readiness. Impl stops
writing after this docs handoff; the coordinator owns mission stop, while the
branch/worktree is preserved.

## Proposed PR title and body (local draft; not submitted)

**Title:** feat(linux): add native desktop support and installable packages

**Body:**

Rebased onto upstream `a9d3b290` (Runner 0.16.6). The latest review preserved the
Active sidebar and adapted its new keyboard test to the platform's primary
modifier. Snapshot14 archive SHA256 is
`cc387c2eecfb0b406ed1d34cde5b1def7f2791e617a39ce91e25145c8c9d4c12`.
The following scoped checks passed after that rebase:

| Platform | Command | Exit |
| --- | --- | --- |
| Mac | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | 0 |
| Mac | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | 0 |
| Mac | `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | 0 |
| Linux ARM64 | `cargo test --locked -p runner-app --profile ci` | 0 |
| Linux ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| Mac | `cargo fmt --all --check` | 0 |

The initial Linux-only primary-modifier failure and an intermediate incorrect
display-label helper attempt are retained in the validation record; the final
test uses the existing executable key mapping. These are application tests and
workspace lint checks, not a new full-workspace or native smoke run. The broader
0.16.5 results and packages below predate this rebase; 0.16.6 packages were not
rebuilt. Hosted CI and the previously documented native/platform gaps remain.

Runner can run its native app, daemon and CLI on Linux, including a fresh headless
installation. Add X11/Wayland integration, shared desktop controls, Linux native
resizing and initial bounds, font fallbacks and Ctrl shortcuts. Linux hides the
unsupported app updater while retaining the macOS/Windows implementations.

Add `/proc` process/rollout discovery and account-shell lookup, plus consistent
ARM64/x86_64 tar/deb/rpm staging with an ownership-aware installer and an opt-in
systemd user service. CI/release/nightly definitions add dual-architecture Linux
builds against Ubuntu 22.04 and preserve channel isolation. Guides and bilingual
platform copy cover installation, service behavior and platform limits.

Final source11 Linux ARM64 and Mac workspace tests exited 0 with no FAILED result;
existing ignored cases remain ignored. Both passed formatting, strict workspace
Clippy, CLI build and CLI reference checks; Mac updater Clippy passed.
A deterministic real-Root regression reproduces the previous shared theme-test
race; explicit test-scope isolation now passes in both concurrent workspace runs.

New source11 tar/deb/rpm artifacts were built and verified locally. Isolated tar/deb
headless installation/reinstall/removal and RPM metadata/layout checks passed on
retry after correcting the coordinator's PATH; the original environment failure
is retained. The local Ubuntu 26 ARM64 packages require glibc 2.43. Ubuntu 22/24,
x86_64, Windows, hosted CI, WSL and native RPM installation remain unverified;
CI definitions alone are not an executed compatibility pass.

Bounded OpenCode live testing retained an initial concurrent-start SQLite failure.
After resume, real tools, directed messages, inbox reads and completed outcomes
passed. Two short independent CLI comparisons did not reproduce the failure;
its origin remains unattributed and no runtime fix is claimed. Window
resize/restore, interruption/recovery and clipboard passes are user-reported;
zoom and the remaining native input/IME checks are unconfirmed. Detailed commands,
artifact identities, original failures and evidence limits are recorded in
`docs/tests/823-linux-support.md`.

The following commands ran against frozen source11 archive SHA256
`393078239b6b98a3f9d9ba609e8cc9e2891648a96cdbdd90e50df6358a147b2b`.
Linux rows use the common prefix
`env LC_ALL=C.UTF-8 LANG=C.UTF-8 PATH=/home/lr/runner/.worktrees/feat-823-linux-support/target/linux-validation/tools/node-v22.14.0-linux-arm64/bin:/home/lr/.cargo/bin:/usr/bin:/bin`.
Only the lifecycle retry substitutes the corrected prefix
`env LC_ALL=C.UTF-8 LANG=C.UTF-8 PATH=/home/lr/runner/.worktrees/feat-823-linux-support/target/linux-validation/tools/node-v22.14.0-linux-arm64/bin:/home/lr/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin`.

| Platform | Command (after the Linux prefix where applicable) | Exit |
| --- | --- | --- |
| Mac | `cargo fmt --all --check` | 0 |
| Mac | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | 0 |
| Mac | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | 0 |
| Mac | `cargo test --locked --workspace --profile ci --features gpui_platform/runtime_shaders --no-fail-fast` | 0 |
| Mac | `cargo build --locked -p runner-cli --profile ci` | 0 |
| Mac | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 |
| Linux ARM64 | `cargo fmt --all --check` | 0 |
| Linux ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| Linux ARM64 | `cargo test --locked --workspace --profile ci --no-fail-fast` | 0 |
| Linux ARM64 | `cargo build --locked -p runner-cli --profile ci` | 0 |
| Linux ARM64 | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 |
| Linux ARM64 | `python3 script/bundle-linux --channel production --arch aarch64 --output-dir target/linux-packages/final11-20261019` | 0 |
| Linux ARM64, initial PATH | `python3 script/linux/test-packaging.py --packages target/linux-packages/final11-20261019` | 1 |
| Linux ARM64, corrected PATH | `python3 script/linux/test-packaging.py --packages target/linux-packages/final11-20261019` | 0 |

The initial lifecycle script exited 1 because dpkg could not find `ldconfig` and
`start-stop-daemon` in the restricted PATH (dpkg itself exited 2). The corrected-PATH
retry exited 0 against unchanged source and package bytes; the first failure is
retained. Tests contain nested child-process summaries, so their sums are not
reported as unique test totals. Existing ignored tests remain ignored.

Related: #823.
