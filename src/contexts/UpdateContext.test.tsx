/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  check: vi.fn<() => Promise<unknown>>(),
  downloadAndInstall: vi.fn<() => Promise<void>>(),
  onFocusChanged: vi.fn(async () => () => {}),
  windowLabel: "main",
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: mocks.check,
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: mocks.windowLabel,
    onFocusChanged: mocks.onFocusChanged,
  }),
}));

import {
  UpdateProvider,
  useUpdate,
} from "./UpdateContext";
import type { UpdateState } from "../hooks/useUpdateChecker";
import { STORAGE_AUTO_INSTALL_UPDATES } from "../lib/settings";

const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

describe("UpdateProvider trigger policy", () => {
  let container: HTMLDivElement;
  let root: Root;
  let latest: UpdateState | null;
  let storage: Map<string, string>;

  function Probe() {
    latest = useUpdate();
    return null;
  }

  function state(): UpdateState {
    if (!latest) throw new Error("update state is unavailable");
    return latest;
  }

  async function renderProvider() {
    await act(async () => {
      root.render(
        <UpdateProvider>
          <Probe />
        </UpdateProvider>,
      );
    });
  }

  async function runLaunchCheck() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
  }

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-22T00:00:00Z"));
    storage = new Map();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
      clear: () => storage.clear(),
    });
    localStorage.setItem("runner.dev.updateStatus", "idle");
    mocks.check.mockReset();
    mocks.check.mockResolvedValue(null);
    mocks.downloadAndInstall.mockReset();
    mocks.downloadAndInstall.mockResolvedValue();
    mocks.onFocusChanged.mockClear();
    mocks.windowLabel = "main";
    latest = null;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("checks again when a focused window is stale", async () => {
    await renderProvider();
    await runLaunchCheck();
    expect(mocks.check).toHaveBeenCalledTimes(1);

    vi.setSystemTime(Date.now() + UPDATE_CHECK_INTERVAL_MS + 1);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });

    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it("does not check again when a focused window is fresh", async () => {
    await renderProvider();
    await runLaunchCheck();

    vi.setSystemTime(Date.now() + UPDATE_CHECK_INTERVAL_MS - 1);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });

    expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  it("checks again when the interval elapses", async () => {
    await renderProvider();
    await runLaunchCheck();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(UPDATE_CHECK_INTERVAL_MS - 3000);
    });

    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it("does not schedule background checks in secondary windows", async () => {
    mocks.windowLabel = "window-secondary";
    await renderProvider();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(UPDATE_CHECK_INTERVAL_MS * 2);
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
    });

    expect(mocks.check).not.toHaveBeenCalled();
  });

  it("checks without downloading when automatic updates are off", async () => {
    localStorage.setItem(STORAGE_AUTO_INSTALL_UPDATES, "0");
    mocks.check.mockResolvedValue({
      version: "0.4.0",
      downloadAndInstall: mocks.downloadAndInstall,
    });

    await renderProvider();
    await runLaunchCheck();

    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(mocks.downloadAndInstall).not.toHaveBeenCalled();
    expect(state().status).toBe("available");
  });

  it("keeps automatic download enabled for opted-in users", async () => {
    localStorage.setItem(STORAGE_AUTO_INSTALL_UPDATES, "1");
    mocks.check.mockResolvedValue({
      version: "0.4.0",
      downloadAndInstall: mocks.downloadAndInstall,
    });

    await renderProvider();
    await runLaunchCheck();

    expect(mocks.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(state().status).toBe("ready");
  });

  it("keeps background errors silent but surfaces explicit failures", async () => {
    mocks.check.mockRejectedValue(new Error("offline"));
    await renderProvider();
    await runLaunchCheck();

    expect(state().status).toBe("idle");
    expect(state().error).toBeNull();

    await act(async () => {
      await state().checkForUpdate();
    });

    expect(state().status).toBe("error");
    expect(state().error).toBe("offline");
  });

  it("surfaces failure when an explicit check joins a silent check", async () => {
    let rejectCheck: ((reason: Error) => void) | null = null;
    mocks.check.mockReturnValue(
      new Promise((_, reject) => {
        rejectCheck = reject;
      }),
    );
    await renderProvider();

    await act(async () => {
      vi.advanceTimersByTime(3000);
      await Promise.resolve();
    });
    expect(state().status).toBe("checking");

    await act(async () => {
      await state().checkForUpdate();
      if (!rejectCheck) throw new Error("missing pending update check");
      rejectCheck(new Error("offline"));
      await Promise.resolve();
    });

    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(state().status).toBe("error");
    expect(state().error).toBe("offline");
  });
});
