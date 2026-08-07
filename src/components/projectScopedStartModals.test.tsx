/** @vitest-environment jsdom */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectRow } from "../lib/api";

const mocks = vi.hoisted(() => ({
  crewList: vi.fn(async () => [
    { id: "crew-1", name: "Crew", runner_count: 1 },
  ]),
  runnerList: vi.fn(async () => [
    {
      id: "runner-1",
      handle: "coder",
      display_name: "Coder",
      runtime: "codex",
      command: "codex",
      args: [],
      env: {},
      working_dir: null,
    },
  ]),
  startRuntime: vi.fn(async () => ({
    id: "session-1",
    mission_id: null,
    runner_id: null,
    handle: "codex",
    pid: 123,
  })),
  renameSession: vi.fn(async () => {}),
  runtimeChanged: null as ((event: unknown) => void) | null,
  runtimeStatus: vi.fn(async () => ({
    shell: {
      shell: "/bin/zsh",
      outcome: "ok",
      duration_ms: 10,
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
        override_path: null,
        effective_command: "/usr/local/bin/codex",
        effective_source: "detected",
        state: "detected",
        invalid_reason: null,
      },
      {
        name: "qoder",
        display_name: "Qoder",
        command: "qodercli",
        detected_path: null,
        override_path: "/opt/qoder/bin/qodercli",
        effective_command: "/opt/qoder/bin/qodercli",
        effective_source: "override",
        state: "override",
        invalid_reason: null,
      },
      {
        name: "trae",
        display_name: "Trae",
        command: "trae-agent",
        detected_path: null,
        override_path: null,
        effective_command: "trae-agent",
        effective_source: "catalog",
        state: "not_found",
        invalid_reason: null,
      },
    ],
  })),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, callback: (event: unknown) => void) => {
    mocks.runtimeChanged = callback;
    return () => {
      mocks.runtimeChanged = null;
    };
  }),
}));

vi.mock("../lib/api", () => ({
  api: {
    crew: { listAll: mocks.crewList },
    slot: { list: vi.fn(async () => []) },
    mission: { start: vi.fn() },
    runner: { list: mocks.runnerList },
    runtime: {
      status: mocks.runtimeStatus,
    },
    session: {
      startDirect: vi.fn(),
      startRuntime: mocks.startRuntime,
      rename: mocks.renameSession,
    },
  },
}));

vi.mock("../lib/settings", () => ({
  AGENT_ENABLED_CHANGED_EVENT: "runner:agent-enabled-changed",
  STORAGE_DISABLED_AGENTS: "settings.disabledAgents",
  isAgentEnabled: (agent: string) => {
    const disabled = JSON.parse(
      localStorage.getItem("settings.disabledAgents") ?? "[]",
    ) as string[];
    return !disabled.includes(agent);
  },
  readDefaultRuntime: () =>
    localStorage.getItem("settings.defaultChatRuntime") ?? "",
  readDefaultWorkingDir: () => "/default",
}));

vi.mock("../lib/terminalSizing", () => ({
  estimateMissionTerminalGrid: () => ({ cols: 80, rows: 24 }),
}));

import { StartChatModal } from "./StartChatModal";
import { StartMissionModal } from "./StartMissionModal";

const project: ProjectRow = {
  id: "project-1",
  name: "Runner",
  cwd: "/projects/runner",
  position: 0,
  created_at: "2026-07-14T00:00:00Z",
};

function field<T extends HTMLInputElement | HTMLTextAreaElement>(
  container: HTMLElement,
  suffix: string,
): T {
  const element = container.querySelector<T>(`[id$="-${suffix}"]`);
  if (!element) throw new Error(`missing ${suffix} field`);
  return element;
}

