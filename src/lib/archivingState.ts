// In-process registry of in-flight archive operations, keyed by
// mission id and session id. Lets the workspace + chat surfaces show
// a transitional "Archiving…" pill while the backend's session_kill +
// session_archive RPCs round-trip, regardless of which surface
// triggered the archive (sidebar kebab, workspace topbar kebab,
// chat header kebab).
//
// Module-scoped sets: archive operations are short-lived and don't
// need to survive a reload — if the user reloads mid-archive the
// backend has already committed the row flip, so the pill would be
// stale anyway.
//
// Subscribers re-render on any change. The hooks below do an exact
// id-membership check so unrelated components don't churn when an
// unrelated archive starts/ends.

import { useSyncExternalStore } from "react";

type Listener = () => void;

const archivingMissions = new Set<string>();
const archivingSessions = new Set<string>();
const listeners = new Set<Listener>();

function emit() {
  for (const l of listeners) l();
}

export function markArchivingMission(missionId: string): void {
  if (!archivingMissions.has(missionId)) {
    archivingMissions.add(missionId);
    emit();
  }
}

export function unmarkArchivingMission(missionId: string): void {
  if (archivingMissions.delete(missionId)) emit();
}

export function markArchivingSession(sessionId: string): void {
  if (!archivingSessions.has(sessionId)) {
    archivingSessions.add(sessionId);
    emit();
  }
}

export function unmarkArchivingSession(sessionId: string): void {
  if (archivingSessions.delete(sessionId)) emit();
}

export function isArchivingMission(missionId: string): boolean {
  return archivingMissions.has(missionId);
}

export function isArchivingSession(sessionId: string): boolean {
  return archivingSessions.has(sessionId);
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useArchivingMission(missionId: string | null | undefined): boolean {
  return useSyncExternalStore(
    subscribe,
    () => (missionId ? archivingMissions.has(missionId) : false),
    () => false,
  );
}

export function useArchivingSession(sessionId: string | null | undefined): boolean {
  return useSyncExternalStore(
    subscribe,
    () => (sessionId ? archivingSessions.has(sessionId) : false),
    () => false,
  );
}
