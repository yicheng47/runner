# Linux tar/deb upgrade acceptance — 2026-10-21

Final result: selected tar upgrade and deb authorization-cancel/retry/upgrade/
relaunch checks passed. Cleanup passed. See the dated steps and limits below.

Candidate: `61ad55f7ca7a850c331476f292478e927e592df7`. The user authorized
pushing this commit and testing tar/deb upgrades on the existing Ubuntu ARM64 VM.
No production signing key, release dispatch or unrelated runtime smoke is selected.

## Scope selected before execution

- SET-UPDATE-03/Linux: real local HTTP feed, download, signature verification and
  retained ready state. Test builds change only compiled-in feed URLs and public
  key; private test key stays on the Mac. Never disable verification.
- SET-UPDATE-04/Linux/tar: distinct older/newer stamped app packages, user-owned
  install directory, atomic replacement, actual relaunch and isolated data canary.
- SET-UPDATE-04/Linux/deb: temporary runner system package (initial inventory has
  no installed runner package), desktop polkit cancellation/retry, real APT upgrade,
  actual relaunch and isolated data. No changes to existing Runner user data.
- Record executable/package identities, observed native UI and cleanup of owned
  processes/temporary installation separately from automated checks.

CI: fork PR1, run39535301364, all four jobs succeeded for the candidate commit.
Tar/deb live checks are pending. RPM, WSL and x86_64 desktop are outside this run.

## Test fixtures and tar result

Two native release builds used the official bundle script, nightly channel and
only a compile-time local feed/test public key substitution in `updater/linux.rs`.
The source file was restored after building. The production trust key and
Windows sources were not changed. The test SHA labels distinguish the fixture
binaries; the source candidate remains commit61ad55f7.

- Older fixture: `nightly.61ad55f.20261021.0001`, build exit0 /319.83s.
- Newer fixture: `nightly.61ad560.20261021.0002`, build exit0 /240.88s.
- Local server listens only on `127.0.0.1:18423`. Native HTTP evidence records
  release metadata, tar bytes and detached signature requests, all HTTP200.
- SET-UPDATE-03/Linux/tar: Passed for real local download/verification and staged
  ready candidate. Cancellation/tampering remain covered by prior unit fixtures,
  not newly claimed as native GUI checks.
- SET-UPDATE-04/Linux/tar: Passed for owned empty-data installation. User clicked
  Install and restart and reported completion. CUA observed the reopened window
  without the update indicator. Old PID212109 exited; new PID212301 appeared.
  All three installed executable hashes match the newer signed tar payload.
  The isolated data canary and English setting survived. No live-agent/session
  history migration was selected or established by this empty-data test.

Old Runner SHA256: `8cbbcbb2bc3bbcbc5c15eda75292d1051075bb428fd965b4cca548272a3bc242`.
New Runner SHA256: `de97c966d2f2711c27fab97e6761220e46d49fbf53d4322f7af7149940569ddc`.
New CLI/runnerd SHA256: `db70259dcf7aeb5805fb702de79fdc3c95ba322e5f3e09d218827e440eeb690e`.
Raw identity: `target/linux-validation/updater-e2e/tar-upgrade-result.json` on VM.
Tar test GUI PID212301 was stopped after verification; isolated data is retained.

## Deb preflight failure and correction

Installing the older temporary deb from a desktop terminal succeeded, but the
updater downloaded a tar. Native inspection established that the package carried
`/usr/lib/runner` mode0775 from the builder's umask0002, although it was root-owned.
The strict system-installer path check correctly refused group write permission;
release availability's source-build fallback selected tar. No APT upgrade was
claimed for this failed candidate. Its GUI PID212831 was stopped and launch
identity retained as `deb-before-permissions-fix-launch.json`.

The production fix changes only `script/bundle-linux`: normalize archive and
native package directory permissions to0755 and regular data files to0644, keeping
executables0755. The new packaging regression drives the real staging/archive
code under umask0002 with fake ELF build inputs and inspects tar members plus
the dpkg build input. Before the fix it fails on writable package/usr; after
the fix it passes. Mac suite:18 tests,2 Linux-tool skips. Ubuntu suite passed.
No additional Rust/workspace test run is selected for this packaging-only fix.

