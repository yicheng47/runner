/** @vitest-environment jsdom */

import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  setZoom: vi.fn<(zoom: number) => Promise<void>>(),
  invoke: vi.fn<(command: string, args?: object) => Promise<void>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    setZoom: mocks.setZoom,
  }),
}));

import { applyAppZoom, syncTitlebarZoom } from "./appZoom";
import { ZOOM_STEPS } from "./settings";

describe("app zoom native titlebar", () => {
  beforeEach(() => {
    document.documentElement.style.removeProperty(
      "--titlebar-sidebar-toggle-gutter",
    );
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue();
    mocks.setZoom.mockReset();
    mocks.setZoom.mockResolvedValue();
  });

  it("sends the current zoom to the invoking window", async () => {
    await syncTitlebarZoom(1.2);

    expect(mocks.invoke).toHaveBeenCalledWith("window_set_titlebar_zoom", {
      zoom: 1.2,
    });
    expect(
      document.documentElement.style.getPropertyValue(
        "--titlebar-sidebar-toggle-gutter",
      ),
    ).toBe("72.2833px");
  });

  it("updates the native titlebar with the webview zoom", () => {
    applyAppZoom(0.8);

    expect(mocks.invoke).toHaveBeenCalledWith("window_set_titlebar_zoom", {
      zoom: 0.8,
    });
    expect(mocks.setZoom).toHaveBeenCalledWith(0.8);
    expect(
      document.documentElement.style.getPropertyValue(
        "--titlebar-sidebar-toggle-gutter",
      ),
    ).toBe("111.575px");
  });

  it("uses ten-percent zoom steps", () => {
    expect(ZOOM_STEPS).toEqual([0.8, 0.9, 1, 1.1, 1.2, 1.3, 1.4, 1.5]);
  });
});
