import { afterEach, describe, expect, it, vi } from "vitest";

import {
  AUTO_RESUME_STAGGER_MS,
  consumeResumeOnLaunch,
  resolveLaunchDims,
} from "./autoResume";
import {
  DEFAULT_RESUME_ON_LAUNCH,
  readStoredBool,
  STORAGE_RESUME_ON_LAUNCH,
} from "./settings";
import type { TerminalGridSize } from "./terminalSizing";

afterEach(() => {
  vi.unstubAllGlobals();
});

const noDims = () => null;

describe("consumeResumeOnLaunch", () => {
  it("resumes sequentially, staggers attempts, and continues after failure", async () => {
    const takeResumeOnLaunch = vi
      .fn<() => Promise<string | null>>()
      .mockResolvedValueOnce("session-a")
      .mockResolvedValueOnce("session-b")
      .mockResolvedValueOnce(null);
    const resumeOnLaunch = vi
      .fn<
        (
          sessionId: string,
          cols: number | null,
          rows: number | null,
        ) => Promise<void>
      >()
      .mockRejectedValueOnce(new Error("rejected key"))
      .mockResolvedValueOnce();
    const clearResumeOnLaunch = vi.fn<() => Promise<void>>();
    const wait = vi.fn<(ms: number) => Promise<void>>().mockResolvedValue();
    const onError = vi.fn();

    await consumeResumeOnLaunch(
      true,
      { takeResumeOnLaunch, clearResumeOnLaunch, resumeOnLaunch },
      noDims,
      wait,
      onError,
    );

    expect(resumeOnLaunch.mock.calls).toEqual([
      ["session-a", null, null],
      ["session-b", null, null],
    ]);
    expect(wait).toHaveBeenCalledOnce();
    expect(wait).toHaveBeenCalledWith(AUTO_RESUME_STAGGER_MS);
    expect(onError).toHaveBeenCalledOnce();
    expect(clearResumeOnLaunch).not.toHaveBeenCalled();
  });

  // The #363 regression at the queue seam: a launch resume forks at the dims
  // the caller resolved from THIS launch's window, not at whatever the row
  // persisted at the previous quit (which is what `null, null` selects).
  it("sends the resolved dims for each session", async () => {
    const takeResumeOnLaunch = vi
      .fn<() => Promise<string | null>>()
      .mockResolvedValueOnce("session-a")
      .mockResolvedValueOnce("session-b")
      .mockResolvedValueOnce(null);
    const resumeOnLaunch = vi
      .fn<
        (
          sessionId: string,
          cols: number | null,
          rows: number | null,
        ) => Promise<void>
      >()
      .mockResolvedValue();
    const dimsFor = vi.fn((sessionId: string) =>
      sessionId === "session-a" ? { cols: 213, rows: 55 } : null,
    );

    await consumeResumeOnLaunch(
      true,
      {
        takeResumeOnLaunch,
        clearResumeOnLaunch: vi.fn<() => Promise<void>>(),
        resumeOnLaunch,
      },
      dimsFor,
      vi.fn<(ms: number) => Promise<void>>().mockResolvedValue(),
    );

    expect(resumeOnLaunch.mock.calls).toEqual([
      ["session-a", 213, 55],
      // No dims resolved — the backend falls back to persisted, then 80×24.
      ["session-b", null, null],
    ]);
  });

  it("keeps draining when one session's dims resolution throws", async () => {
    const takeResumeOnLaunch = vi
      .fn<() => Promise<string | null>>()
      .mockResolvedValueOnce("session-a")
      .mockResolvedValueOnce("session-b")
      .mockResolvedValueOnce(null);
    const resumeOnLaunch = vi
      .fn<
        (
          sessionId: string,
          cols: number | null,
          rows: number | null,
        ) => Promise<void>
      >()
      .mockResolvedValue();
    const dimsFor = vi.fn((sessionId: string) => {
      if (sessionId === "session-a") throw new Error("no layout");
      return { cols: 120, rows: 40 };
    });
    const onError = vi.fn();

    await consumeResumeOnLaunch(
      true,
      {
        takeResumeOnLaunch,
        clearResumeOnLaunch: vi.fn<() => Promise<void>>(),
        resumeOnLaunch,
      },
      dimsFor,
      vi.fn<(ms: number) => Promise<void>>().mockResolvedValue(),
      onError,
    );

    expect(resumeOnLaunch.mock.calls).toEqual([
      ["session-a", null, null],
      ["session-b", 120, 40],
    ]);
    expect(onError).toHaveBeenCalledOnce();
  });

  it("clears pending stamps without resuming when disabled", async () => {
    const takeResumeOnLaunch = vi.fn<() => Promise<string | null>>();
    const resumeOnLaunch =
      vi.fn<
        (
          sessionId: string,
          cols: number | null,
          rows: number | null,
        ) => Promise<void>
      >();
    const clearResumeOnLaunch = vi
      .fn<() => Promise<void>>()
      .mockResolvedValue();

    await consumeResumeOnLaunch(
      false,
      { takeResumeOnLaunch, clearResumeOnLaunch, resumeOnLaunch },
      noDims,
    );

    expect(clearResumeOnLaunch).toHaveBeenCalledOnce();
    expect(takeResumeOnLaunch).not.toHaveBeenCalled();
    expect(resumeOnLaunch).not.toHaveBeenCalled();
  });

  it("clears pending stamps without resuming when the setting is absent", async () => {
    vi.stubGlobal("localStorage", {
      getItem: vi.fn<(key: string) => string | null>().mockReturnValue(null),
    });
    const takeResumeOnLaunch = vi.fn<() => Promise<string | null>>();
    const resumeOnLaunch =
      vi.fn<
        (
          sessionId: string,
          cols: number | null,
          rows: number | null,
        ) => Promise<void>
      >();
    const clearResumeOnLaunch = vi
      .fn<() => Promise<void>>()
      .mockResolvedValue();

    await consumeResumeOnLaunch(
      readStoredBool(
        STORAGE_RESUME_ON_LAUNCH,
        DEFAULT_RESUME_ON_LAUNCH,
      ),
      { takeResumeOnLaunch, clearResumeOnLaunch, resumeOnLaunch },
      noDims,
    );

    expect(localStorage.getItem).toHaveBeenCalledWith(
      STORAGE_RESUME_ON_LAUNCH,
    );
    expect(clearResumeOnLaunch).toHaveBeenCalledOnce();
    expect(takeResumeOnLaunch).not.toHaveBeenCalled();
    expect(resumeOnLaunch).not.toHaveBeenCalled();
  });
});

