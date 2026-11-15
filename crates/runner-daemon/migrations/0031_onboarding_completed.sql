-- #1171: first-run onboarding state is recorded in `_app_state.onboarding_completed`.
-- Ensure `_app_state` exists before the Rust backfill marks upgraded DBs that
-- already have user history (sessions, missions, or non-seed roles/crews).

CREATE TABLE IF NOT EXISTS _app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
