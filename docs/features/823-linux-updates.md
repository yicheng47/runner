# Linux in-app installation updates

Related: #823's native Linux delivery. This extends the existing Linux branch;
The follow-up is committed locally at the user's request; no PR push or release
dispatch is requested.

Linux has independent updater state, scheduling, download/Minisign verification,
settings and dialog modules. Application entry points select the platform backend
at compile time. macOS keeps Sparkle; Windows keeps its existing updater and views.
Linux selects its architecture, channel and installed format. Production compares
semantic versions; nightly channels compare build stamps and remain separate.
Unsigned historical assets support notification/download-page access only.

User tar installs replace the complete application directory with Linux atomic
directory exchange after safe extraction, layout/identity/ELF checks. No mixed
app/CLI/daemon payload is exposed on interruption. Application data is outside
the install tree. System deb/rpm updates use an authenticated, narrowly scoped
installer mode in the root-owned packaged Runner executable. It copies inputs
into root-owned staging, verifies again before reading package metadata, and
uses APT/DNF without starting GPUI, SQLite or agents as root. Authorization
cancellation or installation failure keeps the existing app open with retry/logs.
Successful installation quits with existing update lifecycle semantics, then
relaunches as the original user. Repository dependency signature checks remain
active. DNF and polkit are prerequisites for the RPM automatic install path;
unsupported installations retain a useful error and manual download access.

Release/nightly builds sign all six Linux format/architecture artifacts using
the existing updater trust key. Publication requires and verifies signatures;
PR CI continues to run checks/tests without release packaging or signing secrets.

Affected guides: getting-started/install.md and guides/troubleshooting.md. Development/packaging contract:
docs/arch/linux.md. The Linux dialog follows the existing visual layout without a redesign.
