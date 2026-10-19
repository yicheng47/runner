# Linux desktop and headless support

Related: [#823](https://github.com/yicheng47/runner/issues/823). The current implementation record is [#982 Linux/remote integration](../impls/982-linux-remote-compat.md); the [original local mission brief](../impls/briefs/823-linux-support-local.md) is historical.

Runner supports Linux ARM64 and x86_64 with the native GPUI app, daemon and CLI.
X11 and Wayland share the existing Windows-style window controls and Ctrl bindings.
Linux uses fontconfig with Noto CJK fallbacks. macOS and Windows behavior is retained.
There is no Linux in-app updater or macOS wake/notification-permission integration.

The daemon retains Unix IPC ownership, bounded startup/shutdown and SSH no-autostart.
Linux discovers descendants through /proc and Codex rollouts through exact open file
descriptors, preserving existing marker, ambiguity, claim and timeout checks.
A minimal systemd environment resolves the account login shell without assuming bash.

All packages stage the same Runner and runner-agent-cli bytes, with runnerd selecting
daemon mode from its executable name. The tarball installer and deb/rpm packages expose
a headless CLI, desktop entry, icon and opt-in user service. Installation never enables
the service. Restart=no prevents service restart loops when the app replaces an old
hash or another daemon owns the endpoint. Uninstall preserves application/user data.

Build outputs are architecture-labelled tar.gz/deb/rpm. Release definitions use an
older Ubuntu baseline for both architectures; native Ubuntu 26 ARM64 evidence alone
makes no older-distro compatibility claim. WSL2 needs systemd for the optional service
and WSLg/Vulkan for the app. Native x86_64 GUI, X11, WSL/WSLg and RPM-distro
installation remain unverified. Hosted Ubuntu22 checks/tests cover both
architectures; Ubuntu26 ARM64 user reports cover the window, input, zoom,
OpenCode two-role startup, package/menu launch and corrected dock identity.

Affected guides: getting-started/install.md, reference/keyboard-shortcuts.md,
guides/troubleshooting.md, guides/remote-access.md and (only if help changes) generated reference/cli.md.
Architecture/development details live in docs/arch/linux.md; both READMEs change together.
Historical contributor authorization covered commits, pushes and a draft PR in the fork; no upstream PR,
merge, release publication, global agent changes or design changes are authorized.

The 2026-10-19 integration follows upstream 0.16.5: shared desktop chrome and
Linux platform copy use the English/Simplified Chinese catalogs, file-drop and
multi-target crew messaging remain intact, and Linux nightly definitions honor
the separate `nightly` and `remote-nightly` channels. Linux CI provides Node
22.14.0 for the existing embedded pi extension tests; no agent account is needed.

Current integration: rebased onto Runner 0.18.0. Linux uses stable and the single nightly channel; remote-nightly has been retired. Remote host/client compatibility changes, approved storage policy and automated verification are summarized in `docs/impls/982-linux-remote-compat.md`; fresh native Linux remote acceptance remains outstanding.