Old/new fixture packages were repacked through the same normalization function,
with the exact original binary bytes and labels preserved; original packages
and signatures remain in `original-old/` and `original-new/`. New signatures were
generated with the same test key. `permissions-repack-result.json` binds their
hashes and0755 application directories. Deb cancellation/install/relaunch is
pending a fresh installation of the corrected temporary older fixture.

SSH preparation attempts failed with “dead parents” and then “No authentication
agent found”; moving setup to the existing Ubuntu desktop terminal reached normal
sudo authentication. These are fixture setup outcomes, not updater authorization
results. The user enters passwords directly; no credentials are captured.

The packaging fix and this live evidence are local/uncommitted and are not covered
by the successful CI run for61ad55f7. No follow-up push has been performed.

## Corrected deb installation — native retry

The user completed setup in the reopened desktop terminal. The corrected older
package installed successfully (APT exit0), and native inventory now shows
root-owned `/usr/lib/runner` mode0755. The fresh isolated root is
`/tmp/r823-up-deb-xyk6mxe5`, GUI PID214548, release
`0.16.7~nightly.20261021.0001.61ad55f`. The local HTTP log now records the
`.deb` package and its `.deb.sig` (HTTP200), rather than the earlier tar fallback.
CUA observed the fresh Runner window and update indicator. Cancellation/retry
and successful authorization/install/relaunch are still pending.

The user then reported cancellation with Runner still running. Read-only
verification confirmed PID214548 and the old binary/package/data canary unchanged,
but no `linux-install.log` or attempt marker existed in this isolation root. The
CUA screenshot showed the main screen, not an installation error. This does not
yet prove cancellation of polkit authorization; clarification of the cancelled
dialog is pending. `deb-cancel-result.json` preserves this observation.

## Deb authorization cancellation

The user clarified that the first cancellation was Runner's update dialog, then
repeated the action and cancelled the actual Ubuntu password authorization.
SET-UPDATE-04/Linux/deb/cancel: Passed. `deb-polkit-cancel-result.json` records
PID214548 alive, unchanged old binary/package and retained canary. The install log
contains `Request dismissed` with exit126 and a retained attempt marker. CUA
observed Runner's installation-failed dialog with installed Nightly61ad55f and
the Install and restart retry button. Successful retry is pending user action.

## Successful deb retry

SET-UPDATE-04/Linux/deb/install/relaunch: Passed for the isolated empty-data
installation. The user retried Install and restart, authorized the Ubuntu system
prompt and reported automatic restart. `deb-upgrade-result.json` records:

- APT upgraded only `runner` from
  `0.16.7~nightly.20261021.0001.61ad55f` to
  `0.16.7~nightly.20261021.0002.61ad560`; no packages removed or newly installed.
- PID214548 exited; PID215214 ran `/usr/lib/runner/Runner` as uid1000.
- Installed Runner/CLI/runnerd hashes match the newer signed payload identities
  listed above. The installed directory remains root-owned0755.
- The test canary and English setting survived. CUA observed the reopened window
  without an update indicator. No root GUI/agent was started by installation.

Both requested tar/deb upgrade paths have native evidence with real fixture
packages and test signatures. They are not evidence of official production
signing/publishing, stable semantic-version upgrade, RPM, x86_64 desktop or live
agent/session-history migration. Those variants were not selected for this run.

Cleanup has been started: stop owned GUI/daemon/feed processes and remove only
the exact temporary runner system package, preserving all canaries/evidence.

Local native evidence archive: `target/linux-validation/updater-e2e/native-upgrades-evidence.tar.gz`, SHA256 `e792373af9d0ab9d7d3bada6a01100cd01ad6cfb3302950e1021ab827e1cc62e`. It contains17 whitelisted source-identity/result/log files; no private key or real account data.

## Cleanup completed and commit authorization

The user authorized the cleanup password prompt. APT remove exited0; the
temporary `/usr/lib/runner/Runner` and all three `/usr/bin` aliases are absent.
Only dpkg's removed-package/config-files metadata remains; no purge or autoremove
was requested. The local feed, owned GUI and all three owned daemon processes
were stopped. All three test canaries remain intact. Exact process identities and
results are in `cleanup-result.json`, with APT output in `cleanup-apt.log`; copies
are retained under the Mac worktree's ignored evidence folder.

The user subsequently requested a local commit of the packaging permission fix,
its regression test and this live record. No additional push or release dispatch
is requested. The earlier all-green hosted CI covers commit61ad55f7; a new CI run
will be needed when the permission-fix commit is pushed.
