# 856 — Migrate the workspace to Rust edition 2024

## Starting point

- Issue: [#856](https://github.com/yicheng47/runner/issues/856).
- Branch: `chore/856-rust-edition-2024`.
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/chore-856-rust-edition-2024`, with its own `target/`.
- Base: `36d22b0d41817130369070dffece3d6c95ca4aee` from `origin/main`.
- The separate #1290 mission is working in `feat/1290-archive-linked-missions`. Keep this migration isolated and call out overlapping files and landing order in the final handoff. The issue recommends landing between active branches; this mission prepares an open PR for Jason to coordinate.

## Outcome

Move all five workspace crates from edition 2021 to 2024 using the installed, pinned Rust 1.97.1 toolchain. Preserve runtime behavior and the 2021 formatting style. No Pencil work or live app testing is required for the intended source/tooling-only diff.

## Read first

- `AGENTS.md`, issue #856, `Cargo.toml`, `rust-toolchain.toml`, crate manifests, `Makefile`, and the platform CI workflows.
- `crates/runner-core/src/event_log/ulid.rs`: identifiers named `gen`.
- `crates/runner-cli/src/main.rs`: isolated-daemon environment setup before `runner_daemon::daemon::server::run`.
- `crates/runner-core/src/daemon_process.rs`: `clean_environment`, its callers, and `daemon_cleanup_preserves_recording_without_mission_context`.
- `crates/runner-daemon/src/daemon/server.rs`: environment cleanup, logging, boot and runtime initialization order.
- Official [edition migration guide](https://doc.rust-lang.org/edition-guide/editions/transitioning-an-existing-project-to-a-new-edition.html) and [newly unsafe functions](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html).

## Deliverables and boundaries

1. Run `cargo fix --edition --workspace --all-targets` while the workspace still declares edition 2021, review every fix, then set `[workspace.package].edition = "2024"`. Inspect platform-gated code as well as the host build. Run the Windows migration check through an existing authorized Windows setup if available; otherwise disclose that native `cargo fix` was unavailable and distinguish this from Windows CI compilation/tests.
2. Rename reserved `gen` identifiers descriptively instead of retaining `r#gen`. Audit environment mutation safety and make the required minimal changes; an automatically inserted `unsafe` block alone is not evidence that the preconditions hold. Check all callers, including subprocess tests, and state the safety invariant in review. Review temporary-drop changes, match ergonomics and return-position `impl Trait` lifetime capture. Keep explicit `use<...>` bounds only where necessary to preserve intended behavior.
3. Add `style_edition = "2021"` in `rustfmt.toml` (there is no existing Rustfmt configuration). Keep the toolchain, dependency versions, resolver and public behavior unchanged; avoid unrelated let-chain rewrites, cleanup or broad formatting.
4. Fold this brief into the implementation history. No user-guide changes are expected because commands, settings and screens are unchanged. If the final diff changes observable behavior, disclose it and reassess validation rather than treating it as a mechanical migration.

## Validation and review

Before testing, list the changes and selected checks with their reasons, as Test Scope requires.

- Run `make verify` in this worktree: workspace compilation and tests cover edition-dependent typing and lifetimes, Clippy covers migration fallout, and formatting checks prove the style pin works. Include the updater-feature checks required on macOS.
- Review the existing environment-cleanup subprocess test and isolated-daemon startup coverage against the actual mutation changes; add a focused regression only if an uncovered behavior needs it. Do not launch the desktop app or operate existing sessions.
- Require the PR's platform CI to pass, including macOS, Windows and both Linux architectures configured by the repository. Handle platform-specific migration fixes through the same independent review loop.
- The coder hands the reviewer an unpushed candidate SHA with the diff, checks and any native-platform gaps. The reviewer audits behavior preservation, temporary lifetimes, environment safety and formatting scope, and explicitly records why no live checks are needed on the final candidate. If review finds an actual behavior change needing live checks, report it to the operator before claiming completion.
- The PR links #856, records the validation and any gaps, and uses `no-behavior-change` if the diff remains behavior-preserving, exactly one appropriate risk label, and any automatically required `core-review` label. Independent approval and the `reviewed` label must name the current head.

## Authorization

Jason requested the codex duo (coder and reviewer) on 2026-11-13 after confirming that live testing is unnecessary for this migration. The crew is authorized to implement #856 on the named branch, commit, push, and open a pull request against `main`.
