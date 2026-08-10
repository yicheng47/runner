import { describe, expect, it } from "vitest";

import { hueForSeed } from "./RunnerAvatar";

describe("hueForSeed", () => {
  it("is deterministic for runner seeds", () => {
    expect(hueForSeed("coder")).toBe("var(--color-accent)");
    expect(hueForSeed("reviewer")).toBe("#c792ea");
    expect(hueForSeed("coder")).toBe(hueForSeed("coder"));
  });

  it("reserves the human warning hue", () => {
    expect(hueForSeed("human")).toBe("var(--color-warn)");
    expect(hueForSeed("coder")).not.toBe("var(--color-warn)");
    expect(hueForSeed("reviewer")).not.toBe("var(--color-warn)");
  });
});
