import { describe, expect, it } from "vitest";

import {
  buildSearchDoc,
  clampPage,
  matchesQuery,
  pageWindow,
} from "./listControls";

describe("buildSearchDoc", () => {
  it("joins non-null fields into a lowercase search document", () => {
    expect(buildSearchDoc(["Lead", null, "Claude-Code", undefined, ""])).toBe(
      "lead claude-code ",
    );
  });
});

describe("matchesQuery", () => {
  it("matches normalized substrings", () => {
    expect(matchesQuery("lead claude-code", " CLAUDE ")).toBe(true);
    expect(matchesQuery("lead claude-code", "codex")).toBe(false);
  });
});

describe("clampPage", () => {
  it("clamps pages to the available range", () => {
    expect(clampPage(-2, 6)).toBe(1);
    expect(clampPage(3, 6)).toBe(3);
    expect(clampPage(9, 6)).toBe(6);
  });

  it("keeps an empty result set on page one", () => {
    expect(clampPage(4, 0)).toBe(1);
  });
});

describe("pageWindow", () => {
  it("shows every page when there are at most five", () => {
    expect(pageWindow(1, 1)).toEqual([1]);
    expect(pageWindow(3, 5)).toEqual([1, 2, 3, 4, 5]);
  });

  it("uses the start window through page three", () => {
    expect(pageWindow(1, 10)).toEqual([1, 2, 3, 4, "ellipsis", 10]);
    expect(pageWindow(3, 10)).toEqual([1, 2, 3, 4, "ellipsis", 10]);
  });

  it("uses a centered window away from the boundaries", () => {
    expect(pageWindow(4, 10)).toEqual([
      1,
      "ellipsis",
      3,
      4,
      5,
      "ellipsis",
      10,
    ]);
    expect(pageWindow(7, 10)).toEqual([
      1,
      "ellipsis",
      6,
      7,
      8,
      "ellipsis",
      10,
    ]);
  });

  it("uses the end window for the final three pages", () => {
    expect(pageWindow(8, 10)).toEqual([1, "ellipsis", 7, 8, 9, 10]);
    expect(pageWindow(10, 10)).toEqual([1, "ellipsis", 7, 8, 9, 10]);
  });

  it("clamps an out-of-range current page before building the window", () => {
    expect(pageWindow(20, 8)).toEqual([1, "ellipsis", 5, 6, 7, 8]);
  });
});
