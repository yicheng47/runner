# #1508 — libghostty-vt trial: build and fixture parity

Jason approved a mission with one Antigravity `gemini-4-neon` coder (role `impl-agy`) and no reviewer or qa on 2026-11-27. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1508-libghostty-vt` on the branch `feat/1508-libghostty-vt` (base `origin/main` `47b59c77`, its own `target/`). Its draft PR is [#1509](https://github.com/yicheng47/runner/pull/1509). Never touch the root checkout or sibling worktrees.

## Question

Does Phase 1 of #1508 go ahead? It does if two things hold. First, Ghostty's `libghostty-vt` at tag `vt-1.0.2` builds and links into Runner, through a binding Runner owns, on every CI target. Second, Runner's recorded terminal fixtures replay into it with the screens `alacritty_terminal` produces today, or with differences that are explained and acceptable. This trial answers both with evidence; it is the gate in the spec's Implementation Phases.

The spec is authoritative: [`docs/features/1508-libghostty-vt.md`](../../features/1508-libghostty-vt.md), especially Decisions 2 and 3, Build and dependency wiring, Open question 1 and Implementation Phases.

## Read first

- The spec.
- herdr's build approach (Apache-2.0; read it for the approach and write Runner's own code): [`herdrdev/herdr-app`](https://github.com/herdrdev/herdr-app) `crates/ghostty-sys/build.rs` and `Cargo.toml`, which use a submodule; [`herdrdev/herdr`](https://github.com/herdrdev/herdr) `crates/ghostty-vt/build.rs`, `vendor/libghostty-vt.vendor.json`, `vendor/libghostty-vt.patches.md` and `scripts/vendor_libghostty_vt.py`, which use a vendored copy, `--system` and Windows `--libc`.
- Ghostty `vt-1.0.2`: `build.zig.zon`, `include/ghostty/vt.h` and `include/ghostty/vt/*.h`, mainly `terminal.h`, `render.h`, `screen.h`, `grid_ref.h`, `modes.h` and `sys.h`.
- `runner-terminal`: `src/replay.rs`, `src/fixtures.rs`, `tests/fixture_replay.rs`, and `fixtures/` (recorded `.ndjson` logs with blessed `.snapshot.txt` screens).
- `.github/workflows/ci.yaml`: the macOS, Windows and Linux x64/arm64 jobs.

## Deliverables

1. **`crates/ghostty-vt-sys`, as Decision 2 describes.** Its `build.rs` runs `zig build -Demit-lib-vt -Demit-xcframework=false -Doptimize=ReleaseFast -Dtarget=<zig target>`, with an explicit Zig target for each of the five Rust targets. It links only the static library, generates bindings from `ghostty/vt.h` with `bindgen`, and takes the Zig binary from `ZIG`. Choose how `vt-1.0.2`'s source reaches a worktree (a vendored copy or a git submodule) and how Zig's package dependencies are cached, so that a fresh worktree builds offline after a one-time setup. Record the choice and why in the spec's Open question 1. The crate is a workspace member but not yet a dependency of any production crate: Runner builds and behaves exactly as now.
2. **CI.** Install Zig 0.17.0 in the macOS, Windows and Linux jobs of `ci.yaml`, then build and test `ghostty-vt-sys` there, Windows first. Pushing the branch runs CI on PR #1509. Leave `nightly.yml` and `release.yml` for 1a proper.
3. **The parity harness.** Add a test-only harness that replays every fixture `fixture_replay` uses into a `libghostty-vt` terminal of the same size. It renders the visible screen in the blessed snapshots' text form, including the cursor and the modes they record, and diffs it with each `.snapshot.txt`. Port the focused cases in `tests/fixture_replay.rs` the same way: wide CJK and emoji, combining marks, box drawing, wide characters at the line end, resize reflow, the alternate screen and scrollback. Never edit, re-bless or weaken the existing tests and snapshots. For each fixture and case, report whether it is identical or differs, and classify each difference: a real engine difference, a gap in the harness, or an alacritty quirk the blessed file encodes.
4. **A note for 1b.** List what `input_state` and `delivery_probe` read from `Term` (cells, wide flags, cursor, modes, history, display offset), and whether `libghostty-vt`'s API gives each one, with the header and function. Do not port them.

## Boundaries

- Leave `alacritty_terminal`, `vte` and every production path in `runner-terminal`, `runner-app` and `runner-daemon` unchanged. Do not run the app; this trial has no live checks.
- Zig: Homebrew's Zig on this Mac is 0.16.0, and `vt-1.0.2` needs 0.17.0. Download the official 0.17.0 build for `aarch64-macos` from ziglang.org into `~/.local/share/zig/0.17.0`, check it against the SHA-256 in ziglang.org's download index, and point `ZIG` at it. Do not change Homebrew's Zig or any other global setting.
- Patch Ghostty's source only where a build needs it. Track each patch in a patches file, as herdr does, and report it.
- If a target cannot be made to build with a small tracked patch, stop and report to the operator with the error instead of working around it.

## Validation and report

- Local: `cargo build -p ghostty-vt-sys` from an empty target directory and again warm, timing both; `cargo build --target x86_64-apple-darwin -p ghostty-vt-sys` if that toolchain is installed; the new crate's tests and the parity harness; the existing `cargo test -p runner-terminal`, unchanged and green; `make clippy`, `cargo fmt --all -- --check` and `git diff --check`.
- CI: the new steps green on macOS arm64, Windows x64 and Linux x64 and arm64, with every existing job still green.
- When done, post one report with `runner msg post`, with no `--to`. Include each target's build result with cold and warm times; the static library's size; the source and cache choice; corpus results (identical against differing, with each difference and its classification); the 1b note; any patches carried; and a go or no-go recommendation with the main risks.

## Authorization

Approved: one coder, Antigravity `gemini-4-neon` (`impl-agy`). It may commit on `feat/1508-libghostty-vt` and push it with plain pushes to run CI on draft PR #1509, and may record the source decision in the spec. PR #1509 stays a draft with its title and labels unchanged. No co-author trailers. Do not open another pull request, merge, force-push or start other agents.
