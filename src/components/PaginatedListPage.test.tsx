/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PaginatedListPage } from "./PaginatedListPage";

describe("PaginatedListPage", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("renders the divider and pager for a single page", async () => {
    await act(async () => {
      root.render(
        <PaginatedListPage
          title="Runners"
          description="Runner list"
          action={<button type="button">New</button>}
          error={null}
          loading={false}
          loaded
          totalCount={1}
          filteredCount={1}
          noun="runners"
          emptyState={<div>Empty</div>}
          query=""
          onQueryChange={vi.fn()}
          searchLabel="Search runners"
          searchPlaceholder="Search runners…"
          noMatches={<div>No matches</div>}
          page={1}
          pageCount={1}
          onPageChange={vi.fn()}
        >
          <div>Runner card</div>
        </PaginatedListPage>,
      );
    });

    const pager = container.querySelector('nav[aria-label="Pagination"]');
    expect(pager).not.toBeNull();
    expect(pager?.parentElement?.classList.contains("border-t")).toBe(true);
    expect(container.querySelector('button[aria-current="page"]')?.textContent).toBe(
      "1",
    );
  });
});
