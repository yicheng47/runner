/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

import { McpPane } from "./McpPane";

const clientStatus = {
  registered: false,
  matches_current: false,
  command: null,
  args: [],
  config_path: "",
  error: null,
};

const status = {
  environment: "Development",
  binary_path: "/test/runner-mcp",
  socket_path: "/test/mcp.sock",
  claude_code: clientStatus,
  codex: clientStatus,
  qoder: clientStatus,
  trae: clientStatus,
};

describe("McpPane", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "mcp_integration_status") return status;
      if (command === "mcp_config_snippet") {
        return { claude_code: "{}", codex: "", qoder: "{}", trae: "" };
      }
      return undefined;
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("renders and toggles TRAE registration", async () => {
    await act(async () => {
      root.render(<McpPane />);
    });

    expect(container.textContent).toContain("TRAE CLI");
    const toggles = container.querySelectorAll<HTMLButtonElement>(
      '[role="switch"]',
    );
    expect(toggles).toHaveLength(4);

    await act(async () => toggles[3].click());

    expect(mocks.invoke).toHaveBeenCalledWith("mcp_set_integration", {
      client: "trae",
      enabled: true,
    });
  });
});
