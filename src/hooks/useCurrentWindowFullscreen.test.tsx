/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  fullscreen: false,
  resize: null as (() => void) | null,
  isFullscreen: vi.fn<() => Promise<boolean>>(),
  onResized: vi.fn<(callback: () => void) => Promise<() => void>>(),
  unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: mocks.isFullscreen,
    onResized: mocks.onResized,
  }),
}));

import {
  FULLSCREEN_SETTLE_MS,
  useCurrentWindowFullscreen,
} from "./useCurrentWindowFullscreen";

function Probe() {
  const fullscreen = useCurrentWindowFullscreen();
  return <div data-fullscreen={fullscreen} />;
}

describe("useCurrentWindowFullscreen", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    mocks.fullscreen = false;
    mocks.resize = null;
    mocks.unlisten.mockReset();
    mocks.isFullscreen.mockReset();
    mocks.isFullscreen.mockImplementation(async () => mocks.fullscreen);
    mocks.onResized.mockReset();
    mocks.onResized.mockImplementation(async (callback) => {
      mocks.resize = callback;
      return mocks.unlisten;
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it("updates only after the current window resize state settles", async () => {
    await act(async () => {
      root.render(<Probe />);
    });
    expect(container.firstElementChild?.getAttribute("data-fullscreen")).toBe(
      "false",
    );

    mocks.fullscreen = true;
    await act(async () => {
      mocks.resize?.();
      await vi.advanceTimersByTimeAsync(FULLSCREEN_SETTLE_MS - 1);
    });
    expect(container.firstElementChild?.getAttribute("data-fullscreen")).toBe(
      "false",
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(container.firstElementChild?.getAttribute("data-fullscreen")).toBe(
      "true",
    );
  });

  it("unsubscribes from the current window on unmount", async () => {
    await act(async () => {
      root.render(<Probe />);
    });
    await act(async () => root.unmount());

    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });
});
