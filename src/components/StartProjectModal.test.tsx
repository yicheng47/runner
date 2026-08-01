/** @vitest-environment jsdom */

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectRow } from "../lib/api";

const mocks = vi.hoisted(() => ({
  basename: vi.fn(async (path: string) => {
    if (path === "/") throw new Error("path has no basename");
    const trimmed = path.replace(/\/+$/, "");
    return trimmed.slice(trimmed.lastIndexOf("/") + 1);
  }),
  defaultWorkingDir: "",
  openDialog: vi.fn(async () => null as string | null),
  projectCreate: vi.fn(),
}));

vi.mock("@tauri-apps/api/path", () => ({
  basename: mocks.basename,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.openDialog,
}));

vi.mock("../lib/api", () => ({
  api: {
    project: { create: mocks.projectCreate },
  },
}));

vi.mock("../lib/settings", () => ({
  readDefaultWorkingDir: () => mocks.defaultWorkingDir,
}));

import { StartProjectModal } from "./StartProjectModal";

const project: ProjectRow = {
  id: "project-1",
  name: "Runner",
  cwd: "/projects/runner",
  position: 0,
  created_at: "2026-08-01T00:00:00Z",
};

function field(container: HTMLElement, suffix: string): HTMLInputElement {
  const element = container.querySelector<HTMLInputElement>(`[id$="-${suffix}"]`);
  if (!element) throw new Error(`missing ${suffix} field`);
  return element;
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const element = [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent === label,
  );
  if (!element) throw new Error(`missing ${label} button`);
  return element;
}

async function changeField(element: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(
      element,
      value,
    );
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("StartProjectModal", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    mocks.defaultWorkingDir = "";
    mocks.projectCreate.mockResolvedValue(project);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  async function render(onCreated = vi.fn()) {
    await act(async () => {
      root.render(
        createElement(StartProjectModal, {
          open: true,
          onClose: () => {},
          onCreated,
        }),
      );
    });
  }

  it("updates the name with the directory until the name is edited", async () => {
    await render();

    await changeField(field(container, "cwd"), "/projects/alpha");
    await vi.waitFor(() => expect(field(container, "name").value).toBe("alpha"));

    await changeField(field(container, "cwd"), "/projects/beta");
    await vi.waitFor(() => expect(field(container, "name").value).toBe("beta"));

    await changeField(field(container, "name"), "Custom name");
    await changeField(field(container, "cwd"), "/projects/gamma");

    expect(field(container, "name").value).toBe("Custom name");
  });

  it("ignores transient basename failures while the directory is typed", async () => {
    await render();

    await changeField(field(container, "cwd"), "/");
    await vi.waitFor(() => expect(mocks.basename).toHaveBeenLastCalledWith("/"));
    expect(field(container, "name").value).toBe("");
    expect(container.textContent).not.toContain("path has no basename");

    await changeField(field(container, "cwd"), "/projects/runner");
    await vi.waitFor(() => expect(field(container, "name").value).toBe("runner"));
    expect(container.textContent).not.toContain("path has no basename");
  });

  it("prefills the directory and name from the default working directory", async () => {
    mocks.defaultWorkingDir = "/workspace/runner";

    await render();

    expect(field(container, "cwd").value).toBe("/workspace/runner");
    await vi.waitFor(() => expect(field(container, "name").value).toBe("runner"));
  });

  it("disables create while either field is empty", async () => {
    await render();
    const createButton = button(container, "Create project");

    expect(createButton.disabled).toBe(true);

    await changeField(field(container, "cwd"), "/projects/runner");
    await vi.waitFor(() => expect(field(container, "name").value).toBe("runner"));
    expect(createButton.disabled).toBe(false);

    await changeField(field(container, "name"), "");
    expect(createButton.disabled).toBe(true);

    await changeField(field(container, "name"), "Runner");
    await changeField(field(container, "cwd"), "");
    expect(createButton.disabled).toBe(true);
  });

  it("creates the project and notifies the caller", async () => {
    mocks.defaultWorkingDir = "/projects/runner";
    const onCreated = vi.fn();
    await render(onCreated);
    await vi.waitFor(() => expect(button(container, "Create project").disabled).toBe(false));

    await act(async () => {
      container.querySelector("form")?.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
    });

    expect(mocks.projectCreate).toHaveBeenCalledWith("runner", "/projects/runner");
    expect(onCreated).toHaveBeenCalledWith(project);
  });

  it("browses from the field value and falls back to the default", async () => {
    mocks.defaultWorkingDir = "/workspace/default";
    await render();

    await changeField(field(container, "cwd"), "");
    await act(async () => button(container, "Browse…").click());
    expect(mocks.openDialog).toHaveBeenLastCalledWith({
      directory: true,
      defaultPath: "/workspace/default",
    });

    await changeField(field(container, "cwd"), "/workspace/current");
    await act(async () => button(container, "Browse…").click());
    expect(mocks.openDialog).toHaveBeenLastCalledWith({
      directory: true,
      defaultPath: "/workspace/current",
    });
  });
});