describe("resolveLaunchDims", () => {
  const dims = (cols: number): TerminalGridSize => ({ cols, rows: 40 });

  it("prefers a real measurement over an estimate", () => {
    const estimate = vi.fn(() => dims(150));
    expect(
      resolveLaunchDims({ measure: () => dims(213), estimate }),
    ).toEqual(dims(213));
    expect(estimate).not.toHaveBeenCalled();
  });

  it("estimates when no pane is laid out to measure", () => {
    expect(
      resolveLaunchDims({ measure: () => null, estimate: () => dims(150) }),
    ).toEqual(dims(150));
  });

  it("hands the persisted and default rungs to the backend when neither resolves", () => {
    expect(
      resolveLaunchDims({ measure: () => null, estimate: () => null }),
    ).toBeNull();
  });

  it("falls through a throwing source instead of failing the resume", () => {
    expect(
      resolveLaunchDims({
        measure: () => {
          throw new Error("no rect");
        },
        estimate: () => dims(150),
      }),
    ).toEqual(dims(150));
    expect(
      resolveLaunchDims({
        measure: () => null,
        estimate: () => {
          throw new Error("xterm probe failed");
        },
      }),
    ).toBeNull();
  });

  it("rejects a degenerate grid rather than forking at zero", () => {
    expect(
      resolveLaunchDims({
        measure: () => ({ cols: 0, rows: 40 }),
        estimate: () => dims(150),
      }),
    ).toEqual(dims(150));
  });
});
