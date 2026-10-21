# Linux support validation

Linux in-app installation updates are a later local follow-up; see
[their separate validation record](823-linux-updates.md). Earlier results below
retain their original scope and candidate identities.

Current integration base: upstream 59c1e162 (Runner 0.17.0), rebased on
2026-10-21; see the dated rebase reviews below for scoped checks. The earlier
0.16.5 integration used upstream 4a5e5ecf and brief HEAD 1fff6809. Checkpoints
1–7 retain their original HEAD769e2e40 / feature base0a65b80d identities and
results. Each snapshot and binary keeps its original validation identity.

## Upstream PR integration, 2026-10-21

PR #982 targets upstream main. After creation GitHub reported conflicts with
upstream #978, which moved macOS traffic lights into platform_ui, removed unused
daemon wake code and dropped cli_install re-exports. The four local commits were
rebased from `200ba72b` onto `59c1e162`. Keep upstream's macOS implementation,
place its non-macOS no-op in the existing desktop module and export it on Linux;
keep Linux foreground CLI setup while calling runner_core directly. No installer,
updater, package or Agent adapter behavior changes in this integration.

Selected before execution: Mac/Linux runner-app tests and strict workspace Clippy
cover platform wiring and daemon boot callers; Mac updater-feature Clippy covers
the Sparkle configuration. Focused daemon cli_install tests cover updated imports
and package-aware command installation. Formatting/diff checks cover resolution.
Earlier native tar/deb/Agent tests retain their identities and are not repeated.
Snapshot31 identifies source HEAD `4efa7050`, archive SHA256
`83fb9b228327ef453e0d0520fa63782956867f4a2fbc847c22c208f1187aa8a8`
and source identity
`8d7ce6288ebbc9d70c07bebe198e094c525c4ef2104c9fe5bebc1554af906386`.
Only this validation document changes after the checks.

| Platform | Check | Exit / seconds |
| --- | --- | --- |
| Mac | runner-app tests, ci/runtime_shaders | 0 /50.29 |
| Mac | strict workspace Clippy, ci/runtime_shaders | 0 /23.36 |
| Mac | updater-feature Clippy, ci/runtime_shaders | 0 /18.03 |
| Mac | runner-daemon cli_install::tests | 0 /26.77; 10 cases |
| Mac | cargo fmt --all --check | 0 /2.40 |
| Ubuntu ARM64 | runner-app tests, ci | 0 /82.62 |
| Ubuntu ARM64 | strict workspace Clippy, ci | 0 /14.03 |
| Ubuntu ARM64 | runner-daemon cli_install::tests | 0 /26.54; 11 cases |

Diff whitespace checks pass. Windows updater/backend/dialog/settings bytes match
upstream; Windows execution still requires new hosted CI. Raw logs use
`target/linux-validation/rebase-0978-`. These are scoped integration checks,
not a repeated native upgrade or full regression pass.

## Rebase onto upstream 0.17.0, 2026-10-21

The three Linux commits were rebased from `8e523b5e` onto `200ba72b`. Upstream includes
Agent settings/Claude account work, restored-mission resume, Active sidebar order
and toolchain-based CI cache keys. Catalog append conflicts preserve both sets
of keys. AppStore conflicts preserve upstream removal of activity timestamps and
Linux's mission-count cfg. Range-diff shows only these integration adjustments;
the packaging permission fix remains patch-identical. Windows updater source and
Linux download/installer implementation are unchanged by this rebase.

Selected checks before execution: runner-app tests and strict workspace Clippy on
Mac/Ubuntu cover merged application/state/settings wiring; Mac updater Clippy
covers Sparkle cfg; packaging and nightly shell fixtures cover merged publishing
workflows. Format/diff and catalog parity are checked. No package rebuild, account
switching, live Agent run or repeat native tar/deb upgrade is selected: those
implementations are unchanged. Earlier CI/native evidence retains its original
candidate identity and is not a pass for this rebased candidate.

Snapshot30 binds source HEAD `0ae317d1`, archive SHA256
`1524e49434fd28025beff15b3c79b1df39886105c83d9b5269f89d6b12a48a11`
and source identity
`e6803ef6fd73a7339f904f4d0e7fcb0bcdc73c79c310d517477751afc99e4e3e`.
Only this documentation changes after the checks.

| Platform | Selected check | Exit / seconds |
| --- | --- | --- |
| Mac | runner-app tests, ci profile with runtime_shaders | 0 /58.35 |
| Mac | strict workspace Clippy, ci profile with runtime_shaders | 0 /41.36 |
| Mac | updater-feature strict Clippy with runtime_shaders | 0 /19.51 |
| Mac | packaging fixtures | 0 /1.25; 18 cases, 2 Linux-tool skips |
| Mac | nightly workflow fixtures | 0 /9.75; 18 cases |
| Mac | cargo fmt --all --check | 0 /2.38 |
| Ubuntu ARM64 | runner-app tests, ci profile | 0 /117.17 |
| Ubuntu ARM64 | strict workspace Clippy, ci profile | 0 /79.73 |
| Ubuntu ARM64 | packaging fixtures | 0 /0.76; 18 cases |

The application tests include catalog key/placeholder parity. Diff whitespace
checks also pass. Initial Mac tests exited101 because the network sandbox could
not resolve crates.io for upstream's new dependency; the unchanged command passed
outside that sandbox. The failed attempt remains separate from the retry logs.
Raw evidence uses `target/linux-validation/rebase-0170-`. These are selected
rebase checks, not a full regression pass. Windows CI has not run on this
candidate. No push is requested for this rebase.

## Rebase onto upstream #964 and #967, 2026-10-20

Upstream/main advanced from `f6ab9ef6` to `8e523b5e`: mission-feed virtualization
and scrollbar handling, plus CLI feedback for filtered reads with unread messages.
The clean feature commit `6ced77ff` was backed up, explicit Git identity verified,
and rebased without conflicts to `9f0373cd`. `git range-diff` reports an identical
feature patch. Overlapping mission input/tests, CLI roundtrips, spawn-prompt
expectations and architecture docs were reviewed; the upstream implementations
and Linux adaptations are retained. CI/release/nightly, installer and dock-fix
source bytes are unchanged by the rebase.

Selected checks: Mac and Linux runner-app tests cover the feed integration and
platform UI; strict workspace Clippy covers shared compilation, with Mac updater
Clippy for its feature gate. CLI roundtrip/golden tests and the generated CLI
reference cover changed inbox/help behavior; the exact spawn golden covers the
changed coordination prompt. Formatting and diff checks cover the final patch.
No fresh real-agent mission, GUI pass or package build is selected: this rebase
changes no adapter or package behavior. The user passes above retain their tested
candidate identities. The new pushed commit needs its own hosted CI result.

Before this rebase, hosted run
<https://github.com/ruizer/runner/actions/runs/39390223566> passed all four jobs
for `6ced77ff`: macOS about8m, Windows11m27s, Linux x86_64 6m14s and ARM64 5m39s.
This is the warm-cache result for the previous feature commit, not a result for
the newly rebased commit.

Snapshot18 contains1535 verified files: archive SHA256
`9354132da64fd4a84f427b4e02a27d326bace8206be48da8842c32e760becd16`,
source identity `3d220abe41f3d961654e9c2cbda46d274770861ca1a22a8f6b85b2794d17c29c`.
The VM's previous1530-file snapshot was unchanged before replacement. Later
edits update documentation only; tested Rust/workflow/package bytes are unchanged.
Raw logs/results use `target/linux-validation/rebase-0967-`; Linux result copies
are under its `rebase-0967-linux-evidence/` directory.

