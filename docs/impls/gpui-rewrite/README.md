# GPUI Rewrite

Program directory for the Rust-native UI rewrite. **Shipped:** the line was promoted to `main` by tree-swap on 2026-08-23 and published as [`v0.6.0`](https://github.com/yicheng47/runner/releases/tag/v0.6.0) the same day — one native binary, universal, Sparkle updates, with the one-time bridge from 0.5.x. What remains (the M6 remainder, release chores) is tracked in [#432](https://github.com/yicheng47/runner/issues/432) and lands on `main` as `0.7.0-nightly`. Everything else in `docs/impls/` belongs to the Tauri line's history.

- [impl_log.md](impl_log.md) — the program-wide progress log (current state + dated entries). Start here to catch up.
- [m6-consolidation.md](m6-consolidation.md) — M6 plan, pending on top and landed at the bottom. Landed through GA: M6.6, M6.8, M6.5, M6.10 + M6.11, M6.9, the M6.3 session-lock item, M6.13 + M6.12, M6.1, M6.18, M6.17, M6.21, M6.22. Post-GA queue: M6.16, M6.19, M6.20, M6.15, then M6.2, M6.7, M6.4, the M6.3 remainder.
- [plan.md](plan.md) — the program plan: strategy, standing decisions, workstreams, and the roadmap (milestones M0–M6, then Phases 5–6). Merged 2026-08-18 from impls 0031 (approach plan) and 0046 (main-parity catchup); those numbers remain the citation keys used in the log.

Standing decisions (dated in the plan):

- `main` owns product design and the domain model; repo-and-below stays verbatim-identical, migration numbers are allocated on `main` only (plan §Decisions).
- Framework: `gpui-ce`; terminal architecture mirrors Zed's `terminal`/`terminal_view` split on upstream `alacritty_terminal` (plan §Framework, §Workstream C). Zed's terminal crates are GPL — architectural reference only.
- Pulse (`~/repos/yicheng47/pulse`) is the window-level UI reference and the updater reference — Sparkle via the pulse pattern (plan §UI reference, §Phase 5).
- Crate renames after the M3 session-hardening slice (landed 2026-08-18): `runner-app` → `runner-backend`, `runner-native` → `runner-app` (plan §Decisions).
- Release channels and the cutover bridge: `CFBundleVersion` is always a build stamp, nightlies are prereleases on a rolling `nightly` feed (never the `releases/latest` alias the Tauri updater polls), and GA `v0.6.0` is the single cutover release carrying both Sparkle and Tauri-bridge artifacts (plan decision 12, §Release channels).
