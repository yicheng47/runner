import { describe, expect, it, vi } from "vitest";

import {
  AUTO_RESUME_STAGGER_MS,
  consumeResumeOnLaunch,
} from "./autoResume";

describe("consumeResumeOnLaunch", () => {
  it("resumes sequentially, staggers attempts, and continues after failure", async () => {
    const takeResumeOnLaunch = vi
      .fn<() => Promise<string | null>>()
      .mockResolvedValueOnce("session-a")
      .mockResolvedValueOnce("session-b")
      .mockResolvedValueOnce(null);
    const resumeOnLaunch = vi
      .fn<(sessionId: string) => Promise<void>>()
      .mockRejectedValueOnce(new Error("rejected key"))
      .mockResolvedValueOnce();
    const clearResumeOnLaunch = vi.fn<() => Promise<void>>();
    const wait = vi.fn<(ms: number) => Promise<void>>().mockResolvedValue();
    const onError = vi.fn();

    await consumeResumeOnLaunch(
      true,
      { takeResumeOnLaunch, clearResumeOnLaunch, resumeOnLaunch },
      wait,
      onError,
    );

    expect(resumeOnLaunch.mock.calls).toEqual([
      ["session-a"],
      ["session-b"],
    ]);
    expect(wait).toHaveBeenCalledOnce();
    expect(wait).toHaveBeenCalledWith(AUTO_RESUME_STAGGER_MS);
    expect(onError).toHaveBeenCalledOnce();
    expect(clearResumeOnLaunch).not.toHaveBeenCalled();
  });

  it("clears pending stamps without resuming when disabled", async () => {
    const takeResumeOnLaunch = vi.fn<() => Promise<string | null>>();
    const resumeOnLaunch = vi.fn<(sessionId: string) => Promise<void>>();
    const clearResumeOnLaunch = vi
      .fn<() => Promise<void>>()
      .mockResolvedValue();

    await consumeResumeOnLaunch(false, {
      takeResumeOnLaunch,
      clearResumeOnLaunch,
      resumeOnLaunch,
    });

    expect(clearResumeOnLaunch).toHaveBeenCalledOnce();
    expect(takeResumeOnLaunch).not.toHaveBeenCalled();
    expect(resumeOnLaunch).not.toHaveBeenCalled();
  });
});
