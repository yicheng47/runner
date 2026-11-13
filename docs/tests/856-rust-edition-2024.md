# #856 — Rust edition 2024 validation

Candidate: branch `chore/856-rust-edition-2024`, based on `36d22b0d41817130369070dffece3d6c95ca4aee`. Toolchain: pinned Rust 1.97.1 on macOS aarch64.

## Selected checks

- `cargo fix --edition --workspace --all-targets` before changing the edition: identify source incompatibilities and temporary/lifetime changes.
- `make verify`: all five crates change language edition, so workspace compilation, tests, Clippy (including macOS updater), and the 2021-style formatting check are required.
- Cleanup regression and isolated-daemon integration tests: verify process environment cleanup, fixture recording, and startup after the safety boundary changes.
- Nextest execution of the cleanup regression: verify the single-threaded executable works with the CI test runner.
- Platform CI: macOS, Windows, Linux x86_64 and Linux aarch64 cover platform-gated code. No desktop app launch or live UI checks are selected because the migration preserves behavior.

## Migration audit

The macOS edition fixer completed successfully while the workspace still declared edition 2021. All five crates inherit the new workspace edition. Dependency versions, Cargo.lock, resolver 2 and the toolchain stay unchanged; rustfmt explicitly keeps style edition 2021.

Reserved `gen` bindings in the ULID tests become `generator`. Macro expression fragments become `expr_2021` to retain their original accepted syntax and matching precedence. The retained fixer `if let` to `match` conversions preserve 2021 temporary lifetimes through fallback branches; their reindentation is confined to those expressions. The single-match conversion in mission attachment was reverted because its successful value is unit and its error is moved into the binding; neither fallback retains a resource, and Clippy correctly prefers the original if-let form. No let-chain cleanup is included. Edition 2024 enables Clippy let-chain suggestions throughout the existing nested conditions; all five crates inherit a workspace allowance for collapsible_if so this remains a separate cleanup. Six SQLite query-return bindings that were needed for 2021 temporary lifetimes are removed where edition 2024 makes them redundant and Clippy now rejects them. Query collection and statement ownership remain unchanged. Three analogous redundant return bindings in a UI builder, the remote sidebar builder and a test capture helper are also removed to satisfy the same lint.

The fixer added ten precise capture bounds. Trial compilation without them identified two necessary bounds: the app-shell titlebar drag-area builder (to release self/context borrows before rendering the body) and the updater hint builder (to avoid returning a borrow of a temporary update value). Those two use<> bounds are retained; eight redundant bounds on other UI builders and static closures are removed. The two diagnostic-only cases, the handshake test helper and settings content builder, retain their existing signatures; no caller needs to outlive an extra captured reference.

The remaining temporary-drop diagnostics were audited by ownership and side effect:

- Protocol transports, terminal models and session handles: successful values are moved into bindings or cloned Arcs; failure/empty variants retain no transport or model. Explicit mutex guards keep their existing lexical scopes.
- Daemon boot, remote routing and remote client futures: completed await/select results are moved or consumed. Channel sends, cancellation and route cleanup remain explicit. Buffer/error temporaries hold data rather than lock ownership needed by later branches.
- Session lifecycle, router dispatch and usage persistence: handle extraction moves ownership; mutex guards protecting state remain named locals, and stop/join, delivery and database operations keep their order.
- Temporary-file persistence: successful handles are discarded after a completed persist; retry branches move the returned temporary file back into the retry variable. No file is removed before a retry.
- UI entities, requests and terminal attachment: returned entities/errors are moved or consumed, and cancellation/attachment ownership remains in named state.
- Test fixtures: owned fixtures survive their completed constructor futures; explicit teardown and test assertions remain in place.

## Environment safety

`clean_environment` and daemon `server::run` expose an unsafe startup contract. The only production caller is the synchronous runnerd dispatch from CLI main. Isolated HOME/USERPROFILE/XDG_CONFIG_HOME/CODEX_HOME setup and daemon environment cleanup occur before logging setup, boot (database/watchers/session workers), or the Tokio runtime. No environment mutation is added after those thread-producing operations.

The old cleanup subprocess entered libtest, which still has a harness thread even for one selected test. Its command-environment assertions remain in the unit test. Actual process mutation now runs in a harness-free executable on its sole main thread, with its environment supplied through Command. It checks mission context removal, fixture recording preservation and PATH cleanup, and implements the small listing/execution interface nextest requires.

Windows-only shutdown callbacks now contain explicit unsafe blocks for their existing Win32 calls and raw pointer operations; callback ownership and operation order are unchanged. A source scan found no further environment mutations, reserved gen identifiers, deprecated before_exec calls or unsafe function bodies outside these audited sites.

## Platform gap

The existing native Windows SSH target pc at 192.168.2.137:22 timed out with BatchMode and an eight-second connection timeout on 2026-11-13. Native Windows cargo fix was unavailable. Windows source inspection and final Windows CI compilation/tests are separate evidence and do not replace that migration run.

## Results

- macOS cargo fix: passed on edition 2021 before the manifest change.
- make verify: passed, including workspace tests, Clippy, updater Clippy and formatting. Seven existing tests remain ignored (manual/provider/benchmark coverage); no ignored test was enabled or added by this migration.
- cargo nextest run -p runner-core --test daemon_environment --no-fail-fast: passed (one test); nextest recognizes and executes the harness-free cleanup check.
- Explicit workspace and updater Clippy runs: passed after the migration refinements.
- git diff --check: passed. Cargo.lock, dependency versions and rust-toolchain.toml are unchanged.
- Platform CI is required before the PR handoff; its results and the exact reviewed head are recorded in the PR.

Intermediate checks exposed the edition-enabled collapsible_if and let_and_return lints. Trial removal of all precise captures produced borrow errors for the two builders identified above; only those bounds were restored. Clippy also rejected one fixer-generated single-pattern match, which was reverted after its resource-free fallback was audited. No runtime test failed.

The frozen candidate SHA is carried by the Runner review handoff and subsequent PR review comment.
