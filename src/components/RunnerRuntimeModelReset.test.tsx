/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Runner } from "../lib/types";

const mocks = vi.hoisted(() => ({
  runnerCreate: vi.fn(),
  runnerUpdate: vi.fn(),
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
      ["codex", "Codex", "codex"],
      ["claude-code", "Claude Code", "claude"],
      ["qoder", "Qoder", "qodercli"],
      ["trae", "TRAE CLI", "traecli"],
    ].map(([name, display_name, command]) => ({
      name,
      display_name,
      command,
      detected_path: `/usr/local/bin/${command}`,
      override_path: null,
      effective_command: `/usr/local/bin/${command}`,
      effective_source: "detected",
      state: "detected",
      invalid_reason: null,
    })),
  })),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
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
    runtime: { status: mocks.runtimeStatus },
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
  ).find((button) => button.textContent?.toLowerCase().includes(runtime));
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
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.runnerCreate.mockReset();
    mocks.runnerUpdate.mockReset();
    mocks.runnerUpdate.mockResolvedValue(runner);
    mocks.runtimeStatus.mockClear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("resets a new runner's model when its runtime changes", async () => {
    localStorage.setItem("settings.enabledAgents", '["trae"]');
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
    localStorage.setItem("settings.enabledAgents", '["trae"]');
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

  it("omits disabled agents when creating a runner", async () => {
    localStorage.setItem("settings.disabledAgents", '["qoder"]');
    await act(async () => {
      root.render(
        <CreateRunnerModal
          open
          onClose={() => {}}
          onCreated={() => {}}
        />,
      );
    });

    const trigger = container.querySelector<HTMLButtonElement>(
      "#new-runner-runtime",
    );
    await act(async () => trigger?.click());
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[role="option"]'),
    ).map((option) => option.textContent);
    expect(options.join(" ")).toContain("Codex");
    expect(options.join(" ")).not.toContain("Qoder");
  });

  it("shows a disabled current agent without offering it again", async () => {
    localStorage.setItem("settings.disabledAgents", '["codex"]');
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

    const trigger = container.querySelector<HTMLButtonElement>("#edit-runtime");
    expect(trigger?.textContent).toContain("Codex");
    await act(async () => trigger?.click());
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[role="option"]'),
    ).map((option) => option.textContent);
    expect(options.join(" ")).not.toContain("Codex");
  });

  it("preserves a stored max effort for codex", async () => {
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={{ ...runner, effort: "max" }}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const effort = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        'button[aria-haspopup="listbox"]',
      ),
    ).find((button) => button.textContent?.includes("Max"));
    expect(effort).toBeDefined();

    const save = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(mocks.runnerUpdate).toHaveBeenCalledWith(
      "runner-1",
      expect.objectContaining({ effort: "max" }),
    );
  });

  it("saves crew-slot model and effort overrides without overwriting the runner engine", async () => {
    const saveSlotOverrides = vi.fn();
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={runner}
          runtimeOverride="trae"
          effectiveModel={null}
          effectiveEffort={null}
          onSaveSlotOverrides={saveSlotOverrides}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const runtime = container.querySelector<HTMLButtonElement>("#edit-runtime");
    const model = container.querySelector<HTMLInputElement>("#edit-model");
    expect(runtime?.textContent).toContain("TRAE CLI");
    expect(model?.value).toBe("");
    expect(model?.placeholder).toBe("default");
    expect(container.querySelector("#edit-args")).toBeNull();

    await changeInput(model!, "trae-model");
    const effort = container.querySelector<HTMLButtonElement>("#edit-effort");
    expect(effort?.textContent).toContain("Runtime default");
    await selectRuntime(effort!, "high");

    const save = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(mocks.runnerUpdate).toHaveBeenCalledWith("runner-1", {
      display_name: "Coder",
      working_dir: null,
      system_prompt: null,
    });
    expect(saveSlotOverrides).toHaveBeenCalledWith({
      runtime_override: "trae",
      model_override: "trae-model",
      effort_override: "high",
    });
  });

  it("changes a slot's agent from the drawer, resetting the model", async () => {
    localStorage.setItem("settings.enabledAgents", '["trae"]');
    const saveSlotOverrides = vi.fn();
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={runner}
          runtimeOverride={null}
          effectiveModel="gpt-5-high"
          effectiveEffort={null}
          onSaveSlotOverrides={saveSlotOverrides}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const trigger = container.querySelector<HTMLButtonElement>("#edit-runtime");
    const model = container.querySelector<HTMLInputElement>("#edit-model");
    expect(trigger?.textContent).toContain("Runner default (Codex)");
    expect(model?.value).toBe("gpt-5-high");

    await selectRuntime(trigger!, "trae");
    expect(model?.value).toBe("");

    const save = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(saveSlotOverrides).toHaveBeenCalledWith({
      runtime_override: "trae",
      model_override: null,
      effort_override: null,
    });
  });

  it("clears a matching runtime pin without dropping overrides", async () => {
    const saveSlotOverrides = vi.fn();
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={runner}
          runtimeOverride="codex"
          effectiveModel="slot-model"
          effectiveEffort="high"
          onSaveSlotOverrides={saveSlotOverrides}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    const trigger = container.querySelector<HTMLButtonElement>("#edit-runtime");
    expect(trigger?.textContent).toContain("Codex");
    await selectRuntime(trigger!, "runner default");

    const save = Array.from(
      container.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Save");
    await act(async () => save?.click());

    expect(saveSlotOverrides).toHaveBeenCalledWith({
      runtime_override: null,
      model_override: "slot-model",
      effort_override: "high",
    });
  });

  it("shows the runner command when a crew slot inherits its engine", async () => {
    await act(async () => {
      root.render(
        <RunnerEditDrawer
          open
          runner={{ ...runner, command: "/opt/custom/codex-nightly" }}
          runtimeOverride={null}
          effectiveModel={null}
          effectiveEffort={null}
          onSaveSlotOverrides={() => {}}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
    });

    expect(
      container.querySelector<HTMLInputElement>("#edit-command")?.value,
    ).toBe("/opt/custom/codex-nightly");
  });
});
