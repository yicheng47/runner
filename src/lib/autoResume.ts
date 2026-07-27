import type { TerminalGridSize } from "./terminalSizing";

export const AUTO_RESUME_STAGGER_MS = 300;

interface AutoResumeApi {
  takeResumeOnLaunch: () => Promise<string | null>;
  clearResumeOnLaunch: () => Promise<void>;
  resumeOnLaunch: (
    sessionId: string,
    cols: number | null,
    rows: number | null,
  ) => Promise<unknown>;
}

/**
 * Launch-resume dims precedence (impl 0036, decision 2). A real measurement
 * from a laid-out pane beats an estimate derived from this launch's window
 * geometry; null hands the remaining two rungs to the backend, which resolves
 * `last_cols`/`last_rows` and then `DEFAULT_PTY_SIZE` (`spawn.rs`).
 *
 * Both sources are guarded, so a session whose dims cannot be resolved still
 * resumes — it just resumes without them, exactly as every launch resume did
 * before this impl.
 */
export function resolveLaunchDims(sources: {
  measure: () => TerminalGridSize | null;
  estimate: () => TerminalGridSize | null;
}): TerminalGridSize | null {
  return readDims(sources.measure) ?? readDims(sources.estimate);
}

function readDims(
  source: () => TerminalGridSize | null,
): TerminalGridSize | null {
  try {
    const dims = source();
    return dims && dims.cols > 0 && dims.rows > 0 ? dims : null;
  } catch {
    return null;
  }
}

export async function consumeResumeOnLaunch(
  enabled: boolean,
  sessionApi: AutoResumeApi,
  dimsFor: (sessionId: string) => TerminalGridSize | null,
  wait: (ms: number) => Promise<void> = (ms) =>
    new Promise((resolve) => window.setTimeout(resolve, ms)),
  onError: (error: unknown) => void = console.error,
): Promise<void> {
  if (!enabled) {
    await sessionApi.clearResumeOnLaunch();
    return;
  }

  let attemptedResume = false;
  for (;;) {
    const sessionId = await sessionApi.takeResumeOnLaunch();
    if (sessionId === null) return;
    if (attemptedResume) await wait(AUTO_RESUME_STAGGER_MS);
    attemptedResume = true;
    // Resolved after the stagger so it reflects the freshest layout, and
    // guarded so one unresolvable session cannot abort the drain (#320's
    // failure-tolerance rule).
    let dims: TerminalGridSize | null = null;
    try {
      dims = dimsFor(sessionId);
    } catch (error) {
      onError(error);
    }
    try {
      await sessionApi.resumeOnLaunch(
        sessionId,
        dims?.cols ?? null,
        dims?.rows ?? null,
      );
    } catch (error) {
      onError(error);
    }
  }
}
