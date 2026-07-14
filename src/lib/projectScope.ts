import { useSyncExternalStore } from "react";

import type { DirectSessionEntry, ProjectRow } from "./api";

let activeProject: ProjectRow | null = null;
const listeners = new Set<() => void>();

export function setActiveProjectScope(project: ProjectRow | null): void {
  if (activeProject?.id === project?.id && activeProject?.cwd === project?.cwd) {
    return;
  }
  activeProject = project;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): ProjectRow | null {
  return activeProject;
}

export function useActiveProjectScope(): ProjectRow | null {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function projectIdForTab(
  members: Pick<DirectSessionEntry, "project_id">[],
): string | null {
  if (members.length === 0) return null;
  const projectId = members[0].project_id;
  return members.every((member) => member.project_id === projectId)
    ? projectId
    : null;
}
