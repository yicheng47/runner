// Session ids auto-resumed by THIS launch (#366 diagnostics). autoResume
// marks an id once its resume RPC succeeds; RunnerTerminal consumes the
// mark on the session's next real fit to emit exactly one
// `[launch-dims] first-fit` line per launch-resumed session. Fresh chats,
// UI-started missions, manual resumes, and navigation remounts never
// enter the set, so they never log.

const pendingFirstFit = new Set<string>();

export function markLaunchResumed(sessionId: string): void {
  pendingFirstFit.add(sessionId);
}

/** True exactly once per marked session — the caller owns the log line. */
export function takeLaunchResumed(sessionId: string): boolean {
  return pendingFirstFit.delete(sessionId);
}

export function resetLaunchResumeTraceForTest(): void {
  pendingFirstFit.clear();
}
