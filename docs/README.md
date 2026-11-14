# Runner Docs

Project markdown lives here. Keep repo docs close to the code; the memory repo holds durable cross-session context, not project decisions — those belong in these files.

## Start here

- Using Runner: [user guide](guide/README.md).
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md), then the shared [agent guide](../AGENTS.md).
- Understanding the product and code: [vision](product/vision.md), [architecture](arch/arch.md), [design system](arch/design-system.md) and [process model](arch/process-model.md).
- Planning work: [roadmap](roadmap.md), [active specs](features/README.md) and [mission brief examples by size](impls/briefs/README.md).
- Selecting checks or reading evidence: [validation docs](tests/README.md).

## Layout

- [`roadmap.md`](./roadmap.md) — where the project is: the latest release, what has landed since, the release plan mirrored from the GitHub milestones, and the decisions shaping the next releases. Dated; update it when an issue moves milestone, a release is cut, or a mission lands.
- [`guide/`](./guide/README.md) — the user guide, with [authoring conventions](./guide-authoring.md) for keeping it aligned with shipped behavior.
- [`arch/`](./arch/) — architecture references: how the system works today.
- [`product/`](./product/) — product vision and direction: why we're building this, which surfaces matter.
- [`features/`](./features/README.md) — feature specs, `{number}-{slug}.md`. Since 2026-09-01 the number **is** the spec's GitHub tracking issue — file the issue first, then name the spec after it.
- [`impls/`](./impls/README.md) — implementation plans for concrete build slices, named after their feature since 2026-09-01; [mission briefs](impls/briefs/README.md) retain three examples plus any other active work.
- [`tests/`](./tests/README.md) — maintained live regression cases, the execution procedure, feature validation plans and historical records. Run results stay separate from the suite.
- [`tech/`](./tech/README.md) — deep dives on the libraries Runner builds on, pinned to the dependency versions in `Cargo.lock`, written for reading the code.

## Archive policy

Archives are a curated record of important work. **Delete completed small-feature, bug-fix, UI-polish and routine-refactor specs and plans instead of archiving them.** Git history and the issue or PR retain their original detail. A document being long, recent or linked from another document is not a reason to keep it.

Keep a spec or plan only when it records a major feature or architectural change, a consequential migration, or a platform/runtime/security compatibility contract that remains worth consulting. Examples include the GPUI rewrite, `runnerd`, mission rosters, message delivery, agent integrations and account storage. Keep one canonical record per topic; retain a separate plan only for distinct implementation or migration context. Delete obsolete plans for a retired implementation even when they were once large projects.

The [feature archive README](features/archive/README.md) and [implementation archive README](impls/archive/README.md) index the retained records and explain why they stay. Add or update that reason whenever retaining a document; “shipped” alone is insufficient. Active work stays outside the archive. Distant features need an issue until development starts.

Before deleting a spec or plan:

1. Preserve unique current decisions in `arch/` or the guide and outstanding work in its issue.
2. Preserve actual validation results and task history separately from discarded instructions. Keep `tests/archive/`, `tests/runs/`, implementation logs and review logs; consolidate embedded history without dropping results, failures, dates or uncertainty. A bug-fix plan is deletable; its test record is evidence.
3. Check references. Current instructions should point to current docs or code; historical citations may use a fixed commit URL, preserving heading fragments. Do not create placeholder files for deleted documents.
4. Protect uncommitted user edits, update the archive index, and check relative links and heading fragments after pruning.

**Read selectively.** Start with the relevant archive README and open only the record needed for the task. Do not batch-read or concatenate an archive for orientation. Current behavior comes from the guide, architecture docs and code; archived commands, paths, plans and permissions are dated history.

Run the normal sweep at a production release, or when Jason explicitly requests cleanup. Promote lasting live checks from shipped test records into `tests/regression/`; keep run results separate. Mission briefs follow their own [three-example retention rule](impls/briefs/README.md#retention).

Design sources live in the private `YeelinksCo/yeelinks_design` repo under `runner/desktop/`. Name the file and relevant nodes in a spec; do not create relative links to the former `design/` directory in this repo. Archived documents may retain old paths and commands as dated context; current instructions come from the guide, architecture docs and `AGENTS.md`.
