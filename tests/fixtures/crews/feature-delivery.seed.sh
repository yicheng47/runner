#!/usr/bin/env bash
#
# Seed the runner DB with the feature-delivery crew + architect/impl
# runners + their slots. Pulls system prompts from the sibling
# tests/fixtures/system-prompts/*.md files so a single source of truth
# survives ad-hoc edits.
#
# Usage:
#   ./tests/fixtures/crews/feature-delivery.seed.sh           # default app db
#   ./tests/fixtures/crews/feature-delivery.seed.sh /tmp/x.db # custom path
#
# Safe to re-run: re-seeding overwrites the rows for the fixed IDs
# below. Drop the runner.db (and crews/) first if you want to start
# from a truly empty state.

set -euo pipefail

DEFAULT_DB="$HOME/Library/Application Support/com.wycstudios.runner/runner.db"
DB_PATH="${1:-$DEFAULT_DB}"

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

# Fixed ULIDs so the seed is reproducible. These do not collide with
# real-world IDs (their timestamp prefix `01K000FIXTURE…` is a
# placeholder; the suffix is random padding to satisfy ULID's 26-char
# Crockford base32 shape).
CREW_ID="01K000FIXTURE000FEATUREDLV01"
ARCHITECT_RUNNER_ID="01K000FIXTURE000RUNNERAR0001"
IMPL_RUNNER_ID="01K000FIXTURE000RUNNERIMPL01"
ARCHITECT_SLOT_ID="01K000FIXTURE000SLOTARCH0001"
IMPL_SLOT_ID="01K000FIXTURE000SLOTIMPL0001"

NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
DEFAULT_CWD="${RUNNER_FIXTURE_CWD:-$HOME/go/src/github.com/yicheng47}"

# Default seeded signal_types — matches db::DEFAULT_SIGNAL_TYPES so
# the seeded crew behaves like one created via the UI.
SIGNAL_TYPES_JSON='["mission_goal","human_said","ask_lead","ask_human","human_question","human_response","runner_status","inbox_read"]'

sqlite3 "$DB_PATH" <<SQL
PRAGMA foreign_keys = ON;
BEGIN;

INSERT OR REPLACE INTO crews (id, name, purpose, goal, orchestrator_policy, signal_types, created_at, updated_at)
VALUES (
  '$CREW_ID',
  'Feature delivery',
  'Ship a single, well-scoped feature against an existing codebase. Architect plans, one implementer ships the work.',
  'Definition of done = code merged behind a green test suite, with a one-paragraph human-readable summary posted as a broadcast.',
  NULL,
  '$SIGNAL_TYPES_JSON',
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO runners (id, handle, display_name, runtime, command, args_json, working_dir, system_prompt, env_json, created_at, updated_at)
VALUES (
  '$ARCHITECT_RUNNER_ID',
  'architect',
  'Architect',
  'claude-code',
  'claude',
  '[]',
  '$DEFAULT_CWD',
  '$ARCHITECT_PROMPT',
  NULL,
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO runners (id, handle, display_name, runtime, command, args_json, working_dir, system_prompt, env_json, created_at, updated_at)
VALUES (
  '$IMPL_RUNNER_ID',
  'impl',
  'Implementation',
  'claude-code',
  'claude',
  '[]',
  '$DEFAULT_CWD',
  '$IMPL_PROMPT',
  NULL,
  '$NOW',
  '$NOW'
);

INSERT OR REPLACE INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
VALUES ('$ARCHITECT_SLOT_ID', '$CREW_ID', '$ARCHITECT_RUNNER_ID', 'architect', 0, 1, '$NOW');

INSERT OR REPLACE INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
VALUES ('$IMPL_SLOT_ID', '$CREW_ID', '$IMPL_RUNNER_ID', 'impl', 1, 0, '$NOW');

COMMIT;
SQL

echo "seeded feature-delivery crew + runners + slots into $DB_PATH"
echo "  crew:       $CREW_ID"
echo "  architect:  $ARCHITECT_RUNNER_ID (slot $ARCHITECT_SLOT_ID, lead)"
echo "  impl:       $IMPL_RUNNER_ID (slot $IMPL_SLOT_ID)"
echo "  cwd:        $DEFAULT_CWD"
echo ""
echo "next: launch the app and click 'Start mission' against the crew."
echo "      mission rows aren't seeded — going through the real"
echo "      mission_start path keeps the event log + sidecars in"
echo "      lockstep with the Rust code's expectations."
