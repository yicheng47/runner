# Local Mission Control — Stage 1

This local Markdown ticket is the tracker for Stage 1. It intentionally has no
GitHub issue: the board reads project-local evidence and does not create or
mirror tracker records.

## Outcome

Runner gains a project-scoped native Mission Control board. One read-only
daemon snapshot joins local ticket and triage evidence with Runner's live
mission state, so the desktop can show which work is waiting, active, blocked,
or ready for verification without inventing workflow state.

The first client is the native application. The typed snapshot is deliberately
client-neutral so a future local web client can consume the same daemon result.

## Sources and provenance

The adapter reads, but never writes:

- `issues/CR-*.md`, `issues/README.md`, and ticket archives;
- `.crew/triage/`;
- Runner's mission, slot, feed, pending-question, branch and worktree state.

The protocol exposes a typed `MissionControlSnapshot`, source records and
per-field provenance. Each card keeps source-evidence links rather than copied
ticket bodies in Runner storage. Missing, malformed, stale or inaccessible
inputs produce source-scoped health warnings and partial results; they never
make the whole board fail or create guessed ticket state.

## Derived cards and lanes

Cards are grouped in a compact, horizontally scrolling seven-lane board. Every
lane remains visible when empty and shows its derived-state color, label and
count. A card emphasizes its lane, ticket ID/severity/triage chips and title;
it keeps a real pending human ask prominent in **Needs you**, with active slots,
branch/worktree, handoff and contributing evidence as terse secondary rows and
clickable source chips. The board remains read-only.

Lane assignment is derived only, using this order:

1. **Closed** — the canonical tracker or archive says closed.
2. **Needs you** — a linked Runner mission has a genuine pending question.
3. **Running** — an active slot or running session exists.
4. **Verify / Land** — explicit evidence says verification, review or landing
   awaits after work settled.
5. **Blocked** — tracker or triage evidence explicitly names a blocker.
6. **Ready** — the tracker explicitly says ready and no work is active.
7. **Backlog** — any other open ticket.

`Needs you` is never inferred from a stale handoff, a blocked ticket, an
unread message or a status string; only a pending Runner question can set it.

Selecting a linked card opens the existing Mission Workspace/feed. The board
does not replace that workspace.

## Non-goals

Stage 1 adds no ticket or triage edits, ticket creation, dispatch, retry,
promotion, landing, drag/drop, question creation/answer/dismissal, worktree or
branch mutation, web monitor, or copied ticket content in Runner storage.
Kanbots remains an interaction reference only: this work adds neither its
SQLite issue store, autopilot nor a second worktree system.

## Implementation slices

1. Add the typed protocol model, source health/provenance types and client
   request/response plumbing.
2. Implement the read-only project Markdown and triage adapter, with fixtures
   for valid, absent, malformed and stale sources.
3. Join adapter records with mission summaries, slots, feeds, pending questions
   and worktree facts in `runnerd`; make lane precedence explicit and tested.
4. Add the native board route/surface and its navigation to Mission Workspace.
   The private Pen repository is unavailable for this work; use AffectUI's
   [agent components](https://www.affectui.com/category/agents) and
   [design-system direction](https://www.affectui.com/design-system) only as
   visual references. Preserve Runner's native GPUI patterns rather than
   importing web components or reproducing their mutation controls.
5. Update user guidance for the board's read-only evidence model and run the
   selected native checks.

## Validation plan

Automated coverage proves protocol compatibility, provenance retention, lane
precedence, malformed/unavailable partial results, and the native route's
rendering/navigation. Focused `runner-core`, `runner-daemon`, `runner-cli` and
`runner-app` tests cover their changed behavior, followed by formatting and
workspace Clippy appropriate to the final diff.

After independent review, QA acquires `script/qa-lock` and runs the reviewed
candidate in the development app. The selected checks cover: ticket-only
projects; active missions, slots and worktrees; a genuine pending question;
an empty project; malformed or unavailable ticket/triage inputs; navigation to
the existing Mission Workspace/feed; and absence of mutation controls. QA
records the candidate identity, each verdict and screenshots in the test
record before removing `needs-live-test`.
