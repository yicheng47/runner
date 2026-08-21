# GPUI Rewrite

Program directory for the Rust-native UI rewrite (`gpui-nightly` line). New impls for this program get numbered directories here; everything else in `docs/impls/` belongs to the Tauri line's history.

- [impl_log.md](impl_log.md) — the program-wide progress log (current state + dated entries). Start here to catch up.
- [m6-consolidation.md](m6-consolidation.md) — M6 plan: terminal resize smoothness (M6.6) and long-lived terminals (M6.8) landed 2026-08-21; `arch.md` rewritten 2026-08-22 (M6.5); next real input-state tracking (M6.1), the in-app update prompt (M6.9), hook-based session status, post-audit defects, and structural debt, with the `main`-first landing rule.
- [plan.md](plan.md) — the program plan: strategy, standing decisions, workstreams, and the roadmap (milestones M0–M6, then Phases 5–6). Merged 2026-08-18 from impls 0031 (approach plan) and 0046 (main-parity catchup); those numbers remain the citation keys used in the log.

Standing decisions (dated in the plan):

- `main` owns product design and the domain model; repo-and-below stays verbatim-identical, migration numbers are allocated on `main` only (plan §Decisions).
- Framework: `gpui-ce`; terminal architecture mirrors Zed's `terminal`/`terminal_view` split on upstream `alacritty_terminal` (plan §Framework, §Workstream C). Zed's terminal crates are GPL — architectural reference only.
- Pulse (`~/repos/yicheng47/pulse`) is the window-level UI reference and the updater reference — Sparkle via the pulse pattern (plan §UI reference, §Phase 5).
- Crate renames after the M3 session-hardening slice (landed 2026-08-18): `runner-app` → `runner-backend`, `runner-native` → `runner-app` (plan §Decisions).
- Release channels and the cutover bridge: `CFBundleVersion` is always a build stamp, nightlies are prereleases on a rolling `nightly` feed (never the `releases/latest` alias the Tauri updater polls), and GA `v0.6.0` is the single cutover release carrying both Sparkle and Tauri-bridge artifacts (plan decision 12, §Release channels).
