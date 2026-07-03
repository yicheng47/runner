// Registry of live RunnerTerminal handles, keyed by session id. A closure
// over plain Maps instead of a ref so `refFor` can be called during render
// (react-hooks/refs forbids `ref.current` there); the returned callbacks
// are stable per session so React doesn't detach/reattach them per commit.

import type { RunnerTerminalHandle } from "../components/RunnerTerminal";

export function createTerminalRegistry() {
  const handles = new Map<string, RunnerTerminalHandle>();
  const cbs = new Map<string, (h: RunnerTerminalHandle | null) => void>();
  return {
    refFor(sessionId: string) {
      let cb = cbs.get(sessionId);
      if (!cb) {
        cb = (h) => {
          if (h) handles.set(sessionId, h);
          else handles.delete(sessionId);
        };
        cbs.set(sessionId, cb);
      }
      return cb;
    },
    get(sessionId: string): RunnerTerminalHandle | null {
      return handles.get(sessionId) ?? null;
    },
  };
}

export type TerminalRegistry = ReturnType<typeof createTerminalRegistry>;
