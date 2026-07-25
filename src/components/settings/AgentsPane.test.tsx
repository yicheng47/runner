/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  setOverride: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));

vi.mock("../../lib/api", () => ({
  api: {
    runtime: {
      status: vi.fn(async () => ({
        shell: {
          shell: "/bin/zsh",
          outcome: "ok",
          duration_ms: 25,
          checking: false,
          using_last_known_good: false,
          last_known_good_captured_at: null,
        },
        runtimes: [
          {
            name: "codex",
            display_name: "Codex",
            command: "codex",
            detected_path: "/usr/local/bin/codex",
            override_path: "/opt/codex",
            effective_command: "/opt/codex",
            effective_source: "override",
            state: "override",
            invalid_reason: null,
          },
        ],
      })),
      setOverride: mocks.setOverride,
      clearOverride: vi.fn(),
      refresh: vi.fn(),
    },
  },
}));

import { AgentsPane } from "./AgentsPane";

describe("AgentsPane", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.setOverride.mockClear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("discards an edited override on Escape without saving it on blur", async () => {
    await act(async () => {
      root.render(<AgentsPane />);
    });
    const input = container.querySelector<HTMLInputElement>(
      '[aria-label="Codex executable override"]',
    );
    expect(input).not.toBeNull();

    await act(async () => {
      input?.focus();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")
        ?.set?.call(input, "/tmp/new-codex");
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      input?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });

    expect(mocks.setOverride).not.toHaveBeenCalled();
    expect(input?.value).toBe("/opt/codex");
  });
});
