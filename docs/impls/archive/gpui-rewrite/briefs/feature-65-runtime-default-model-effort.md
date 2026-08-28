# Mission brief — Feature 65: runtime default model and effort

Drafted 2026-08-28; started as mission `01M136PE1BBGJTBP4DQMNN5N1B` the same day. Start with `mission_start` on crew `codex peer` (`01K000DEFAULT000PEERCODING01`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZWQA969B26J6WSP68RZP4Y9`), no `cwd`, title `Feature 65 — Runtime default model and effort`, the text below as `goal_override`. Landed as PR #456 → `491307b` on 2026-08-28, closing #380; the smoke test hid the Agents caption on Not-found rows.

---

Implement **feature 65 — runtime default model and effort** (`docs/features/65-runtime-default-model-effort.md`, issue #380; this brief lives at `docs/impls/archive/gpui-rewrite/briefs/feature-65-runtime-default-model-effort.md`). **Branch off `main`** (suggested `feat/runtime-default-model-effort`); one feature branch in this checkout. The two docs above plus `docs/features/README.md` are uncommitted, human-authored, and out of review scope — leave them alone and do not treat them as a dirty-tree blocker. Standing GPUI rules in `docs/impls/gpui-rewrite/README.md` apply. **Crews do not launch the app**: Jason smoke-tests; you implement and verify with tests.

## The change (surveyed 2026-08-28)

Read-only. Runner never writes a runtime's config; only `mcp_servers.runner` is ever touched by `ops/mcp.rs`.

1. **Reader** — new `crates/runner-backend/src/runtime_defaults.rs` (register in `lib.rs`): `pub struct RuntimeDefaults { pub model: Option<String>, pub effort: Option<String> }` and `pub fn runtime_defaults(runtime: &str, home: &Path) -> RuntimeDefaults`, path-injected so tests use a temp home. Per runtime: codex `~/.codex/config.toml` and trae `~/.trae/traecli.toml` via `toml_edit` (already a dep; `ops/mcp.rs:256` parses the same file) — `model`, `model_reasoning_effort`, with a top-level `profile = "<name>"` making `[profiles.<name>]` keys win over top-level ones; claude-code `~/.claude/settings.json` and qoder `~/.qoder/settings.json` via `serde_json` — `model`, `effortLevel`. Missing file, missing key, malformed file, non-string value → `None`; the function never errors. Values verbatim, trimmed (`claude-fable-5[1m]` stays as written). Reuse the home-dir/path helpers pattern from `ops/mcp.rs:80-101` rather than duplicating constants where they can be shared.
2. **Status + catalog** — `RuntimeExecutableStatus` (`runtime_status.rs`) gains `default_model` / `default_effort`, filled in `status_list` for every runtime regardless of row state. `RuntimeCatalogEntry` (`ops/runtime.rs`) gains the same two fields, copied from the status inside `runtime_catalog` next to `available`. Fix the app-side test constructor at `surfaces/settings/agents.rs:1027`.
3. **Settings → Agents** (`surfaces/settings/agents.rs`) — `render_runtime_row` adds a caption line under the executable row, same styling as the existing caption (JetBrains Mono, 11/16 rem, `theme::faint()`), always rendered: `Model: <model> · Effort: <effort>`, each half `runtime default` when `None`. Derive the string in a pure helper next to `runtime_presentation` so it is unit-testable; keep the existing validation caption where it is (it stays red on error; the defaults line is always faint).
4. **Runner create/edit** (`surfaces/runners.rs`) — the model `TextField` placeholders at `:301` and `:490` become `default (<model>)` when the selected runtime's catalog entry carries `default_model`, `default` otherwise, and must update when the Agent select changes (follow the path that already swaps `runtime_models` on runtime change; `TextField` has a placeholder setter or add one). `effort_options` (`:2747`): the empty option's label for the plain runner form becomes `Runtime default (<effort>)` / `Runtime default`; the `edits_slot` branches are unchanged. Do not touch `crews.rs` or `start_chat.rs`.

## Tests

- `runtime_defaults.rs`: codex top-level keys; codex `profile` precedence (profile sets model only → effort still from top level); trae same keys; claude `model` + `effortLevel`; qoder `model` only; missing file; malformed TOML and JSON; non-string `model` value; unknown runtime name → both `None`.
- `agents.rs`: the defaults line for known / half-known / unknown values.
- `runners.rs`: placeholder and empty-option label with and without a default, if the file's tests already cover option building; otherwise a small pure helper with its own test.
- Existing assertions extended, never weakened or deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- Handoff to Jason: the branch name plus the manual pass from the spec §Verification.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) any write path to a runtime config file, or a read that can surface as an error or panic on a missing/malformed file; (2) key names wrong per runtime (`model_reasoning_effort` for codex/trae, `effortLevel` for claude/qoder) or profile precedence inverted; (3) values altered (lowercased, suffix-stripped) rather than shown verbatim; (4) defaults missing from `RuntimeCatalogEntry` so the runner form cannot see them, or the placeholder not following the Agent select; (5) the Agents caption line missing for Not found / Checking rows or restyled away from the existing caption; (6) scope creep into `crews.rs`, `start_chat.rs`, env-var resolution, or file watching; (7) tests weakened or the app-side status constructor patched by adding a wildcard instead of the real fields.
