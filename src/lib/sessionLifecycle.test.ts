import { describe, expect, it } from "vitest";

import { snapshotIndicatesTuiReady } from "./sessionLifecycle";
import type { SessionOutputEvent } from "./types";

const READY = btoa("\x1b[?2004h");
const PLAIN = btoa("plain output, no ready escape");

function ev(seq: number, data: string): SessionOutputEvent {
  return { session_id: "s1", mission_id: null, seq, data };
}

describe("snapshotIndicatesTuiReady", () => {
  it("ignores a ready escape at or below the watermark", () => {
    // Retained pre-resume chunks (impl 0024) carry the old PTY's
    // bracketed-paste escape; they must not clear the resuming pill.
    expect(snapshotIndicatesTuiReady([ev(5, READY)], 5)).toBe(false);
    expect(snapshotIndicatesTuiReady([ev(3, READY), ev(6, PLAIN)], 5)).toBe(
      false,
    );
  });

  it("honors a ready escape above the watermark", () => {
    expect(snapshotIndicatesTuiReady([ev(3, PLAIN), ev(6, READY)], 5)).toBe(
      true,
    );
  });

  it("degenerates to any-chunk-counts at watermark 0", () => {
    expect(snapshotIndicatesTuiReady([ev(1, READY)], 0)).toBe(true);
    expect(snapshotIndicatesTuiReady([ev(1, PLAIN)], 0)).toBe(false);
    expect(snapshotIndicatesTuiReady([], 0)).toBe(false);
  });
});
