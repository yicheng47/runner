/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Crew, SlotWithRunner } from "../lib/types";

const mocks = vi.hoisted(() => ({
  crewGet: vi.fn(),
  slotList: vi.fn(),
  slotUpdate: vi.fn(),
  runnerEditDrawer: vi.fn<(props: unknown) => null>(() => null),
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("../lib/api", () => ({
  api: {
    crew: {
      get: mocks.crewGet,
      update: vi.fn(),
    },
    slot: {
      list: mocks.slotList,
      update: mocks.slotUpdate,
      setLead: vi.fn(),
      delete: vi.fn(),
      reorder: vi.fn(),
    },
  },
}));

vi.mock("../components/AddSlotModal", () => ({
  AddSlotModal: () => null,
}));

vi.mock("../components/RunnerEditDrawer", () => ({
  RunnerEditDrawer: mocks.runnerEditDrawer,
}));

vi.mock("../components/StartMissionModal", () => ({
  StartMissionModal: () => null,
}));

import CrewEditor from "./CrewEditor";

const crew: Crew = {
  id: "crew-1",
  name: "Crew",
  purpose: null,
  goal: null,
  system_prompt_addendum: null,
  created_at: "2026-07-27T00:00:00Z",
  updated_at: "2026-07-27T00:00:00Z",
};

const slot: SlotWithRunner = {
  id: "slot-1",
  crew_id: crew.id,
  runner_id: "runner-1",
  slot_handle: "coder",
  position: 0,
  lead: true,
  runtime_override: "trae",
  model_override: null,
  effort_override: null,
  added_at: "2026-07-27T00:00:00Z",
  runner: {
    id: "runner-1",
    handle: "coder",
    display_name: "Coder",
    runtime: "codex",
    command: "codex",
    args: [],
    working_dir: null,
    system_prompt: null,
    env: {},
    model: null,
    effort: null,
    created_at: "2026-07-27T00:00:00Z",
    updated_at: "2026-07-27T00:00:00Z",
  },
};

describe("CrewEditor slot agent overrides", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.crewGet.mockReset();
    mocks.slotList.mockReset();
    mocks.slotUpdate.mockReset();
    mocks.runnerEditDrawer.mockClear();
    mocks.listen.mockClear();
    mocks.crewGet.mockResolvedValue(crew);
    mocks.slotList.mockResolvedValue([slot]);
    mocks.slotUpdate.mockResolvedValue(slot);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("keeps model and effort selectors out of slot rows", async () => {
    mocks.slotList.mockResolvedValue([
      {
        ...slot,
        model_override: "trae-model",
        effort_override: "high",
      },
    ]);

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[`/crews/${crew.id}`]}>
          <Routes>
            <Route path="/crews/:crewId" element={<CrewEditor />} />
          </Routes>
        </MemoryRouter>,
      );
    });

    const buttons = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    );
    expect(
      buttons.some((button) => button.textContent?.includes("model:")),
    ).toBe(false);
    expect(
      buttons.some((button) => button.textContent?.includes("effort:")),
    ).toBe(false);
    expect(
      buttons.some((button) => button.title?.startsWith("Runtime override")),
    ).toBe(true);
    expect(container.textContent).toContain("model trae-model · effort high");
  });

  it("passes slot model and effort overrides through the edit drawer", async () => {
    mocks.slotList.mockResolvedValue([
      {
        ...slot,
        model_override: "trae-model",
        effort_override: "high",
      },
    ]);

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[`/crews/${crew.id}`]}>
          <Routes>
            <Route path="/crews/:crewId" element={<CrewEditor />} />
          </Routes>
        </MemoryRouter>,
      );
    });

    const actions = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Slot actions for @coder"]',
    );
    await act(async () => actions?.click());
    const edit = Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    ).find((button) => button.textContent?.includes("Edit runner"));
    await act(async () => edit?.click());

    const drawerCalls = mocks.runnerEditDrawer.mock.calls;
    const props = drawerCalls[drawerCalls.length - 1]?.[0] as {
      effectiveRuntime: string;
      effectiveModel: string | null;
      effectiveEffort: string | null;
      onSaveSlotOverrides: (input: {
        model_override: string | null;
        effort_override: string | null;
      }) => Promise<void>;
    };
    expect(props.effectiveRuntime).toBe("trae");
    expect(props.effectiveModel).toBe("trae-model");
    expect(props.effectiveEffort).toBe("high");

    await act(async () => {
      await props.onSaveSlotOverrides({
        model_override: "next-model",
        effort_override: "low",
      });
    });
    expect(mocks.slotUpdate).toHaveBeenCalledWith("slot-1", {
      model_override: "next-model",
      effort_override: "low",
    });
  });

  it("delegates runtime changes to the backend's atomic override reset", async () => {
    mocks.slotList.mockResolvedValue([
      {
        ...slot,
        model_override: "trae-model",
        effort_override: "high",
      },
    ]);

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[`/crews/${crew.id}`]}>
          <Routes>
            <Route path="/crews/:crewId" element={<CrewEditor />} />
          </Routes>
        </MemoryRouter>,
      );
    });

    const runtime = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.title?.startsWith("Runtime override"));
    expect(runtime).toBeDefined();
    await act(async () => runtime?.click());

    const claude = Array.from(
      document.querySelectorAll<HTMLButtonElement>('[role="option"] button'),
    ).find((button) => button.textContent?.includes("Claude Code"));
    expect(claude).toBeDefined();
    await act(async () => claude?.click());

    expect(mocks.slotUpdate).toHaveBeenCalledWith("slot-1", {
      runtime_override: "claude-code",
    });
  });

  it("preserves overrides when clearing a matching runtime pin", async () => {
    mocks.slotList.mockResolvedValue([
      {
        ...slot,
        runtime_override: "codex",
        model_override: "slot-model",
        effort_override: "high",
      },
    ]);

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[`/crews/${crew.id}`]}>
          <Routes>
            <Route path="/crews/:crewId" element={<CrewEditor />} />
          </Routes>
        </MemoryRouter>,
      );
    });

    const runtime = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.title === "Runtime (runner default)");
    expect(runtime).toBeDefined();
    await act(async () => runtime?.click());

    const runnerDefault = Array.from(
      document.querySelectorAll<HTMLButtonElement>('[role="option"] button'),
    ).find((button) => button.textContent?.includes("Runner default"));
    expect(runnerDefault).toBeDefined();
    await act(async () => runnerDefault?.click());

    expect(mocks.slotUpdate).toHaveBeenCalledWith("slot-1", {
      runtime_override: null,
    });
  });
});
