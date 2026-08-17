# GPUI Rewrite

Program directory for the Rust-native UI rewrite (`gpui-nightly` line). New impls for this program get numbered directories here; everything else in `docs/impls/` belongs to the Tauri line's history.

- [impl_log.md](impl_log.md) — the program-wide progress log (current state + dated entries). Start here to catch up.

- [0031 — Rust-native UI rewrite: approach plan](0031-rust-native-ui-rewrite/plan.md) — the strategy record: end state, framework decision, branch strategy (single repo, two branches), phases 1–6.
- [0046 — Main-parity catchup](0046-main-parity-catchup/plan.md) — the working plan: backend sync with `main`, node-tree adoption, terminal split, gpui-ce. Supersedes 0031's Phase 4 slice list.

Standing decisions (dated in the specs):

- `main` owns product design and the domain model; repo-and-below stays verbatim-identical, migration numbers are allocated on `main` only (0046 §Decisions).
- Framework: `gpui-ce`; terminal architecture mirrors Zed's `terminal`/`terminal_view` split on upstream `alacritty_terminal` (0046 §Workstreams C–D). Zed's terminal crates are GPL — architectural reference only.
- Updater (Phase 5): Sparkle via the pulse pattern — `~/repos/yicheng47/pulse` is the in-house reference implementation on the same stack (0031 §Phase 5).
