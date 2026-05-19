# 20 — Drop the per-crew signal allowlist

> Tracking issue: [#168](https://github.com/yicheng47/runner/issues/168)

## Motivation

The crew row carries a `signal_types` JSON array — the set of signal type strings the bundled `runner` CLI will accept when an agent emits `runner signal <type>`. The default seed covers every built-in type the router actually handles (`mission_goal`, `human_said`, `ask_lead`, `ask_human`, `human_question`, `human_response`, `runner_status`, `inbox_read`), and we've never used the column for anything else: no crew template shipped customizes it, no user-defined signal types are wired up in the router, and the UI doesn't expose it.

In other words, this is a low-level protocol concept that:

1. The user never sees and never has to think about.
2. The router doesn't gate behavior on (only the CLI does, at validation time).
3. Adds machinery — a per-crew DB column, a per-crew JSON sidecar at `$APPDATA/runner/crews/{crew_id}/signal_types.json`, a CLI-time allowlist check, an `allowlist` module in `runner-cli` — for no product benefit.

If a user ever wanted to define a custom signal type (out of scope today), the router would also need to grow handlers for it — at which point the gating happens at the *router*, not at the CLI's input validation. The allowlist as it exists is a guardrail against typos, nothing more, and we can replace that guardrail with a fixed code-side enum.

## Scope

### Drop

1. **DB column.** Add a migration that removes `crews.signal_types`.
2. **JSON sidecar.** Remove the `$APPDATA/runner/crews/{crew_id}/signal_types.json` write at mission start, the `event_log::path::signal_types_path()` helper, and the seed/sync logic.
3. **CLI validation.** Remove `cli/src/allowlist.rs` and its caller in `cli/src/signal.rs`. The CLI no longer reads the sidecar.
4. **Backend/UI surface.** Remove `signal_types` from `model::Crew`, `commands::crew::{create, update, get, list}`, and the frontend types in `src/lib/types.ts`. Remove the (currently hidden) UI for editing it, if any.
5. **Arch doc.** The `signal_types.json` line in §9.2 Filesystem; the "crew has a signal-type allowlist" mention in §3.3 (already trimmed); the §3.4 "Signal allowlist" dedicated section (already removed when this feature was scoped).

### Replace (validation-side)

Replace the file-backed allowlist check in the CLI with a **code-side enum** of recognized signal types in `runner-core`. The CLI rejects unknown strings with a clear stderr message — same UX as today, but the source of truth lives in code, not in a DB column + sidecar file.

This means: adding a new built-in signal type is a one-line code change in `runner-core` plus a router handler in `src-tauri/src/router/`. There is no "now also update the seed DEFAULT and every existing crew's allowlist" step.

### Keep

- The `runner signal <type>` CLI verb shape — unchanged.
- All existing signal types — they remain valid via the new code-side enum.
- The router's handler dispatch — unchanged. The router has always matched on the enum, not on the allowlist.

## Implementation phases

### Phase 1 — code-side enum
- Add `runner_core::signal::Type` (or extend the existing one) so it enumerates every recognized signal type.
- Wire `cli/src/signal.rs` to reject unknown strings against that enum, replacing the allowlist file read.
- Delete `cli/src/allowlist.rs`. Update `cli/tests/roundtrip.rs` fixtures accordingly.

### Phase 2 — drop the sidecar
- Delete `event_log::path::signal_types_path()` and its uses in `commands::mission::mission_start` (the sidecar write).
- Remove the test `layout_matches_arch_section_*` assertion for it.
- Clean up any stale sidecar files at app startup (one-time best effort; ignore failures).

### Phase 3 — drop the DB column
- Migration `000N_drop_crews_signal_types.sql`: `ALTER TABLE crews DROP COLUMN signal_types;` (SQLite supports this directly in 3.35+; bundled rusqlite is fine).
- Remove `signal_types` from `model::Crew`, the SELECT/INSERT/UPDATE in `commands::crew::*`, and any DTO in `src/lib/types.ts`.
- Update the SQL block in arch §9.1.

### Phase 4 — doc cleanup
- Remove the `signal_types.json` line from arch §9.2.
- Confirm no remaining references to the allowlist in arch.md, vision.md, or feature specs.

## Verification

- **Unit:** Removed allowlist module's tests are gone; new enum rejection test covers "unknown signal type → non-zero exit + clear stderr."
- **Integration:** `mission_start` no longer creates the sidecar file (assert on the directory contents).
- **Manual smoke:**
  1. Spawn a fresh mission; the `signal_types.json` file does not appear under `$APPDATA/runner/crews/<id>/`.
  2. `runner signal mission_goal` (a real type) succeeds.
  3. `runner signal made_up_type` fails with a clear error.
  4. Restarting the app and reopening the live mission works (no missing-sidecar crash).

## Notes

This feature is purely a cleanup; nothing in the product changes from the user's POV. The win is fewer moving parts: one place to add a signal type (the enum) instead of three (enum, DB DEFAULT, sidecar writer).
