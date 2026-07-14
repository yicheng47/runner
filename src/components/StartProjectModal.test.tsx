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
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));

vi.mock("../lib/api", () => ({
  api: {
    crew: { list: mocks.crewList },
    slot: { list: vi.fn(async () => []) },
    mission: { start: vi.fn() },
    runner: { list: mocks.runnerList },
    runtime: {
      list: vi.fn(async () => [
        { name: "codex", display_name: "Codex", command: "codex" },
      ]),
    },
    session: {
      startDirect: vi.fn(),
      startRuntime: vi.fn(),
      rename: vi.fn(),
    },
  },
}));

vi.mock("../lib/settings", () => ({
  readDefaultChatRuntime: () => "codex",
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
  collapsed: true,
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

describe("project-scoped start modals", () => {
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
      root.render(render({ ...project, collapsed: false }));
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
      root.render(render({ ...project, collapsed: false }));
    });

    expect(field(container, "title").value).toBe("Keep this chat");
    expect(field(container, "cwd").value).toBe("/custom/chat");
  });
});
