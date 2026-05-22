# Docs Crew — Mission Goal

A documentation crew for an existing, complex codebase. One architect
partitions the repo into modules and assembles the final doc set; two (or
more) writers work modules in parallel; one editor harmonizes the result.

## Roles

- **@architect** (lead) — surveys the repo, decides the doc structure
  (per-module pages + an index), dispatches writers with concrete file
  scopes, then assembles the index and cross-links. Stays out of the
  prose. See `architect.md`.
- **@writer-a**, **@writer-b** — pick up one module at a time, read the
  code carefully, write the module doc. Same persona template, different
  slot handles so the architect can dispatch to them independently. See
  `writer.md`. Add `@writer-c`, `@writer-d`, etc. by adding more slots
  pointing at the same `writer` runner template.
- **@editor** — once writers land their drafts, reads the full doc set
  against the code, finds gaps, broken cross-refs, voice drift, and
  claims that don't match what the code actually does. See `editor.md`.

## Goal (replace this)

Drop in whatever you want the crew to document. Examples that fit the
shape:

- *"Document the runner backend end-to-end — one page per top-level
  module in `src-tauri/src/`, plus an index. Output under `docs/code/`."*
- *"Write a contributor guide for the `cli/` crate: every public command,
  every flag, with worked examples."*
- *"Audit and rewrite the existing docs under `docs/impls/` to match the
  current code — flag any doc whose claims have drifted from the
  implementation."*

The architect surveys the target, proposes a partition (one module per
writer), asks one disambiguating question if the scope is unclear, then
dispatches the first round of writer assignments. The editor sweeps once
drafts are in.

## How a turn looks

1. Human types the goal into the mission input (target path, output
   location, depth — "API reference" vs "narrative guide").
2. `@architect` reads the repo top-down, posts the partition as a short
   table (module → writer → output file), then dispatches:
   `"@writer-a — document src-tauri/src/session/ into docs/code/session.md
    per the partition. @writer-b — document src-tauri/src/router/ into
    docs/code/router.md. @editor — sweep once both writers land."`
3. Each writer reads their assigned module, drafts the page, signals
   "done" with the file path.
4. `@editor` reads the full set, posts findings (gaps, drift, broken
   cross-refs) or signals "clean".
5. `@architect` reconciles — assembles `docs/code/index.md` with the
   cross-links, or dispatches a fix turn back to the relevant writer.

## Scaling

This shape assumes ~2 writers, but the slot redesign means you can add
more writers (`@writer-c`, `@writer-d`) without changing the persona
files — just add more slots pointing at the same `writer` runner
template. The architect's partition table is the only thing that
changes; it grows a row per writer.
