/** @vitest-environment jsdom */

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  tabRows: [
    {
      id: "tab-a",
      folder_id: null,
      name: "",
      position: 0,
      layout:
        '{"preset":"cols-2","slots":["A","B"],"sizes":{"cols-2:outer":[50,50]}}',
      created_at: "2026-07-14T00:00:00Z",
    },
    {
      id: "tab-b",
      folder_id: "folder-1",
      name: "",
      position: 1,
      layout:
        '{"preset":"cols-2","slots":["C","D"],"sizes":{"cols-2:outer":[50,50]}}',
      created_at: "2026-07-14T00:00:01Z",
    },
  ],
  tabUpsert: vi.fn(),
  sessionPin: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "test-window" }),
}));

vi.mock("@dnd-kit/core", () => ({
  DndContext: ({ children }: { children: ReactNode }) => children,
  DragOverlay: ({ children }: { children: ReactNode }) => children,
  PointerSensor: function PointerSensor() {},
  pointerWithin: () => [],
  useDroppable: () => ({ setNodeRef: () => {} }),
  useSensor: () => ({}),
  useSensors: () => [],
}));

vi.mock("@dnd-kit/sortable", () => ({
  SortableContext: ({ children }: { children: ReactNode }) => children,
  useSortable: () => ({ listeners: {}, setNodeRef: () => {} }),
  verticalListSortingStrategy: {},
}));

vi.mock("../lib/api", () => ({
  api: {
    project: {
      list: vi.fn(async () => []),
    },
    mission: {
      listSummary: vi.fn(async () => []),
    },
    session: {
      listRecentDirect: vi.fn(async () => [
        {
          session_id: "A",
          project_id: null,
          handle: "alpha",
          display_name: "Alpha",
          title: null,
          pinned: false,
          status: "running",
        },
        {
          session_id: "B",
          project_id: null,
          handle: "beta",
          display_name: "Beta",
          title: null,
          pinned: false,
          status: "running",
        },
        {
          session_id: "C",
          project_id: null,
          handle: "coder",
          display_name: "Coder",
          title: null,
          pinned: true,
          status: "running",
        },
        {
          session_id: "D",
          project_id: null,
          handle: "reviewer",
          display_name: "Reviewer",
          title: null,
          pinned: true,
          status: "running",
        },
      ]),
      activitySnapshot: vi.fn(async () => ({})),
      pin: mocks.sessionPin,
    },
    tab: {
      importOnce: vi.fn(async () => mocks.tabRows),
      list: vi.fn(async () => mocks.tabRows),
      upsert: vi.fn(async (input) => {
        mocks.tabUpsert(input);
        mocks.tabRows = mocks.tabRows.map((row) =>
          row.id === input.id ? { ...row, name: input.name } : row,
        );
      }),
      delete: vi.fn(async () => undefined),
    },
    folder: {
      list: vi.fn(async () => [
        {
          id: "folder-1",
          name: "Review",
          position: 0,
          collapsed: false,
          created_at: "2026-07-14T00:00:00Z",
        },
      ]),
    },
  },
}));

vi.mock("./StartMissionModal", () => ({ StartMissionModal: () => null }));
vi.mock("./StartChatModal", () => ({ StartChatModal: () => null }));
vi.mock("./CommandPalette", () => ({ CommandPalette: () => null }));
vi.mock("./UpdatePromptCard", () => ({ UpdatePromptCard: () => null }));

import {
  getPaneLayout,
  getPaneLayouts,
  hydratePaneLayoutsFromDb,
} from "../lib/paneLayout";
import { Sidebar } from "./Sidebar";

function buttonWithTitle(container: HTMLElement, title: string) {
  const button = Array.from(
    container.querySelectorAll<HTMLButtonElement>("button"),
  ).find((candidate) => candidate.title === title);
  if (!button) throw new Error(`missing button: ${title}`);
  return button;
}

