// Thin invoke wrappers for Tauri commands.
//
// Tauri auto-converts top-level arg names between camelCase (JS) and
// snake_case (Rust), so `crewId` here maps to `crew_id` in the Rust
// handlers. Nested struct fields pass through unchanged — `input` objects
// match the Rust struct shapes in src-tauri/src/commands/{crew,runner,slot,mission,session}.rs,
// mirrored by src/lib/types.ts.

import { invoke } from "@tauri-apps/api/core";

import type {
  CreateCrewInput,
  CreateRunnerInput,
  CreateSlotInput,
  Crew,
  CrewListItem,
  CrewMembership,
  Event,
  Mission,
  MissionSummary,
  PostHumanSignalInput,
  Runner,
  RunnerActivity,
  RunnerWithActivity,
  Session,
  SessionOutputEvent,
  SlotWithRunner,
  SpawnedSession,
  StartMissionInput,
  StartMissionOutput,
  UpdateCrewInput,
  UpdateRunnerInput,
  UpdateSlotInput,
} from "./types";

/** Session row joined with the runner's handle for UI labels. */
export interface SessionRow extends Session {
  handle: string;
  lead: boolean;
}

/**
 * Sidebar SESSION row: one entry per un-archived direct session. Multi
 * chat per runner — see docs/impls/0003-direct-chats.md — so the list is
 * flat. Stopped/crashed sessions stay listed because they can be
 * resumed via session_resume (which respawns the same row's PTY).
 */
export interface DirectSessionEntry {
  session_id: string;
  runner_id: string;
  handle: string;
  status: "running" | "stopped" | "crashed";
  title: string | null;
  cwd: string | null;
  started_at: string | null;
  stopped_at: string | null;
  resumable: boolean;
  pinned: boolean;
  // Set when the session has been archived. `listRecentDirect` filters
  // these at SQL so rows from that endpoint always have `archived_at:
  // null`. `session_get` is the unfiltered lookup the chat page uses
  // to detect an archived direct-URL navigation and lock the
  // workspace read-only.
  archived_at: string | null;
}