async function changeField(
  element: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  await act(async () => {
    const prototype =
      element instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
      element,
      value,
    );
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("project-scoped chat and mission start modals", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.startRuntime.mockClear();
    mocks.renameSession.mockClear();
    mocks.runtimeStatus.mockClear();
    mocks.runtimeChanged = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("preserves an edited mission form when the project row refreshes", async () => {
    const render = (row: ProjectRow) =>
      createElement(StartMissionModal, {
        open: true,
        project: row,
        onClose: () => {},
        onStarted: () => {},
      });
    await act(async () => {
      root.render(render(project));
    });
    await changeField(field(container, "title"), "Keep this title");
    await changeField(field(container, "goal"), "Keep this goal");
    await changeField(field(container, "cwd"), "/custom/mission");

    await act(async () => {
      root.render(render({ ...project, name: "Runner renamed" }));
    });

    expect(field(container, "title").value).toBe("Keep this title");
    expect(field(container, "goal").value).toBe("Keep this goal");
    expect(field(container, "cwd").value).toBe("/custom/mission");
  });

  it("preserves edited chat fields when the project row refreshes", async () => {
    const render = (row: ProjectRow) =>
      createElement(StartChatModal, {
        open: true,
        project: row,
        onClose: () => {},
        onStarted: () => {},
      });
    await act(async () => {
      root.render(render(project));
    });
    await changeField(field(container, "title"), "Keep this chat");
    await changeField(field(container, "cwd"), "/custom/chat");

    await act(async () => {
      root.render(render({ ...project, name: "Runner renamed" }));
    });

    expect(field(container, "title").value).toBe("Keep this chat");
    expect(field(container, "cwd").value).toBe("/custom/chat");
  });

  it("uses the default runtime selected in Agents settings", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");
    localStorage.setItem("settings.defaultChatRuntime", "qoder");

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });

    const runtimePicker = container.querySelector<HTMLButtonElement>(
      '[id$="-runtime"]',
    );
    expect(runtimePicker?.textContent).toContain("Qoder");
    expect(field(container, "title").value).toBe("Qoder");
  });

  it("only offers detected runtimes and valid overrides", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });

    const runtimePicker = container.querySelector<HTMLButtonElement>(
      '[id$="-runtime"]',
    );
    await act(async () => runtimePicker?.click());
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[role="option"]'),
    ).map((option) => option.textContent);

    expect(options).toEqual([
      expect.stringContaining("Codex"),
      expect.stringContaining("Qoder"),
    ]);
    expect(options.join(" ")).not.toContain("Trae");
  });

  it("omits disabled agents from the runtime picker", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");
    localStorage.setItem("settings.disabledAgents", '["qoder"]');

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });

    const runtimePicker = container.querySelector<HTMLButtonElement>(
      '[id$="-runtime"]',
    );
    await act(async () => runtimePicker?.click());
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[role="option"]'),
    ).map((option) => option.textContent);

    expect(options).toEqual([expect.stringContaining("Codex")]);
    expect(options.join(" ")).not.toContain("Qoder");
  });

  it("refreshes the agent picker when executable discovery completes", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");
    const checking = await mocks.runtimeStatus();
    mocks.runtimeStatus
      .mockResolvedValueOnce({
        ...checking,
        shell: { ...checking.shell, checking: true },
        runtimes: checking.runtimes.map((runtime) => ({
          ...runtime,
          detected_path: null,
          effective_command: runtime.command,
          effective_source: "catalog",
          state: "checking",
        })),
      });

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });
    expect(container.textContent).toContain("Detecting agents…");

    await act(async () => mocks.runtimeChanged?.({}));
    expect(container.textContent).not.toContain("Detecting agents…");
    expect(
      container.querySelector<HTMLButtonElement>('[id$="-runtime"]')
        ?.textContent,
    ).toContain("Codex");
  });

  it("starts a runtime chat with default model and effort overrides", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });

    const model = field<HTMLInputElement>(container, "model");
    const effort = container.querySelector<HTMLButtonElement>(
      '[id$="-effort"]',
    );
    expect(model.value).toBe("");
    expect(model.placeholder).toBe("default");
    expect(effort?.textContent).toContain("default");

    const start = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Start chat");
    await act(async () => start?.click());

    expect(mocks.startRuntime).toHaveBeenCalledWith(
      "codex",
      "/projects/runner",
      null,
      null,
      "project-1",
      null,
      null,
    );
  });

  it("passes selected model and effort to a runtime chat", async () => {
    localStorage.setItem("runner.startChat.mode", "runtime");

    await act(async () => {
      root.render(
        createElement(StartChatModal, {
          open: true,
          project,
          onClose: () => {},
          onStarted: () => {},
        }),
      );
    });

    await changeField(
      field<HTMLInputElement>(container, "model"),
      "gpt-5.6-sol",
    );
    const effort = container.querySelector<HTMLButtonElement>(
      '[id$="-effort"]',
    );
    await act(async () => effort?.click());
    const max = Array.from(
      document.querySelectorAll<HTMLButtonElement>('[role="option"] button'),
    ).find((button) => button.textContent?.includes("Max"));
    expect(max).toBeDefined();
    await act(async () => max?.click());

    const start = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Start chat");
    await act(async () => start?.click());

    expect(mocks.startRuntime).toHaveBeenCalledWith(
      "codex",
      "/projects/runner",
      null,
      null,
      "project-1",
      "gpt-5.6-sol",
      "max",
    );
  });
});
