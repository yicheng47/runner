/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
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
      status: mocks.status,
      setOverride: mocks.setOverride,
      clearOverride: vi.fn(),
      refresh: vi.fn(),
    },
  },
}));

import { RUNTIME_OPTIONS } from "../ui/runtimes";
import { AgentsPane } from "./AgentsPane";

const runtimeStatus = {
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
    {
      name: "qoder",
      display_name: "Qoder",
      command: "qodercli",
      detected_path: "/usr/local/bin/qodercli",
      override_path: null,
      effective_command: "/usr/local/bin/qodercli",
      effective_source: "detected",
      state: "detected",
      invalid_reason: null,
    },
  ],
};

describe("AgentsPane", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.status.mockReset();
    mocks.status.mockResolvedValue(runtimeStatus);
    mocks.setOverride.mockClear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("renders one loading row per catalog runtime", async () => {
    mocks.status.mockReturnValue(new Promise(() => {}));
    await act(async () => {
      root.render(<AgentsPane />);
    });
    expect(container.querySelectorAll(".animate-pulse")).toHaveLength(
      RUNTIME_OPTIONS.length,
    );
  });

  it("owns and persists the direct chat default agent", async () => {
    localStorage.setItem("settings.defaultChatRuntime", "qoder");
    localStorage.setItem("settings.enabledAgents", '["qoder"]');
    await act(async () => {
      root.render(<AgentsPane />);
    });

    const trigger = container.querySelector<HTMLButtonElement>(
      "#agents-default-runtime",
    );
    expect(trigger?.textContent).toContain("Qoder");

    await act(async () => trigger?.click());
    const codexOption = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent?.trim() === "Codex");
    expect(codexOption).toBeDefined();

    await act(async () => codexOption?.click());
    expect(localStorage.getItem("settings.defaultChatRuntime")).toBe("codex");
    expect(trigger?.textContent).toContain("Codex");
  });

  it("enables agents by default and persists a disabled agent", async () => {
    localStorage.setItem("settings.defaultChatRuntime", "codex");
    localStorage.setItem("settings.enabledAgents", '["qoder"]');
    await act(async () => {
      root.render(<AgentsPane />);
    });

    const toggle = container.querySelector<HTMLButtonElement>(
      '[aria-label="Disable Codex"]',
    );
    expect(toggle?.getAttribute("aria-checked")).toBe("true");

    await act(async () => toggle?.click());
    expect(localStorage.getItem("settings.disabledAgents")).toBe('["codex"]');
    expect(localStorage.getItem("settings.defaultChatRuntime")).toBeNull();
    expect(
      container
        .querySelector<HTMLButtonElement>('[aria-label="Enable Codex"]')
        ?.getAttribute("aria-checked"),
    ).toBe("false");

    const defaultAgent = container.querySelector<HTMLButtonElement>(
      "#agents-default-runtime",
    );
    await act(async () => defaultAgent?.click());
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[role="option"]'),
    ).map((option) => option.textContent);
    expect(options.join(" ")).not.toContain("Codex");
    expect(options.join(" ")).toContain("Qoder");
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
