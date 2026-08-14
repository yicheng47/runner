import type { SessionRow } from "./api";
import type { TerminalGridSize } from "./terminalSizing";

interface MissionResumeApi {
  list: (missionId: string) => Promise<SessionRow[]>;
  resume: (
    sessionId: string,
    cols: number | null,
    rows: number | null,
  ) => Promise<unknown>;
}

export interface MissionResumeResult {
  sessions: SessionRow[];
  error: unknown | null;
}

// Keep these fragments synchronized with SessionManager::resume_with_fresh_fallback.
const CONCURRENT_RESUME_ERRORS = [
  "is already being resumed",
  "is already running — attach instead",
];

function isConcurrentResumeError(error: unknown): boolean {
  const message = String(error);
  return CONCURRENT_RESUME_ERRORS.some((fragment) => message.includes(fragment));
}

export async function resumeStoppedMissionSessions(
  missionId: string,
  dims: TerminalGridSize | null,
  sessionApi: MissionResumeApi,
): Promise<MissionResumeResult> {
  let sessions = await sessionApi.list(missionId);
  let firstError: unknown | null = null;

  for (const candidate of sessions) {
    const current = sessions.find((session) => session.id === candidate.id);
    if (current?.status === "running") continue;

    try {
      await sessionApi.resume(
        candidate.id,
        dims?.cols ?? null,
        dims?.rows ?? null,
      );
    } catch (error) {
      if (!isConcurrentResumeError(error)) {
        firstError ??= error;
        continue;
      }

      try {
        sessions = await sessionApi.list(missionId);
      } catch (refreshError) {
        firstError ??= refreshError;
      }
    }
  }

  try {
    sessions = await sessionApi.list(missionId);
  } catch (refreshError) {
    firstError ??= refreshError;
  }

  return { sessions, error: firstError };
}