| Platform | Exact command | Exit / seconds |
| --- | --- | --- |
| Linux ARM64 | `cargo test --locked -p runner-app --profile ci` | 0 /118.39 |
| Linux ARM64 | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 /0.24 |
| Linux ARM64 | `cargo test --locked -p runner-cli --profile ci --test roundtrip --test golden` | 0 /91.3 |
| Linux ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 /21.24 |
| Linux ARM64 | `cargo test --locked -p runner-daemon --profile ci spawn_goldens` | 0 /32.62 |
| Mac | `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | 0 /41.18 |
| Mac | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | 0 /17.51 |
| Mac | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 /0.35 |
| Mac | `cargo test --locked -p runner-cli --profile ci --test roundtrip --test golden` | 0 /57.17 |
| Mac | `cargo fmt --all --check` | 0 /2.19 |
| Mac | `cargo test --locked -p runner-daemon --profile ci spawn_goldens` | 0 /22.87 |
| Mac | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | 0 /12.95 |

The initial Mac CLI run was interrupted after repeated isolated-daemon startup
failures (Cargo exit101 /271.83s). A single-test diagnostic also exited101 and
reported `failed to bind .../runnerd.sock: Operation not permitted (os error 1)`.
The unchanged CLI tests passed outside the filesystem/socket sandbox; the
successful retry and initial failure keep separate logs. This is an execution
restriction, not a product code correction. CLI runs each passed88 golden and25
roundtrip cases. Each exact daemon spawn check ran one test. `git diff --check`
also passed. These are selected rebase checks, not a full regression pass.

## Human validation and Linux dock identity, 2026-10-20

After syncing snapshot16 (source commit `825d684e`, all1530 files verified), the
user reported passing Chinese input, application zoom, simultaneous OpenCode
two-role startup, and local-package installation/application-menu launch. These
are human-reported passes, not independent native automation; the OpenCode
startup result does not assign or resolve the historical SQLite-lock cause.

The same report includes a GNOME dock screenshot with a generic gear and an
unrecognized application name. Source inspection found no `WindowOptions.app_id`,
and `runner.desktop` declared `StartupWMClass=com.wycstudios.runner`. In GPUI0.3.7,
`set_app_identity` only sets the Linux notification application name; it does not
set a window app ID. The window option is the path that reaches both Wayland
`set_app_id` and X11 `WM_CLASS` before window mapping.

The fix supplies `runner` for every Linux Runner window and aligns
`StartupWMClass` with the existing `runner.desktop` basename. Installer paths,
icon assets and data identifiers remain unchanged. A contract test compares
the actual packaged desktop entry with the window identity constant. Selected
checks: demonstrate the mismatch with the original desktop metadata, run the
application tests and strict workspace Clippy, and check the Linux build and
desktop entry. After reinstalling the corrected package, the user explicitly
confirmed on 2026-10-20 that the dock icon/name had recovered. This is a
human-reported native pass for the dock correction, separate from the earlier
four passes; no independent after screenshot was captured.

With the original desktop metadata, the exact new contract test failed with
exit101 (`linux-desktop-identity-before.log`): the WM class was
`com.wycstudios.runner` instead of `runner`. Snapshot17 binds the corrected source:
archive SHA256 `e9a533aaf1b318299421fd7a01ad8cfe9542937126c75a75c2c92e445863c209`,
base commit `825d684e`, source identity
`c69feb92d9dcd8be698cbf470d069698bfe2a12b95bd1f9faed1887fd79083dd`.
Mac runner-app tests passed (exit0 /25.75s), as did strict workspace Clippy
(exit0 /6.96s), using profile ci and runtime_shaders. Ubuntu ARM64 runner-app
tests passed (exit0 /70.28s), strict workspace Clippy passed (exit0 /18.03s), and
desktop-file-validate passed (exit0 /0.03s). Mac logs/results use the local
`target/linux-validation/linux-dock-mac-` prefix; Ubuntu results use
`target/linux-validation/linux-dock-`. Formatting and diff checks also passed.

A new ARM64 tar-only build completed on Ubuntu26 (exit0 /240.98s), without
installing it or operating any existing Runner window/session. Its path is
VM `target/linux-packages/dock-fix/Runner-0.16.7-linux-aarch64.tar.gz`, SHA256
`adddbc017da804c8ed94793844ea8d5091d7c4ad159045bd70a03bae3a1b88e0`.
The actual archive contains `StartupWMClass=runner`, `Icon=runner` and the new
Runner payload SHA256 `1a96977558995bf6a93498c961a0462e2f561be0cca93004e55f8c7ecc715ceb`.
CLI/runnerd payloads are identical, SHA256
`20f0bbbbf200ffc8af896e0e906b3f7174747d31eb5ce53f9e850249fb3d2321`.
Full payload and snapshot identity are in VM
`target/linux-validation/linux-dock-package-identity.json`. This is a local
Ubuntu26 package, not an Ubuntu22 compatibility result. Native dock confirmation
comes from the later user report above.

## PR CI scope and performance, 2026-10-20

Hosted run <https://github.com/ruizer/runner/actions/runs/39373177620>, commit
`57054d66`, passed all four jobs. Both Linux architectures passed 2430 tests
(6 skipped) and real package validation in Ubuntu22 containers. Linux x86_64
took 37m40s and ARM64 39m49s: source-installed nextest cost about 5 minutes,
and release/package building another 12 minutes; actual tests took about 140s.
These results belong to that pre-optimization commit, not the later CI changes.

The user requested ordinary PR checks/tests only, with installation packages
built during release. Linux PR CI now retains formatting, strict workspace
Clippy, nextest, CLI reference/lockfile checks, installer fixtures and desktop
entry validation, with no release/package build. Existing release/nightly build,
package verification and artifact steps are retained unchanged. Linux CI also
uses pinned prebuilt nextest 0.9.148 for each native architecture, successful PR
cache writes, Cargo's default CPU concurrency and a 60-minute timeout. The
ordinary-user, locale, init and exact-checkout-trust corrections are retained.

Selected checks: workflow parsing and extracted shell `bash -n` passed, with
macOS/Windows jobs identical to `57054d66` and no packaging command in Linux PR
steps. Both downloaded archives contain the expected ELF architecture and need
at most GLIBC2.27; the ARM64 binary reports nextest0.9.148 in the Ubuntu VM.
`python3 script/linux/test-packaging.py` passed all16 cases in0.571s, and
`desktop-file-validate packaging/linux/runner.desktop` exited0. These are scoped
workflow checks; no Rust source changed and no additional local workspace test
run or release/nightly dispatch was selected. The optimized hosted run and any
later warm-cache timing must be observed separately before claiming a speedup.

The optimized run <https://github.com/ruizer/runner/actions/runs/39378745359>
subsequently passed all four jobs for `825d684e`. Linux ARM64 took14m41s and
x86_64 19m21s; both passed2430 tests with6 skipped. Both were cache misses and
successfully saved separate approximately1.2GB caches; this is not a warm-cache
timing. These results predate the later dock-identity correction above.

## Rebase onto 0.16.7, 2026-10-20

After the user reported another upstream update, fetch advanced upstream/main
from `a9d3b290` to `f6ab9ef6`. Seven upstream commits add collapsible Active
sidebar groups, restore the usage footer and bump the workspace to 0.16.7.
The clean feature commit `feba4c9a` was backed up in
`target/linux-validation/pre-rebase-feba4c9a.bundle`, then rebased without text
conflicts to intermediate commit `d40ac98f`. Explicit repository author identity
was checked before rewriting. `git range-diff` reports an identical feature
patch, including the container CI correction below. Cargo.lock differs only by
the upstream workspace version bump; CI/release/nightly and bundler bytes remain
identical to `feba4c9a`.

Review covered the overlapping app settings, usage footer, sidebar tests and
shortcut guide. Upstream's new collapse state, row numbering and footer behavior
are preserved. Selected checks are runner-app tests on Mac and Linux for the
changed sidebar/layout, strict workspace Clippy for shared compilation, Mac
updater Clippy for the footer's updater path, and formatting. No runtime adapter,
packaging source or additional native GUI behavior changed in this rebase.

Snapshot15: archive `target/linux-checkpoint-15.tar.gz`, SHA256
`82c3836cd4a313ef6f9654231ddc7eac3beb364ebdc07d1f4967abb4d3cdc1c6`,
1530 entries, source identity
`e8bbf5605b44fbf7f3e5867ec933a3906b3df69a2029f9a0f2a0f7ae88e91896`.
The VM verified all source files before running checks; later record edits do
not change the tested Rust code or workflows. Results use the
`target/linux-validation/rebase-0167-` prefix. Hosted CI must test the final
rebased commit; the earlier `feba4c9a` run is not that result.

| Platform | Scoped check | Exit / seconds |
| --- | --- | --- |
| Mac | Workspace strict Clippy, profile ci, runtime_shaders | 0 /21.19 |
| Mac | App updater strict Clippy, profile ci, updater + runtime_shaders | 0 /14.95 |
| Mac | runner-app package tests, profile ci, runtime_shaders | 0 /46.51 |
| Linux ARM64 | runner-app package tests, profile ci | 0 /81.84 |
| Linux ARM64 | Workspace strict Clippy, profile ci | 0 /26.03 |
| Mac | cargo fmt --all --check; git diff --check | 0 |

These are scoped rebase checks; no new native GUI or 0.16.7 package validation
is inferred from them. The later workflow-only correction below was syntax
checked separately and still needs hosted execution.

## Fork CI environment correction, 2026-10-20

Fork run <https://github.com/ruizer/runner/actions/runs/39360247434> tested
feature commit `7aa82ecf`. macOS and Windows passed. Both Linux architectures
completed formatting and strict workspace Clippy, then reported the same seven
failures: 2423 passed, 7 failed, 6 skipped. Packaging steps were skipped and have
no hosted result in that run. Raw failed-job logs are retained locally in
`target/linux-validation/fork-ci-39360247434-failed.log`.

Three permission-denial tests unexpectedly wrote successfully because the
Ubuntu job container ran tests as root: MCP read-only configuration, mission
roster rollback, and read-only skill saving. Three spawn/fork golden tests
included the `LC_CTYPE=UTF-8` fallback because the container supplied no locale.
The killed-daemon test still observed the child's PID with `kill(pid, 0)`;
an exited orphan that PID 1 has not reaped is a candidate explanation, not yet
a proven product process leak or a verified correction.

The scoped fix changes only Linux CI execution: set `LANG` and `LC_ALL` to
`C.UTF-8`, run the unchanged workspace tests as an ordinary dedicated user with
access to the existing Rust toolchain/build tree, and start the job container
with Docker `--init` to reap orphaned children. No assertions, expected snapshots,
production Rust code, macOS job or Windows job are changed.

Selected checks: YAML parsing and `sh -n` validate the changed workflow and test
command (both exit0); rerun the seven exact tests as the ordinary Ubuntu ARM64
VM user with `C.UTF-8`; then rerun hosted CI to validate the actual Ubuntu22
container, ordinary-user execution and init behavior on both architectures.
The local VM has no container engine and the Mac Docker daemon is unavailable,
so local ordinary-user results do not establish container behavior. Hosted
verification remains pending until the new run completes.

The seven selected VM commands completed with exit0 as UID1000, using the
unchanged Rust source from snapshot14. Each used
`cargo test --locked -p <crate> --profile ci <exact-test> -- --exact`, with
`--test daemon_process` for the CLI case and `--lib` for daemon cases. Wall
times in the order listed above (daemon cleanup, MCP, roster, skills, fork,
spawn capabilities, spawn) were 28.80, 30.94, 1.13, 0.17, 0.20, 0.13 and 1.90
seconds. Command arrays, exit codes and snapshot identity are retained in
`target/linux-validation/fork-ci-fix-focused-result.json`; per-case logs remain
in VM `target/linux-validation/fork-ci-fix-focused/`. This is a selected-test
pass, not a container, x86_64 or full-workspace pass.

### Second hosted run and checkout ownership correction

Run <https://github.com/ruizer/runner/actions/runs/39363486421>, commit
`feba4c9a`, passed macOS/Windows and both Linux workspace test jobs, including
the original seven cases. Linux installer fixtures also passed. Both Linux
architectures then failed at the start of packaging: Git refused to read HEAD
with `detected dubious ownership`. The prior correction changed checkout
ownership for ordinary-user tests; the later root packaging process had no
persistent trusted-checkout entry. This was a CI configuration omission, and
no release binary or package was built in this run. Original logs remain in
`target/linux-validation/fork-ci-39363486421-failed.log`.

The follow-up explicitly adds only `$GITHUB_WORKSPACE` to Git's safe-directory
list in the ephemeral CI container after changing ownership. This does not
change the user's Git configuration, author identity, product code or package
script, and preserves the later lockfile diff check. The workflow parses and
the extracted test script passes `sh -n`; the final rebased commit must still
complete hosted CI, including the previously unreached release/package stages.
A local scratch repository with Git's `GIT_TEST_ASSUME_DIFFERENT_OWNER=1`
reproduced exit128 before the exact-path safe-directory entry and exited0 after
it; `GIT_CONFIG_GLOBAL` pointed only to the scratch configuration. This checks
the Git correction without changing the user's configuration, and is not a
Docker or package-build pass.

## Rebase review, 2026-10-20

The user requested reviewing new upstream changes and rebasing the committed
feature. The worktree was clean and explicit repository author configuration was
`ruizer <1244388148@qq.com>`. Commit `e4143765` was backed up in the ignored
`target/linux-validation/pre-rebase-e4143765.bundle`, then rebased from `4a5e5ecf`
onto upstream `a9d3b290` (Runner 0.16.6), producing intermediate HEAD
`81169c3f2a8c98b9f4234d5fab254d1f678c51c4`. Git reported no text conflicts.

Review covered the nine overlapping paths, the Active sidebar's new call sites,
primary-modifier routing, sidebar update-button gating, window geometry/theme
tests, i18n, lockfile and guide merges. The upstream design-repository migration
was preserved without accessing that external repository. `git range-diff`
preserved the feature patch apart from upstream guide context. After normalizing
workspace version 0.16.6 to 0.16.5, Cargo.lock is byte-identical to the previous
feature lockfile. Linux CI/release/nightly definitions, build dependencies and
bundler are also byte-identical to `e4143765`.

Selected checks were stated before execution: runner-app tests cover the new
Active sidebar, existing platform chrome/layout and theme consumers; workspace
Clippy covers shared and target-gated compilation; Mac updater Clippy preserves
the updater-bearing sidebar path; formatting checks the one-line test change.
Runtime adapters and packaging code did not change, so no complete workspace,
agent, package rebuild or native GUI test was selected for this rebase.

Review found one compatibility issue: upstream's new
`active_shortcut_and_row_click_focus_the_split_pane_needing_attention` test sent
Cmd+1 on every non-Windows platform. Linux binds Ctrl+1, so the original test
remained on Roles instead of selecting Chat. The exact Linux test failed with
exit101 /58.65s on snapshot12, archive SHA256
`17f3f9fce3f375fccc23ec4df21692f641523d8b9024db9ef2ccd719df71b6e3`.
Its log/result prefix is `rebase-20261020-linux-active-before`.

An intermediate correction incorrectly used `fixed_shortcut`, which produces a
display label rather than executable keystroke syntax. Snapshot13 retained that
attempt: Linux application tests exit101 /94.19s, Mac application tests exit101
/43.85s, with this same test as the only binary-test failure. These results remain
under `rebase-20261020-linux-app-tests` and `rebase-20261020-mac-app-tests`.
The final change uses `keymap::platform_default("cmd-1")`, as existing input
tests do. It changes one test line, retains every route/focused-pane assertion,
and changes no product key binding or runtime behavior.

The corrected snapshot14 archive is `target/linux-checkpoint-14.tar.gz`, SHA256
`cc387c2eecfb0b406ed1d34cde5b1def7f2791e617a39ce91e25145c8c9d4c12`;
manifest `target/linux-checkpoint-14.json`, 1530 source entries, intermediate HEAD
`81169c3f2a8c98b9f4234d5fab254d1f678c51c4`, source identity
`87506705b1832bd896ac3203b123f238105c392b83b72f3e5bb838fbed0e6425`.
Mac and VM source bytes matched that snapshot before the documentation update.

| Platform | Command | Exit / seconds | Log/result prefix |
| --- | --- | --- | --- |
| Mac | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | 0 /7.67 | `rebase-20261020-mac-fixed-clippy` |
| Mac | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | 0 /5.54 | `rebase-20261020-mac-fixed-updater-clippy` |
| Mac | `cargo test --locked -p runner-app --profile ci --features gpui_platform/runtime_shaders` | 0 /30.47 | `rebase-20261020-mac-fixed-app-tests` |
| Linux ARM64 | `cargo test --locked -p runner-app --profile ci` | 0 /82.01 | `rebase-20261020-linux-fixed-app-tests` |
| Linux ARM64 | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 /33.49 | `rebase-20261020-linux-fixed-clippy` |
| Mac | `cargo fmt --all --check` | 0 | Coordinator command result |

Mac used build jobs8 and runtime shaders, without installing Xcode. Linux used
jobs2, `LC_ALL=C.UTF-8`, `LANG=C.UTF-8`, and PATH containing the existing private
Node22.14.0 directory, `/home/lr/.cargo/bin`, `/usr/bin`, `/bin`, `/usr/sbin` and
`/sbin`. Both final app logs explicitly pass the corrected test; existing ignored
cases remain ignored. The existing upstream `block` future-incompatibility notice
remains on Mac.

Linux evidence is `target/linux-validation/rebase-20261020-evidence.tar.gz`,
SHA256 `692a1f6419de29c0a6b6643c0e6a45b020f1977d0e4d05c6bc31f82e59013123`,
with nine ordinary members verified and copied into `rebase-20261020-evidence/`.
Mac logs/results remain directly under `target/linux-validation/`.
The incremental review found no remaining must-fix issue after the keyboard test
adaptation. The final commit folds that fix and this documentation into the rebased
feature; its new commit identity does not relabel prior snapshots. No push or PR
was performed. Full-workspace and package results recorded below still belong to
their older 0.16.5 candidates; no 0.16.6 packages or fresh live acceptance is claimed.

## Selected checks before execution

| Changed behavior | Checks and reason |
| --- | --- |
| Linux descendants and process-assisted capture | /proc parser/process fixtures, missing/dead PID, exact rollout root, duplicate descriptors, claimed/ambiguous/timeout tests; prevent orphan cleanup and conversation misidentification. |
| Linux daemon/CLI and service | DAE-CLI-01/02, DAE-ENV-01, DAE-REPLACE-01, CLI-STATUS-01, CLI-EXIT-01, CLI-CONNECT-02, isolated synthetic variants: startup, SSH guard, minimal PATH, lock/endpoint ownership, hash replacement. |
| Linux GUI selection/fonts/Ctrl/chrome | TERM-INPUT-01/04, TERM-RESIZE-01, TERM-RENDER-02, WS-WINDOW-01, WS-CWD-03, SET-INSTALL-01/03; coordinator runs shell UTF-8/copy/paste/Ctrl+C/folder/window checks and notification availability. |
| Packages | Architecture ELF/labels, staging hashes, tar/deb isolated install/upgrade/uninstall preserving data/foreign links, rpm metadata/layout, no enabled service, headless first launch. |
| Platform gates/shared changes | macOS workspace Clippy and tests with GPUI runtime shaders, updater Clippy, fmt and diff checks. Final Linux workspace Clippy/tests because new platform spans all crates. |

Real agent/hook/crew checks need existing sign-in; VM has none and credentials must not
be copied. Synthetic process/IPC evidence is labelled and is not a native agent pass.
Native IME, WSL/WSLg, Windows and unexecuted x86_64 checks remain Blocked with prerequisites.
No full unrelated smoke suite is selected. Coders/reviewers do not launch the app.

## Attempts and results

Implementation in progress; no feature pass claimed. Preserve initial failures here
with exact commands/exit codes and later scoped retries; coordinator supplies Linux
source identity, native evidence, package paths/hashes and cleanup ledger.

- Initial Mac headless `cargo check --locked -p runner-core -p runner-terminal -p runner-daemon -p runner-cli --all-targets --profile ci`: exit 101, lockfile needs new direct Linux GPUI dependency edges. Regenerating lockfile without changing dependency versions before retry.

- Mac headless `cargo check --offline -p runner-core -p runner-terminal -p runner-daemon -p runner-cli --all-targets --profile ci`: exit 0, 42.85s after adding newly activated Linux backend edges to Cargo.lock.
- Mac headless `cargo clippy --locked -p runner-core -p runner-terminal -p runner-daemon -p runner-cli --all-targets --profile ci -- -D warnings`: exit 0, 11.97s (checkpoint 1).
- Mac app `cargo check --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders`: exit 0, 52.54s; emitted one WIP duplicate Managed uninstall-arm warning. Fixed before checkpoint 2; strict final gate still required.
- `python3 script/linux/test-packaging.py`: four installer ownership/preservation tests pass. These are synthetic file fixtures, not native package/service evidence.
- Checkpoint 1 immutable archive: `target/linux-checkpoint-1.tar.gz`, SHA256 d36b7072d3364071600745f3f75cd68b2c8aded862773ff5d33012e2240a2791; HEAD 769e2e401e33261ad333a311fea07207df7f5f5c + dirty 91a8d3c01b30be9739edb738539f7d16d85e3094f36ce31404228a630f40d590. Coordinator's earlier a671... snapshot is retained for investigation, not attributed to this check.

- Initial full Mac daemon `cargo test --locked -p runner-daemon --lib --profile ci`: exit 101, 1275 passed / 2 failed in 26.00s. Failures: `session::manager::tests::replay::session_scenario_goldens` (Claude alias-resume `history_exists=false`, expected true), and `session::manager::tests::golden::spawn_goldens` (`--session-id`, expected `--resume`). Cause: fixture-only old cwd encoding left underscores intact while the current encoder normalizes all nonalphanumeric UTF-16 units. Mac TMPDIR contains underscores. Both fixtures now reuse the current encoder; golden files were not changed.
- Narrow retries with `cargo test --locked -p runner-daemon --lib --profile ci session_scenario_goldens` and the same command ending `spawn_goldens`: both exit 0, one selected test each (1.36s / 3.68s). No full daemon rerun after this isolated fixture correction.
- Nightly initial workflow tests: exit 1, 12/15 passed; three pre-existing retention expectations kept more than the two-build contract. Corrected expected stale assets to all but the newest old stamp. `uv run --no-project --with pyyaml python script/test-nightly.py -v`: exit 0, 15 tests, 4.672s. Added Linux/all build predicates, exact architecture assets, partial-upload/public-download failure gates and independent two-stamp Linux retention.
- Installer fixtures: `test_custom_prefix_quotes_and_service_is_not_enabled`, `test_foreign_service_desktop_and_package_directory_are_preserved`, `test_install_upgrade_uninstall_retains_user_data_and_foreign_replacements`, `test_refuses_foreign_commands_before_writing` all pass. Prefix includes Unicode, spaces, quotes, dollar, backtick and percent; systemd/Desktop encodings are distinct.

## Coordinator-only native service check

Build the `linux_service_check` example and prepare two genuinely distinct release builds/stamps; never edit binary bytes to create a mismatch. The first argument is an extracted package `lib/runner` directory containing actual same-byte `runnerd` and `runner-agent-cli`; the second contains the second build's `runner-agent-cli`.

```sh
CARGO_BUILD_JOBS=2 cargo run --locked -p runner-cli --example linux_service_check --profile ci -- /absolute/old-package/lib/runner /absolute/new-release-bin-dir
```

The harness creates `/tmp/r823-svc-*` roots and uniquely named `runner-823-<pid>-<index>.service` transient user units with the shipped Restart/KillMode/TimeoutStopSec policy. It never enables or edits `runnerd.service`. It verifies app-first/service-first lock ownership, duplicate successful exit, same-hash app reuse, normal Stop, different-hash stamped replacement with zero systemd restarts, per-user installed daemon hash and endpoint cleanup. It prints root, hashes and PIDs; retain stdout plus `systemctl --user show`/journal evidence as available. Drop cleanup stops only its own units/daemon and removes its temporary root. A missing user service manager or distinct build leaves this check Blocked; direct process tests do not establish a systemd pass.

Package verification runs `python3 script/linux/test-packaging.py --packages target/linux-packages`: tar installer lifecycle and actual CLI headless boot in short `/tmp/r823-pkg-*` roots, deb extraction plus isolated dpkg install/upgrade/foreign-command refusal/remove/purge/headless boot, rpm metadata/layout. The isolated dpkg fixture uses --force-not-root, --force-script-chrootless and --force-depends to validate lifecycle independently of host installation/dependency resolution; it must execute successfully before a package-manager lifecycle pass is claimed. None of these launch the GUI or real agents.

- Coordinator ARM64 checkpoint 1: `CARGO_BUILD_JOBS=2 cargo check --locked -p runner-core -p runner-terminal -p runner-daemon -p runner-cli --all-targets --profile ci`, exit 0 / 74.15s, immutable d36b... archive + HEAD769e2e40/dirty91a8..., logs under VM `target/linux-validation/checkpoint-1-headless-*`. The initial BSD tar carried AppleDouble metadata; coordinator removed those extra entries. Subsequent checkpoints use Python tarfile with only manifest-listed source files.
- Mac shared `cargo check --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders`: exit 0 /14.20s. `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings`: exit 0 /16.22s (`target/linux-mac-check.log`, `target/linux-mac-clippy.log`). The upstream `block` dependency reports a future-incompatibility notice, not a workspace lint failure.
- Linux GUI development prerequisites remain missing at coordinator's last report (fontconfig/FreeType/xkbcommon-X11/Wayland/clang/libclang/rpm). One human apt request is pending; no repeated sudo/password request. Native GUI, actual packages, systemd, x86_64/WSL and final Linux workspace gates remain pending.

- Mac `cargo test --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders`: exit 0; aggregate 713 passed / 0 failed / 3 existing ignored tests (real paid-provider smoke/performance fixtures), 10 test binaries, log `target/linux-mac-app-tests.log`. No dev app or paid agent was launched.
- Mac updater `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings`: exit 0, 1m32s including target-lock wait, `target/linux-mac-updater-clippy.log`.
- `cargo fmt --all --check`, `git diff --check`, changed Markdown local link target check: exit 0 / no missing local targets. CLI help was unchanged; Linux error remediation is covered by platform-aware roundtrip/golden expectations.

## Independent review fixes (source review clean; native validation pending)

Reviewer reproduced a foreign icon symlink target overwritten on upgrade, and replacement unit/desktop symlinks removed on uninstall. The installer now refuses icon symlinks or changed icon bytes before any mutation, and retains replaced symlinks on uninstall. Added `test_upgrade_and_uninstall_preserve_replaced_foreign_icons` and `test_uninstall_preserves_replacement_unit_and_desktop_symlinks`; source/system prefix spellings are also checked before mutation.

Reviewer found missing Linux appearance-preview updater test setup and an unconditional sidebar-update expectation despite Linux update UI being hidden. Both test paths now explicitly cover Linux's disabled app updater. The Linux plus-key meta label expects Super, preserving the Windows Win label.

Nightly Debian Version and RPM Release initially sorted by SHA before stamp. Metadata now puts stamp before SHA; filenames keep the common SHA/stamp identity. Version tests use decreasing SHA and increasing stamp, with actual dpkg/rpm comparator tests on Linux. Mac result: 9 passed / 2 explicitly skipped because dpkg/rpm are absent. Prebuilt `--binary-dir` mode was removed: it could not establish namespace from CLI `--version`. Bundling always builds both binaries with an explicit native release target and debug assertions disabled. The rejected-prebuilt-input regression uses the actual `target/ci` path.

Fresh-headless integration fixture now stages the actual `runner-agent-cli` package-source basename and proves the app-data CLI was absent before spawn. The prior fixture staged only `runner` and therefore could not exercise source bootstrap.

Linux cleanup initially retained only PID integers. It now retains pidfds after verifying proc start-time identities and uses pidfd polling/signals, avoiding signalling reused PIDs. New synthetic identity-change refusal and owned-child pidfd lifetime tests passed ARM64 execution with the existing orphan test in the checkpoint4 results below. macOS keeps its existing signal behavior.

For a headless service-only validation before GUI libraries are available, coordinator may run `./script/linux/test-service`. It builds two honestly labelled release CLI fixture variants (same source SHA, distinct explicit fixture stamps/marketing labels), copies each actual binary to runnerd, prints SHA256 values, then runs the native systemd example. These are CLI service fixtures, not published/package/native-GUI acceptance. This optional path requires a working systemd user manager; it never enables a persistent unit, invokes an agent, or edits executable bytes.

Mac CLI `cargo test --locked -p runner-cli --test roundtrip --test golden --profile ci`: exit 0, 86 golden +22 roundtrip tests passed, log `target/linux-mac-cli-tests.log` (compile29.74s, tests56.99s/0.20s).

- Review follow-up retains and checks the original Linux root pidfd/start-time before/after enumeration; changed-root token is a refusal case. pidfd polling retries EINTR. This closes root reuse as well as child reuse; Mac/Windows cleanup contracts are retained.
- Native package aliases were removed from ordinary payload ownership. Shared embedded maintainer scripts check all three links before install/upgrade, create only owned/missing aliases, distinguish RPM upgrade from erase, and remove only exact owned links on Debian remove/purge. `NativeAliasTests` verifies foreign regular/symlink refusal/preservation. Current Mac packaging suite: 10 passed / 2 explicit missing-tool comparator skips.
- Narrow Mac post-review orphan check `cargo test --locked -p runner-daemon --lib --profile ci stop_reaps_descendants_the_agent_left_behind`: exit0, one test passed /0.28s. Narrow sidebar check `cargo test --locked -p runner-app --bin Runner --profile ci --features gpui_platform/runtime_shaders available_app_update_sits_right_after_runner_in_the_sidebar_header`: exit0, one test passed /0.17s. Linux equivalents remain required for platform-only branches.

- Independent reviewer verdict at 2026-10-16 07:14 UTC: **NO REMAINING MUST-FIX ISSUES (source review)** for checkpoint4, HEAD769e2e401e33261ad333a311fea07207df7f5f5c + sourceb86bd7256d2de60c3912131833154514d50cc148d44997066ed5978ac3855338, archive SHA256a79a9c8b3b83e581afb04f6b9f87664fa8778f4fed9c8b482a273774e4c79d53, all1530manifest hashes verified. Reviewer independently observed10packaging fixtures pass/2explicit unavailable comparator tools. This is source review only: current Linux proc/pidfd/orphan/fresh-headless/managed/example checks, real packages/systemd/GUI and final Linux workspace gates remain required.
- Post-review Mac `cargo clippy --locked -p runner-daemon -p runner-cli --all-targets --profile ci -- -D warnings`: exit0 /9.67s, `target/linux-mac-review-clippy.log`. No production changes after checkpoint4; this record's subsequent evidence-only update does not change executable source.

- Frozen-checkpoint4 Mac workspace recheck after Descendant call-site refinement: `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings`, exit0 /7.66s, `target/linux-mac-checkpoint4-workspace-clippy.log`. Executable source matches checkpoint4; only this evidence document differs from its manifest. Reviewer read-verified the preceding evidence-only edit without changing its source verdict.

- Coordinator reported old checkpoint3 `cargo test --locked -p runner-cli --test daemon_process --profile ci fresh_headless_daemon_installs_real_shim_and_starts_shell_without_shell_env`: exit101 /32.90s, missing app-data bin/runner. This is the retained, already-fixed source-basename fixture failure; it is not attributed to checkpoint4.
- Checkpoint4 initial Linux proc-filter compile: exit101, compiler could not find Managed/inspect symbols although the verified checkpoint4 core source contains them. The initial investigation suspected extraction with mtime0; coordinator corrected this at 2026-10-16 07:18 UTC: checkpoint4 retained historical source mtimes, which can still predate existing Cargo fingerprints. Content hashes match checkpoint4. Coordinator is refreshing only manifest-listed source mtimes and forcing affected workspace crates to recheck while retaining dependency cache; stale artifact reuse remains under investigation until the rebuild result. No production-code workaround or success claim; initial failure is retained separately from the checkpoint3 fixture bug.

## Checkpoint4 ARM64 headless validation after timestamp refresh

Coordinator reported these results at 2026-10-16 07:24 UTC, confirmed by architect at 07:25 UTC. Source is the immutable checkpoint4 archive SHA256 `a79a9c8b3b83e581afb04f6b9f87664fa8778f4fed9c8b482a273774e4c79d53`, HEAD `769e2e401e33261ad333a311fea07207df7f5f5c`, dirty source SHA256 `b86bd7256d2de60c3912131833154514d50cc148d44997066ed5978ac3855338`. Coordinator verified content and refreshed source mtimes before recompiling; there was no production-code change. This successful rebuild establishes recovery from the initial stale-artifact failure retained above. Logs are under VM `target/linux-validation/`. Commands ran serially with `CARGO_BUILD_JOBS=2`.

| Command | Reported result |
| --- | --- |
| `cargo test --locked -p runner-daemon --lib --profile ci -- proc_ stop_reaps_descendants_the_agent_left_behind` | Single invocation with two libtest filters: 6/6 passed, 21.25s |
| `cargo test --locked -p runner-cli --test daemon_process --profile ci fresh_headless_daemon_installs_real_shim_and_starts_shell_without_shell_env` | 1/1 passed, 15.53s; real generated shim and terminal without SHELL |
| `cargo test --locked -p runner-daemon --lib --profile ci packaged_command_is_managed_and_settings_uninstall_preserves_it` | 1/1 passed, 0.49s |
| `cargo check --locked -p runner-cli --examples --profile ci` | Passed, 4.80s |
| `python3 script/linux/test-packaging.py` | 11 passed / 1 explicit skip because rpm is absent, 0.53s |

The packaging result covers fixtures and the available dpkg version comparator, without `--packages`; it does not establish actual tar/deb/rpm package lifecycle acceptance. Native systemd, GUI, actual packages and final Linux workspace gates remain pending. x86_64, older-baseline, WSL, real-agent and IME gaps remain explicit; these headless results are not a Linux feature pass.

Coordinator corrected the proc command at 2026-10-16 07:32 UTC: the planned two separate filtered commands were actually executed as the single two-filter invocation recorded above. The 21.25s duration belongs to that one invocation. At this update, `checkpoint-4-systemd` is still cold-building two release CLI fixture stamps with jobs2; it has no native service result yet. Headless Linux strict Clippy is queued after that task.

## Checkpoint4 ARM64 native service and headless lint results

Coordinator's 2026-10-16 07:44 UTC report, relayed by architect at 07:46 UTC, completes this headless batch on the same checkpoint4 archive/HEAD/dirty identity recorded above. Production source is unchanged; only this evidence document differs from the frozen manifest.

- `sh script/linux/test-service`: exit 0 / 282.62s including both release CLI fixture builds, run label `checkpoint-4-systemd`; coordinator supplied `CARGO_BUILD_JOBS=2` in the child environment. Old executable SHA256 `96b3016d2f7dbb1a3758fefb07e4b0373c5cb31cc9887f72ba82b285805f6f56`; new executable SHA256 `60c1fee4086f3f1839917c03d157099d2f0de43e908c5a1c95c87cea28e89e11`. App-client-first reused PID 27657; service-first PID 27683 covered duplicate successful exit and normal Stop. Different-hash replacement moved PID 27713 to 27730 with the old unit inactive and `NRestarts=0`.
- Service units, daemon endpoints and temporary roots were cleaned up. Coordinator retained the redundant not-loaded cleanup diagnostics and saved the journal at VM `target/linux-validation/checkpoint-4-systemd-journal.log`. The scoped unit list was empty and `/tmp/r823-svc-d3Fr9H` no longer existed after cleanup. This is native user-systemd plus app-client `Launch` evidence using release CLI fixtures; it does not establish GUI or actual-package acceptance.
- `CARGO_BUILD_JOBS=2 cargo clippy --locked -p runner-core -p runner-terminal -p runner-daemon -p runner-cli --all-targets --profile ci -- -D warnings`: exit 0 / 15.84s, run label `checkpoint-4-headless-clippy`, logs under VM `target/linux-validation/`. This covers Linux-only headless types, syscall code and test lint paths; it does not replace the final workspace Clippy/test gate.

Current remaining gates: GUI, actual tar/deb/rpm packages and final Linux workspace Clippy/tests are **Blocked** on the previously requested development/package dependencies. No additional compilation or rootless dependency route is being started while awaiting those prerequisites. x86_64, older-baseline, WSL, real-agent and IME gaps remain as recorded. No Linux feature pass, commit, push or PR is claimed.

## Dependency-ready app check and checkpoint5 warning correction

Coordinator confirmed dependency installation complete at 2026-10-16 08:11 UTC and resumed the same mission on source4. The ARM64 app all-target check completed with exit 0 / 173.52s, reported by architect at 08:28 UTC. It emitted two warnings: `surfaces/app_shell.rs` declared `zoom` although its only use is in the non-Linux updater hint, and `notifications/permission.rs` retained `Allowed`/`Denied` variants that Linux neither constructs nor uses in tests. This original warning-bearing result is retained; it is not a strict-lint pass.

Checkpoint5 changes only these two production files relative to source4: `zoom` now shares the updater hint's `not(target_os = "linux")` condition; `Allowed`/`Denied` are compiled only on macOS or Windows, matching their constructors, consumers and existing tests. There are no broad lint allowances or behavior changes. Selected local checks cover cfg/type compatibility and formatting: `cargo check --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders` passed on Mac, exit 0 / 7.80s, log `target/linux-mac-checkpoint5-app-check.log`; only the existing upstream `block` future-incompatibility notice remains. `cargo fmt --all --check` and `git diff --check` passed. No Mac full test rerun was selected for these cfg-only changes.

The new immutable checkpoint5 archive and complete source manifest are `target/linux-checkpoint-5.tar.gz` and `target/linux-checkpoint-5.json`; the manifest records HEAD, dirty-source SHA256 and every file/symlink content hash. Relative to checkpoint4, only these two production files and this evidence document may differ. Coordinator will run Linux app strict Clippy and the actual GUI build serially with jobs2 after review. GUI, actual package lifecycle and the final Linux workspace Clippy/test gate remain unexecuted; dependency readiness is not a feature pass. Earlier dependency-Blocked entries describe the prior phase. Agent, IME, x86_64, older-baseline and WSL gaps remain unchanged.

## Source5 package failure and checkpoint6 helper diagnostics

Source5 identity: archive SHA256 `1e96f2ffe9fa9219768357c55f3aa90131d98f3c71492c27c093c265a98bb492`, HEAD `769e2e401e33261ad333a311fea07207df7f5f5c`, dirty-source SHA256 `efed1b5c2122e6aea76cf90068f4bdb493281004d973d12d10351da9680132ee`. Coordinator reported actual package build exit 0 / 412.30s, then actual package lifecycle verification exit 1 / 1.84s (architect relay at 2026-10-16 09:05 UTC). The first `dpkg --install` in `check_deb_lifecycle` returned exit 1, but the helper's captured stderr was hidden by `CalledProcessError`; the root cause is unknown pending the coordinator's raw stderr reproduction. Earlier tar/extracted-deb steps do not establish a full package lifecycle pass. These build artifacts and results remain attributed to source5.

Checkpoint6 changes only `script/linux/test-packaging.py` and this evidence document relative to source5. Unexpected checked-command failures now report exact argv, stdout and stderr before raising; successful commands and expected foreign-command refusals remain captured and quiet. All RPM comparator/metadata/listing invocations use fixture-private `--dbpath` directories, avoiding the host RPM database/lock. Product packaging, maintainer scripts and installer behavior are unchanged. No product fix is inferred from the unknown dpkg failure.

Selected helper checks: `python3 script/linux/test-packaging.py` passed on Mac, 12 passed / 2 explicit missing dpkg/rpm comparator skips, 14 total, 0.441s, log `target/linux-mac-checkpoint6-packaging.log`. Two new subprocess tests verify unexpected failures expose both streams and preserve the exit code, while success/expected failure remain quiet. `python3 -m py_compile script/linux/test-packaging.py` and `git diff --check` passed. No Mac full gate, VM or app was run by impl. After review, source6 helper may validate the original source5 products without rebuilding unchanged binaries; the helper and product identities must both be recorded.

GUI evidence remains limited to the coordinator's source5 visible window rendering and New terminal displaying bash. Input, shortcuts, clipboard, resize and other selected interaction checks have not passed. UI work paused after UTM mouse capture interfered with the user's mouse; crew does not take over the desktop or resume capture. An inherited SSH `LC_CTYPE=UTF-8` warning remains a fixture/environment observation; any launcher-only `LC_ALL`/`LANG=C.UTF-8` correction is not a product change. Actual packages and the final Linux workspace gate remain pending; no feature pass is claimed.

## Confirmed dpkg fixture errors and checkpoint7 correction

Architect relayed coordinator's `checkpoint-5-dpkg-diagnose.log` at 2026-10-16 09:07 UTC: exit 1 / 0.04s, `could not open log /var/log/dpkg.log: Permission denied`; preinst and cleanup postrm also failed to chdir into `/tmp/r823-dpkg-kereoic9/root` because it did not exist. These establish helper setup failures, not a product-package defect. `check_deb_lifecycle` now creates its isolated install root before invoking dpkg and supplies `--log=<scratch>/dpkg.log`, alongside its existing private admin/install directories.

Checkpoint6 had already been frozen before the additional diagnosis was read: `target/linux-checkpoint-6.tar.gz`, SHA256 `5882e12a8a3b277d8a55658100488c0a65e476c61f94f87b0baa30d46890a468`, dirty-source SHA256 `39676012c6e904143711b9ca62389d21183cf22e60315aee056505bfb155b4a3`, unchanged HEAD. It remains immutable as a diagnostics-only intermediate; it does not include the root/log correction and is superseded by checkpoint7 for lifecycle validation. Source5 products are still unchanged. Checkpoint7 adds the two setup corrections and a helper test that intercepts the first dpkg invocation, verifies the install root exists and the log is scratch-local, then fails before any package install. No host dpkg installation occurs in this fixture.

Checkpoint7 local checks: `python3 script/linux/test-packaging.py` exit 0, 13 passed / 2 explicit missing dpkg/rpm tool skips, 15 total / 0.448s, `target/linux-mac-checkpoint7-packaging.log`. Python syntax compilation and `git diff --check` passed. Only the helper and this record differ from source5; native retry and actual package acceptance remain pending. After review the immutable archive/manifest are `target/linux-checkpoint-7.tar.gz` and `target/linux-checkpoint-7.json`, with separate attribution to unchanged source5 package binaries.

## Retained checkpoint7 final-test failure

Before the rebase, coordinator ran `env LC_ALL=C.UTF-8 LANG=C.UTF-8 cargo test --locked --workspace --profile ci --no-fail-fast` against checkpoint7: exit 101 / 370.87s. The 29 result summaries total 2345 passed / 4 failed / 6 existing ignored. Raw evidence remains in repository `target/linux-validation/checkpoint-7-final-tests.log` and `checkpoint-7-final-tests-result.json`, which bind HEAD769e2e40, dirty54a8770b... and archive567aa9fa... . The failures were:

- `work_card_insets_every_route_and_contains_chat_and_mission_columns`: `/chats/direct-agent` failed `card.top() == chrome.top() + 8`, comparing32px/40px; actual top inset0px versus expected8px. These are the assertion's left/right values, not horizontal coordinates.
- `sidebar_and_mission_fills_follow_carbon_and_runner_light`: RunnerLight versus Carbon.
- `nav_search_filters_labels_and_removes_empty_groups`: left12/right13.
- `embedded_extension_overflow_fails_closed_and_signals_once`: Node executable absent.

The first three still require classification against the rebased callers; no assertion/golden was changed or test skipped during integration. The coordinator has provided a task-private, officially checksum-verified Node22.14.0 on Ubuntu for a later narrow retry. This is a test prerequisite, not an agent login or product fix. No old result is attributed to the new base.

Architect also relayed these old-base coordinator results on 2026-10-19: checkpoint7 final workspace strict Clippy exit0 /30.04s; helper7 against unchanged product5 actual package lifecycle exit0 /2.73s; helper fixtures15/15 passed /0.30s. These were initially coordinator-reported outcomes; the subsequent raw-log review is recorded below. Product5 keeps its original artifact/source identity, helper7 and Clippy keep HEAD769e2e40 / dirty54a8770b... / archive567aa9fa...; they do not establish any rebased checkpoint8 pass. Earlier failed attempts remain retained above.

## Rebased checkpoint8 integration scope and checks

The coordinator preserved the complete pre-rebase source archive `target/linux-validation/pre-rebase-20261019-004317.tar.gz` (SHA256 `61684c3ef0ddb35a21f49c463e7789c505efc76123e35ca1cdb01425389479a5`) and feature stash `399876349922c75983d71ddb1c902da7c25c3650`, then rebased the brief commit onto upstream `4a5e5ecf`. Impl resolved all 12 content conflicts and marked only those precise paths resolved; no feature commit, push, PR, stash drop, worktree change or VM/app operation was performed.

Conflict decisions preserve upstream About/menu localization; macOS platform exports; Windows chrome localization inside shared `desktop.rs`; multi-target Crew composer state/assertions plus Linux Ctrl undo; localized diagnostics/notification/settings labels with Linux's disabled updater/desktop-owned notification permission; and channel-aware nightly docs/workflows/tests. Automatic-merge audit covered dependency/lockfile edges, shared input/keymap/window paths, file-drop handlers, mission messages, shell/bootstrap/CLI ownership and platform docs. All upstream locked package/version/source triples remain in Cargo.lock; additional edges are the existing Linux backend activation. Linux-only strings use paired English/Chinese catalog keys, including Super and package ownership warnings. File-drop and multi-target message handlers remain upstream's implementation.

Remote-nightly integration changes Linux bundling to accept that channel, retain its artifact prefix/display label, order Debian/RPM metadata by stamp and publish/query/download/prune only the selected channel. `platform=all` still includes both Linux architectures; legacy `both` remains macOS+Windows. Added remote Linux/all isolation, retention and failure-gate fixtures plus remote package-identity coverage. Only Linux CI adds `actions/setup-node@v4` with exact Node22.14.0 to match the coordinator's private test tool; macOS/Windows CI behavior is unchanged. Guide/architecture/spec/plan updates accompany the integration.

Selected checks target integration risks: Mac `cargo check --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders` covers merged shared code/types and both normal/test cfg paths; formatting, conflict-marker/unmerged checks and diff whitespace cover resolution integrity; nightly shell/workflow fixtures cover channel/platform publication isolation without remote dispatch; packaging fixtures cover the changed channel identity; catalog parsing verifies bilingual key/placeholder parity. No workspace/full Mac rerun or native GUI was selected. Initial Mac app check passed in13.78s (`target/linux-mac-checkpoint8-app-check.log`); final check after the localized package-shadow warning passed, exit0 /4.45s (`target/linux-mac-checkpoint8-final-app-check.log`), with only the existing upstream `block` future-incompatibility notice. Initial nightly tests had16/18 pass and exposed one hard-coded Linux public-download URL; after correcting it to `$CHANNEL`, all18 passed /5.182s (`target/linux-mac-checkpoint8-nightly-retry.log`; initial failure retained in `target/linux-mac-checkpoint8-nightly.log`). Packaging fixtures:14 passed /2 explicit missing-tool skips,16 total /0.480s (`target/linux-mac-checkpoint8-packaging.log`). Workflow YAML, bilingual key/placeholder parity and upstream lockfile preservation checks passed. Earlier Linux binaries/packages/tests do not establish checkpoint8 validation; coordinator synchronization and narrow retries remain pending.

Independent reviewer concluded **NO REMAINING MUST-FIX ISSUES** for the rebased integration source/current evidence scope at 2026-10-19 00:53 UTC, after reviewing the tracked diff, reused/new untracked blobs, sensitive upstream callers and actual local check logs. This is not Linux feature acceptance: the earlier four failures still require new-base classification and narrow retries. The immutable handoff is `target/linux-checkpoint-8.tar.gz` plus its complete `target/linux-checkpoint-8.json` manifest, bound to rebased HEAD1fff68095aa665c1981aabc3b33c0f56559a31fc; the manifest records dirty-source and file/symlink hashes. Coordinator must verify the archive/content, refresh source mtimes and preserve prior candidate attribution before any Ubuntu checks.

## Historical source5/7 raw-evidence audit

At 2026-10-19 01:21 UTC reviewer independently audited `target/linux-validation/checkpoints-5-7-evidence-20261019.tar.gz`, SHA256 `5c95efc0a36bc981245a9ca09bc42188e49bb9f0334d523c4417a9db2504f82e`, its20 regular files and hashes, and all1530source7 manifest entries. The copied logs/results match the original identities and recorded failures. Source5 app strict Clippy exit0 /17.35s, GUI app/CLI build exit0 /26.11s, release package build exit0 /412.30s, initial lifecycle exit1 /1.84s and dpkg diagnosis exit1 /0.04s were raw-verified. Source7 workspace strict Clippy exit0 /30.04s, helper7→product5 lifecycle exit0 /2.73s,15/15 helper fixtures (test0.248s, command wall0.30s), and the29-summary full-test failure (2345pass/4fail/6ignored) were also raw-verified; no new must-fix issue was found. These remain historical old-base results.

The old source5 command `python3 script/bundle-linux --channel production --arch aarch64` produced the following files under VM `target/linux-packages/`; reviewer independently checked their bytes against the recorded hashes. Source7 helper command was `python3 script/linux/test-packaging.py --packages target/linux-packages`, not a new source7 product build.

| Product5 artifact | SHA256 |
| --- | --- |
| `Runner-0.16.2-linux-aarch64.tar.gz` | `b7d88025be6f2629f3f3a32be24ba50e97d24c77ef8c949dd963be108f22146d` |
| `Runner-0.16.2-linux-aarch64.deb` | `57a824c5e1757b61b1e585e60edacfe5900e3330970f9f14cd0df0ff7d6affe7` |
| `Runner-0.16.2-linux-aarch64.rpm` | `5de1f6870c3181baa36c126dc11c3e9a5fb7a46cf7c49195d94ef3b401fa0df8` |

The GUI-launch metadata binds source5, app/CLI bytes and PID only. Rendered window and bash observations remain coordinator visual reports; this metadata does not prove input/clipboard/resize or other interaction checks. Release package CLI bytes are distinct from ci GUI fixture bytes and must not be interchanged in evidence.

## Checkpoint8 Linux narrow results and checkpoint9 test corrections

Checkpoint8 identity: HEAD `1fff68095aa665c1981aabc3b33c0f56559a31fc`, dirty-source SHA256 `cc068c746b25d9afcc63a603bef71ec155ad64f620e479c29fee3abffaaecce9`, archive SHA256 `0978a30fe77e05a86a71e2c4a495f344ebebf787d66c871bdceff5711a612207`,1557manifest files. Architect relayed these coordinator results at 2026-10-19 01:21–01:22 UTC. Their raw logs/result JSON have since been copied into `target/linux-validation/` with the checkpoint8/9 evidence batch described below:

- `CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8 LANG=C.UTF-8 cargo check --locked -p runner-app --all-targets --profile ci`: exit0 /18.21s, no warnings.
- `CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8 LANG=C.UTF-8 cargo test --locked -p runner-app --bin Runner --profile ci surfaces::mission_workspace::tests::sidebar_and_mission_fills_follow_carbon_and_runner_light -- --exact`: exit0, exact1test passed /54.15s (compile53.97s, test0.05s), label `checkpoint-8-theme-retry`. Original expectation unchanged. This isolated pass does not diagnose or fix the earlier concurrent-suite global-theme failure.
- `CARGO_BUILD_JOBS=2 LC_ALL=C.UTF-8 LANG=C.UTF-8 cargo test --locked -p runner-daemon --lib --profile ci runtimes::pi::pi_status::tests::embedded_extension_overflow_fails_closed_and_signals_once -- --exact`: exit0, exact1test passed /28.57s (compile26.25s, test2.21s), label `checkpoint-8-pi-overflow-retry`; coordinator added only the task-private official Node22.14.0 bin to the child process PATH. No skip or agent account was used.

Checkpoint9 changes only two test bodies and this record relative to frozen8. The Unix work-card test now expects Linux's existing0px top inset and macOS's8px inset, matching `work_card` and adjacent caption tests; right/bottom/sidebar/child containment and collapsed checks remain intact. The navigation test preserves explicit App/Integrations membership and adds the exact System members,12/Linux versus13/other total, Linux-hidden Updates search and existing General route fallback, and non-Linux Updates search/route. It uses the existing scoped English test locale, leaving product language and routing unchanged. No theme expectation, product geometry, packaging or other production code changed.

Selected local check: `cargo test --locked -p runner-app --bin Runner --profile ci --features gpui_platform/runtime_shaders -- surfaces::app_shell::tests::work_card_insets_every_route_and_contains_chat_and_mission_columns surfaces::settings_page::tests::nav_search_filters_labels_and_removes_empty_groups`, exit0,2passed/0failed (compile33.53s, tests0.11s), `target/linux-mac-checkpoint9-card-nav-tests.log`. Only the existing upstream `block` future-incompatibility notice remains; fmt/diff checks passed. No full-workspace tests, VM access or app launch were performed by impl. After independent review, the new immutable handoff is `target/linux-checkpoint-9.tar.gz` and `target/linux-checkpoint-9.json`; production bytes remain identical to rebased8. Linux card/nav retries remain pending, and old source5 packages do not validate the new base. No feature pass or new workspace-green claim is made.

Reviewer concluded **NO REMAINING MUST-FIX ISSUES** for the two checkpoint9 test changes and evidence scope at 2026-10-19 01:24 UTC, after checking product intent, preserved assertions, locale isolation and the actual Mac test log. Linux test retries and strict Clippy remain coordinator-required; this source-review verdict is not feature acceptance.

## Checkpoint9 rebased ARM64 automation and actual packages

Architect relayed the completed coordinator batch at 2026-10-19 02:09 UTC. These results belong to frozen checkpoint9: HEAD `1fff68095aa665c1981aabc3b33c0f56559a31fc`, dirty-source SHA256 `d2c465f577af0bd579293e62b1c9701a1ea96dea5be0942988cd4a6293e82495`, source archive SHA256 `ff24fcef3af57ef8d7bf206895911526b6fefa215f15fe677456096e0fcfe281`,1557manifest entries. Only this evidence record changes after freezing; executable source is unchanged. Raw checkpoint8/9 logs, result JSON and new GUI metadata are now in repository `target/linux-validation/`. Impl checked their recorded argv/identity/counts and computed the evidence archive and all three package hashes. At 2026-10-19 02:15 UTC reviewer independently verified the22-file evidence archive, all copied file hashes, source9 metadata/1557entries, result identities/argv/exits/durations/counts, all three package hashes and the release tarball ELF/binary contents. The reported outcomes are:

| Selected check | Result |
| --- | --- |
| Four named app filters: work-card insets, fixed window navigation, navigation search, Settings route fallback | Exit0 /19.06s; exactly4 passed,0 skipped (compile18.66s, tests0.28s) |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Exit0 /25.81s |
| `cargo build --locked -p runner-app -p runner-cli --profile ci` | Exit0 /41.36s |
| `python3 script/bundle-linux --channel production --arch aarch64 --output-dir target/linux-packages/rebased-20261019`, label `checkpoint-9-packages` | Exit0 /262.86s, output VM `target/linux-packages/rebased-20261019` |
| `python3 script/linux/test-packaging.py --packages target/linux-packages/rebased-20261019` | Exit0 /2.81s; isolated tar/deb install/upgrade/uninstall and headless startup, RPM metadata/layout |
| `python3 script/linux/test-packaging.py` | Exit0 /0.25s command wall;16 passed,0 skipped (test0.209s) |

All Linux builds/checks remain coordinator-owned and serial with jobs2 and child `LC_ALL`/`LANG=C.UTF-8`. The copied result JSON binds the actual argv. The card/nav command used name filters, not libtest exact mode, despite the earlier planned `--exact` invocation; the raw log confirms exactly4 tests selected,4 passed,0 skipped. No rerun is needed for this recording correction:

```sh
env LC_ALL=C.UTF-8 LANG=C.UTF-8 cargo test --locked -p runner-app --bin Runner --profile ci -- work_card_insets_every_route_and_contains_chat_and_mission_columns nav_search_filters_labels_and_removes_empty_groups window_navigation_stays_fixed_when_sidebar_animates settings_routes_fall_back_to_general
```

Checkpoint8 theme/pi commands did use `--exact` and selected one test each. New source9 product files are separate from old source5 artifacts:

| Product9 artifact under local and VM `target/linux-packages/rebased-20261019/` | Locally verified SHA256 |
| --- | --- |
| `Runner-0.16.5-linux-aarch64.tar.gz` | `36e04b58f314f9b6a2093991027db26ca5c22d19cc8ba550660e8ba52f1b601e` |
| `Runner-0.16.5-linux-aarch64.deb` | `7b907b5a0baf367ae0f5fefa121e42c16210e72186d8137e9b7f2994073dc993` |
| `Runner-0.16.5-linux-aarch64.rpm` | `316a20332ec7fc466b0b51965516b711e7c6a9ee2c885b3a8128182bac66e7b3` |

Checkpoint8's app check and unchanged theme/private-Node exact retries remain attributed to8 in the preceding section. The original source7 full-suite failure remains retained: isolated theme success does not establish a fixed concurrent-theme root cause, and no new full-suite/workspace-test pass is claimed. The earlier native systemd service fixture remains source4 evidence and has not been rerun or relabelled9.

Coordinator established a new ci GUI test window through task `target/gui-launch.py` using isolation label `gui-rebased-20261019`. Copied `target/linux-validation/gui-rebased-20261019.json` binds source9 archive, root `/tmp/r823-gui-3nmehjb3`, PID15980, ci app SHA256 `f9dcd9c71c05ab9ec4ad412b06df79a9c46e66689eaf1a2b72507f5c969e4f22` and ci CLI SHA256 `d6db763fb4f69646454b14691d1b7175d7e3ff3402bc5ac0b36a48f468cd1e70`. This records launch identity only; it does not establish rendering or native interaction acceptance. Ubuntu is locked pending the user's unlock; crew does not capture the desktop or repeat the unlock request. Native input, shortcuts, clipboard, resize and the remaining selected GUI checks are still unverified. Old source5 PIDs and visual observations are not reused for this candidate. No Linux feature pass is claimed; x86_64, older-distro, WSL, real-agent and IME gaps remain.

Raw evidence archive: `target/linux-validation/checkpoints-8-9-evidence-20261019.tar.gz`, SHA256 `0f0ac1dce56e8edeeee84ba7600c66e58471de1bb08229978a5542847d8d47ac`; architect verified22 regular members and source9 metadata. Copied `checkpoint-8/9` log/result pairs, `gui-rebased-20261019.json`/`.log`, `source.json` and `vm-base-sync-20261019-012025.json` retain the per-candidate commands, source identities and new-base synchronization record. Source9 archive remains immutable; only this evidence document differs from its manifest. Reviewer raw-evidence audit and final evidence-record review completed at 2026-10-19 02:16 UTC with **NO REMAINING MUST-FIX ISSUES**. This covers source and audited evidence; native interaction and feature acceptance remain open.

The source9 release tarball contains ARM64 ELF machine183 executables: app SHA256 `2ebe608419ac39d5cacf175b5e2ff8caf35381a15ea9e86b24e319745ef4bf17`, and identical CLI/runnerd SHA256 `2ab530ba3c0ccec8bb5f7d962fced667391520ac24607abb97213c561ca52a07`. These were checked independently by reviewer and recomputed locally by impl. They are distinct from the ci GUI launch hashes above.

The retained new GUI startup log includes EGL `eglInitialize`/DRI2 warnings and ZINK device-selection failure, followed by successful Vulkan llvmpipe adapter configuration and selection. Wayland reports unsupported server-side decorations and falls back to client-side decorations. These are startup/environment diagnostics, not evidence of visible rendering or usable native input; the lock-screen and selected-interaction gaps remain open. No product change or repeated build was made in response to these diagnostics.

## Coordinator-observed GUI handoff, 2026-10-19 02:43 UTC

The user unlocked Ubuntu. Coordinator rechecked that PID15980 still belongs to the expected ci Runner executable and isolated `gui-rebased-20261019` root `/tmp/r823-gui-3nmehjb3`, and observed normal VM load. CUA observed the Runner landing window only within GNOME overview. Native input, clipboard and resize checks have not passed.

An attempt to focus the guest unexpectedly triggered UTM mouse capture. Coordinator immediately released it with Ctrl+Alt+Escape and independently confirmed Capture Input Value0. This is a test-environment interaction, not a classified product bug. Coordinator will not click the guest or enable capture again and has requested the user manually foreground Runner and choose New terminal. Current UI work awaits that handoff rather than screen unlock; earlier lock-screen events remain historical evidence.

The task helper `target/linux-validation/native-input-receiver.py` was copied into the owned isolation root as `/tmp/r823-gui-3nmehjb3/scratch/qa.py`, but has not been executed. Its presence is not a test or pass. This update records coordinator observations only: source9, package identities and all remaining native gaps are unchanged. Impl performed no new source change, build, test, VM access or app operation.

## Checkpoint10 Linux native window correction

At 2026-10-19 03:26 UTC the user reported that maximize works, restore leaves an oversized normal window, and none of the eight edge/corner drags resize it. Reported `stty size` tracking establishes only PTY geometry following available window size, not a native window-resize pass. Coordinator read the owned source9 root's saved normal frame as1440×900/maximized=true. That live root now contains user-created chats; crew does not operate or close those sessions. ScreenCaptureKit/TCC blocks fresh screenshots/accessibility observation. A new candidate must use a separate coordinator-owned launch/root or explicit manual handoff; old source9 PIDs and packages cannot prove the fix.

Diagnosis used only repository code and official0.3.7 archives downloaded into task `target/linux-window-source/` and checked against Cargo.lock. Core SHA256 `0e87a42bb37c7cb4e76dd1ac0ce88851e46e976e0373a47ab3e0757abffee54d`; Linux backend SHA256 `0aac7022e347409454777701082201742710052813964a1e25160f7e22c965ad`. Pinned source evidence:

- `gpui-pre/src/platform.rs` documents `appears_transparent` as macOS/Windows-only; Linux uses `window_decorations`. The official `examples/window_shadow.rs` explicitly handles client resize regions and calls `start_window_resize`.
- Linux Wayland `client.rs:1108` returns no primary display. Core `geometry.rs:740` uses only the requested/primary display in `Bounds::centered`; without one it returns the requested size. Runner passed a fresh1440×900 fallback, and secondary windows also used the unclamped default size.
- Wayland `window.rs:622` retains initial options.bounds. `handle_xdg_surface_event` around1082–1106 restores retained normal size when leaving maximize; `zoom` around1864 only requests set/unset maximized. The observed1440×900 normal frame follows this path. Interactive normal configure events update the retained normal bounds, so the fix leaves this native maximize/restore path intact.
- Wayland `start_window_resize` around2020 sends the edge with the latest MousePress serial. Runner source9 had no resize-region handler/request. Caption `WindowControlArea` alone does not provide Linux client resize edges.

Architect approved the two-part minimal plan before code edits. Linux now explicitly requests Client decorations, reserves6 logical pixels outside caption/content for eight native resize regions, removes them for actual server decorations/maximized/fullscreen state, and omits tiled edges. Left mouse-down on a resize region calls the native request and stops propagation; content and caption buttons retain their own input. Insets match GPUI's tiling-aware client geometry; no whole-window move handler or terminal-resize code was introduced. Shared desktop chrome only adds the Linux frame seam and a mutable window borrow; macOS/Windows geometry and controls are retained.

Fresh Linux main and secondary windows now use enumerated display visible logical bounds when no primary display exists; requested size is actually clamped and cascade origins are kept inside those bounds. Saved windows still use the existing restore helper; manually resized normal frames remain under GPUI's retained-bounds contract. No-display fallback is explicit. Pinned Wayland visible bounds equal the logical output and do not expose separate desktop panel/work-area reservations, so this fallback is documented rather than claimed as guaranteed usable work area.

Selected checks cover the changed behavior only: three new tests exercise missing-primary/enumerated/small/secondary bounds, eight directions and boundaries with no center/control hits, server/max/full/tiled state, and mouse event→resize-request routing with real content/caption handlers. Existing saved-size clamp, normal-frame snapshot and navigation tests cover directly affected restore/chrome contracts. The Linux policy/render helpers are compiled under test cfg on Mac to exercise the same code without a native app. Native acceptance still requires coordinator/user maximize→restore and eight-direction drag on the new executable.

Mac checks:

- Final `cargo test --locked -p runner-app --bin Runner --profile ci --features gpui_platform/runtime_shaders -- platform_ui::linux::tests window_state::tests::invalid_small_and_oversized_frames_restore_to_usable_bounds window_state::tests::maximized_and_fullscreen_snapshots_keep_the_normal_frame surfaces::app_shell::tests::window_navigation_stays_fixed_when_sidebar_animates`: exit0,6passed/0failed/0ignored, compile8.01s/tests0.18s, `target/linux-mac-checkpoint10-window-final.log`. Initial/expanded test attempts also passed and remain in `target/linux-mac-checkpoint10-window-tests.log` and `target/linux-mac-checkpoint10-window-retry.log`.
- Initial app strict Clippy exit101 found only a redundant `.into()` in a new debug selector (`target/linux-mac-checkpoint10-app-clippy.log`); removed it. Retry `cargo clippy --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings`: exit0 /8.16s, `target/linux-mac-checkpoint10-app-clippy-retry.log`. Only the existing upstream `block` future-incompatibility notice remains.
- Formatting and `git diff --check` passed. No full suite, native app/VM operation or release package rebuild was run by impl. Guide/architecture updates accompany the window behavior. Source9 package/native results retain their original identities. The new immutable handoff is `target/linux-checkpoint-10.tar.gz` plus complete manifest `target/linux-checkpoint-10.json`; Linux compilation/manual acceptance remain pending, not a native or feature pass.

## Coordinator-reported checkpoint10 Linux batch, 2026-10-19 05:27 UTC

Coordinator verified source10 archive SHA256 `f17cf5061266518c7ffd584d4b6416c14c04d5ded4369d3e64d1bfa04633c300`, all1557manifest entries, HEAD `1fff68095aa665c1981aabc3b33c0f56559a31fc` and dirty-source SHA256 `8bf1d1a20d114500cf11e36230b8b5c3e4c0be8d230f5c9146f6e914a19d04ea`. Architect relayed the following completed Linux results:

- Six focused window tests: exit0 /35.56s (compile34.36s, tests0.23s).
- Workspace all-target strict Clippy: exit0 /22.39s.
- Matching ci-profile app and CLI build: exit0 /26.01s.

The new isolation label is `gui-window-fix-checkpoint10`, root `/tmp/r823-gui-pmfe_x8w`, PID22297. Its ci app SHA256 is `8f114203571c1bdc7bcbaf90434f7775921e5d154577315e1764f946c05214f1`; ci CLI SHA256 remains `d6db763fb4f69646454b14691d1b7175d7e3ff3402bc5ac0b36a48f468cd1e70` because this change does not affect the CLI. The old source9 PID17481/root and user-created chats were not operated or closed.

Vulkan llvmpipe startup succeeded. The recorded normal layout is still1440×900; without a fresh visual or measured display geometry, this could reflect a legitimate output size or the documented no-display fallback. It establishes neither fit-screen behavior nor native resize acceptance. ScreenCaptureKit/TCC still blocks screenshot/accessibility inspection. User validation remains pending in the new empty window: shrink it, maximize, use the same middle caption button to restore, drag all eight edges/corners, and confirm caption/content input. Existing native gaps remain open.

Coordinator supplied evidence identity `target/linux-validation/checkpoint-10-evidence-20261019.tar.gz`, SHA256 `9f92ac4149f2f22240a399d379f53a6ee9d3902f9ee2d6dbd451c3904956918d`,9files. This entry records the coordinator/architect report; impl did not independently inspect those raw files during this record-only update. Source10 remains frozen, and source9 release packages are not relabelled as containing this fix. No new source candidate, build, test, release package, review loop or app/VM operation was initiated by impl. Further code work requires a concrete user/native failure report; no feature/native pass is claimed.

## User-reported window-fix confirmation, 2026-10-19 05:44 UTC

The user reported “没有问题了，已经测试” (“No problems now; tested”) for the reported window dragging/resizing and maximize/restore problem in the new source10 window `gui-window-fix-user-request-1`. Coordinator bound this handoff to PID22839, isolation root `/tmp/r823-gui-gao200p0`, ci app SHA256 `8f114203571c1bdc7bcbaf90434f7775921e5d154577315e1764f946c05214f1`, and source10 archive SHA256 `f17cf5061266518c7ffd584d4b6416c14c04d5ded4369d3e64d1bfa04633c300`.

This is the user's confirmation that the specific reported native window problem is resolved. It is not independent screenshot verification or a claim that all eight directions were individually checked. The earlier normal `stty size` result remains user-reported PTY geometry evidence. Clipboard, IME, real agents, x86_64, WSL and the broader Linux feature acceptance are not established by this feedback. Production code, existing packages and candidate identities remain unchanged; no new tests, archive, package rebuild, review cycle or old-session operation followed this record-only addition.


## Theme-test concurrency diagnosis and isolation, 2026-10-19

The remaining checkpoint7 `sidebar_and_mission_fills_follow_carbon_and_runner_light` failure is an existing test-isolation defect exposed by concurrent Linux testing. The original `checkpoint-7-final-tests.log` failure (expected Carbon, actual RunnerLight) and checkpoint8 isolated pass remain unchanged evidence. Before this repair, `theme.rs` and `theme_snapshot.rs` were byte-identical to upstream `4a5e5ecf` and source10; the `NativeRoot::new`/`render_app_shell` theme synchronization path was also unchanged by this branch. The original full-suite log cannot identify which concurrent test supplied the competing write.

`ACTIVE_VARIANT` and `GLASS` were shared atomics. `ThemeGuard` held a mutex used only by cooperating snapshot tests; ordinary Root fixtures did not acquire it. `NativeRoot::new` calls `sync_theme`, every app-shell render calls it again, and rendering calls `appearance::configure` to write glass state. Direct theme-setting snapshot tests already use a guard, including guards retained by the role/crew harnesses, but unguarded Root setup/render can still overwrite those guarded tests. Adding that same mutex to already guarded tests would not isolate these writers.

A deterministic regression now runs a guard-owning worker with Carbon/glass=true, releases a barrier, then constructs and renders a real unguarded Light/Solid `NativeRoot` on the test thread. A completion channel orders the guarded assertion after those real writes; there is no sleep or assertion of unprotected global values. Before the fix, the final regression failed with actual `(RunnerLight, false)` versus expected `(Carbon, true)`: exit101 /9.541s, one failed test /0.05s, `target/linux-theme-race-old-final.log` and `-result.json`. The exact red-run source is retained under `target/linux-theme-race-old-source/`; its file hashes match that result manifest. The unchanged regression source has SHA256 `913bed492d775d824939c70d599a626fd8da65f147010e1cf503db0eaad3bb24`. An earlier invocation used the wrong GPUI feature name; its feature-resolution failure is retained separately as `target/linux-theme-race-feature-error.log`, not counted as the reproduced race.

The repair changes only `theme.rs`, `theme_snapshot.rs` and the new app-shell regression relative to source10. Like the existing i18n testing API, an explicit thread-local scope is available under `cfg(any(test, debug_assertions))`, including binary tests linking the non-test library. Only `ThemeGuard`/explicit test scopes activate it. Variant and glass share this scope; the guard no longer writes restored values into global atomics or relies on the ineffective mutex. RAII restores nested state, including unwinding and returning the outermost scope to None; the scope cannot move across threads. Unscoped debug code continues to read/write the existing atomics, and release product builds exclude this testing override. Production settings/theme intent and the original Carbon expectation are unchanged.

Selected Mac checks cover the changed isolation and its actual rendering consumers, with GPUI runtime shaders:

- `cargo test --locked -p runner-app --bin Runner --profile ci --features gpui_platform/runtime_shaders -- theme_guard_isolates_unscoped_root_theme_and_material_updates sidebar_and_mission_fills_follow_carbon_and_runner_light work_card_insets_every_route_and_contains_chat_and_mission_columns`: exit0 /15.583s, 3passed/0failed/0ignored, tests0.14s; `target/linux-theme-race-fixed-bin.log`. The old red command uses the same options with only the first test filter and `--nocapture`.
- `cargo test --locked -p runner-app --lib --profile ci --features gpui_platform/runtime_shaders -- theme::tests`: final exit0 /4.265s, 6passed/0failed/0ignored, including glass behavior and nested/unwind/outer-scope restoration; `target/linux-theme-race-fixed-lib-final.log`.
- `cargo clippy --locked -p runner-app --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings`: final exit0 /4.521s; `target/linux-theme-race-fixed-clippy-final.log`. Format and diff checks passed. Only the existing upstream `block` future-incompatibility notice remains.

HEAD remains `1fff68095aa665c1981aabc3b33c0f56559a31fc`. Final SHA256 values are `eb0b436fa5e67a70d981b456d13bd6cc4c86952f69f2ca55bfa3d25a1a0db0b6` for `theme.rs`, `a6a4152a4f90877fad7a1281113f634d0b39cea02bddda69253643040603a93e` for `theme_snapshot.rs`, and `913bed492d775d824939c70d599a626fd8da65f147010e1cf503db0eaad3bb24` for `surfaces/app_shell.rs`. `target/linux-theme-race-fixed-source.json` binds these hashes and result files; source10's other 1553 manifest entries remain byte-identical, with only these three sources and this evidence record differing. Reviewer independently inspected the old/new source identities, real Root write chain and final logs and reported **NO REMAINING MUST-FIX ISSUES** within this theme-only scope. No new Linux/full-app/workspace run, native app/VM operation, package build, candidate archive, commit or publication was performed. These focused Mac results do not turn the historical checkpoint7 full-suite failure into a passing Linux full suite, and existing source9 packages/source10 native-window evidence retain their original identities.


## Coordinator OpenCode live evidence and user recovery report, 2026-10-19

Impl read `target/linux-validation/opencode-live-result.json`, the tool/error log, exact marker artifacts, archived mission summary and the evidence archive's message/status feeds. This bounded run used OpenCode 2.0.32 with `opencode/deepseek-v4.1-flash-free` on Ubuntu 26.04.1 ARM64 / UTM Wayland. Its source is checkpoint10 archive SHA256 `f17cf5061266518c7ffd584d4b6416c14c04d5ded4369d3e64d1bfa04633c300`, ci app SHA256 `8f114203571c1bdc7bcbaf90434f7775921e5d154577315e1764f946c05214f1` and CLI SHA256 `d6db763fb4f69646454b14691d1b7175d7e3ff3402bc5ac0b36a48f468cd1e70`. It predates the theme-test scope repair and does not validate a new final source candidate.

- **Initial simultaneous startup failed.** The worker exited1 with `SQLiteError: database is locked`; its standalone service exited before reporting readiness. The lead started. The original error remains in `opencode-live-tool-and-error-evidence.log`; a later resume is not an initial concurrent-start pass. Whether this is also reproducible with independent OpenCode CLIs remains pending coordinator comparison.
- **Tools passed after recovery.** Resuming the worker succeeded. Real agents wrote/read the exact owned-scratch markers in `lead.txt`, `worker.txt` and `reply.txt`, corroborated by `opencode-live-artifacts.json` and tool logs. This is bounded shell/file-tool evidence, not a general permission-dialog pass.
- **Crew messages and completion passed after recovery.** The feeds show directed lead PING, worker `inbox_read`, worker PONG, lead `inbox_read`, lead ACK and worker `inbox_read`. Both final session statuses report idle with completed outcome. Feed1/feed2/final/complete and lead-final/worker-complete logs in the archive support this sequence; final stopped status came from cleanup, not a crash or proof of interruption.
- **Automated interruption remained blocked.** The native-control pipe could not start. No UI input/current-turn interrupt was attempted by that route; stopping the mission during cleanup does not count as a foreground interrupt.
- **Manual interrupt/recovery is user-reported passed.** After the separate foreground `sleep 30`, native interrupt and `RECOVERED` follow-up instructions, the user confirmed “是可以的” (“it works”). The current result JSON records this in `user_manual_verification`. There is no independent screenshot or structured manual-check status; the automated-control blocker above remains intact.

The original evidence archive `target/linux-validation/opencode-live-evidence.tar.gz` has SHA256 `deb5cbc6b7b16c210aee3c381187cbd783deda9dc47a03b6f0db381d5e773731`. Its internal result predates the user-manual addition, so the current external result JSON is the latest summary and is not claimed byte-identical to the archived copy. Raw logs stay local; no session links or transcripts are copied into the delivery/PR draft.

The coordinator used fresh HOME/XDG directories and scratch without copying credentials. Mission-level permission resolved to bypass (`--auto`) despite the role default; only harmless owned scratch tools were requested, so interactive permission prompts remain unverified. The test mission was archived, the owned app received SIGTERM and daemon stop exited0. Impl only read these captured files and updated local documentation; no new app/VM operation, broad check, package rebuild, commit, push or PR followed this recording step. Final Linux/Mac gates, rebuilt packages and a short actual-package check are pending coordinator execution against the next frozen source; source9 packages still lack the window fix, and the theme isolation repair currently has only Mac focused validation.


## Final source11 gates, rebuilt packages and remaining limits, 2026-10-19

The coordinator ran the final automated gates against frozen source11: HEAD `1fff68095aa665c1981aabc3b33c0f56559a31fc`, dirty-source SHA256 `dd55df48b51107d65dfe7a4b87fccf642d7106d153129c3aae0901dabfc42ee9`, source archive SHA256 `393078239b6b98a3f9d9ba609e8cc9e2891648a96cdbdd90e50df6358a147b2b`, manifest `target/linux-checkpoint-11.json`, 1557 source entries. Impl read the result JSON and logs below and matched the VM evidence's complete source list to that manifest. The final evidence archive `target/linux-validation/final11-evidence-20261019.tar.gz` has SHA256 `abe1ba15fafd7aa21bbe3730bab7b5c95a004ed6b72381a68f20d6347e480012`. Aggregate metadata is in `target/linux-validation/final11-handoff-summary.json`.

Mac raw files are under `target/linux-validation/`; Linux raw files are under `target/linux-validation/final11-evidence/`. Each stem below names its `.log` and `-result.json` pair. Times are result-JSON wall times, not only libtest execution time. These are completed coordinator runs; impl did not rerun them during this documentation update.

| Mac check | Exact command | Exit / wall time | Evidence stem |
| --- | --- | --- | --- |
| Format | `cargo fmt --all --check` | 0 / 2.35s | `final11-mac-fmt` |
| Workspace strict Clippy | `cargo clippy --locked --workspace --all-targets --profile ci --features gpui_platform/runtime_shaders -- -D warnings` | 0 / 20.30s | `final11-mac-clippy` |
| App updater strict Clippy | `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater,gpui_platform/runtime_shaders -- -D warnings` | 0 / 10.21s | `final11-mac-updater-clippy` |
| Workspace tests | `cargo test --locked --workspace --profile ci --features gpui_platform/runtime_shaders --no-fail-fast` | 0 / 275.80s | `final11-mac-tests` |
| CLI build | `cargo build --locked -p runner-cli --profile ci` | 0 / 14.42s | `final11-mac-cli-build` |
| CLI guide check | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 / 0.91s | `final11-mac-cli-guide` |

Every Linux command below was prefixed with `env LC_ALL=C.UTF-8 LANG=C.UTF-8 PATH=/home/lr/runner/.worktrees/feat-823-linux-support/target/linux-validation/tools/node-v22.14.0-linux-arm64/bin:/home/lr/.cargo/bin:/usr/bin:/bin`. Only the lifecycle retry used the corrected prefix `env LC_ALL=C.UTF-8 LANG=C.UTF-8 PATH=/home/lr/runner/.worktrees/feat-823-linux-support/target/linux-validation/tools/node-v22.14.0-linux-arm64/bin:/home/lr/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin`. The selected task-private Node supports the existing pi protocol fixture without an agent account.

| Linux ARM64 check | Command after environment prefix | Exit / wall time | Evidence stem |
| --- | --- | --- | --- |
| Format | `cargo fmt --all --check` | 0 / 4.86s | `final11-linux-fmt` |
| Workspace strict Clippy | `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 / 55.34s | `final11-linux-clippy` |
| Workspace tests | `cargo test --locked --workspace --profile ci --no-fail-fast` | 0 / 326.43s | `final11-linux-tests` |
| CLI build | `cargo build --locked -p runner-cli --profile ci` | 0 / 20.04s | `final11-linux-cli-build` |
| CLI guide check | `python3 script/generate-cli-reference.py --runner target/ci/runner-agent-cli --check` | 0 / 0.10s | `final11-linux-cli-guide` |
| Production package build | `python3 script/bundle-linux --channel production --arch aarch64 --output-dir target/linux-packages/final11-20261019` | 0 / 287.92s | `final11-linux-packages` |
| Initial actual-package lifecycle | `python3 script/linux/test-packaging.py --packages target/linux-packages/final11-20261019` | 1 / 1.95s | `final11-linux-package-lifecycle` |
| Actual-package lifecycle retry | `python3 script/linux/test-packaging.py --packages target/linux-packages/final11-20261019` | 0 / 2.65s | `final11-linux-package-lifecycle-retry` |

