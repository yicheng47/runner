const STORAGE_PREFIX = "runner.mission.lastTerminal.";

function storageKey(missionId: string): string {
  return `${STORAGE_PREFIX}${missionId}`;
}

export function getLastMissionTerminalId(missionId: string): string | null {
  try {
    const value = localStorage.getItem(storageKey(missionId));
    return value && value.trim() ? value : null;
  } catch {
    return null;
  }
}

export function setLastMissionTerminalId(
  missionId: string,
  sessionId: string,
): void {
  try {
    localStorage.setItem(storageKey(missionId), sessionId);
  } catch {
    // ignore quota / disabled-storage errors
  }
}

export function clearLastMissionTerminalId(missionId: string): void {
  try {
    localStorage.removeItem(storageKey(missionId));
  } catch {
    // ignore quota / disabled-storage errors
  }
}
