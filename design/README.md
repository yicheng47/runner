# Design

Pencil (`.pen`) source files for the Runners UI. Open these in the Pencil app; they are encrypted on disk and not meant to be read as plain text.

Conventions:

- One `.pen` file per major surface (e.g. `home.pen`, `crew-editor.pen`, `runner-card.pen`).
- `runner-mvp-design.pen` is the historical MVP canvas; do not add new feature work to it.
- `chat-attention-indicators.pen` frame `R4LJz` contains the issue #285 working, unread, and collapsed-rollup states.
- Exports land in `/assets/design/` once we need them in-app.

Follow-ups parked in design (no issue yet):

- Tooltip primitive (kebab-menu language): `runner-mvp-design.pen` node `sE5dM`. Spec'd during issue #34 Phase 4 but deferred — v1 ships with native `title`. Pick this up when a sidebar/rail tooltip primitive is genuinely needed.