Both workspace test commands exited0, and neither raw log contains a FAILED result. The logs include nested child-process test summaries, so summing all summary lines would double-count some cases; no unique test total is claimed. Both explicitly passed the barrier-based real-Root theme regression, nested/unwind scope restoration and the original mission Carbon/RunnerLight test. These source11 results validate the repair under concurrent workspace tests; they do not rewrite the retained source7 failures or turn source8's earlier isolated pass into repair evidence. Existing ignored cases remain ignored, not passed. Mac still emits the existing upstream `block` future-incompatibility notice.

The first actual-package lifecycle command failed with script exit1 /1.95s. Its captured `dpkg --install` exited2 because the coordinator's restricted PATH could not find `ldconfig` and `start-stop-daemon`; the raw stderr explicitly requests the sbin directories. The command was retried against the same source and package bytes with only the corrected PATH above: exit0 /2.65s. Both logs/results remain intact. No product or test-source change was made to obtain this pass. The helper covers isolated tar/deb headless first launch, reinstall, ownership refusal, uninstall/remove/purge with preserved data; RPM coverage is metadata/layout, not native installation on an RPM distro. The opt-in systemd live check remains the earlier source4 fixture evidence and is not relabelled as a source11 native service run.

Final source11 packages live under `target/linux-packages/final11-20261019/`:

