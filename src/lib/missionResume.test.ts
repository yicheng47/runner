import { describe, expect, it, vi } from "vitest";

import type { SessionRow } from "./api";
import { resumeStoppedMissionSessions } from "./missionResume";

function session(id: string, status: SessionRow["status"]): SessionRow {
  return {
    id,
    mission_id: "mission-1",
    runner_id: `runner-${id}`,
    cwd: null,
    status,
    pid: status === "running" ? 123 : null,
    started_at: null,
    stopped_at: null,
    handle: id,
    runtime: "codex",
    lead: id === "session-a",
    agent_session_key: null,
  };
}

describe("resumeStoppedMissionSessions", () => {
  it("re-fetches sessions before choosing which slots to resume", async () => {
    const list = vi
      .fn<(missionId: string) => Promise<SessionRow[]>>()
      .mockResolvedValueOnce([
        session("session-a", "running"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "running"),
        session("session-b", "running"),
      ]);
    const resume = vi.fn().mockResolvedValue(undefined);

    const result = await resumeStoppedMissionSessions(
      "mission-1",
      { cols: 180, rows: 48 },
      { list, resume },
    );

    expect(list).toHaveBeenCalledTimes(2);
    expect(list.mock.invocationCallOrder[0]).toBeLessThan(
      resume.mock.invocationCallOrder[0],
    );
    expect(resume).toHaveBeenCalledOnce();
    expect(resume).toHaveBeenCalledWith("session-b", 180, 48);
    expect(result).toEqual({
      sessions: [
        session("session-a", "running"),
        session("session-b", "running"),
      ],
      error: null,
    });
  });

  it("refreshes and continues when a racing resume reports already running", async () => {
    const list = vi
      .fn<(missionId: string) => Promise<SessionRow[]>>()
      .mockResolvedValueOnce([
        session("session-a", "stopped"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "running"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "running"),
        session("session-b", "running"),
      ]);
    const resume = vi
      .fn()
      .mockRejectedValueOnce(
        "session_resume: session session-a is already running — attach instead",
      )
      .mockResolvedValueOnce(undefined);

    const result = await resumeStoppedMissionSessions("mission-1", null, {
      list,
      resume,
    });

    expect(list).toHaveBeenCalledTimes(3);
    expect(resume.mock.calls).toEqual([
      ["session-a", null, null],
      ["session-b", null, null],
    ]);
    expect(result.error).toBeNull();
    expect(result.sessions.every((row) => row.status === "running")).toBe(true);
  });

  it("refreshes and continues while a racing resume is still in flight", async () => {
    const list = vi
      .fn<(missionId: string) => Promise<SessionRow[]>>()
      .mockResolvedValueOnce([
        session("session-a", "stopped"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "stopped"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "running"),
        session("session-b", "running"),
      ]);
    const resume = vi
      .fn()
      .mockRejectedValueOnce(
        "session_resume: session session-a is already being resumed",
      )
      .mockResolvedValueOnce(undefined);

    const result = await resumeStoppedMissionSessions("mission-1", null, {
      list,
      resume,
    });

    expect(list).toHaveBeenCalledTimes(3);
    expect(resume.mock.calls).toEqual([
      ["session-a", null, null],
      ["session-b", null, null],
    ]);
    expect(result.error).toBeNull();
    expect(result.sessions.every((row) => row.status === "running")).toBe(true);
  });

  it("keeps resuming other slots after a real failure", async () => {
    const failed = new Error("spawn failed");
    const list = vi
      .fn<(missionId: string) => Promise<SessionRow[]>>()
      .mockResolvedValueOnce([
        session("session-a", "stopped"),
        session("session-b", "stopped"),
      ])
      .mockResolvedValueOnce([
        session("session-a", "crashed"),
        session("session-b", "running"),
      ]);
    const resume = vi
      .fn()
      .mockRejectedValueOnce(failed)
      .mockResolvedValueOnce(undefined);

    const result = await resumeStoppedMissionSessions("mission-1", null, {
      list,
      resume,
    });

    expect(resume).toHaveBeenCalledTimes(2);
    expect(result.error).toBe(failed);
  });
});
