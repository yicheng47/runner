import { describe, expect, it } from "vitest";

import {
  EFFORT_OPTIONS_BY_RUNTIME,
  inferPermissionMode,
  modelSuggestions,
  PERMISSION_MODES_BY_RUNTIME,
  RUNTIME_OPTIONS,
  runtimeClearsOnResize,
  runtimeSupportsEffort,
  runtimeSupportsPermissionMode,
  stripPermissionFlags,
} from "./runtimes";

describe("qoder runtime metadata", () => {
  it("appends qoder without changing the default runtime", () => {
    expect(RUNTIME_OPTIONS.map((runtime) => runtime.value)).toEqual([
      "codex",
      "claude-code",
      "qoder",
    ]);
    expect(RUNTIME_OPTIONS[RUNTIME_OPTIONS.length - 1]).toMatchObject({
      value: "qoder",
      defaultCommand: "qodercli",
    });
  });

  it("supports only default and auto permission modes", () => {
    expect(runtimeSupportsPermissionMode("qoder")).toBe(true);
    expect(
      PERMISSION_MODES_BY_RUNTIME.qoder.map((mode) => mode.value),
    ).toEqual(["default", "auto"]);
    expect(
      inferPermissionMode("qoder", ["--permission-mode", "auto"]),
    ).toBe("auto");
    expect(
      stripPermissionFlags("qoder", [
        "--debug",
        "--permission-mode=auto",
      ]),
    ).toEqual(["--debug"]);
  });

  it("does not expose unprobed model or effort options", () => {
    expect(runtimeSupportsEffort("qoder")).toBe(false);
    expect(EFFORT_OPTIONS_BY_RUNTIME.qoder).toBeUndefined();
    expect(modelSuggestions("qoder")).toEqual([]);
  });

  it("keeps the frontend resize policy aligned for full-repaint runtimes", () => {
    expect(runtimeClearsOnResize("claude-code")).toBe(true);
    expect(runtimeClearsOnResize("codex")).toBe(true);
    expect(runtimeClearsOnResize("qoder")).toBe(true);
    expect(runtimeClearsOnResize("shell")).toBe(false);
  });
});