export const api = {
  crew: {
    list: () => invoke<CrewListItem[]>("crew_list"),
    get: (id: string) => invoke<Crew>("crew_get", { id }),
    create: (input: CreateCrewInput) => invoke<Crew>("crew_create", { input }),
    update: (id: string, input: UpdateCrewInput) =>
      invoke<Crew>("crew_update", { id, input }),
    delete: (id: string) => invoke<void>("crew_delete", { id }),

  },
  slot: {
    list: (crewId: string) =>
      invoke<SlotWithRunner[]>("slot_list", { crewId }),
    create: (input: CreateSlotInput) =>
      invoke<SlotWithRunner>("slot_create", { input }),
    update: (slotId: string, input: UpdateSlotInput) =>
      invoke<SlotWithRunner>("slot_update", { slotId, input }),
    delete: (slotId: string) => invoke<void>("slot_delete", { slotId }),
    setLead: (slotId: string) =>
      invoke<SlotWithRunner>("slot_set_lead", { slotId }),
    reorder: (crewId: string, orderedSlotIds: string[]) =>
      invoke<SlotWithRunner[]>("slot_reorder", { crewId, orderedSlotIds }),
  },
  runner: {
    list: () => invoke<Runner[]>("runner_list"),
    listWithActivity: () =>
      invoke<RunnerWithActivity[]>("runner_list_with_activity"),
    get: (id: string) => invoke<Runner>("runner_get", { id }),
    getByHandle: (handle: string) =>
      invoke<Runner>("runner_get_by_handle", { handle }),
    create: (input: CreateRunnerInput) =>
      invoke<Runner>("runner_create", { input }),
    update: (id: string, input: UpdateRunnerInput) =>
      invoke<Runner>("runner_update", { id, input }),
    delete: (id: string) => invoke<void>("runner_delete", { id }),
    activity: (id: string) => invoke<RunnerActivity>("runner_activity", { id }),
    crews: (runnerId: string) =>
      invoke<CrewMembership[]>("runner_crews_list", { runnerId }),
  },
  mission: {
    list: (crewId?: string) =>
      invoke<Mission[]>("mission_list", crewId ? { crewId } : {}),
    listSummary: (crewId?: string) =>
      invoke<MissionSummary[]>(
        "mission_list_summary",
        crewId ? { crewId } : {},
      ),
    get: (id: string) => invoke<Mission>("mission_get", { id }),
    start: (input: StartMissionInput) =>
      invoke<StartMissionOutput>("mission_start", { input }),
    /** Re-mount router/bus on workspace mount; idempotent. After app restart
     *  the in-memory router/bus need to be rebuilt from the persisted log
     *  before stdin pushes can land on resumed slot PTYs. */
    attach: (missionId: string) =>
      invoke<Mission>("mission_attach", { missionId }),
    /** Kill every live PTY in the mission. Mission row stays `running`,
     *  router/bus stay mounted; pair with per-slot `session_resume` to
     *  bring the agents back. */
    stop: (id: string) => invoke<Mission>("mission_stop", { id }),
    /** Terminal end-of-mission. Drops router + bus, flips status to
     *  `completed`. Cannot be undone. */
    archive: (id: string) => invoke<Mission>("mission_archive", { id }),
    /** Wipe the run context and respawn every slot. Mostly for
     *  testing — keeps the mission row, crew, and slots intact;
     *  drops the event log, agent session keys, and router state. */
    reset: (id: string) => invoke<Mission>("mission_reset", { id }),
    /** Toggle a mission's pin. Pinned missions float to the top of
     *  the sidebar's MISSION list. */
    pin: (id: string, pinned: boolean) =>
      invoke<Mission>("mission_pin", { id, pinned }),
    /** Rename a mission. Title is trimmed and rejected if empty. */
    rename: (id: string, title: string) =>
      invoke<Mission>("mission_rename", { id, title }),
    eventsReplay: (missionId: string) =>
      invoke<Event[]>("mission_events_replay", { missionId }),
    postHumanSignal: (input: PostHumanSignalInput) =>
      invoke<Event>("mission_post_human_signal", { input }),
  },
  session: {
    list: (missionId: string) =>
      invoke<SessionRow[]>("session_list", { missionId }),
    listRecentDirect: () =>
      invoke<DirectSessionEntry[]>("session_list_recent_direct"),
    /** Unfiltered single-row lookup (includes archived rows). Used by
     *  RunnerChat to detect an archived direct-URL navigation so the
     *  workspace can render read-only. Returns null if the id is
     *  unknown or belongs to a mission session. */
    get: (sessionId: string) =>
      invoke<DirectSessionEntry | null>("session_get", { sessionId }),
    archive: (sessionId: string) =>
      invoke<void>("session_archive", { sessionId }),
    rename: (sessionId: string, title: string | null) =>
      invoke<void>("session_rename", { sessionId, title }),
    pin: (sessionId: string, pinned: boolean) =>
      invoke<void>("session_pin", { sessionId, pinned }),
    resume: (
      sessionId: string,
      cols: number | null,
      rows: number | null,
    ) =>
      invoke<SpawnedSession>("session_resume", {
        sessionId,
        cols,
        rows,
      }),
    injectStdin: (sessionId: string, text: string) =>
      invoke<void>("session_inject_stdin", { sessionId, text }),
    kill: (sessionId: string) => invoke<void>("session_kill", { sessionId }),
    resize: (sessionId: string, cols: number, rows: number) =>
      invoke<void>("session_resize", { sessionId, cols, rows }),
    outputSnapshot: (sessionId: string) =>
      invoke<SessionOutputEvent[]>("session_output_snapshot", { sessionId }),
    pasteImage: (bytes: Uint8Array) =>
      invoke<void>("session_paste_image", {
        bytes: Array.from(bytes),
      }),
    startDirect: (
      runnerId: string,
      cwd: string | null,
      cols: number | null,
      rows: number | null,
    ) =>
      invoke<SpawnedSession>("session_start_direct", {
        runnerId,
        cwd,
        cols,
        rows,
      }),
  },
};
