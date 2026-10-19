# Linux desktop support — local Build squad mission

## Goal and current authority

The human requested a usable Linux build of Runner, including the native GPUI app,
daemon and CLI, with installable packages. Use **Build squad**: `architect` leads
planning and acceptance, `impl` implements and fixes, `reviewer` independently
reviews. Deliver the reviewed implementation locally; **do not push or open a PR**.

Related issue: https://github.com/yicheng47/runner/issues/823. The upstream brief
`docs/impls/briefs/823-linux.md` is historical input, not this mission's authority:
its maintainer-specific paths, absence of a VM, x86_64-only scope and publication
authorization do not apply. Work from the current code and callers at base
`0a65b80d2efa57a45b9f8f1bc30031f644042aa1` (Runner 0.16.2), including current
runtime adapters, runnerd, GPUI, empty-mission setup and guide conventions.

On 2026-10-19 the human requested rebasing before further work. The coordinator
rebased the local brief commit onto upstream `4a5e5ecf` (Runner 0.16.5), producing
HEAD `1fff68095aa665c1981aabc3b33c0f56559a31fc`, and restored the preserved feature
stash. This is the resumed development base; earlier snapshot identities and
validation remain historical evidence, not results for the rebased candidate.
The local-only authorization and coordinator-owned VM/app boundary are unchanged.

On 2026-10-19 the user subsequently authorized committing the reviewed feature
locally (`e4143765`). On 2026-10-20 the user requested a new upstream review and
rebase onto `a9d3b290` (Runner 0.16.6), including necessary compatibility fixes
folded into that local feature commit. These later requests authorize local
commit/rebase operations; no push, PR, release or worktree removal is authorized.
The completed Build squad sessions stay stopped during this follow-up review.

Later on 2026-10-20, the user authorized pushing the branch and opening a draft
PR in `ruizer/runner` to exercise the fork's CI. That authorization covers the
necessary CI corrections and the subsequent requested rebase onto upstream
`f6ab9ef6` (Runner 0.16.7), folded into the feature commit and pushed with a lease.
It does not authorize an upstream PR, merge, release or local cleanup. Earlier
local-only restrictions above describe the mission's original scope and are
superseded only by these later explicit requests. Crew sessions remain stopped.

## Workspaces and environment

- Mac branch: `feat/823-linux-support`.
- Mac worktree: `/Users/lr/我的/runner/.worktrees/feat-823-linux-support`.
- Mac repository root remains on `main`. Do not modify another worktree or project.
- Ubuntu host: `lr@192.168.64.3`, Ubuntu 26.04.1 ARM64 in UTM.
- Its repository root `/home/lr/runner` stays on `main`; the coordinator prepares
  `/home/lr/runner/.worktrees/feat-823-linux-support` for this mission's builds.
- The Mac worktree is the source of truth. Linux receives deliberate source
  snapshots into its task worktree, retaining its own `.git` and `target/`.
  No shared Cargo target directory, no concurrent writer to either worktree,
  and no build from a partially synchronized snapshot.
- Ubuntu has Rust 1.97.1, Clippy, rustfmt, cc, pkg-config, git, curl, OpenSSL
  headers, rsync and vulkan-tools. The coordinator verified Rust linking and a
  120-frame Wayland `vkcube` run. Vulkan currently uses Mesa llvmpipe (CPU).
  This is an environment probe, not a Runner GUI or agent acceptance pass.
- SSH public-key authentication works. Use existing credentials; never copy
  private SSH keys or agent/account credentials. The project-local known-hosts
  file is `/Users/lr/我的/runner/target/linux-dev-known-hosts`.
- VM: 4 virtual CPUs, about 5.3 GiB RAM plus 4 GiB swap, initially 47 GiB free.
  Use `CARGO_BUILD_JOBS=2`, the `ci` profile and one Linux build at a time.
- Ubuntu sudo requires a human password. Do not request/log that password,
  change sudo policy or repeatedly trigger interactive sudo. Report one exact
  missing-dependency command to the coordinator for the human to run.
- No Xcode install, macOS upgrade, dependency/toolchain bump, release/nightly
  dispatch, external website edit or global agent configuration change.

## Deliverables

1. **Current spec and plan.** Inventory present platform gates before editing.
   Have `impl` write `docs/features/823-linux-support.md` and a focused plan,
   naming affected user-guide pages and documented architectural decisions.
   Link #823 as related work; do not edit its labels, milestone or status.
2. **Linux headless behavior.** Build and exercise runner-core, runner-terminal,
   runner-daemon and runner-cli on Linux. Implement necessary `/proc` descendant
   cleanup and exact process-assisted conversation lookup using current seams;
   cover failure/timeout/ownership cases without guessing newest sessions.
   Preserve IPC bounds, launch definitions, shutdown and surviving sessions.
3. **Linux GUI.** Add proper Linux GPUI dependencies and platform selection for
   X11/Wayland, fontconfig/CJK fallbacks, appropriate window chrome and Ctrl
   bindings, native folder/link opening, clipboard and notifications behavior.
   Audit macOS/Windows `cfg` assumptions individually. Reuse established shared
   UI where appropriate; avoid unrelated abstractions or redesign. No `.pen`
   changes. The initial app should run in the UTM ARM64 desktop. Existing Mac
   and Windows behavior must be retained. Unsupported platform facilities are
   explicitly gated and documented; no fake success or placeholder readiness.
