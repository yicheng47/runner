# Design

Pencil (`.pen`) source files for the Runners UI. Open these in the Pencil app; they are encrypted on disk and not meant to be read as plain text.

Conventions:
- One `.pen` file per major surface (e.g. `home.pen`, `crew-editor.pen`, `runner-card.pen`).
- Exports land in `/assets/design/` once we need them in-app.

Follow-ups parked in design (no issue yet):
- Tooltip primitive (kebab-menu language): `runners-design.pen` node `sE5dM`. Spec'd during issue #34 Phase 4 but deferred — v1 ships with native `title`. Pick this up when a sidebar/rail tooltip primitive is genuinely needed.
