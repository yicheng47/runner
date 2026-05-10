# Feature delivery crew

Sample crew fixture for end-to-end testing of the slot redesign + mission
lifecycle. Mirrors the shape a real user would set up for shipping a
contained feature against an existing codebase.

## Crew row

- **Name:** Feature delivery
- **Purpose:** Ship a single, well-scoped feature against an existing
  codebase. Architect plans, two implementers split the work, reviewer
  gates the merge.
- **Goal (default, overridable per mission):** Ship the feature described
  in `mission_goal`. Definition of done = code merged behind a green test
  suite, with a reviewer LGTM and a one-paragraph human-readable summary
  posted as a broadcast.

## Slots

Use the prompts in `tests/fixtures/system-prompts/` as each slot's runner
template `system_prompt`. The same `architect` runner template can be
reused across crews; only the `slot_handle` differs in-crew.

| slot_handle | runner template | runtime | lead | system_prompt fixture |
|-------------|-----------------|---------|------|-----------------------|
| `architect` | `architect`     | claude-code | yes | `system-prompts/architect.md` |
| `designer`  | `designer`      | claude-code | no  | `system-prompts/designer.md` |
| `impl-a`    | `impl`          | claude-code | no  | `system-prompts/impl.md` |
| `impl-b`    | `impl`          | claude-code | no  | `system-prompts/impl.md` |
| `reviewer`  | `reviewer`      | claude-code | no  | `system-prompts/reviewer.md` |

The two implementer slots both reference the same `impl` runner template
— this is the slot redesign's defining feature. They share runtime and
brief; only their slot_handle differs, so the architect can dispatch to
`@impl-a` and `@impl-b` independently.

The `designer` slot uses Pencil (`.pen` files via the `pencil` MCP) and
expects to read/write designs under `design/` at the repo root. On
missions with a UI surface the architect dispatches a design task to
`@designer` first, then implementation to `@impl-a` / `@impl-b`
referencing the resulting frames. Pure-backend missions (like the
suggested first mission below) leave the designer idle.

## Allowed signal types (default)

The DB seeds new crews with the v0 default allowlist; no override needed:

```
mission_goal, human_said, ask_lead, ask_human, human_question,
human_response, runner_status, inbox_read
```

## Suggested first mission

- **Title:** Smoke: dual-impl handoff
- **Goal:** Add a `--dry-run` flag to `cli/src/main.rs::msg_post` that
  prints the envelope it would append and exits 0 without touching the
  log. Cover with a unit test in `cli/tests/roundtrip.rs`.
- **Working directory:** the repo root.

Expected coordination shape:

1. Architect receives `mission_goal`, splits into:
   - `@impl-a`: thread the flag through `MsgPostArgs` + main.
   - `@impl-b`: add the test.
2. Both implementers report completion as directed messages.
3. Architect dispatches review to `@reviewer`.
4. Reviewer posts `LGTM` (or change list) back to `@architect`.
5. Architect broadcasts the summary; mission_stop.

This exercises: lead launch prompt with two same-template slots, directed
routing by slot_handle, `ask_lead` ↔ lead reply path, and the reviewer's
verdict pattern.
