# Runner Docs

Project markdown lives here. Keep repo docs close to the code; the memory repo holds durable cross-session context, not project decisions — those belong in these files.

## Layout

- [`arch/`](./arch/) — architecture references: how the system works today.
- [`product/`](./product/) — product vision and direction: why we're building this, which surfaces matter.
- [`features/`](./features/README.md) — feature specs, `{number}-{slug}.md`. Since 2026-09-01 the number **is** the spec's GitHub tracking issue — file the issue first, then name the spec after it.
- [`impls/`](./impls/README.md) — implementation plans for concrete build slices; new plans named after their feature since 2026-09-01.
- [`tests/`](./tests/) — validation and human smoke-test plans, numbered by feature.

`features/` and `impls/` each have an `archive/`. A doc moves there once it stops describing current truth — a shipped spec, a superseded plan — keeping its filename so links and numbers stay stable. Anything still in the parent directory is expected to be accurate now, so the listing itself signals what is live. Before archiving a doc, move any decision it carries that outlives the work into `arch/`.