| Package | Bytes | SHA256 |
| --- | --- | --- |
| `Runner-0.16.5-linux-aarch64.deb` | 28042354 | `6e8a132d16d2154cb16bf86e1c3c81852361c7150a3e593e1bc08a8b1d161836` |
| `Runner-0.16.5-linux-aarch64.rpm` | 28054622 | `81315a1c5398969607c54b7f28a5fe68c1f5a20034b746a1ea6615db0a2101d6` |
| `Runner-0.16.5-linux-aarch64.tar.gz` | 43531670 | `a69e6279807fba66d341ce8b7b60b57195939ce0e0ad14f375851dc336eea042` |

The authoritative identity is `target/linux-validation/final11-evidence/final11-package-identity.json`; its identical mirror and `SHA256SUMS` are now in the package directory. Impl independently read/hash-verified all three local packages. In the tar, `Runner` is an ARM64 ELF (machine183), SHA256 `c2e6301d497bf31d54c7fa2eb94761b55728ce7dd991ea3c9c2ba49ac57ac1d4`; `runner-agent-cli` and the real regular-file `runnerd` both have SHA256 `2ab530ba3c0ccec8bb5f7d962fced667391520ac24607abb97213c561ca52a07`. These match the coordinator's release payload identities. Tar `build-platform.txt` also matches the identity. These are newly rebuilt source11 packages containing the window repair; older source9 packages remain historical and are not substituted for them.

