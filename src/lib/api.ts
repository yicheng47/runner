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
  Subject,
  SessionActivityState,
  UpdateCrewInput,
  UpdateRunnerInput,
  UpdateSlotInput,
  WindowEntry,
} from "./types";

/** Session row joined with the runner's handle for UI labels. */
export interface SessionRow extends Session {
  handle: string;
  /** Runner-side `runtime` kind, denormalized onto the row for
   *  per-runtime UX decisions (e.g. clear-on-resize for full-screen
   *  TUI agents). See docs/impls/archive/0011 §"Per-runtime clear-on-resize". */
  runtime: string;
  lead: boolean;
  agent_session_key: string | null;
}

/**
 * Sidebar SESSION row: one entry per un-archived direct session. Multi
 * chat per runner — see docs/impls/archive/0003-direct-chats.md — so the list is
 * flat. Stopped/crashed sessions stay listed because they can be
 * resumed via session_resume (which respawns the same row's PTY).
 */
export interface DirectSessionEntry {
  session_id: string;
  project_id: string | null;
  runner_id: string | null;
  handle: string | null;
  agent_runtime: string;
  agent_command: string;
  display_name: string;
  status: "running" | "stopped" | "crashed";
  title: string | null;
  cwd: string | null;
  started_at: string | null;
  stopped_at: string | null;
  resumable: boolean;
  agent_session_key: string | null;
  pinned: boolean;
  // Set when the session has been archived. `listRecentDirect` filters
  // these at SQL so rows from that endpoint always have `archived_at:
  // null`. `session_get` is the unfiltered lookup the chat page uses
  // to detect an archived direct-URL navigation and lock the
  // workspace read-only.
  archived_at: string | null;
}

export type PasteImageMimeType = "image/png" | "image/jpeg";

export interface FolderRow {
  id: string;
  name: string;
  position: number;
  created_at: string;
}

export interface ProjectRow {
  id: string;
  name: string;
  cwd: string;
  position: number;
  created_at: string;
}

export interface TabRow {
  id: string;
  folder_id: string | null;
  name: string;
  position: number;
  layout: string;
  created_at: string;
  last_completed_at: string | null;
  last_viewed_at: string | null;
}

export interface TabUpsertInput {
  id: string;
  folder_id: string | null;
  name: string;
  position: number;
  layout: string;
}

export interface TabImportInput {
  name: string;
  position: number;
  layout: string;
}

export interface RuntimeDefinition {
  name: string;
  display_name: string;
  command: string;
}