function menuItem(container: HTMLElement, label: string) {
  const button = Array.from(
    container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
  ).find((candidate) => candidate.textContent?.trim() === label);
  if (!button) throw new Error(`missing menu item: ${label}`);
  return button;
}

function renameInput(container: HTMLElement) {
  const input = container.querySelector<HTMLInputElement>("input");
  if (!input) throw new Error("missing rename input");
  return input;
}

async function click(element: HTMLElement) {
  await act(async () => {
    element.dispatchEvent(
      new MouseEvent("click", {
        bubbles: true,
        clientX: 100,
        clientY: 100,
      }),
    );
  });
}

async function startRename(container: HTMLElement, title: string) {
  const row = buttonWithTitle(container, title).parentElement!;
  await click(buttonWithTitle(row, "More actions"));
  await click(menuItem(container, "Rename tab"));
}

async function changeInput(input: HTMLInputElement, value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(input: HTMLInputElement, key: string) {
  await act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key }));
  });
}

describe("Sidebar chat tab rename", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      clear: () => storage.clear(),
      getItem: (key: string) => storage.get(key) ?? null,
      removeItem: (key: string) => storage.delete(key),
      setItem: (key: string, value: string) => storage.set(key, value),
    });
    localStorage.clear();
    mocks.tabRows[0].name = "";
    mocks.tabRows[1].name = "";
    mocks.tabUpsert.mockClear();
    mocks.sessionPin.mockClear();
    await hydratePaneLayoutsFromDb();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("renames and clears a background durable tab without activating or moving it", async () => {
    await act(async () => {
      root.render(
        createElement(
          MemoryRouter,
          { initialEntries: ["/chats/A"] },
          createElement(Sidebar, {
            collapsed: false,
            onCollapsedChange: () => {},
            previewOpen: false,
            onPreviewOpenChange: () => {},
          }),
        ),
      );
    });
    const originalPlacement = getPaneLayouts().map((layout) => ({
      id: layout.id,
      folderId: layout.folderId,
    }));

    await startRename(container, "@coder + @reviewer");
    let input = renameInput(container);
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("@coder + @reviewer");

    await changeInput(input, "Review pair");
    input = renameInput(container);
    await press(input, "Enter");

    expect(buttonWithTitle(container, "Review pair")).toBeTruthy();
    expect(getPaneLayout().id).toBe("tab-a");
    expect(getPaneLayout("A").name).toBeNull();
    expect(getPaneLayout("C").name).toBe("Review pair");
    expect(mocks.tabUpsert).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "tab-b", name: "Review pair" }),
    );
    expect(
      getPaneLayouts().map((layout) => ({
        id: layout.id,
        folderId: layout.folderId,
      })),
    ).toEqual(originalPlacement);
    expect(mocks.sessionPin).not.toHaveBeenCalled();

    const writesAfterRename = mocks.tabUpsert.mock.calls.length;
    await startRename(container, "Review pair");
    input = renameInput(container);
    await changeInput(input, "Discarded");
    input = renameInput(container);
    await press(input, "Escape");
    expect(getPaneLayout("C").name).toBe("Review pair");
    expect(mocks.tabUpsert).toHaveBeenCalledTimes(writesAfterRename);

    await startRename(container, "Review pair");
    input = renameInput(container);
    await changeInput(input, "");
    input = renameInput(container);
    await press(input, "Enter");

    expect(buttonWithTitle(container, "@coder + @reviewer")).toBeTruthy();
    expect(getPaneLayout().id).toBe("tab-a");
    expect(getPaneLayout("C").name).toBeNull();
    expect(mocks.tabUpsert).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "tab-b", name: "" }),
    );
    expect(
      getPaneLayouts().map((layout) => ({
        id: layout.id,
        folderId: layout.folderId,
      })),
    ).toEqual(originalPlacement);
    expect(mocks.sessionPin).not.toHaveBeenCalled();
  });
});
