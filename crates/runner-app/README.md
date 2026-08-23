# runner-app

The Runner application: GPUI UI, terminal renderer, Sparkle updater, packaging. The Cargo package is `runner-app`; the binary it builds is `Runner` (`[[bin]]`), which `script/bundle-mac` wraps into `Runner.app` / `Runner Nightly.app`. Everything UI-agnostic — sessions, router, event log, SQLite, MCP — lives in `runner-backend`; the terminal model, input encoding, and the fixture corpus live in `runner-terminal`. How it fits together: [`docs/arch/arch.md`](../../docs/arch/arch.md); how it got here: [`docs/impls/gpui-rewrite/README.md`](../../docs/impls/gpui-rewrite/README.md).

## Run

```sh
make run
```

The development build uses `~/Library/Application Support/com.wycstudios.runner-dev/runner.db`, keeping production state isolated; release bundles use `com.wycstudios.runner`. It has no bundle, so macOS labels it by the binary name (`Runner`) and the updater is a no-op (the `updater` feature is off in dev). GPUI requires the Xcode Metal Toolchain component.

One app instance at a time: the dev build and an installed Runner share nothing, but an installed production and nightly bundle share one data directory.

## Checks

```sh
make verify                      # check + workspace tests + clippy --features updater + fmt
cargo test -p runner-app         # app unit tests
cargo test -p runner-app --test bundle_mac   # bundle script plist contract
cargo test -p runner-terminal    # fixture corpus replay (grid snapshots + input-state transitions)
```

## Fixture corpus

Recorded PTY byte logs in `../runner-terminal/fixtures/*.ndjson` (header line + base64 data/input/exit events) are replayed into a headless `Term` and compared against blessed `*.snapshot.txt` grids; `input-*` fixtures also pin the input-state tracker's transition list. Re-bless with `UPDATE_SNAPSHOTS=1 cargo test -p runner-terminal`. Record a new fixture from the running app with `RUNNER_RECORD_INPUT_FIXTURE=<prefix>` (writes `<prefix>.<session>.ndjson`).

## Packaging

`script/bundle-mac --channel nightly|production [--dev]` assembles, signs, and notarizes the universal bundle; CI runs it from `nightly.yml` (dispatch-only, rolling prerelease) and `release.yml` (`v*` tags → draft release). See `docs/arch/arch.md` §14 and the `/nightly` skill.
