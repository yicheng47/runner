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
  RunnerEditDrawer: () => null,
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

describe("CrewEditor slot model override", () => {
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

  it("selects and saves a model for an overridden runtime", async () => {
    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[`/crews/${crew.id}`]}>
          <Routes>
            <Route path="/crews/:crewId" element={<CrewEditor />} />
          </Routes>
        </MemoryRouter>,
      );
    });

    const trigger = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent?.includes("model: default"));
    expect(trigger).toBeDefined();

    await act(async () => trigger?.click());

    const input = document.querySelector<HTMLInputElement>("#slot-model-slot-1");
    expect(input).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")
        ?.set?.call(input, "trae-model");
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const save = Array.from(
      document.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(mocks.slotUpdate).toHaveBeenCalledWith("slot-1", {
      model_override: "trae-model",
    });
  });
});
