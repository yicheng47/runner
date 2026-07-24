/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const eventMocks = vi.hoisted(() => {
  const handlers = new Map<string, Set<(event: { payload: unknown }) => void>>();
  const stops: Array<ReturnType<typeof vi.fn>> = [];
  return {
    handlers,
    stops,
    listen: vi.fn(
      async (
        name: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        const listeners = handlers.get(name) ?? new Set();
        listeners.add(handler);
        handlers.set(name, listeners);
        const stop = vi.fn(() => listeners.delete(handler));
        stops.push(stop);
        return stop;
      },
    ),
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

import { useMissionDeliveryBlocked } from "./deliveryBlocked";
import type { DeliveryBlockedEvent } from "./types";

function Harness({
  missionId,
  sessionIds,
}: {
  missionId: string;
  sessionIds: string[];
}) {
  const blocked = useMissionDeliveryBlocked(missionId, sessionIds);
  return (
    <div>
      {Object.values(blocked)
        .map((event) => `${event.session_id}:${event.unread_count}`)
        .join(",")}
    </div>
  );
}

async function emit(name: string, payload: unknown) {
  await act(async () => {
    for (const handler of eventMocks.handlers.get(name) ?? []) {
      handler({ payload });
    }
  });
}

describe("useMissionDeliveryBlocked", () => {
  let container: HTMLDivElement;
  let root: Root | null;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    eventMocks.handlers.clear();
    eventMocks.stops.length = 0;
    eventMocks.listen.mockClear();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    if (root) {
      await act(async () => root?.unmount());
    }
    container.remove();
    vi.unstubAllGlobals();
  });

  async function render(sessionIds: string[]) {
    await act(async () => {
      root?.render(
        <Harness missionId="mission-1" sessionIds={sessionIds} />,
      );
    });
  }

  const blockedEvent: DeliveryBlockedEvent = {
    mission_id: "mission-1",
    session_id: "S-1",
    handle: "impl",
    unread_count: 2,
    blocked: true,
  };

  it("clears on session exit and pane replacement", async () => {
    await render(["S-1"]);
    await emit("router/delivery-blocked", blockedEvent);
    expect(container.textContent).toBe("S-1:2");

    await emit("session/exit", {
      session_id: "S-1",
      mission_id: "mission-1",
    });
    expect(container.textContent).toBe("");

    await emit("router/delivery-blocked", blockedEvent);
    await render(["S-2"]);
    expect(container.textContent).toBe("");
  });

  it("unsubscribes on workspace unmount and remounts empty", async () => {
    await render(["S-1"]);
    await emit("router/delivery-blocked", blockedEvent);
    expect(container.textContent).toBe("S-1:2");

    await act(async () => root?.unmount());
    root = null;
    expect(eventMocks.stops).toHaveLength(2);
    expect(eventMocks.stops.every((stop) => stop.mock.calls.length === 1)).toBe(
      true,
    );

    root = createRoot(container);
    await render(["S-1"]);
    expect(container.textContent).toBe("");
  });
});