export const api = {
  project: {
    list: () => invoke<ProjectRow[]>("project_list"),
    create: (name: string, cwd: string) =>
      invoke<ProjectRow>("project_create", { name, cwd }),
    rename: (id: string, name: string) =>
      invoke<ProjectRow>("project_rename", { id, name }),
    setCwd: (id: string, cwd: string) =>
      invoke<ProjectRow>("project_set_cwd", { id, cwd }),
    reorder: (orderedIds: string[]) =>
      invoke<ProjectRow[]>("project_reorder", { orderedIds }),
    delete: (id: string) => invoke<void>("project_delete", { id }),
  },
  folder: {
    list: () => invoke<FolderRow[]>("folder_list"),
    create: (name: string) => invoke<FolderRow>("folder_create", { name }),
    rename: (id: string, name: string) =>
      invoke<FolderRow>("folder_rename", { id, name }),
    reorder: (orderedIds: string[]) =>
      invoke<FolderRow[]>("folder_reorder", { orderedIds }),
    delete: (id: string) => invoke<void>("folder_delete", { id }),
  },
  tab: {
    list: () => invoke<TabRow[]>("tab_list"),
    upsert: (input: TabUpsertInput) =>
      invoke<TabRow>("tab_upsert", { input }),
    delete: (id: string) => invoke<void>("tab_delete", { id }),
    moveToFolder: (id: string, folderId: string | null) =>
      invoke<TabRow>("tab_move_to_folder", { id, folderId }),
    reorder: (id: string, folderId: string | null, orderedIds: string[]) =>
      invoke<TabRow[]>("tab_reorder", { id, folderId, orderedIds }),
    importOnce: (tabs: TabImportInput[]) =>
      invoke<TabRow[]>("tab_import_once", { tabs }),
    markViewed: (id: string, memberIds: string[]) =>
      invoke<TabRow>("tab_mark_viewed", { id, memberIds }),
  },
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
  runtime: {
    list: () => invoke<RuntimeDefinition[]>("runtime_list"),
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
    start: (
      input: StartMissionInput,
      initialSize?: { cols: number; rows: number } | null,
    ) =>
      invoke<StartMissionOutput>("mission_start", {
        input,
        initialCols: initialSize?.cols ?? null,
        initialRows: initialSize?.rows ?? null,
      }),
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
     *  `completed`. Reversible via `unarchive`. */
    archive: (id: string) => invoke<Mission>("mission_archive", { id }),
    /** Clear the archive marker only — status stays `completed`,
     *  `stopped_at` survives. The backend emits `mission/changed` so
     *  the sidebar reinstates the row live. Idempotent. */
    unarchive: (id: string) => invoke<Mission>("mission_unarchive", { id }),
    /** Archived missions, newest-archived first (Settings → Archived). */
    listArchived: (crewId?: string) =>
      invoke<Mission[]>("mission_list_archived", crewId ? { crewId } : {}),
    /** Permanently delete an archived mission: session rows, the
     *  mission row, and its on-disk event log. Refused unless
     *  archived — archive is the reversible step, this is not. */
    delete: (id: string) => invoke<void>("mission_delete", { id }),
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
    setProject: (id: string, projectId: string | null) =>
      invoke<Mission>("mission_set_project", { id, projectId }),
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
    /** Clear the archive marker; the row rejoins the sidebar CHAT list
     *  (backend emits `session/updated`). Resume keeps working — the
     *  agent_session_key survives archive/unarchive. Idempotent. */
    unarchive: (sessionId: string) =>
      invoke<void>("session_unarchive", { sessionId }),
    /** Archived direct chats, newest-archived first (Settings →
     *  Archived). Same DTO as listRecentDirect, keys withheld. */
    listArchived: () =>
      invoke<DirectSessionEntry[]>("session_list_archived"),
    /** Permanently delete an archived direct chat. Refused unless
     *  archived — archive is the reversible step, this is not. */
    delete: (sessionId: string) =>
      invoke<void>("session_delete", { sessionId }),
    rename: (sessionId: string, title: string | null) =>
      invoke<void>("session_rename", { sessionId, title }),
    pin: (sessionId: string, pinned: boolean) =>
      invoke<void>("session_pin", { sessionId, pinned }),
    setProject: (sessionIds: string[], projectId: string | null) =>
      invoke<void>("session_set_project", { sessionIds, projectId }),
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
    /** Seq the output ring had reached when the most recent resume
     *  started; 0 for sessions that never resumed. The pill effects
     *  only honor TUI-ready escapes in snapshot chunks above this,
     *  so retained pre-resume scrollback (claude-code, impl 0024)
     *  can't clear a resuming overlay early. */
    replayWatermark: (sessionId: string) =>
      invoke<number>("session_replay_watermark", { sessionId }),
    activitySnapshot: () =>
      invoke<Record<string, SessionActivityState>>("session_activity_snapshot"),
    pasteImage: (bytes: Uint8Array, mimeType: PasteImageMimeType) =>
      invoke<void>("session_paste_image", {
        bytes: Array.from(bytes),
        mimeType,
      }),
    startDirect: (
      runnerId: string,
      cwd: string | null,
      cols: number | null,
      rows: number | null,
      projectId: string | null = null,
      runtime: string | null = null,
    ) =>
      invoke<SpawnedSession>("session_start_direct", {
        runnerId,
        runtime,
        projectId,
        cwd,
        cols,
        rows,
      }),
    startRuntime: (
      runtime: string,
      cwd: string | null,
      cols: number | null,
      rows: number | null,
      projectId: string | null = null,
    ) =>
      invoke<SpawnedSession>("session_start_runtime", {
        runtime,
        projectId,
        cwd,
        cols,
        rows,
      }),
  },
  window: {
    /** Open a new webview window, optionally pre-navigated to a route
     *  (carried as a URL hash fragment) and positioned. Returns the new
     *  window's label. */
    open: (
      initialRoute?: string | null,
      position?: [number, number] | null,
    ) =>
      invoke<string>("window_open", {
        initialRoute: initialRoute ?? null,
        position: position ?? null,
      }),
    /** Bring another window to the front (the overlay's "Focus that
     *  window" action). */
    focusOther: (label: string) =>
      invoke<void>("window_focus_other", { label }),
    /** Report every subject this window currently shows (one per visible
     *  pane); the backend recomputes the focus map and broadcasts it. */
    reportSubjects: (subjects: Subject[]) =>
      invoke<void>("window_report_subjects", { subjects }),
    /** Snapshot of the focus map for hydrate-on-mount. */
    listSubjects: () => invoke<WindowEntry[]>("window_list_subjects"),
  },
};
