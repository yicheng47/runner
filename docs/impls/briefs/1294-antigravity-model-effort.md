# Fix Antigravity model and effort selectors (#1294)

## Starting point

- Issue: `yicheng47/runner#1294` (`P1`, milestone `0.19`)
- Branch: `fix/1294-antigravity-model-effort`
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1294-antigravity-model-effort`
- Base: `origin/main` at `95d74efa`

Work only in this worktree and use its own Cargo `target/` directory. The root checkout stays on `main`.

## Problem and outcome

`agy models` (`agy 1.5.8`) emits effort-suffixed entries for current models, such as `gemini-4-carbon-{low,medium,high}`, `gemini-4-argon-{low,medium,high}`, `gemini-4-hydrogen-{low,medium,high,xhigh}`, `claude-opus-5-5-{low,medium,high}`, and `claude-sonnet-5-5-{low,medium,high}`, alongside `gemini-3.8-flash-{low,medium,high}` and non-suffixed entries (`gemini-pro-orchestrator`, `gemini-pro-orchestrator-argon`, `gpt-oss-120b-medium`).

In Runner today:

1. `antigravity::models::parse` only strips `-(low|medium|high)` (missing `xhigh` / `max`) and only groups entries that appear in the stale static `ANTIGRAVITY_MODELS` allowlist. Newer Gemini 4 and Claude 5.5 entries stay ungrouped with the effort baked into both the model ID (`gemini-4-hydrogen-high`) and label (`Gemini 4 Hydrogen (High)`), and `supported_efforts: Some([])`.
2. `ANTIGRAVITY_EFFORTS` only lists `["low", "medium", "high"]` (missing `xhigh`, while `agy --help` lists `low|medium|high|xhigh|max`), and `Antigravity::model_effort_args` drops `--effort` unless `(model, effort)` is in the static `ANTIGRAVITY_MODELS` table.
3. In Roles (`role_model_efforts`), leaving an Antigravity role's model blank falls back to `runtime.efforts` even though Start Chat (`effort_needs_launch_model`) disables effort without a model and `Antigravity::model_effort_args` drops `--effort` without `--model`.

**First verify `agy 1.5.8` CLI behavior** before designing the exact fix: check how `agy` handles `--model <base> --effort <level>` vs leveled model IDs (e.g. `gemini-4-hydrogen` + `--effort xhigh` vs `gemini-4-hydrogen-xhigh`, and whether `agy` requires leveled IDs or accepts base + `--effort` across its current Gemini/Claude models). Then make model and effort selection across Start Chat, Roles (create/edit), and Crew slots consistent with what `agy` actually accepts, without duplicating effort inside the model selector alongside a conflicting or disabled effort selector.

## Read first

- `crates/runner-daemon/src/runtimes/antigravity/models.rs` (`parse`)
- `crates/runner-daemon/src/runtimes/antigravity/mod.rs` (`Antigravity::model_effort_args`)
- `crates/runner-core/src/protocol/runtime_metadata.rs` (`ANTIGRAVITY_MODELS`, `ANTIGRAVITY_EFFORTS`, `antigravity::capabilities`, `antigravity::catalog`)
- `crates/runner-core/src/protocol/runtime.rs` (`RuntimeCatalogEntry::efforts_for_model`)
- `crates/runner-app/src/surfaces/front_page/model.rs` (`RuntimeChoice::efforts_for_model`)
- `crates/runner-app/src/surfaces/start_chat/controls.rs` (`sync_effort_control`, `effort_needs_launch_model`)
- `crates/runner-app/src/surfaces/roles/logic.rs` (`role_model_efforts`, `effort_options`, `create_role_effort_options`)
- `crates/runner-daemon/src/runtime_defaults.rs` (note how `agy` persists display labels in `settings.json`)

## Deliverables and boundaries

- Probe `agy 1.5.8` cleanly (without touching user settings or non-test data) to confirm exact `--model` / `--effort` CLI semantics for current Gemini 4 and Claude 5.5 entries.
- Fix Antigravity model discovery/catalog parsing, effort vocabulary (`xhigh`, and `max` if supported), `model_effort_args`, and static fallback metadata/expectations accordingly.
- Align Role create/edit, Crew slot, and Start Chat effort controls for Antigravity (including blank/default model and leveled/base models).
- Check whether `docs/guide/` or `crates/runner-app` i18n keys need updates; keep changes scoped to `#1294`.

## Validation and handoffs

- **Automated**: unit/expectation tests in `runner-core`, `runner-daemon` (`antigravity::models::tests`, `antigravity::tests`, `ops::runtime`, catalog expectations if affected), and `runner-app` (`roles::tests`, `start_chat` controls), plus `make verify` / CI-profile Clippy and formatting (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`).
- **Handoffs**: `coder` implements and sends a frozen unpushed commit SHA to `reviewer`; `reviewer` reviews the snapshot and names the required live UI/CLI checks for `qa` under `AGENTS.md` Test Scope; `qa` acquires `script/qa-lock`, verifies the built development app against the selected checks (model & effort dropdown behavior in Start Chat, Roles, and Crew slot overrides, plus composed launch args), and releases the lock before `coder` commits/folds the brief into the PR history and opens the PR.

## Authorization

Jason approved the `agy trio` (`coder`, `reviewer`, `qa`) to fix `#1294`, commit, push branch `fix/1294-antigravity-model-effort`, open a PR against `main`, and drive CI green. Fold this brief into the implementation commit per `AGENTS.md`. Stop at the open, reviewed, QA'd, CI-green PR; do not merge, release, or clean up the worktree.
