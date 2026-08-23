# Mission brief — M6.17: universal macOS builds (Apple Silicon + Intel)

Drafted 2026-08-23. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), explicit `cwd` the `runner-gpui` worktree, title `M6.17 — Universal macOS builds`, the text below as `goal_override`. The human's nightly app stays running throughout — the crew never launches a bundle and never dispatches a workflow.

---

Implement **M6.17** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.17 — read it first; this brief also lives at `docs/impls/gpui-rewrite/briefs/m6-17-universal-builds.md`). One feature branch in this worktree off `gpui-nightly`; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. Release pipeline only — no app code: `rust-toolchain.toml`, `script/bundle-mac`, `.github/workflows/nightly.yml`, two README lines. `runner-app` is the binary, `runner-backend` the core.

## The defect

The nightly is arm64-only: `script/bundle-mac:6` pins `TARGET_TRIPLE="aarch64-apple-darwin"`, `:221-222` build that one target, `:230-233` fail the bundle unless `lipo -archs` is exactly `arm64`, `:281` names the notarization zip `-arm64.zip` and the `DMG_NAME`s at `:124`/`:131` `-arm64.dmg`; `nightly.yml:46` adds only the arm64 target and `:107`/`:111` name and prune `-arm64.dmg`. An arm64-only Mach-O does not launch on an Intel Mac, and the planned Tauri bridge manifest would only have carried `darwin-aarch64`, so Intel 0.5.2 installs would never be offered 0.6.0. Decision (Jason, 2026-08-22): **one universal DMG**, not two — a Sparkle appcast item has one enclosure and no architecture field; the updater is unaffected because Sparkle compares `CFBundleVersion` and verifies signatures, never architecture.

## Scope

1. `rust-toolchain.toml`: add `targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"]` so `rustup` installs both locally and in CI.
2. `script/bundle-mac`: replace the single triple with both; run the two `cargo build` invocations per triple (release, `--locked`, same `RUSTFLAGS` — the `-Lframework`/rpath flags apply to both link steps through the env; same `MACOSX_DEPLOYMENT_TARGET=12.0`); for each of `runner-app`, `runner-agent-cli`, `runner-mcp` `lipo -create` the two slices straight into `Contents/MacOS/`, then assert `lipo -archs` reports **both** `arm64` and `x86_64` — test set membership, not string equality (lipo's print order is fixed but not worth depending on); rename `-arm64` → `-universal` in both `DMG_NAME`s, the `APP_ZIP`, and the final `printf`. Also verify the embedded `Sparkle.framework/Versions/B/Sparkle` carries both slices and fail the bundle if not.
3. `.github/workflows/nightly.yml`: `rustup target add aarch64-apple-darwin x86_64-apple-darwin`; `new_dmg` → `-universal.dmg`; the prune regex `([0-9]{8})\.([0-9]{4})-arm64\.dmg$` → `-(arm64|universal)\.dmg$` so the earlier arm64 items keep counting toward the ten and the appcast keeps them; `timeout-minutes: 60` → `90` — the cold arm64 run took 27 min and the cargo step now runs twice. Leave the channel-isolation steps untouched.
4. README: line 32 `macOS, Apple Silicon.` → `macOS, Apple Silicon and Intel (one universal build).`; line 50 `(Apple Silicon \`.dmg\`)` → `(universal \`.dmg\`, Apple Silicon and Intel)` and `Intel Macs, Linux, and Windows are not supported.` → `Linux and Windows are not supported.`
5. Nothing else: no `release.yml` (it does not exist yet; plan §Release channels owns it), no app code, no plist changes (`LSMinimumSystemVersion` 12.0 already covers Intel).

## Proof, locally, without launching anything

`rustup target add x86_64-apple-darwin`, then `CARGO_BUILD_JOBS=12 ./script/bundle-mac --channel nightly --dev` (ad-hoc signing, no notarization; it downloads Sparkle 2.9.5 once into `target/sparkle/`). Expected: both release builds succeed — the x86_64 one cross-compiles on this arm64 host with the Xcode toolchain; if gpui-ce's Metal build script or any C dependency misbehaves for the x86_64 target, that is a finding to report with the error, not to work around silently — `lipo -archs` on the three installed binaries and on the Sparkle binary reports both slices, `codesign --verify --deep --strict` passes, and the script's final line names the universal bundle. Record in the handoff: wall time of each target's build, the app's on-disk size arm64-only vs universal (the old bundle is in `target/release/` from the last run if present; otherwise size the x86_64 slice with `lipo -thin`). **Do not open the produced `.app`** — the human's nightly is running, both would share `com.wycstudios.runner`'s data dir and the MCP socket. **Do not run `gh workflow run`.** The Rosetta launch (`arch -x86_64 …/runner-app`) is the human's check on the next nightly.

## Gates

- `make verify` green (the Rust workspace is unchanged; this proves nothing broke by accident), `bash -n script/bundle-mac`, `git diff --check` clean, the local `--dev` bundle as above.
- Handoff: the `lipo -archs` lines, the build timings and size delta, and any x86_64 cross-build finding.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) any path still built from one triple (`target/$TARGET_TRIPLE/release/…`, the zip name, the prune regex, the `lipo` assertion) — the bundle must fail loudly, never ship one slice under a universal name; (2) the prune regex dropping the older `-arm64` items from the rolling feed, or the ten-item window miscounting across the two names; (3) `rustup target add` missing in CI so the cross-build fails after the 10-minute warm-up; (4) the `lipo -create` inputs in the wrong order or a leftover `install -m 755` copying only one slice; (5) `timeout-minutes` left at 60; (6) `--locked` dropped, or `RUSTFLAGS`/`MACOSX_DEPLOYMENT_TARGET` exported for only one of the two builds; (7) the README or plan still saying arm64 anywhere (`grep -rn arm64 README.md docs/impls/gpui-rewrite/plan.md`); (8) the crew having launched the bundle or dispatched a workflow.
