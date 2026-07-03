import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Subject } from "./types";

const mocks = vi.hoisted(() => ({
  reportSubjects: vi.fn<() => Promise<void>>(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("./api", () => ({
  api: {
    window: {
      reportSubjects: mocks.reportSubjects,
      listSubjects: vi.fn(),
    },
  },
}));

import { reportSubjects, reportSubjectsNow } from "./windowFocus";

const direct = (value: string): Subject => ({ type: "DirectChat", value });

describe("reportSubjectsNow", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.reportSubjects.mockClear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("clears a pending debounced report and writes immediately", async () => {
    reportSubjects([direct("A")]);
    reportSubjectsNow([direct("A"), direct("B")]);

    expect(mocks.reportSubjects).toHaveBeenCalledTimes(1);
    expect(mocks.reportSubjects).toHaveBeenCalledWith([
      direct("A"),
      direct("B"),
    ]);

    await vi.advanceTimersByTimeAsync(100);
    expect(mocks.reportSubjects).toHaveBeenCalledTimes(1);
  });
});