4. **Architectures and compatibility.** Support aarch64/ARM64 and x86_64 Linux.
   Native validation uses this ARM64 VM. Add x86_64 Linux CI/build coverage and
   an ARM64 build path using supported runners/containers. Define actual distro,
   glibc and graphics prerequisites. Prefer an older reproducible release build
   baseline where feasible; an Ubuntu 26 build cannot establish compatibility
   with Ubuntu 22/24. Do not claim unexecuted x86_64 or WSL acceptance.
5. **Installable packages.** Produce architecture-labelled `.tar.gz` plus `.deb`
   and `.rpm` from a consistent staging layout. Include the app, CLI sidecar,
   desktop entry, icon and an opt-in systemd user unit. Install/uninstall should
   preserve user data. A headless install must expose a working CLI and daemon
   without requiring a prior GUI launch. Make CLI symlink ownership and upgrade
   behavior fit the existing installer. No AppImage/Flatpak/Snap expansion.
6. **Systemd and upgrades.** Keep daemon bytes consistent with app sidecar hash
   checks. The unit must coexist with app-launched daemons, endpoint ownership,
   upgrades and restart-on-mismatch without duplicate daemons/restart loops.
   Explain chosen restart/exit policies. Installation does not silently enable
   a persistent service. Preserve login-shell executable discovery under a
   minimal service environment and explicit SSH no-auto-start policy.
7. **CI/release definitions and docs.** Add Linux checks and package verification
   to CI and Linux build outputs to release/nightly definitions without changing
   Mac/Windows feed semantics or publishing anything. Document WSL2/WSLg setup,
   service lifetime and known gaps using current official docs. Update relevant
   guides, platform architecture docs, development commands and both READMEs.
   Preserve honest current download availability: packages built locally are
   not already publicly released. Regenerate CLI guide if help changes.

## Validation and evidence

Before execution, list changed behaviors, selected cases from the live regression
suite and feature checks, with reasons. Start with compilation and exact affected
unit/process/packaging cases; iterate locally with narrow filters. Do not repeatedly
run broad suites while implementing. A new platform changes compilation and process
contracts across all crates, so final Linux workspace Clippy and automated tests
are an appropriate platform gate once the build is ready. Mac checks cover shared
edits and platform gating; do not repeat unrelated full smoke matrices. Use runtime
shaders for Mac GPUI compilation; do not install Xcode. Record missing Windows,
x86_64, WSL and graphics prerequisites honestly rather than treating them as passes.

Create `docs/tests/823-linux-support.md`, retaining initial failures, exact commands,
candidate commit plus dirty-source identity, results and remaining limitations.
Cover synthetic PTY/IPC/session lifecycle, process cleanup, platform shell/path
quoting, package layout, install/upgrade/uninstall preservation and the systemd
single-daemon contract. Verify the actual ARM64 tarball/deb installation in isolated
prefixes/data. Validate rpm metadata/layout even if no RPM distro is available.

The coordinator owns native GUI/live checks and source synchronization. Coders and
reviewers do not run the app. They send Linux dependency/build/check requests and
candidate identity to the coordinator through this mission. Use development data,
follow the repository smoke-test isolation and evidence rules for selected cases,
and never touch installed Runner sessions or unrelated dev sessions. Bounded real
agent checks may use existing sign-ins under standing development authorization;
this VM has no established agent sign-in, so do not copy credentials, log in or
invoke paid inference. Synthetic fixtures and any blocked native checks are labelled.
Only archive/stop test sessions created for this mission. A Vulkan cube pass does
not prove Runner UI, terminal interaction, IME, agent hooks or a crew round trip.

## Coordination and review loop

`architect` reads the brief and current code, posts a concise plan/acceptance split,
then explicitly dispatches implementation to `impl`. It stays responsible for
scope and final audit. `impl` is the sole production-code writer and owns relevant
automated checks, documentation, follow-up fixes and packaging. `reviewer` waits
for an explicit handoff, then reads the entire working-tree diff and affected
callers without editing. Review Linux behavior, Mac/Windows cfg preservation,
ownership/timeout cleanup, service/install/upgrade safety, packaging, architecture
labels, guides and truthfulness of evidence. Iterate until the reviewer explicitly
states `NO REMAINING MUST-FIX ISSUES`. Read and acknowledge human instructions.

Post milestones, blockers and final handoff in the Runner mission channel, so the
human can see progress. Request coordinator actions with exact commands, paths and
scope; do not stall silently. Do not start additional agents/crews/subagents or
independent missions. All three crew roles already exist in this Build squad.

## Authorization and stopping point

The human authorized Build squad implementation with **no PR for now**. This
overrides the repository's normal mission-publication default. The coordinator
may create the linked worktree and the required local brief setup commit after
verifying explicit Git author name/email. This is the only preparation commit;
leave feature edits uncommitted for human review. Do not push, create a PR, merge,
tag, publish, alter Git identity, or delete branches/worktrees. If code commits
are later authorized, fold this brief into focused implementation commits, verify
explicit author identity again, and omit co-author trailers unless requested.

Final local handoff: branch/worktree, architecture/platform scope, packages and
how to run them, exact verification results, clean-review statement, real native
evidence and remaining gaps. Stop coding sessions after that handoff while keeping
the worktree and test evidence. No PR has been requested or submitted.
