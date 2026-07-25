export const AUTO_RESUME_STAGGER_MS = 300;

interface AutoResumeApi {
  takeResumeOnLaunch: () => Promise<string | null>;
  clearResumeOnLaunch: () => Promise<void>;
  resumeOnLaunch: (sessionId: string) => Promise<unknown>;
}

export async function consumeResumeOnLaunch(
  enabled: boolean,
  sessionApi: AutoResumeApi,
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
    try {
      await sessionApi.resumeOnLaunch(sessionId);
    } catch (error) {
      onError(error);
    }
  }
}
