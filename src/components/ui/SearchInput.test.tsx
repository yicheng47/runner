/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SearchInput } from "./SearchInput";

describe("SearchInput", () => {
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

  it("keeps Escape and the clear button without rendering a keycap", async () => {
    const onChange = vi.fn();
    await act(async () => {
      root.render(
        <SearchInput
          value="needle"
          onChange={onChange}
          label="Search runners"
          placeholder="Search runners…"
        />,
      );
    });

    expect(container.textContent).not.toContain("esc");
    expect(
      container.querySelector('button[aria-label="Clear search runners"]'),
    ).not.toBeNull();
    const input = container.querySelector("input");
    if (!input) throw new Error("search input is missing");
    await act(async () => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    expect(onChange).toHaveBeenCalledWith("");
  });
});
