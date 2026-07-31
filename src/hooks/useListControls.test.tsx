/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ListPage } from "../lib/types";
import {
  LIST_QUERY_DEBOUNCE_MS,
  useListControls,
} from "./useListControls";

interface Item {
  id: string;
}

type Controls = ReturnType<typeof useListControls<Item>>;

function Probe({
  loadPage,
  onRender,
}: {
  loadPage: (
    page: number,
    pageSize: number,
    query: string,
  ) => Promise<ListPage<Item>>;
  onRender: (controls: Controls) => void;
}) {
  onRender(useListControls(loadPage));
  return null;
}

describe("useListControls", () => {
  let container: HTMLDivElement;
  let root: Root;
  let controls: Controls | null;

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    controls = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  async function renderProbe(
    loadPage: (
      page: number,
      pageSize: number,
      query: string,
    ) => Promise<ListPage<Item>>,
  ) {
    await act(async () => {
      root.render(
        <Probe
          loadPage={loadPage}
          onRender={(next) => {
            controls = next;
          }}
        />,
      );
    });
  }

  function current(): Controls {
    if (!controls) throw new Error("list controls are unavailable");
    return controls;
  }

  it("debounces query-driven IPC and resets to page one", async () => {
    const loadPage = vi.fn(
      async (page: number): Promise<ListPage<Item>> => ({
        items: [{ id: page === 1 ? "one" : "nine" }],
        total_count: 9,
        filtered_count: 9,
      }),
    );
    await renderProbe(loadPage);
    expect(loadPage).toHaveBeenCalledWith(1, 8, "");

    await act(async () => current().setPage(2));
    expect(current().page).toBe(2);
    expect(loadPage).toHaveBeenLastCalledWith(2, 8, "");
    loadPage.mockClear();

    await act(async () => current().setQuery("needle"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LIST_QUERY_DEBOUNCE_MS - 1);
    });
    expect(loadPage).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(loadPage).toHaveBeenLastCalledWith(1, 8, "needle");
    expect(current().page).toBe(1);
  });

  it("steps back when the current page disappears", async () => {
    let shrunk = false;
    const firstPage = Array.from({ length: 8 }, (_, index) => ({
      id: `item-${index + 1}`,
    }));
    const loadPage = vi.fn(
      async (page: number): Promise<ListPage<Item>> => {
        if (!shrunk) {
          return {
            items: page === 1 ? firstPage : [{ id: "item-9" }],
            total_count: 9,
            filtered_count: 9,
          };
        }
        return {
          items: firstPage,
          total_count: 8,
          filtered_count: 8,
        };
      },
    );
    await renderProbe(loadPage);

    await act(async () => current().setPage(2));
    expect(current().page).toBe(2);
    expect(current().pageItems).toEqual([{ id: "item-9" }]);

    shrunk = true;
    await act(async () => {
      await current().refresh();
    });
    expect(current().page).toBe(1);
    expect(current().pageItems).toEqual(firstPage);
    expect(loadPage).toHaveBeenCalledWith(2, 8, "");
    expect(loadPage).toHaveBeenLastCalledWith(1, 8, "");
  });
});
