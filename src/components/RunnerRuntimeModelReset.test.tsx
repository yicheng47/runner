/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Runner } from "../lib/types";

const mocks = vi.hoisted(() => ({
  runnerCreate: vi.fn(),
  runnerUpdate: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));

vi.mock("../lib/api", () => ({
  api: {
    runner: {
      create: mocks.runnerCreate,
      update: mocks.runnerUpdate,
    },
  },
}));

import { CreateRunnerModal } from "./CreateRunnerModal";
import { RunnerEditDrawer } from "./RunnerEditDrawer";

async function changeInput(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")
      ?.set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function selectRuntime(trigger: HTMLButtonElement, runtime: string) {
  await act(async () => trigger.click());
  const option = Array.from(
    document.querySelectorAll<HTMLButtonElement>('[role="option"] button'),
  ).find((button) => button.textContent?.includes(runtime));
  expect(option).toBeDefined();
  await act(async () => option?.click());
}

const runner: Runner = {
  id: "runner-1",
  handle: "coder",
  display_name: "Coder",
  runtime: "codex",
  command: "codex",
  args: [],
  working_dir: null,
  system_prompt: null,
  env: {},
  model: "gpt-5",
  effort: null,
  created_at: "2026-07-27T00:00:00Z",
  updated_at: "2026-07-27T00:00:00Z",
};

describe("runner runtime model reset", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.runnerCreate.mockReset();
    mocks.runnerUpdate.mockReset();
    mocks.runnerUpdate.mockResolvedValue(runner);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("resets a new runner's model when its runtime changes", async () => {
    await act(async () => {
      root.render(
        <CreateRunnerModal
          open
          onClose={() => {}}
          onCreated={() => {}}
        />,
      );
    });

    const model = container.querySelector<HTMLInputElement>("#new-runner-model");
    const runtime =
      container.querySelector<HTMLButtonElement>("#new-runner-runtime");
    expect(model).not.toBeNull();
    expect(runtime).not.toBeNull();

    await changeInput(model!, "gpt-5");
    await selectRuntime(runtime!, "trae");

    expect(model?.value).toBe("");
    expect(model?.placeholder).toBe("default");
  });

  it("resets an existing runner's model when its runtime changes", async () => {
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={runner}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const model = container.querySelector<HTMLInputElement>("#edit-model");
    const runtime =
      container.querySelector<HTMLButtonElement>("#edit-runtime");
    expect(model?.value).toBe("gpt-5");
    expect(runtime).not.toBeNull();

    await selectRuntime(runtime!, "trae");

    expect(model?.value).toBe("");
    expect(model?.placeholder).toBe("default");
  });

  it("shows a crew slot's effective engine without overwriting the runner engine", async () => {
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={runner}
          effectiveRuntime="trae"
          effectiveModel={null}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const runtime = container.querySelector<HTMLInputElement>("#edit-runtime");
    const model = container.querySelector<HTMLInputElement>("#edit-model");
    expect(runtime?.value).toBe("trae");
    expect(model?.value).toBe("default");
    expect(container.querySelector("#edit-args")).toBeNull();

    const save = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(mocks.runnerUpdate).toHaveBeenCalledWith("runner-1", {
      display_name: "Coder",
      working_dir: null,
      system_prompt: null,
    });
  });
});
