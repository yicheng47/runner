#!/usr/bin/env bash
#
# Seed the runner DB with the Build squad crew + architect/impl/reviewer
# runners + their slots. Pulls system prompts from the sibling
# tests/fixtures/system-prompts/*.md files so a single source of truth
# survives ad-hoc edits.
#
# This shell script seeds against any DB you point it at and is the
# fixture used by manual testing. The same crew shape ships to all
# users via migration 0002_default_crew.sql — keep the two in sync.
#
# Usage:
#   ./tests/fixtures/crews/build-squad.seed.sh           # default (prod) app db
#   ./tests/fixtures/crews/build-squad.seed.sh --dev     # dev app db
#   ./tests/fixtures/crews/build-squad.seed.sh /tmp/x.db # custom path
#
# Safe to re-run: re-seeding overwrites the rows for the fixed IDs
# below. Drop the runner.db (and crews/) first if you want to start
# from a truly empty state.

set -euo pipefail

APP_SUPPORT="$HOME/Library/Application Support"
PROD_DB="$APP_SUPPORT/com.wycstudios.runner/runner.db"
DEV_DB="$APP_SUPPORT/com.wycstudios.runner-dev/runner.db"

case "${1:-}" in
  --dev) DB_PATH="$DEV_DB" ;;
  --prod|"") DB_PATH="$PROD_DB" ;;
  *) DB_PATH="$1" ;;
esac

if [[ ! -f "$DB_PATH" ]]; then
  echo "error: db not found at $DB_PATH" >&2
  echo "       launch the app once to create it, or pass a path" >&2
  exit 1
fi

FIXTURES_DIR="$(cd "$(dirname "$0")" && pwd)"
PROMPTS_DIR="$FIXTURES_DIR/../system-prompts"
# SQLite single-quote escape: a literal ' inside a string literal is
# written as ''. sed handles this consistently across bash + zsh; the
# parameter-expansion equivalent (${var//\'/\'\'}) doubles the escape
# in zsh and lands `\'\'` in the SQL.
ARCHITECT_PROMPT="$(sed "s/'/''/g" "$PROMPTS_DIR/architect.md")"
IMPL_PROMPT="$(sed "s/'/''/g" "$PROMPTS_DIR/impl.md")"
REVIEWER_PROMPT="$(sed "s/'/''/g" "$PROMPTS_DIR/reviewer.md")"

# Fixed ULID-shaped identifiers so the seed is reproducible. They share
# the `01K000DEFAULT...` prefix with migration 0002 so the migration's
# `INSERT OR IGNORE` and this script's `INSERT OR REPLACE` operate on
# the same rows.
CREW_ID="01K000DEFAULT000BUILDSQUAD01"
ARCHITECT_RUNNER_ID="01K000DEFAULT000RUNNERARCH01"
IMPL_RUNNER_ID="01K000DEFAULT000RUNNERIMPL01"
REVIEWER_RUNNER_ID="01K000DEFAULT000RUNNERREVW01"
ARCHITECT_SLOT_ID="01K000DEFAULT000SLOTARCH0001"
IMPL_SLOT_ID="01K000DEFAULT000SLOTIMPL0001"
REVIEWER_SLOT_ID="01K000DEFAULT000SLOTREVW0001"

NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Default seeded signal_types — matches db::DEFAULT_SIGNAL_TYPES so
# the seeded crew behaves like one created via the UI.
SIGNAL_TYPES_JSON='["mission_goal","human_said","ask_lead","ask_human","human_question","human_response","runner_status","inbox_read"]'

sqlite3 "$DB_PATH" <<SQL
PRAGMA foreign_keys = ON;
BEGIN;

INSERT OR REPLACE INTO crews (id, name, purpose, goal, orchestrator_policy, signal_types, created_at, updated_at)
VALUES (
  '$CREW_ID',
  'Build squad',
  'Plan, build, and review a single feature end-to-end. Architect dispatches, implementer ships, reviewer gates merge.',
  'Definition of done = code merged behind a green test suite and a clean review pass, with a one-paragraph human-readable summary posted as a broadcast.',
  NULL,
  '$SIGNAL_TYPES_JSON',
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO runners (id, handle, display_name, runtime, command, args_json, working_dir, system_prompt, env_json, model, effort, created_at, updated_at)
VALUES (
  '$ARCHITECT_RUNNER_ID',
  'architect',
  'Architect',
  'claude-code',
  'claude',
  '["--dangerously-skip-permissions"]',
  NULL,
  '$ARCHITECT_PROMPT',
  NULL,
  'claude-opus-4-7',
  'xhigh',
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO runners (id, handle, display_name, runtime, command, args_json, working_dir, system_prompt, env_json, model, effort, created_at, updated_at)
VALUES (
  '$IMPL_RUNNER_ID',
  'impl',
  'Implementation',
  'claude-code',
  'claude',
  '["--dangerously-skip-permissions"]',
  NULL,
  '$IMPL_PROMPT',
  NULL,
  'claude-opus-4-7',
  'xhigh',
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO runners (id, handle, display_name, runtime, command, args_json, working_dir, system_prompt, env_json, model, effort, created_at, updated_at)
VALUES (
  '$REVIEWER_RUNNER_ID',
  'reviewer',
  'Reviewer',
  'claude-code',
  'claude',
  '["--dangerously-skip-permissions"]',
  NULL,
  '$REVIEWER_PROMPT',
  NULL,
  'claude-opus-4-7',
  'xhigh',
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
VALUES ('$ARCHITECT_SLOT_ID', '$CREW_ID', '$ARCHITECT_RUNNER_ID', 'architect', 0, 1, '$NOW');

INSERT OR REPLACE INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
VALUES ('$IMPL_SLOT_ID', '$CREW_ID', '$IMPL_RUNNER_ID', 'impl', 1, 0, '$NOW');

INSERT OR REPLACE INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
VALUES ('$REVIEWER_SLOT_ID', '$CREW_ID', '$REVIEWER_RUNNER_ID', 'reviewer', 2, 0, '$NOW');

COMMIT;
SQL

echo "seeded Build squad crew + runners + slots into $DB_PATH"
echo "  crew:       $CREW_ID"
echo "  architect:  $ARCHITECT_RUNNER_ID (slot $ARCHITECT_SLOT_ID, lead, opus / high)"
echo "  impl:       $IMPL_RUNNER_ID (slot $IMPL_SLOT_ID, sonnet / medium)"
echo "  reviewer:   $REVIEWER_RUNNER_ID (slot $REVIEWER_SLOT_ID, sonnet / medium)"
echo ""
echo "next: launch the app and click 'Start mission' against the crew."
echo "      mission rows aren't seeded — going through the real"
echo "      mission_start path keeps the event log + sidecars in"
echo "      lockstep with the Rust code's expectations."