These local packages were built on **Ubuntu 26.04.1 ARM64 and require glibc >=2.43**. The Ubuntu22.04 baseline exists in the CI/release definitions but was not executed here; these binaries do not establish Ubuntu22/24 compatibility. x86_64 execution, Windows regression execution, hosted CI, WSL, native RPM lifecycle and the remaining native input/IME checks are unverified. No new source11 native GUI smoke or screenshot verification was performed merely by rebuilding packages. Window resize/restore and foreground interrupt/recovery retain the earlier user-reported passes; the coordinator additionally reports the user's copy/paste check working. This clipboard evidence is user-reported, not independent automation or acceptance of the rebuilt source11 app. Zoom remains unconfirmed.

Two bounded independent OpenCode controls used the same binary SHA256 `4199ae2d2c96ad2a140d32cabe6a1d9280b4cd1d9d8ed533a5431f54c422693c` without Runner daemon/hooks/plugin. Both used fresh shared HOME/XDG and model warmup (exit0). Two simultaneous `opencode --standalone` starts were observed for11.15s; two simultaneous `opencode --auto --standalone --prompt 'Reply exactly DIRECT_READY. Do not use tools.'` starts were observed for10.92s. Both pairs remained live until owned cleanup, with no logged SQLite lock (`opencode-direct-concurrency-result.json` and `opencode-direct-prompt-result.json`, plus associated logs in the final evidence directory). These short controls did not reproduce the initial failure and do not determine its cause. Reviewer found no concrete must-fix defect attributable to this Linux diff; the known live concurrent-start limitation remains **unattributed**, with the original SQLite failure and successful resume retained. Runner integration is not excluded, OpenCode is not assigned blame, and no fix is claimed.

Architect and reviewer independently verified the complete source11 archive/manifest and entire working-tree review concluded **NO REMAINING MUST-FIX ISSUES** before these final evidence-only updates. This authorized update changes only this record and `docs/impls/823-linux-support.md`; production/test files and the immutable source11 archive remain unchanged. The working-tree documentation now postdates source11's archived documentation, so its recorded dirty identity describes the tested snapshot, not a freshly recomputed hash of this final docs-only tree. No new source snapshot, build, test, package install, app/VM operation, commit, push or PR is initiated by impl; the coordinator retains mission-stop responsibility and the branch/worktree is preserved.
