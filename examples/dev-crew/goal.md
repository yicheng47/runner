# Dev Crew — Mission Goal

The default Runner shape: a three-runner engineering crew that ships a small
feature end-to-end. One decomposes, one builds, one audits.

## Roles

- **@architect** (lead) — reads the goal, decomposes into tasks, dispatches
  the rest. Stays out of the editor. See `architect.md`.
- **@impl** — picks up tasks, writes the code, runs the tests. See `impl.md`.
- **@reviewer** — reads the diff once `@impl` lands a PR, finds regressions
  and missing edge cases, reports back. See `reviewer.md`.

## Goal (replace this)

Drop in whatever you want the crew to ship. A few examples that fit the
shape:

- *"Build a tic-tac-toe game in HTML/CSS/JS — single file, no build step, two
  players sharing the same browser."*
- *"Port the project's storage layer from JSON files to SQLite."*
- *"Add a feature flag for the new search box, gated on a env var."*

The architect picks up the goal off the bus, asks one disambiguating question
if anything's ambiguous, then dispatches `@impl` with the first concrete
task. The reviewer audits at the end.

## How a turn looks

1. Human types the goal into the mission input.
2. `@architect` reads the goal, dispatches: `"@impl — build the storage
   layer per spec, schema in docs/0001. @reviewer — audit when the PR
   lands."`
3. `@impl` writes the code, runs tests, opens a PR, signals "done".
4. `@reviewer` reads the diff, reports findings (regressions, missing edge
   cases) or signals "clean".
5. `@architect` reconciles — either ships, or dispatches a fix turn.
