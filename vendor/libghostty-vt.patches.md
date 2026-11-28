# libghostty-vt local patches

This file tracks intentional local changes applied on top of the vendored `vendor/libghostty-vt` source (`vt-1.0.2`, commit `573c4a21e02311ae87844af26b6a7a179f82858e`). Remove a patch only when the vendored source version contains the upstream fix and the listed verification passes without it.

## 1. Preserve explicit macOS minimum OS version in `src/build/Config.zig`

- **Files:** `vendor/libghostty-vt/src/build/Config.zig`
- **Why:** Runner ships macOS bundles with `MINIMUM_SYSTEM_VERSION="12.0"` (`script/bundle-mac`), and `crates/ghostty-vt-sys/build.rs` passes `-Dtarget=<arch>-macos.<MACOSX_DEPLOYMENT_TARGET>` (defaulting to `12.0`). Upstream `Config.zig` replaces any macOS target on a Darwin host with `genericMacOSTarget(b, result.query.cpu_arch)`, which discarded `result.query.os_version_min` and hardcoded `osVersionMin(.macos)` (`13.0.0`), producing Mach-O objects with `minos 13.0`. Adding `genericMacOSTargetWithOsVersionMin` preserves an explicit `-Dtarget=*-macos.<ver>` floor while keeping the `generic` CPU workaround and the `13.0.0` default when no version is given.
- **Verification:** Build `ghostty-vt-sys` for both `aarch64-apple-darwin` and `x86_64-apple-darwin`, extract every object member from each `libghostty-vt.a`, and confirm `otool -l` reports `minos 12.0` (with no `minos 13` or higher) on every object.
- **Remove when:** Upstream Ghostty preserves `result.query.os_version_min` on macOS hosts (or lowers `osVersionMinLibVt(.macos)` / Runner raises `MINIMUM_SYSTEM_VERSION` to 13.0+).
