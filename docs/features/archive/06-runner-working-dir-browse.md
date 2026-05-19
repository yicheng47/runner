# 06 — Runner working directory: system browse

> Tracking issue: [#77](https://github.com/yicheng47/runner/issues/77)

## Motivation

The runner template form (`CreateRunnerModal`, `RunnerEditDrawer`) asks
users to type an absolute path into the "Working directory" field by
hand. Two failure modes follow from that:

1. **Typos and stale paths.** Users paste paths from the terminal,
   transpose a directory, or carry a path forward from a project they
   moved. The field validates only on save (or not at all), so the
   first signal something is wrong is a runner that won't spawn — which
   surfaces deep inside the mission lifecycle, not at edit time.
2. **It's the odd one out.** The mission-launch surface
   (`StartMissionModal`) already has a "Browse…" button next to its
   working-directory input that calls
   `@tauri-apps/plugin-dialog`'s `open({ directory: true })`. Users who
   set up a mission first and then come back to create a runner expect
   the same affordance and get a bare textbox instead.

Adding a system folder picker fixes both: the path comes from the OS
dialog, so it's guaranteed to exist and to be absolute, and the runner
form matches the mission form's pattern so the surface feels coherent.

## Scope

### In scope (v1)

- **`CreateRunnerModal` "Working directory" field**: add a "Browse…"
  button to the right of the existing `<Input>`, mirroring the layout
  in `StartMissionModal` (input grows, button is fixed-width).
- **`RunnerEditDrawer` "Working directory" field**: same treatment so
  edit flows match create flows.
- **Picker behaviour**: `openDialog({ directory: true, multiple: false,
  title: "Pick a working directory" })`. On success, write the picked
  path into the existing `workingDir` state. On cancel, no-op. On
  error, surface via the existing `error` field on each modal.
- **Free-text still works.** The picker writes into the same input the
  user can still type into. Users who prefer to paste paths or use
  `~`-shorthand (rejected today, but still valid intent) keep that
  flow.
- **Default starting directory**: leave unset. The OS dialog opens
  wherever it was last left, which matches macOS conventions and
  matches `StartMissionModal`.

### Out of scope (deferred)

- **`~` expansion.** The path written by the picker is always
  absolute, so the picker path itself doesn't need this. Tilde
  expansion for typed paths is a separate, broader question (does the
  backend expand or does the frontend?) and isn't gated by this work.
- **Path validation / existence check on blur.** The picker
  guarantees existence at pick time; typed paths could go stale
  between save and spawn. Adding a validate-on-blur is a follow-up if
  it shows up as a real problem.
- **Recent / pinned working directories dropdown.** Could be useful
  for users who run many runners against the same repo, but the
  StartMission form has lived without this and there's no signal yet
  it's missed.
- **Settings or system-prompt picker surfaces.** This spec is the
  runner working-dir field only.

### Key decisions

1. **Match `StartMissionModal`'s layout exactly.** A trailing
   `Browse…` button to the right of the input, same gap, same
   button styling. Two surfaces with the same field should render
   identically — no "this one has Browse… below the field, the other
   one to the right" inconsistency.
2. **Don't replace the text input.** A pure "Browse" button would lock
   out paste-from-clipboard, which is how power users move paths. The
   browse button is additive.
3. **Reuse the existing `Field` / `Input` / `Button` primitives.** The
   `StartMissionModal` version uses raw `<input>` markup; this work
   should use `Field` + `Input` (already in `CreateRunnerModal`) so we
   don't fork the component vocabulary just to add a button.
4. **No new shared component.** Two callsites is below the threshold
   for extracting a `<DirectoryInput>` wrapper. If a third surface
   needs this we'll revisit.

## Implementation phases

### Phase 1 — `CreateRunnerModal`

- Import `open as openDialog` from `@tauri-apps/plugin-dialog`.
- Add a `browseWorkingDir` async helper inside the component that
  calls `openDialog({ directory: true, multiple: false, title: "Pick
  a working directory" })`, sets `workingDir` on success, and writes
  to the existing `setError` on failure.
- Replace the bare `<Input>` in the working-directory `Field` with a
  flex row containing the `<Input>` and a trailing
  `<Button onClick={() => void browseWorkingDir()}>Browse…</Button>`.
- Make sure the input still grows to fill the row (flex-1, min-w-0)
  and the button is `disabled` while submitting.

### Phase 2 — `RunnerEditDrawer`

- Same change as Phase 1 against `RunnerEditDrawer.tsx`. The state
  hook (`workingDir` / `setWorkingDir`) already exists and is wired
  to the API payload, so the only edit is the markup + the
  browseWorkingDir handler.

### Phase 3 — verify across both flows

- Smoke create-runner with the picker; confirm the spawned runner's
  PTY actually starts in the picked directory.
- Smoke edit-runner with the picker; confirm the edit persists and
  that the picker pre-populates from the existing value (it doesn't
  need to — we don't pass `defaultPath` in StartMissionModal either —
  but verify behaviour is consistent across the three surfaces).
- Visual diff against `StartMissionModal` to confirm the row layout
  matches.

## Verification

- [ ] In `CreateRunnerModal`, clicking "Browse…" opens the OS folder
      picker; picking a folder writes the absolute path into the
      input.
- [ ] In `CreateRunnerModal`, cancelling the picker leaves the input
      unchanged; no error toast.
- [ ] In `CreateRunnerModal`, typing a path manually still works.
- [ ] Same three checks pass against `RunnerEditDrawer`.
- [ ] Both surfaces render the row with the same layout as
      `StartMissionModal` (input fills, "Browse…" button at the
      right, same gap and disabled-during-submit behaviour).
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
